
import os
import random
import warnings
from datetime import datetime
from decimal import Decimal
from pathlib import Path


# Suppress annoying warnings
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import PositiveInt
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.indicators import ExponentialMovingAverage
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.trading.strategy import Strategy


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


_load_dotenv()

# --- 策略部分 ---

class EMACrossConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    bar_type: BarType
    trade_size: Decimal
    fast_ema_period: PositiveInt = 10
    slow_ema_period: PositiveInt = 20

    # 历史数据加载配置 (可选)
    request_bars: bool = False


class EMACross(Strategy):
    """
    均线交叉策略 (适配 A 股 / ThinkTrader)
    """

    def __init__(self, config: EMACrossConfig) -> None:
        PyCondition.is_true(
            config.fast_ema_period < config.slow_ema_period,
            "{config.fast_ema_period=} must be less than {config.slow_ema_period=}",
        )
        super().__init__(config)

        self.instrument: Instrument | None = None

        # 创建指标
        self.fast_ema = ExponentialMovingAverage(config.fast_ema_period)
        self.slow_ema = ExponentialMovingAverage(config.slow_ema_period)

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.config.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.config.instrument_id}")
            self.stop()
            return

        # 注册指标到 Bar 数据流
        self.register_indicator_for_bars(self.config.bar_type, self.fast_ema)
        self.register_indicator_for_bars(self.config.bar_type, self.slow_ema)

        # 订阅实时 Bar 数据 (由 Nautilus 内部合成，或者外部推送)
        self.subscribe_bars(self.config.bar_type)

        # 如果需要，订阅 QuoteTick (主要用于合成 Bar)
        # 如果 BarType 是 INTERNAL，则必须订阅 QuoteTick
        if "INTERNAL" in str(self.config.bar_type):
            self.subscribe_quote_ticks(self.config.instrument_id)

    def on_bar(self, bar: Bar) -> None:
        # self.log.info(f"Bar Received: {bar}", LogColor.CYAN)
        self.log.info(f"[{self.config.instrument_id}] Bar Close: {bar.close}, FastMA({self.fast_ema.period}): {self.fast_ema.value:.2f}, SlowMA({self.slow_ema.period}): {self.slow_ema.value:.2f}")

        # 等待指标预热
        if not self.indicators_initialized():
            return

        # 交易逻辑
        # 金叉 (Fast 上穿 Slow) -> 买入
        if self.fast_ema.value >= self.slow_ema.value:
            if self.portfolio.is_flat(self.config.instrument_id):
                self.log.info(f"[{self.config.instrument_id}] GOLDEN CROSS (金叉) -> BUY")
                self.buy(bar.close)
            elif self.portfolio.is_net_short(self.config.instrument_id):
                self.log.info(f"[{self.config.instrument_id}] GOLDEN CROSS (金叉) -> CLOSE SHORT & BUY")
                self.close_all_positions(self.config.instrument_id)
                self.buy(bar.close)

        # 死叉 (Fast 下穿 Slow) -> 卖出
        elif self.fast_ema.value < self.slow_ema.value:
            if self.portfolio.is_flat(self.config.instrument_id):
                # A 股不能做空，所以如果空仓则不动，如果是期货则可以开空
                # self.sell(bar.close)
                pass
            elif self.portfolio.is_net_long(self.config.instrument_id):
                self.log.info(f"[{self.config.instrument_id}] DEATH CROSS (死叉) -> SELL")
                self.close_all_positions(self.config.instrument_id)
                # self.sell(bar.close) # close_all 已经会平仓了

    def buy(self, current_price: Decimal) -> None:
        if not self.instrument:
            return

        qty = self.config.trade_size
        # 确保整百
        qty = (int(qty) // 100) * 100
        if qty < 100:
            return

        # 使用限价单模拟市价买入 (挂高价，例如涨停价或卖五价，这里简单处理为当前价 * 1.02)
        # 注意：这里为了安全，使用当前 Bar Close 价格 + 滑点
        limit_price = current_price * Decimal("1.01")

        order = self.order_factory.limit(
            instrument_id=self.config.instrument_id,
            order_side=OrderSide.BUY,
            quantity=self.instrument.make_qty(qty),
            price=self.instrument.make_price(limit_price),
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)

    def sell(self, current_price: Decimal) -> None:
        if not self.instrument:
            return

        qty = self.config.trade_size
        # 确保整百
        qty = (int(qty) // 100) * 100
        if qty < 100:
            return

        # 限价卖出 (挂低价)
        limit_price = current_price * Decimal("0.99")

        order = self.order_factory.limit(
            instrument_id=self.config.instrument_id,
            order_side=OrderSide.SELL,
            quantity=self.instrument.make_qty(qty),
            price=self.instrument.make_price(limit_price),
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)


# --- 配置部分 ---

# MiniQMT 路径与账户配置
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
session_id = random.randint(100000, 999999)

# 多标的列表
instrument_ids_str = ["000547.SZSE", "600916.SSE", "601808.SSE"]
instrument_ids = [InstrumentId.from_str(i) for i in instrument_ids_str]

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset(instrument_ids_str),
)

# 日志配置
timestamp_str = datetime.now().strftime("%Y%m%d_%H%M%S")
log_dir = Path("outputs")
log_dir.mkdir(exist_ok=True)
log_file_path = log_dir / f"live_ema_cross_multi_{timestamp_str}.log"

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-EMA-MULTI"),
    logging=LoggingConfig(
        log_level="INFO",
        log_level_file="INFO",
        log_directory=str(log_dir),
        log_file_name=f"live_ema_cross_multi_{timestamp_str}",
        log_component_levels={"Strategy": "INFO"},
    ),
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
    data_engine=LiveDataEngineConfig(
        validate_data_sequence=True,
    ),
    timeout_connection=90.0,
    timeout_reconciliation=5.0,
    timeout_portfolio=5.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)

# --- 启动流程 ---

if __name__ == "__main__":
    node = TradingNode(config=config_node)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    # 为每个标的添加策略实例
    for instrument_id, instrument_id_str in zip(instrument_ids, instrument_ids_str):
        # 构造 BarType: {InstrumentID}-1-MINUTE-MID-INTERNAL
        bar_type_str = f"{instrument_id_str}-1-MINUTE-MID-INTERNAL"
        bar_type = BarType.from_str(bar_type_str)

        strategy_config = EMACrossConfig(
            instrument_id=instrument_id,
            bar_type=bar_type,
            trade_size=Decimal(100),     # 每次100股
            fast_ema_period=2,             # 2周期
            slow_ema_period=5,            # 5周期
        )

        strategy = EMACross(config=strategy_config)
        node.trader.add_strategy(strategy)
        print(f"已添加策略: {instrument_id} (EMA Cross, BarType={bar_type})")

    node.build()

    print("正在启动多标的 EMA Cross 策略...")
    print(f"日志路径: {log_file_path}")
    print(f"跟踪标的: {instrument_ids_str}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
