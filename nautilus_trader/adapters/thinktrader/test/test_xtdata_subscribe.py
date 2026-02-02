
import time
from xtquant import xtdata

def on_data(datas):
    """
    数据推送回调
    datas 格式: { stock_code : [data1, data2, ...] }
    """
    print("\n[Callback] 收到行情推送:")
    for stock_code in datas:
        print(f"合约: {stock_code}")
        for data in datas[stock_code]:
            print(f"数据详情: {data}")

def test_subscribe():
    # 待订阅的合约代码 (上证: .SH, 深证: .SZ)
    stock_code = "600000.SH" 
    period = "tick" # 订阅 Tick 行情
    
    print(f"正在发起订阅: {stock_code}, 周期: {period}...")
    
    # 调用 xtdata 接口
    # count=1 表示请求最后 1 条历史数据，通常会立即触发回调
    subscribe_id = xtdata.subscribe_quote(
        stock_code=stock_code,
        period=period,
        count=1,
        callback=on_data
    )
    
    if subscribe_id > 0:
        print(f"✅ 订阅成功，订阅号: {subscribe_id}")
        
        # 1. 同步获取快照 (证明实时链路通)
        print("\n[Check 1] 同步获取最新快照 (get_full_tick):")
        full_tick = xtdata.get_full_tick([stock_code])
        print(f"结果: {full_tick}")
        
        # 2. 获取历史日线 (证明历史数据链路通，不受交易时段限制)
        print("\n[Check 2] 获取最近 5 条历史日线 (get_market_data):")
        hist_data = xtdata.get_market_data(
            field_list=['open', 'high', 'low', 'close', 'volume'],
            stock_list=[stock_code],
            period='1d',
            count=10
        )
        print(f"结果预览 (收盘价): \n{hist_data['close']}")
        
        print("\n[Check 3] 启动 xtdata.run() 等待 10 秒看是否有推送 (Ctrl+C 退出)...")
        try:
            # 短暂运行看是否有缓存推送
            xtdata.run()
        except KeyboardInterrupt:
            pass
        
        xtdata.unsubscribe_quote(subscribe_id)
        print("\n测试结束。")
    else:
        print(f"❌ 订阅失败，错误码: {subscribe_id}")

if __name__ == "__main__":
    test_subscribe()
