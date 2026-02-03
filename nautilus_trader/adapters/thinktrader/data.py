import asyncio
from typing import Any
from typing import cast

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.parsing.instruments import instrument_id_to_stock_code
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.data.messages import RequestBars
from nautilus_trader.data.messages import RequestData
from nautilus_trader.data.messages import RequestInstrument
from nautilus_trader.data.messages import RequestInstruments
from nautilus_trader.data.messages import RequestQuoteTicks
from nautilus_trader.data.messages import RequestTradeTicks
from nautilus_trader.data.messages import SubscribeBars
from nautilus_trader.data.messages import SubscribeInstrument
from nautilus_trader.data.messages import SubscribeInstruments
from nautilus_trader.data.messages import SubscribeOrderBook
from nautilus_trader.data.messages import SubscribeQuoteTicks
from nautilus_trader.data.messages import SubscribeTradeTicks
from nautilus_trader.data.messages import UnsubscribeBars
from nautilus_trader.data.messages import UnsubscribeInstrument
from nautilus_trader.data.messages import UnsubscribeInstruments
from nautilus_trader.data.messages import UnsubscribeOrderBook
from nautilus_trader.data.messages import UnsubscribeQuoteTicks
from nautilus_trader.data.messages import UnsubscribeTradeTicks
from nautilus_trader.live.data_client import LiveMarketDataClient
from nautilus_trader.model.data import Bar
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import InstrumentId


class ThinkTraderDataClient(LiveMarketDataClient):
    """ThinkTrader 数据客户端"""

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client: ThinkTraderClient,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        instrument_provider: InstrumentProvider,
        config: ThinkTraderDataClientConfig,
    ) -> None:
        super().__init__(
            loop=loop,
            client_id=ClientId("THINKTRADER"),
            venue=TT_VENUE,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=instrument_provider,
            config=config,
        )
        self._client = client
        self._config = config
        self._client.register_event_handler("data", self._on_client_data)

    def _on_client_data(self, data: Any) -> None:
        self._handle_data(data)

    async def _connect(self) -> None:
        self._client._clock = self._clock
        self._client._cache = self._cache
        self._client._msgbus = self._msgbus
        self._client._instrument_provider = self._instrument_provider

        if not self._config.skip_trader_login:
            await self._client._connect()
        else:
            self._log.info("Skipping ThinkTrader client connection (market data only mode)")
            self._client._is_connected.set()
            await asyncio.sleep(0)

        self._client.configure_xtdata_data_dir(self._config.miniqmt_path)

        await self.instrument_provider.initialize()
        for instrument in self.instrument_provider.list_all():
            self._handle_data(instrument)

        if self._config.subscribe_whole_quote:
            self._log.info("Subscribing to whole market quotes (SH, SZ)...")
            code_list = ["SH", "SZ"]
            self._client.subscribe_whole_quote(code_list)

    async def _disconnect(self) -> None:
        await self._client._disconnect()

    async def _subscribe_instruments(self, command: SubscribeInstruments) -> None:
        return

    async def _subscribe_instrument(self, command: SubscribeInstrument) -> None:
        await self.instrument_provider.load_ids_async([command.instrument_id])
        if instrument := self.instrument_provider.find(command.instrument_id):
            self._handle_data(instrument)

    async def _subscribe_quote_ticks(self, command: SubscribeQuoteTicks) -> None:
        if self._config.subscribe_whole_quote:
            return

        stock_code = instrument_id_to_stock_code(command.instrument_id)
        await self._client.subscribe_market_data(command.instrument_id, stock_code)
        await asyncio.sleep(self._config.subscription_delay_secs)

    async def _subscribe_trade_ticks(self, command: SubscribeTradeTicks) -> None:
        stock_code = instrument_id_to_stock_code(command.instrument_id)
        await self._client.subscribe_tick_by_tick(
            instrument_id=command.instrument_id,
            stock_code=stock_code,
            tick_type="AllLast",
        )
        await asyncio.sleep(self._config.subscription_delay_secs)

    async def _subscribe_order_book_deltas(self, command: SubscribeOrderBook) -> None:
        stock_code = instrument_id_to_stock_code(command.instrument_id)
        await self._client.subscribe_order_book(command.instrument_id, stock_code)
        await asyncio.sleep(self._config.subscription_delay_secs)

    async def _subscribe_order_book_depth(self, command: SubscribeOrderBook) -> None:
        self._log.error(
            f"无法为 {command.instrument_id} 订阅订单簿深度: ThinkTrader 仅支持订单簿增量",
        )

    async def _subscribe_bars(self, command: SubscribeBars) -> None:
        stock_code = instrument_id_to_stock_code(command.bar_type.instrument_id)
        await self._client.subscribe_realtime_bars(bar_type=command.bar_type, stock_code=stock_code)
        await asyncio.sleep(self._config.subscription_delay_secs)

    async def _unsubscribe_instruments(self, command: UnsubscribeInstruments) -> None:
        return

    async def _unsubscribe_instrument(self, command: UnsubscribeInstrument) -> None:
        return

    async def _unsubscribe_quote_ticks(self, command: UnsubscribeQuoteTicks) -> None:
        await self._client.unsubscribe_market_data(command.instrument_id)

    async def _unsubscribe_trade_ticks(self, command: UnsubscribeTradeTicks) -> None:
        stock_code = instrument_id_to_stock_code(command.instrument_id)
        await self._client.unsubscribe_tick_by_tick(
            command.instrument_id, stock_code, tick_type="AllLast"
        )

    async def _unsubscribe_order_book_deltas(self, command: UnsubscribeOrderBook) -> None:
        await self._client.unsubscribe_order_book(command.instrument_id)

    async def _unsubscribe_order_book_depth(self, command: UnsubscribeOrderBook) -> None:
        return

    async def _unsubscribe_bars(self, command: UnsubscribeBars) -> None:
        await self._client.unsubscribe_realtime_bars(command.bar_type)

    async def _request(self, request: RequestData) -> None:
        if isinstance(request, RequestQuoteTicks):
            await self._request_quote_ticks(request)
            return
        if isinstance(request, RequestTradeTicks):
            await self._request_trade_ticks(request)
            return
        if isinstance(request, RequestBars):
            await self._request_bars(request)
            return
        if isinstance(request, RequestInstrument):
            await self._request_instrument(request)
            return
        if isinstance(request, RequestInstruments):
            await self._request_instruments(request)
            return
        self._log.error(f"不支持的请求类型: {type(request)!r}")

    async def _request_instrument(self, request: RequestInstrument) -> None:
        await self.instrument_provider.load_ids_async([request.instrument_id], request.params)
        if instrument := self.instrument_provider.find(request.instrument_id):
            self._handle_data(instrument)
            self._handle_instrument(
                instrument, request.id, request.start, request.end, request.params
            )
        else:
            self._log.warning(f"{request.instrument_id} 的工具不可用")

    async def _request_instruments(self, request: RequestInstruments) -> None:
        await self.instrument_provider.initialize(
            reload=request.params.get("reload", False) if request.params else False
        )
        instruments = list(self.instrument_provider.list_all())
        for instrument in instruments:
            self._handle_data(instrument)
        self._handle_instruments(
            request.venue, instruments, request.id, request.start, request.end, request.params
        )

    async def _request_quote_ticks(self, request: RequestQuoteTicks) -> None:
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time

        stock_code = instrument_id_to_stock_code(request.instrument_id)
        start_ns = int(request.start.timestamp() * 1e9) if request.start else 0
        end_ns = int(request.end.timestamp() * 1e9) if request.end else self._clock.timestamp_ns()

        await self._client.download_history_data(
            stock_list=[stock_code],
            period="tick",
            start_time=ns_to_xt_time(start_ns),
            end_time=ns_to_xt_time(end_ns),
        )

        data = await self._client.get_historical_ticks(
            instrument_id=request.instrument_id,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )

        ticks = self._parse_historical_ticks(request.instrument_id, data, quote_only=True)
        self._handle_quote_ticks(
            request.instrument_id,
            ticks,
            request.id,
            request.start,
            request.end,
            request.params,
        )

    async def _request_trade_ticks(self, request: RequestTradeTicks) -> None:
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time

        stock_code = instrument_id_to_stock_code(request.instrument_id)
        start_ns = int(request.start.timestamp() * 1e9) if request.start else 0
        end_ns = int(request.end.timestamp() * 1e9) if request.end else self._clock.timestamp_ns()

        await self._client.download_history_data(
            stock_list=[stock_code],
            period="tick",
            start_time=ns_to_xt_time(start_ns),
            end_time=ns_to_xt_time(end_ns),
        )

        data = await self._client.get_historical_ticks(
            instrument_id=request.instrument_id,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )

        ticks = self._parse_historical_ticks(request.instrument_id, data, trade_only=True)
        self._handle_trade_ticks(
            request.instrument_id,
            ticks,
            request.id,
            request.start,
            request.end,
            request.params,
        )

    def _parse_historical_ticks(
        self,
        instrument_id: InstrumentId,
        data: Any,
        quote_only: bool = False,
        trade_only: bool = False,
    ) -> list[Any]:
        if not data or not isinstance(data, dict):
            return []

        stock_code = instrument_id_to_stock_code(instrument_id)
        arr = data.get(stock_code)
        if arr is None:
            return []

        try:
            import numpy as np
        except ModuleNotFoundError:
            return []

        if not isinstance(arr, np.ndarray):
            return []

        ts_init = self._clock.timestamp_ns()
        try:
            return self._parse_numpy_tick_array(
                instrument_id=instrument_id,
                arr=arr,
                ts_init=ts_init,
                quote_only=quote_only,
                trade_only=trade_only,
            )
        except Exception as e:
            self._log.warning(f"Failed to parse numpy ticks for {stock_code}: {e}")
            return []

    def _parse_numpy_tick_array(
        self,
        instrument_id: InstrumentId,
        arr: Any,
        ts_init: int,
        quote_only: bool,
        trade_only: bool,
    ) -> list[Any]:
        from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_quote_tick
        from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_trade_tick

        ticks: list[Any] = []
        for row in arr:
            if hasattr(row, "dtype") and getattr(row.dtype, "names", None):
                row_dict = {}
                for k in row.dtype.names:
                    val = row[k]
                    row_dict[k] = val.item() if hasattr(val, "item") else val
            else:
                row_dict = dict(row)

            if not trade_only:
                ticks.append(parse_tick_to_quote_tick(instrument_id, row_dict, ts_init))
            if not quote_only:
                ticks.append(parse_tick_to_trade_tick(instrument_id, row_dict, ts_init))

        return ticks

    async def _request_bars(self, request: RequestBars) -> None:
        import pandas as pd

        from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        from nautilus_trader.adapters.thinktrader.parsing.data import parse_kline_to_bar

        bar_type = request.bar_type
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        start_ns = int(request.start.timestamp() * 1e9) if request.start else 0
        end_ns = int(request.end.timestamp() * 1e9) if request.end else self._clock.timestamp_ns()

        period = bar_spec_to_period(bar_type.spec)
        await self._client.download_history_data(
            stock_list=[stock_code],
            period=period,
            start_time=ns_to_xt_time(start_ns),
            end_time=ns_to_xt_time(end_ns),
        )

        data = await self._client.get_historical_bars(
            bar_type=bar_type,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )

        bars: list[Bar] = []
        if isinstance(data, dict):
            data = cast(dict[str, Any], data)
            series_list: dict[str, Any] = {}
            for field in ("open", "high", "low", "close", "volume"):
                field_df = data.get(field)
                if field_df is None or getattr(field_df, "empty", True):
                    continue
                try:
                    if stock_code in field_df.index:
                        series = field_df.loc[stock_code]
                        series.name = field
                        series_list[field] = series
                except Exception as e:
                    self._log.warning(f"Failed to parse field {field} for {stock_code}: {e}")
                    continue

            if series_list:
                df = pd.DataFrame(series_list)

                ts_init = self._clock.timestamp_ns()

                for time_val, row in df.iterrows():
                    bar_data = {
                        "time": time_val,
                        "open": row.get("open", 0.0),
                        "high": row.get("high", 0.0),
                        "low": row.get("low", 0.0),
                        "close": row.get("close", 0.0),
                        "volume": row.get("volume", 0),
                    }

                    try:
                        bar = parse_kline_to_bar(
                            bar_type.instrument_id,
                            bar_type,
                            bar_data,
                            ts_init,
                        )
                        bars.append(bar)
                    except Exception as e:
                        self._log.error(f"Failed to parse bar for {stock_code} at {time_val}: {e}")
                        continue

        if request.limit > 0 and len(bars) > request.limit:
            bars = bars[-request.limit :]

        self._handle_bars(bar_type, bars, request.id, request.start, request.end, request.params)
        self._msgbus.publish(
            topic=f"requests.{request.id}",
            msg={
                "id": request.id,
                "status": "Success" if bars else "Failed",
            },
        )
