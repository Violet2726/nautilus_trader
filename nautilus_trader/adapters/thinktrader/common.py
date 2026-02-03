from nautilus_trader.model.identifiers import Venue


TT = "THINKTRADER"
TT_VENUE = Venue(TT)

# ============================================================================
# XtQuant 市场代码映射
# ============================================================================
MARKET_CODE_MAP = {
    "SH": "xtconstant.SH_MARKET",  # 上交所
    "SZ": "xtconstant.SZ_MARKET",  # 深交所
    "BJ": "xtconstant.MARKET_ENUM_BEIJING",  # 北交所
    "HGT": "xtconstant.MARKET_ENUM_SHANGHAI_HONGKONG_STOCK",  # 沪港通
    "SGT": "xtconstant.MARKET_ENUM_SHENZHEN_HONGKONG_STOCK",  # 深港通
    "SF": "xtconstant.MARKET_ENUM_SHANGHAI_FUTURE",  # 上期所
    "DF": "xtconstant.MARKET_ENUM_DALIANG_FUTURE",  # 大商所
    "ZF": "xtconstant.MARKET_ENUM_ZHENGZHOU_FUTURE",  # 郑商所
    "IF": "xtconstant.MARKET_ENUM_INDEX_FUTURE",  # 中金所
    "INE": "xtconstant.MARKET_ENUM_INTL_ENERGY_FUTURE",  # 能源中心
    "GF": "xtconstant.MARKET_ENUM_GUANGZHOU_FUTURE",  # 广期所
    "SHO": "xtconstant.MARKET_ENUM_SHANGHAI_STOCK_OPTION",  # 上海期权
    "SZO": "xtconstant.MARKET_ENUM_SHENZHEN_STOCK_OPTION",  # 深圳期权
}

# Nautilus Venue -> XtQuant 市场代码
VENUE_TO_MARKET = {
    "SSE": "SH",  # 上交所
    "SZSE": "SZ",  # 深交所
    "BSE": "BJ",  # 北交所
    "SHFE": "SF",  # 上期所
    "DCE": "DF",  # 大商所
    "CZCE": "ZF",  # 郑商所
    "CFFEX": "IF",  # 中金所
    "INE": "INE",  # 能源中心
    "GFEX": "GF",  # 广期所
}

# XtQuant 市场代码 -> Nautilus Venue
MARKET_TO_VENUE = {v: k for k, v in VENUE_TO_MARKET.items()}

# ============================================================================
# 账号类型映射
# ============================================================================
ACCOUNT_TYPE_MAP = {
    "FUTURE": "xtconstant.FUTURE_ACCOUNT",  # 期货
    "STOCK": "xtconstant.SECURITY_ACCOUNT",  # 股票
    "CREDIT": "xtconstant.CREDIT_ACCOUNT",  # 信用
    "FUTURE_OPTION": "xtconstant.FUTURE_OPTION_ACCOUNT",  # 期货期权
    "STOCK_OPTION": "xtconstant.STOCK_OPTION_ACCOUNT",  # 股票期权
    "HGT": "xtconstant.HUGANGTONG_ACCOUNT",  # 沪港通
    "SGT": "xtconstant.SHENGANGTONG_ACCOUNT",  # 深港通
}
