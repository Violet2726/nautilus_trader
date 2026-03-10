from decimal import Decimal


def test_ashare_price_limits_python_entrypoint_uses_rust_when_available():
    from nautilus_trader.core.nautilus_pyo3.markets import compute_ashare_price_limits
    from nautilus_trader.core.nautilus_pyo3.model import Price

    prev_close = Price(10.13, 2)
    tick = Price(0.01, 2)
    limit_up, limit_down = compute_ashare_price_limits("600000", "平安银行", prev_close, tick)

    assert Decimal(str(float(limit_up))) == Decimal("11.15")
    assert Decimal(str(float(limit_down))) == Decimal("9.11")
