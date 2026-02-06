import asyncio
import math
import os
import secrets
import warnings
from collections import deque
from datetime import datetime
from datetime import timedelta
from pathlib import Path
from zoneinfo import ZoneInfo

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient
from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.config import BacktestEngineConfig
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.currencies import CNY
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.events import OrderCanceled
from nautilus_trader.model.events import OrderExpired
from nautilus_trader.model.events import OrderFilled
from nautilus_trader.model.events import OrderRejected
from nautilus_trader.model.events import PositionClosed
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.objects import Money
from nautilus_trader.trading.strategy import Strategy


warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)


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


class TickScalperMarketConfig(StrategyConfig, frozen=True):
    # 交易标的
    instrument_id: InstrumentId

    # 下单与节奏控制
    trade_qty: int = 100
    cooldown_seconds: float = 10.0
    max_position_qty: int = 100
    max_trades_per_session: int = 50

    # 动量与波动率信号参数
    momentum_window: int = 30
    momentum_bps: float = 2.0
    volatility_window: int = 30
    min_volatility_bps: float = 1.0

    # 止盈止损与持仓限制
    take_profit_bps: float = 8.0
    stop_loss_bps: float = 6.0
    max_hold_seconds: float = 180.0

    # 风险控制
    max_session_loss: float = 500.0
    disable_trading_on_loss: bool = True
    close_on_stop: bool = True


class TickScalperMarketStrategy(Strategy):
    def __init__(self, config: TickScalperMarketConfig):
        super().__init__(config)
        # 基础配置
        self.instrument_id = config.instrument_id
        self.trade_qty = config.trade_qty
        self.cooldown_ns = int(config.cooldown_seconds * 1_000_000_000)
        self.max_position_qty = config.max_position_qty
        self.max_trades_per_session = config.max_trades_per_session

        # 信号参数
        self.momentum_window = config.momentum_window
        self.momentum_bps = config.momentum_bps
        self.volatility_window = config.volatility_window
        self.min_volatility_bps = config.min_volatility_bps

        # 止盈止损参数
        self.take_profit_bps = config.take_profit_bps
        self.stop_loss_bps = config.stop_loss_bps
        self.max_hold_ns = int(config.max_hold_seconds * 1_000_000_000)

        # 风控参数
        self.max_session_loss = config.max_session_loss
        self.disable_trading_on_loss = config.disable_trading_on_loss
        self.close_on_stop = config.close_on_stop

        # 内部状态
        self._mid_prices: deque[float] = deque(
            maxlen=max(self.momentum_window, self.volatility_window) + 1,
        )
        self._returns_bps: deque[float] = deque(maxlen=self.volatility_window)
        self._last_action_ns: int = 0
        self._entry_price: float | None = None
        self._entry_ts_ns: int | None = None
        self._pending_order = False
        self._position_open = False
        self._trade_count = 0
        self._session_realized_pnl = 0.0
        self._trading_enabled = True
        self._instrument = None

    def on_start(self) -> None:
        # 订阅实时行情, 并缓存 instrument 以便后续生成订单
        self.subscribe_quote_ticks(self.instrument_id)
        self._instrument = self.cache.instrument(self.instrument_id)
        self.log.info(f"Subscribed to {self.instrument_id}")

    def on_stop(self) -> None:
        # 取消订阅并清理未完成订单
        self.unsubscribe_quote_ticks(self.instrument_id)
        self.cancel_all_orders(self.instrument_id)
        if self.close_on_stop:
            self.close_all_positions(self.instrument_id)
        self.log.info(f"Unsubscribed from {self.instrument_id}")

    def on_quote_tick(self, tick: QuoteTick) -> None:
        # 只处理目标标的
        if tick.instrument_id != self.instrument_id:
            return

        # 有未完成订单时先等待, 避免重复下单
        if self._pending_order:
            return

        # 计算 mid price
        bid = tick.bid_price.as_double()
        ask = tick.ask_price.as_double()
        if bid <= 0 or ask <= 0:
            return

        mid = (bid + ask) * 0.5
        self._update_signal_buffer(mid)

        # 如果已有持仓, 优先检查止盈止损与时间止损
        if self._position_open:
            self._check_exit_conditions(mid)
            return

        now_ns = self.clock.timestamp_ns()
        if now_ns - self._last_action_ns < self.cooldown_ns:
            return

        if not self._trading_enabled:
            return

        if self._trade_count >= self.max_trades_per_session:
            return

        # 信号不足时直接跳过
        signal = self._compute_signal()
        if signal is None:
            return

        # 动量与波动率同时满足阈值时开仓
        if signal["momentum_bps"] >= self.momentum_bps and signal["volatility_bps"] >= self.min_volatility_bps:
            self._submit_market_order(OrderSide.BUY, self.trade_qty)
            return

    def on_order_filled(self, event: OrderFilled) -> None:
        # 订单成交后更新状态
        self._pending_order = False

        if event.order_side == OrderSide.BUY:
            self._position_open = True
            self._last_action_ns = self.clock.timestamp_ns()
            self._entry_price = event.last_px.as_double()
            self._entry_ts_ns = self._last_action_ns
            return

        self._position_open = False
        self._entry_price = None
        self._entry_ts_ns = None
        self._last_action_ns = self.clock.timestamp_ns()
        self._trade_count += 1

    def on_order_rejected(self, event: OrderRejected) -> None:
        # 拒单后允许重新尝试
        self._pending_order = False
        self.log.error(f"Order rejected: {event}")

    def on_order_canceled(self, event: OrderCanceled) -> None:
        # 取消后允许重新尝试
        self._pending_order = False
        self.log.warning(f"Order canceled: {event}")

    def on_order_expired(self, event: OrderExpired) -> None:
        # 订单过期视为取消
        self._pending_order = False
        self.log.warning(f"Order expired: {event}")

    def on_position_closed(self, event: PositionClosed) -> None:
        # 累计已实现盈亏, 用于风控
        realized_money = event.realized_pnl
        realized = realized_money.as_double() if realized_money is not None else 0.0
        self._session_realized_pnl += realized
        if self.disable_trading_on_loss and self._session_realized_pnl <= -self.max_session_loss:
            self._trading_enabled = False

    def _submit_market_order(self, side: OrderSide, qty: int) -> None:
        # 生成市价单并提交
        if self._instrument is None:
            return
        use_qty = min(qty, self.max_position_qty)
        quantity = self._instrument.make_qty(use_qty)
        order = self.order_factory.market(
            instrument_id=self.instrument_id,
            order_side=side,
            quantity=quantity,
        )
        self.submit_order(order)
        self._pending_order = True
        self._last_action_ns = self.clock.timestamp_ns()

    def _close_position(self) -> None:
        # 使用市价单平仓
        if not self._position_open or self._pending_order:
            return

        position = None
        positions = self.cache.positions_open(instrument_id=self.instrument_id)
        if positions:
            position = positions[0]

        if position is not None:
            quantity = position.quantity
        else:
            if self._instrument is None:
                return
            quantity = self._instrument.make_qty(self.trade_qty)

        order = self.order_factory.market(
            instrument_id=self.instrument_id,
            order_side=OrderSide.SELL,
            quantity=quantity,
        )
        self.submit_order(order)
        self._pending_order = True
        self._last_action_ns = self.clock.timestamp_ns()

    def _update_signal_buffer(self, mid: float) -> None:
        # 更新 mid 缓存与收益率序列, 用于动量与波动率计算
        if self._mid_prices:
            prev = self._mid_prices[-1]
            if prev > 0:
                ret_bps = (mid - prev) / prev * 10_000.0
                self._returns_bps.append(ret_bps)
        self._mid_prices.append(mid)

    def _compute_signal(self) -> dict[str, float] | None:
        # 只有在缓存足够时才计算信号
        if len(self._mid_prices) <= self.momentum_window or len(self._returns_bps) < self.volatility_window:
            return None

        now_mid = self._mid_prices[-1]
        past_mid = self._mid_prices[-self.momentum_window - 1]
        if past_mid <= 0:
            return None

        momentum_bps = (now_mid - past_mid) / past_mid * 10_000.0
        mean_ret = sum(self._returns_bps) / len(self._returns_bps)
        var = sum((x - mean_ret) ** 2 for x in self._returns_bps) / len(self._returns_bps)
        volatility_bps = math.sqrt(var)

        return {
            "momentum_bps": momentum_bps,
            "volatility_bps": volatility_bps,
        }

    def _check_exit_conditions(self, mid: float) -> None:
        # 根据止盈止损与持仓时间判断是否平仓
        if self._entry_price is None or self._entry_ts_ns is None:
            return

        profit_target = self._entry_price * (1 + self.take_profit_bps / 10_000.0)
        stop_target = self._entry_price * (1 - self.stop_loss_bps / 10_000.0)

        if mid >= profit_target:
            self._close_position()
            return

        if mid <= stop_target:
            self._close_position()
            return

        if self.max_hold_ns > 0:
            now_ns = self.clock.timestamp_ns()
            if now_ns - self._entry_ts_ns >= self.max_hold_ns:
                self._close_position()
                return

        if self.disable_trading_on_loss and self._session_realized_pnl <= -self.max_session_loss:
            self._close_position()


def _parse_dt(value: str, tz_name: str) -> datetime:
    # 支持 ISO8601 格式, 如果无时区则使用指定时区
    parsed = datetime.fromisoformat(value)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=ZoneInfo(tz_name))
    return parsed


async def _load_backtest_data(
    miniqmt_path: str,
    session_id: int,
    instrument_id: InstrumentId,
    start_dt: datetime,
    end_dt: datetime,
    tz_name: str,
) -> tuple[object, list]:
    # 使用 ThinkTrader 历史客户端拉取 QuoteTick 数据
    client = HistoricThinkTraderClient(
        miniqmt_path=miniqmt_path,
        session_id=session_id,
        log_level="INFO",
        instrument_provider_config=ThinkTraderInstrumentProviderConfig(load_contracts_on_start=True),
    )
    instruments = await client.request_instruments([instrument_id])
    if not instruments:
        raise RuntimeError("未能获取标的定义, 请检查 instrument_id 是否正确")
    ticks = await client.request_ticks(
        tick_type="BID_ASK",
        start_date_time=start_dt,
        end_date_time=end_dt,
        tz_name=tz_name,
        instrument_ids=[instrument_id],
    )
    return instruments[0], ticks


_load_dotenv()

miniqmt_path = os.environ.get(
    "MINIQMT_PATH",
    r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini",
)
session_id = int(os.environ.get("MINIQMT_SESSION_ID", str(secrets.randbelow(900000) + 100000)))
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")

instrument_id = InstrumentId.from_str(
    os.environ.get("XT_LIVE_INSTRUMENT_ID", "601005.SHSE"),
)

trade_qty = int(os.environ.get("TT_TRADE_QTY", "100"))
cooldown_seconds = float(os.environ.get("TT_COOLDOWN_SECS", "10"))
max_position_qty = int(os.environ.get("TT_MAX_POSITION_QTY", "100"))
max_trades = int(os.environ.get("TT_MAX_TRADES", "50"))
momentum_window = int(os.environ.get("TT_MOMENTUM_WINDOW", "30"))
momentum_bps = float(os.environ.get("TT_MOMENTUM_BPS", "2.0"))
vol_window = int(os.environ.get("TT_VOL_WINDOW", "30"))
min_vol_bps = float(os.environ.get("TT_MIN_VOL_BPS", "1.0"))
take_profit_bps = float(os.environ.get("TT_TP_BPS", "8.0"))
stop_loss_bps = float(os.environ.get("TT_SL_BPS", "6.0"))
max_hold_seconds = float(os.environ.get("TT_MAX_HOLD_SECS", "180"))
max_session_loss = float(os.environ.get("TT_MAX_SESSION_LOSS", "500"))
disable_on_loss = os.environ.get("TT_DISABLE_ON_LOSS", "true").lower() == "true"
close_on_stop = os.environ.get("TT_CLOSE_ON_STOP", "true").lower() == "true"
run_mode = os.environ.get("TT_RUN_MODE", "live").lower()

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TICK-SCALPER-001"),
    logging=LoggingConfig(log_level="INFO"),
    data_clients={
        TT: ThinkTraderDataClientConfig(
            miniqmt_path=miniqmt_path,
            session_id=session_id,
            instrument_provider=instrument_provider,
        ),
    },
    exec_clients={
        TT: ThinkTraderExecClientConfig(
            miniqmt_path=miniqmt_path,
            account_id=account_id,
            account_type=account_type,
            session_id=session_id,
            instrument_provider=instrument_provider,
            routing=RoutingConfig(default=True),
        ),
    },
    data_engine=LiveDataEngineConfig(validate_data_sequence=True),
    timeout_connection=90.0,
    timeout_reconciliation=5.0,
    timeout_portfolio=5.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)


if __name__ == "__main__":
    strategy_config = TickScalperMarketConfig(
        instrument_id=instrument_id,
        trade_qty=trade_qty,
        cooldown_seconds=cooldown_seconds,
        max_position_qty=max_position_qty,
        max_trades_per_session=max_trades,
        momentum_window=momentum_window,
        momentum_bps=momentum_bps,
        volatility_window=vol_window,
        min_volatility_bps=min_vol_bps,
        take_profit_bps=take_profit_bps,
        stop_loss_bps=stop_loss_bps,
        max_hold_seconds=max_hold_seconds,
        max_session_loss=max_session_loss,
        disable_trading_on_loss=disable_on_loss,
        close_on_stop=close_on_stop,
    )
    strategy = TickScalperMarketStrategy(config=strategy_config)

    if run_mode == "backtest":
        tz_name = os.environ.get("TT_BACKTEST_TZ", "Asia/Shanghai")
        end_value = os.environ.get("TT_BACKTEST_END")
        start_value = os.environ.get("TT_BACKTEST_START")
        if end_value is None:
            end_dt = datetime.now(tz=ZoneInfo(tz_name))
        else:
            end_dt = _parse_dt(end_value, tz_name)
        if start_value is None:
            start_dt = end_dt - timedelta(minutes=30)
        else:
            start_dt = _parse_dt(start_value, tz_name)

        instrument, ticks = asyncio.run(
            _load_backtest_data(
                miniqmt_path=miniqmt_path,
                session_id=session_id,
                instrument_id=instrument_id,
                start_dt=start_dt,
                end_dt=end_dt,
                tz_name=tz_name,
            ),
        )

        engine_config = BacktestEngineConfig(
            trader_id=TraderId("TICK-SCALPER-BT-001"),
            logging=LoggingConfig(log_level="INFO"),
        )
        engine = BacktestEngine(config=engine_config)
        engine.add_venue(
            venue=Venue("SIM"),
            oms_type=OmsType.NETTING,
            account_type=AccountType.CASH,
            base_currency=CNY,
            starting_balances=[Money(1_000_000, CNY)],
        )
        engine.add_instrument(instrument)
        engine.add_data(ticks)
        engine.add_strategy(strategy)

        engine.run()
        engine.dispose()
    else:
        node = TradingNode(config=config_node)
        node.trader.add_strategy(strategy)
        node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
        node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
        node.build()

        try:
            node.run()
        except KeyboardInterrupt:
            node.kernel.logger.info("用户停止运行 (Ctrl+C)")
        finally:
            node.dispose()
