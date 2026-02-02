import asyncio
import sys
import os
import signal

# Ensure we can import nautilus_trader from the current root
# sys.path.append(os.getcwd())  # Removed to avoid shadowing installed package

from nautilus_trader.model.identifiers import InstrumentId, Symbol, Venue
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.component import Logger
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock

from nautilus_trader.adapters.thinktrader.config import (
    ThinkTraderDataClientConfig,
    ThinkTraderInstrumentProviderConfig,
)
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory

# User Configuration (Derived from check_connection.py)
MINIQMT_PATH = r'D:\中信证券QMT交易终端仿真\userdata_mini'
ACCOUNT_ID = '10100002780'
TEST_SYMBOL = "600000.SSE"  # Maps to 600000.SH

class TestAdapter:
    def __init__(self):
        self.loop = asyncio.get_event_loop()
        self.clock = LiveClock()
        from nautilus_trader.model.identifiers import TraderId
        self.trader_id = TraderId("TESTER-001")
        self.msgbus = MessageBus(trader_id=self.trader_id, clock=self.clock)
        self.cache = Cache(database=None)
        
        # Configure
        self.config = ThinkTraderDataClientConfig(
            miniqmt_path=MINIQMT_PATH,
            session_id=123456,
            instrument_provider=ThinkTraderInstrumentProviderConfig(
                load_contracts_on_start=False, # Don't load all on start for speed
                sectors=[],
            )
        )

        
        # Create Client
        print(f"Creating Client with path: {MINIQMT_PATH}")
        self.client = ThinkTraderLiveDataClientFactory.create(
            loop=self.loop,
            name="ThinkTraderNode",
            config=self.config,
            msgbus=self.msgbus,
            cache=self.cache,
            clock=self.clock,
        )

    async def run(self):
        print("--- Starting ThinkTrader Adapter Test ---")
        
        # 1. Connect
        print("Connecting...")
        self.client.connect()
        
        # Wait for connection or error
        # In LiveDataClient, the task is added to self.client._tasks
        while not self.client.is_connected:
            # Check if any tasks failed
            for task in list(self.client._tasks):
                if task.done() and task.exception():
                    print(f"❌ Connection task failed: {task.exception()}")
                    return
            await asyncio.sleep(0.1)
        print("Connected.")
        
        # 2. Load Instrument
        print(f"Loading Instrument: {TEST_SYMBOL}...")
        instrument_id = InstrumentId.from_str(TEST_SYMBOL)
        
        # We manually trigger load_ids_async on the provider
        # The client has a reference to the provider, but it's private `_instrument_provider`
        # In a real node, the node controller calls this. We will call it directly for test.
        provider = self.client._instrument_provider
        await provider.load_ids_async([instrument_id])
        
        instrument = self.cache.instrument(instrument_id)
        if instrument:
            print(f"✅ Instrument Loaded: {instrument}")
        else:
            print(f"❌ Failed to load instrument: {TEST_SYMBOL}")
            return

        # 3. Subscribe
        print(f"Subscribing to Ticks for {TEST_SYMBOL}...")
        self.client.subscribe_instrument(instrument_id)
        
        # 4. Listen for data
        print("Listening for data (Press Ctrl+C to stop)...")
        # We need to hook into the msgbus or just override handle_data for visual confirmation?
        # The DataClient calls `self._handle_data(quote_tick)`.
        # Base `LiveMarketDataClient` uses `self._msgbus.publish(tick)`.
        # So we can subscribe to the msgbus.
        
        self.msgbus.subscribe(
            topic=type(None), # Subscribe to all? No, MessageBus uses strict types usually.
            handler=self.on_data_received
        )
        # Note: Nautilus MessageBus subscription is a bit complex to setup manually without a component.
        # Alternatively, we can just monkey-patch the client's `_handle_data` method for visibility.
        
        original_handle = self.client._handle_data
        def print_tick(data):
            print(f"Received Data: {data}")
            original_handle(data)
        
        self.client._handle_data = print_tick

        # Keep running
        try:
            for i in range(10):
                await asyncio.sleep(1)
                print(f"tick {i+1}...")
        except asyncio.CancelledError:
            pass
        finally:
            print("Disconnecting...")
            await self.client.disconnect()
            print("Done.")

    def on_data_received(self, msg):
        print(f"MsgBus: {msg}")

if __name__ == "__main__":
    # Ensure consistent event loop
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    
    test = TestAdapter()
    try:
        loop.run_until_complete(test.run())
    except KeyboardInterrupt:
        pass
    finally:
        loop.close()
