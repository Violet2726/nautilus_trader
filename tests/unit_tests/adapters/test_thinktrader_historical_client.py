import datetime as dt
from unittest.mock import AsyncMock
from unittest.mock import Mock
from unittest.mock import patch

import pandas as pd
import pytest

from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarSpecification
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import AggregationSource
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity
from nautilus_trader.test_kit.providers import TestInstrumentProvider
from nautilus_trader.test_kit.stubs.data import TestDataStubs


def _make_client(*, bars_result: list[Bar] | None = None, ticks_result: list | None = None):
    client = HistoricThinkTraderClient.__new__(HistoricThinkTraderClient)

    instrument_provider = Mock()
    instrument_provider.load_ids_async = AsyncMock()

    data_client = Mock()
    data_client.instrument_provider = instrument_provider
    data_client.cache = Mock()
    data_client.get_historical_bars_chunked = AsyncMock(return_value=bars_result or [])
    data_client.get_historical_ticks_chunked = AsyncMock(return_value=ticks_result or [])

    client._data_client = data_client
    return client


@pytest.mark.asyncio
async def test_request_bars_rejects_start_and_duration() -> None:
    client = _make_client()
    with pytest.raises(ValueError, match="start_date_time 或 duration"):
        await client.request_bars(
            bar_specifications=["1-MINUTE-LAST"],
            start_date_time=dt.datetime(2024, 1, 1, tzinfo=dt.UTC),
            end_date_time=dt.datetime(2024, 1, 2, tzinfo=dt.UTC),
            tz_name="UTC",
            duration="1 D",
            instrument_ids=["000001.SZSE"],
        )


@pytest.mark.asyncio
async def test_request_bars_requires_instrument_ids() -> None:
    client = _make_client()
    with pytest.raises(ValueError, match="instrument_ids"):
        await client.request_bars(
            bar_specifications=["1-MINUTE-LAST"],
            start_date_time=dt.datetime(2024, 1, 1, tzinfo=dt.UTC),
            end_date_time=dt.datetime(2024, 1, 2, tzinfo=dt.UTC),
            tz_name="UTC",
            instrument_ids=None,
        )


@pytest.mark.asyncio
async def test_request_bars_sorts_by_ts_event_and_calls_data_client() -> None:
    instrument = TestInstrumentProvider.equity(symbol="000001", venue="SZSE")
    instrument_id = instrument.id
    bar_type = BarType(
        instrument_id=instrument_id,
        bar_spec=BarSpecification.from_str("1-MINUTE-LAST"),
        aggregation_source=AggregationSource.EXTERNAL,
    )
    bar_2 = Bar(
        bar_type=bar_type,
        open=Price.from_str("10.00"),
        high=Price.from_str("10.01"),
        low=Price.from_str("9.99"),
        close=Price.from_str("10.00"),
        volume=Quantity.from_int(1),
        ts_event=2,
        ts_init=2,
    )
    bar_1 = Bar(
        bar_type=bar_type,
        open=Price.from_str("10.00"),
        high=Price.from_str("10.01"),
        low=Price.from_str("9.99"),
        close=Price.from_str("10.00"),
        volume=Quantity.from_int(1),
        ts_event=1,
        ts_init=1,
    )

    client = _make_client(bars_result=[bar_2, bar_1])

    start_dt = dt.datetime(2024, 1, 1, 0, 0, 0, tzinfo=dt.UTC)
    end_dt = dt.datetime(2024, 1, 1, 0, 1, 0, tzinfo=dt.UTC)

    expected_start_ns = int(pd.Timestamp(start_dt).tz_convert("Asia/Shanghai").timestamp() * 1e9)
    expected_end_ns = int(pd.Timestamp(end_dt).tz_convert("Asia/Shanghai").timestamp() * 1e9)

    with patch(
        "nautilus_trader.adapters.thinktrader.historical.client.instrument_id_to_stock_code",
        return_value="000001.SZ",
    ):
        result = await client.request_bars(
            bar_specifications=["1-MINUTE-LAST"],
            start_date_time=start_dt,
            end_date_time=end_dt,
            tz_name="UTC",
            instrument_ids=[instrument_id],
        )

    assert [b.ts_event for b in result] == [1, 2]

    client._data_client.instrument_provider.load_ids_async.assert_awaited_once()
    client._data_client.get_historical_bars_chunked.assert_awaited()
    _, kwargs = client._data_client.get_historical_bars_chunked.call_args
    assert kwargs["bar_type"].instrument_id == instrument_id
    assert kwargs["stock_code"] == "000001.SZ"
    assert kwargs["start_ns"] == expected_start_ns
    assert kwargs["end_ns"] == expected_end_ns


@pytest.mark.asyncio
async def test_request_ticks_sorts_and_applies_limit_from_end() -> None:
    instrument = TestInstrumentProvider.equity(symbol="000001", venue="SZSE")
    instrument_id = instrument.id

    tick_1 = TestDataStubs.trade_tick(instrument=instrument, ts_event=1, ts_init=1)
    tick_3 = TestDataStubs.trade_tick(instrument=instrument, ts_event=3, ts_init=3)
    tick_2 = TestDataStubs.trade_tick(instrument=instrument, ts_event=2, ts_init=2)

    client = _make_client(ticks_result=[tick_1, tick_3, tick_2])

    with patch(
        "nautilus_trader.adapters.thinktrader.historical.client.instrument_id_to_stock_code",
        return_value="000001.SZ",
    ):
        result = await client.request_ticks(
            tick_type="TRADES",
            start_date_time=dt.datetime(2024, 1, 1, tzinfo=dt.UTC),
            end_date_time=dt.datetime(2024, 1, 2, tzinfo=dt.UTC),
            tz_name="UTC",
            instrument_ids=[InstrumentId.from_str(str(instrument_id))],
            limit=2,
        )

    assert [t.ts_event for t in result] == [2, 3]
    client._data_client.instrument_provider.load_ids_async.assert_awaited_once()
    client._data_client.get_historical_ticks_chunked.assert_awaited()
    _, kwargs = client._data_client.get_historical_ticks_chunked.call_args
    assert kwargs["trade_only"] is True
