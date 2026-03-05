import pytest
from decimal import Decimal

from nautilus_trader.adapters.ashare.price_limits import compute_limits

def test_compute_limits_main_board_normal():
    # 上证主板，常规 10%
    # 昨日收盘价 10.00，涨停：11.00，跌停：9.00
    up, down = compute_limits("MAIN", "NORMAL", Decimal("10.00"), Decimal("0.01"))
    assert up == Decimal("11.00")
    assert down == Decimal("9.00")

    # 昨日收盘价 10.05
    # 涨停 10.05 * 1.1 = 11.055 -> 四舍五入到 0.01 -> 这里是向下取整 / 向上取整机制
    up, down = compute_limits("MAIN", "NORMAL", Decimal("10.05"), Decimal("0.01"))
    assert up == Decimal("11.05")
    assert down == Decimal("9.05")

def test_compute_limits_main_board_st():
    # 上证主板 ST，5%
    # 昨日收盘价 10.00
    up, down = compute_limits("MAIN", "ST", Decimal("10.00"), Decimal("0.01"))
    assert up == Decimal("10.50")
    assert down == Decimal("9.50")

def test_compute_limits_gem_star_normal():
    # 创业板/科创板，常规 20%
    up, down = compute_limits("GEM", "NORMAL", Decimal("10.00"), Decimal("0.01"))
    assert up == Decimal("12.00")
    assert down == Decimal("8.00")

    up, down = compute_limits("STAR", "NORMAL", Decimal("50.00"), Decimal("0.01"))
    assert up == Decimal("60.00")
    assert down == Decimal("40.00")

def test_compute_limits_star_ipo():
    # 科创板 前5日无涨跌幅
    up, down = compute_limits("STAR", "IPO5", Decimal("50.00"), Decimal("0.01"))
    assert up is None
    assert down is None
