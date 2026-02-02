# coding=utf-8
import xtquant.xtdata as xtdata
import time

def test_xtdata():
    print("=== 测试 xtdata 行情模块 (针对 QMT 内置库) ===")
    
    try:
        # 在内置库版本中，通常不需要调用 connect()
        # 直接尝试获取数据，或者调用内部的 get_client()
        print("尝试获取股票列表以触发连接...")
        stocks = xtdata.get_stock_list_in_sector('沪深A股')
        
        if stocks:
            print(f"✅ 行情模块通信成功!")
            print(f"✅ 获取到 {len(stocks)} 只股票")
            print(f"前5只: {stocks[:5]}")
        else:
            print("❌ 获取股票列表为空，行情服务可能未启动。")
            
    except Exception as e:
        print(f"❌ 运行异常: {e}")
        print("💡 提示: 某些版本的 xtdata 使用 reconnect(ip, port) 或直接调用数据接口。")

if __name__ == "__main__":
    test_xtdata()
