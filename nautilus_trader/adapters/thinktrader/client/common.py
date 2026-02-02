from typing import NamedTuple, Any
from abc import ABC


class TTPosition(NamedTuple):
    """ThinkTrader 持仓结构"""
    account_id: str
    stock_code: str
    volume: int
    available_volume: int
    avg_price: float
    market_value: float


class TTOrder(NamedTuple):
    """ThinkTrader 订单结构"""
    order_id: int
    order_sysid: str
    stock_code: str
    order_type: int
    order_volume: int
    traded_volume: int
    price: float
    traded_price: float
    order_status: int
    order_remark: str


class TTTrade(NamedTuple):
    """ThinkTrader 成交结构"""
    account_id: str
    order_id: int
    order_sysid: str
    stock_code: str
    traded_id: str
    traded_price: float
    traded_volume: int
    traded_time: int


class BaseMixin(ABC):
    """Mixin 基类，提供类型提示"""
    _loop: Any
    _log: Any
    _trader: Any
    _account: Any
    _miniqmt_path: str
    _session_id: int
    _account_id: str
    _subscriptions: dict
    _requests: dict
