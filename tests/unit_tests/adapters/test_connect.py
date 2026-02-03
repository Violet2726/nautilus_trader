
import time
from xtquant import xttrader
from xtquant.xttype import StockAccount

def test_specific_account():
    path = r"D:\迅投QMT交易终端财通证券版\userdata_mini"
    # 使用我们刚刚发现的账号
    account_id = "2007576" 
    session_id = 888888
    
    trader = xttrader.XtQuantTrader(path, session_id)
    trader.start()
    res = trader.connect()
    
    if res == 0:
        print(f"连接成功！尝试订阅账号 {account_id}...")
        
        acc = StockAccount(account_id, 'STOCK')
        res_sub = trader.subscribe(acc)
        
        if res_sub == 0:
            print(f"账号 {account_id} 订阅成功。")
            time.sleep(2)
            
            print(f"\n--- 查询资产 ---")
            asset = trader.query_stock_asset(acc)
            if asset:
                print(f"可用资金: {asset.cash}")
                print(f"总资产: {asset.total_asset}")
                print(f"持仓市值: {asset.market_value}")
            else:
                print("查询资产返回 None")
                
            print(f"\n--- 查询持仓 ---")
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
