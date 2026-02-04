import asyncio
from typing import Any
from unittest.mock import Mock

import pytest

from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.execution import ThinkTraderExecutionClient
from nautilus_trader.common.factories import OrderFactory
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.objects import Quantity
from nautilus_trader.test_kit.providers import TestInstrumentProvider
from nautilus_trader.test_kit.stubs.commands import TestCommandStubs
from nautilus_trader.test_kit.stubs.identifiers import TestIdStubs


class StubInstrumentProvider(InstrumentProvider):
    async def load_all_async(self, filters: Any = None) -> None:
        return None

    async def load_ids_async(self, instrument_ids: Any, filters: Any = None) -> None:
        return None

    async def load_async(self, instrument_id: Any, filters: Any = None) -> None:
        return None


@pytest.fixture
def instrument_provider() -> StubInstrumentProvider:
    return StubInstrumentProvider()


@pytest.fixture
def client() -> Mock:
    client = Mock()
    client.cancel_order = Mock(return_value=1)
    client.cancel_order_async = Mock(return_value=1)
    client.cancel_order_by_sysid = Mock(return_value=0)
    client.cancel_order_by_sysid_async = Mock(return_value=1)
    client.query_orders = Mock(return_value=[])
    client.register_event_handler = Mock()
    return client


def _make_exec_client(
    *,
    event_loop: Any,
    client: Any,
    msgbus: Any,
    cache: Any,
    clock: Any,
    instrument_provider: InstrumentProvider,
    use_async_cancel: bool,
) -> ThinkTraderExecutionClient:
    config = ThinkTraderExecClientConfig(
        miniqmt_path="D:\\miniqmt",
        account_id="000000",
        session_id=1,
        account_type="STOCK",
        use_async_order=True,
        use_async_cancel=use_async_cancel,
        relaxed_response_order=True,
    )
    return ThinkTraderExecutionClient(
        loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        config=config,
    )


@pytest.mark.asyncio
async def test_cancel_order_async_stores_seq_mapping(event_loop, client, msgbus, cache, clock, instrument_provider):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=True,
    )

    instrument_id = InstrumentId.from_str("000001.SZSE")
    client_order_id = ClientOrderId("COID-1")
    order_id = 123

    order = OrderFactory(
        trader_id=TestIdStubs.trader_id(),
        strategy_id=TestIdStubs.strategy_id(),
        clock=clock,
    ).market(
        instrument_id=instrument_id,
        order_side=OrderSide.BUY,
        quantity=Quantity.from_int(1),
        client_order_id=client_order_id,
    )
    cache.add_order(order)

    exec_client._client_order_id_to_order_id[client_order_id] = order_id

    client.cancel_order_async.return_value = 42

    command = TestCommandStubs.cancel_order_command(
        instrument_id=instrument_id,
        client_order_id=client_order_id,
        venue_order_id=None,
    )

    await exec_client._cancel_order(command)

    client.cancel_order_async.assert_called_once_with(order_id)
    assert exec_client._cancel_seq_to_client_order_id[42] == client_order_id


@pytest.mark.asyncio
async def test_cancel_order_async_rejected_when_seq_non_positive(
    event_loop,
    client,
    msgbus,
    cache,
    clock,
    instrument_provider,
):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=True,
    )
    exec_client._on_cancel_rejected = Mock()

    instrument_id = InstrumentId.from_str("000001.SZSE")
    client_order_id = ClientOrderId("COID-2")
    order_id = 456

    order = OrderFactory(
        trader_id=TestIdStubs.trader_id(),
        strategy_id=TestIdStubs.strategy_id(),
        clock=clock,
    ).market(
        instrument_id=instrument_id,
        order_side=OrderSide.BUY,
        quantity=Quantity.from_int(1),
        client_order_id=client_order_id,
    )
    cache.add_order(order)

    exec_client._client_order_id_to_order_id[client_order_id] = order_id

    client.cancel_order_async.return_value = 0

    command = TestCommandStubs.cancel_order_command(
        instrument_id=instrument_id,
        client_order_id=client_order_id,
        venue_order_id=None,
    )

    await exec_client._cancel_order(command)

    exec_client._on_cancel_rejected.assert_called_once()
    called_order_id, reason = exec_client._on_cancel_rejected.call_args.args
    assert called_order_id == order_id
    assert "撤单请求失败" in reason
    assert exec_client._cancel_seq_to_client_order_id == {}


def test_on_position_update_emits_report_and_instrument_to_data_engine(
    event_loop,
    client,
    msgbus,
    cache,
    clock,
    instrument_provider,
):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=False,
    )
    exec_client._send_position_status_report = Mock()

    instrument = TestInstrumentProvider.equity(symbol="000001", venue="SZSE")
    instrument_id = instrument.id

    received = []
    msgbus.register("DataEngine.process", received.append)
    instrument_provider._instruments[instrument_id] = instrument

    position = Mock(stock_code="000001.SZ", volume=100, avg_price=10.25)

    exec_client._on_position_update(position)

    assert msgbus.sent_count == 1
    assert received == [instrument]

    exec_client._send_position_status_report.assert_called_once()
    report = exec_client._send_position_status_report.call_args.args[0]
    assert report.instrument_id == instrument_id


@pytest.mark.asyncio
async def test_modify_order_generates_modify_rejected(event_loop, client, msgbus, cache, clock, instrument_provider):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=False,
    )
    exec_client.generate_order_modify_rejected = Mock()

    instrument_id = InstrumentId.from_str("000001.SZSE")
    client_order_id = ClientOrderId("COID-3")

    order = OrderFactory(
        trader_id=TestIdStubs.trader_id(),
        strategy_id=TestIdStubs.strategy_id(),
        clock=clock,
    ).market(
        instrument_id=instrument_id,
        order_side=OrderSide.BUY,
        quantity=Quantity.from_int(1),
        client_order_id=client_order_id,
    )
    cache.add_order(order)

    command = TestCommandStubs.modify_order_command(
        instrument_id=instrument_id,
        client_order_id=client_order_id,
        venue_order_id=None,
        quantity=Quantity.from_int(2),
    )

    await exec_client._modify_order(command)

    exec_client.generate_order_modify_rejected.assert_called_once()
    kwargs = exec_client.generate_order_modify_rejected.call_args.kwargs
    assert kwargs["strategy_id"] == command.strategy_id
    assert kwargs["instrument_id"] == command.instrument_id
    assert kwargs["client_order_id"] == command.client_order_id
    assert kwargs["venue_order_id"] == VenueOrderId(command.client_order_id.value)
    assert kwargs["reason"] == "ThinkTrader 暂不支持修改订单, 请撤单后重下"
    assert isinstance(kwargs["ts_event"], int)


@pytest.mark.asyncio
async def test_on_order_update_partially_filled_sends_order_status_report(event_loop, client, msgbus, cache, clock, instrument_provider):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=False,
    )
    exec_client._send_order_status_report = Mock()

    instrument_id = InstrumentId.from_str("000001.SZSE")
    client_order_id = ClientOrderId("COID-ORDER-UPDATE-1")
    order_id = 10

    order = OrderFactory(
        trader_id=TestIdStubs.trader_id(),
        strategy_id=TestIdStubs.strategy_id(),
        clock=clock,
    ).market(
        instrument_id=instrument_id,
        order_side=OrderSide.BUY,
        quantity=Quantity.from_int(100),
        client_order_id=client_order_id,
    )
    cache.add_order(order)

    xt_order = Mock(
        order_id=order_id,
        order_sysid="SYS-1",
        stock_code="000001.SZ",
        order_type=1,
        order_status=55,
        order_volume=100,
        traded_volume=10,
        traded_price=10.25,
        price=10.25,
        price_type=0,
        order_time=1_700_000_000,
        order_remark=str(client_order_id),
    )

    exec_client._on_order_update(xt_order)
    await asyncio.sleep(0)

    assert exec_client._order_id_to_client_order_id[order_id] == client_order_id
    assert exec_client._client_order_id_to_order_id[client_order_id] == order_id

    exec_client._send_order_status_report.assert_called_once()
    report = exec_client._send_order_status_report.call_args.args[0]
    assert report.client_order_id == client_order_id


def test_on_trade_recovers_order_id_mapping_from_order_remark(event_loop, client, msgbus, cache, clock, instrument_provider):
    exec_client = _make_exec_client(
        event_loop=event_loop,
        client=client,
        msgbus=msgbus,
        cache=cache,
        clock=clock,
        instrument_provider=instrument_provider,
        use_async_cancel=False,
    )
    exec_client.generate_order_filled = Mock()

    instrument_id = InstrumentId.from_str("000001.SZSE")
    client_order_id = ClientOrderId("COID-TRADE-1")
    order_id = 99

    order = OrderFactory(
        trader_id=TestIdStubs.trader_id(),
        strategy_id=TestIdStubs.strategy_id(),
        clock=clock,
    ).market(
        instrument_id=instrument_id,
        order_side=OrderSide.BUY,
        quantity=Quantity.from_int(1),
        client_order_id=client_order_id,
    )
    cache.add_order(order)

    trade = Mock(
        order_id=order_id,
        order_sysid="SYS-99",
        stock_code="000001.SZ",
        traded_id="T-1",
        traded_price=10.0,
        traded_volume=1,
        traded_time=1_700_000_000,
        order_remark=str(client_order_id),
    )

    exec_client._on_trade(trade)

    assert exec_client._order_id_to_client_order_id[order_id] == client_order_id
    assert exec_client._client_order_id_to_order_id[client_order_id] == order_id
    exec_client.generate_order_filled.assert_called_once()
