import asyncio
import csv
import os
from pathlib import Path
from typing import Any
from unittest.mock import Mock

from dotenv import load_dotenv

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import TestClock
from nautilus_trader.model.data import OrderBookDelta
from nautilus_trader.model.identifiers import InstrumentId


env_path = Path(__file__).parents[3] / ".env"
load_dotenv(dotenv_path=env_path)


def _print_section(title: str) -> None:
    print("\n" + "=" * 88)
    print(f"【ThinkTrader 测试】{title}")
    print("=" * 88)


def _try_import_xtdata() -> Any:
    import importlib
    import importlib.util

    if importlib.util.find_spec("xtquant") is None:
        print("未安装 xtquant, 跳过真实行情测试")
        return None
    return importlib.import_module("xtquant.xtdata")


def _prepare_xtdata_data_dir(xtdata_module: Any, data_dir: str) -> None:
    xtdata_module.data_dir = data_dir


def _miniqmt_path() -> str:
    return os.environ.get("MINIQMT_PATH") or "D:\\迅投QMT交易终端财通证券版\\userdata_mini"


def _live_stock_code() -> str:
    return os.environ.get("XT_LIVE_STOCK_CODE") or "000001.SZ"


def _live_instrument_id() -> InstrumentId:
    return InstrumentId.from_str(os.environ.get("XT_LIVE_INSTRUMENT_ID") or "000001.SZSE")


def _live_timeout_seconds() -> float:
    try:
        return float(os.environ.get("XT_LIVE_TIMEOUT", "5.0"))
    except Exception:
        return 5.0


def _is_no_level2_permission(exc: BaseException) -> bool:
    text = str(exc).lower()
    return "no level2 permission" in text or "level2 permission" in text


def _create_thinktrader_client(
    loop: asyncio.AbstractEventLoop,
    miniqmt_path: str,
) -> ThinkTraderClient:
    thinktrader_client = ThinkTraderClient(
        loop=loop,
        logger=Logger("ThinkTraderClientStandalone"),
        miniqmt_path=miniqmt_path,
        session_id=1,
        account_id="",
    )
    thinktrader_client._clock = TestClock()
    thinktrader_client._cache = Mock()
    thinktrader_client._msgbus = Mock()
    thinktrader_client.configure_xtdata_data_dir(miniqmt_path)
    return thinktrader_client


def _flatten(delta: OrderBookDelta) -> dict[str, object]:
    d = OrderBookDelta.to_dict(delta)
    order = d.get("order") or {}
    if not isinstance(order, dict):
        order = {}
    return {
        "type": d.get("type"),
        "instrument_id": d.get("instrument_id"),
        "action": d.get("action"),
        "side": order.get("side"),
        "price": order.get("price"),
        "size": order.get("size"),
        "order_id": order.get("order_id"),
        "flags": d.get("flags"),
        "sequence": d.get("sequence"),
        "ts_event": d.get("ts_event"),
        "ts_init": d.get("ts_init"),
    }


async def _subscribe_level2(
    thinktrader_client: ThinkTraderClient,
    instrument_id: InstrumentId,
    stock_code: str,
) -> str | None:
    try:
        await thinktrader_client.subscribe_tick_by_tick(
            instrument_id=instrument_id,
            stock_code=stock_code,
            tick_type="BidAsk",
        )
        print(f"已订阅 {stock_code} (Level2 BidAsk), 等待数据写入...")
        return "tick_by_tick:BidAsk"
    except RuntimeError as exc:
        print(f"订阅 BidAsk 失败: {exc}")
        if not _is_no_level2_permission(exc):
            raise

    try:
        await thinktrader_client.subscribe_order_book(
            instrument_id=instrument_id,
            stock_code=stock_code,
        )
        print(f"已改用 {stock_code} (Level2 l2quote), 等待数据写入...")
        return "order_book:l2quote"
    except RuntimeError as exc:
        print(f"订阅 l2quote 失败: {exc}")
        return None


async def _unsubscribe_level2(
    thinktrader_client: ThinkTraderClient,
    instrument_id: InstrumentId,
    stock_code: str,
    subscription_mode: str | None,
) -> None:
    if subscription_mode == "tick_by_tick:BidAsk":
        await thinktrader_client.unsubscribe_tick_by_tick(
            instrument_id=instrument_id,
            stock_code=stock_code,
            tick_type="BidAsk",
        )
    elif subscription_mode == "order_book:l2quote":
        await thinktrader_client.unsubscribe_order_book(instrument_id=instrument_id)


async def main():
    _print_section("真实行情: 订阅 Level2(OrderBookDelta) 并写入 CSV 文件 (Standalone)")
    xtdata = _try_import_xtdata()
    if xtdata is None:
        return

    miniqmt_path = _miniqmt_path()
    _prepare_xtdata_data_dir(xtdata, miniqmt_path)
    print(f"miniqmt_path: {miniqmt_path}")

    loop = asyncio.get_running_loop()
    thinktrader_client = _create_thinktrader_client(loop=loop, miniqmt_path=miniqmt_path)

    instrument_id = _live_instrument_id()
    stock_code = _live_stock_code()

    csv_path = (Path(__file__).parent / "thinktrader_level2.csv").resolve()
    print(f"CSV文件路径: {csv_path}")
    print(f"instrument_id: {instrument_id}")
    print(f"stock_code: {stock_code}")

    with csv_path.open("w", newline="", encoding="utf-8") as f:
        wrote_future = loop.create_future()
        ctx = {"writer": None, "count": 0}
        subscription_mode: str | None = None

        def on_data(data):
            if not isinstance(data, OrderBookDelta):
                return

            row = _flatten(data)
            if ctx["writer"] is None:
                ctx["writer"] = csv.DictWriter(f, fieldnames=list(row.keys()))
                ctx["writer"].writeheader()

            ctx["writer"].writerow(row)
            f.flush()

            ctx["count"] += 1
            print(f"写入第 {ctx['count']} 条数据...")

            if ctx["count"] >= 10 and not wrote_future.done():
                wrote_future.set_result(ctx["count"])

        thinktrader_client.register_event_handler("data", on_data)

        timeout = max(_live_timeout_seconds(), 5.0)

        try:
            subscription_mode = await _subscribe_level2(
                thinktrader_client=thinktrader_client,
                instrument_id=instrument_id,
                stock_code=stock_code,
            )
            if subscription_mode is None:
                return

            try:
                await asyncio.wait_for(wrote_future, timeout=timeout)
                print("已收到 10 条数据并写入。稍作等待后退出...")
                await asyncio.sleep(1.0)
            except TimeoutError:
                print("等待 Level2 超时, 可能是当前无实时推送")
        finally:
            try:
                await _unsubscribe_level2(
                    thinktrader_client=thinktrader_client,
                    instrument_id=instrument_id,
                    stock_code=stock_code,
                    subscription_mode=subscription_mode,
                )
                print("已取消订阅")
            except Exception as e:
                print(f"取消订阅时发生错误: {e}")

    if not csv_path.exists():
        print("CSV 文件未生成")


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n用户手动停止")
