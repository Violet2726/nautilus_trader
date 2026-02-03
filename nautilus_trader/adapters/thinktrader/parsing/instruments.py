from datetime import datetime
from datetime import timedelta
from datetime import timezone
from typing import Any

from nautilus_trader.adapters.thinktrader.common import TT_VENUE
from nautilus_trader.model.enums import AssetClass
from nautilus_trader.model.enums import OptionKind
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import Symbol
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.instruments import Equity
from nautilus_trader.model.instruments import FuturesContract
from nautilus_trader.model.instruments import OptionContract
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity


# 市场代码映射: XtQuant市场后缀 -> Nautilus Venue 后缀
MARKET_TO_VENUE = {
    "SH": "SSE",     # 上交所
    "SZ": "SZSE",    # 深交所
    "BJ": "BSE",     # 北交所
    "SF": "SHFE",    # 上期所
    "DF": "DCE",     # 大商所
    "ZF": "CZCE",    # 郑商所
    "IF": "CFFEX",   # 中金所
    "INE": "INE",    # 能源中心
    "GF": "GFEX",    # 广期所
}

VENUE_TO_MARKET = {v: k for k, v in MARKET_TO_VENUE.items()}

CHINA_TZ = timezone(timedelta(hours=8))


def parse_equity(detail: dict, instrument_id: InstrumentId) -> Equity:
    """解析股票合约"""
    price_tick = detail.get("PriceTick", 0.01)
    price_precision = _get_precision(price_tick)

    return Equity(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        lot_size=Quantity.from_int(100),  # A股最小交易单位
        ts_event=0,
        ts_init=0,
    )


def parse_future(detail: dict, instrument_id: InstrumentId) -> FuturesContract:
    """解析期货合约"""
    price_tick = detail.get("PriceTick", 0.01)
    price_precision = _get_precision(price_tick)
    multiplier = detail.get("VolumeMultiple", 1)

    # 解析到期日: ExpireDate 格式通常是 YYYYMMDD
    expire_date_str = str(detail.get("ExpireDate", ""))
    activation_ns = 0
    expiration_ns = 0
    if expire_date_str and len(expire_date_str) == 8:
        try:
            expire_dt = datetime.strptime(expire_date_str, "%Y%m%d").replace(tzinfo=CHINA_TZ)
            expiration_ns = int(expire_dt.timestamp() * 1_000_000_000)
        except ValueError:
            pass

    return FuturesContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        asset_class=AssetClass.COMMODITY,  # 可根据品种调整
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        multiplier=Quantity.from_int(int(multiplier)),
        lot_size=Quantity.from_int(1),
        activation_ns=activation_ns,
        expiration_ns=expiration_ns,
        ts_event=0,
        ts_init=0,
    )


def parse_option(detail: dict, instrument_id: InstrumentId) -> OptionContract:
    """解析期权合约"""
    price_tick = detail.get("PriceTick", 0.0001)
    price_precision = _get_precision(price_tick)
    multiplier = detail.get("OptUnit", detail.get("VolumeMultiple", 10000))

    # 期权类型: OptionType -1=非期权, 0=认购, 1=认沽
    option_type = detail.get("OptionType", -1)
    option_kind = OptionKind.CALL if option_type == 0 else OptionKind.PUT

    # 行权价
    strike_price = detail.get("OptExercisePrice", 0.0)

    # 标的代码
    underlying_code = detail.get("OptUndlCode", "")

    # 到期日
    expire_date_str = str(detail.get("ExpireDate", detail.get("EndDelivDate", "")))
    activation_ns = 0
    expiration_ns = 0
    if expire_date_str and len(expire_date_str) >= 8:
        try:
            expire_dt = datetime.strptime(expire_date_str[:8], "%Y%m%d").replace(tzinfo=CHINA_TZ)
            expiration_ns = int(expire_dt.timestamp() * 1_000_000_000)
        except ValueError:
            pass

    return OptionContract(
        instrument_id=instrument_id,
        raw_symbol=Symbol(detail.get("InstrumentID", "")),
        asset_class=AssetClass.EQUITY,  # 股票期权
        currency=Currency.from_str("CNY"),
        price_precision=price_precision,
        price_increment=Price.from_str(f"{price_tick}"),
        multiplier=Quantity.from_int(int(multiplier)),
        lot_size=Quantity.from_int(1),
        underlying=underlying_code,
        option_kind=option_kind,
        strike_price=Price.from_str(f"{strike_price}"),
        activation_ns=activation_ns,
        expiration_ns=expiration_ns,
        ts_event=0,
        ts_init=0,
    )


def _get_precision(price_tick: float) -> int:
    """根据最小价格变动单位计算精度"""
    if price_tick <= 0:
        return 4
    s = f"{price_tick:.10f}".rstrip("0")
    if "." in s:
        return len(s.split(".")[1])
    return 0


def stock_code_to_instrument_id(stock_code: str) -> InstrumentId:
    """
    将 XtQuant 代码转换为 InstrumentId

    例如:
    600000.SH -> 600000.SSE
    000001.SZ -> 000001.SZSE
    """
    parts = stock_code.split(".")
    if len(parts) != 2:
        # Fallback for unknown format
        return InstrumentId(Symbol(stock_code), TT_VENUE)

    symbol = parts[0]
    market = parts[1]

    venue_str = MARKET_TO_VENUE.get(market, TT_VENUE.value)
    return InstrumentId(Symbol(symbol), Venue(venue_str))


def instrument_id_to_stock_code(instrument_id: InstrumentId, cache: Any | None = None) -> str:
    """
    将 InstrumentId 转换为 XtQuant 代码

    如果提供 cache, 尝试从缓存的 instrument 获取市场信息
    否则根据 symbol 首字符推断市场
    """
    symbol = str(instrument_id.symbol)

    # 尝试从 cache 获取市场信息
    if cache:
        instrument = cache.instrument(instrument_id)
        if instrument and hasattr(instrument, "exchange"):
            market = VENUE_TO_MARKET.get(str(instrument.exchange), "SH")
            return f"{symbol}.{market}"

    # 尝试使用 Venue 映射
    venue_str = instrument_id.venue.value
    if venue_str in VENUE_TO_MARKET:
        market = VENUE_TO_MARKET[venue_str]
        return f"{symbol}.{market}"

    # 作为后备, 根据 symbol 首字符推断市场 (仅适用于股票)
    if symbol.startswith("6"):
        return f"{symbol}.SH"  # 上交所 A 股
    elif symbol.startswith(("0", "3")):
        return f"{symbol}.SZ"  # 深交所 A 股
    elif symbol.startswith(("8", "4")):
        return f"{symbol}.BJ"  # 北交所
    elif len(symbol) <= 4:  # 简单的期货逻辑
        return f"{symbol}.IF"
    else:
        return f"{symbol}.SH"  # 最后的默认值
