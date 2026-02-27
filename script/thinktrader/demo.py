import requests
import hashlib
import json

def generate_md5(params, api_key="noesis_2025"):
    # 复制参数并加入 apiKey
    md5_params = params.copy()
    md5_params['apiKey'] = api_key

    # 过滤掉 md5 字段（忽略大小写）
    md5_params = {k: v for k, v in md5_params.items() if not k.lower() == 'md5'}

    # 按字段名字符串自然排序
    sorted_items = sorted(md5_params.items(), key=lambda x: x[0])

    # 拼接成 key=value&key=value...
    param_string = '&'.join([f"{k}={v}" for k, v in sorted_items])

    # 用 UTF-8 编码生成 MD5（16进制，大写）
    md5_obj = hashlib.md5(param_string.encode('utf-8'))
    return md5_obj.hexdigest().upper()

# ================= 配置区域 =================
BASE_URL = "https://noesisai.cn"
API_ENDPOINT = "/quantization-ai/api/portal/stockNamePrediction"

HEADERS = {
    "Content-Type": "application/json",
    "tenant-id": "YOUR_TENANT_ID",
    "X-Access-Token": "YOUR_ACCESS_TOKEN"
}


PAYLOAD = {
    "capital": "1000000",
    "date": "20260226",
    "initDate": "20260227",
    "license": "c5ddbf5f-2917-44d4-9b66-d4ab239cbac5",
    "macAddress": "00:16:3E:41:EA:AF",
    "positionStockCodes": "",
    "securitiesAccount": "179",
    "sign": "noesis_2025"
}

# 生成 MD5
PAYLOAD['md5'] = generate_md5(PAYLOAD)
# ===========================================

try:
    response = requests.post(
        url=f"{BASE_URL}{API_ENDPOINT}",
        headers=HEADERS,
        data=json.dumps(PAYLOAD),
        timeout=30
    )
    response.raise_for_status()
    result = response.json()
    print("请求成功：")
    print(json.dumps(result, indent=2, ensure_ascii=False))
except requests.exceptions.RequestException as e:
    print(f"请求错误: {e}")