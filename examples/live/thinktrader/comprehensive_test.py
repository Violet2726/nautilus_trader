import argparse
import asyncio
import contextlib
import datetime as dt
import os
import secrets
import warnings
from pathlib import Path
from typing import Any

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.adapters.thinktrader.historical import HistoricThinkTraderClient
from nautilus_trader.common.component import init_logging
from nautilus_trader.common.component import is_logging_initialized
from nautilus_trader.common.component import log_level_from_str
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.core.message import Event
from nautilus_trader.live.config import LiveExecClientConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import OrderBookDeltas
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import BookType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.events import AccountState
from nautilus_trader.model.events import OrderAccepted
from nautilus_trader.model.events import OrderCanceled
from nautilus_trader.model.events import OrderCancelRejected
from nautilus_trader.model.events import OrderDenied
from nautilus_trader.model.events import OrderEmulated
from nautilus_trader.model.events import OrderExpired
from nautilus_trader.model.events import OrderFilled
from nautilus_trader.model.events import OrderInitialized
from nautilus_trader.model.events import OrderModifyRejected
from nautilus_trader.model.events import OrderPendingCancel
from nautilus_trader.model.events import OrderPendingUpdate
from nautilus_trader.model.events import OrderRejected
from nautilus_trader.model.events import OrderReleased
from nautilus_trader.model.events import OrderSubmitted
from nautilus_trader.model.events import OrderTriggered
from nautilus_trader.model.events import OrderUpdated
from nautilus_trader.model.events import PositionChanged
from nautilus_trader.model.events import PositionClosed
from nautilus_trader.model.events import PositionEvent
from nautilus_trader.model.events import PositionOpened
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.objects import Price
from nautilus_trader.trading.strategy import Strategy


warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

_LOG_GUARD = None
CHINA_TZ = dt.timezone(dt.timedelta(hours=8))


def _floor_to_minute(ts: dt.datetime) -> dt.datetime:
    return ts.replace(second=0, microsecond=0)


def _clamp_to_cn_trading_end(now_utc: dt.datetime) -> dt.datetime:
    now_sh = now_utc.astimezone(CHINA_TZ)
    day = now_sh.date()

    morning_open = dt.datetime.combine(day, dt.time(9, 30), tzinfo=CHINA_TZ)
    morning_close = dt.datetime.combine(day, dt.time(11, 30), tzinfo=CHINA_TZ)
    afternoon_open = dt.datetime.combine(day, dt.time(13, 0), tzinfo=CHINA_TZ)
    day_close = dt.datetime.combine(day, dt.time(15, 0), tzinfo=CHINA_TZ)

    if now_sh >= day_close:
        end_sh = day_close
    elif afternoon_open <= now_sh < day_close:
        end_sh = now_sh
    elif morning_close <= now_sh < afternoon_open:
        end_sh = morning_close
    elif morning_open <= now_sh < morning_close:
        end_sh = now_sh
    else:
        end_sh = dt.datetime.combine(day - dt.timedelta(days=1), dt.time(15, 0), tzinfo=CHINA_TZ)

    return _floor_to_minute(end_sh).astimezone(dt.UTC)


def _load_dotenv() -> None:
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


def _parse_bool(value: str | None, default: bool = False) -> bool:
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "y", "on"}


def _parse_instrument_ids(value: str | None) -> tuple[str, ...]:
    if not value:
        return ("000001.SZSE",)
    items = [x.strip() for x in value.split(",")]
    return tuple(x for x in items if x)


class ThinkTraderComprehensiveConfig(StrategyConfig, frozen=True):
    instrument_ids: tuple[str, ...] = ("000001.SZSE",)
    account_id: str = ""
    client_id: str = TT
    enable_trading: bool = False
    subscribe_trade_ticks: bool = False
    subscribe_order_book: bool = False
    quotes_lookback_secs: int = 300
    bars_lookback_secs: int = 1800
    stop_after_secs: int = 180
    verbose_events: bool = True
    tick_print_every: int = 50
    test_trade_qty: int = 100


class ThinkTraderComprehensiveStrategy(Strategy):
    def __init__(self, config: ThinkTraderComprehensiveConfig) -> None:
        super().__init__(config)
        self._client_id = ClientId(config.client_id)
        self._instrument_ids = [InstrumentId.from_str(x) for x in config.instrument_ids]
        self._bar_types = [
            BarType.from_str(f"{x}-1-MINUTE-LAST-EXTERNAL") for x in config.instrument_ids
        ]

        self._seen_quotes = 0
        self._seen_trades = 0
        self._seen_books = 0
        self._seen_bars = 0
        self._seen_hist_quotes = 0
        self._seen_hist_trades = 0
        self._seen_hist_bars = 0

        self._cancel_probe_order = None
        self._fill_order = None
        self._close_order = None
        self._modify_requested = False
        self._cancel_requested = False
        self._fill_requested = False
        self._close_requested = False
        self._test_trade_started = False
        self._cancel_probe_modify_price: Price | None = None

    def on_start(self) -> None:
        self._event("START", instrument_ids=[str(x) for x in self._instrument_ids])
        self._event(
            "CONFIG",
            enable_trading=self.config.enable_trading,
            subscribe_trade_ticks=self.config.subscribe_trade_ticks,
            subscribe_order_book=self.config.subscribe_order_book,
            test_trade_qty=self.config.test_trade_qty,
        )
        now = dt.datetime.now(dt.UTC)
        quotes_start = now - dt.timedelta(seconds=self.config.quotes_lookback_secs)
        bars_start = now - dt.timedelta(seconds=self.config.bars_lookback_secs)

        self.request_instruments(TT_VENUE, client_id=self._client_id)

        for instrument_id in self._instrument_ids:
            self.request_instrument(instrument_id, client_id=self._client_id)
            self.request_quote_ticks(
                instrument_id=instrument_id,
                start=quotes_start,
                end=now,
                limit=10_000,
                client_id=self._client_id,
            )
            self.request_trade_ticks(
                instrument_id=instrument_id,
                start=quotes_start,
                end=now,
                limit=10_000,
                client_id=self._client_id,
            )

            self.subscribe_quote_ticks(instrument_id=instrument_id, client_id=self._client_id)
            if self.config.subscribe_trade_ticks:
                self.subscribe_trade_ticks(instrument_id=instrument_id, client_id=self._client_id)
            if self.config.subscribe_order_book:
                self.subscribe_order_book_deltas(
                    instrument_id=instrument_id,
                    book_type=BookType.L2_MBP,
                    client_id=self._client_id,
                )

        for bar_type in self._bar_types:
            self.request_bars(
                bar_type=bar_type,
                start=bars_start,
                end=now,
                limit=10_000,
                client_id=self._client_id,
            )
            self.subscribe_bars(bar_type=bar_type, client_id=self._client_id)

        if self.config.account_id:
            self._event("QUERY_ACCOUNT", account_id=f"{TT}-{self.config.account_id}")
            self._query_account()
        self.clock.set_time_alert(
            name="STATE@T+2s",
            alert_time=now + dt.timedelta(seconds=2),
            override=True,
            allow_past=True,
        )
        self.clock.set_time_alert(
            name="STATE@T+8s",
            alert_time=now + dt.timedelta(seconds=8),
            override=True,
            allow_past=True,
        )

        self.clock.set_time_alert(
            name="STOP_AFTER",
            alert_time=now + dt.timedelta(seconds=float(self.config.stop_after_secs)),
            override=True,
            allow_past=True,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        name = event.name
        if name.startswith("STATE@"):
            self._dump_state(name)
            return
        if name == "MODIFY_CANCEL_PROBE":
            if self._cancel_probe_order is not None and self._cancel_probe_modify_price is not None:
                self.modify_order(
                    self._cancel_probe_order,
                    price=self._cancel_probe_modify_price,
                    client_id=self._client_id,
                )
            self._cancel_probe_modify_price = None
            return
        if name == "STOP_AFTER":
            self.stop()

    def on_historical_data(self, data: Any) -> None:
        items = data if isinstance(data, list) else [data]
        for item in items:
            if isinstance(item, QuoteTick):
                self._seen_hist_quotes += 1
            elif isinstance(item, TradeTick):
                self._seen_hist_trades += 1
            elif isinstance(item, Bar):
                self._seen_hist_bars += 1

    def on_quote_tick(self, tick: QuoteTick) -> None:
        self._seen_quotes += 1
        if self.config.verbose_events and (self._seen_quotes % self.config.tick_print_every == 1):
            self._event(
                "QUOTE",
                instrument_id=str(tick.instrument_id),
                bid=str(tick.bid_price),
                ask=str(tick.ask_price),
                ts_event=tick.ts_event,
            )
        if self.config.enable_trading and not self._test_trade_started:
            self._test_trade_started = True
            self._submit_test_buy(tick)

    def on_trade_tick(self, _tick: TradeTick) -> None:
        self._seen_trades += 1
        if self.config.verbose_events and (self._seen_trades % self.config.tick_print_every == 1):
            self._event("TRADE", ts_event=_tick.ts_event, instrument_id=str(_tick.instrument_id))

    def on_order_book_deltas(self, _deltas: OrderBookDeltas) -> None:
        self._seen_books += 1

    def on_bar(self, _bar: Bar) -> None:
        self._seen_bars += 1
        if self.config.verbose_events and (self._seen_bars % 5 == 1):
            self._event("BAR", ts_event=_bar.ts_event, bar_type=str(_bar.bar_type))

    def on_order_initialized(self, event: OrderInitialized) -> None:
        self._event(
            "ORDER_INITIALIZED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_denied(self, event: OrderDenied) -> None:
        self._event(
            "ORDER_DENIED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@ORDER_DENIED")
        self.stop()

    def on_order_emulated(self, event: OrderEmulated) -> None:
        self._event(
            "ORDER_EMULATED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_released(self, event: OrderReleased) -> None:
        self._event(
            "ORDER_RELEASED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_submitted(self, event: OrderSubmitted) -> None:
        self._event(
            "ORDER_SUBMITTED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_rejected(self, event: OrderRejected) -> None:
        self._event(
            "ORDER_REJECTED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@ORDER_REJECTED")
        self.stop()

    def on_order_pending_update(self, event: OrderPendingUpdate) -> None:
        self._event(
            "ORDER_PENDING_UPDATE",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_pending_cancel(self, event: OrderPendingCancel) -> None:
        self._event(
            "ORDER_PENDING_CANCEL",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_modify_rejected(self, event: OrderModifyRejected) -> None:
        self._event(
            "ORDER_MODIFY_REJECTED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        if self._cancel_probe_order is None:
            return
        if event.client_order_id != self._cancel_probe_order.client_order_id:
            return
        if self._cancel_requested:
            return

        self._cancel_requested = True
        self.cancel_order(self._cancel_probe_order, client_id=self._client_id)

    def on_order_cancel_rejected(self, event: OrderCancelRejected) -> None:
        self._event(
            "ORDER_CANCEL_REJECTED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@ORDER_CANCEL_REJECTED")
        self.stop()

    def on_order_accepted(self, event: OrderAccepted) -> None:
        self._event(
            "ORDER_ACCEPTED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        if self._cancel_probe_order is None:
            return
        if event.client_order_id != self._cancel_probe_order.client_order_id:
            return
        if self._modify_requested:
            return

        self._modify_requested = True
        tick = self.cache.quote_tick(event.instrument_id)
        if tick and tick.ask_price.as_double() > 0:
            new_price = Price.from_str(f"{(tick.ask_price.as_double() + 0.02):.2f}")
        else:
            new_price = Price.from_str("0.01")
        self._cancel_probe_modify_price = new_price
        self.clock.set_time_alert(
            name="MODIFY_CANCEL_PROBE",
            alert_time=dt.datetime.now(dt.UTC) + dt.timedelta(seconds=1),
            override=True,
            allow_past=True,
        )

    def on_order_canceled(self, event: OrderCanceled) -> None:
        self._event(
            "ORDER_CANCELED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        if self._cancel_probe_order is None:
            return
        if event.client_order_id != self._cancel_probe_order.client_order_id:
            return
        if self._fill_requested:
            return

        self._fill_requested = True
        tick = self.cache.quote_tick(event.instrument_id)
        if tick is None or tick.ask_price.as_double() <= 0:
            self._dump_state("STATE@ORDER_CANCELED_NO_TICK")
            self.stop()
            return
        self._dump_state("STATE@ORDER_CANCELED")
        self._submit_fill_order(event.instrument_id, tick)

    def on_order_updated(self, event: OrderUpdated) -> None:
        self._event(
            "ORDER_UPDATED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_expired(self, event: OrderExpired) -> None:
        self._event(
            "ORDER_EXPIRED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@ORDER_EXPIRED")
        self.stop()

    def on_order_triggered(self, event: OrderTriggered) -> None:
        self._event(
            "ORDER_TRIGGERED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )

    def on_order_filled(self, event: OrderFilled) -> None:
        self._event(
            "ORDER_FILLED",
            client_order_id=str(event.client_order_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@ORDER_FILLED")
        if self._close_order and event.client_order_id == self._close_order.client_order_id:
            self.stop()

    def on_position_opened(self, event: PositionOpened) -> None:
        self._event(
            "POSITION_OPENED",
            position_id=str(event.position_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@POSITION_OPENED")
        if not self.config.enable_trading:
            return
        if self._close_requested:
            return
        tick = self.cache.quote_tick(event.instrument_id)
        if tick is None or tick.bid_price.as_double() <= 0:
            self._dump_state("STATE@POSITION_OPENED_NO_TICK")
            self.stop()
            return
        self._close_requested = True
        self._submit_test_sell(event.instrument_id, tick)

    def on_position_changed(self, event: PositionChanged) -> None:
        self._event(
            "POSITION_CHANGED",
            position_id=str(event.position_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@POSITION_CHANGED")

    def on_position_closed(self, event: PositionClosed) -> None:
        self._event(
            "POSITION_CLOSED",
            position_id=str(event.position_id),
            instrument_id=str(event.instrument_id),
        )
        self._dump_state("STATE@POSITION_CLOSED")
        if self.config.enable_trading:
            self.stop()

    def on_position_event(self, event: PositionEvent) -> None:
        self._event("POSITION_EVENT", event_type=type(event).__name__)

    def on_event(self, event: Event) -> None:
        if isinstance(event, AccountState):
            self._event("ACCOUNT_STATE", account_id=str(event.account_id))

    def on_stop(self) -> None:
        with contextlib.suppress(Exception):
            self.clock.cancel_timer("STATE@T+2s")
        with contextlib.suppress(Exception):
            self.clock.cancel_timer("STATE@T+8s")
        with contextlib.suppress(Exception):
            self.clock.cancel_timer("STOP_AFTER")
        with contextlib.suppress(Exception):
            self.clock.cancel_timer("MODIFY_CANCEL_PROBE")
        self._cancel_probe_modify_price = None
        self._dump_state("STATE@STOP")
        self._event(
            "STOP",
            seen_quotes=self._seen_quotes,
            seen_trades=self._seen_trades,
            seen_books=self._seen_books,
            seen_bars=self._seen_bars,
            hist_quotes=self._seen_hist_quotes,
            hist_trades=self._seen_hist_trades,
            hist_bars=self._seen_hist_bars,
        )
        for instrument_id in self._instrument_ids:
            self.cancel_all_orders(instrument_id=instrument_id, client_id=self._client_id)

    def _account_id(self) -> AccountId | None:
        if not self.config.account_id:
            return None
        return AccountId(f"{TT}-{self.config.account_id}")

    def _query_account(self) -> None:
        account_id = self._account_id()
        if account_id is None:
            return
        self.query_account(account_id=account_id, client_id=self._client_id)

    def _dump_state(self, tag: str) -> None:
        if not self.config.verbose_events:
            return

        self._dump_account_state(tag)

        for instrument_id in self._instrument_ids:
            self._dump_positions_state(tag, instrument_id)
            self._dump_orders_state(tag, instrument_id)

    def _dump_account_state(self, tag: str) -> None:
        account_id = self._account_id()
        if account_id is None:
            return

        account = self.cache.account(account_id)
        if account is None:
            self._event(tag, account_id=str(account_id), account="None")
            return

        total = None
        free = None
        locked = None
        with contextlib.suppress(Exception):
            total = account.balance_total()
        with contextlib.suppress(Exception):
            free = account.balance_free()
        with contextlib.suppress(Exception):
            locked = account.balance_locked()

        self._event(
            tag,
            account_id=str(account_id),
            balance_total=str(total) if total is not None else None,
            balance_free=str(free) if free is not None else None,
            balance_locked=str(locked) if locked is not None else None,
        )

    def _dump_positions_state(self, tag: str, instrument_id: InstrumentId) -> None:
        with contextlib.suppress(Exception):
            positions = self.cache.positions_open(instrument_id=instrument_id, strategy_id=self.id)
            self._event(tag, instrument_id=str(instrument_id), positions_open=len(positions))
            for pos in positions[:5]:
                self._event(tag, position=str(pos))

    def _dump_orders_state(self, tag: str, instrument_id: InstrumentId) -> None:
        with contextlib.suppress(Exception):
            orders = self.cache.orders_open(instrument_id=instrument_id, strategy_id=self.id)
            self._event(tag, instrument_id=str(instrument_id), orders_open=len(orders))
            for o in orders[:5]:
                coid = getattr(o, "client_order_id", None)
                status = getattr(o, "status", None)
                side = getattr(o, "side", None)
                qty = getattr(o, "quantity", None)
                px = getattr(o, "price", None)
                self._event(
                    tag,
                    order_client_order_id=str(coid) if coid is not None else None,
                    order_status=str(status) if status is not None else None,
                    order_side=str(side) if side is not None else None,
                    order_qty=str(qty) if qty is not None else None,
                    order_price=str(px) if px is not None else None,
                )

    def _event(self, kind: str, **kwargs: Any) -> None:
        if not self.config.verbose_events:
            return
        kv = " ".join(f"{k}={v}" for k, v in kwargs.items() if v is not None)
        ts = self.clock.timestamp_ns()
        print(f"[LIVE] ts={ts} {kind} {kv}".rstrip())

    def _start_cancel_probe(self, tick: QuoteTick) -> None:
        if tick.bid_price.as_double() <= 0:
            return

        instrument = self.cache.instrument(tick.instrument_id)
        if instrument is None:
            return

        price = Price.from_str(f"{max(tick.bid_price.as_double() * 0.50, 0.01):.2f}")
        qty = instrument.make_qty(100)
        self._cancel_probe_order = self.order_factory.limit(
            instrument_id=tick.instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(self._cancel_probe_order, client_id=self._client_id)

    def _submit_fill_order(self, instrument_id: InstrumentId, tick: QuoteTick) -> None:
        instrument = self.cache.instrument(instrument_id)
        if instrument is None:
            self.stop()
            return

        price = Price.from_str(f"{(tick.ask_price.as_double() + 0.02):.2f}")
        qty = instrument.make_qty(100)
        self._fill_order = self.order_factory.limit(
            instrument_id=instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(self._fill_order, client_id=self._client_id)

    def _submit_close_order(self, instrument_id: InstrumentId, tick: QuoteTick) -> None:
        positions = self.cache.positions_open(instrument_id=instrument_id, strategy_id=self.id)
        if not positions:
            self.stop()
            return

        sell_qty = positions[0].quantity
        price = Price.from_str(f"{max(tick.bid_price.as_double() - 0.02, 0.01):.2f}")
        self._close_order = self.order_factory.limit(
            instrument_id=instrument_id,
            order_side=OrderSide.SELL,
            quantity=sell_qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(self._close_order, client_id=self._client_id)

    def _submit_test_buy(self, tick: QuoteTick) -> None:
        instrument = self.cache.instrument(tick.instrument_id)
        if instrument is None:
            self._dump_state("STATE@TEST_BUY_NO_INSTRUMENT")
            self.stop()
            return

        if tick.ask_price.as_double() > 0:
            px = tick.ask_price.as_double() + 0.05
        elif tick.bid_price.as_double() > 0:
            px = tick.bid_price.as_double() + 0.05
        else:
            self._dump_state("STATE@TEST_BUY_NO_TICK")
            self.stop()
            return

        qty = instrument.make_qty(int(self.config.test_trade_qty))
        price = Price.from_str(f"{max(px, 0.01):.2f}")
        self._fill_order = self.order_factory.limit(
            instrument_id=tick.instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self._event(
            "TEST_BUY_SUBMIT",
            instrument_id=str(tick.instrument_id),
            qty=str(qty),
            price=str(price),
        )
        self.submit_order(self._fill_order, client_id=self._client_id)

    def _submit_test_sell(self, instrument_id: InstrumentId, tick: QuoteTick) -> None:
        positions = self.cache.positions_open(instrument_id=instrument_id, strategy_id=self.id)
        if not positions:
            self._dump_state("STATE@TEST_SELL_NO_POSITION")
            self.stop()
            return

        px = max(tick.bid_price.as_double() - 0.05, 0.01)
        price = Price.from_str(f"{px:.2f}")
        qty = positions[0].quantity
        self._close_order = self.order_factory.limit(
            instrument_id=instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self._event(
            "TEST_SELL_SUBMIT",
            instrument_id=str(instrument_id),
            qty=str(qty),
            price=str(price),
        )
        self.submit_order(self._close_order, client_id=self._client_id)


async def _run_historical_phase(
    *,
    miniqmt_path: str,
    session_id: int,
    instrument_ids: tuple[str, ...],
    lookback_minutes: int,
    timeout_secs: int,
) -> None:
    client = HistoricThinkTraderClient(miniqmt_path=miniqmt_path, session_id=session_id)
    end = _clamp_to_cn_trading_end(dt.datetime.now(dt.UTC))
    start = end - dt.timedelta(minutes=lookback_minutes)
    print(f"[HIST] range (UTC): {start.isoformat()} -> {end.isoformat()}")
    bars = await client.request_bars(
        bar_specifications=["1-MINUTE-LAST"],
        start_date_time=start,
        end_date_time=end,
        tz_name="UTC",
        instrument_ids=list(instrument_ids),
        timeout=timeout_secs,
    )
    print(f"[HIST] bars: {len(bars)}")
    if bars:
        print(f"[HIST] bars ts_event range: {bars[0].ts_event} -> {bars[-1].ts_event}")

    quotes = await client.request_ticks(
        tick_type="BID_ASK",
        start_date_time=start,
        end_date_time=end,
        tz_name="UTC",
        instrument_ids=list(instrument_ids),
        limit=1000,
        timeout=timeout_secs,
    )
    print(f"[HIST] quote ticks: {len(quotes)}")
    if quotes:
        print(f"[HIST] quote ticks ts_event range: {quotes[0].ts_event} -> {quotes[-1].ts_event}")

    trades = await client.request_ticks(
        tick_type="TRADES",
        start_date_time=start,
        end_date_time=end,
        tz_name="UTC",
        instrument_ids=list(instrument_ids),
        limit=1000,
        timeout=timeout_secs,
    )
    print(f"[HIST] trade ticks: {len(trades)}")
    if trades:
        print(f"[HIST] trade ticks ts_event range: {trades[0].ts_event} -> {trades[-1].ts_event}")


def main() -> None:
    _load_dotenv()

    global _LOG_GUARD
    if _LOG_GUARD is None and not is_logging_initialized():
        _LOG_GUARD = init_logging(
            level_stdout=log_level_from_str(os.environ.get("LOG_LEVEL", "INFO"))
        )

    parser = argparse.ArgumentParser()
    parser.add_argument("--run-historical", action="store_true", default=False)
    parser.add_argument("--run-live", action="store_true", default=False)
    args = parser.parse_args()

    miniqmt_path = os.environ.get(
        "MINIQMT_PATH",
        r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini",
    )
    account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "")
    account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")

    instrument_ids = _parse_instrument_ids(os.environ.get("TT_TEST_INSTRUMENT_IDS"))
    subscribe_whole_quote = _parse_bool(os.environ.get("TT_SUBSCRIBE_WHOLE_QUOTE"), default=False)
    enable_trading = _parse_bool(os.environ.get("TT_ENABLE_TRADING"), default=bool(account_id))

    run_historical = args.run_historical or _parse_bool(
        os.environ.get("TT_RUN_HISTORICAL"), default=True
    )
    run_live = args.run_live or _parse_bool(os.environ.get("TT_RUN_LIVE"), default=True)

    session_id = 100_000 + secrets.randbelow(900_000)

    if run_historical:
        asyncio.run(
            _run_historical_phase(
                miniqmt_path=miniqmt_path,
                session_id=session_id,
                instrument_ids=instrument_ids,
                lookback_minutes=int(os.environ.get("TT_HIST_LOOKBACK_MIN", "30")),
                timeout_secs=int(os.environ.get("TT_HIST_TIMEOUT_SECS", "60")),
            ),
        )

    if not run_live:
        return

    instrument_provider = ThinkTraderInstrumentProviderConfig(
        load_all=False,
        load_ids=frozenset(InstrumentId.from_str(x) for x in instrument_ids),
    )

    exec_clients: dict[str, LiveExecClientConfig] = {}
    if account_id:
        exec_clients = {
            TT: ThinkTraderExecClientConfig(
                miniqmt_path=miniqmt_path,
                account_id=account_id,
                account_type=account_type,
                session_id=session_id,
                instrument_provider=instrument_provider,
                routing=RoutingConfig(default=True),
            ),
        }

    config_node = TradingNodeConfig(
        trader_id=TraderId(os.environ.get("TRADER_ID", "TT-COMP-TEST-001")),
        logging=LoggingConfig(log_level=os.environ.get("LOG_LEVEL", "INFO")),
        data_clients={
            TT: ThinkTraderDataClientConfig(
                miniqmt_path=miniqmt_path,
                session_id=session_id,
                instrument_provider=instrument_provider,
                subscribe_whole_quote=subscribe_whole_quote,
            ),
        },
        exec_clients=exec_clients,
        data_engine=LiveDataEngineConfig(validate_data_sequence=True),
        timeout_connection=90.0,
        timeout_reconciliation=10.0,
        timeout_portfolio=10.0,
        timeout_disconnection=5.0,
        timeout_post_stop=2.0,
    )

    node = TradingNode(config=config_node)

    strategy = ThinkTraderComprehensiveStrategy(
        ThinkTraderComprehensiveConfig(
            instrument_ids=instrument_ids,
            account_id=account_id,
            enable_trading=enable_trading,
            subscribe_trade_ticks=_parse_bool(
                os.environ.get("TT_SUBSCRIBE_TRADE_TICKS"),
                default=False,
            ),
            subscribe_order_book=_parse_bool(
                os.environ.get("TT_SUBSCRIBE_ORDER_BOOK"),
                default=False,
            ),
            verbose_events=_parse_bool(os.environ.get("TT_VERBOSE_EVENTS"), default=True),
            tick_print_every=int(os.environ.get("TT_TICK_PRINT_EVERY", "50")),
            stop_after_secs=int(os.environ.get("TT_STOP_AFTER_SECS", "180")),
            test_trade_qty=int(os.environ.get("TT_TEST_TRADE_QTY", "100")),
        ),
    )
    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    if exec_clients:
        node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    node.build()

    try:
        live_run_secs = int(
            os.environ.get(
                "TT_LIVE_RUN_SECS",
                "0" if enable_trading else "10",
            ),
        )
        if live_run_secs > 0:
            node.kernel.loop.call_later(live_run_secs, node.stop)
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()


if __name__ == "__main__":
    main()
