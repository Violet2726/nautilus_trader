# coding=utf-8
import time
import random
import os
import sys
import xtquant.xtdata as xtdata
from xtquant.xttrader import XtQuantTrader
from xtquant.xttype import StockAccount

def check_connection():

    
    print(f"当前 Python 版本: {sys.version}")
    
    # 路径
    mini_qmt_path = r'D:\中信证券QMT交易终端仿真\userdata_mini'

    file_path = r"D:\中信证券QMT交易终端仿真\userdata_mini\example.txt"
    with open(file_path, "w") as file:
        file.write("123")  # 向文件写入内容


    account_id = '10100002780'
    session_id = random.randint(100000, 999999)

    print(f"\n=== 1. 基础链路测试 (xtdata) ===")
    try:
        # 尝试获取一个数据
        stocks = xtdata.get_stock_list_in_sector('沪深A股')
        if stocks and len(stocks) > 0:
            print(f"✅ 基础链路正常！已获取到 {len(stocks)} 只股票。")
        else:
            print("❌ 基础链路异常: 获取股票列表为空。")
            print(">> 请确保 QMT 已启动并进入极简模式。")
            return
    except Exception as e:
        print(f"❌ 基础链路测试失败: {e}")
        return

    print(f"\n=== 2. 交易链路测试 (xttrader) ===")
    print(f"目标路径: {mini_qmt_path}")
    print(f"Session ID: {session_id}")

    try:
        xt_trader = XtQuantTrader(mini_qmt_path, session_id)
        xt_trader.start()
        time.sleep(1) # 给一点点初始化时间
        
        print("正在连接交易模块...")
        connect_result = xt_trader.connect()
        
        if connect_result == 0:
            print(f"✅ 交易模块连接成功！")
            
            acc = StockAccount(account_id)
            subscribe_res = xt_trader.subscribe(acc)
            
            if subscribe_res == 0:
                print(f"✅ 账号 {account_id} 订阅成功！")
                asset = xt_trader.query_stock_asset(acc)
                if asset:
                    print(f"✅ 资产验证通过: 总资产={asset.total_asset}")
                else:
                    print("⚠️ 账号已订阅，但查询资产为空（可能需等待同步）")
            else:
                print(f"❌ 账号订阅失败: {subscribe_res}")
        else:
            print(f"❌ 交易连接失败 (错误码: {connect_result})")
            
    except Exception as e:
        print(f"❌ 运行异常: {e}")
    finally:
        if 'xt_trader' in locals():
            xt_trader.stop()

if __name__ == "__main__":
    check_connection()
