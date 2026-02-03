import asyncio
from datetime import datetime
from datetime import timezone
from unittest.mock import Mock
from unittest.mock import AsyncMock
from unittest.mock import patch
from unittest.mock import sentinel

import pytest

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import TestClock
from nautilus_trader.model.data import BarType
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.test_kit.stubs.identifiers import TestIdStubs


def _print_section(title: str) -> None:
    print("\n" + "=" * 88)
    print(f"【ThinkTrader 测试】{title}")
    print("=" * 88)


@pytest.fixture
def thinktrader_client(event_loop):
    client = ThinkTraderClient(
        loop=event_loop,
        logger=Logger("ThinkTraderClientTests"),
        miniqmt_path="D:\\中信证券QMT交易终端仿真\\userdata_mini",
        session_id=1,
        account_id="",
    )
    client._clock = TestClock()
    client._cache = Mock()
    client._msgbus = Mock()
    return client


@pytest.mark.asyncio
async def test_subscribe_ticks_and_unsubscribe_ticks(thinktrader_client):
    _print_section("订阅/反订阅 tick（xtdata.subscribe_quote / unsubscribe_quote）")
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=123,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ) as unsubscribe_quote:
        await thinktrader_client.subscribe_ticks(instrument_id=instrument_id, stock_code=stock_code)

        subscribe_quote.assert_called_once()
        _, kwargs = subscribe_quote.call_args
        assert kwargs["stock_code"] == stock_code
        assert kwargs["period"] == "tick"
        assert kwargs["count"] == 0
        assert callable(kwargs["callback"])

        name = (str(instrument_id), "tick")
        sub = thinktrader_client._subscriptions.get(name=name)
        assert sub is not None
        assert sub.req_id == 123

        await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
        unsubscribe_quote.assert_called_once_with(123)
        assert thinktrader_client._subscriptions.get(name=name) is None


@pytest.mark.asyncio
async def test_subscribe_is_idempotent_by_name(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=1001,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ):
        await thinktrader_client.subscribe_ticks(instrument_id=instrument_id, stock_code=stock_code)
        await thinktrader_client.subscribe_ticks(instrument_id=instrument_id, stock_code=stock_code)
        subscribe_quote.assert_called_once()


@pytest.mark.asyncio
async def test_subscribe_order_book_uses_l2quote_period(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=456,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ):
        await thinktrader_client.subscribe_order_book(instrument_id=instrument_id, stock_code=stock_code)
        _, kwargs = subscribe_quote.call_args
        assert kwargs["period"] == "l2quote"


@pytest.mark.asyncio
async def test_subscribe_tick_by_tick_periods(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=789,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ):
        await thinktrader_client.subscribe_tick_by_tick(
            instrument_id=instrument_id,
            stock_code=stock_code,
            tick_type="AllLast",
        )
        _, kwargs = subscribe_quote.call_args
        assert kwargs["period"] == "l2transaction"

    thinktrader_client._subscriptions = type(thinktrader_client._subscriptions)()
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=790,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ):
        await thinktrader_client.subscribe_tick_by_tick(
            instrument_id=instrument_id,
            stock_code=stock_code,
            tick_type="BidAsk",
        )
        _, kwargs = subscribe_quote.call_args
        assert kwargs["period"] == "l2order"


@pytest.mark.asyncio
async def test_subscribe_realtime_bars_uses_bar_spec_period(thinktrader_client):
    bar_type = BarType.from_str("000001.SZSE-1-MINUTE-LAST-EXTERNAL")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.subscribe_quote",
        return_value=101,
    ) as subscribe_quote, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ):
        await thinktrader_client.subscribe_realtime_bars(bar_type=bar_type, stock_code=stock_code)
        _, kwargs = subscribe_quote.call_args
        assert kwargs["period"] == "1m"


@pytest.mark.asyncio
async def test_get_historical_bars_calls_get_market_data(thinktrader_client):
    _print_section("历史 K 线获取（xtdata.get_market_data）")
    from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time

    bar_type = BarType.from_str("000001.SZSE-1-MINUTE-LAST-EXTERNAL")
    stock_code = "000001.SZ"
    start_ns = 0
    end_ns = 60_000_000_000
    expected_result = {"result": "bars"}

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_market_data",
        return_value=expected_result,
    ) as get_market_data:
        result = await thinktrader_client.get_historical_bars(
            bar_type=bar_type,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
            timeout=1,
        )

        assert result == expected_result
        _, kwargs = get_market_data.call_args
        assert kwargs["stock_list"] == [stock_code]
        assert kwargs["period"] == "1m"
        assert kwargs["start_time"] == ns_to_xt_time(start_ns)
        assert kwargs["end_time"] == ns_to_xt_time(end_ns)
        assert kwargs["count"] == -1
        assert kwargs["dividend_type"] == "none"
        assert kwargs["fill_data"] is True
        print("已调用参数:", kwargs)


@pytest.mark.asyncio
async def test_get_historical_ticks_calls_get_market_data(thinktrader_client):
    _print_section("历史 Tick 获取（xtdata.get_market_data period=tick）")
    from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time

    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"
    start_ns = 0
    end_ns = 1_000_000_000
    expected_result = {"result": "ticks"}

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_market_data",
        return_value=expected_result,
    ) as get_market_data:
        result = await thinktrader_client.get_historical_ticks(
            instrument_id=instrument_id,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
            timeout=1,
        )

        assert result == expected_result
        _, kwargs = get_market_data.call_args
        assert kwargs["stock_list"] == [stock_code]
        assert kwargs["period"] == "tick"
        assert kwargs["start_time"] == ns_to_xt_time(start_ns)
        assert kwargs["end_time"] == ns_to_xt_time(end_ns)
        assert kwargs["count"] == -1
        print("已调用参数:", kwargs)


@pytest.mark.asyncio
async def test_req_fundamental_data_returns_financial_and_detail(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    financial = {"financial": True}
    detail = {"detail": True}

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_financial_data",
        return_value=financial,
    ) as get_financial_data, patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_instrument_detail",
        return_value=detail,
    ) as get_instrument_detail:
        result = await thinktrader_client.req_fundamental_data(
            instrument_id=instrument_id,
            stock_code=stock_code,
            report_type="report_time",
            timeout=1,
        )

        assert result == {"financial": financial, "detail": detail}
        get_financial_data.assert_called_once_with(stock_list=[stock_code], report_type="report_time")
        get_instrument_detail.assert_called_once_with(stock_code)


@pytest.mark.asyncio
async def test_get_price_reads_last_price_from_full_tick(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_full_tick",
        return_value={stock_code: {"lastPrice": 12.34}},
    ) as get_full_tick:
        result = await thinktrader_client.get_price(instrument_id=instrument_id, stock_code=stock_code)
        assert result == 12.34
        get_full_tick.assert_called_once_with([stock_code])


@pytest.mark.asyncio
async def test_on_quote_data_schedules_handle_quote_data(thinktrader_client):
    thinktrader_client._handle_quote_data = Mock()

    datas = {"000001.SZ": [{"k": 1}, {"k": 2}]}
    name = ("000001.SZSE", "tick")
    thinktrader_client._on_quote_data(datas, name=name)

    await asyncio.sleep(0)

    assert thinktrader_client._handle_quote_data.call_count == 2
    thinktrader_client._handle_quote_data.assert_any_call(name, "000001.SZ", {"k": 1})
    thinktrader_client._handle_quote_data.assert_any_call(name, "000001.SZ", {"k": 2})


def test_handle_quote_data_tick_forwards_quote_and_trade(thinktrader_client):
    seen = []
    thinktrader_client.register_event_handler("data", seen.append)

    instrument_id = InstrumentId.from_str("000001.SZSE")
    name = (str(instrument_id), "tick")

    with patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_tick_to_quote_tick",
        return_value=sentinel.quote,
    ) as parse_quote, patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_tick_to_trade_tick",
        return_value=sentinel.trade,
    ) as parse_trade:
        thinktrader_client._handle_quote_data(name, "000001.SZ", {"time": 0})

        parse_quote.assert_called_once()
        parse_trade.assert_called_once()
        assert seen == [sentinel.quote, sentinel.trade]


def test_handle_quote_data_order_book_forwards_all_deltas(thinktrader_client):
    seen = []
    thinktrader_client.register_event_handler("data", seen.append)

    instrument_id = InstrumentId.from_str("000001.SZSE")
    name = (str(instrument_id), "order_book")

    with patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_l2_quote_to_order_book_deltas",
        return_value=[sentinel.d1, sentinel.d2],
    ) as parse_deltas:
        thinktrader_client._handle_quote_data(name, "000001.SZ", {"time": 0})
        parse_deltas.assert_called_once()
        assert seen == [sentinel.d1, sentinel.d2]


def test_handle_quote_data_l2order_forwards_delta(thinktrader_client):
    seen = []
    thinktrader_client.register_event_handler("data", seen.append)

    instrument_id = InstrumentId.from_str("000001.SZSE")
    name = (str(instrument_id), "l2order")

    with patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_l2_order_to_delta",
        return_value=sentinel.delta,
    ) as parse_delta:
        thinktrader_client._handle_quote_data(name, "000001.SZ", {"time": 0})
        parse_delta.assert_called_once()
        assert seen == [sentinel.delta]


def test_handle_quote_data_l2transaction_forwards_trade_tick(thinktrader_client):
    seen = []
    thinktrader_client.register_event_handler("data", seen.append)

    instrument_id = InstrumentId.from_str("000001.SZSE")
    name = (str(instrument_id), "l2transaction")

    with patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_l2_transaction_to_trade_tick",
        return_value=sentinel.trade,
    ) as parse_trade:
        thinktrader_client._handle_quote_data(name, "000001.SZ", {"time": 0})
        parse_trade.assert_called_once()
        assert seen == [sentinel.trade]


def test_handle_quote_data_bar_type_string_forwards_bar(thinktrader_client):
    seen = []
    thinktrader_client.register_event_handler("data", seen.append)

    bar_type = BarType.from_str("000001.SZSE-1-MINUTE-LAST-EXTERNAL")
    name = str(bar_type)

    with patch(
        "nautilus_trader.adapters.thinktrader.parsing.data.parse_kline_to_bar",
        return_value=sentinel.bar,
    ) as parse_bar:
        thinktrader_client._handle_quote_data(name, "000001.SZ", {"time": 0})
        parse_bar.assert_called_once()
        assert seen == [sentinel.bar]


@pytest.mark.asyncio
async def test_download_history_data_waits_for_finished(thinktrader_client):
    _print_section("历史数据下载等待完成（xtdata.download_history_data2 callback）")
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.download_history_data2",
    ) as download_history_data2:
        def side_effect(*, callback, **_kwargs):
            callback({"finished": True})

        download_history_data2.side_effect = side_effect

        await thinktrader_client.download_history_data(
            stock_list=["000001.SZ"],
            period="1m",
            start_time="20240101000000",
            end_time="20240102000000",
        )
        print("download_history_data2 已触发 finished=True，等待结束通过。")


@pytest.mark.asyncio
async def test_unsubscribe_missing_subscription_does_not_call_xtdata(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ) as unsubscribe_quote:
        await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
        unsubscribe_quote.assert_not_called()


@pytest.fixture
def thinktrader_data_client(event_loop):
    from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
    from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
    from nautilus_trader.cache.cache import Cache
    from nautilus_trader.common.component import MessageBus
    from nautilus_trader.common.providers import InstrumentProvider

    clock = TestClock()
    msgbus = MessageBus(trader_id=TestIdStubs.trader_id(), clock=clock)
    cache = Cache(database=None)
    client = ThinkTraderClient(
        loop=event_loop,
        logger=Logger("ThinkTraderDataClientTests"),
        miniqmt_path="D:\\中信证券QMT交易终端仿真\\userdata_mini",
        session_id=1,
        account_id="",
    )

    config = ThinkTraderDataClientConfig(
        miniqmt_path="D:\\中信证券QMT交易终端仿真\\userdata_mini",
        session_id=1,
        subscribe_whole_quote=False,
        skip_trader_login=True,
    )

    return ThinkTraderDataClient(
        loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=InstrumentProvider(),
        config=config,
    )


@pytest.mark.asyncio
async def test_data_client_request_quote_ticks_parses_numpy_ticks(thinktrader_data_client):
    _print_section("DataClient 请求 QuoteTicks：解析 xtdata tick(np.ndarray) -> QuoteTick 列表")
    import numpy as np

    from nautilus_trader.core.uuid import UUID4
    from nautilus_trader.data.messages import RequestQuoteTicks

    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    arr = np.array(
        [
            (
                20240101093000,
                [10.0],
                [10.01],
                [100],
                [120],
                10.005,
                50,
            ),
            (
                20240101093001,
                [10.01],
                [10.02],
                [110],
                [130],
                10.015,
                60,
            ),
        ],
        dtype=[
            ("time", "i8"),
            ("bidPrice", "O"),
            ("askPrice", "O"),
            ("bidVol", "O"),
            ("askVol", "O"),
            ("lastPrice", "f8"),
            ("volume", "i8"),
        ],
    )

    thinktrader_data_client._client.download_history_data = AsyncMock(return_value=None)
    thinktrader_data_client._client.get_historical_ticks = AsyncMock(return_value={stock_code: arr})

    seen: list = []
    thinktrader_data_client._handle_quote_ticks = Mock(
        side_effect=lambda _instrument_id, ticks, *_args: seen.extend(ticks),
    )

    request = RequestQuoteTicks(
        instrument_id=instrument_id,
        start=datetime(2024, 1, 1, 1, 0, 0, tzinfo=timezone.utc),
        end=datetime(2024, 1, 1, 1, 5, 0, tzinfo=timezone.utc),
        limit=0,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        callback=None,
        request_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )

    await thinktrader_data_client._request_quote_ticks(request)

    assert len(seen) == 2
    print(f"解析得到 QuoteTick 数量: {len(seen)}")
    print("样例 QuoteTick[0]:", seen[0])
    print("样例 QuoteTick[1]:", seen[1])


@pytest.mark.asyncio
async def test_data_client_request_bars_parses_kline_fields(thinktrader_data_client):
    _print_section("DataClient 请求 Bars：解析 xtdata K线(dict[field]->DataFrame) -> Bar 列表")
    import pandas as pd

    from nautilus_trader.core.uuid import UUID4
    from nautilus_trader.data.messages import RequestBars

    bar_type = BarType.from_str("000001.SZSE-1-MINUTE-LAST-EXTERNAL")
    stock_code = "000001.SZ"

    cols = [20240101093000, 20240101093100]
    data = {
        "open": pd.DataFrame([[10.0, 10.1]], index=[stock_code], columns=cols),
        "high": pd.DataFrame([[10.2, 10.3]], index=[stock_code], columns=cols),
        "low": pd.DataFrame([[9.9, 10.0]], index=[stock_code], columns=cols),
        "close": pd.DataFrame([[10.05, 10.15]], index=[stock_code], columns=cols),
        "volume": pd.DataFrame([[1000, 1200]], index=[stock_code], columns=cols),
    }

    thinktrader_data_client._client.download_history_data = AsyncMock(return_value=None)
    thinktrader_data_client._client.get_historical_bars = AsyncMock(return_value=data)
    thinktrader_data_client._msgbus.publish = Mock()

    seen: list = []
    thinktrader_data_client._handle_bars = Mock(side_effect=lambda _bar_type, bars, *_args: seen.extend(bars))

    request = RequestBars(
        bar_type=bar_type,
        start=datetime(2024, 1, 1, 1, 0, 0, tzinfo=timezone.utc),
        end=datetime(2024, 1, 1, 1, 5, 0, tzinfo=timezone.utc),
        limit=0,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        callback=None,
        request_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )

    await thinktrader_data_client._request_bars(request)

    assert len(seen) == 2
    print(f"解析得到 Bar 数量: {len(seen)}")
    print("样例 Bar[0]:", seen[0])
    print("样例 Bar[1]:", seen[1])
