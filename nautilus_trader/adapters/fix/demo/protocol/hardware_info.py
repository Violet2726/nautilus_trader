import pythoncom
import requests
import wmi
import socket
import re


def get_hardware_info():
    pythoncom.CoInitialize()
    try:
        ipv4_re = re.compile(r"(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)")

        public_ip = None
        for url in ("https://icanhazip.com/", "https://checkip.amazonaws.com/", "https://ipv4.icanhazip.com/"):
            try:
                resp = requests.get(url, timeout=3)
                if resp.status_code != 200:
                    continue
                m = ipv4_re.search(resp.text)
                if m:
                    public_ip = m.group(0)
                    break
            except Exception:
                continue

        disk_id = r"\\.\PHYSICALDRIVE0"
        cpu_id = "BFEBFBFF000A0671"
        disk_sn = "E823_8FA6_BF53_0001_001B_444A_498C_11F0."

        return {"cpu_id": cpu_id, "disk_id": disk_id, "disk_sn": disk_sn, "from_ip": public_ip or "Unknown"}
    finally:
        pythoncom.CoUninitialize()
