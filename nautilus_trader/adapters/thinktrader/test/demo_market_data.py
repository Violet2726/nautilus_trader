# coding=utf-8
import time
import pandas as pd
import xtquant.xtdata as xtdata

def on_data_callback(datas):
    """
    实时行情回调函数
    datas: {stock_code: [data_list]}
    """
    for stock, data_list in datas.items():
        if data_list:
            # 打印最新的一笔 Tick 数据
            latest = data_list[-1]
            print(f"🔔 [回调] {stock} | 时间: {time.strftime('%H:%M:%S', time.localtime(latest['time']/1000))} | 价格: {latest['lastPrice']} | 数量: {latest['volume']}")

def test_market_data():
    print("=== XtQuant 行情模块综合测试 ===")
    
    # 1. 基础信息
    print("\n[1] 获取板块列表...")
    sectors = xtdata.get_sector_list()
    print(f"板块数量: {len(sectors)}")
    
    target_stock = '600000.SH'  # 浦发银行
    print(f"目标标的: {target_stock}")

    # 2. 实时快照 (Tick)
    print("\n[2] 获取实时快照 (Tick)...")
    ticks = xtdata.get_full_tick([target_stock])
    if target_stock in ticks:
        tick = ticks[target_stock]
        print(f"名称: {tick.get('stockName')}") # 注意：某些版本 key 可能是 stock_name
        print(f"最新价: {tick['lastPrice']}")
        print(f"买一: {tick['bidPrice'][0]} | 卖一: {tick['askPrice'][0]}")
    else:
        print("❌ 未获取到快照数据")

    # 3. 历史数据下载与读取
    print("\n[3] 历史 K 线测试 (日线)...")
    # 先下载最近 10 天的数据 (必要步骤，否则 get_market_data 可能为空)
    print("正在下载数据...")
    # 下载 2026 年以来的数据
    xtdata.download_history_data(target_stock, period='1d', start_time='20250101', end_time='20260202')
    
    print("读取本地数据...")
    # 获取所有常见字段
    fields = ['time', 'open', 'high', 'low', 'close', 'volume']
    hl_data = xtdata.get_market_data(fields, [target_stock], period='1d', count=5)
    
    if hl_data:
        # 简单转换一下格式以便查看
        # get_market_data 返回格式: {field: DataFrame(index=stock, columns=time)}
        print("最近 5 日 K 线:")
        # 我们取 'close' 字段看看
        if 'close' in hl_data:
            print(hl_data['close'])
    else:
        print("❌ 历史数据为空")

    # 4. 实时订阅
    print("\n[4] 实时行情订阅测试 (监听 5 秒)...")
    # 订阅 Tick 数据
    seq = xtdata.subscribe_quote(target_stock, period='tick', start_time='', end_time='', count=0, callback=on_data_callback)
    print(f"订阅号: {seq}")
    
    time.sleep(5)
    
    # 取消订阅
    xtdata.unsubscribe_quote(seq)

    print("\n[5] 合约详情测试...")
    try:
        # 测试获取合约详细信息
        detail = xtdata.get_instrument_detail('600000.SH')
        if detail:
            print(f"✅ 合约详情: {detail}")
        else:
            print("⚠️ 合约详情为空 (可能是仿真环境数据缺失)")

        # 测试获取主力合约 (如果是期货)
        main_contract = xtdata.get_main_contract('IF00.IF')
        print(f"股指期货主力: {main_contract}")
            
    except Exception as e:
        print(f"❌ 详情测试异常: {e}")

    print("\n[6] 财务数据测试...")
    try:
        # 获取财务数据 (例如流通股本等)
        # Note: 仿真环境可能不支持财务数据下载
        stock = '600000.SH'
        print(f"正在下载 {stock} 财务数据...")
        xtdata.download_financial_data([stock])
        
        data = xtdata.get_financial_data([stock])
        if data:
            print(f"✅ 获取到财务数据 keys: {list(data.get(stock, {}).keys())}")
        else:
            print("⚠️ 财务数据为空")
            
    except Exception as e:
        print(f"❌ 财务数据异常: {e}")

    print("\n[7] 全推行情测试 (仅订阅 2 秒)...")
    
    count = 0
    def on_whole_quote(datas):
        nonlocal count
        count += len(datas)
        # 只打印前3个收到的以证明收到
        first_key = next(iter(datas))
        if count % 100 == 0: # 减少打印频率
             print(f"收到全推: {first_key} 等 {len(datas)} 只股票 update (Total: {count})")

    try:
        # 订阅沪深全市场
        seq = xtdata.subscribe_whole_quote(['SH', 'SZ'], callback=on_whole_quote)
        print(f"全推订阅号: {seq}")
        time.sleep(2)
        xtdata.unsubscribe_quote(seq)
        print(f"全推测试结束，共收到 {count} 条更新")
        
    except Exception as e:
        print(f"❌ 全推异常: {e}")

    print("测试结束。")

if __name__ == "__main__":
    test_market_data()
