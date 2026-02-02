
import asyncio
import unittest
import pandas as pd
from unittest.mock import MagicMock

from nautilus_trader.common.component import MessageBus, LiveClock, Logger
from nautilus_trader.model.identifiers import TraderId, InstrumentId, Venue, Symbol
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.data import ThinkTraderDataClient
from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderInstrumentProviderConfig
)
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.cache.cache import Cache

# Use real XtQuant if available
try:
    from xtquant import xtdata
    XT_AVAILABLE = True
except ImportError:
    XT_AVAILABLE = False

class TestMarketDataLive(unittest.TestCase):
    def setUp(self):
        if not XT_AVAILABLE:
            self.skipTest("xtquant not available")
            
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        
        # Patch XtQuantTrader to avoid loading trade modules
        from unittest.mock import patch
        self.trader_patcher = patch('xtquant.xttrader.XtQuantTrader')
        self.mock_trader_cls = self.trader_patcher.start()
        
        self.trader_id = TraderId("TESTER-001")
        self.clock = LiveClock()
        self.msgbus = MessageBus(trader_id=self.trader_id, clock=self.clock)
        from nautilus_trader.cache.cache import Cache
        self.cache = Cache(database=None)
        
        from nautilus_trader.common.providers import InstrumentProvider
        self.provider = MagicMock(spec=InstrumentProvider)
        
        # Config with skip_trader_login=True
        self.config = ThinkTraderDataClientConfig(
            miniqmt_path="D:\\中信证券QMT交易终端仿真\\userdata_mini", 
            session_id=123456,
            subscribe_whole_quote=False,
            skip_trader_login=True, # NEW FEATURE
            instrument_provider=ThinkTraderInstrumentProviderConfig(load_contracts_on_start=False),
        )

        self.client = ThinkTraderClient(
            loop=self.loop,
            logger=Logger(type(self).__name__),
            miniqmt_path=self.config.miniqmt_path,
            session_id=self.config.session_id,
            account_id="TEST_ACC", 
        )
        
        self.data_client = ThinkTraderDataClient(
            loop=self.loop,
            client=self.client,
            msgbus=self.msgbus,
            cache=self.cache,
            clock=self.clock,
            instrument_provider=self.provider,
            config=self.config,
        )
        
        # Hook handle_data
        self.received_ticks = []
        self.data_client._handle_data = self._handle_data_mock

    def _handle_data_mock(self, data):
        print(f"✅ Received Data: {data}")
        self.received_ticks.append(data)
        
    def tearDown(self):
        try:
            self.loop.run_until_complete(self.data_client.disconnect())
        except Exception:
            pass
        self.loop.close()
        # Stop patcher
        if hasattr(self, 'trader_patcher'):
            self.trader_patcher.stop()

    def test_live_subscription(self):
        print("\n--- Testing Live Subscription (Real xtdata) ---")
        try:
            self.loop.run_until_complete(self.data_client.connect())
            
            # Map symbol
            instrument_id = InstrumentId(Symbol("600000"), Venue("SSE"))
            
            print(f"Subscribing to {instrument_id}...")
            self.loop.run_until_complete(self.data_client.subscribe_quote_ticks(instrument_id))
            
            print("Waiting for tick data (10s)...")
            async def wait_for_data():
                for i in range(20):
                    if self.received_ticks:
                        return
                    await asyncio.sleep(0.5)
            self.loop.run_until_complete(wait_for_data())
            
            if self.received_ticks:
                print("🎉 Success! Received live ticks.")
            else:
                print("⚠️ No data received. (Market closed?)")

            # Check instrument ID
            if self.received_ticks:
                tick = self.received_ticks[0]
                self.assertEqual(tick.instrument_id.symbol.value, "600000")
                self.assertEqual(tick.instrument_id.venue.value, "SSE")

        except Exception as e:
            import traceback
            traceback.print_exc()
            self.fail(str(e))

    def test_historical_request(self):
        print("\n--- Testing Historical Request ---")
        try:
            from nautilus_trader.model.data import BarType, BarSpecification, BarAggregation, PriceType
            
            self.loop.run_until_complete(self.data_client.connect())
            
            instrument_id = InstrumentId(Symbol("600000"), Venue("SSE"))
            bar_spec = BarSpecification(1, BarAggregation.DAY, PriceType.LAST)
            bar_type = BarType(instrument_id, bar_spec)
            
            end = pd.Timestamp.now() - pd.Timedelta(days=1)
            start = end - pd.Timedelta(days=50) # Request 50 days
            
            self.received_bars = []
            self.data_client._handle_bars = self._handle_bars_mock
            
            print(f"Requesting bars for {instrument_id} from {start.date()} to {end.date()}...")
            self.loop.run_until_complete(
                self.data_client.request_bars(
                    bar_type,
                    limit=10,
                    correlation_id="test_hist",
                    start=start,
                    end=end
                )
            )
            
            if self.received_bars:
                print(f"🎉 Success! Received {len(self.received_bars)} bars.")
                print(self.received_bars[0])
            else:
                 print("⚠️ No bars received. Attempting download history...")
                 # Try download
                 xtdata.download_history_data("600000.SH", period="1d", start_time="20240101")
                 self.loop.run_until_complete(
                    self.data_client.request_bars(
                        bar_type,
                        limit=10,
                        correlation_id="test_hist_2",
                        start=start,
                        end=end
                    )
                 )
                 if self.received_bars:
                     print(f"🎉 Success! Received {len(self.received_bars)} bars after download.")
                 else:
                     print("❌ Still no bars.")
                     
        except Exception as e:
            import traceback
            traceback.print_exc()
            self.fail(str(e))

    def _handle_bars_mock(self, bar_type, bars, *args):
        print(f"✅ Received {len(bars)} Bars")
        self.received_bars.extend(bars)

if __name__ == "__main__":
    unittest.main()
