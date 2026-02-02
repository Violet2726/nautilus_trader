import sys
import unittest
import importlib.util
import types
from unittest.mock import MagicMock

# ==============================================================================
# Helper to import by file path
# ==============================================================================
def import_from_path(module_name, file_path):
    spec = importlib.util.spec_from_file_location(module_name, file_path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module

# ==============================================================================
# 1. Mock Nautilus compiled modules GLOBAL SETUP
# ==============================================================================

# Setup package structure
nt_pkg = types.ModuleType("nautilus_trader")
nt_pkg.__path__ = []
sys.modules["nautilus_trader"] = nt_pkg

nt_model = types.ModuleType("nautilus_trader.model")
nt_model.__path__ = []
sys.modules["nautilus_trader.model"] = nt_model
nt_pkg.model = nt_model

# Mock Core
sys.modules["nautilus_trader.core"] = MagicMock()
sys.modules["nautilus_trader.core.data"] = MagicMock()

# Mock Objects
class MockPrice:
    @staticmethod
    def from_str(s): return float(s)
class MockQuantity:
    @staticmethod
    def from_int(i): return int(i)
class MockCurrency:
    @staticmethod
    def from_str(s): return str(s)

mock_objects = MagicMock()
mock_objects.Price = MockPrice
mock_objects.Quantity = MockQuantity
mock_objects.Currency = MockCurrency
sys.modules["nautilus_trader.model.objects"] = mock_objects
nt_model.objects = mock_objects

# Mock Identifiers
mock_ids = MagicMock()
sys.modules["nautilus_trader.model.identifiers"] = mock_ids
nt_model.identifiers = mock_ids

# Mock Enums
mock_enums = MagicMock()
mock_enums.BarAggregation.TICK = "TICK"
sys.modules["nautilus_trader.model.enums"] = mock_enums
nt_model.enums = mock_enums

# Mock Data
class MockQuoteTick:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)
class MockTradeTick:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)
class MockBar:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)

mock_data = MagicMock()
mock_data.QuoteTick = MockQuoteTick
mock_data.TradeTick = MockTradeTick
mock_data.Bar = MockBar
sys.modules["nautilus_trader.model.data"] = mock_data
nt_model.data = mock_data

# Mock Instruments
class MockEquity:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)
class MockFuturesContract:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)
class MockOptionsContract:
    def __init__(self, **kwargs): self.__dict__.update(kwargs)

mock_instr = MagicMock()
mock_instr.Equity = MockEquity
mock_instr.FuturesContract = MockFuturesContract
mock_instr.OptionsContract = MockOptionsContract
sys.modules["nautilus_trader.model.instruments"] = mock_instr
nt_model.instruments = mock_instr

# Mock Common
sys.modules["nautilus_trader.adapters.thinktrader.common"] = MagicMock()
sys.modules["nautilus_trader.adapters.thinktrader.common"].TT_VENUE = "THINKTRADER"


# ==============================================================================
# 2. Import Code Under Test
# ==============================================================================
import os
adapter_dir = os.path.join(os.getcwd(), "nautilus_trader", "adapters", "thinktrader")
parsing_data_path = os.path.join(adapter_dir, "parsing", "data.py")
parsing_instr_path = os.path.join(adapter_dir, "parsing", "instruments.py")

print(f"Importing data parsing from: {parsing_data_path}")
parsing_data = import_from_path("nautilus_trader.adapters.thinktrader.parsing.data", parsing_data_path)

print(f"Importing instrument parsing from: {parsing_instr_path}")
parsing_instr = import_from_path("nautilus_trader.adapters.thinktrader.parsing.instruments", parsing_instr_path)


# ==============================================================================
# 3. Test Cases
# ==============================================================================
class TestThinkTraderParsing(unittest.TestCase):
    
    def test_parse_instrument_equity(self):
        print("Testing Equity parsing...")
        detail = {
            "InstrumentID": "600000",
            "PriceTick": 0.01,
        }
        instrument_id = "600000.SSE" # Mock string
        
        equity = parsing_instr.parse_equity(detail, instrument_id)
        self.assertEqual(equity.instrument_id, instrument_id)
        self.assertEqual(equity.price_increment, 0.01)
        print("✅ Equity parsed successfully")

    def test_parse_tick_to_quote_tick(self):
        print("Testing QuoteTick parsing...")
        xt_tick = {
            "time": 1700000000123, 
            "lastPrice": 10.51,
            "bidPrice": [10.50],
            "askPrice": [10.52],
            "bidVol": [100],
            "askVol": [300],
        }
        instrument_id = "600000.SSE"
        ts_init = 1000
        
        qt = parsing_data.parse_tick_to_quote_tick(instrument_id, xt_tick, ts_init)
        
        self.assertEqual(qt.instrument_id, instrument_id)
        self.assertEqual(qt.bid_price, 10.50)
        self.assertEqual(qt.ask_price, 10.52)
        print("✅ QuoteTick parsed successfully")

    def test_parse_tick_to_trade_tick(self):
        print("Testing TradeTick parsing...")
        xt_tick = {
            "time": 1700000000123,
            "lastPrice": 10.51,
            "volume": 5000,
        }
        instrument_id = "600000.SSE"
        
        tt = parsing_data.parse_tick_to_trade_tick(instrument_id, xt_tick, 0)
        self.assertEqual(tt.price, 10.51)
        print("✅ TradeTick parsed successfully")

if __name__ == "__main__":
    unittest.main()
