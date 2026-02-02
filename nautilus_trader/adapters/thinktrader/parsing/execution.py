from xtquant import xtconstant
from nautilus_trader.model.enums import OrderStatus, OrderSide, OrderType, TimeInForce

# ============================================================================
# 订单状态映射 (来源: API_NOTES 4.5)
# ============================================================================
ORDER_STATUS_MAP = {
    xtconstant.ORDER_UNREPORTED: OrderStatus.SUBMITTED,      # 48: 未报
    xtconstant.ORDER_WAIT_REPORTING: OrderStatus.SUBMITTED,  # 49: 待报
    xtconstant.ORDER_REPORTED: OrderStatus.ACCEPTED,         # 50: 已报
    xtconstant.ORDER_REPORTED_CANCEL: OrderStatus.PENDING_CANCEL,  # 51: 已报待撤
    xtconstant.ORDER_PARTSUCC_CANCEL: OrderStatus.PARTIALLY_FILLED,  # 52: 部成待撤
    xtconstant.ORDER_PART_CANCEL: OrderStatus.CANCELED,      # 53: 部撤
    xtconstant.ORDER_CANCELED: OrderStatus.CANCELED,         # 54: 已撤
    xtconstant.ORDER_PART_SUCC: OrderStatus.PARTIALLY_FILLED,  # 55: 部成
    xtconstant.ORDER_SUCCEEDED: OrderStatus.FILLED,          # 56: 已成
    xtconstant.ORDER_JUNK: OrderStatus.REJECTED,             # 57: 废单
    xtconstant.ORDER_UNKNOWN: OrderStatus.DENIED,            # 255: 未知
}

# ============================================================================
# 股票委托类型映射 (来源: API_NOTES 4.3)
# ============================================================================
STOCK_ORDER_TYPE_MAP = {
    "BUY": xtconstant.STOCK_BUY,                   # 股票买入
    "SELL": xtconstant.STOCK_SELL,                 # 股票卖出
}

# ============================================================================
# 信用委托类型映射 (来源: API_NOTES 4.3)
# ============================================================================
CREDIT_ORDER_TYPE_MAP = {
    "MARGIN_BUY": xtconstant.CREDIT_FIN_BUY,            # 融资买入
    "MARGIN_SELL": xtconstant.CREDIT_SELL_SECU_REPAY,       # 卖券还款
    "SHORT_SELL": xtconstant.CREDIT_SLO_SELL,           # 融券卖出
    "BUY_TO_COVER": xtconstant.CREDIT_BUY_SECU_REPAY,   # 买券还券
    "BUY_COLLATERAL": xtconstant.CREDIT_BUY,            # 担保品买入
    "SELL_COLLATERAL": xtconstant.CREDIT_SELL,          # 担保品卖出
}

# ============================================================================
# 期货委托类型映射 - 六键风格 (来源: API_NOTES 4.3)
# ============================================================================
FUTURES_SIX_KEY_ORDER_TYPE_MAP = {
    "OPEN_LONG": xtconstant.FUTURE_OPEN_LONG,              # 买开
    "CLOSE_LONG_HISTORY": xtconstant.FUTURE_CLOSE_LONG_HISTORY,  # 买平昨
    "CLOSE_LONG_TODAY": xtconstant.FUTURE_CLOSE_LONG_TODAY,      # 买平今
    "OPEN_SHORT": xtconstant.FUTURE_OPEN_SHORT,            # 卖开
    "CLOSE_SHORT_HISTORY": xtconstant.FUTURE_CLOSE_SHORT_HISTORY,  # 卖平昨
    "CLOSE_SHORT_TODAY": xtconstant.FUTURE_CLOSE_SHORT_TODAY,      # 卖平今
}

# ============================================================================
# 期货委托类型映射 - 四键风格 (来源: API_NOTES 4.3)
# ============================================================================
FUTURES_FOUR_KEY_ORDER_TYPE_MAP = {
    "OPEN_LONG": xtconstant.FUTURE_OPEN_LONG,       # 买开
    "CLOSE_LONG": xtconstant.FUTURE_CLOSE_LONG_HISTORY_FIRST,     # 卖平 (自动优先平昨)
    "OPEN_SHORT": xtconstant.FUTURE_OPEN_SHORT,     # 卖开
    "CLOSE_SHORT": xtconstant.FUTURE_CLOSE_SHORT_HISTORY_FIRST,   # 买平 (自动优先平昨)
}

# ============================================================================
# 期货委托类型映射 - 两键风格 (来源: API_NOTES 4.3)
# ============================================================================
# FUTURES_TWO_KEY_ORDER_TYPE_MAP = {
#     "SMART_BUY": xtconstant.FUTURE_SMART_BUY,            # 智能买入 (自动判断开/平)
#     "SMART_SELL": xtconstant.FUTURE_SMART_SELL,          # 智能卖出 (自动判断开/平)
#     "SMART_BUY_TODAY": xtconstant.FUTURE_SMART_BUY_TODAY,   # 智能买入平今优先
#     "SMART_SELL_TODAY": xtconstant.FUTURE_SMART_SELL_TODAY, # 智能卖出平今优先
# }

# ============================================================================
# 多空方向 (来源: API_NOTES 4.7)
# ============================================================================
DIRECTION_MAP = {
    "LONG": xtconstant.DIRECTION_FLAG_LONG,         # 48: 多
    "SHORT": xtconstant.DIRECTION_FLAG_SHORT,       # 49: 空
}

# ============================================================================
# 交易操作 / 开平标志 (来源: API_NOTES 4.8)
# ============================================================================
OFFSET_FLAG_MAP = {
    "OPEN": xtconstant.OFFSET_FLAG_OPEN,                  # 48: 开仓
    "CLOSE": xtconstant.OFFSET_FLAG_CLOSE,                # 49: 平仓
    "FORCE_CLOSE": xtconstant.OFFSET_FLAG_FORCECLOSE,     # 50: 强平
    "CLOSE_TODAY": xtconstant.OFFSET_FLAG_CLOSETODAY,     # 51: 平今
    "CLOSE_YESTERDAY": xtconstant.OFFSET_FLAG_ClOSEYESTERDAY,  # 52: 平昨
    "FORCE_OFF": xtconstant.OFFSET_FLAG_FORCEOFF,         # 53: 强减
    "LOCAL_FORCE_CLOSE": xtconstant.OFFSET_FLAG_LOCALFORCECLOSE,  # 54: 本地强平
}

# ============================================================================
# 报价类型映射 (来源: API_NOTES 4.4)
# ============================================================================
PRICE_TYPE_MAP = {
    # 通用
    xtconstant.FIX_PRICE: OrderType.LIMIT,          # 指定价
    xtconstant.LATEST_PRICE: OrderType.MARKET,      # 最新价
    xtconstant.MARKET_PEER_PRICE_FIRST: OrderType.MARKET,  # 对手方最优
}

# Nautilus OrderSide -> XtQuant 委托类型 (股票)
NAUTILUS_SIDE_TO_XT = {
    OrderSide.BUY: xtconstant.STOCK_BUY,
    OrderSide.SELL: xtconstant.STOCK_SELL,
}
