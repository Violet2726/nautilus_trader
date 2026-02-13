from __future__ import annotations


def get_exchange_info_v3(order_code: str) -> tuple[str, str]:
    _, suffix = order_code.split(".")
    suffix = suffix.upper()

    mapping = {
        "SH": ("SSE", "CS"),
        "SZ": ("SZSE", "CS"),
        "HK": ("HKEX", "CS"),
        "N": ("NYSEA", "CS"),
        "OQ": ("NASDAQ", "CS"),
        "A": ("AMEX", "CS"),
        "SHN": ("SZN", "CS"),
        "SZN": ("SHN", "CS"),
    }

    res = mapping.get(suffix, ("SSE", "CS"))
    return res[0], res[1]


def get_hardware_info() -> dict[str, str]:
    return {
        "cpu_id": "BFEBFBFF000A0671",
        "disk_id": r"\\.\PHYSICALDRIVE0",
        "disk_sn": "E823_8FA6_BF53_0001_001B_444A_498C_11F0.",
        "from_ip": "Unknown",
    }

