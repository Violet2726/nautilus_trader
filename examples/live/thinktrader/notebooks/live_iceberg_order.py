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
from nautilus_trader.examples.algorithms.iceberg import IcebergExecAlgorithm
from nautilus_trader.examples.algorithms.iceberg import IcebergExecAlgorithmConfig
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

class IcebergOrderStrategyConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    trade_size: Decimal = Decimal(1000)
    slice_qty: Decimal = Decimal(100) # 每次拆单露出的切片数量
    randomize_pct: float = 0.0        # 切片数量的随机化浮动百分比 (0.0 ~ 0.5)


class IcebergOrderStrategy(Strategy):
    """
    单次 Iceberg (冰山) 下单的简单测试策略
    在收到该股票的第一个 QuoteTick 后，发起一个冰山买单。
    """

    def __init__(self, config: IcebergOrderStrategyConfig) -> None:
        super().__init__(config)

        self.instrument_id = config.instrument_id
        self.trade_size = config.trade_size
        self.slice_qty = config.slice_qty
        self.randomize_pct = config.randomize_pct
        
        self.instrument: Instrument | None = None
        self.order_placed = False

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.instrument_id}")
            self.stop()
            return
            
        self.log.info(f"订阅行情以触发 Iceberg 下单: {self.instrument_id}")
        self.subscribe_quote_ticks(self.instrument_id)

    def on_quote_tick(self, tick: QuoteTick) -> None:
        if self.order_placed:
            return

        # 收到第一个 tick 时开始 Iceberg 下单
        self.order_placed = True
        self.log.info(f"收到第一个 QuoteTick: Bid={tick.bid_price}, Ask={tick.ask_price}")
        self.place_iceberg_order()

    def place_iceberg_order(self) -> None:
        if not self.instrument:
            return

        # 执行参数
        exec_algorithm_id = ExecAlgorithmId("Iceberg")
        exec_algorithm_params = {
            "slice_qty": float(self.slice_qty),
            "randomize_pct": self.randomize_pct,
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
            f"正在提交 Iceberg 订单: 标的={self.instrument_id}, 总数量={self.trade_size}, "
            f"单次切片数量={self.slice_qty}, 随机浮动={self.randomize_pct*100}%"
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
    trader_id=TraderId("TESTER-ICEBERG"),
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

    strategy_config = IcebergOrderStrategyConfig(
        instrument_id=instrument_id,
        trade_size=Decimal(1000),      # 总数量 1000
        slice_qty=Decimal(200),        # 每次向市场抛出 200 数量进行成交
        randomize_pct=0.0              # 是否随机化切面大小，这里为简单起见先使用精确值 0.0
    )

    strategy = IcebergOrderStrategy(config=strategy_config)
    node.trader.add_strategy(strategy)

    # 注册 Iceberg 执行算法
    iceberg_algo = IcebergExecAlgorithm(config=IcebergExecAlgorithmConfig())
    node.trader.add_exec_algorithm(iceberg_algo)

    node.build()

    print(f"正在启动 Iceberg 测试策略, 标的: {instrument_id}...")
    print(f"数据路径: {miniqmt_path}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
