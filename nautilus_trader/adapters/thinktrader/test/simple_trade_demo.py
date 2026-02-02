# coding=utf-8
import time
from xtquant.xttrader import XtQuantTrader, XtQuantTraderCallback
from xtquant.xttype import StockAccount
from xtquant import xtconstant

# 1. 定义回调类 (用于接收成交、委托状态变化等推送)
class MyXtQuantTraderCallback(XtQuantTraderCallback):
    def on_disconnected(self):
        """连接断开推送"""
        print("连接断开 (Connection lost)")

    def on_stock_order(self, order):
        """委托回报推送"""
        print(f"委托状态更新: 订单编号:{order.order_id} 状态:{order.order_status} 备注:{order.order_remark}")

    def on_stock_trade(self, trade):
        """成交变动推送"""
        print(f"成交回报: 股票:{trade.stock_code} 价格:{trade.traded_price} 数量:{trade.traded_volume}")

    def on_order_error(self, order_error):
        """下单失败推送"""
        print(f"下单失败: 订单:{order_error.order_id} 错误码:{order_error.error_id} 原因:{order_error.error_msg}")

    def on_cancel_error(self, cancel_error):
        """撤单失败推送"""
        print(f"撤单失败: 订单:{cancel_error.order_id} 原因:{cancel_error.error_msg}")


def main():
    print("=== 启动交易 Demo ===")

    # 2. 设置路径和 Session ID
    # 注意：路径必须指向 MiniQMT 的 userdata_mini 文件夹
    mini_qmt_path = r'D:\中信证券QMT交易终端仿真\userdata_mini'
    session_id = int(time.time())  # 使用时间戳作为会话ID，避免冲突

    # 3. 创建交易对象
    print(f"正在连接 MiniQMT 路径: {mini_qmt_path}")
    xt_trader = XtQuantTrader(mini_qmt_path, session_id)

    # 4. 启动并连接
    xt_trader.start()
    connect_result = xt_trader.connect()

    if connect_result == 0:
        print(">> 连接成功")
    else:
        print(f">> 连接失败，错误码: {connect_result}")
        print("请检查 MiniQMT 客户端是否已登录并在运行中。")
        return

    # 5. 创建账户对象
    account_id = '10100002780'  
    acc = StockAccount(account_id)

    # 注册回调
    callback = MyXtQuantTraderCallback()
    xt_trader.register_callback(callback)

    # 订阅账户资金推送
    subscribe_res = xt_trader.subscribe(acc)
    print(f">> 账户订阅结果: {subscribe_res}")

    # ============================================================
    # 6. 执行简单的买入操作
    # ============================================================
    stock_code = '600000.SH'  # 浦发银行
    buy_price = 10.50         # 指定买入价
    volume = 100              # 买入数量 (至少100股)
    
    print(f"\n[买入演示] 正在发送买单: 代码={stock_code}, 价格={buy_price}, 数量={volume}")
    
    # order_stock 参数说明:
    # 账户对象, 证券代码, 委托类型(买入), 数量, 报价类型(限价), 价格, 策略名, 备注
    buy_order_id = xt_trader.order_stock(
        acc, 
        stock_code, 
        xtconstant.STOCK_BUY, 
        volume, 
        xtconstant.FIX_PRICE, 
        buy_price, 
        'strategy_demo', 
        'buy_test'
    )
    print(f">> 买单发送完成，本地订单ID: {buy_order_id}")

    # 等待几秒，观察回调输出
    time.sleep(3)

    # ============================================================
    # 7. 执行简单的卖出操作
    # ============================================================
    sell_price = 10.60
    
    print(f"\n[卖出演示] 正在发送卖单: 代码={stock_code}, 价格={sell_price}, 数量={volume}")
    
    sell_order_id = xt_trader.order_stock(
        acc, 
        stock_code, 
        xtconstant.STOCK_SELL, 
        volume, 
        xtconstant.FIX_PRICE, 
        sell_price, 
        'strategy_demo', 
        'sell_test'
    )
    print(f">> 卖单发送完成，本地订单ID: {sell_order_id}")

    # ============================================================
    # 8. (可选) 撤单演示
    # ============================================================
    # time.sleep(2)
    # print(f"\n[撤单演示] 撤销刚才的买单: {buy_order_id}")
    # xt_trader.cancel_order_stock(acc, buy_order_id)

    # 保持程序运行以接收回调
    print("\n程序正在运行，按 Ctrl+C 退出...")
    try:
        xt_trader.run_forever()
    except KeyboardInterrupt:
        print("停止运行")
        xt_trader.stop()

if __name__ == "__main__":
    main()