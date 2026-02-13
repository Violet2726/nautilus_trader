from __future__ import annotations

import asyncio
from typing import cast

from nautilus_trader.adapters.fix.config import FixExecClientConfig
from nautilus_trader.adapters.fix.execution import FixExecutionClient
from nautilus_trader.adapters.fix.providers import FixInstrumentProvider
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.live.factories import LiveExecClientFactory


class FixLiveExecClientFactory(LiveExecClientFactory):
    @staticmethod
    def create(  # type: ignore[override]
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: FixExecClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> FixExecutionClient:
        config = cast(FixExecClientConfig, config)
        provider_config = (
            config.instrument_provider
            if isinstance(config.instrument_provider, InstrumentProviderConfig)
            else InstrumentProviderConfig()
        )
        provider = FixInstrumentProvider(cache=cache, config=provider_config)
        return FixExecutionClient(
            loop=loop,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
            name=name,
        )

