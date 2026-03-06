#!/usr/bin/env python3
"""
ThinkTrader 行情数据全面测试

测试目标：
1. 测试所有可以获取的行情数据
2. 记录每个数据源的详细字段
3. 不涉及性能测试，只关注数据可用性

注意：此脚本仅测试行情功能，不涉及交易功能
"""

import asyncio
import os
import sys
from datetime import datetime, timedelta
from pathlib import Path

import pandas as pd

from nautilus_trader.adapters.thinktrader.historical.client import HistoricThinkTraderClient


def load_dotenv():
    """加载环境变量"""
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


class ThinkTraderMarketDataTester:
    """ThinkTrader 行情数据测试器"""

    def __init__(self, miniqmt_path: str):
        self.miniqmt_path = miniqmt_path
        self.client = None
        self.test_results = []

    async def initialize(self):
        """初始化客户端"""
        try:
            print(f"正在初始化客户端...")
            self.client = HistoricThinkTraderClient(
                miniqmt_path=self.miniqmt_path,
                session_id=999999,
                log_level="INFO",
            )
            print("✓ 客户端初始化成功")
            return True
        except Exception as e:
            print(f"✗ 客户端初始化失败: {e}")
            return False

    def add_result(self, category: str, test_name: str, success: bool, data: dict = None):
        """添加测试结果"""
        self.test_results.append({
            "分类": category,
            "测试项": test_name,
            "状态": "✓ 成功" if success else "✗ 失败",
            "数据内容": str(data) if data else "",
        })

    def print_section(self, title: str):
        """打印分节标题"""
        print("\n" + "=" * 100)
        print(f"  {title}")
        print("=" * 100)

    def print_subsection(self, title: str):
        """打印子节标题"""
        print("\n" + "-" * 100)
        print(f"  {title}")
        print("-" * 100)

    async def test_stock_list(self):
        """测试股票列表"""
        self.print_subsection("1. 股票列表")

        try:
            stock_codes = self.client.get_stock_list("沪深A股")
            print(f"✓ 成功获取股票列表")
            print(f"  总数: {len(stock_codes)} 只股票")
            print(f"  示例: {stock_codes[:10]}")
            self.add_result("基础数据", "获取股票列表", True, {
                "总数": len(stock_codes),
                "示例": stock_codes[:5]
            })
            return stock_codes
        except Exception as e:
            print(f"✗ 获取股票列表失败: {e}")
            self.add_result("基础数据", "获取股票列表", False, str(e))
            return []

    async def test_trading_dates(self):
        """测试交易日历"""
        self.print_subsection("2. 交易日历")

        try:
            dates = self.client._client.get_trading_dates(
                market="SH",
                start_date="20260101",
                end_date="20260331"
            )
            print(f"✓ 成功获取交易日历")
            print(f"  交易日数量: {len(dates)}")
            print(f"  示例日期: {dates[:10]}")
            self.add_result("基础数据", "获取交易日历", True, {
                "数量": len(dates),
                "示例": dates[:5]
            })
        except Exception as e:
            print(f"✗ 获取交易日历失败: {e}")
            self.add_result("基础数据", "获取交易日历", False, str(e))

    async def test_instrument_detail(self, stock_code: str):
        """测试合约详情"""
        self.print_subsection("3. 合约详情")

        try:
            detail = self.client.get_instrument_detail(stock_code)
            if detail:
                print(f"✓ 成功获取合约详情")
                print(f"  字段总数: {len(detail)}")
                print(f"  所有字段及值:")
                for key, value in detail.items():
                    print(f"    {key}: {value}")
                self.add_result("合约信息", "获取合约详情", True, {
                    "字段数": len(detail),
                    "字段列表": list(detail.keys())
                })
                return detail
            else:
                print(f"✗ 未找到合约详情")
                self.add_result("合约信息", "获取合约详情", False, "未找到合约")
                return None
        except Exception as e:
            print(f"✗ 获取合约详情失败: {e}")
            self.add_result("合约信息", "获取合约详情", False, str(e))
            return None

    async def test_instrument_type(self, stock_code: str):
        """测试合约类型"""
        self.print_subsection("4. 合约类型")

        try:
            instrument_type = self.client._client.get_instrument_type(stock_code)
            print(f"✓ 成功获取合约类型")
            print(f"  类型: {instrument_type}")
            self.add_result("合约信息", "获取合约类型", True, instrument_type)
        except Exception as e:
            print(f"✗ 获取合约类型失败: {e}")
            self.add_result("合约信息", "获取合约类型", False, str(e))

    async def test_market_data_fields(self, stock_codes: list):
        """测试市场数据字段"""
        self.print_subsection("5. 市场数据字段")

        try:
            # 测试所有可能的字段
            all_fields = [
                "open", "high", "low", "close", "volume", "amount", "preClose",
                "lastPrice", "pctChg", "amplitude", "turnover", "pe", "pb", "ps"
            ]

            data = self.client.get_market_data(
                field_list=all_fields,
                stock_list=stock_codes[:5],
                period="1d",
                count=1
            )

            if data:
                print(f"✓ 成功获取市场数据")
                print(f"  可用字段数量: {len(data)}")
                print(f"  所有字段及数据:")

                for field, df in data.items():
                    if not df.empty:
                        print(f"\n  字段: {field}")
                        print(f"    数据类型: {df.dtypes[field] if field in df.columns else 'N/A'}")
                        print(f"    所有股票的值:")
                        for stock_code, value in df.iloc[:, 0].items():
                            print(f"      {stock_code}: {value}")

                self.add_result("市场数据", "获取市场数据", True, {
                    "可用字段": list(data.keys()),
                    "字段数量": len(data)
                })
                return data
            else:
                print(f"✗ 未获取到市场数据")
                self.add_result("市场数据", "获取市场数据", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取市场数据失败: {e}")
            self.add_result("市场数据", "获取市场数据", False, str(e))
            return None

    async def test_full_tick(self, stock_codes: list):
        """测试全推行情快照"""
        self.print_subsection("6. 全推行情快照")

        try:
            tick_data = self.client.get_full_tick(stock_codes[:5])
            if tick_data:
                print(f"✓ 成功获取全推行情快照")
                print(f"  股票数量: {len(tick_data)}")

                for code, data in list(tick_data.items())[:3]:
                    print(f"\n  股票: {code}")
                    print(f"    字段数量: {len(data)}")
                    print(f"    所有字段及值:")
                    for key, value in data.items():
                        print(f"      {key}: {value}")

                self.add_result("实时行情", "获取全推行情快照", True, {
                    "股票数量": len(tick_data),
                    "字段示例": list(list(tick_data.values())[0].keys()) if tick_data else []
                })
                return tick_data
            else:
                print(f"✗ 未获取到全推行情快照")
                self.add_result("实时行情", "获取全推行情快照", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取全推行情快照失败: {e}")
            self.add_result("实时行情", "获取全推行情快照", False, str(e))
            return None

    async def test_get_price(self, stock_code: str):
        """测试获取最新价格"""
        self.print_subsection("7. 获取最新价格")

        try:
            # 需要使用 instrument_id
            from nautilus_trader.model.identifiers import InstrumentId
            instrument_id = InstrumentId.from_str(f"{stock_code.split('.')[0]}.SSE")

            price = await self.client._client.get_price(instrument_id, stock_code)
            print(f"✓ 成功获取最新价格")
            print(f"  价格: {price}")
            self.add_result("实时行情", "获取最新价格", True, {"价格": price})
        except Exception as e:
            print(f"✗ 获取最新价格失败: {e}")
            self.add_result("实时行情", "获取最新价格", False, str(e))

    async def test_historical_bars(self, instrument_ids: list):
        """测试历史K线数据"""
        self.print_subsection("8. 历史K线数据")

        try:
            end_time = datetime.now()
            bars = await self.client.request_bars(
                bar_specifications=["1-MINUTE-LAST", "5-MINUTE-LAST", "15-MINUTE-LAST",
                                   "30-MINUTE-LAST", "1-HOUR-LAST", "1-DAY-LAST"],
                instrument_ids=instrument_ids[:2],
                end_date_time=end_time,
                duration="7 D",
                tz_name="Asia/Shanghai",
                timeout=120,
            )

            if bars:
                print(f"✓ 成功获取历史K线")
                print(f"  K线总数: {len(bars)}")

                # 统计不同类型的K线
                bar_types = {}
                for bar in bars:
                    bar_type = str(bar.bar_type.spec)
                    bar_types[bar_type] = bar_types.get(bar_type, 0) + 1

                print(f"  K线类型统计:")
                for bar_type, count in bar_types.items():
                    print(f"    {bar_type}: {count} 根")

                # 显示样例数据（每种类型显示3条）
                print(f"\n  样例K线数据:")
                for bar_type in bar_types.keys():
                    type_bars = [b for b in bars if str(b.bar_type.spec) == bar_type]
                    print(f"\n    类型: {bar_type}")
                    for i, sample in enumerate(type_bars[:3]):
                        print(f"      第{i+1}根:")
                        print(f"        合约: {sample.bar_type.instrument_id}")
                        print(f"        时间: {datetime.fromtimestamp(sample.ts_event / 1e9)}")
                        print(f"        OHLC: {sample.open}/{sample.high}/{sample.low}/{sample.close}")
                        print(f"        成交量: {sample.volume}")

                self.add_result("历史数据", "获取历史K线", True, {
                    "总数": len(bars),
                    "类型统计": bar_types
                })
                return bars
            else:
                print(f"✗ 未获取到历史K线")
                self.add_result("历史数据", "获取历史K线", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取历史K线失败: {e}")
            import traceback
            traceback.print_exc()
            self.add_result("历史数据", "获取历史K线", False, str(e))
            return None

    async def test_historical_ticks(self, instrument_ids: list):
        """测试历史Tick数据"""
        self.print_subsection("9. 历史Tick数据")

        try:
            end_time = datetime.now()
            start_time = end_time - timedelta(hours=1)

            ticks = await self.client.request_ticks(
                tick_type="BID_ASK",
                instrument_ids=instrument_ids[:2],
                start_date_time=start_time,
                end_date_time=end_time,
                tz_name="Asia/Shanghai",
                timeout=120,
                limit=0,
            )

            if ticks:
                print(f"✓ 成功获取历史Tick")
                print(f"  Tick总数: {len(ticks)}")

                # 显示样例数据（前5个）
                print(f"\n  样例Tick数据（前5个）:")
                for i, sample in enumerate(ticks[:5]):
                    print(f"    第{i+1}个Tick:")
                    print(f"      合约: {sample.instrument_id}")
                    print(f"      时间: {datetime.fromtimestamp(sample.ts_event / 1e9)}")
                    print(f"      买价: {sample.bid_price}")
                    print(f"      卖价: {sample.ask_price}")
                    print(f"      买量: {sample.bid_size}")
                    print(f"      卖量: {sample.ask_size}")

                self.add_result("历史数据", "获取历史Tick", True, {
                    "总数": len(ticks)
                })
                return ticks
            else:
                print(f"✗ 未获取到历史Tick")
                self.add_result("历史数据", "获取历史Tick", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取历史Tick失败: {e}")
            import traceback
            traceback.print_exc()
            self.add_result("历史数据", "获取历史Tick", False, str(e))
            return None

    async def test_fundamental_data(self, stock_code: str):
        """测试财务数据"""
        self.print_subsection("10. 财务数据")

        try:
            financial_data = await self.client._client.req_fundamental_data(
                instrument_id=stock_code,
                stock_code=stock_code,
                report_type="report_time",
                timeout=60,
            )

            if financial_data:
                print(f"✓ 成功获取财务数据")
                print(f"  数据类型: {list(financial_data.keys())}")

                if "financial" in financial_data:
                    financial = financial_data["financial"]
                    if isinstance(financial, dict) and stock_code in financial:
                        data = financial[stock_code]
                        if isinstance(data, pd.DataFrame):
                            print(f"\n  财务数据:")
                            print(f"    记录数: {len(data)}")
                            print(f"    字段数: {len(data.columns)}")
                            print(f"    所有字段:")
                            for col in data.columns:
                                print(f"      {col}")
                            print(f"\n    前3条记录:")
                            for idx, row in data.head(3).iterrows():
                                print(f"      记录 {idx}:")
                                for col in data.columns:
                                    print(f"        {col}: {row[col]}")
                        else:
                            print(f"  财务数据类型: {type(data)}")
                            print(f"  财务数据内容: {data}")

                if "detail" in financial_data:
                    detail = financial_data["detail"]
                    if detail:
                        print(f"\n  合约详情:")
                        print(f"    字段数: {len(detail)}")
                        print(f"    所有字段及值:")
                        for key, value in detail.items():
                            print(f"      {key}: {value}")

                self.add_result("财务数据", "获取财务数据", True, {
                    "数据类型": list(financial_data.keys())
                })
                return financial_data
            else:
                print(f"✗ 未获取到财务数据")
                self.add_result("财务数据", "获取财务数据", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取财务数据失败: {e}")
            self.add_result("财务数据", "获取财务数据", False, str(e))
            return None

    async def test_instrument_info(self, instrument_ids: list):
        """测试合约信息"""
        self.print_subsection("11. 合约信息")

        try:
            instruments = await self.client.request_instruments(instrument_ids[:5])

            if instruments:
                print(f"✓ 成功获取合约信息")
                print(f"  合约数量: {len(instruments)}")

                for inst in instruments[:3]:
                    print(f"\n  合约: {inst.id}")
                    print(f"    类型: {inst.__class__.__name__}")
                    print(f"    所有属性及值:")
                    for attr in dir(inst):
                        if not attr.startswith('_'):
                            try:
                                value = getattr(inst, attr)
                                if not callable(value):
                                    print(f"      {attr}: {value}")
                            except:
                                pass

                self.add_result("合约信息", "获取合约信息", True, {
                    "合约数量": len(instruments)
                })
                return instruments
            else:
                print(f"✗ 未获取到合约信息")
                self.add_result("合约信息", "获取合约信息", False, "未获取到数据")
                return None
        except Exception as e:
            print(f"✗ 获取合约信息失败: {e}")
            import traceback
            traceback.print_exc()
            self.add_result("合约信息", "获取合约信息", False, str(e))
            return None

    async def test_l2_quote(self, stock_codes: list):
        """测试 Level 2 五档行情"""
        self.print_subsection("12. Level 2 五档行情")

        try:
            import xtquant.xtdata as xtdata

            test_stock = stock_codes[0]
            print(f"测试股票: {test_stock}")

            # 尝试订阅 Level 2 行情
            seq = xtdata.subscribe_quote(
                stock_code=test_stock,
                period="l2quote",
                count=1,
                callback=None
            )

            if seq <= 0:
                print(f"✗ 订阅 Level 2 行情失败")
                print(f"  可能原因: 需要 Level 2 权限")
                self.add_result("Level 2 行情", "获取五档行情", False, "需要 Level 2 权限")
                return None

            print(f"✓ 订阅成功，seq={seq}")

            # 等待数据
            await asyncio.sleep(1)

            # 获取数据
            data = xtdata.get_market_data(
                field_list=[],
                stock_list=[test_stock],
                period="l2quote",
                count=1
            )

            # 取消订阅
            xtdata.unsubscribe_quote(seq)

            if data and test_stock in data:
                l2_data = data[test_stock]
                if isinstance(l2_data, dict):
                    print(f"✓ 成功获取 Level 2 五档行情")
                    print(f"  数据字段: {list(l2_data.keys())}")

                    # 提取五档数据
                    bid_prices = l2_data.get("bidPrice", [])
                    bid_vols = l2_data.get("bidVol", [])
                    ask_prices = l2_data.get("askPrice", [])
                    ask_vols = l2_data.get("askVol", [])

                    print(f"\n  买五档:")
                    for i in range(min(5, len(bid_prices))):
                        if bid_prices[i] > 0:
                            print(f"    买{i+1}: 价格={bid_prices[i]:.4f}, 数量={bid_vols[i]}")

                    print(f"\n  卖五档:")
                    for i in range(min(5, len(ask_prices))):
                        if ask_prices[i] > 0:
                            print(f"    卖{i+1}: 价格={ask_prices[i]:.4f}, 数量={ask_vols[i]}")

                    self.add_result("Level 2 行情", "获取五档行情", True, {
                        "买档数": len(bid_prices),
                        "卖档数": len(ask_prices)
                    })
                    return l2_data
                else:
                    print(f"✗ 数据格式错误: {type(l2_data)}")
                    self.add_result("Level 2 行情", "获取五档行情", False, "数据格式错误")
                    return None
            else:
                print(f"✗ 未获取到 Level 2 数据")
                self.add_result("Level 2 行情", "获取五档行情", False, "未获取到数据")
                return None

        except Exception as e:
            error_msg = str(e)
            if "level2 permission" in error_msg.lower():
                print(f"✗ 获取五档行情失败: 需要 Level 2 权限")
                self.add_result("Level 2 行情", "获取五档行情", False, "需要 Level 2 权限")
            else:
                print(f"✗ 获取五档行情失败: {e}")
                import traceback
                traceback.print_exc()
                self.add_result("Level 2 行情", "获取五档行情", False, str(e))
            return None

    async def run_all_tests(self):
        """运行所有测试"""
        self.print_section("ThinkTrader 行情数据全面测试")
        print(f"MiniQMT路径: {self.miniqmt_path}")
        print(f"测试时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
        print(f"测试目标: 获取所有可用的行情数据")

        # 1. 初始化
        if not await self.initialize():
            return

        # 2. 获取股票列表
        stock_codes = await self.test_stock_list()
        if not stock_codes:
            print("\n✗ 无法继续测试：未获取到股票列表")
            return

        # 3. 测试交易日历
        await self.test_trading_dates()

        # 4. 测试合约信息
        test_stock = stock_codes[0]
        await self.test_instrument_detail(test_stock)
        await self.test_instrument_type(test_stock)

        # 5. 测试市场数据
        await self.test_market_data_fields(stock_codes)

        # 6. 测试实时行情
        await self.test_full_tick(stock_codes)
        await self.test_get_price(test_stock)

        # 7. 测试历史数据
        instrument_ids = []
        for code in stock_codes[:5]:
            parts = code.split(".")
            if len(parts) == 2:
                symbol = parts[0]
                market = parts[1]
                venue = "SSE" if market == "SH" else "SZSE" if market == "SZ" else "BSE"
                instrument_ids.append(f"{symbol}.{venue}")

        await self.test_historical_bars(instrument_ids)
        await self.test_historical_ticks(instrument_ids)
        await self.test_instrument_info(instrument_ids)

        # 8. 测试财务数据
        await self.test_fundamental_data(test_stock)

        # 9. 测试 Level 2 五档行情
        await self.test_l2_quote(stock_codes)

        # 10. 打印测试结果
        self.print_results()


    def print_results(self):
        """打印测试结果"""
        self.print_section("测试结果汇总")

        df = pd.DataFrame(self.test_results)
        print(df.to_string(index=False))

        # 按分类统计
        print("\n" + "=" * 100)
        print("按分类统计")
        print("=" * 100)
        for category in df["分类"].unique():
            category_df = df[df["分类"] == category]
            success_count = len(category_df[category_df["状态"] == "✓ 成功"])
            total_count = len(category_df)
            print(f"{category}: {success_count}/{total_count} 项成功")

        # 总体统计
        success_count = len(df[df["状态"] == "✓ 成功"])
        total_count = len(df)
        print(f"\n总计: {success_count}/{total_count} 项成功 ({success_count/total_count*100:.1f}%)")


async def main():
    """主函数"""
    load_dotenv()

    miniqmt_path = os.environ.get(
        "MINIQMT_PATH",
        r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini"
    )

    if not Path(miniqmt_path).exists():
        print(f"✗ 错误: MiniQMT路径不存在: {miniqmt_path}")
        print("请设置环境变量 MINIQMT_PATH 或修改脚本中的路径")
        return

    tester = ThinkTraderMarketDataTester(miniqmt_path)
    await tester.run_all_tests()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n\n用户中断，退出测试")
    except Exception as e:
        print(f"\n程序异常: {e}")
        import traceback
        traceback.print_exc()
