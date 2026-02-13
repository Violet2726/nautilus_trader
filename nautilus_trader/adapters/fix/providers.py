from __future__ import annotations

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.instruments import Instrument


class FixInstrumentProvider(InstrumentProvider):
    def __init__(
        self,
        cache: Cache,
        config: InstrumentProviderConfig | None = None,
        venue: Venue | None = None,
    ) -> None:
        super().__init__(config=config)
        self._cache = cache
        self._venue = venue

    async def load_all_async(self, filters: dict | None = None) -> None:
        instruments: list[Instrument] = self._cache.instruments(self._venue)
        for instrument in instruments:
            self.add(instrument)

    async def load_ids_async(
        self,
        instrument_ids: list[InstrumentId],
        filters: dict | None = None,
    ) -> None:
        PyCondition.not_none(instrument_ids, "instrument_ids")
        for instrument_id in instrument_ids:
            await self.load_async(instrument_id, filters)

    async def load_async(
        self,
        instrument_id: InstrumentId,
        filters: dict | None = None,
    ) -> None:
        PyCondition.not_none(instrument_id, "instrument_id")
        instrument: Instrument | None = self._cache.instrument(instrument_id)
        if instrument is None and self._cache.has_backing:
            instrument = self._cache.load_instrument(instrument_id)
        if instrument is None:
            return
        self.add(instrument)
