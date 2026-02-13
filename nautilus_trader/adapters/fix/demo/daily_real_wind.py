import os
import sys
import json
import time

# 自动切换到脚本所在目录，确保配置文件的相对路径有效
base_path = os.path.dirname(os.path.abspath(__file__))
os.chdir(base_path)
if base_path not in sys.path:
    sys.path.append(base_path)

import quickfix as fix

from protocol.quick_fix import QMTToFIXAdapter, ensure_tls_tunnel


class WindFixConn:
    """连接wind fix"""

    def __init__(self):
        self.wind_cfg_path = "wind_fix_config.cfg"  # wind fix协议配置文件位置
        self.wind_fix44_path = "FIX44.xml"  # wind fix协议字典位置
        self.login_account = "HA2032139003"  # wind 通道账号
        self.password = "60374602"  # wind 通道密码
        self.waite_time = 3  # 登录等待时间

    def test_network_connection(self, host, port):
        """测试网络连接"""
        import socket
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sock.settimeout(5)
            result = sock.connect_ex((host, port))
            sock.close()
            return result == 0
        except Exception as e:
            print(f"网络测试失败: {e}")
            return False

    def get_fix_application(self):
        # 测试网络连接
        settings = fix.SessionSettings(self.wind_cfg_path)
        # 从配置中获取连接信息
        host = "114.80.213.49"
        port = 16669
        
        print(f"正在测试网络连接: {host}:{port}")
        if not self.test_network_connection(host, port):
            print(f"错误: 无法连接到 {host}:{port}，请检查网络配置")
            return None, None
        
        print("网络连接测试成功，正在初始化 FIX 连接...")
        
        try:
            ensure_tls_tunnel(host, port)
            application = QMTToFIXAdapter(self.wind_fix44_path, self.login_account, self.password)  # fix 对接实现类
            store_factory = fix.FileStoreFactory(settings)
            log_factory = fix.FileLogFactory(settings)

            # 启动客户端
            initiator = fix.SocketInitiator(application, store_factory, settings, log_factory)
            initiator.start()

            # 优化：循环检查直到登录成功或超时
            start_wait = time.time()
            while time.time() - start_wait < 30: # 最长等待 30 秒
                if application.session_id is not None:
                    # 额外等待一小会确保应用层登录也完成
                    time.sleep(2)
                    print("FIX 传输层连接成功")
                    return initiator, application
                time.sleep(0.5)
            
            print("警告: FIX 连接在 30 秒内未建立，请检查网络配置、SSL 设置或认证信息")
            print("建议检查：1. 账号密码是否正确 2. TargetCompID 配置是否正确 3. 服务器是否运行")
            return initiator, application
        except Exception as e:
            print(f"初始化 FIX 连接时出错: {e}")
            return None, None


# 保持住链接
_, app = WindFixConn().get_fix_application()


def place_order(C, op_type, order_code, pr_type, price, volume, note) -> str:
    # 下单接口（异步变同步，等待最长30s超时）,C是上下文对象，里面包含账号等信息，看情况使用
    # 参数：
    #   op_type: 23-买入, 24-卖出
    #   order_code: 股票代码（如：000002.SZ）
    #   pr_type: 11-限价, 14-市价
    #   price: 价格, 当pr_type=14时传None
    #   volume: 股数
    #   note: 备注（当模拟交易时，传 FF 随时撮合）
    # 响应：
    #   order_id: 柜台合同号，撤单时传入的order_id

    return app.passorder(opt_ype=op_type, account_id=C.acct, order_code=order_code, pr_type=pr_type, price=price,
                         volume=volume, user_order_id=note)


def cancel_order(C, order_id, op_type):
    # 撤单，order_id是柜台合同号，C是上下文对象，里面包含账号等信息，看情况使用
    # 参数：
    #   order_id: 是柜台合同号
    #   op_type: 23-买入, 24-卖出

    side = 1 if op_type == 23 else 2  # 1:Buy  2:Sell
    app.cancel(order_id=order_id, account_id=C.acct, side=side)


class AccountInfo:
    def __init__(self):
        self.m_dBalance = 0.0  # 总资产
        self.m_dAvailable = 0.0  # 可用资金
        self.m_dInstrumentValue = 0.0  # 持仓市值 (没有就设置为0，但必须保证m_dBalance值正确)


def get_account(C):
    # 获取账号信息，C是上下文对象，里面包含账号等信息，看情况使用
    # 返回：
    #   AccountInfo
    raw_data = app.get_trade_detail_data(C.acct, "ACCOUNT")
    if isinstance(raw_data, dict): # 处理查询超时返回的空字典
        return AccountInfo()
    
    account_json = json.loads(raw_data)

    account_info = AccountInfo()
    if account_json is not None and account_json.get("NoFundings")[0] is not None:
        for no_funding in account_json.get("NoFundings"):
            if no_funding.get('Currency') == 'CNY':
                account_info.m_dAvailable = 0.0 if no_funding.get('UseableAmt') is None else no_funding.get(
                    'UseableAmt')  # 可用资金
                account_info.m_dInstrumentValue = 0.0 if no_funding.get('MarketCap') is None else no_funding.get(
                    'MarketCap')  # 持仓市值
                account_info.m_dBalance = no_funding.get('UseableAmt') + no_funding.get(
                    'MarketCap')  # 总资产

    return account_info


class PositionInfo:
    def __init__(self):
        self.m_sInstrumentID = ""  # 证券代码, 例如"000001"
        self.m_sExchangeID = ""  # 交易所代码, 例如"SZ" 没有的话要说明
        self.m_nVolume = 0  # 持仓数量
        self.m_nCanUseVolume = 0  # 可用数量
        self.m_dMarketValue = 0.0  # 持仓市值 （对方接口没有提供）


def get_position(C):
    # 获取持仓信息，C是上下文对象，里面包含账号等信息，看情况使用
    # 返回：
    #   List[PositionInfo]

    raw_data = app.get_trade_detail_data(C.acct, "POSITION")
    if isinstance(raw_data, dict):
        return []
    
    position_json = json.loads(raw_data)

    positions = []
    if position_json is not None and position_json.get("NoHoldings")[0] is not None:
        no_holdings = position_json.get("NoHoldings")
        for no_holding in no_holdings:
            position_info = PositionInfo()
            symbol_group = no_holding.get('Symbol').split('.')
            position_info.m_sInstrumentID = symbol_group[0]  # 证券代码
            if len(symbol_group) == 2:
                position_info.m_sExchangeID = symbol_group[1]  # 交易所代码
            position_info.m_nVolume = no_holding.get('PositionQty')  # 持仓数量
            position_info.m_nCanUseVolume = no_holding.get('LeavesQty')  # 可用数量
            position_info.m_dMarketValue = 0.0  # 持仓市值 #
            positions.append(position_info)
    return positions


class OrderInfo:
    def __init__(self):
        self.m_sInstrumentID = ""  # 证券代码, 例如"000001"
        self.m_sExchangeID = ""  # 交易所代码, 例如"SZ" 没有的话要说明
        self.m_strInstrumentName = ""  # 证券名称, 例如"平安银行" 没有就为空
        self.m_strOrderSysID = ""  # 委托ID
        self.m_nOffsetFlag = 48  # 买卖标记, 48买入, 49卖出
        self.m_nOrderStatus = 49  # 委托状态, 49待报, 50已报, 55部成，只有这三个状态的会撤单 （-1状态为未匹配）
        self.m_strOptName = ""  # 买卖操作，没有就为空
        self.m_nVolumeTotal = 0  # 委托剩余量，没有就为0
        self.m_nVolumeTotalOriginal = 0  # 委托原始总量，没有就为0


def get_order(C):
    # 获取委托信息，C是上下文对象，里面包含账号等信息，看情况使用
    # 返回：
    #   List[OrderInfo]

    def order_status_convert(order_status):
        order_status_int = int(order_status)
        if order_status_int == 0:  # 0:新订单(New)
            return 49
        elif order_status_int == 1:  # 1:部分成交(Partially filled)
            return 55
        else:
            return -1

    trade_detail = app.get_trade_detail_data(C.acct, "ORDER")
    if trade_detail is None or len(trade_detail) == 0 or isinstance(trade_detail, dict):
        return []

    orders_json = json.loads(trade_detail)

    orders = []
    if orders_json is not None and orders_json.get("NoQueryOrders") is not None:
        no_query_orders = orders_json.get("NoQueryOrders")
        for no_query_order in no_query_orders:
            order_info = OrderInfo()
            symbol_group = no_query_order.get('Symbol').split('.')
            order_info.m_sInstrumentID = symbol_group[0]  # 证券代码
            if len(symbol_group) == 2:
                order_info.m_sExchangeID = symbol_group[1]  # 交易所代码
            order_info.m_strInstrumentName = ''  # 证券名称
            order_info.m_strOrderSysID = no_query_order.get('OrderID')  # 委托ID
            order_info.m_nOffsetFlag = 48 if int(no_query_order.get('Side')) == 1 else 49  # 买卖标记 1:Buy  2:Sell
            order_info.m_nOrderStatus = order_status_convert(no_query_order.get('OrdStatus'))  # 委托状态
            order_info.m_strOptName = ''  # 买卖操作 #
            order_info.m_nVolumeTotal = no_query_order.get('LeavesQty')  # 委托剩余量 #
            order_info.m_nVolumeTotalOriginal = no_query_order.get('OrderQty')  # 委托原始总量 #
            orders.append(order_info)
    return orders


class DealInfo:
    def __init__(self):
        self.m_sInstrumentID = ""  # 证券代码, 例如"000001"
        self.m_sExchangeID = ""  # 交易所代码, 例如"SZ" 没有的话要说明
        self.m_strInstrumentName = ""  # 证券名称, 例如"平安银行" 没有就为空
        self.m_nOffsetFlag = 48  # 买卖标记, 48买入, 49卖出
        self.m_dPrice = 0  # 成交价格
        self.m_nVolume = 0  # 成交数量
        self.m_strTradeTime = ""  # 成交时间


def get_deal(C):
    # 获取成交信息，C是上下文对象，里面包含账号等信息，看情况使用

    trade_detail = app.get_trade_detail_data(C.acct, "DEAL")
    if trade_detail is None or len(trade_detail) == 0 or isinstance(trade_detail, dict):
        return []

    deal_json = json.loads(trade_detail)
    deals = []
    if deal_json is not None and deal_json.get("NoQueryOrders") is not None:
        no_query_orders = deal_json.get("NoQueryOrders")
        for no_query_order in no_query_orders:
            no_executes = no_query_order.get("NoExecutes")
            if no_executes is not None and no_executes[0] is not None:
                for no_execute in no_executes:
                    deal_info = DealInfo()
                    symbol_group = no_query_order.get('Symbol').split('.')
                    deal_info.m_sInstrumentID = symbol_group[0]  # 证券代码
                    if len(symbol_group) == 2:
                        deal_info.m_sExchangeID = symbol_group[1]  # 交易所代码
                    deal_info.m_strInstrumentName = ''  # 证券名称
                    deal_info.m_nOffsetFlag = 48 if int(no_query_order.get('Side')) == 1 else 49  # 买卖标记 1:Buy  2:Sell
                    deal_info.m_dPrice = no_execute.get('LastPx')  # 成交价格
                    deal_info.m_nVolume = no_execute.get('LastQty')  # 成交数量
                    deal_info.m_strTradeTime = no_execute.get('OrderQty')  # 成交时间
                    deals.append(deal_info)

    return deals
