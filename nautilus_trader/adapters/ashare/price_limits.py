from decimal import Decimal, ROUND_DOWN, ROUND_UP

# A股涨跌幅限制表：(板块, 状态) -> (上涨比例, 下跌比例)
LIMIT_TABLE: dict[tuple[str, str], tuple[Decimal, Decimal] | None] = {
    ("MAIN", "NORMAL"):   (Decimal("0.10"), Decimal("0.10")), # 主板普通
    ("MAIN", "ST"):       (Decimal("0.05"), Decimal("0.05")), # 主板ST
    ("GEM", "NORMAL"):    (Decimal("0.20"), Decimal("0.20")), # 创业板
    ("STAR", "NORMAL"):   (Decimal("0.20"), Decimal("0.20")), # 科创板
    ("STAR", "IPO5"):     None,  # 科创板上市前5日，不设涨跌幅
}

def compute_limits(
    board: str, status: str, prev_close: Decimal, tick: Decimal = Decimal("0.01"),
) -> tuple[Decimal | None, Decimal | None]:
    """
    计算 A 股涨跌停价格
    """
    entry = LIMIT_TABLE.get((board, status), (Decimal("0.10"), Decimal("0.10")))
    if entry is None:
        return None, None
    pct_up, pct_down = entry
    # 涨停价：向上取整到 tick
    up = (prev_close * (1 + pct_up) / tick).quantize(Decimal('1'), rounding=ROUND_DOWN) * tick
    # 跌停价：向下取整到 tick
    down = (prev_close * (1 - pct_down) / tick).quantize(Decimal('1'), rounding=ROUND_UP) * tick
    return up, down
