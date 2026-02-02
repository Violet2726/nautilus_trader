from nautilus_trader.model.data import QuoteTick, TradeTick, Bar, BarType
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.objects import Price, Quantity
from nautilus_trader.model.enums import BarAggregation

# ============================================================================
# 周期类型映射
# ============================================================================
PERIOD_MAP = {
    # Level1 数据
    BarAggregation.TICK: "tick",
    BarAggregation.MINUTE: "1m",
    BarAggregation.HOUR: "1h",
    BarAggregation.DAY: "1d",
    BarAggregation.WEEK: "1w",
    BarAggregation.MONTH: "1mon",
}

# Level2 周期类型
LEVEL2_PERIOD_MAP = {
    "l2quote": "l2quote",        # Level2实时行情快照
    "l2order": "l2order",        # Level2逐笔委托
    "l2transaction": "l2transaction",  # Level2逐笔成交
    "l2quoteaux": "l2quoteaux",  # Level2实时行情补充
    "l2orderqueue": "l2orderqueue",    # Level2委托队列
}

# step -> period (分钟级别)
STEP_TO_PERIOD = {
    1: "1m",
    5: "5m",
    15: "15m",
    30: "30m",
    60: "1h",
}

# ============================================================================
# 证券状态映射
# ============================================================================
STOCK_STATUS_MAP = {
    0: "UNKNOWN",       # 默认未知
    10: "UNKNOWN",      # 默认未知
    11: "PRE_OPEN",     # 开盘前 S
    12: "AUCTION",      # 集合竞价时段 C
    13: "CONTINUOUS",   # 连续交易 T
    14: "BREAK",        # 休市 B
    15: "CLOSED",       # 闭市 E
    16: "HALT",         # 波动性中断 V
    17: "SUSPENDED",    # 临时停牌 P
    18: "CLOSE_AUCTION", # 收盘集合竞价 U
    19: "MID_AUCTION",  # 盘中集合竞价 M
    20: "HALT_TO_CLOSE", # 暂停交易至闭市 N
    21: "ERROR",        # 获取字段异常
    22: "POST_TRADING", # 盘后固定价格行情
    23: "POST_CLOSED",  # 盘后固定价格行情完毕
}


def xt_time_to_ns(time_val: int) -> int:
    """
    将 XtQuant 时间 (毫秒戳或 YYYYMMDDHHMMSS) 转换为纳秒时间戳
    """
    # 简单的启发式判断
    # 2000 年的毫秒戳约为 946684800000 (12位)
    # 2100 年的毫秒戳约为 4102444800000 (13位)
    # YYYYMMDDHHMMSS 格式 (如 20230101000000) 是 14 位
    
    if time_val > 10_000_000_000_000: # 大于 13 位，假设是 YYYYMMDDHHMMSS 格式
        from datetime import datetime
        s = str(time_val)
        try:
            dt = datetime.strptime(s, "%Y%m%d%H%M%S")
            return int(dt.timestamp() * 1_000_000_000)
        except ValueError:
             # Fallback: maybe it includes milliseconds?
             return time_val * 1_000_000 # Assume ms as fallback
    
    # 默认假设为毫秒时间戳
    return time_val * 1_000_000


def parse_tick_to_quote_tick(
    instrument_id: InstrumentId,
    data: dict,
    ts_init: int,
) -> QuoteTick:
    """
    将 XtQuant tick 数据转换为 QuoteTick
    
    XtQuant tick 字段 (参见 行情模块文档.md - tick分笔数据):
    - time: 时间戳 (毫秒)
    - lastPrice: 最新价
    - bidPrice: 委买价 (数组，多档)
    - askPrice: 委卖价 (数组，多档)
    - bidVol: 委买量 (数组，多档)
    - askVol: 委卖量 (数组，多档)
    """
    # 取第一档买卖盘
    bid_prices = data.get("bidPrice", [0.0])
    ask_prices = data.get("askPrice", [0.0])
    bid_vols = data.get("bidVol", [0])
    ask_vols = data.get("askVol", [0])
    
    # 确保数组非空
    bid_price = bid_prices[0] if bid_prices else 0.0
    ask_price = ask_prices[0] if ask_prices else 0.0
    bid_vol = int(bid_vols[0]) if bid_vols else 0
    ask_vol = int(ask_vols[0]) if ask_vols else 0
    
    ts_event = xt_time_to_ns(int(data.get("time", 0)))
    
    return QuoteTick(
        instrument_id=instrument_id,
        bid_price=Price.from_str(f"{bid_price:.4f}"),
        ask_price=Price.from_str(f"{ask_price:.4f}"),
        bid_size=Quantity.from_int(bid_vol),
        ask_size=Quantity.from_int(ask_vol),
        ts_event=ts_event,
        ts_init=ts_init,
    )

def parse_tick_to_trade_tick(
    instrument_id: InstrumentId,
    data: dict,
    ts_init: int,
) -> TradeTick:
    """
    将 XtQuant tick 数据转换为 TradeTick
    
    XtQuant tick 字段:
    - lastPrice: 最新价
    - volume: 成交总量
    - transactionNum: 成交笔数
    """
    from nautilus_trader.model.identifiers import TradeId
    from nautilus_trader.model.enums import AggressorSide
    
    ts_event = xt_time_to_ns(int(data.get("time", 0)))
    
    return TradeTick(
        instrument_id=instrument_id,
        price=Price.from_str(f"{data.get('lastPrice', 0.0):.4f}"),
        size=Quantity.from_int(int(data.get("volume", 0))),
        aggressor_side=AggressorSide.NO_AGGRESSOR,
        trade_id=TradeId(str(data.get("time", 0))),
        ts_event=ts_event,
        ts_init=ts_init,
    )

def parse_kline_to_bar(
    instrument_id: InstrumentId,
    bar_type: BarType,
    data: dict,
    ts_init: int,
) -> Bar:
    """
    将 XtQuant K线数据转换为 Bar
    
    XtQuant K线字段 (参见 行情模块文档.md - K线数据):
    - time: 时间戳
    - open/high/low/close: OHLC 价格
    - volume: 成交量
    - amount: 成交额
    - preClose: 前收价
    """
    ts_event = xt_time_to_ns(int(data.get("time", 0)))
    
    return Bar(
        bar_type=bar_type,
        open=Price.from_str(f"{data.get('open', 0.0):.4f}"),
        high=Price.from_str(f"{data.get('high', 0.0):.4f}"),
        low=Price.from_str(f"{data.get('low', 0.0):.4f}"),
        close=Price.from_str(f"{data.get('close', 0.0):.4f}"),
        volume=Quantity.from_int(int(data.get("volume", 0))),
        ts_event=ts_event,
        ts_init=ts_init,
    )
