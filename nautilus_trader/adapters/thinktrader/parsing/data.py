from datetime import datetime
from nautilus_trader.model.data import QuoteTick, TradeTick, Bar, BarType
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.objects import Price, Quantity
from nautilus_trader.model.enums import BarAggregation
from nautilus_trader.model.data import BarSpec

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


def xt_time_to_ns(time_val: int) -> int:
    """
    将 XtQuant 时间 (毫秒戳或 YYYYMMDDHHMMSS) 转换为纳秒时间戳
    """
    if time_val > 10_000_000_000_000: # 大于 13 位，假设是 YYYYMMDDHHMMSS 格式
        s = str(time_val)
        try:
            dt = datetime.strptime(s, "%Y%m%d%H%M%S")
            return int(dt.timestamp() * 1_000_000_000)
        except ValueError:
             return time_val * 1_000_000 
    
    return time_val * 1_000_000


def ns_to_xt_time(ts_ns: int) -> str:
    """
    将纳秒时间戳转换为 XtQuant 格式 (YYYYMMDDHHMMSS)
    """
    dt = datetime.fromtimestamp(ts_ns / 1_000_000_000)
    return dt.strftime("%Y%m%d%H%M%S")


def bar_spec_to_period(bar_spec: BarSpec) -> str:
    """
    将 Nautilus BarSpec 转换为 XtQuant period 字符串
    """
    agg = bar_spec.aggregation
    step = bar_spec.step
    
    if agg == BarAggregation.TICK:
        return "tick"
    elif agg == BarAggregation.MINUTE:
        if step in [1, 5, 15, 30]:
            return f"{step}m"
        elif step == 60:
            return "1h"
    elif agg == BarAggregation.HOUR:
        return f"{step}h"
    elif agg == BarAggregation.DAY:
        return "1d"
    elif agg == BarAggregation.WEEK:
        return "1w"
    elif agg == BarAggregation.MONTH:
        return "1mon"
        
    raise ValueError(f"不支持的 BarSpec: {bar_spec}")


def parse_tick_to_quote_tick(
    instrument_id: InstrumentId,
    data: dict,
    ts_init: int,
) -> QuoteTick:
    """
    将 XtQuant tick 数据转换为 QuoteTick
    """
    bid_prices = data.get("bidPrice", [0.0])
    ask_prices = data.get("askPrice", [0.0])
    bid_vols = data.get("bidVol", [0])
    ask_vols = data.get("askVol", [0])
    
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
