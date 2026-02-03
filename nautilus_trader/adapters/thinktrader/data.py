import asyncio
from nautilus_trader.live.data_client import LiveMarketDataClient
from nautilus_trader.model.identifiers import ClientId, InstrumentId
from nautilus_trader.model.objects import Price, Quantity, Money
from nautilus_trader.model.enums import BarAggregation

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.adapters.thinktrader.parsing.data import (
    parse_tick_to_quote_tick,
)
from nautilus_trader.adapters.thinktrader.parsing.instruments import (
    instrument_id_to_stock_code,
)


class ThinkTraderDataClient(LiveMarketDataClient):
    """ThinkTrader 数据客户端"""
    
    def __init__(
        self,
        loop,
        client: ThinkTraderClient,
        msgbus,
        cache,
        clock,
        instrument_provider,
        config,
    ):
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
        
        # 注册回调
        self._client.register_event_handler("data", self._handle_data)
    
    async def _connect(self) -> None:
        if not self._config.skip_trader_login:
            await self._client._connect()
        else:
            self._log.info("Skipping ThinkTrader client connection (market data only mode)")
            self._client._is_connected.set()
            # Yield control to event loop to ensure proper coroutine scheduling
            await asyncio.sleep(0)
        
        # Subscribe to whole market quotes if configured
        if self._config.subscribe_whole_quote:
            self._log.info("Subscribing to whole market quotes (SH, SZ)...")
            # Usually we subscribe to main markets. 
            # Users can customize this list in config if we extend Config, 
            # for now hardcode main markets or use sectors config?
            # XtQuant: ['SH', 'SZ'] for full market.
            code_list = ['SH', 'SZ']
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
        # 订阅记录现在由 client._subscriptions 管理，DataClient 仅需同步状态
        self._subscription_map[instrument_id] = 1 # 占位
    
    async def _unsubscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        if instrument_id in self._subscription_map:
            await self._client.unsubscribe_ticks(instrument_id)
            self._subscription_map.pop(instrument_id)

    async def _subscribe_order_book(self, instrument_id: InstrumentId) -> None:
        stock_code = instrument_id_to_stock_code(instrument_id)
        await self._client.subscribe_order_book(instrument_id, stock_code)

    async def _unsubscribe_order_book(self, instrument_id: InstrumentId) -> None:
        await self._client.unsubscribe_order_book(instrument_id)

    async def _request_quote_ticks(self, request) -> None:
        stock_code = instrument_id_to_stock_code(request.instrument_id)
        start_ns = int(request.start.timestamp() * 1e9) if request.start else 0
        end_ns = int(request.end.timestamp() * 1e9) if request.end else self._clock.timestamp_ns()
        
        data = await self._client.get_historical_ticks(
            instrument_id=request.instrument_id,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )
        
        ticks = self._parse_historical_ticks(request.instrument_id, data, quote_only=True)
        self._handle_quote_ticks(request.instrument_id, ticks, request.id, request.start, request.end)

    async def _request_trade_ticks(self, request) -> None:
        stock_code = instrument_id_to_stock_code(request.instrument_id)
        start_ns = int(request.start.timestamp() * 1e9) if request.start else 0
        end_ns = int(request.end.timestamp() * 1e9) if request.end else self._clock.timestamp_ns()
        
        data = await self._client.get_historical_ticks(
            instrument_id=request.instrument_id,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )
        
        ticks = self._parse_historical_ticks(request.instrument_id, data, trade_only=True)
        self._handle_trade_ticks(request.instrument_id, ticks, request.id, request.start, request.end)

    def _parse_historical_ticks(self, instrument_id, data, quote_only=False, trade_only=False):
        if not data:
            return []
            
        import pandas as pd
        from nautilus_trader.adapters.thinktrader.parsing.data import (
            parse_tick_to_quote_tick,
            parse_tick_to_trade_tick,
        )
        
        # XtQuant returns {field: DF}
        # We need to reconstruct individual ticks.
        # This is expensive for many ticks, but necessary for Nautilus compatibility.
        
        # Combine all fields into one DF for the specific stock_code
        fields = list(data.keys())
        series_list = {}
        stock_code = instrument_id_to_stock_code(instrument_id)
        
        for f in fields:
            if stock_code in data[f].index:
                series_list[f] = data[f].loc[stock_code]
                
        if not series_list:
            return []
            
        df = pd.DataFrame(series_list)
        ticks = []
        ts_init = self._clock.timestamp_ns()
        
        for time_val, row in df.iterrows():
            row_dict = row.to_dict()
            row_dict['time'] = time_val
            
            if not trade_only:
                quote = parse_tick_to_quote_tick(instrument_id, row_dict, ts_init)
                ticks.append(quote)
            if not quote_only:
                trade = parse_tick_to_trade_tick(instrument_id, row_dict, ts_init)
                ticks.append(trade)
                
        return ticks
            

    async def _request_bars(
        self,
        bar_type,
        limit: int,
        correlation_id,
        start=None,
        end=None,
    ) -> None:
        """请求历史 K 线"""
        from nautilus_trader.adapters.thinktrader.parsing.data import (
            parse_kline_to_bar,
            PERIOD_MAP,
        )
        import pandas as pd
        from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        start_ns = int(start.timestamp() * 1e9) if start else 0
        end_ns = int(end.timestamp() * 1e9) if end else int(pd.Timestamp.now().timestamp() * 1e9)
        
        # 使用新的 mixin 方法 (异步且处理了请求队列)
        data = await self._client.get_historical_bars(
            bar_type=bar_type,
            stock_code=stock_code,
            start_ns=start_ns,
            end_ns=end_ns,
        )
        
        bars = []
        if data:
            # Pivot data: {field: DF} -> Single DF with Time index
            series_list = {}
            for field in ['open', 'high', 'low', 'close', 'volume']:
                if field in data and not data[field].empty:
                    # XtQuant DF: index=stock_list, columns=times
                    try:
                        # Extract the row for this stock code
                        if stock_code in data[field].index:
                            series = data[field].loc[stock_code]
                            series.name = field
                            series_list[field] = series
                    except Exception:
                        continue
            
            if series_list:
                df = pd.DataFrame(series_list)
                # df index is now timestamps (int or str)
                
                ts_init = self._clock.timestamp_ns()
                
                for time_val, row in df.iterrows():
                    bar_data = {
                        'time': time_val,
                        'open': row.get('open', 0.0),
                        'high': row.get('high', 0.0),
                        'low': row.get('low', 0.0),
                        'close': row.get('close', 0.0),
                        'volume': row.get('volume', 0),
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

        self._handle_bars(bar_type, bars, correlation_id, start, end)
