import asyncio
from typing import cast

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.execution import ThinkTraderExecutionClient
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import MessageBus
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.config import LiveExecClientConfig
from nautilus_trader.live.factories import LiveDataClientFactory
from nautilus_trader.live.factories import LiveExecClientFactory


class ThinkTraderLiveDataClientFactory(LiveDataClientFactory):
    @staticmethod
    def create(
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: LiveDataClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> ThinkTraderDataClient:
        config = cast(ThinkTraderDataClientConfig, config)
        logger = Logger(name)
        client = ThinkTraderClient(
            loop=loop,
            logger=logger,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id="",
        )
        provider_config = (
            config.instrument_provider
            if isinstance(config.instrument_provider, ThinkTraderInstrumentProviderConfig)
            else ThinkTraderInstrumentProviderConfig()
        )
        provider = ThinkTraderInstrumentProvider(
            client=client,
            config=provider_config,
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
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: LiveExecClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> ThinkTraderExecutionClient:
        config = cast(ThinkTraderExecClientConfig, config)
        logger = Logger(name)

        client = ThinkTraderClient(
            loop=loop,
            logger=logger,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id=config.account_id,
        )

        provider_config = (
            config.instrument_provider
            if isinstance(config.instrument_provider, ThinkTraderInstrumentProviderConfig)
            else ThinkTraderInstrumentProviderConfig()
        )
        provider = ThinkTraderInstrumentProvider(
            client=client,
            config=provider_config,
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
