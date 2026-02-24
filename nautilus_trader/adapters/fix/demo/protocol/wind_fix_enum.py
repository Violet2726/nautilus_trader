from enum import Enum


class FixMsgType(Enum):
    """消息类型 (Tag 35) [cite: 7]"""

    HEARTBEAT = "0"  # 心跳
    LOGON = "A"  # 登录(传输层)
    LOGOUT = "5"  # 登出(传输层)
    NEW_ORDER_SINGLE = "D"  # 下单委托 [cite: 10]
    ORDER_CANCEL_REQUEST = "F"  # 撤单请求 [cite: 11]
    EXECUTION_REPORT = "8"  # 执行/成交回报 [cite: 12]
    BUSINESS_MESSAGE_REJECT = "j"  # 业务拒绝 [cite: 8]
    USER_REQUEST = "BE"  # 应用层登录请求 [cite: 9]
    USER_RESPONSE = "BF"  # 应用层登录响应 [cite: 9]
    # 自定义查询类 [cite: 25, 27, 28]
    QUERY_REQUEST = "U02"  # 委托/成交查询请求
    QUERY_RESPONSE = "U03"  # 委托/成交查询应答
    HOLDING_QUERY_REQUEST = "U04"  # 持仓查询请求
    HOLDING_QUERY_RESPONSE = "U05"  # 持仓查询应答
    FUNDING_QUERY_REQUEST = "U06"  # 资金查询请求
    FUNDING_QUERY_RESPONSE = "U07"  # 资金查询应答


class UserRequestType(Enum):
    """用户请求类型 (Tag 924) [cite: 9]"""

    LOGIN = 1  # 登录请求
    LOGOUT = 2  # 登出请求


class QueryType(Enum):
    """查询类型 (Tag 8000) [cite: 25]"""

    ORDER = 1  # 1: 委托查询
    EXECUTION = 2  # 2: 成交查询


class StorageTopic(Enum):
    """内存存储的主题分类"""

    ORDERS = "orders"
    DEALS = "deals"
    POSITIONS = "positions"
    ACCOUNTS = "accounts"
    EXEC_ORDER = "execOrder"


class OrdType(Enum):
    """订单类型 (Tag 40) [cite: 10, 12, 19]"""

    LIMIT = "2"  # 限价单
    MARKET = "1"  # 市价单


class Side(Enum):
    """交易方向 (Tag 54) [cite: 10, 12, 19]"""

    BUY = "1"  # 买入
    SELL = "2"  # 卖出
    SELL_SHORT = "5"  # 卖空
    REVERSE_REPO = "1"  # 逆回购 (同 Buy) [cite: 19]
    REPO = "2"  # 正回购 (同 Sell) [cite: 19]


class OrdStatus(Enum):
    """订单状态 (Tag 39) [cite: 12]"""

    NEW = "0"  # 新订单
    PARTIALLY_FILLED = "1"  # 部分成交
    FILLED = "2"  # 全部成交
    DONE_FOR_DAY = "3"  # 当天已完结
    CANCELED = "4"  # 已取消
    PENDING_CANCEL = "6"  # 待取消
    STOPPED = "7"  # 已停止
    REJECTED = "8"  # 已拒绝
    EXPIRED = "C"  # 已过期
    PENDING_REPLACE = "E"  # 待替换


class UserStatus(Enum):
    """用户状态 (Tag 926) [cite: 9]"""

    LOGGED_IN = "1"  # 已登陆
    NOT_LOGGED_IN = "2"  # 未登陆
    USER_NOT_EXIST = "3"  # 用户不存在
    PASSWORD_WRONG = "4"  # 密码不对
    NEED_CHANGE_PASSWORD = "5"  # 需要修改密码
    OTHER_ERROR = "6"  # 其它错误


class ExecType(Enum):
    """成交类型 (Tag 150)"""

    NEW = "0"  # 新订单(New)
    DONE_FOR_DAY = "3"  # 当天已完结(Done for day)
    CANCELED = "4"  # 已取消(Canceled)
    REPLACED = "5"  # 已替换
    PENDING_CANCEL = "6"  # 待取消(Pending Cancel)
    STOPPED = "7"  # 已停止(Stopped)
    REJECTED = "8"  # 拒绝(Rejected)
    SUSPENDED = "9"  # 已暂停
    PENDING_NEW = "A"  # 待接收
    EXPIRED = "C"  # 已过期(Expired)
    PENDING_REPLACE = "E"  # 待替换(Pending Replace)
    ORDER_STATUS = "I"  # 订单状态查询返回
    TRADE = "F"  # 交易推送 (成交)


class SecurityExchange(Enum):
    """交易所类型 (Tag 207) [cite: 10]"""

    HKEX = "HKEX"  # 香港交易所
    NYSE = "NYSEA"  # 纽交所
    NASDAQ = "NASDAQ"  # 纳斯达克
    AMEX = "AMEX"  # 美交所
    SH_NORTH = "SHN"  # 沪股通（北向） [cite: 30]
    SZ_NORTH = "SZN"  # 深股通（北向） [cite: 30]
    SSE = "SSE"  # 上交所
    SZSE = "SZSE"  # 深交所


class BookingType(Enum):
    """委托类型 (Tag 775) [cite: 10]"""

    CASH = 0  # 现金交易
    SWAP = 1  # 收益互换


class TimeInForce(Enum):
    """有效时间 (Tag 59) [cite: 10]"""

    GFD = "0"  # 当日有效
    GTC = "1"  # 取消前有效
    GTD = "5"  # 到期前有效


class HandlInst(Enum):
    """执行指令 (Tag 21) [cite: 10]"""

    LowTouch = "1"  # 自动处理
    HighTouch = "3"  # 人工处理


class SecurityType(Enum):
    """证券类型 (Tag 167) [cite: 10, 13, 19]"""

    COMMON_STOCK = "CS"  # 股票
    FUTURE = "FUT"  # 期货
    OPTION = "OPT"  # 期权
    CASH_BOUND = "CASHBOUND"  # 现券 [cite: 13]
    PLEDGED_REPO = "PLEDGEDREPO"  # 质押式回购 [cite: 19]


class WindCustomTags(Enum):
    """Wind EMS 私有协议自定义 Tag 字典"""

    # 查询相关
    LocateBrokerId = 5700  # 融券来源 (美股 Sell Short 时必填)
    QueryType = 8000  # 查询类型 (1:委托查询, 2:成交查询)
    QueryStatus = 8003  # 查询状态 (0:成功, 1:失败)

    HedgeFlag = 8009  # 套保标记 (0:投机, 1:保值 2:套利)

    # 数量与统计组 (重复组)
    NoOrders = 8004  # 委托查询返回的重复组个数
    NoExecutes = 8005  # 重复组个数8000=2时必传
    NumInGroup = 8006  # ？持仓查询返回的重复组个数
    NoFundings = 8007  # 资金查询返回的重复组个数

    # 资金与市值字段
    UseableAmt = 8008  # 可用资金
    MarketCap = 8001  # 持仓证券市值
    AvailableMargin = 8017  # 可用保证金
    MaintenanceMargin = 8018  # 维持保证金
    FrozenMargin = 8019  # 冻结保证金
    TotalMargin = 8013  # 保证金总额
    Equity = 8014  # 权益

    # 持仓明细字段
    PositionQty = 8015  # 持仓数量
    PositionCost = 8016  # 持仓成本

    StartDate = 916  # 开始时间（LocalMktDate类型）
    EndDate = 917  # 结束时间（LocalMktDate类型）

    # 用户信息
    UserName = 553  # 用户名
    Password = 554  # 密码


def get_exchange_info_v3(order_code):
    """
    根据用户提供的最新对照表进行映射
    """
    symbol, suffix = order_code.split(".")
    suffix = suffix.upper()

    # 映射表：完全匹配用户提供的类型
    mapping = {
        "SH": ("SSE", "CS"),  # 上交所
        "SZ": ("SZSE", "CS"),  # 深交所
        "HK": ("HKEX", "CS"),  # 香港交易所
        "N": ("NYSEA", "CS"),  # 纽交所 (假设后缀为N)
        "OQ": ("NASDAQ", "CS"),  # 纳斯达克 (假设后缀为OQ)
        "A": ("AMEX", "CS"),  # 美交所
        "SHN": ("SZN", "CS"),  # 特别注意：用户提到 SZN 对应北向-上交所
        "SZN": ("SHN", "CS"),  # 特别注意：用户提到 SHN 对应北向-深交所
    }

    # 获取映射结果，如果匹配不到则默认返回 SSE
    res = mapping.get(suffix, ("SSE", "CS"))
    return res[0], res[1]
