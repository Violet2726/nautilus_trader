import asyncio
import os
from pathlib import Path

from nautilus_trader.adapters.thinktrader.client import ThinkTraderClient
from nautilus_trader.common.component import Logger


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


async def main() -> None:
    _load_dotenv()

    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    session_id = int(os.environ.get("MINIQMT_SESSION_ID", "123456"))

    stock_codes = [os.environ.get("XT_LIVE_STOCK_CODE", "000547.SZ")]

    client = ThinkTraderClient(
        loop=asyncio.get_running_loop(),
        logger=Logger("ThinkTraderContractDownload"),
        miniqmt_path=miniqmt_path,
        session_id=session_id,
        account_id=os.environ.get("MINIQMT_ACCOUNT_ID", ""),
    )
    client.configure_xtdata_data_dir(miniqmt_path)

    for stock_code in stock_codes:
        instrument_type = client.get_instrument_type(stock_code)
        detail = client.get_instrument_detail(stock_code)
        print({"stock_code": stock_code, "type": instrument_type, "detail": detail})


if __name__ == "__main__":
    asyncio.run(main())
