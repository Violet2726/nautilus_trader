import asyncio
from datetime import UTC
from datetime import datetime
from decimal import Decimal
from typing import Any

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.parsing.execution import NAUTILUS_SIDE_TO_XT
from nautilus_trader.adapters.thinktrader.parsing.execution import ORDER_STATUS_MAP
from nautilus_trader.adapters.thinktrader.parsing.instruments import instrument_id_to_stock_code
from nautilus_trader.adapters.thinktrader.parsing.instruments import stock_code_to_instrument_id
from nautilus_trader.common.providers import InstrumentProvider
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
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.objects import AccountBalance
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity


class ThinkTraderExecutionClient(LiveExecutionClient):
    """ThinkTrader 执行客户端"""

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client: ThinkTraderClient,
        msgbus: Any,
        cache: Any,
        clock: Any,
        instrument_provider: InstrumentProvider,
        config: ThinkTraderExecClientConfig,
    ):
        account_type = AccountType.MARGIN if config.account_type == "CREDIT" else AccountType.CASH
        super().__init__(
            loop=loop,
            client_id=ClientId("THINKTRADER"),
            venue=TT_VENUE,
            oms_type=OmsType.NETTING,
            account_type=account_type,
            base_currency=Currency.from_str("CNY"),
            instrument_provider=instrument_provider,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )
        self._client = client
        self._instrument_provider = instrument_provider
        self._config = config
        self._set_account_id(AccountId(f"{TT_VENUE.value}-{config.account_id}"))

        # 订单 ID 映射
        self._client_order_id_to_order_id: dict[ClientOrderId, int] = {}
        self._order_id_to_client_order_id: dict[int, ClientOrderId] = {}
        self._order_seq_to_client_order_id: dict[int, ClientOrderId] = {}
        self._cancel_seq_to_client_order_id: dict[int, ClientOrderId] = {}

        # 注册回调
        self._client.register_event_handler("order_update", self._on_order_update)
        self._client.register_event_handler("trade", self._on_trade)
        self._client.register_event_handler("order_rejected", self._on_order_rejected)
        self._client.register_event_handler("order_async_response", self._on_order_async_response)
        self._client.register_event_handler("cancel_async_response", self._on_cancel_async_response)
        self._client.register_event_handler("asset_update", self._on_asset_update)
        self._client.register_event_handler("position_update", self._on_position_update)
        self._client.register_event_handler("cancel_rejected", self._on_cancel_rejected)

    async def _connect(self) -> None:
        self._client._clock = self._clock
        self._client._cache = self._cache
        self._client._msgbus = self._msgbus
        self._client._instrument_provider = self._instrument_provider

        await self._client._connect()
        self._client.set_relaxed_response_order_enabled(self._config.relaxed_response_order)

        await self._instrument_provider.initialize()

    async def _disconnect(self) -> None:
        await self._client._disconnect()

    async def generate_order_status_report(
        self,
        command: GenerateOrderStatusReport,
    ) -> OrderStatusReport | None:
        if command.client_order_id is None and command.venue_order_id is None:
            return None

        for xt_order in self._client.query_orders():
            report = self._try_parse_xt_order_to_order_status_report(xt_order)
            if report is None:
                continue

            if (
                command.client_order_id is not None
                and report.client_order_id == command.client_order_id
            ):
                return report
            if (
                command.venue_order_id is not None
                and report.venue_order_id == command.venue_order_id
            ):
                return report

        return None

    async def generate_order_status_reports(
        self,
        command: GenerateOrderStatusReports,
    ) -> list[OrderStatusReport]:
        reports: list[OrderStatusReport] = []
        start_ns = self._datetime_to_ns(command.start) if command.start else None
        end_ns = self._datetime_to_ns(command.end) if command.end else None
        for xt_order in self._client.query_orders():
            report = self._try_parse_xt_order_to_order_status_report(xt_order)
            if report is None:
                continue

            if command.instrument_id is not None and report.instrument_id != command.instrument_id:
                continue
            if start_ns is not None and report.ts_last < start_ns:
                continue
            if end_ns is not None and report.ts_last > end_ns:
                continue

            if command.open_only and report.order_status in (
                OrderStatus.CANCELED,
                OrderStatus.FILLED,
                OrderStatus.REJECTED,
                OrderStatus.DENIED,
            ):
                continue

            reports.append(report)

        return reports

    async def generate_fill_reports(
        self,
        command: GenerateFillReports,
    ) -> list[FillReport]:
        reports: list[FillReport] = []
        start_ns = self._datetime_to_ns(command.start) if command.start else None
        end_ns = self._datetime_to_ns(command.end) if command.end else None

        for xt_trade in self._client.query_trades():
            report = self._try_parse_xt_trade_to_fill_report(xt_trade)
            if report is None:
                continue

            if command.instrument_id is not None and report.instrument_id != command.instrument_id:
                continue
            if (
                command.venue_order_id is not None
                and report.venue_order_id != command.venue_order_id
            ):
                continue

            if start_ns is not None and report.ts_event < start_ns:
                continue
            if end_ns is not None and report.ts_event > end_ns:
                continue

            reports.append(report)

        return reports

    async def generate_position_status_reports(
        self,
        command: GeneratePositionStatusReports,
    ) -> list[PositionStatusReport]:
        reports: list[PositionStatusReport] = []
        now = self._clock.timestamp_ns()

        positions = self._client.query_positions()
        if not positions:
            if command.instrument_id is None:
                return []

            instrument = self._cache.instrument(command.instrument_id)
            size_precision = instrument.size_precision if instrument else 0
            return [
                PositionStatusReport.create_flat(
                    account_id=self.account_id,
                    instrument_id=command.instrument_id,
                    size_precision=size_precision,
                    ts_init=now,
                ),
            ]

        for pos in positions:
            instrument_id = stock_code_to_instrument_id(pos.stock_code)
            if command.instrument_id is not None and instrument_id != command.instrument_id:
                continue

            instrument = self._cache.instrument(instrument_id)
            size_precision = instrument.size_precision if instrument else 0

            if pos.volume > 0:
                side = PositionSide.LONG
            elif pos.volume < 0:
                side = PositionSide.SHORT
            else:
                side = PositionSide.FLAT

            reports.append(
                PositionStatusReport(
                    account_id=self.account_id,
                    instrument_id=instrument_id,
                    position_side=side,
                    quantity=Quantity.from_int(abs(pos.volume), size_precision),
                    report_id=UUID4(),
                    ts_last=now,
                    ts_init=now,
                    avg_px_open=Decimal(str(pos.avg_price)),
                ),
            )

        return reports

    async def generate_mass_status(
        self,
        lookback_mins: int | None = None,
    ) -> ExecutionMassStatus | None:
        ts_init = self._clock.timestamp_ns()
        mass_status = ExecutionMassStatus(
            client_id=self.client_id,
            account_id=self.account_id,
            venue=self.venue,
            report_id=UUID4(),
            ts_init=ts_init,
        )

        order_reports = await self.generate_order_status_reports(
            GenerateOrderStatusReports(
                instrument_id=None,
                start=None,
                end=None,
                open_only=False,
                command_id=UUID4(),
                ts_init=ts_init,
            ),
        )
        mass_status.add_order_reports(order_reports)

        start = None
        if lookback_mins is not None:
            start = datetime.fromtimestamp(
                (ts_init - lookback_mins * 60 * 1_000_000_000) / 1_000_000_000,
                tz=UTC,
            )

        fill_reports = await self.generate_fill_reports(
            GenerateFillReports(
                instrument_id=None,
                venue_order_id=None,
                start=start,
                end=None,
                command_id=UUID4(),
                ts_init=ts_init,
            ),
        )
        mass_status.add_fill_reports(fill_reports)

        pos_reports = await self.generate_position_status_reports(
            GeneratePositionStatusReports(
                instrument_id=None,
                start=None,
                end=None,
                command_id=UUID4(),
                ts_init=ts_init,
            ),
        )
        mass_status.add_position_reports(pos_reports)

        return mass_status

    async def _submit_order(self, command: SubmitOrder) -> None:
        """提交订单"""
        order = command.order
        stock_code = instrument_id_to_stock_code(order.instrument_id)

        # 目前仅支持股票买卖映射
        # 如需支持期货, 需扩展逻辑处理 open/close 标志
        order_type = NAUTILUS_SIDE_TO_XT.get(order.side)
        if order_type is None:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"不支持的订单方向: {order.side}",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        price = float(order.price) if order.price else 0

        if self._config.use_async_order:
            seq = self._client.place_order_async(
                stock_code=stock_code,
                order_type=order_type,
                volume=int(order.quantity),
                price=price,
                order_remark=str(order.client_order_id),
            )

            if seq > 0:
                self._order_seq_to_client_order_id[seq] = order.client_order_id
                self.generate_order_submitted(
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    ts_event=self._clock.timestamp_ns(),
                )
            else:
                self.generate_order_rejected(
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    reason=f"异步下单失败, 返回值: {seq}",
                    ts_event=self._clock.timestamp_ns(),
                )
            return

        order_id = self._client.place_order(
            stock_code=stock_code,
            order_type=order_type,
            volume=int(order.quantity),
            price=price,
            order_remark=str(order.client_order_id),
        )

        if order_id > 0:
            self._client_order_id_to_order_id[order.client_order_id] = order_id
            self._order_id_to_client_order_id[order_id] = order.client_order_id
            self.generate_order_submitted(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                ts_event=self._clock.timestamp_ns(),
            )
        else:
            self.generate_order_rejected(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                reason=f"下单失败, 返回值: {order_id}",
                ts_event=self._clock.timestamp_ns(),
            )

    async def _submit_order_list(self, command: SubmitOrderList) -> None:
        ts_init = self._clock.timestamp_ns()
        for order in command.order_list.orders:
            await self._submit_order(
                SubmitOrder(
                    trader_id=command.trader_id,
                    strategy_id=command.strategy_id,
                    order=order,
                    command_id=UUID4(),
                    ts_init=ts_init,
                    position_id=command.position_id,
                    client_id=command.client_id,
                    params=command.params,
                    correlation_id=command.correlation_id,
                ),
            )

    async def _modify_order(self, _command: ModifyOrder) -> None:
        command = _command
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
            reason="ThinkTrader 暂不支持修改订单, 请撤单后重下",
            ts_event=self._clock.timestamp_ns(),
        )

    def _cancel_order_id(
        self,
        *,
        order_id: int,
        client_order_id: ClientOrderId,
    ) -> None:
        if self._config.use_async_cancel:
            seq = self._client.cancel_order_async(order_id)
            if seq <= 0:
                self._on_cancel_rejected(order_id, f"撤单请求失败, 返回值: {seq}")
                return
            self._cancel_seq_to_client_order_id[seq] = client_order_id
            return

        result = self._client.cancel_order(order_id)
        if result <= 0:
            self._on_cancel_rejected(order_id, f"撤单请求失败, 返回值: {result}")

    def _find_order_id_from_orders(self, command: CancelOrder) -> int | None:
        for xt_order in self._client.query_orders():
            if (
                command.venue_order_id is not None
                and getattr(xt_order, "order_sysid", None) == command.venue_order_id.value
            ):
                order_id = getattr(xt_order, "order_id", None)
                if order_id:
                    return int(order_id)

            if getattr(xt_order, "order_remark", None) == command.client_order_id.value:
                order_id = getattr(xt_order, "order_id", None)
                if order_id:
                    return int(order_id)

        return None

    def _try_cancel_by_sysid(self, *, cached_order: Any, command: CancelOrder) -> bool:
        if command.venue_order_id is None:
            return False

        try:
            from xtquant import xtconstant as xtconstant
        except ModuleNotFoundError:
            xtconstant = None

        if xtconstant is None:
            return False

        venue = cached_order.instrument_id.venue.value
        market = (
            xtconstant.SH_MARKET
            if venue == "SSE"
            else xtconstant.SZ_MARKET
            if venue == "SZSE"
            else None
        )
        if market is None:
            return False

        if self._config.use_async_cancel:
            seq = self._client.cancel_order_by_sysid_async(
                market=market,
                order_sysid=command.venue_order_id.value,
            )
            if seq > 0:
                self._cancel_seq_to_client_order_id[seq] = command.client_order_id
                return True
            return False

        result = self._client.cancel_order_by_sysid(
            market=market,
            order_sysid=command.venue_order_id.value,
        )
        return result == 0

    async def _cancel_order(self, command: CancelOrder) -> None:
        """取消订单"""
        cached_order = self._cache.order(command.client_order_id)
        if cached_order is None:
            venue_order_id = command.venue_order_id or VenueOrderId(command.client_order_id.value)
            self.generate_order_cancel_rejected(
                strategy_id=command.strategy_id,
                instrument_id=command.instrument_id,
                client_order_id=command.client_order_id,
                venue_order_id=venue_order_id,
                reason=f"缓存中未找到订单: {command.client_order_id!r}",
                ts_event=self._clock.timestamp_ns(),
            )
            return

        if cached_order.is_closed:
            return

        order_id = self._client_order_id_to_order_id.get(command.client_order_id)
        if order_id:
            self._cancel_order_id(order_id=order_id, client_order_id=command.client_order_id)
            return

        found_order_id = self._find_order_id_from_orders(command)
        if found_order_id is not None:
            self._cancel_order_id(order_id=found_order_id, client_order_id=command.client_order_id)
            return

        if self._try_cancel_by_sysid(cached_order=cached_order, command=command):
            return

        venue_order_id = (
            cached_order.venue_order_id
            or command.venue_order_id
            or VenueOrderId(command.client_order_id.value)
        )
        self.generate_order_cancel_rejected(
            strategy_id=cached_order.strategy_id,
            instrument_id=cached_order.instrument_id,
            client_order_id=command.client_order_id,
            venue_order_id=venue_order_id,
            reason=f"未找到订单映射: {command.client_order_id!r}",
            ts_event=self._clock.timestamp_ns(),
        )

    async def _cancel_all_orders(self, command: CancelAllOrders) -> None:
        orders = self._client.query_orders()
        for xt_order in orders:
            instrument_id = stock_code_to_instrument_id(xt_order.stock_code)
            if command.instrument_id is not None and instrument_id != command.instrument_id:
                continue

            if command.order_side in (OrderSide.BUY, OrderSide.SELL):
                side = None
                order_type_field = getattr(xt_order, "order_type", None)
                if order_type_field == NAUTILUS_SIDE_TO_XT.get(OrderSide.BUY):
                    side = OrderSide.BUY
                elif order_type_field == NAUTILUS_SIDE_TO_XT.get(OrderSide.SELL):
                    side = OrderSide.SELL
                if side is not None and side != command.order_side:
                    continue

            mapped = ORDER_STATUS_MAP.get(xt_order.order_status)
            if mapped in (
                OrderStatus.CANCELED,
                OrderStatus.FILLED,
                OrderStatus.REJECTED,
                OrderStatus.DENIED,
            ):
                continue

            if xt_order.order_id:
                order_id = int(xt_order.order_id)
                if self._config.use_async_cancel:
                    self._client.cancel_order_async(order_id)
                else:
                    self._client.cancel_order(order_id)

    async def _batch_cancel_orders(self, command: BatchCancelOrders) -> None:
        for cancel in command.cancels:
            await self._cancel_order(cancel)

    async def _query_account(self, _command: QueryAccount) -> None:
        asset = self._client.query_asset()
        if not asset:
            return

        currency = Currency.from_str("CNY")
        cash = float(asset.get("cash", 0.0))
        frozen = float(asset.get("frozen_cash", 0.0))
        balances = [
            AccountBalance(
                total=Money(cash + frozen, currency),
                locked=Money(frozen, currency),
                free=Money(cash, currency),
            ),
        ]
        self.generate_account_state(
            balances=balances,
            margins=[],
            reported=True,
            ts_event=self._clock.timestamp_ns(),
            info={
                "market_value": asset.get("market_value"),
                "total_asset": asset.get("total_asset"),
            },
        )

    def _on_order_update(self, order: Any) -> None:
        """处理委托更新"""
        # order 是 XtOrder 对象
        order_id = order.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)

        if not client_order_id and order.order_remark:
            client_order_id = ClientOrderId(order.order_remark)

        if not client_order_id:
            self._log.warning(f"未知订单: order_id={order_id}")
            return

        status = ORDER_STATUS_MAP.get(order.order_status)
        cached_order = self._cache.order(client_order_id)

        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return

        ts_event = self._clock.timestamp_ns()

        # 根据状态生成相应事件
        if status == OrderStatus.ACCEPTED:
            if cached_order.status == OrderStatus.SUBMITTED:
                self.generate_order_accepted(
                    strategy_id=cached_order.strategy_id,
                    instrument_id=cached_order.instrument_id,
                    client_order_id=client_order_id,
                    venue_order_id=VenueOrderId(order.order_sysid),
                    ts_event=ts_event,
                )
        elif status == OrderStatus.CANCELED:
            self.generate_order_canceled(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                venue_order_id=VenueOrderId(order.order_sysid),
                ts_event=ts_event,
            )
        elif status == OrderStatus.REJECTED and cached_order.status not in (
            OrderStatus.REJECTED,
            OrderStatus.CANCELED,
            OrderStatus.FILLED,
        ):
            self.generate_order_rejected(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                reason=order.status_msg or "废单",
                ts_event=ts_event,
            )

    def _on_trade(self, trade: Any) -> None:
        """处理成交回报"""
        from nautilus_trader.model.identifiers import TradeId

        order_id = trade.order_id
        client_order_id = self._order_id_to_client_order_id.get(order_id)

        if not client_order_id:
            # 尝试从 order_remark 恢复
            if trade.order_remark:
                client_order_id = ClientOrderId(trade.order_remark)
            else:
                self._log.warning(f"未知成交: order_id={order_id}")
                return

        cached_order = self._cache.order(client_order_id)
        if cached_order is None:
            self._log.warning(f"缓存中未找到订单: {client_order_id}")
            return

        instrument_id = stock_code_to_instrument_id(trade.stock_code)
        instrument = self._cache.instrument(instrument_id)

        ts_event = (
            int(trade.traded_time) * 1_000_000_000
            if trade.traded_time
            else self._clock.timestamp_ns()
        )

        self.generate_order_filled(
            strategy_id=cached_order.strategy_id,
            instrument_id=cached_order.instrument_id,
            client_order_id=client_order_id,
            venue_order_id=VenueOrderId(trade.order_sysid),
            trade_id=TradeId(trade.traded_id),
            order_side=cached_order.side,
            order_type=cached_order.type,
            last_qty=Quantity.from_int(trade.traded_volume),
            last_px=Price.from_str(f"{trade.traded_price:.4f}"),
            quote_currency=instrument.quote_currency if instrument else Currency.from_str("CNY"),
            commission=Money(0, Currency.from_str("CNY")),
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            ts_event=ts_event,
        )

    def _on_asset_update(self, asset: Any) -> None:
        currency = Currency.from_str("CNY")
        cash = float(getattr(asset, "cash", 0.0) or 0.0)
        frozen = float(getattr(asset, "frozen_cash", 0.0) or 0.0)
        balances = [
            AccountBalance(
                total=Money(cash + frozen, currency),
                locked=Money(frozen, currency),
                free=Money(cash, currency),
            ),
        ]
        self.generate_account_state(
            balances=balances,
            margins=[],
            reported=True,
            ts_event=self._clock.timestamp_ns(),
            info={
                "market_value": getattr(asset, "market_value", None),
                "total_asset": getattr(asset, "total_asset", None),
            },
        )

    def _on_position_update(self, position: Any) -> None:
        try:
            instrument_id = stock_code_to_instrument_id(position.stock_code)
        except Exception:
            return

        volume = int(getattr(position, "volume", 0) or 0)
        if volume > 0:
            side = PositionSide.LONG
        elif volume < 0:
            side = PositionSide.SHORT
        else:
            side = PositionSide.FLAT

        instrument = self._cache.instrument(instrument_id)
        if instrument is None and self._instrument_provider.find(instrument_id) is not None:
            instrument = self._instrument_provider.find(instrument_id)

        if instrument is None:
            self.create_task(
                self._send_position_report_after_load(instrument_id=instrument_id, position=position),
            )
            return

        if not self._cache.instrument(instrument.id):
            self._msgbus.send(endpoint="DataEngine.process", msg=instrument)

        report = PositionStatusReport(
            account_id=self.account_id,
            instrument_id=instrument_id,
            position_side=side,
            quantity=instrument.make_qty(abs(volume)),
            report_id=UUID4(),
            ts_last=self._clock.timestamp_ns(),
            ts_init=self._clock.timestamp_ns(),
            avg_px_open=Decimal(str(getattr(position, "avg_price", 0.0) or 0.0)),
        )
        self._send_position_status_report(report)

    async def _send_position_report_after_load(self, instrument_id: Any, position: Any) -> None:
        await self._instrument_provider.load_ids_async([instrument_id])
        instrument = self._instrument_provider.find(instrument_id)
        if instrument is None:
            return
        if not self._cache.instrument(instrument.id):
            self._msgbus.send(endpoint="DataEngine.process", msg=instrument)

        volume = int(getattr(position, "volume", 0) or 0)
        if volume > 0:
            side = PositionSide.LONG
        elif volume < 0:
            side = PositionSide.SHORT
        else:
            side = PositionSide.FLAT

        report = PositionStatusReport(
            account_id=self.account_id,
            instrument_id=instrument_id,
            position_side=side,
            quantity=instrument.make_qty(abs(volume)),
            report_id=UUID4(),
            ts_last=self._clock.timestamp_ns(),
            ts_init=self._clock.timestamp_ns(),
            avg_px_open=Decimal(str(getattr(position, "avg_price", 0.0) or 0.0)),
        )
        self._send_position_status_report(report)

    def _on_order_rejected(self, order_id: int, reason: str) -> None:
        """处理订单拒绝"""
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        if not client_order_id:
            self._log.warning(f"未知的拒绝订单: order_id={order_id}")
            return

        cached_order = self._cache.order(client_order_id)
        if cached_order and cached_order.status not in (
            OrderStatus.REJECTED,
            OrderStatus.CANCELED,
            OrderStatus.FILLED,
        ):
            self.generate_order_rejected(
                strategy_id=cached_order.strategy_id,
                instrument_id=cached_order.instrument_id,
                client_order_id=client_order_id,
                reason=reason,
                ts_event=self._clock.timestamp_ns(),
            )

    def _on_order_async_response(self, response: Any) -> None:
        client_order_id: ClientOrderId | None = None
        if getattr(response, "order_remark", None):
            client_order_id = ClientOrderId(response.order_remark)
        elif getattr(response, "seq", None) in self._order_seq_to_client_order_id:
            client_order_id = self._order_seq_to_client_order_id.get(response.seq)

        if client_order_id is None:
            return

        order_id = getattr(response, "order_id", None)
        if not order_id:
            return

        self._client_order_id_to_order_id[client_order_id] = int(order_id)
        self._order_id_to_client_order_id[int(order_id)] = client_order_id

    def _on_cancel_async_response(self, response: Any) -> None:
        client_order_id: ClientOrderId | None = None
        if getattr(response, "order_remark", None):
            client_order_id = ClientOrderId(response.order_remark)
        elif getattr(response, "seq", None) in self._cancel_seq_to_client_order_id:
            client_order_id = self._cancel_seq_to_client_order_id.get(response.seq)

        if client_order_id is None:
            return

        order_id = getattr(response, "order_id", None)
        if order_id:
            self._client_order_id_to_order_id[client_order_id] = int(order_id)
            self._order_id_to_client_order_id[int(order_id)] = client_order_id

    def _on_cancel_rejected(self, order_id: int, reason: str) -> None:
        client_order_id = self._order_id_to_client_order_id.get(order_id)
        if client_order_id is None:
            for xt_order in self._client.query_orders():
                if getattr(xt_order, "order_id", None) == order_id and getattr(
                    xt_order,
                    "order_remark",
                    None,
                ):
                    client_order_id = ClientOrderId(xt_order.order_remark)
                    break

        if client_order_id is None:
            self._log.warning(f"未知的撤单失败订单: order_id={order_id}")
            return

        cached_order = self._cache.order(client_order_id)
        if cached_order is None or cached_order.is_closed:
            return

        venue_order_id = cached_order.venue_order_id or VenueOrderId(str(order_id))
        self.generate_order_cancel_rejected(
            strategy_id=cached_order.strategy_id,
            instrument_id=cached_order.instrument_id,
            client_order_id=client_order_id,
            venue_order_id=venue_order_id,
            reason=reason,
            ts_event=self._clock.timestamp_ns(),
        )

    @staticmethod
    def _datetime_to_ns(dt: datetime) -> int:
        return int(dt.timestamp() * 1_000_000_000)

    @staticmethod
    def _to_timestamp_ns(value: Any) -> int | None:
        if value is None:
            return None
        try:
            v = int(value)
        except Exception:
            return None
        if v >= 1_000_000_000_000:
            return v * 1_000_000
        if v >= 1_000_000_000:
            return v * 1_000_000_000
        return None

    def _try_parse_xt_order_to_order_status_report(self, xt_order: Any) -> OrderStatusReport | None:
        try:
            instrument_id = stock_code_to_instrument_id(xt_order.stock_code)
        except Exception:
            return None

        side = self._try_get_order_side(xt_order)
        if side is None:
            return None

        mapped_status = ORDER_STATUS_MAP.get(
            getattr(xt_order, "order_status", None), OrderStatus.SUBMITTED
        )
        order_type = self._parse_order_type(xt_order)

        ts_init = self._clock.timestamp_ns()
        ts_last = self._to_timestamp_ns(getattr(xt_order, "order_time", None)) or ts_init

        venue_order_id_value = getattr(xt_order, "order_sysid", "") or str(
            getattr(xt_order, "order_id", "")
        )
        client_order_id_value = getattr(xt_order, "order_remark", None)
        client_order_id = ClientOrderId(client_order_id_value) if client_order_id_value else None

        return OrderStatusReport(
            account_id=self.account_id,
            instrument_id=instrument_id,
            venue_order_id=VenueOrderId(venue_order_id_value),
            order_side=side,
            order_type=order_type,
            time_in_force=TimeInForce.DAY,
            order_status=mapped_status,
            quantity=Quantity.from_int(int(getattr(xt_order, "order_volume", 0))),
            filled_qty=Quantity.from_int(int(getattr(xt_order, "traded_volume", 0))),
            avg_px=Decimal(str(getattr(xt_order, "traded_price", 0.0))),
            report_id=UUID4(),
            ts_accepted=ts_last,
            ts_last=ts_last,
            ts_init=ts_init,
            client_order_id=client_order_id,
            price=Price.from_str(str(getattr(xt_order, "price", 0.0)))
            if getattr(xt_order, "price", None)
            else None,
        )

    def _try_get_order_side(self, obj: Any) -> OrderSide | None:
        offset_flag = getattr(obj, "offset_flag", None)
        if offset_flag is not None:
            try:
                from xtquant import xtconstant as xtconstant
            except ModuleNotFoundError:
                pass
            else:
                if offset_flag == xtconstant.OFFSET_FLAG_OPEN:
                    return OrderSide.BUY
                if offset_flag == xtconstant.OFFSET_FLAG_CLOSE:
                    return OrderSide.SELL

        order_type_field = getattr(obj, "order_type", None)
        if order_type_field == NAUTILUS_SIDE_TO_XT.get(OrderSide.BUY):
            return OrderSide.BUY
        if order_type_field == NAUTILUS_SIDE_TO_XT.get(OrderSide.SELL):
            return OrderSide.SELL
        return None

    @staticmethod
    def _parse_order_type(obj: Any) -> OrderType:
        price_type = getattr(obj, "price_type", None)
        try:
            from nautilus_trader.adapters.thinktrader.parsing.execution import PRICE_TYPE_MAP
        except Exception:
            return OrderType.LIMIT
        return PRICE_TYPE_MAP.get(price_type, OrderType.LIMIT)

    def _try_parse_xt_trade_to_fill_report(self, xt_trade: Any) -> FillReport | None:
        try:
            instrument_id = stock_code_to_instrument_id(xt_trade.stock_code)
        except Exception:
            return None

        side = self._try_get_order_side(xt_trade)
        if side is None:
            return None

        venue_order_id_value = getattr(xt_trade, "order_sysid", "") or str(
            getattr(xt_trade, "order_id", "")
        )
        ts_init = self._clock.timestamp_ns()
        ts_event = self._to_timestamp_ns(getattr(xt_trade, "traded_time", None)) or ts_init

        client_order_id_value = getattr(xt_trade, "order_remark", None)
        client_order_id = ClientOrderId(client_order_id_value) if client_order_id_value else None

        commission_value = getattr(xt_trade, "commission", 0.0) or 0.0

        from nautilus_trader.model.identifiers import TradeId

        return FillReport(
            account_id=self.account_id,
            instrument_id=instrument_id,
            venue_order_id=VenueOrderId(venue_order_id_value),
            trade_id=TradeId(str(getattr(xt_trade, "traded_id", ""))),
            order_side=side,
            last_qty=Quantity.from_int(int(getattr(xt_trade, "traded_volume", 0))),
            last_px=Price.from_str(str(getattr(xt_trade, "traded_price", 0.0))),
            commission=Money(float(commission_value), Currency.from_str("CNY")),
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            report_id=UUID4(),
            ts_event=ts_event,
            ts_init=ts_init,
            client_order_id=client_order_id,
            venue_position_id=None,
        )
