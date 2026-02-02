# ThinkTrader (XtQuant) Adapter for Nautilus Trader

This adapter integrates the ThinkTrader (XtQuant / MiniQmt) platform into Nautilus Trader. It provides both market data and execution capabilities for the Chinese domestic markets (Stock and Futures).

## Features

- **Live Market Data**: Real-time tick data and bar data via MiniQmt.
- **Historical Data**: Fetching historical bars using `xtdata`.
- **Execution**: Support for Stock buying/selling, includes order tracking and trade reporting.
- **Instrument Provider**: Automatically loads instrument details from MiniQmt sectors.

## Requirements

- **Windows Environment**: XtQuant/MiniQmt requires Windows (or potentially Wine on Linux, but not officially supported for all features).
- **MiniQmt Client**: The MiniQmt terminal must be running and logged into a broker account.
- **Python Dependencies**:
  - `xtquant`
  - `nautilus_trader`
- **Compiler**: Rust toolchain and `maturin` are required to build Nautilus Trader from source if not using a pre-built wheel.

## Configuration

### Data Client
```python
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig

config = ThinkTraderDataClientConfig(
    miniqmt_path=r"C:\QuantTerminal\userdata_mini",
    session_id=123456,
)
```

### Execution Client
```python
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig

config = ThinkTraderExecClientConfig(
    miniqmt_path=r"C:\QuantTerminal\userdata_mini",
    account_id="your_account_id",
    account_type="STOCK", # "STOCK" or "FUTURE"
)
```

## Structure

- `client/`: Low-level wrapper for XtQuant API.
- `parsing/`: Logic for converting between XtQuant and Nautilus data models.
- `data.py`: `ThinkTraderDataClient` implementation.
- `execution.py`: `ThinkTraderExecutionClient` implementation.
- `providers.py`: `ThinkTraderInstrumentProvider` implementation.
- `factories.py`: Factory classes for easy instantiation.

## Testing

1. **Standalone Test**: Verifies parsing logic without requiring a full Nautilus build or live connection.
   ```powershell
   python nautilus_trader/adapters/thinktrader/test/test_parsing_standalone.py
   ```
2. **Integration Test**: Requires a live MiniQmt connection.
   ```powershell
   python nautilus_trader/adapters/thinktrader/test/test_adapter_integration.py
   ```

## Known Limitations

- **Order Types**: Currently optimized for Limit and Market orders.
- **Market Coverage**: Supports SH, SZ, BJ for stocks; SHFE, DCE, CZCE, CFFEX, INE, GFEX for futures.
- **Thread Safety**: XtQuant callbacks are handled via `loop.call_soon_threadsafe` to ensure they run on the Nautilus event loop.
