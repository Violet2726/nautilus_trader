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


THINKTRADER_CLIENTS: dict[tuple, ThinkTraderClient] = {}
THINKTRADER_INSTRUMENT_PROVIDERS: dict[tuple, ThinkTraderInstrumentProvider] = {}


def get_cached_thinktrader_client(
    loop: asyncio.AbstractEventLoop,
    miniqmt_path: str,
    session_id: int,
    account_id: str,
    account_type: str = "STOCK",
) -> ThinkTraderClient:
    global THINKTRADER_CLIENTS

    # 使用 (miniqmt_path, session_id) 作为 key，确保 DataClient 和 ExecClient 共享同一个实例
    client_key = (miniqmt_path, session_id)
    if client_key not in THINKTRADER_CLIENTS:
        logger = Logger(f"ThinkTraderClient[{session_id}:{account_id or 'DATA'}]")
        THINKTRADER_CLIENTS[client_key] = ThinkTraderClient(
            loop=loop,
            logger=logger,
            miniqmt_path=miniqmt_path,
            session_id=session_id,
            account_id=account_id,
            account_type=account_type,
        )
    else:
        # 如果已存在的 client 没有 account_id，但现在传入了，则更新
        client = THINKTRADER_CLIENTS[client_key]
        if not client._account_id and account_id:
            client._account_id = account_id
            client._account_type = account_type

    return THINKTRADER_CLIENTS[client_key]


def get_cached_thinktrader_instrument_provider(
    client: ThinkTraderClient,
    config: ThinkTraderInstrumentProviderConfig,
) -> ThinkTraderInstrumentProvider:
    global THINKTRADER_INSTRUMENT_PROVIDERS

    client_key = (client._miniqmt_path, client._session_id, client._account_id)
    provider_key = (client_key, hash(config))

    if provider_key not in THINKTRADER_INSTRUMENT_PROVIDERS:
        THINKTRADER_INSTRUMENT_PROVIDERS[provider_key] = ThinkTraderInstrumentProvider(
            client=client,
            config=config,
        )

    return THINKTRADER_INSTRUMENT_PROVIDERS[provider_key]


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
        provider_config = (
            config.instrument_provider
            if isinstance(config.instrument_provider, ThinkTraderInstrumentProviderConfig)
            else ThinkTraderInstrumentProviderConfig()
        )
        client = get_cached_thinktrader_client(
            loop=loop,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id="",
            account_type="STOCK",
        )
        provider = get_cached_thinktrader_instrument_provider(client=client, config=provider_config)

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
        provider_config = (
            config.instrument_provider
            if isinstance(config.instrument_provider, ThinkTraderInstrumentProviderConfig)
            else ThinkTraderInstrumentProviderConfig()
        )
        client = get_cached_thinktrader_client(
            loop=loop,
            miniqmt_path=config.miniqmt_path,
            session_id=config.session_id,
            account_id=config.account_id,
            account_type=config.account_type,
        )
        provider = get_cached_thinktrader_instrument_provider(client=client, config=provider_config)

        return ThinkTraderExecutionClient(
            loop=loop,
            client=client,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
        )
