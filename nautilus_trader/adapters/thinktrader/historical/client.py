import datetime
import re
from typing import Literal

import pandas as pd

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.parsing.instruments import instrument_id_to_stock_code
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.component import init_logging
from nautilus_trader.common.component import is_logging_initialized
from nautilus_trader.common.component import log_level_from_str
from nautilus_trader.common.functions import get_event_loop
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarSpecification
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import AggregationSource
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId


_GLOBAL_LOG_GUARD = None


class HistoricThinkTraderClient:
    """
    提供回测所需的历史行情数据请求方法。

    注意: 该客户端不依赖 DataEngine/TradingNode, 可独立用于批量下载与解析历史数据。
    """

    def __init__(
        self,
        miniqmt_path: str,
        session_id: int = 123456,
        log_level: str = "INFO",
        instrument_provider_config: ThinkTraderInstrumentProviderConfig | None = None,
    ) -> None:
        global _GLOBAL_LOG_GUARD

        loop = get_event_loop()
        loop.set_debug(False)

        self._clock = LiveClock()
        if _GLOBAL_LOG_GUARD is None and not is_logging_initialized():
            _GLOBAL_LOG_GUARD = init_logging(level_stdout=log_level_from_str(log_level))
        self._log_guard = _GLOBAL_LOG_GUARD
        self.log = Logger(name="HistoricThinkTraderClient")

        trader_id = TraderId("historic_thinktrader_client-001")
        msgbus = MessageBus(trader_id, self._clock)
        cache = Cache()
        self._cache = cache

        self._client = ThinkTraderClient(
            loop=loop,
            logger=self.log,
            miniqmt_path=miniqmt_path,
            session_id=session_id,
            account_id="",
        )
        self._client._clock = self._clock
        self._client._cache = cache
        self._client._msgbus = msgbus

        if instrument_provider_config is None:
            instrument_provider_config = ThinkTraderInstrumentProviderConfig(load_contracts_on_start=False)
        self._instrument_provider_config = instrument_provider_config
        instrument_provider = ThinkTraderInstrumentProvider(client=self._client, config=instrument_provider_config)

        self._data_client = ThinkTraderDataClient(
            loop=loop,
            client=self._client,
            msgbus=msgbus,
            cache=cache,
            clock=self._clock,
            instrument_provider=instrument_provider,
            config=ThinkTraderDataClientConfig(
                miniqmt_path=miniqmt_path,
                session_id=session_id,
                instrument_provider=instrument_provider_config,
                skip_trader_login=True,
            ),
        )

        self._client.configure_xtdata_data_dir(miniqmt_path)

    async def request_instruments(
        self,
        instrument_ids: list[str | InstrumentId],
    ) -> list:
        converted = [
            InstrumentId.from_str(x) if isinstance(x, str) else x
            for x in (instrument_ids or [])
        ]
        await self._data_client.instrument_provider.load_ids_async(converted)
        return list(self._data_client.instrument_provider.list_all())

    def _to_shanghai_ts(self, value: datetime.datetime, tz_name: str) -> pd.Timestamp:
        ts = pd.Timestamp(value)
        if ts.tzinfo is None:
            ts = ts.tz_localize(tz_name)
        return ts.tz_convert("Asia/Shanghai")

    def _duration_to_timedelta(self, duration: str) -> pd.Timedelta:
        if not re.match(r"^\d+\s[SDWMY]$", duration):
            raise ValueError("duration 必须符合格式: 'int S|D|W|M|Y'")
        amount, unit = duration.split(" ")
        n = int(amount)
        if unit == "S":
            return pd.Timedelta(seconds=n)
        if unit == "D":
            return pd.Timedelta(days=n)
        if unit == "W":
            return pd.Timedelta(weeks=n)
        if unit == "M":
            return pd.Timedelta(days=30 * n)
        return pd.Timedelta(days=365 * n)

    def _convert_instrument_ids(self, instrument_ids: list[str | InstrumentId]) -> list[InstrumentId]:
        return [InstrumentId.from_str(x) if isinstance(x, str) else x for x in instrument_ids]

    async def request_bars(
        self,
        bar_specifications: list[str],
        end_date_time: datetime.datetime,
        tz_name: str,
        start_date_time: datetime.datetime | None = None,
        duration: str | None = None,
        instrument_ids: list[str | InstrumentId] | None = None,
        timeout: int = 120,
    ) -> list[Bar]:
        if start_date_time and duration:
            raise ValueError("应提供 start_date_time 或 duration 其中之一, 不能两者都提供.")

        if instrument_ids is None or not instrument_ids:
            raise ValueError("必须提供 instrument_ids")

        end_ts = self._to_shanghai_ts(end_date_time, tz_name)
        start_ts = (
            self._to_shanghai_ts(start_date_time, tz_name)
            if start_date_time is not None
            else end_ts - self._duration_to_timedelta(duration)
            if duration is not None
            else None
        )
        if start_ts is None:
            raise ValueError("必须提供 start_date_time 或 duration")

        if start_ts >= end_ts:
            raise ValueError("开始日期必须早于结束日期.")

        converted_ids = self._convert_instrument_ids(instrument_ids)
        await self._data_client.instrument_provider.load_ids_async(converted_ids)

        data: list[Bar] = []
        start_ns = int(start_ts.timestamp() * 1e9)
        end_ns = int(end_ts.timestamp() * 1e9)
        cache = getattr(self, "_cache", None)

        for instrument_id in converted_ids:
            stock_code = instrument_id_to_stock_code(instrument_id, cache)
            for bar_spec in bar_specifications:
                bar_type = BarType(
                    instrument_id,
                    BarSpecification.from_str(bar_spec),
                    AggregationSource.EXTERNAL,
                )
                bars = await self._data_client.get_historical_bars_chunked(
                    bar_type=bar_type,
                    stock_code=stock_code,
                    start_ns=start_ns,
                    end_ns=end_ns,
                    timeout=timeout,
                )
                if bars:
                    data.extend(bars)

        return sorted(data, key=lambda b: b.ts_event)

    async def request_ticks(
        self,
        tick_type: Literal["TRADES", "BID_ASK"],
        start_date_time: datetime.datetime,
        end_date_time: datetime.datetime,
        tz_name: str,
        instrument_ids: list[str | InstrumentId] | None = None,
        timeout: int = 60,
        limit: int = 0,
    ) -> list[TradeTick | QuoteTick]:
        if tick_type not in ("TRADES", "BID_ASK"):
            raise ValueError("tick_type 必须是 'TRADES' 或 'BID_ASK'")

        if start_date_time >= end_date_time:
            raise ValueError("开始日期必须早于结束日期.")

        if instrument_ids is None or not instrument_ids:
            raise ValueError("必须提供 instrument_ids")

        start_ts = self._to_shanghai_ts(start_date_time, tz_name)
        end_ts = self._to_shanghai_ts(end_date_time, tz_name)

        converted_ids = [
            InstrumentId.from_str(x) if isinstance(x, str) else x
            for x in instrument_ids
        ]
        await self._data_client.instrument_provider.load_ids_async(converted_ids)

        start_ns = int(start_ts.timestamp() * 1e9)
        end_ns = int(end_ts.timestamp() * 1e9)

        data: list[TradeTick | QuoteTick] = []
        cache = getattr(self, "_cache", None)
        for instrument_id in converted_ids:
            stock_code = instrument_id_to_stock_code(instrument_id, cache)
            if tick_type == "TRADES":
                ticks = await self._data_client.get_historical_ticks_chunked(
                    instrument_id=instrument_id,
                    stock_code=stock_code,
                    start_ns=start_ns,
                    end_ns=end_ns,
                    timeout=timeout,
                    trade_only=True,
                )
            else:
                ticks = await self._data_client.get_historical_ticks_chunked(
                    instrument_id=instrument_id,
                    stock_code=stock_code,
                    start_ns=start_ns,
                    end_ns=end_ns,
                    timeout=timeout,
                    quote_only=True,
                )

            if ticks:
                data.extend(ticks)

        data.sort(key=lambda t: t.ts_event)
        if limit > 0 and len(data) > limit:
            data = data[-limit:]

        return data

    def get_stock_list(self, sector: str = "沪深A股") -> list[str]:
        """获取板块内的合约列表"""
        return self._client.get_stock_list(sector=sector)

    def get_instrument_detail(self, stock_code: str) -> dict | None:
        """获取合约详情"""
        return self._client.get_instrument_detail(stock_code=stock_code)

    def get_market_data(
        self,
        field_list: list[str],
        stock_list: list[str],
        period: str = "1d",
        start_time: str = "",
        end_time: str = "",
        count: int = -1,
        dividend_type: str = "none",
        fill_data: bool = True,
    ) -> dict[str, pd.DataFrame]:
        """批量获取市场数据"""
        return self._client.get_market_data(
            field_list=field_list,
            stock_list=stock_list,
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=count,
            dividend_type=dividend_type,
            fill_data=fill_data,
        )

    def get_full_tick(self, stock_list: list[str]) -> dict[str, dict]:
        """获取全推行情快照"""
        return self._client.get_full_tick(stock_list=stock_list)

    def subscribe_whole_quote(self, code_list: list[str], callback=None) -> int:
        """订阅全推行情"""
        return self._client.subscribe_whole_quote(code_list=code_list, callback=callback)

    def subscribe_quote(self, stock_code: str, period: str = "tick", count: int = 1, callback=None) -> int:
        """订阅单股行情"""
        import xtquant.xtdata as xtdata
        return xtdata.subscribe_quote(stock_code=stock_code, period=period, count=count, callback=callback)
