import asyncio

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.parsing.instruments import instrument_id_to_stock_code
from nautilus_trader.adapters.thinktrader.parsing.instruments import parse_equity
from nautilus_trader.adapters.thinktrader.parsing.instruments import parse_future
from nautilus_trader.adapters.thinktrader.parsing.instruments import parse_option
from nautilus_trader.adapters.thinktrader.parsing.instruments import stock_code_to_instrument_id
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument


class ThinkTraderInstrumentProvider(InstrumentProvider):
    """ThinkTrader 工具提供者"""

    def __init__(
        self,
        client: ThinkTraderClient,
        config: ThinkTraderInstrumentProviderConfig,
    ) -> None:
        super().__init__(config=config)
        self._client = client
        self._tt_config = config

    async def initialize(self, reload: bool = False) -> None:
        await super().initialize(reload)
        if self._load_all_on_start or self._load_ids_on_start:
            return

        if self._loaded and not reload:
            return

        if not self._tt_config.load_contracts_on_start:
            return

        if reload:
            self._loaded = False

        await self.load_all_async(self._filters)
        self._loaded = True

    async def load_all_async(self, filters: dict | None = None) -> None:
        """异步加载所有工具"""
        if self._loaded and self._tt_config.cache_instruments:
            return

        for sector in self._tt_config.sectors:
            await self._load_sector(sector)

        self._loaded = True
        self._log.info(f"已加载 {len(self._instruments)} 个工具")

    async def _load_sector(self, sector: str) -> None:
        """加载板块内的工具"""
        stock_codes = self._client.get_stock_list(sector)

        for stock_code in stock_codes:
            try:
                detail = self._client.get_instrument_detail(stock_code)
                if detail is None:
                    continue

                instrument = self._parse_instrument(stock_code, detail)
                if instrument:
                    self.add(instrument)
            except Exception as e:
                self._log.warning(f"加载工具失败: {stock_code}, 错误: {e}")

            # 避免请求过快
            await asyncio.sleep(0.01)

    def _parse_instrument(self, stock_code: str, detail: dict) -> Instrument | None:
        """解析工具"""
        instrument_id = stock_code_to_instrument_id(stock_code)

        # 根据合约类型解析
        instrument_type = self._client.get_instrument_type(stock_code)

        if instrument_type.get("stock") or instrument_type.get("fund"):
            return parse_equity(detail, instrument_id)
        elif instrument_type.get("future"):
            return parse_future(detail, instrument_id)
        elif instrument_type.get("option"):
            return parse_option(detail, instrument_id)
        else:
            return parse_equity(detail, instrument_id)  # 默认

    async def load_ids_async(
        self,
        instrument_ids: list[InstrumentId],
        filters: dict | None = None,
    ) -> None:
        """按 ID 加载工具"""
        for instrument_id in instrument_ids:
            if self.find(instrument_id) is not None:
                continue

            stock_code = instrument_id_to_stock_code(instrument_id)
            detail = self._client.get_instrument_detail(stock_code)

            if detail:
                instrument = self._parse_instrument(stock_code, detail)
                if instrument:
                    self.add(instrument)
