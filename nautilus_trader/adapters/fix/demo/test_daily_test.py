import warnings


warnings.filterwarnings("ignore", category=SyntaxWarning)

import json
import re
import socket

import requests


# 测试网络IP - 使用多个源获取
def get_public_ip():
    sources = [
        "https://icanhazip.com/",
        "https://checkip.amazonaws.com/",
        "https://ipv4.icanhazip.com/"
    ]
    ipv4_re = re.compile(r"(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)")
    for source in sources:
        try:
            response = requests.get(source, timeout=3)
            if response.status_code == 200:
                text = response.text.strip()
                try:
                    data = response.json()
                    if isinstance(data, dict) and "ip" in data:
                        return str(data["ip"]).strip()
                except Exception:
                    pass

                m = ipv4_re.search(text)
                if m:
                    return m.group(0)
        except Exception:
            continue
    return "Unknown"

# 获取本地IP
def get_local_ip():
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("114.80.213.49", 16669))
        ip = s.getsockname()[0]
        s.close()
        return ip
    except:
        return "Unknown"

print("=" * 60)
print("网络诊断信息")
print("=" * 60)
public_ip = get_public_ip()
local_ip = get_local_ip()
print(f"公网IP: {public_ip}")
print(f"本地IP: {local_ip}")
print("账号绑定IP: 124.160.32.18")
print("=" * 60)

if public_ip != "Unknown" and public_ip != "124.160.32.18":
    print("⚠️ 警告: 您的公网IP与账号绑定的IP不匹配！")
    print("这很可能是导致连接被拒绝的原因。")
    print("解决方案:")
    print("1. 联系 Wind 客服更新IP绑定")
    print("2. 或者使用VPN获取正确的IP地址")
    print("=" * 60)

from daily_real_wind import get_account
from daily_real_wind import get_deal
from daily_real_wind import get_order
from daily_real_wind import get_position
from daily_real_wind import place_order


class ContextInfo:
    def __init__(self):
        self.acct = "690"


C = ContextInfo()


def custom_serializer(obj):
    if hasattr(obj, "__dict__"):
        return obj.__dict__  # 对于有__dict__属性的对象
    elif isinstance(obj, list):
        # 递归处理列表中的元素
        return [custom_serializer(item) for item in obj]
    else:
        # 对于其他类型，让JSON模块处理
        return obj


def sell_all(positions):
    for p in positions:
        place_order(C, 24, p.m_sInstrumentID + "." + p.m_sExchangeID,
                    "11", 10, p.m_nVolume, "")


if __name__ == "__main__":
    # 接口测试
    # order_id_1 = place_order(C, 23, "000002.SZ", "11", 4.95, 100, "")
    # order_id_2 = place_order(C, 23, "000002.SZ", "12", None, 100, "FF")

    # cancel_order(C, order_id_1, 1)  # 撤单
    # cancel_order(C, order_id_2, 1)  # 撤单

    # 查询交易数据缓存
    account = get_account(C)
    position = get_position(C)
    order = get_order(C)
    deal = get_deal(C)

    print(f"account:{json.dumps(account, default=custom_serializer, indent=4, ensure_ascii=False)}")
    print(f"position:{json.dumps(position, default=custom_serializer, indent=4, ensure_ascii=False)}")
    print(f"order:{json.dumps(order, default=custom_serializer, indent=4, ensure_ascii=False)}")
    print(f"deal:{json.dumps(deal, default=custom_serializer, indent=4, ensure_ascii=False)}")

    # position = get_position(C)
    # sell_all(position)

    # place_order(C, 24, "000011.SZ", "12", None, 500, "FF")
    # cancel_order(C, "1858", 23)
