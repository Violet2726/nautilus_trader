
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
from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import PositiveInt
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.indicators import RelativeStrengthIndex
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
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

class RSIReversionConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    bar_type: BarType
    trade_size: Decimal
    rsi_period: PositiveInt = 6       # RSI 周期
    rsi_oversold: Decimal = Decimal("0.30") # 超卖阈值 (0.30 对应 30)
    rsi_overbought: Decimal = Decimal("0.70") # 超买阈值 (0.70 对应 70)

class RSIReversion(Strategy):
    """
    RSI 均值回归策略 (低吸高抛)
    """

    def __init__(self, config: RSIReversionConfig) -> None:
        super().__init__(config)

        self.instrument: Instrument | None = None
        # 创建 RSI 指标
        self.rsi = RelativeStrengthIndex(config.rsi_period)

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.config.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.config.instrument_id}")
            self.stop()
            return

        # 注册指标到 Bar 数据流
        self.register_indicator_for_bars(self.config.bar_type, self.rsi)

        # 订阅实时 Bar 数据
        self.subscribe_bars(self.config.bar_type)

        # 必须订阅 QuoteTick 以供内部合成 Bar
        if "INTERNAL" in str(self.config.bar_type):
            self.subscribe_quote_ticks(self.config.instrument_id)

    def on_bar(self, bar: Bar) -> None:
        if not self.indicators_initialized():
            self.log.info(f"[{self.config.instrument_id}] Bar Close: {bar.close}, RSI({self.config.rsi_period}): {self.rsi.value:.2f}")
            return
        
        rsi_val = self.rsi.value
        self.log.info(f"[{self.config.instrument_id}] Close: {bar.close}, RSI({self.config.rsi_period}): {rsi_val:.2f}")

        # 买入逻辑 (超卖 -> 触底反弹预期 -> 买入)
        if rsi_val < float(self.config.rsi_oversold):
            is_flat = self.portfolio.is_flat(self.config.instrument_id)
            self.log.info(f"[{self.config.instrument_id}] Is Flat: {is_flat}")
            
            # 简化逻辑：只要超卖就买入 (即使已有持仓也加仓)
            self.log.info(f"[{self.config.instrument_id}] RSI OVERSOLD (< {self.config.rsi_oversold}) -> BUY (抄底/加仓)")
            if self.portfolio.is_net_short(self.config.instrument_id):
                 self.close_all_positions(self.config.instrument_id)
            self.buy(bar.close)

        # 卖出逻辑 (超买 -> 见顶回落预期 -> 卖出)
        elif rsi_val > float(self.config.rsi_overbought):
            if self.portfolio.is_net_long(self.config.instrument_id):
                self.log.info(f"[{self.config.instrument_id}] RSI OVERBOUGHT (> {self.config.rsi_overbought}) -> SELL (止盈/高抛)")
                self.close_all_positions(self.config.instrument_id)
                # self.sell(bar.close) # close_all 已经平仓了

    def buy(self, current_price: Decimal) -> None:
        if not self.instrument:
            return

        qty = self.config.trade_size
        qty = (int(qty) // 100) * 100
        if qty < 100:
            return

        # 挂限价单买入 (对手价 + 滑点)
        limit_price = current_price * Decimal("1.005") 
        
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
        qty = (int(qty) // 100) * 100
        if qty < 100:
            return

        limit_price = current_price * Decimal("0.995")

        order = self.order_factory.limit(
            instrument_id=self.config.instrument_id,
            order_side=OrderSide.SELL,
            quantity=self.instrument.make_qty(qty),
            price=self.instrument.make_price(limit_price),
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)


# --- 配置部分 ---

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
# 注意：Nautilus 会自动附加 .log 或 .json 后缀
log_file_name = f"live_rsi_reversion_multi_{timestamp_str}"

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-RSI-MULTI"),
    logging=LoggingConfig(
        log_level="INFO",
        log_level_file="INFO",
        log_directory=str(log_dir),
        log_file_name=log_file_name,
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

    for instrument_id, instrument_id_str in zip(instrument_ids, instrument_ids_str):
        bar_type_str = f"{instrument_id_str}-1-MINUTE-MID-INTERNAL" 
        bar_type = BarType.from_str(bar_type_str)
        
        strategy_config = RSIReversionConfig(
            instrument_id=instrument_id,
            bar_type=bar_type,
            trade_size=Decimal("100"),     # 每次100股
            rsi_period=6,                  # RSI 周期设置为 6 (更敏感)
            rsi_oversold=Decimal("0.30"),    # RSI < 0.30 买入
            rsi_overbought=Decimal("0.70"),  # RSI > 0.70 卖出
        )
        
        strategy = RSIReversion(config=strategy_config)
        node.trader.add_strategy(strategy)
        print(f"已添加策略: {instrument_id} (RSI Reversion, Period=6)")

    node.build()

    print(f"正在启动多标的 RSI Reversion 策略...")
    print(f"日志将输出到: {log_dir / log_file_name}.log")
    print(f"跟踪标的: {instrument_ids_str}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
