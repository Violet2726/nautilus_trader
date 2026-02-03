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
from nautilus_trader.data.messages import RequestQuoteTicks
from nautilus_trader.data.messages import RequestTradeTicks
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
        self._subscription_map: dict[InstrumentId, int] = {}

        self._client.register_event_handler("data", self._on_client_data)

    def _on_client_data(self, data: Any) -> None:
        self._handle_data(data)

    async def _connect(self) -> None:
        if not self._config.skip_trader_login:
            await self._client._connect()
        else:
            self._log.info("Skipping ThinkTrader client connection (market data only mode)")
            self._client._is_connected.set()
            # Yield control to event loop to ensure proper coroutine scheduling
            await asyncio.sleep(0)

        self._client.configure_xtdata_data_dir(self._config.miniqmt_path)

        # Subscribe to whole market quotes if configured
        if self._config.subscribe_whole_quote:
            self._log.info("Subscribing to whole market quotes (SH, SZ)...")
            # Usually we subscribe to main markets.
            # Users can customize this list in config if we extend Config,
            # for now hardcode main markets or use sectors config?
            # XtQuant: ['SH', 'SZ'] for full market.
            code_list = ["SH", "SZ"]
            self._client.subscribe_whole_quote(code_list)

    async def _disconnect(self) -> None:
        await self._client._disconnect()

    async def _subscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        # If whole quote is enabled, we don't need individual subscription usually,
        # UNLESS the user wants to ensure historical cache for this specific instrument is active?
        # But 'subscribe_quote' is for L1. 'subscribe_whole_quote' is also L1.
        if self._config.subscribe_whole_quote:
            return

        stock_code = instrument_id_to_stock_code(instrument_id)
        # 使用新的 mixin 方法
        await self._client.subscribe_ticks(instrument_id, stock_code)
        # 订阅记录现在由 client._subscriptions 管理, DataClient 仅需同步状态
        self._subscription_map[instrument_id] = 1  # 占位

    async def _unsubscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        if instrument_id in self._subscription_map:
            await self._client.unsubscribe_ticks(instrument_id)
            self._subscription_map.pop(instrument_id)

    async def _subscribe_order_book(self, instrument_id: InstrumentId) -> None:
        stock_code = instrument_id_to_stock_code(instrument_id)
        await self._client.subscribe_order_book(instrument_id, stock_code)

    async def _unsubscribe_order_book(self, instrument_id: InstrumentId) -> None:
        await self._client.unsubscribe_order_book(instrument_id)

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
        if not data:
            return []

        from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_quote_tick
        from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_trade_tick

        stock_code = instrument_id_to_stock_code(instrument_id)
        ts_init = self._clock.timestamp_ns()

        if isinstance(data, dict) and stock_code in data:
            try:
                import numpy as np

                arr = data.get(stock_code)
                if isinstance(arr, np.ndarray):
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
            except Exception:
                pass

        if not isinstance(data, dict):
            return []

        return []


    async def _request_bars(self, request: RequestBars) -> None:
        from nautilus_trader.adapters.thinktrader.parsing.data import parse_kline_to_bar
        from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        import pandas as pd

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
