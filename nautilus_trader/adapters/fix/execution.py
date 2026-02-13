from __future__ import annotations

import asyncio
import contextlib
import datetime
import json
from decimal import Decimal
from typing import Any

from nautilus_trader.adapters.fix.client import ExecType
from nautilus_trader.adapters.fix.client import FixCredentials
from nautilus_trader.adapters.fix.client import OrdStatus
from nautilus_trader.adapters.fix.client import WindFixApplication
from nautilus_trader.adapters.fix.client import ensure_tls_tunnel
from nautilus_trader.adapters.fix.config import FixExecClientConfig
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.execution.messages import BatchCancelOrders
from nautilus_trader.execution.messages import CancelAllOrders
from nautilus_trader.execution.messages import CancelOrder
from nautilus_trader.execution.messages import GenerateFillReports
from nautilus_trader.execution.messages import GenerateOrderStatusReport
from nautilus_trader.execution.messages import GenerateOrderStatusReports
from nautilus_trader.execution.messages import GeneratePositionStatusReports
from nautilus_trader.execution.messages import ModifyOrder
from nautilus_trader.execution.messages import QueryAccount
from nautilus_trader.execution.messages import SubmitOrder
from nautilus_trader.execution.messages import SubmitOrderList
from nautilus_trader.execution.reports import ExecutionMassStatus
from nautilus_trader.execution.reports import FillReport
from nautilus_trader.execution.reports import OrderStatusReport
from nautilus_trader.execution.reports import PositionStatusReport
from nautilus_trader.live.execution_client import LiveExecutionClient
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import LiquiditySide
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderStatus
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.enums import PositionSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TradeId
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.objects import AccountBalance
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders.base import Order


def _parse_fix_kv(raw: str) -> dict[str, str]:
    if "\x01" in raw:
        parts = raw.split("\x01")
    elif "|" in raw:
        parts = raw.split("|")
    else:
        parts = [raw]

    out: dict[str, str] = {}
    for p in parts:
        if not p or "=" not in p:
            continue
        k, v = p.split("=", 1)
        out[k] = v
    return out


def _parse_utc_datetime_ns(value: str | None) -> int | None:
    if not value:
        return None
    for fmt in ("%Y%m%d-%H:%M:%S", "%Y%m%d-%H:%M:%S.%f"):
        try:
            dt = datetime.datetime.strptime(value, fmt).replace(tzinfo=datetime.UTC)
            return int(dt.timestamp() * 1_000_000_000)
        except ValueError:
            continue
    return None


def _ord_status_to_nautilus(status: str | None) -> OrderStatus:
    if status == OrdStatus.NEW.value:
        return OrderStatus.ACCEPTED
    if status == OrdStatus.PARTIALLY_FILLED.value:
        return OrderStatus.PARTIALLY_FILLED
    if status == OrdStatus.FILLED.value:
        return OrderStatus.FILLED
    if status == OrdStatus.CANCELED.value:
        return OrderStatus.CANCELED
    if status == OrdStatus.PENDING_CANCEL.value:
        return OrderStatus.PENDING_CANCEL
    if status == OrdStatus.REJECTED.value:
        return OrderStatus.REJECTED
    if status == OrdStatus.EXPIRED.value:
        return OrderStatus.EXPIRED
    return OrderStatus.ACCEPTED


def _order_side_from_fix(side: str | None) -> OrderSide:
    if side == "1":
        return OrderSide.BUY
    if side == "2":
        return OrderSide.SELL
    return OrderSide.NO_ORDER_SIDE


def _order_type_from_fix(ord_type: str | None) -> OrderType:
    if ord_type == "1":
        return OrderType.MARKET
    if ord_type == "2":
        return OrderType.LIMIT
    return OrderType.MARKET


def _tif_from_fix(tif: str | None) -> TimeInForce:
    if tif == "0":
        return TimeInForce.DAY
    if tif == "1":
        return TimeInForce.GTC
    if tif == "5":
        return TimeInForce.GTD
    return TimeInForce.DAY


class FixExecutionClient(LiveExecutionClient):
    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        instrument_provider: InstrumentProvider,
        config: FixExecClientConfig,
        name: str | None = None,
    ) -> None:
        client_id_str = name or "FIX"
        super().__init__(
            loop=loop,
            client_id=ClientId(client_id_str),
            venue=None,
            oms_type=OmsType.NETTING,
            account_type=AccountType.CASH,
            base_currency=Currency.from_str("CNY"),
            instrument_provider=instrument_provider,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._config = config
        self._instrument_provider = instrument_provider
        self._fix_application: WindFixApplication | None = None
        self._fix_initiator: Any = None

        self._client_order_id_to_venue_order_id: dict[ClientOrderId, VenueOrderId] = {}

        self._set_account_id(AccountId(f"{client_id_str}-{config.account_id}"))

    async def _connect(self) -> None:
        if self._config.use_tls_tunnel and self._config.remote_host and self._config.remote_port:
            ensure_tls_tunnel(
                remote_host=self._config.remote_host,
                remote_port=self._config.remote_port,
                local_host=self._config.tls_local_host,
                local_port=self._config.tls_local_port,
            )

        def start_fix() -> tuple[WindFixApplication, Any]:
            from nautilus_trader.adapters.fix.client import fix

            if fix is None:
                raise ModuleNotFoundError("quickfix is required to use the FIX adapter")

            settings = fix.SessionSettings(self._config.fix_settings_path)
            store_factory = fix.FileStoreFactory(settings)
            log_factory = fix.FileLogFactory(settings)

            wrapper = WindFixApplication(
                dictionary_path=self._config.fix_dictionary_path,
                credentials=FixCredentials(
                    username=self._config.username,
                    password=self._config.password,
                ),
            )
            wrapper.on_execution_report = self._on_execution_report_raw
            application = wrapper._as_application()
            initiator = fix.SocketInitiator(application, store_factory, settings, log_factory)
            initiator.start()
            return wrapper, initiator

        self._fix_application, self._fix_initiator = await asyncio.to_thread(start_fix)

        app = self._fix_application
        if app is None:
            return

        start_wait = self._clock.timestamp_ns()
        while app.session_id is None:
            await asyncio.sleep(0.25)
            if self._clock.timestamp_ns() - start_wait > 30_000_000_000:
                break

        await self._instrument_provider.initialize()

        self.create_task(self._query_account(None), log_msg="Initial account query")

    async def _disconnect(self) -> None:
        initiator = self._fix_initiator
        self._fix_initiator = None
        self._fix_application = None
        if initiator is not None:
            await asyncio.to_thread(initiator.stop)

    def _on_execution_report_raw(self, raw: str) -> None:
        self._loop.call_soon_threadsafe(self._handle_execution_report_raw, raw)

    def _cache_venue_order_id(self, client_order_id: ClientOrderId, venue_order_id: VenueOrderId) -> None:
        self._client_order_id_to_venue_order_id[client_order_id] = venue_order_id
        with contextlib.suppress(Exception):
            self._cache.add_venue_order_id(client_order_id, venue_order_id, overwrite=True)

    def _resolve_venue_order_id(
        self,
        client_order_id: ClientOrderId,
        explicit: VenueOrderId | None,
    ) -> VenueOrderId | None:
        if explicit is not None:
            return explicit
        mapped = self._client_order_id_to_venue_order_id.get(client_order_id)
        if mapped is not None:
            return mapped
        with contextlib.suppress(Exception):
            return self._cache.venue_order_id(client_order_id)
        return None

    def _handle_report_rejected(self, order: Order, reason: str, ts_event: int) -> None:
        self.generate_order_rejected(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            reason=reason,
            ts_event=ts_event,
        )

    def _handle_report_canceled(
        self,
        order: Order,
        client_order_id: ClientOrderId,
        explicit_venue_order_id: VenueOrderId | None,
        ts_event: int,
    ) -> None:
        venue_order_id = self._resolve_venue_order_id(client_order_id, explicit_venue_order_id)
        if venue_order_id is None:
            return
        self.generate_order_canceled(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            ts_event=ts_event,
        )

    def _handle_report_trade(
        self,
        order: Order,
        kv: dict[str, str],
        client_order_id: ClientOrderId,
        explicit_venue_order_id: VenueOrderId | None,
        ts_event: int,
    ) -> None:
        venue_order_id = self._resolve_venue_order_id(client_order_id, explicit_venue_order_id)
        if venue_order_id is None:
            return

        last_qty = kv.get("32")
        last_px = kv.get("31")
        exec_id = kv.get("17")
        if last_qty is None or last_px is None or exec_id is None:
            return

        self.generate_order_filled(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            trade_id=TradeId(exec_id),
            order_side=order.side,
            order_type=order.order_type,
            last_qty=Quantity.from_str(last_qty),
            last_px=Price.from_str(last_px),
            quote_currency=Currency.from_str("CNY"),
            commission=Money(0, Currency.from_str("CNY")),
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            ts_event=ts_event,
        )

    def _handle_execution_report_raw(self, raw: str) -> None:
        kv = _parse_fix_kv(raw)
        cl_ord_id = kv.get("11")
        if not cl_ord_id:
            return

        client_order_id = ClientOrderId(cl_ord_id)
        order = self._cache.order(client_order_id)
        if order is None:
            return

        ts_event = _parse_utc_datetime_ns(kv.get("60")) or self._clock.timestamp_ns()

        venue_order_id_str = kv.get("37")
        venue_order_id = VenueOrderId(venue_order_id_str) if venue_order_id_str else None
        if venue_order_id is not None:
            self._cache_venue_order_id(client_order_id, venue_order_id)

        exec_type = kv.get("150")
        ord_status = kv.get("39")
        text = kv.get("58") or ""

        if exec_type == ExecType.REJECTED.value or ord_status == OrdStatus.REJECTED.value:
            self._handle_report_rejected(order, text or "FIX order rejected", ts_event)
            return

        if exec_type == ExecType.CANCELED.value or ord_status == OrdStatus.CANCELED.value:
            self._handle_report_canceled(order, client_order_id, venue_order_id, ts_event)
            return

        if venue_order_id is not None and ord_status == OrdStatus.NEW.value:
            self.generate_order_accepted(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=venue_order_id,
                ts_event=ts_event,
            )

        if exec_type == ExecType.TRADE.value or ord_status in (
            OrdStatus.PARTIALLY_FILLED.value,
            OrdStatus.FILLED.value,
        ):
            self._handle_report_trade(order, kv, client_order_id, venue_order_id, ts_event)

    async def _submit_order(self, command: SubmitOrder) -> None:
        PyCondition.not_none(command, "command")
        order = command.order
        ts_event = self._clock.timestamp_ns()

        self.generate_order_submitted(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            ts_event=ts_event,
        )

        if order.order_type not in (OrderType.MARKET, OrderType.LIMIT):
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"Unsupported order type: {order.order_type}",
                ts_event=ts_event,
            )
            return

        app = self._fix_application
        if app is None:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason="FIX not connected",
                ts_event=ts_event,
            )
            return

        opt_ype = 23 if order.side == OrderSide.BUY else 24
        pr_type = 11 if order.order_type == OrderType.LIMIT else 14

        order_code = f"{order.instrument_id.symbol.value}.{order.instrument_id.venue.value}"

        price = order.price.as_double() if order.has_price else None
        volume = order.quantity.as_double()

        def send_order() -> str | None:
            return app.passorder(
                opt_ype=opt_ype,
                account_id=self._config.account_id,
                order_code=order_code,
                pr_type=pr_type,
                price=price,
                volume=volume,
                user_order_id=order.client_order_id.to_str(),
            )

        venue_order_id_str = await asyncio.to_thread(send_order)
        if not venue_order_id_str:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason="No order id returned by venue",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        venue_order_id = VenueOrderId(venue_order_id_str)
        self._cache_venue_order_id(order.client_order_id, venue_order_id)

        self.generate_order_accepted(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            ts_event=self._clock.timestamp_ns(),
        )

    async def _submit_order_list(self, command: SubmitOrderList) -> None:
        for order in command.order_list.orders:
            await self._submit_order(
                SubmitOrder(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    order=order,
                    command_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                    client_id=self.id,
                ),
            )

    async def _modify_order(self, command: ModifyOrder) -> None:
        cached_order = self._cache.order(command.client_order_id)
        venue_order_id = (
            command.venue_order_id
            or (cached_order.venue_order_id if cached_order is not None else None)
            or VenueOrderId(command.client_order_id.value)
        )
        self.generate_order_modify_rejected(
            strategy_id=command.strategy_id,
            instrument_id=command.instrument_id,
            client_order_id=command.client_order_id,
            venue_order_id=venue_order_id,
            reason="Modify not supported by FIX adapter",
            ts_event=self._clock.timestamp_ns(),
        )

    async def _cancel_order(self, command: CancelOrder) -> None:
        PyCondition.not_none(command, "command")
        order = self._cache.order(command.client_order_id)
        if order is None:
            return

        app = self._fix_application
        if app is None:
            self.generate_order_cancel_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=order.venue_order_id or VenueOrderId(order.client_order_id.value),
                reason="FIX not connected",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        venue_order_id = command.venue_order_id
        if venue_order_id is None:
            venue_order_id = self._client_order_id_to_venue_order_id.get(command.client_order_id)
        if venue_order_id is None:
            try:
                venue_order_id = self._cache.venue_order_id(command.client_order_id)
            except Exception:
                venue_order_id = None
        if venue_order_id is None:
            self.generate_order_cancel_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=VenueOrderId(order.client_order_id.value),
                reason="Missing venue_order_id",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        side = "1" if order.side == OrderSide.BUY else "2"

        def send_cancel() -> str | None:
            return app.cancel(
                order_id=venue_order_id.to_str(),
                account_id=self._config.account_id,
                side=side,
            )

        result = await asyncio.to_thread(send_cancel)
        if not result:
            self.generate_order_cancel_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                venue_order_id=venue_order_id,
                reason="Cancel rejected or timed out",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        self.generate_order_pending_cancel(
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=venue_order_id,
            ts_event=self._clock.timestamp_ns(),
        )

    async def _cancel_all_orders(self, command: CancelAllOrders) -> None:
        if command.order_side != OrderSide.NO_ORDER_SIDE:
            orders = self._cache.orders_open(account_id=self.account_id, side=command.order_side)
        else:
            orders = self._cache.orders_open(account_id=self.account_id)
        for order in orders:
            await self._cancel_order(
                CancelOrder(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    command_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                    client_id=self.id,
                ),
            )

    async def _batch_cancel_orders(self, command: BatchCancelOrders) -> None:
        for cxl in command.cancels:
            await self._cancel_order(cxl)

    async def _query_account(self, command: QueryAccount | None) -> None:
        app = self._fix_application
        if app is None:
            return

        def fetch_account() -> Any:
            return app.get_trade_detail_data(self._config.account_id, "ACCOUNT")

        raw = await asyncio.to_thread(fetch_account)
        if not raw or raw == {} or isinstance(raw, dict):
            return

        data = json.loads(raw)
        fundings = data.get("NoFundings") or []

        balances: list[AccountBalance] = []
        for f in fundings:
            if not isinstance(f, dict):
                continue
            currency = Currency.from_str(f.get("Currency") or "CNY")
            useable = float(f.get("UseableAmt") or 0.0)
            market_cap = float(f.get("MarketCap") or 0.0)
            total = useable + market_cap
            balances.append(
                AccountBalance(
                    total=Money(total, currency),
                    locked=Money(0, currency),
                    free=Money(useable, currency),
                ),
            )

        if balances:
            self.generate_account_state(
                balances=balances,
                margins=[],
                reported=True,
                ts_event=self._clock.timestamp_ns(),
                info={},
            )

    async def generate_order_status_report(
        self,
        command: GenerateOrderStatusReport,
    ) -> OrderStatusReport | None:
        if command.client_order_id is None and command.venue_order_id is None:
            return None

        reports = await self.generate_order_status_reports(
            GenerateOrderStatusReports(
                instrument_id=command.instrument_id,
                start=command.start,
                end=command.end,
                open_only=False,
                command_id=command.id,
                ts_init=command.ts_init,
                params=command.params,
            ),
        )

        for r in reports:
            if command.client_order_id is not None and r.client_order_id == command.client_order_id:
                return r
            if command.venue_order_id is not None and r.venue_order_id == command.venue_order_id:
                return r
        return None

    async def generate_order_status_reports(
        self,
        command: GenerateOrderStatusReports,
    ) -> list[OrderStatusReport]:
        app = self._fix_application
        if app is None:
            return []

        def fetch_orders() -> Any:
            return app.get_trade_detail_data(self._config.account_id, "ORDER")

        raw = await asyncio.to_thread(fetch_orders)
        if not raw or raw == {} or isinstance(raw, dict):
            return []

        data = json.loads(raw)
        orders = data.get("NoQueryOrders") or []

        reports: list[OrderStatusReport] = []
        now_ns = self._clock.timestamp_ns()
        for o in orders:
            if not isinstance(o, dict):
                continue
            symbol = o.get("Symbol")
            venue_order_id_str = o.get("OrderID")
            if not symbol or not venue_order_id_str:
                continue

            instrument_id = InstrumentId.from_str(symbol)

            qty_str = str(o.get("OrderQty") or "0")
            leaves_str = str(o.get("LeavesQty") or "0")
            try:
                qty = Decimal(qty_str)
                leaves = Decimal(leaves_str)
                filled = max(Decimal(0), qty - leaves)
            except Exception:
                qty = Decimal(0)
                filled = Decimal(0)

            ts_last = _parse_utc_datetime_ns(o.get("TransactTime")) or now_ns

            report = OrderStatusReport(
                account_id=self.account_id,
                instrument_id=instrument_id,
                venue_order_id=VenueOrderId(str(venue_order_id_str)),
                order_side=_order_side_from_fix(o.get("Side")),
                order_type=_order_type_from_fix(o.get("OrdType")),
                time_in_force=_tif_from_fix(o.get("TimeInForce")),
                order_status=_ord_status_to_nautilus(o.get("OrdStatus")),
                quantity=Quantity.from_str(str(qty)),
                filled_qty=Quantity.from_str(str(filled)),
                report_id=UUID4(),
                ts_accepted=ts_last,
                ts_last=ts_last,
                ts_init=now_ns,
                client_order_id=ClientOrderId(o.get("ClOrdID")) if o.get("ClOrdID") else None,
                price=Price.from_str(str(o.get("Price"))) if o.get("Price") else None,
                avg_px=Decimal(str(o.get("AvgPx"))) if o.get("AvgPx") else None,
                cancel_reason=o.get("Text") if o.get("Text") else None,
            )
            reports.append(report)

        return reports

    async def generate_fill_reports(
        self,
        command: GenerateFillReports,
    ) -> list[FillReport]:
        app = self._fix_application
        if app is None:
            return []

        def fetch_deals() -> Any:
            return app.get_trade_detail_data(self._config.account_id, "DEAL")

        raw = await asyncio.to_thread(fetch_deals)
        if not raw or raw == {} or isinstance(raw, dict):
            return []

        data = json.loads(raw)
        orders = data.get("NoQueryOrders") or []
        now_ns = self._clock.timestamp_ns()

        reports: list[FillReport] = []
        for o in orders:
            if not isinstance(o, dict):
                continue
            symbol = o.get("Symbol")
            venue_order_id_str = o.get("OrderID")
            if not symbol or not venue_order_id_str:
                continue

            executes = o.get("NoExecutes") or []
            for e in executes:
                if not isinstance(e, dict):
                    continue
                exec_id = e.get("ExecID") or e.get("TradeID") or e.get("ExecRefID")
                last_qty = e.get("LastQty") or e.get("CumQty")
                last_px = e.get("LastPx") or e.get("LastPxPrice") or e.get("Price")
                if exec_id is None or last_qty is None or last_px is None:
                    continue
                ts_event = _parse_utc_datetime_ns(e.get("TransactTime")) or now_ns
                reports.append(
                    FillReport(
                        account_id=self.account_id,
                        instrument_id=InstrumentId.from_str(symbol),
                        venue_order_id=VenueOrderId(str(venue_order_id_str)),
                        trade_id=TradeId(str(exec_id)),
                        order_side=_order_side_from_fix(o.get("Side")),
                        last_qty=Quantity.from_str(str(last_qty)),
                        last_px=Price.from_str(str(last_px)),
                        commission=Money(0, Currency.from_str("CNY")),
                        liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
                        report_id=UUID4(),
                        ts_event=ts_event,
                        ts_init=now_ns,
                        client_order_id=ClientOrderId(o.get("ClOrdID")) if o.get("ClOrdID") else None,
                    ),
                )

        return reports

    async def generate_position_status_reports(
        self,
        command: GeneratePositionStatusReports,
    ) -> list[PositionStatusReport]:
        app = self._fix_application
        if app is None:
            return []

        def fetch_positions() -> Any:
            return app.get_trade_detail_data(self._config.account_id, "POSITION")

        raw = await asyncio.to_thread(fetch_positions)
        if not raw or raw == {} or isinstance(raw, dict):
            return []

        data = json.loads(raw)
        holdings = data.get("NoHoldings") or []
        now_ns = self._clock.timestamp_ns()

        reports: list[PositionStatusReport] = []
        for h in holdings:
            if not isinstance(h, dict):
                continue
            symbol = h.get("Symbol")
            if not symbol:
                continue
            qty = h.get("PositionQty")
            q = Decimal(str(qty or 0))
            position_side = PositionSide.LONG if q > 0 else PositionSide.SHORT if q < 0 else PositionSide.FLAT
            instrument_id = InstrumentId.from_str(symbol)
            reports.append(
                PositionStatusReport(
                    account_id=self.account_id,
                    instrument_id=instrument_id,
                    position_side=position_side,
                    quantity=Quantity.from_str(str(abs(q))),
                    report_id=UUID4(),
                    ts_last=now_ns,
                    ts_init=now_ns,
                    avg_px_open=Price.from_str(str(h.get("PositionCost") or 0)),
                ),
            )

        return reports

    async def generate_mass_status(
        self,
        lookback_mins: int | None = None,
    ) -> ExecutionMassStatus | None:
        now_ns = self._clock.timestamp_ns()
        status = ExecutionMassStatus(
            client_id=self.id,
            account_id=self.account_id,
            venue=None,
            report_id=UUID4(),
            ts_init=now_ns,
        )

        order_reports = await self.generate_order_status_reports(
            GenerateOrderStatusReports(
                instrument_id=None,
                start=None,
                end=None,
                open_only=False,
                command_id=UUID4(),
                ts_init=now_ns,
                params=None,
            ),
        )
        fill_reports = await self.generate_fill_reports(
            GenerateFillReports(
                instrument_id=None,
                venue_order_id=None,
                start=None,
                end=None,
                command_id=UUID4(),
                ts_init=now_ns,
                params=None,
            ),
        )
        pos_reports = await self.generate_position_status_reports(
            GeneratePositionStatusReports(
                instrument_id=None,
                start=None,
                end=None,
                command_id=UUID4(),
                ts_init=now_ns,
                params=None,
            ),
        )

        status.add_order_reports(order_reports)
        status.add_fill_reports(fill_reports)
        status.add_position_reports(pos_reports)
        return status
