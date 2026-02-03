import asyncio
import datetime
from unittest.mock import AsyncMock
from unittest.mock import Mock
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
from nautilus_trader.test_kit.providers import TestInstrumentProvider
from nautilus_trader.test_kit.stubs.identifiers import TestIdStubs


def _print_section(title: str) -> None:
    print("\n" + "=" * 88)
    print(f"【ThinkTrader 测试】{title}")
    print("=" * 88)


def _print_kv(key: str, value) -> None:
    print(f"{key}: {value}")


def _try_import_xtdata():
    import importlib
    import importlib.util

    if importlib.util.find_spec("xtquant") is None:
        pytest.skip("未安装 xtquant, 跳过真实行情测试")

    return importlib.import_module("xtquant.xtdata")


def _prepare_xtdata_data_dir(xtdata_module, data_dir: str) -> None:
    xtdata_module.data_dir = data_dir
    _print_kv("xtdata.data_dir", getattr(xtdata_module, "data_dir", None))




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
async def test_instrument_provider_initialize_loads_all_on_start():
    _print_section("InstrumentProvider 启动加载: load_contracts_on_start=True 时加载全量工具")
    from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
    from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider

    client = Mock()
    client.get_stock_list = Mock(return_value=["000001.SZ", "000002.SZ"])
    client.get_instrument_detail = Mock(return_value={"dummy": True})
    client.get_instrument_type = Mock(return_value={"stock": True})

    config = ThinkTraderInstrumentProviderConfig(
        load_contracts_on_start=True,
        cache_instruments=True,
        sectors=("沪深A股",),
    )

    provider = ThinkTraderInstrumentProvider(client=client, config=config)
    provider._parse_instrument = Mock(
        side_effect=[
            TestInstrumentProvider.equity(symbol="000001", venue="SZSE"),
            TestInstrumentProvider.equity(symbol="000002", venue="SZSE"),
        ],
    )

    await provider.initialize()

    instruments = provider.list_all()
    assert len(instruments) == 2
    _print_kv("sectors", config.sectors)
    _print_kv("cache_instruments", config.cache_instruments)
    _print_kv("client.get_stock_list 返回", client.get_stock_list.return_value)
    print(f"已加载工具数量: {len(instruments)}")
    print("样例工具[0]:", instruments[0].id)
    print("样例工具[1]:", instruments[1].id)


@pytest.mark.asyncio
async def test_instrument_provider_initialize_reload_forces_reload():
    _print_section("InstrumentProvider 重载: reload=True 时强制重新加载")
    from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
    from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider

    client = Mock()
    config = ThinkTraderInstrumentProviderConfig(
        load_contracts_on_start=True,
        cache_instruments=True,
        sectors=("沪深A股",),
    )
    provider = ThinkTraderInstrumentProvider(client=client, config=config)
    provider._loaded = True
    provider.load_all_async = AsyncMock(return_value=None)

    await provider.initialize(reload=True)

    provider.load_all_async.assert_called_once()
    _print_kv("reload", True)
    _print_kv("load_all_async 调用次数", provider.load_all_async.call_count)


@pytest.mark.asyncio
async def test_subscribe_ticks_and_unsubscribe_ticks(thinktrader_client):
    _print_section("订阅/反订阅 tick (xtdata.subscribe_quote / unsubscribe_quote)")
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
        _print_kv("subscribe_quote.call_args", subscribe_quote.call_args)
        assert kwargs["stock_code"] == stock_code
        assert kwargs["period"] == "tick"
        assert kwargs["count"] == 0
        assert callable(kwargs["callback"])

        name = (str(instrument_id), "tick")
        sub = thinktrader_client._subscriptions.get(name=name)
        assert sub is not None
        assert sub.req_id == 123
        _print_kv("创建 Subscription", sub)

        await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
        unsubscribe_quote.assert_called_once_with(123)
        assert thinktrader_client._subscriptions.get(name=name) is None
        _print_kv("unsubscribe_quote.call_args", unsubscribe_quote.call_args)
        _print_kv("Subscription 已移除", True)


@pytest.mark.asyncio
async def test_subscribe_is_idempotent_by_name(thinktrader_client):
    _print_section("订阅幂等: 相同 name 第二次订阅应复用 Subscription")
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
        _print_kv("subscribe_quote.call_count", subscribe_quote.call_count)
        name = (str(instrument_id), "tick")
        _print_kv("Subscription 当前状态", thinktrader_client._subscriptions.get(name=name))


@pytest.mark.asyncio
async def test_subscribe_order_book_uses_l2quote_period(thinktrader_client):
    _print_section("订阅 order_book: period 应为 l2quote")
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
        _print_kv("subscribe_quote.call_args", subscribe_quote.call_args)
        assert kwargs["period"] == "l2quote"


@pytest.mark.asyncio
async def test_subscribe_tick_by_tick_periods(thinktrader_client):
    _print_section("订阅 tick_by_tick: AllLast/l2transaction, BidAsk/l2order")
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
        _print_kv("AllLast subscribe_quote.call_args", subscribe_quote.call_args)
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
        _print_kv("BidAsk subscribe_quote.call_args", subscribe_quote.call_args)
        assert kwargs["period"] == "l2order"


@pytest.mark.asyncio
async def test_subscribe_realtime_bars_uses_bar_spec_period(thinktrader_client):
    _print_section("订阅 realtime bars: bar_spec_to_period 映射")
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
        _print_kv("subscribe_quote.call_args", subscribe_quote.call_args)
        _print_kv("bar_type", bar_type)
        assert kwargs["period"] == "1m"


@pytest.mark.asyncio
async def test_get_historical_bars_calls_get_market_data(thinktrader_client):
    _print_section("历史 K 线获取 (xtdata.get_market_data)")
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
        _print_kv("get_market_data.return_value", get_market_data.return_value)
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
    _print_section("历史 Tick 获取 (xtdata.get_market_data period=tick)")
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
        _print_kv("get_market_data.return_value", get_market_data.return_value)
        _, kwargs = get_market_data.call_args
        assert kwargs["stock_list"] == [stock_code]
        assert kwargs["period"] == "tick"
        assert kwargs["start_time"] == ns_to_xt_time(start_ns)
        assert kwargs["end_time"] == ns_to_xt_time(end_ns)
        assert kwargs["count"] == -1
        print("已调用参数:", kwargs)


@pytest.mark.asyncio
async def test_req_fundamental_data_returns_financial_and_detail(thinktrader_client):
    _print_section("基本面数据: 财务数据与合约详情原始响应")
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
        _print_kv("xtdata.get_financial_data 原始返回", financial)
        _print_kv("xtdata.get_instrument_detail 原始返回", detail)
        _print_kv("适配器聚合后返回", result)
        get_financial_data.assert_called_once_with(stock_list=[stock_code], report_type="report_time")
        get_instrument_detail.assert_called_once_with(stock_code)


@pytest.mark.asyncio
async def test_get_price_reads_last_price_from_full_tick(thinktrader_client):
    _print_section("实时价格: get_full_tick 原始响应与返回值")
    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"

    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.get_full_tick",
        return_value={stock_code: {"lastPrice": 12.34}},
    ) as get_full_tick:
        result = await thinktrader_client.get_price(instrument_id=instrument_id, stock_code=stock_code)
        assert result == 12.34
        get_full_tick.assert_called_once_with([stock_code])
        _print_kv("xtdata.get_full_tick 原始响应", get_full_tick.return_value)
        _print_kv("适配器返回价格", result)


@pytest.mark.asyncio
async def test_on_quote_data_schedules_handle_quote_data(thinktrader_client):
    _print_section("回调派发: _on_quote_data 应拆分 datas 并调度处理")
    thinktrader_client._handle_quote_data = Mock()

    datas = {"000001.SZ": [{"k": 1}, {"k": 2}]}
    name = ("000001.SZSE", "tick")
    _print_kv("输入 datas", datas)
    _print_kv("订阅 name", name)
    thinktrader_client._on_quote_data(datas, name=name)

    await asyncio.sleep(0)

    assert thinktrader_client._handle_quote_data.call_count == 2
    _print_kv("调度处理次数", thinktrader_client._handle_quote_data.call_count)
    thinktrader_client._handle_quote_data.assert_any_call(name, "000001.SZ", {"k": 1})
    thinktrader_client._handle_quote_data.assert_any_call(name, "000001.SZ", {"k": 2})


def test_handle_quote_data_tick_forwards_quote_and_trade(thinktrader_client):
    _print_section("数据转发: tick 应转发 QuoteTick 与 TradeTick")
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
        _print_kv("转发数据列表", seen)


def test_handle_quote_data_order_book_forwards_all_deltas(thinktrader_client):
    _print_section("数据转发: order_book 应转发全部 OrderBookDelta")
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
        _print_kv("转发数据列表", seen)


def test_handle_quote_data_l2order_forwards_delta(thinktrader_client):
    _print_section("数据转发: l2order 应转发单条 OrderBookDelta")
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
        _print_kv("转发数据列表", seen)


def test_handle_quote_data_l2transaction_forwards_trade_tick(thinktrader_client):
    _print_section("数据转发: l2transaction 应转发 TradeTick")
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
        _print_kv("转发数据列表", seen)


@pytest.mark.asyncio
async def test_xtdata_live_full_tick_output(event_loop):
    _print_section("xtdata 实测: get_full_tick 原始响应与解析")
    xtdata = _try_import_xtdata()

    client = ThinkTraderClient(
        loop=event_loop,
        logger=Logger("ThinkTraderLive"),
        miniqmt_path="D:\\中信证券QMT交易终端仿真\\userdata_mini",
        session_id=1,
        account_id="",
    )
    _prepare_xtdata_data_dir(xtdata, "D:\\中信证券QMT交易终端仿真\\userdata_mini")
    client.configure_xtdata_data_dir("D:\\中信证券QMT交易终端仿真\\userdata_mini")
    _print_kv("miniqmt_path", "D:\\中信证券QMT交易终端仿真\\userdata_mini")

    instrument_id = InstrumentId.from_str("000001.SZSE")
    stock_code = "000001.SZ"
    _print_kv("instrument_id", instrument_id)
    _print_kv("stock_code", stock_code)

    raw = xtdata.get_full_tick([stock_code])
    print("xtdata.get_full_tick 原始响应:", raw)

    price = await client.get_price(instrument_id=instrument_id, stock_code=stock_code)
    print(f"适配器解析后返回价格: {price}")

    assert isinstance(raw, dict)


@pytest.mark.asyncio
async def test_xtdata_live_subscribe_tick_once_and_unsubscribe(event_loop):
    _print_section("xtdata 实测: subscribe_quote(tick) 回调原始数据与取消订阅")
    xtdata = _try_import_xtdata()
    _prepare_xtdata_data_dir(xtdata, "D:\\中信证券QMT交易终端仿真\\userdata_mini")

    stock_code = "000001.SZ"
    _print_kv("stock_code", stock_code)

    loop = event_loop
    future = loop.create_future()

    def on_tick(datas):
        if future.done():
            return
        print("xtdata.subscribe_quote callback 原始参数:", datas)
        loop.call_soon_threadsafe(future.set_result, datas)

    seq = xtdata.subscribe_quote(
        stock_code=stock_code,
        period="tick",
        count=0,
        callback=on_tick,
    )
    _print_kv("subscribe_quote 返回 seq", seq)
    assert isinstance(seq, int)
    if seq <= 0:
        pytest.skip("subscribe_quote 返回非正值, 可能未连接 MiniQmt 或无权限")

    try:
        try:
            datas = await asyncio.wait_for(future, timeout=5.0)
        except TimeoutError:
            pytest.skip("等待 tick 回调超时, 可能当前无实时推送或未连接 MiniQmt")

        _print_kv("回调数据类型", type(datas))
        if isinstance(datas, dict):
            _print_kv("回调 dict keys", list(datas.keys())[:10])
            first_key = next(iter(datas.keys()), None)
            if first_key is not None:
                _print_kv("回调样例 stock_code", first_key)
                _print_kv("回调样例 payload", datas[first_key])
    finally:
        xtdata.unsubscribe_quote(seq)
        _print_kv("unsubscribe_quote 已调用", True)


@pytest.mark.asyncio
async def test_xtdata_live_get_market_data_tick_latest(event_loop):
    _print_section("xtdata 实测: get_market_data(period=tick) 原始响应与样例")
    xtdata = _try_import_xtdata()
    _prepare_xtdata_data_dir(xtdata, "D:\\中信证券QMT交易终端仿真\\userdata_mini")

    stock_code = "000001.SZ"
    result = xtdata.get_market_data(
        field_list=[],
        stock_list=[stock_code],
        period="tick",
        start_time="",
        end_time="",
        count=10,
    )
    print("xtdata.get_market_data 原始响应:", result)

    assert isinstance(result, dict)
    data = result.get(stock_code)
    _print_kv("result[stock_code] 类型", type(data))
    if data is None:
        pytest.skip("tick 返回为空, 可能本地无缓存且未收到订阅数据")

    length = getattr(data, "shape", [len(data)])[0] if hasattr(data, "shape") else len(data)  # type: ignore[arg-type]
    _print_kv("tick 记录数", length)
    from contextlib import suppress

    with suppress(Exception):
        _print_kv("tick dtype.names", getattr(getattr(data, "dtype", None), "names", None))
    with suppress(Exception):
        _print_kv("tick 样例[0]", data[0])


@pytest.mark.asyncio
async def test_xtdata_live_get_market_data_1m_latest(event_loop):
    _print_section("xtdata 实测: get_market_data(period=1m) 原始响应与 DataFrame 结构")
    xtdata = _try_import_xtdata()
    _prepare_xtdata_data_dir(xtdata, "D:\\中信证券QMT交易终端仿真\\userdata_mini")

    stock_code = "000001.SZ"
    result = xtdata.get_market_data(
        field_list=["open", "high", "low", "close", "volume"],
        stock_list=[stock_code],
        period="1m",
        start_time="",
        end_time="",
        count=10,
        dividend_type="none",
        fill_data=True,
    )
    print("xtdata.get_market_data 原始响应 keys:", list(result.keys()) if isinstance(result, dict) else type(result))
    assert isinstance(result, dict)

    for field in ("open", "high", "low", "close", "volume"):
        df = result.get(field)
        _print_kv(f"field={field} type", type(df))
        if df is None:
            continue
        _print_kv(f"field={field} shape", getattr(df, "shape", None))
        from contextlib import suppress

        with suppress(Exception):
            _print_kv(f"field={field} index_sample", list(getattr(df, "index", []))[:5])
            _print_kv(f"field={field} columns_sample", list(getattr(df, "columns", []))[:5])

def test_handle_quote_data_bar_type_string_forwards_bar(thinktrader_client):
    _print_section("数据转发: bar_type 字符串应解析并转发 Bar")
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
        _print_kv("转发数据列表", seen)


@pytest.mark.asyncio
async def test_download_history_data_waits_for_finished(thinktrader_client):
    _print_section("历史数据下载等待完成 (xtdata.download_history_data2 callback)")
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.download_history_data2",
    ) as download_history_data2:
        def side_effect(*, callback, **_kwargs):
            payload = {"finished": True}
            print("download_history_data2 回调原始数据:", payload)
            callback(payload)

        download_history_data2.side_effect = side_effect

        await thinktrader_client.download_history_data(
            stock_list=["000001.SZ"],
            period="1m",
            start_time="20240101000000",
            end_time="20240102000000",
        )
        _print_kv("download_history_data2.call_args", download_history_data2.call_args)
        _print_kv("等待完成状态", True)


@pytest.mark.asyncio
async def test_unsubscribe_missing_subscription_does_not_call_xtdata(thinktrader_client):
    _print_section("取消订阅: Subscription 不存在时不应调用 xtdata")
    instrument_id = InstrumentId.from_str("000001.SZSE")
    with patch(
        "nautilus_trader.adapters.thinktrader.client.market_data.xtdata.unsubscribe_quote",
    ) as unsubscribe_quote:
        await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
        unsubscribe_quote.assert_not_called()
        _print_kv("unsubscribe_quote.call_count", unsubscribe_quote.call_count)


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
    _print_section("DataClient 请求 QuoteTicks: 解析 xtdata tick(np.ndarray) -> QuoteTick 列表")
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
        start=datetime.datetime(2024, 1, 1, 1, 0, 0, tzinfo=datetime.UTC),
        end=datetime.datetime(2024, 1, 1, 1, 5, 0, tzinfo=datetime.UTC),
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
    _print_kv("RequestQuoteTicks.instrument_id", request.instrument_id)
    _print_kv("RequestQuoteTicks.start", request.start)
    _print_kv("RequestQuoteTicks.end", request.end)
    _print_kv("mock get_historical_ticks.return_value keys", list(thinktrader_data_client._client.get_historical_ticks.return_value.keys()))
    _print_kv("mock tick ndarray dtype.names", arr.dtype.names)
    _print_kv("mock tick ndarray[0]", arr[0])
    _print_kv("download_history_data.await_args", thinktrader_data_client._client.download_history_data.await_args)
    _print_kv("get_historical_ticks.await_args", thinktrader_data_client._client.get_historical_ticks.await_args)
    print(f"解析得到 QuoteTick 数量: {len(seen)}")
    print("样例 QuoteTick[0]:", seen[0])
    print("样例 QuoteTick[1]:", seen[1])


@pytest.mark.asyncio
async def test_data_client_request_bars_parses_kline_fields(thinktrader_data_client):
    _print_section("DataClient 请求 Bars: 解析 xtdata K线(dict[field]->DataFrame) -> Bar 列表")
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

    seen: list = []
    thinktrader_data_client._handle_bars = Mock(side_effect=lambda _bar_type, bars, *_args: seen.extend(bars))

    request = RequestBars(
        bar_type=bar_type,
        start=datetime.datetime(2024, 1, 1, 1, 0, 0, tzinfo=datetime.UTC),
        end=datetime.datetime(2024, 1, 1, 1, 5, 0, tzinfo=datetime.UTC),
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
    _print_kv("RequestBars.bar_type", request.bar_type)
    _print_kv("RequestBars.start", request.start)
    _print_kv("RequestBars.end", request.end)
    _print_kv("mock kline keys", list(data.keys()))
    _print_kv("mock kline open.columns", list(data["open"].columns))
    _print_kv("download_history_data.await_args", thinktrader_data_client._client.download_history_data.await_args)
    _print_kv("get_historical_bars.await_args", thinktrader_data_client._client.get_historical_bars.await_args)
    print(f"解析得到 Bar 数量: {len(seen)}")
    print("样例 Bar[0]:", seen[0])
    print("样例 Bar[1]:", seen[1])


@pytest.mark.asyncio
async def test_data_client_subscribe_and_unsubscribe_quote_ticks_calls_client_methods(thinktrader_data_client):
    _print_section("DataClient 订阅/反订阅 QuoteTicks: 调用 client.subscribe_market_data")
    from nautilus_trader.core.uuid import UUID4
    from nautilus_trader.data.messages import SubscribeQuoteTicks
    from nautilus_trader.data.messages import UnsubscribeQuoteTicks

    instrument_id = InstrumentId.from_str("000001.SZSE")

    thinktrader_data_client._client.subscribe_market_data = AsyncMock(return_value=None)
    thinktrader_data_client._client.unsubscribe_market_data = AsyncMock(return_value=None)

    sub = SubscribeQuoteTicks(
        instrument_id=instrument_id,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._subscribe_quote_ticks(sub)
    thinktrader_data_client._client.subscribe_market_data.assert_awaited_once()
    _print_kv("subscribe_market_data.await_args", thinktrader_data_client._client.subscribe_market_data.await_args)

    unsub = UnsubscribeQuoteTicks(
        instrument_id=instrument_id,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._unsubscribe_quote_ticks(unsub)
    thinktrader_data_client._client.unsubscribe_market_data.assert_awaited_once_with(instrument_id)
    _print_kv("unsubscribe_market_data.await_args", thinktrader_data_client._client.unsubscribe_market_data.await_args)


@pytest.mark.asyncio
async def test_data_client_subscribe_and_unsubscribe_trade_ticks_calls_client_methods(thinktrader_data_client):
    _print_section("DataClient 订阅/反订阅 TradeTicks: 调用 client.subscribe_tick_by_tick")
    from nautilus_trader.core.uuid import UUID4
    from nautilus_trader.data.messages import SubscribeTradeTicks
    from nautilus_trader.data.messages import UnsubscribeTradeTicks

    instrument_id = InstrumentId.from_str("000001.SZSE")

    thinktrader_data_client._client.subscribe_tick_by_tick = AsyncMock(return_value=None)
    thinktrader_data_client._client.unsubscribe_tick_by_tick = AsyncMock(return_value=None)

    sub = SubscribeTradeTicks(
        instrument_id=instrument_id,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._subscribe_trade_ticks(sub)
    thinktrader_data_client._client.subscribe_tick_by_tick.assert_awaited_once()
    _print_kv("subscribe_tick_by_tick.await_args", thinktrader_data_client._client.subscribe_tick_by_tick.await_args)

    unsub = UnsubscribeTradeTicks(
        instrument_id=instrument_id,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._unsubscribe_trade_ticks(unsub)
    thinktrader_data_client._client.unsubscribe_tick_by_tick.assert_awaited_once()
    _print_kv("unsubscribe_tick_by_tick.await_args", thinktrader_data_client._client.unsubscribe_tick_by_tick.await_args)


@pytest.mark.asyncio
async def test_data_client_subscribe_and_unsubscribe_bars_calls_client_methods(thinktrader_data_client):
    _print_section("DataClient 订阅/反订阅 Bars: 调用 client.subscribe_realtime_bars")
    from nautilus_trader.core.uuid import UUID4
    from nautilus_trader.data.messages import SubscribeBars
    from nautilus_trader.data.messages import UnsubscribeBars

    bar_type = BarType.from_str("000001.SZSE-1-MINUTE-LAST-EXTERNAL")

    thinktrader_data_client._client.subscribe_realtime_bars = AsyncMock(return_value=None)
    thinktrader_data_client._client.unsubscribe_realtime_bars = AsyncMock(return_value=None)

    sub = SubscribeBars(
        bar_type=bar_type,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._subscribe_bars(sub)
    thinktrader_data_client._client.subscribe_realtime_bars.assert_awaited_once()
    _print_kv("subscribe_realtime_bars.await_args", thinktrader_data_client._client.subscribe_realtime_bars.await_args)

    unsub = UnsubscribeBars(
        bar_type=bar_type,
        client_id=ClientId("THINKTRADER"),
        venue=Venue("THINKTRADER"),
        command_id=UUID4(),
        ts_init=thinktrader_data_client._clock.timestamp_ns(),
        params=None,
    )
    await thinktrader_data_client._unsubscribe_bars(unsub)
    thinktrader_data_client._client.unsubscribe_realtime_bars.assert_awaited_once_with(bar_type)
    _print_kv("unsubscribe_realtime_bars.await_args", thinktrader_data_client._client.unsubscribe_realtime_bars.await_args)
