import requests
import hashlib
import json
from datetime import datetime, timedelta
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

# ================= 配置与请求逻辑 =================
BASE_URL = "https://noesisai.cn"
API_ENDPOINT = "/quantization-ai/api/portal/stockNamePrediction"

HEADERS = {
    "Content-Type": "application/json",
    "tenant-id": "YOUR_TENANT_ID",
    "X-Access-Token": "YOUR_ACCESS_TOKEN"
}

BASE_PAYLOAD = {
    "capital": "1000000",
    "initDate": datetime.now().strftime("%Y%m%d"),
    "license": "c5ddbf5f-2917-44d4-9b66-d4ab239cbac5",
    "macAddress": "00:16:3E:41:EA:AF",
    "positionStockCodes": "",
    "securitiesAccount": "179",
    "sign": "noesis_2025"
}

success = False
for i in range(1, 11):
    target_date = (datetime.now() - timedelta(days=i)).strftime("%Y%m%d")
    payload = BASE_PAYLOAD.copy()
    payload["date"] = target_date
    payload['md5'] = generate_md5(payload)
    
    print(f"正在尝试获取日期 {target_date} 的数据 (尝试 {i}/10)...")
    
    try:
        response = requests.post(
            url=f"{BASE_URL}{API_ENDPOINT}",
            headers=HEADERS,
            data=json.dumps(payload),
            timeout=30
        )
        response.raise_for_status()
        result = response.json()
        
        # 检查是否获取到有效数据
        name_list = result.get('result', {}).get('nameList', []) if result.get('result') else []
        if name_list:
            print(f"✅ 日期 {target_date} 请求成功，获取到 {len(name_list)} 只股票！")
            print(json.dumps(result, indent=2, ensure_ascii=False))
            success = True
            break
        else:
            print(f"⚠️ 日期 {target_date} 返回结果为空，继续尝试前一天...")
            
    except requests.exceptions.RequestException as e:
        print(f"❌ 日期 {target_date} 请求错误: {e}")

if not success:
    print("FATAL: 经过10天追溯，仍未能获取到任何有效数据。")
# ===========================================