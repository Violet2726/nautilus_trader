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
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.identifiers import InstrumentId

# Load .env
env_path = Path(__file__).parents[3] / ".env"
load_dotenv(dotenv_path=env_path)


def _print_section(title: str) -> None:
    print("\n" + "=" * 88)
    print(f"【ThinkTrader 测试】{title}")
    print("=" * 88)


def _try_import_xtdata() -> Any:
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


async def main():
    _print_section("真实行情: 订阅 tick 并写入 CSV 文件")
    xtdata = _try_import_xtdata()
    if xtdata is None:
        return

    miniqmt_path = _miniqmt_path()
    _prepare_xtdata_data_dir(xtdata, miniqmt_path)

    loop = asyncio.get_running_loop()
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

    instrument_id = _live_instrument_id()
    stock_code = _live_stock_code()
    
    csv_path = (Path(__file__).parent / "thinktrader_ticks.csv").resolve()
    print(f"CSV文件路径: {csv_path}")

    wrote_future = loop.create_future()
    ctx = {"writer": None, "count": 0}

    f = csv_path.open("w", newline="", encoding="utf-8")
    
    try:
        def on_data(data):
            if not isinstance(data, QuoteTick):
                return

            row = QuoteTick.to_dict(data)
            
            if ctx["writer"] is None:
                ctx["writer"] = csv.DictWriter(f, fieldnames=list(row.keys()))
                ctx["writer"].writeheader()
            
            ctx["writer"].writerow(row)
            f.flush()
            
            ctx["count"] += 1
            print(f"写入第 {ctx['count']} 条数据...")
            
            if ctx["count"] >= 10 and not wrote_future.done():
                loop.call_soon_threadsafe(wrote_future.set_result, ctx["count"])

        thinktrader_client.register_event_handler("data", on_data)

        try:
            await thinktrader_client.subscribe_ticks(
                instrument_id=instrument_id,
                stock_code=stock_code,
            )
            print(f"已订阅 {stock_code}，等待数据写入...")

            try:
                await asyncio.wait_for(wrote_future, timeout=60.0)
                print("已收到 10 条数据并写入。稍作等待后退出...")
                await asyncio.sleep(1.0)
            except TimeoutError:
                print("等待 QuoteTick 超时, 可能是当前无实时推送")
        except RuntimeError as exc:
            print(f"适配器订阅失败: {exc}")
        finally:
            # 尝试取消订阅（即使前面报错了也要尝试）
            try:
                await thinktrader_client.unsubscribe_ticks(instrument_id=instrument_id)
                print("已取消订阅")
            except Exception as e:
                print(f"取消订阅时发生错误 (可忽略): {e}")

    finally:
        f.close()

    if not csv_path.exists():
        print("CSV 文件未生成")


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n用户手动停止")
