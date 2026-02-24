"""
历史数据批量下载工具

功能：
1. 批量下载多只股票的历史K线数据
2. 批量下载历史Tick数据
3. 自动保存为CSV文件

使用方法：
python historical_download.py
"""

import asyncio
import datetime
import os
from pathlib import Path

import pandas as pd
from dotenv import load_dotenv

from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient


# 加载环境变量（可选）
load_dotenv()

# ============================================================================
# 配置参数
# ============================================================================

# MiniQMT路径
MINIQMT_PATH = os.getenv(
    "MINIQMT_PATH",
    r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini",
)

# 输出目录
OUTPUT_DIR = Path(__file__).parent / "outputs"
OUTPUT_DIR.mkdir(exist_ok=True)

# 要下载的股票列表
TARGET_SYMBOLS = [
    "601808.SSE",  # 中国海油
    "601005.SSE",  # 重庆钢铁
    "000001.SZSE", # 平安银行
]

# K线类型
BAR_TYPES = [
    "1-MINUTE-LAST",   # 1分钟
    "5-MINUTE-LAST",   # 5分钟
    "15-MINUTE-LAST",  # 15分钟
    "30-MINUTE-LAST",  # 30分钟
    "1-HOUR-LAST",     # 1小时
    "1-DAY-LAST",      # 日K
    "1-WEEK-LAST",     # 周K
    "1-MONTH-LAST",    # 月K
]

# 时间范围
END_TIME = datetime.datetime(2026, 2, 6, 15, 0, 0)  # 结束时间
BAR_DURATION = "60 D"  # K线数据：往前60天
TICK_DURATION_HOURS = 2  # Tick数据：往前2小时

# ============================================================================
# 工具函数
# ============================================================================


def bars_to_dataframe(bars) -> pd.DataFrame:
    """将Nautilus Bar对象转换为DataFrame"""
    if not bars:
        return pd.DataFrame()

    data = []
    for bar in bars:
        data.append(
            {
                "symbol": str(bar.bar_type.instrument_id),
                "bar_type": str(bar.bar_type.spec),
                "timestamp": pd.Timestamp(bar.ts_event, unit="ns"),
                "open": bar.open.as_double(),
                "high": bar.high.as_double(),
                "low": bar.low.as_double(),
                "close": bar.close.as_double(),
                "volume": bar.volume.as_double(),
            },
        )

    df = pd.DataFrame(data)
    df = df.sort_values(["symbol", "bar_type", "timestamp"])
    return df


def ticks_to_dataframe(ticks) -> pd.DataFrame:
    """将Nautilus QuoteTick对象转换为DataFrame"""
    if not ticks:
        return pd.DataFrame()

    data = []
    for tick in ticks:
        data.append(
            {
                "symbol": str(tick.instrument_id),
                "timestamp": pd.Timestamp(tick.ts_event, unit="ns"),
                "bid": tick.bid_price.as_double(),
                "ask": tick.ask_price.as_double(),
                "bid_size": tick.bid_size.as_double(),
                "ask_size": tick.ask_size.as_double(),
            },
        )

    df = pd.DataFrame(data)
    df = df.sort_values(["symbol", "timestamp"])
    return df


# ============================================================================
# 下载任务
# ============================================================================


async def download_bars(client: HistoricThinkTraderClient):
    """下载历史K线数据"""
    print("=" * 80)
    print("任务1: 下载历史K线数据")
    print("=" * 80)
    print(f"股票列表: {TARGET_SYMBOLS}")
    print(f"K线类型: {BAR_TYPES}")
    print(f"结束时间: {END_TIME}")
    print(f"时间跨度: {BAR_DURATION}")
    print()

    try:
        bars = await client.request_bars(
            bar_specifications=BAR_TYPES,
            instrument_ids=TARGET_SYMBOLS,
            end_date_time=END_TIME,
            duration=BAR_DURATION,
            tz_name="Asia/Shanghai",
            timeout=180,  # 3分钟超时
        )

        print(f"✓ 下载成功: 共 {len(bars)} 根K线")
        print()

        # 转换为DataFrame
        df = bars_to_dataframe(bars)

        # 按K线类型分组保存
        for bar_type in BAR_TYPES:
            df_type = df[df["bar_type"] == bar_type]
            if not df_type.empty:
                # 生成文件名
                bar_type_safe = bar_type.replace("-", "_").lower()
                filename = OUTPUT_DIR / f"bars_{bar_type_safe}.csv"

                # 保存
                df_type.to_csv(filename, index=False)
                print(f"✓ 已保存 {bar_type}: {filename} ({len(df_type)} 行)")



        # 数据统计
        print("数据统计:")
        print(df.groupby(["symbol", "bar_type"]).size())
        print()

        return df

    except Exception as e:
        print(f"✗ 下载失败: {e}")
        import traceback

        traceback.print_exc()
        return None


async def download_ticks(client: HistoricThinkTraderClient):
    """下载历史Tick数据"""
    print("=" * 80)
    print("任务2: 下载历史Tick数据")
    print("=" * 80)

    # 计算时间范围（往前N小时）
    end_time = END_TIME
    start_time = end_time - datetime.timedelta(hours=TICK_DURATION_HOURS)

    print(f"股票列表: {TARGET_SYMBOLS}")
    print(f"开始时间: {start_time}")
    print(f"结束时间: {end_time}")
    print()

    all_ticks = []

    # 逐个下载（避免数据量过大）
    for symbol in TARGET_SYMBOLS:
        try:
            print(f"正在下载 {symbol} 的Tick数据...")

            ticks = await client.request_ticks(
                tick_type="BID_ASK",
                instrument_ids=[symbol],
                start_date_time=start_time,
                end_date_time=end_time,
                tz_name="Asia/Shanghai",
                timeout=120,
                limit=0,  # 不限制
            )

            print(f"✓ {symbol}: 下载了 {len(ticks)} 个Tick")
            all_ticks.extend(ticks)

        except Exception as e:
            print(f"✗ {symbol}: 下载失败 - {e}")

    print()
    print(f"✓ 总计下载: {len(all_ticks)} 个Tick")
    print()

    if all_ticks:
        # 转换为DataFrame
        df = ticks_to_dataframe(all_ticks)

        # 保存为CSV
        tick_file = OUTPUT_DIR / "ticks_quote.csv"
        df.to_csv(tick_file, index=False)
        print(f"✓ 已保存: {tick_file} ({len(df)} 行)")
        print()

        # 数据统计
        print("数据统计:")
        print(df.groupby("symbol").size())
        print()

        # 显示样例数据
        print("样例数据（前5行）:")
        print(df.head())
        print()

        return df

    return None


async def download_instruments(client: HistoricThinkTraderClient):
    """下载合约信息"""
    print("=" * 80)
    print("任务0: 加载合约信息")
    print("=" * 80)

    try:
        instruments = await client.request_instruments(
            instrument_ids=TARGET_SYMBOLS,
        )

        print(f"✓ 加载了 {len(instruments)} 个合约")
        print()

        # 显示合约信息
        for inst in instruments:
            print(f"  {inst.id}:")
            print(f"    价格精度: {inst.price_precision}")
            print(f"    数量精度: {inst.size_precision}")
            print(f"    最小价格变动: {inst.price_increment}")
            print(f"    交易单位: {inst.lot_size}")
            print()

        return instruments

    except Exception as e:
        print(f"✗ 加载失败: {e}")
        return None


# ============================================================================
# 主程序
# ============================================================================


async def main():
    """主函数"""
    print()
    print("╔" + "=" * 78 + "╗")
    print("║" + " " * 20 + "ThinkTrader 历史数据下载工具" + " " * 20 + "║")
    print("╚" + "=" * 78 + "╝")
    print()
    print(f"MiniQMT路径: {MINIQMT_PATH}")
    print(f"输出目录: {OUTPUT_DIR}")
    print()

    # 检查MiniQMT路径
    if not Path(MINIQMT_PATH).exists():
        print(f"✗ 错误: MiniQMT路径不存在: {MINIQMT_PATH}")
        print("请修改脚本中的 MINIQMT_PATH 配置")
        return

    # 1. 创建客户端
    print("正在创建历史数据客户端...")
    client = HistoricThinkTraderClient(
        miniqmt_path=MINIQMT_PATH,
        session_id=999999,  # 使用固定ID
        log_level="INFO",
    )
    print("✓ 客户端创建成功")
    print()

    # 2. 加载合约信息
    await download_instruments(client)

    # 3. 下载K线数据
    df_bars = await download_bars(client)

    # 4. 下载Tick数据
    df_ticks = await download_ticks(client)

    # 5. 总结
    print("=" * 80)
    print("下载任务完成!")
    print("=" * 80)
    print(f"输出目录: {OUTPUT_DIR.absolute()}")
    print()

    if df_bars is not None:
        print("K线数据文件:")
        for bar_type in BAR_TYPES:
            bar_type_safe = bar_type.replace("-", "_").lower()
            filename = OUTPUT_DIR / f"bars_{bar_type_safe}.csv"
            if filename.exists():
                size_kb = filename.stat().st_size / 1024
                print(f"  - {filename.name} ({size_kb:.1f} KB)")

        print()

    if df_ticks is not None:
        tick_file = OUTPUT_DIR / "ticks_quote.csv"
        if tick_file.exists():
            size_kb = tick_file.stat().st_size / 1024
            print("Tick数据文件:")
            print(f"  - {tick_file.name} ({size_kb:.1f} KB)")
        print()



if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print()
        print("用户中断，退出程序")
    except Exception as e:
        print(f"程序异常: {e}")
        import traceback

        traceback.print_exc()
