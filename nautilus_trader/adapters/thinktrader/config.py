from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.config import LiveExecClientConfig


class ThinkTraderInstrumentProviderConfig(InstrumentProviderConfig, kw_only=True, frozen=True):
    """ThinkTrader 工具提供者配置"""

    load_contracts_on_start: bool = True
    cache_instruments: bool = True
    sectors: tuple[str, ...] = ("沪深A股",)  # 要加载的板块列表
    filter_expiry: bool = False  # 是否过滤已过期合约


class ThinkTraderDataClientConfig(LiveDataClientConfig, kw_only=True, frozen=True):
    """ThinkTrader 数据客户端配置"""

    miniqmt_path: str  # MiniQmt userdata 路径
    session_id: int = 123456  # 会话 ID
    instrument_provider: InstrumentProviderConfig = ThinkTraderInstrumentProviderConfig()
    subscribe_whole_quote: bool = False  # 是否使用全推行情
    subscription_delay_secs: float = 0.1  # 订阅间隔 (避免过快)
    skip_trader_login: bool = False  # 是否跳过交易端登录 (仅使用行情)


class ThinkTraderExecClientConfig(LiveExecClientConfig, kw_only=True, frozen=True):
    """ThinkTrader 执行客户端配置"""

    miniqmt_path: str
    account_id: str  # 资金账号
    account_type: str = "STOCK"  # 账号类型: STOCK, CREDIT, FUTURE, OPTION
    session_id: int = 123456  # 会话 ID
    use_async_order: bool = True  # 是否使用异步下单
    relaxed_response_order: bool = True  # 开启宽松时序模式
    instrument_provider: InstrumentProviderConfig = ThinkTraderInstrumentProviderConfig()
