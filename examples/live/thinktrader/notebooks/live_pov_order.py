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
from nautilus_trader.examples.algorithms.pov import POVExecAlgorithm
from nautilus_trader.examples.algorithms.pov import POVExecAlgorithmConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import ExecAlgorithmId
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

class POVOrderStrategyConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    trade_size: Decimal = Decimal(1000)
    pov_rate: float = 0.1  # 目标参与率 (0 < pov_rate <= 1)，例如 0.1 表示跟踪市场 10% 的成交量
    interval_secs: float = 10.0
    max_horizon_secs: float = 300.0


class POVOrderStrategy(Strategy):
    """
    单次 POV 下单的简单测试策略
    在收到该股票的第一个 QuoteTick 后，发起一个 POV 买单。
    """

    def __init__(self, config: POVOrderStrategyConfig) -> None:
        super().__init__(config)

        self.instrument_id = config.instrument_id
        self.trade_size = config.trade_size
        self.pov_rate = config.pov_rate
        self.interval_secs = config.interval_secs
        self.max_horizon_secs = config.max_horizon_secs
        
        self.instrument: Instrument | None = None
        self.order_placed = False

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.instrument_id}")
            self.stop()
            return
            
        self.log.info(f"订阅行情以触发 POV 下单: {self.instrument_id}")
        self.subscribe_quote_ticks(self.instrument_id)

    def on_quote_tick(self, tick: QuoteTick) -> None:
        if self.order_placed:
            return

        # 收到第一个 tick 时开始 POV 下单
        self.order_placed = True
        self.log.info(f"收到第一个 QuoteTick: Bid={tick.bid_price}, Ask={tick.ask_price}")
        self.place_pov_order()

    def place_pov_order(self) -> None:
        if not self.instrument:
            return

        # 执行参数
        exec_algorithm_id = ExecAlgorithmId("POV")
        exec_algorithm_params = {
            "pov_rate": self.pov_rate,
            "interval_secs": self.interval_secs,
            "max_horizon_secs": self.max_horizon_secs,
            "max_slice_qty": 300, # 增加防冲击限额参数
        }

        # 构建订单: 替换为限价单
        quote = self.cache.quote_tick(self.instrument_id)
        if not quote:
            self.log.error(f"无法获取合约 {self.instrument_id} 的最新盘口数据，取消发单")
            return
            
        price = quote.bid_price
        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.BUY,
            price=price,
            quantity=self.instrument.make_qty(self.trade_size),
            time_in_force=TimeInForce.GTC,
            exec_algorithm_id=exec_algorithm_id,
            exec_algorithm_params=exec_algorithm_params,
        )

        self.log.info(
            f"正在提交 POV 订单: 标的={self.instrument_id}, 数量={self.trade_size}, 限价={price}, "
            f"参与率={self.pov_rate*100}%, 间隔={self.interval_secs}秒, 最长执行时长={self.max_horizon_secs}秒"
        )
        self.submit_order(order)


# --- 配置部分 ---

# MiniQMT 路径与账户配置
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
session_id = random.randint(100000, 999999)

# 标的: 000001.SZSE
instrument_id_str = "000001.SZSE"
instrument_id = InstrumentId.from_str(instrument_id_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id_str]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-POV"),
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

    strategy_config = POVOrderStrategyConfig(
        instrument_id=instrument_id,
        trade_size=Decimal(200),      # 总数量
        pov_rate=0.1,                  # 目标跟随真实市场 10% 的成交量
        interval_secs=10.0,            # 每隔 10 秒评估一次市场成交量并下单
        max_horizon_secs=300.0         # 兜底超时时间，超过 300 秒则强制提交所有剩余数量
    )

    strategy = POVOrderStrategy(config=strategy_config)
    node.trader.add_strategy(strategy)

    # 注册 POV 执行算法
    pov_algo = POVExecAlgorithm(config=POVExecAlgorithmConfig())
    node.trader.add_exec_algorithm(pov_algo)

    node.build()

    print(f"正在启动 POV 测试策略, 标的: {instrument_id}...")
    print(f"数据路径: {miniqmt_path}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
