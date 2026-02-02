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
        self._client.register_event_handler("quote_data", self._on_quote_data)
    
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
        seq = self._client.subscribe_quote(stock_code, period="tick")
        if seq > 0:
            self._subscription_map[instrument_id] = seq
    
    async def _unsubscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        if self._config.subscribe_whole_quote:
            return

        if seq := self._subscription_map.pop(instrument_id, None):
            self._client.unsubscribe_quote(seq)
            
    def _on_quote_data(self, stock_code: str, data: dict) -> None:
        """处理行情回调"""
        from nautilus_trader.adapters.thinktrader.parsing.instruments import (
            stock_code_to_instrument_id,
        )
        
        # XtQuant tick data sometimes comes without instrument ID in the dict itself
        # But we have stock_code from the handler argument.
        
        instrument_id = stock_code_to_instrument_id(stock_code)
        
        # In a real whole-market scenario, we might receive ticks for instruments 
        # that Nautilus hasn't loaded. We should check if we care about this instrument.
        # However, checking cache for every tick might be specific.
        # Usually DataClient filters by subscriptions. 
        # But if subscribe_whole_quote is True, we essentially subscribe to everything.
        # So we should pass it to kernel. Kernel will discard if not subscribed?
        # Actually LiveMarketDataClient typically handles subscription mapping.
        # If we use whole quote, we bypass explicit subscription map checks?
        
        if self._config.subscribe_whole_quote:
             # If whole quote, we push everything? Or check if instrument is known?
             # Checking if instrument is in cache is good practice to avoid pollution
             if not self._cache.instrument(instrument_id):
                 return
        
        ts_init = self._clock.timestamp_ns()
        
        quote_tick = parse_tick_to_quote_tick(instrument_id, data, ts_init)
        self._handle_data(quote_tick)

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
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        period = PERIOD_MAP.get(bar_type.spec.aggregation, "1d")
        
        start_time = start.strftime("%Y%m%d") if start else ""
        end_time = end.strftime("%Y%m%d") if end else ""
        
        data = self._client.get_market_data(
            stock_list=[stock_code],
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=limit,
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

        self._handle_bars(bar_type, bars, None, correlation_id)
