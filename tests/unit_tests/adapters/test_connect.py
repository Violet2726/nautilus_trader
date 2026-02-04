import os
import time
from pathlib import Path

import pytest
from dotenv import load_dotenv


# 自动加载项目根目录下的 .env 文件
env_path = Path(__file__).parents[3] / ".env"
load_dotenv(dotenv_path=env_path)


def test_specific_account():
    try:
        from xtquant import xttrader
        from xtquant.xttype import StockAccount
    except ModuleNotFoundError:
        pytest.skip("未安装 xtquant, 跳过连接测试")

    if not (os.environ.get("MINIQMT_PATH") and os.environ.get("MINIQMT_ACCOUNT_ID")):
        pytest.skip("需设置 MINIQMT_PATH 与 MINIQMT_ACCOUNT_ID 才能运行此用例")

    path = os.environ["MINIQMT_PATH"]
    account_id = os.environ["MINIQMT_ACCOUNT_ID"]
    session_id = int(os.environ.get("MINIQMT_SESSION_ID", "888888"))

    trader = xttrader.XtQuantTrader(path, session_id)
    trader.start()
    res = trader.connect()

    if res == 0:
        print(f"连接成功! 尝试订阅账号 {account_id}...")

        acc = StockAccount(account_id, "STOCK")
        res_sub = trader.subscribe(acc)

        if res_sub == 0:
            print(f"账号 {account_id} 订阅成功。")
            time.sleep(2)

            print("\n--- 查询资产 ---")
            asset = trader.query_stock_asset(acc)
            if asset:
                print(f"可用资金 (cash): {asset.cash}")
                print(f"总资产 (total_asset): {asset.total_asset}")
                print(f"持仓市值 (market_value): {asset.market_value}")
                # 打印所有底层属性以供调试
                print("底层属性:")
                for attr in dir(asset):
                    if attr.startswith("m_") or attr in [
                        "cash",
                        "total_asset",
                        "market_value",
                        "frozen_cash",
                    ]:
                        print(f"  {attr}: {getattr(asset, attr)}")
            else:
                print("查询资产返回 None")

            print("\n--- 查询持仓 ---")
            positions = trader.query_stock_positions(acc)
            if positions:
                print(f"发现 {len(positions)} 个持仓")
                for p in positions:
                    print(f"  代码: {p.stock_code}, 数量: {p.volume}, 市值: {p.market_value}")
            else:
                print("未发现持仓")
        else:
            print(f"账号订阅失败: {res_sub}")
    else:
        print(f"连接失败: {res}")

    trader.stop()


if __name__ == "__main__":
    test_specific_account()
