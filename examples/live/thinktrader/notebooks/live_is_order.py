import os
import random
import warnings
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
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.trading.strategy import Strategy
from nautilus_trader.examples.algorithms.is_algo import ISExecAlgorithm
from nautilus_trader.examples.algorithms.is_algo import ISExecAlgorithmConfig

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

class ISOrderStrategyConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    trade_size: Decimal = Decimal(1000)
    horizon_secs: float = 60.0
    interval_secs: float = 10.0
    urgency: float = 0.5  # 紧迫度 (0.0 ~ 1.0)


class ISOrderStrategy(Strategy):
    """
    单次 IS (实现缺口) 下单的简单测试策略
    在收到该股票的第一个 QuoteTick 后，发起一个 IS 买单。
    """

    def __init__(self, config: ISOrderStrategyConfig) -> None:
        super().__init__(config)

        self.instrument_id = config.instrument_id
        self.trade_size = config.trade_size
        self.horizon_secs = config.horizon_secs
        self.interval_secs = config.interval_secs
        self.urgency = config.urgency
        
        self.instrument: Instrument | None = None
        self.order_placed = False

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.instrument_id}")
            self.stop()
            return
            
        self.log.info(f"订阅行情以触发 IS 下单: {self.instrument_id}")
        self.subscribe_quote_ticks(self.instrument_id)

    def on_quote_tick(self, tick: QuoteTick) -> None:
        if self.order_placed:
            return

        # 收到第一个 tick 时开始 IS 下单
        self.order_placed = True
        self.log.info(f"收到第一个 QuoteTick: Bid={tick.bid_price}, Ask={tick.ask_price}")
        self.place_is_order()

    def place_is_order(self) -> None:
        if not self.instrument:
            return

        # 执行参数
        exec_algorithm_id = ExecAlgorithmId("IS")
        exec_algorithm_params = {
            "horizon_secs": self.horizon_secs,
            "interval_secs": self.interval_secs,
            "urgency": self.urgency,
        }

        # 构建订单
        order = self.order_factory.market(
            instrument_id=self.instrument_id,
            order_side=OrderSide.BUY,
            quantity=self.instrument.make_qty(self.trade_size),
            time_in_force=TimeInForce.GTC,
            exec_algorithm_id=exec_algorithm_id,
            exec_algorithm_params=exec_algorithm_params,
        )

        self.log.info(
            f"正在提交 IS 订单: 标的={self.instrument_id}, 总数量={self.trade_size}, "
            f"执行周期={self.horizon_secs}秒, 分配间隔={self.interval_secs}秒, 紧迫度={self.urgency}"
        )
        self.submit_order(order)


# --- 配置部分 ---

# MiniQMT 路径与账户配置
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
session_id = random.randint(100000, 999999)

# 标的: 您刚才切成了 001330.SZSE，这里与之保持一致
instrument_id_str = "001330.SZSE"
instrument_id = InstrumentId.from_str(instrument_id_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id_str]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-IS"),
    logging=LoggingConfig(
        log_level="INFO",
        log_component_levels={"Strategy": "INFO"}
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

    strategy_config = ISOrderStrategyConfig(
        instrument_id=instrument_id,
        trade_size=Decimal(1000),      # 总数量
        horizon_secs=60.0,             # 执行周期：60秒
        interval_secs=10.0,            # 拆单与采样评估间隔：10秒
        urgency=0.5                    # 紧迫度为 0.5。数值越高，订单前置分配得越多；数值越低，越接近时间均分（TWAP）
    )

    strategy = ISOrderStrategy(config=strategy_config)
    node.trader.add_strategy(strategy)

    # 注册 IS 执行算法
    is_algo = ISExecAlgorithm(config=ISExecAlgorithmConfig())
    node.trader.add_exec_algorithm(is_algo)

    node.build()

    print(f"正在启动 IS 测试策略, 标的: {instrument_id}...")
    print(f"数据路径: {miniqmt_path}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
