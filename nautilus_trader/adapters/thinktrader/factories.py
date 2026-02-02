from nautilus_trader.live.factories import LiveDataClientFactory, LiveExecClientFactory
from nautilus_trader.common.component import Logger

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.execution import ThinkTraderExecutionClient
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider
from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderExecClientConfig,
)

class ThinkTraderLiveDataClientFactory(LiveDataClientFactory):
    @staticmethod
    def create(
        loop,
        name,
        config: ThinkTraderDataClientConfig,
        msgbus,
        cache,
        clock,
    ) -> ThinkTraderDataClient:
        logger = Logger(name, clock=clock)
        # Assuming account_id is not strictly needed for DataClient only but client needs it?
        # Actually standard ThinkTraderClient requires account_id.
        # But DataClientConfig doesn't seem to have account_id in my definion? 
        # Let's check config.py.
        # DataClientConfig: miniqmt_path, session_id. No account_id.
        # But ThinkTraderClient requires account_id in __init__.
        # This implies we might need a dummy account ID for DataClient if it only does market data?
        # Or we should add account_id to DataClientConfig.
        # XtQuant `subscribe_quote` does not technically require a trading account login, BUT `XtQuantTrader` does.
        # If we use `xtdata` directly we don't need `XtQuantTrader` instance.
        # However, my design in `client.py` wraps `XtQuantTrader`.
        # WE SHOULD USE A DUMMY ACCOUNT OR UPDATE CONFIG.
        # Let's check if `xtdata` works without `XtQuantTrader`. Yes it does.
        # But my `ThinkTraderClient` mixes them.
        # For simplicity, if we use `ThinkTraderClient`, we need an account.
        # I'll default to empty string or expect it in config.
        # I'll update config to valid.
        
        # ACTUALLY: For pure market data, we should probably not initialize the Trader part if possible, 
        # OR just require an account ID. 
        # Let's pass a dummy for now as `xtdata` is separate.
        # Wait, `ThinkTraderClient` allows `xtdata` calls via `MarketDataMixin` which uses `xtdata` module directly, 
        # NOT `self._trader`.
        # So `_trader` is only needed for account/order mixins.
        # So providing a dummy account_id is fine for DataClient.
        
        client = ThinkTraderClient(
            loop=loop,
            logger=logger,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id="", # Dummy for data-only
        )
        
        # Instantiate provider if config exists
        provider = None
        if config.instrument_provider:
            provider = ThinkTraderInstrumentProvider(
                client=client,
                config=config.instrument_provider,
            )
            
        return ThinkTraderDataClient(
            loop=loop,
            client=client,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
        )


class ThinkTraderLiveExecClientFactory(LiveExecClientFactory):
    @staticmethod
    def create(
        loop,
        name,
        config: ThinkTraderExecClientConfig,
        msgbus,
        cache,
        clock,
    ) -> ThinkTraderExecutionClient:
        logger = Logger(name, clock=clock)
        
        client = ThinkTraderClient(
            loop=loop,
            logger=logger,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id=config.account_id,
        )
        
        provider = None
        if config.instrument_provider:
            provider = ThinkTraderInstrumentProvider(
                client=client,
                config=config.instrument_provider,
            )
            
        return ThinkTraderExecutionClient(
            loop=loop,
            client=client,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
        )
