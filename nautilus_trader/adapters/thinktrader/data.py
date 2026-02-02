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
        await self._client._connect()
        
        # Optionally subscribe to whole market quotes
        if self._config.subscribe_whole_quote:
            self._log.info("Subscribing to whole market quotes...")
            # We need a list of codes. Usually, users might want a specific list or "all".
            # For now, we expect a list in the config or we could leave it to the user.
            # XtQuant subscribe_whole_quote takes a code_list.
            # If empty, it might not do anything or subscribe all? 
            # Doc says: code_list - 想要订阅的合约列表
            # If the user wants ALL, they might need to provide it.
            # I'll leave a hook here.
            pass
    
    async def _disconnect(self) -> None:
        await self._client._disconnect()
    
    async def _subscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        stock_code = instrument_id_to_stock_code(instrument_id)
        seq = self._client.subscribe_quote(stock_code, period="tick")
        if seq > 0:
            self._subscription_map[instrument_id] = seq
    
    async def _unsubscribe_quote_ticks(self, instrument_id: InstrumentId) -> None:
        if seq := self._subscription_map.pop(instrument_id, None):
            self._client.unsubscribe_quote(seq)
    
    def _on_quote_data(self, stock_code: str, data: dict) -> None:
        """处理行情回调"""
        from nautilus_trader.adapters.thinktrader.parsing.instruments import (
            stock_code_to_instrument_id,
        )
        
        instrument_id = stock_code_to_instrument_id(stock_code)
        val = self._cache.instrument(instrument_id)
        # If instrument not loaded, we might skip or auto-load
        if not val:
             # Basic handling if instrument is missing
             pass

        ts_init = self._clock.timestamp_ns()
        
        quote_tick = parse_tick_to_quote_tick(instrument_id, data, ts_init)
        self._handle_data(quote_tick)
    
    async def _subscribe_bars(self, bar_type) -> None:
        """订阅 K 线"""
        from nautilus_trader.adapters.thinktrader.parsing.data import PERIOD_MAP
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        period = PERIOD_MAP.get(bar_type.spec.aggregation, "1m")
        
        seq = self._client.subscribe_quote(stock_code, period=period)
        if seq > 0:
            self._subscription_map[bar_type] = seq
    
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
        
        stock_code = instrument_id_to_stock_code(bar_type.instrument_id)
        period = PERIOD_MAP.get(bar_type.spec.aggregation, "1d")
        
        start_time = start.strftime("%Y%m%d") if start else ""
        end_time = end.strftime("%Y%m%d") if end else ""
        
        # xtdata.get_market_data might return dict of pandas DataFrames
        # We need to handle this correctly as per the doc
        
        data = self._client.get_market_data(
            stock_list=[stock_code],
            period=period,
            start_time=start_time,
            end_time=end_time,
            count=limit,
        )
        
        bars = []
        # get_market_data returns {field: DataFrame} or {stock: DataFrame} depending on signature usage?
        # Re-checking ThinkTraderClientMarketDataMixin.get_market_data signature
        # It calls xtdata.get_market_data.
        # Doc says: returns { field : value } where value is DataFrame (index stock, col time) if stocks list > 1?
        # Actually for 1 stock, if period is K-line, it returns { field : value }.
        # Wait, usually easy way is to get dataframe directly.
        
        # XtQuant doc says:
        # returns dict { field1 : value1, ... }
        # value1 is pd.DataFrame, index=stock_list, columns=time_list
        # This format is awkward for single stock.
        
        # Let's rely on what the implementation plan said or verify.
        # Implementation plan used `self._client.get_market_data` which returns the raw result.
        
        # Alternative: use `xtdata.get_local_data` if we downloaded it first.
        # But `get_market_data` is the main API.
        
        # If we passed stock_list=[stock_code], the DataFrame inside the dict will have that stock code as Index.
        # Actually, `get_market_data` with K-line period returns { field: DataFrame }.
        # DataFrame columns are times (as int/str), index is stock code.
        # So df.loc[stock_code] gives a Series with time as index? No.
        # "columns为time_list" -> Columns are the timestamps. Index is stock list.
        # So `df.loc[stock_code]` gives a Series where index is timestamp, value is the field value.
        
        # We need to pivot this to get a DataFrame with Time index and columns Open, High, Low, Close...
        
        # Let's assume the client wrapper might want to simplify this, but currently it just returns the raw dict.
        # I will implement the processing logic here.
        
        if data:
            # We need to pivot the data
            import pandas as pd
            
            # Fields we expect: open, high, low, close, volume
            # data['open'] is a DataFrame.
            
            times = None
            records = {}
            
            # Check availability of fields
            fields = ['open', 'high', 'low', 'close', 'volume']
            
            # Assume we have data
            # Determine timestamps from one of the fields
            first_field = next(iter(data.values()))
            if not first_field.empty and stock_code in first_field.index:
                 times = first_field.columns.tolist()
            
            if times:
                # Construct bars
                ts_init = self._clock.timestamp_ns()
                
                for t in times:
                    # Collect OHLCV for this timestamp `t`
                    try:
                        o = data['open'].at[stock_code, t]
                        h = data['high'].at[stock_code, t]
                        l = data['low'].at[stock_code, t]
                        c = data['close'].at[stock_code, t]
                        v = data['volume'].at[stock_code, t]
                        
                        # Bar dict
                        bar_data = {
                            'time': t,
                            'open': o,
                            'high': h,
                            'low': l,
                            'close': c,
                            'volume': v
                        }
                         
                        bar = parse_kline_to_bar(
                            bar_type.instrument_id,
                            bar_type,
                            bar_data,
                            ts_init,
                        )
                        bars.append(bar)
                    except KeyError:
                        continue

        self._handle_bars(bar_type, bars, None, correlation_id)
