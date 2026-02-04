#!/usr/bin/env python3

import asyncio
import contextlib
import datetime
import os
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.parsing.data import bar_spec_to_period
from nautilus_trader.adapters.thinktrader.parsing.data import ns_to_xt_time
from nautilus_trader.adapters.thinktrader.parsing.data import parse_kline_to_bar
from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_quote_tick
from nautilus_trader.adapters.thinktrader.parsing.data import parse_tick_to_trade_tick
from nautilus_trader.adapters.thinktrader.parsing.instruments import instrument_id_to_stock_code
from nautilus_trader.adapters.thinktrader.providers import ThinkTraderInstrumentProvider
from nautilus_trader.common.component import Logger
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.persistence.catalog import ParquetDataCatalog


CHINA_TZ = datetime.timezone(datetime.timedelta(hours=8))


def _load_dotenv() -> None:
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


def _coerce_xt_time(time_val: Any) -> int | None:
    if isinstance(time_val, pd.Timestamp):
        return int(time_val.strftime("%Y%m%d%H%M%S"))
    try:
        return int(time_val)
    except (TypeError, ValueError):
        return None


def _parse_historical_bars(
    bar_type: BarType,
    stock_code: str,
    data: Any,
    ts_init: int,
) -> list[Bar]:
    if not isinstance(data, dict):
        return []

    series_list: dict[str, Any] = {}
    for field in ("open", "high", "low", "close", "volume"):
        field_df = data.get(field)
        if field_df is None or getattr(field_df, "empty", True):
            continue
        if stock_code not in field_df.index:
            continue
        series = field_df.loc[stock_code]
        series.name = field
        series_list[field] = series

    if not series_list:
        return []

    df = pd.DataFrame(series_list)
    bars: list[Bar] = []

    for time_val, row in df.iterrows():
        xt_time = _coerce_xt_time(time_val)
        if xt_time is None:
            continue

        bar_data = {
            "time": xt_time,
            "open": row.get("open", 0.0),
            "high": row.get("high", 0.0),
            "low": row.get("low", 0.0),
            "close": row.get("close", 0.0),
            "volume": row.get("volume", 0),
        }

        with contextlib.suppress(Exception):
            bars.append(parse_kline_to_bar(bar_type.instrument_id, bar_type, bar_data, ts_init))

    return bars


def _row_to_dict(row: Any) -> dict[str, Any] | None:
    if hasattr(row, "dtype") and getattr(row.dtype, "names", None):
        row_dict: dict[str, Any] = {}
        for k in row.dtype.names:
            val = row[k]
            row_dict[k] = val.item() if hasattr(val, "item") else val
        return row_dict

    try:
        return dict(row)
    except (TypeError, ValueError):
        return None


def _parse_historical_ticks(
    instrument_id: InstrumentId,
    stock_code: str,
    data: Any,
    ts_init: int,
) -> tuple[list[QuoteTick], list[TradeTick]]:
    if not isinstance(data, dict):
        return ([], [])

    arr = data.get(stock_code)
    if not isinstance(arr, np.ndarray):
        return ([], [])

    quotes: list[QuoteTick] = []
    trades: list[TradeTick] = []

    for row in arr:
        row_dict = _row_to_dict(row)
        if row_dict is None:
            continue

        with contextlib.suppress(Exception):
            quotes.append(parse_tick_to_quote_tick(instrument_id, row_dict, ts_init))

        with contextlib.suppress(Exception):
            trades.append(parse_tick_to_trade_tick(instrument_id, row_dict, ts_init))

    return (quotes, trades)


async def main() -> None:
    _load_dotenv()

    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    session_id = int(os.environ.get("MINIQMT_SESSION_ID", "123456"))

    instrument_id = InstrumentId.from_str(
        os.environ.get("XT_LIVE_INSTRUMENT_ID", "000547.SZSE"),
    )

    client = ThinkTraderClient(
        loop=asyncio.get_running_loop(),
        logger=Logger("ThinkTraderHistoricalDownload"),
        miniqmt_path=miniqmt_path,
        session_id=session_id,
        account_id=os.environ.get("MINIQMT_ACCOUNT_ID", ""),
    )
    client.configure_xtdata_data_dir(miniqmt_path)

    instrument_provider = ThinkTraderInstrumentProvider(
        client=client,
        config=ThinkTraderInstrumentProviderConfig(load_all=False, load_ids=frozenset([instrument_id])),
    )
    await instrument_provider.initialize()
    instruments = instrument_provider.list_all()

    stock_code = instrument_id_to_stock_code(instrument_id)

    bar_type = BarType.from_str(f"{instrument_id}-1-HOUR-LAST-EXTERNAL")
    period = bar_spec_to_period(bar_type.spec)

    start_dt = datetime.datetime(2025, 11, 6, 9, 30, tzinfo=CHINA_TZ)
    end_dt = datetime.datetime(2025, 11, 6, 16, 30, tzinfo=CHINA_TZ)

    start_ns = int(start_dt.timestamp() * 1_000_000_000)
    end_ns = int(end_dt.timestamp() * 1_000_000_000)

    await client.download_history_data(
        stock_list=[stock_code],
        period=period,
        start_time=ns_to_xt_time(start_ns),
        end_time=ns_to_xt_time(end_ns),
    )
    raw_bars = await client.get_historical_bars(
        bar_type=bar_type,
        stock_code=stock_code,
        start_ns=start_ns,
        end_ns=end_ns,
    )

    ts_init = int(datetime.datetime.now(tz=datetime.UTC).timestamp() * 1_000_000_000)
    bars = _parse_historical_bars(bar_type=bar_type, stock_code=stock_code, data=raw_bars, ts_init=ts_init)

    tick_start_dt = datetime.datetime(2025, 11, 6, 10, 0, tzinfo=CHINA_TZ)
    tick_end_dt = datetime.datetime(2025, 11, 6, 10, 1, tzinfo=CHINA_TZ)
    tick_start_ns = int(tick_start_dt.timestamp() * 1_000_000_000)
    tick_end_ns = int(tick_end_dt.timestamp() * 1_000_000_000)

    await client.download_history_data(
        stock_list=[stock_code],
        period="tick",
        start_time=ns_to_xt_time(tick_start_ns),
        end_time=ns_to_xt_time(tick_end_ns),
    )
    raw_ticks = await client.get_historical_ticks(
        instrument_id=instrument_id,
        stock_code=stock_code,
        start_ns=tick_start_ns,
        end_ns=tick_end_ns,
    )
    quote_ticks, trade_ticks = _parse_historical_ticks(
        instrument_id=instrument_id,
        stock_code=stock_code,
        data=raw_ticks,
        ts_init=ts_init,
    )

    catalog = ParquetDataCatalog("./catalog")
    catalog.write_data(instruments)
    catalog.write_data(bars)
    catalog.write_data(trade_ticks)
    catalog.write_data(quote_ticks)


if __name__ == "__main__":
    asyncio.run(main())
