from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import LiveExecClientConfig


class FixInstrumentProviderConfig(InstrumentProviderConfig, kw_only=True, frozen=True):
    pass


class FixExecClientConfig(LiveExecClientConfig, kw_only=True, frozen=True):
    fix_settings_path: str
    fix_dictionary_path: str
    username: str
    password: str
    account_id: str
    remote_host: str | None = None
    remote_port: int | None = None
    use_tls_tunnel: bool = True
    tls_local_host: str = "127.0.0.1"
    tls_local_port: int = 16670
    instrument_provider: InstrumentProviderConfig = FixInstrumentProviderConfig()

