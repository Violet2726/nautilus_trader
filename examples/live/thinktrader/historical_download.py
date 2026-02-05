#!/usr/bin/env python3

import asyncio
import contextlib
import datetime
import json
import os
import secrets
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


def _dict_for_csv(value: Any) -> Any:
    if isinstance(value, (dict, list, tuple)):
        return json.dumps(value, ensure_ascii=False, default=str)
    return value


def _write_csv(path: Path, rows: list[dict[str, Any]], *, columns: list[str] | None = None) -> None:
    if not rows:
        pd.DataFrame(columns=columns or []).to_csv(path, index=False, encoding="utf-8-sig")
        return

    normalized_rows: list[dict[str, Any]] = []
    for row in rows:
        normalized_rows.append({k: _dict_for_csv(v) for k, v in row.items()})

    pd.DataFrame.from_records(normalized_rows, columns=columns).to_csv(path, index=False, encoding="utf-8-sig")


def _parse_dt_env(name: str) -> datetime.datetime | None:
    raw = os.environ.get(name, "").strip()
    if not raw:
        return None

    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%d"):
        with contextlib.suppress(ValueError):
            dt = datetime.datetime.strptime(raw, fmt)
            if fmt == "%Y-%m-%d":
                dt = dt.replace(hour=9, minute=30, second=0)
            return dt.replace(tzinfo=CHINA_TZ)

    with contextlib.suppress(ValueError):
        dt = datetime.datetime.fromisoformat(raw)
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=CHINA_TZ)
        else:
            dt = dt.astimezone(CHINA_TZ)
        return dt

    return None


async def main() -> None:
    _load_dotenv()

    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    session_id = int(os.environ.get("MINIQMT_SESSION_ID", "0") or "0") or (100000 + secrets.randbelow(900000))

    instrument_id = InstrumentId.from_str(
        os.environ.get("XT_LIVE_INSTRUMENT_ID", "000001.SZSE"),
    )
    output_dir = Path(os.environ.get("XT_CSV_DIR", "./csv"))
    output_dir.mkdir(parents=True, exist_ok=True)

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

    bar_spec_str = os.environ.get("XT_BAR_SPEC", "1-MINUTE-LAST").strip() or "1-MINUTE-LAST"
    bar_type = BarType.from_str(f"{instrument_id}-{bar_spec_str}-EXTERNAL")
    period = bar_spec_to_period(bar_type.spec)

    now_cn = datetime.datetime.now(tz=CHINA_TZ)
    start_dt = _parse_dt_env("XT_BARS_START") or (now_cn - datetime.timedelta(days=2))
    end_dt = _parse_dt_env("XT_BARS_END") or now_cn

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
    if isinstance(raw_bars, dict):
        keys = sorted([str(k) for k in raw_bars])
        print(f"raw_bars.keys={keys}")
        for field in ("open", "high", "low", "close", "volume"):
            obj = raw_bars.get(field)
            if obj is None:
                continue
            shape = getattr(obj, "shape", None)
            idx = getattr(obj, "index", None)
            cols = getattr(obj, "columns", None)
            in_index = bool(idx is not None and stock_code in idx)
            in_cols = bool(cols is not None and stock_code in cols)
            print(f"raw_bars[{field}].type={type(obj).__name__} shape={shape} stock_in_index={in_index} stock_in_cols={in_cols}")

    ts_init = int(datetime.datetime.now(tz=datetime.UTC).timestamp() * 1_000_000_000)
    bars = _parse_historical_bars(bar_type=bar_type, stock_code=stock_code, data=raw_bars, ts_init=ts_init)

    tick_end_dt = _parse_dt_env("XT_TICKS_END") or end_dt
    tick_start_dt = _parse_dt_env("XT_TICKS_START") or (tick_end_dt - datetime.timedelta(minutes=1))
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
    if not quote_ticks and not trade_ticks:
        fallback_start_dt = datetime.datetime(
            end_dt.year,
            end_dt.month,
            end_dt.day,
            10,
            0,
            0,
            tzinfo=CHINA_TZ,
        )
        fallback_end_dt = fallback_start_dt + datetime.timedelta(minutes=1)
        fallback_start_ns = int(fallback_start_dt.timestamp() * 1_000_000_000)
        fallback_end_ns = int(fallback_end_dt.timestamp() * 1_000_000_000)
        if (fallback_start_ns, fallback_end_ns) != (tick_start_ns, tick_end_ns):
            await client.download_history_data(
                stock_list=[stock_code],
                period="tick",
                start_time=ns_to_xt_time(fallback_start_ns),
                end_time=ns_to_xt_time(fallback_end_ns),
            )
            fallback_raw_ticks = await client.get_historical_ticks(
                instrument_id=instrument_id,
                stock_code=stock_code,
                start_ns=fallback_start_ns,
                end_ns=fallback_end_ns,
            )
            quote_ticks, trade_ticks = _parse_historical_ticks(
                instrument_id=instrument_id,
                stock_code=stock_code,
                data=fallback_raw_ticks,
                ts_init=ts_init,
            )
            tick_start_dt = fallback_start_dt
            tick_end_dt = fallback_end_dt

    instrument_rows = [instrument.to_dict(instrument) for instrument in instruments]
    bar_rows = [bar.to_dict(bar) for bar in bars]
    trade_tick_rows = [tick.to_dict(tick) for tick in trade_ticks]
    quote_tick_rows = [tick.to_dict(tick) for tick in quote_ticks]

    _write_csv(
        output_dir / "instruments.csv",
        instrument_rows,
        columns=list(instrument_rows[0].keys()) if instrument_rows else None,
    )
    _write_csv(
        output_dir / "bars.csv",
        bar_rows,
        columns=list(bar_rows[0].keys()) if bar_rows else ["type", "bar_type", "open", "high", "low", "close", "volume", "ts_event", "ts_init"],
    )
    _write_csv(
        output_dir / "trade_ticks.csv",
        trade_tick_rows,
        columns=list(trade_tick_rows[0].keys())
        if trade_tick_rows
        else ["type", "instrument_id", "price", "size", "aggressor_side", "trade_id", "ts_event", "ts_init"],
    )
    _write_csv(
        output_dir / "quote_ticks.csv",
        quote_tick_rows,
        columns=list(quote_tick_rows[0].keys())
        if quote_tick_rows
        else ["type", "instrument_id", "bid_price", "ask_price", "bid_size", "ask_size", "ts_event", "ts_init"],
    )
    print(f"CSV 输出目录: {output_dir.resolve()}")
    print(f"instruments={len(instrument_rows)} bars={len(bar_rows)} trade_ticks={len(trade_tick_rows)} quote_ticks={len(quote_tick_rows)}")
    print(f"instrument_id={instrument_id} stock_code={stock_code} bar_period={period}")
    print(f"bars_range={start_dt.isoformat()}..{end_dt.isoformat()} ticks_range={tick_start_dt.isoformat()}..{tick_end_dt.isoformat()}")


if __name__ == "__main__":
    asyncio.run(main())
