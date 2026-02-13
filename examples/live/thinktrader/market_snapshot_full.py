"""
ThinkTrader 全市场行情快照导出 (使用适配器版)

功能：
1. 使用 Nautilus ThinkTrader Adapter 获取沪深全市场股票行情
2. 包含核心行情指标 (最新价、涨跌幅、成交量、成交额等)
3. 导出为 CSV 文件到 outputs 目录
"""

import os
import time
import pandas as pd
from datetime import datetime
from pathlib import Path
from dotenv import load_dotenv

from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient

# 加载环境变量
load_dotenv()

# ============================================================================
# 配置参数
# ============================================================================

# MiniQMT路径
MINIQMT_PATH = os.getenv(
    "MINIQMT_PATH",
    r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini",
)

# 输出目录 (相对于脚本所在位置)
SCRIPT_DIR = Path(__file__).parent
OUTPUT_DIR = SCRIPT_DIR / "outputs"
OUTPUT_DIR.mkdir(parents=True, exist_ok=True)


def get_full_market_snapshot(client: HistoricThinkTraderClient):
    """获取全市场 A 股行情快照"""
    # 1. 获取所有沪深 A 股代码
    print("正在获取沪深 A 股股票列表...")
    stock_codes = client.get_stock_list("沪深A股")
    print(f"共发现 {len(stock_codes)} 只股票")

    # 2. 批量请求数据
    print("正在通过适配器批量请求行情数据...")
    
    k_fields = ['open', 'high', 'low', 'close', 'volume', 'amount', 'preClose']
    
    # 批量获取数据
    batch_data = client.get_market_data(
        field_list=k_fields,
        stock_list=stock_codes,
        period='1d',
        count=1
    )
    
    # 3. 处理数据
    print("正在处理数据并合并...")
    records = []
    
    for code in stock_codes:
        try:
            # 提取数据
            last_price = float(batch_data['close'].loc[code].iloc[-1])
            pre_close = float(batch_data['preClose'].loc[code].iloc[-1])
            open_p = float(batch_data['open'].loc[code].iloc[-1])
            high = float(batch_data['high'].loc[code].iloc[-1])
            low = float(batch_data['low'].loc[code].iloc[-1])
            volume = float(batch_data['volume'].loc[code].iloc[-1])
            amount = float(batch_data['amount'].loc[code].iloc[-1])
            
            if last_price <= 0 or pre_close <= 0:
                continue

            change_val = last_price - pre_close
            change_pct = (change_val / pre_close) * 100
            amplitude = ((high - low) / pre_close) * 100
            
            # 获取名称
            detail = client.get_instrument_detail(code)
            name = detail.get("InstrumentName", "未知") if detail else "未知"
            
            records.append({
                "代码": code,
                "名称": name,
                "最新价": round(last_price, 2),
                "涨跌幅(%)": round(change_pct, 2),
                "涨跌额": round(change_val, 2),
                "成交量(手)": int(volume),
                "成交额(元)": round(amount, 2),
                "振幅(%)": round(amplitude, 2),
                "最高": round(high, 2),
                "最低": round(low, 2),
                "今开": round(open_p, 2),
                "昨收": round(pre_close, 2)
            })
        except Exception:
            continue
            
    df = pd.DataFrame(records)
    if not df.empty:
        df = df.sort_values(by="代码").reset_index(drop=True)
    return df


def main():
    if not Path(MINIQMT_PATH).exists():
        print(f"✗ 错误: MiniQMT路径不存在: {MINIQMT_PATH}")
        return

    # 初始化适配器客户端
    client = HistoricThinkTraderClient(miniqmt_path=MINIQMT_PATH)
    
    start_time = time.time()
    df = get_full_market_snapshot(client)
    
    if df is not None and not df.empty:
        today = datetime.now().strftime("%Y%m%d")
        filename = OUTPUT_DIR / f"market_snapshot_{today}.csv"
        df.to_csv(filename, index=False, encoding='utf-8-sig')
        
        elapsed = time.time() - start_time
        print(f"\n✓ 任务完成!")
        print(f"有效数据: {len(df)} 行")
        print(f"文件路径: {filename}")
        print(f"总耗时: {elapsed:.2f} 秒")
    else:
        print("\n未能生成快照文件。")


if __name__ == "__main__":
    main()
