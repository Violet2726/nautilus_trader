import asyncio
from unittest.mock import Mock
from unittest.mock import patch
from unittest.mock import sentinel

import pytest

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import TestClock
from nautilus_trader.model.data import BarType
from nautilus_trader.model.identifiers import InstrumentId


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


@pytest.mark.asyncio
async def test_get_historical_ticks_calls_get_market_data(thinktrader_client):
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


@pytest.mark.asyncio
async def test_unsubscribe_missing_subscription_does_not_call_xtdata(thinktrader_client):
    instrument_id = InstrumentId.from_str("000001.SZSE")
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ) as unsubscribe_quote:
        await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
        unsubscribe_quote.assert_not_called()

