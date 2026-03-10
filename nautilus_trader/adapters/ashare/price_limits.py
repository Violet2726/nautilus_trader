from decimal import Decimal

from nautilus_trader.core.nautilus_pyo3.markets import (
    BoardType,
    StockStatus,
    compute_ashare_price_limits_by_board_status,
    identify_ashare_board_type_from_str,
    identify_ashare_stock_status_from_str,
)
from nautilus_trader.core.nautilus_pyo3.model import Price

def compute_limits(
    board: str, status: str, prev_close: Decimal, tick: Decimal = Decimal("0.01"),
) -> tuple[Decimal | None, Decimal | None]:
    """
    计算 A 股涨跌停价格
    """
    prev_close_f = float(prev_close)
    tick_f = float(tick)
    prev_close_p = Price(prev_close_f, max(-prev_close.as_tuple().exponent, 0))
    tick_p = Price(tick_f, max(-tick.as_tuple().exponent, 0))

    b = identify_ashare_board_type_from_str(board)
    s = identify_ashare_stock_status_from_str(status)

    limit_up, limit_down = compute_ashare_price_limits_by_board_status(b, s, prev_close_p, tick_p)
    if limit_up is None or limit_down is None:
        return None, None
    return Decimal(str(limit_up.as_f64())), Decimal(str(limit_down.as_f64()))
