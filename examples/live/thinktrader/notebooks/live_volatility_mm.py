
import os
import random
import warnings
from decimal import Decimal
from pathlib import Path


# Suppress annoying warnings from dependencies
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.examples.strategies.volatility_market_maker import VolatilityMarketMaker
from nautilus_trader.examples.strategies.volatility_market_maker import VolatilityMarketMakerConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import BarType
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId


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

from nautilus_trader.adapters.thinktrader.common import TT


# --- 配置部分 ---

# MiniQMT 路径与账户配置
# 请根据实际环境修改以下路径和账户ID
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"  # 股票账户
session_id = random.randint(100000, 999999) # 随机 Session ID 防止冲突

# 标的配置: 601808 (中海油服)
# 注意：Nautilus 中通常使用 .SSE (上海) 和 .SZSE (深圳) 作为后缀
instrument_id_str = "601808.SSE"
instrument_id = InstrumentId.from_str(instrument_id_str)

# 确保只加载我们需要的标的，加快启动速度
instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id_str]),
)

# 交易节点配置
config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-MM-001"),
    logging=LoggingConfig(
        log_level="INFO",
        log_component_levels={"Strategy": "INFO"} # 重点关注策略日志
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
        # 如果需要由 Tick 生成 Bar，DataEngine 会自动处理 INTERNAL 类型的 BarType
    ),
    timeout_connection=90.0,
    timeout_reconciliation=5.0,
    timeout_portfolio=5.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)

# --- 策略配置 ---

# 定义 Bar 类型：1分钟 Bar，由 Nautilus 内部根据 Tick 合成 (INTERNAL)
# 这样即使 Adapter 只提供 Tick，策略也能收到 Bar 数据用于计算 ATR
bar_type = BarType.from_str(f"{instrument_id_str}-1-MINUTE-MID-INTERNAL")

strategy_config = VolatilityMarketMakerConfig(
    instrument_id=instrument_id,
    bar_type=bar_type,
    atr_period=14,              # ATR 周期
    atr_multiple=2.0,           # 挂单距离 ATR 的倍数
    trade_size=Decimal(100),  # 交易数量 (A股通常为100股一手)
    client_id=None,
    emulation_trigger="NO_TRIGGER", # 实盘通常不用模拟触发，除非在测试 Testnet
    reduce_only_on_stop=True,
)

# --- 启动流程 ---

if __name__ == "__main__":
    # 创建节点
    node = TradingNode(config=config_node)

    # 实例化策略
    strategy = VolatilityMarketMaker(config=strategy_config)

    # 添加策略到节点
    node.trader.add_strategy(strategy)

    # 注册适配器工厂
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    # 构建节点
    node.build()

    print(f"正在启动策略: {strategy_config.instrument_id}...")
    print(f"数据路径: {miniqmt_path}")
    print(f"账户 ID: {account_id}")

    try:
        # 运行节点（阻塞直到停止）
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
