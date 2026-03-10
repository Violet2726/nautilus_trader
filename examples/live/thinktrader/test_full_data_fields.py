#!/usr/bin/env python3
"""
ThinkTrader 完整数据字段测试工具

测试目标：
1. 测试所有可以获取的数据字段（持仓、成交、委托、资金）
2. 显示每个数据源的完整字段及值
3. 验证扩展数据结构的有效性

注意：此脚本仅测试数据获取功能，不涉及交易执行
"""

import asyncio
import os
import random
import sys
import warnings
from datetime import datetime, timedelta
from pathlib import Path

import pandas as pd

# 抑制烦人的警告信息
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LiveExecEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.trading.strategy import Strategy


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


class FullDataTestConfig(StrategyConfig, frozen=True):
    """完整数据测试配置"""
    query_delay_secs: float = 3.0
    auto_stop_secs: float = 30.0


class FullDataTestStrategy(Strategy):
    """
    完整数据字段测试策略
    
    测试流程：
    1. 查询账户资金（显示所有字段）
    2. 查询持仓（显示所有字段）
    3. 查询委托（显示所有字段）
    4. 查询成交（显示所有字段）
    5. 格式化输出所有数据
    """
    
    def __init__(self, config: FullDataTestConfig):
        super().__init__(config)
        self.query_delay_secs = config.query_delay_secs
        self.auto_stop_secs = config.auto_stop_secs
        self.account_id = None
        self.test_results = []
    
    def on_start(self):
        """策略启动"""
        self.log.info("=" * 100)
        self.log.info("ThinkTrader 完整数据字段测试")
        self.log.info("=" * 100)
        
        # 获取账户 ID
        account_id_raw = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
        self.account_id = AccountId(f"{TT}-{account_id_raw}")
        
        self.log.info(f"测试账户：{self.account_id}")
        self.log.info(f"测试时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
        self.log.info("=" * 100)
        
        # 延迟查询
        self.clock.set_time_alert(
            name="query_all",
            alert_time=self.clock.utc_now() + timedelta(seconds=self.query_delay_secs),
            callback=lambda e: self._query_all_data(),
        )
        
        # 自动停止
        if self.auto_stop_secs > 0:
            self.clock.set_time_alert(
                name="auto_stop",
                alert_time=self.clock.utc_now() + timedelta(seconds=self.auto_stop_secs),
                callback=lambda e: self.stop(),
            )
    
    def on_stop(self):
        """策略停止"""
        self.log.info("\n" + "=" * 100)
        self.log.info("测试结束")
        self.log.info("=" * 100)
        
        # 打印测试结果摘要
        if self.test_results:
            print("\n测试结果摘要:")
            for result in self.test_results:
                status = "✓" if result["success"] else "✗"
                self.log.info(f"{status} {result['name']}: {result['count']} 条记录")
    
    def _query_all_data(self):
        """查询所有数据"""
        from nautilus_trader.adapters.thinktrader.factories import THINKTRADER_CLIENTS
        
        if not THINKTRADER_CLIENTS:
            self.log.error("未找到 ThinkTraderClient 实例")
            return
        
        # 获取第一个带有账户信息的客户端
        client = None
        for c in THINKTRADER_CLIENTS.values():
            if hasattr(c, '_account') and c._account is not None:
                client = c
                break
        
        if client is None:
            self.log.error("未找到已初始化账户的 ThinkTraderClient 实例")
            self.log.error("请确保执行客户端配置中包含了正确的 account_id 和 account_type")
            return
        
        self.log.info(f"使用客户端：{client}")
        self.log.info(f"账户：{client._account}")
        
        # 1. 查询账户资金
        self._test_account_asset(client)
        
        # 2. 查询持仓
        self._test_positions(client)
        
        # 3. 查询委托
        self._test_orders(client)
        
        # 4. 查询成交
        self._test_trades(client)
    
    def _test_account_asset(self, client):
        """测试账户资金数据"""
        self.log.info("\n" + "=" * 100)
        self.log.info("1. 账户资金数据")
        self.log.info("=" * 100)
        
        try:
            asset_data = client.query_asset()
            
            if asset_data:
                self.log.info(f"✓ 成功获取账户资金数据")
                self.log.info(f"  字段数量：{len(asset_data)}")
                
                print("\n【账户资金完整字段】")
                print("-" * 100)
                for key, value in asset_data.items():
                    print(f"  {key:30s}: {value}")
                print("-" * 100)
                
                self.test_results.append({
                    "name": "账户资金",
                    "success": True,
                    "count": len(asset_data),
                })
            else:
                self.log.warning("✗ 未获取到账户资金数据")
                self.test_results.append({
                    "name": "账户资金",
                    "success": False,
                    "count": 0,
                })
        except Exception as e:
            self.log.error(f"✗ 查询账户资金失败：{e}")
            self.test_results.append({
                "name": "账户资金",
                "success": False,
                "count": 0,
                "error": str(e),
            })
    
    def _test_positions(self, client):
        """测试持仓数据"""
        self.log.info("\n" + "=" * 100)
        self.log.info("2. 持仓数据")
        self.log.info("=" * 100)
        
        try:
            positions = client.query_positions()
            
            if positions:
                self.log.info(f"✓ 成功获取持仓数据")
                self.log.info(f"  持仓数量：{len(positions)}")
                
                # 显示第一个持仓的完整字段
                first_pos = positions[0]
                print(f"\n【持仓完整字段】 (共 {len(positions)} 条持仓)")
                print("-" * 100)
                print(f"字段数量：{len(first_pos._fields)}")
                print("\n字段列表:")
                for i, field in enumerate(first_pos._fields, 1):
                    value = getattr(first_pos, field)
                    print(f"  {i:2d}. {field:25s}: {value}")
                print("-" * 100)
                
                # 格式化表格输出
                print("\n【持仓汇总表】")
                pos_data = []
                for pos in positions:
                    pos_data.append({
                        "证券代码": pos.stock_code,
                        "证券名称": pos.stock_name,
                        "总数量": pos.volume,
                        "可用数量": pos.available_volume,
                        "冻结数量": pos.frozen_volume,
                        "在途股份": pos.pending_volume,
                        "市值": pos.market_value,
                        "最新价": pos.last_price,
                        "成本价": pos.avg_price,
                        "盈亏": pos.float_pnl,
                        "盈亏比例": f"{pos.profit_loss_ratio:.2f}%",
                        "昨夜拥股": pos.overnight_volume,
                        "账号名称": pos.account_name,
                        "证券公司": pos.broker_name,
                        "到期日": pos.expiry_date,
                    })
                
                df = pd.DataFrame(pos_data)
                print(df.to_string(index=False))
                print("-" * 100)
                
                self.test_results.append({
                    "name": "持仓数据",
                    "success": True,
                    "count": len(positions),
                })
            else:
                self.log.info("✓ 当前无持仓")
                self.test_results.append({
                    "name": "持仓数据",
                    "success": True,
                    "count": 0,
                })
        except Exception as e:
            self.log.error(f"✗ 查询持仓失败：{e}")
            import traceback
            traceback.print_exc()
            self.test_results.append({
                "name": "持仓数据",
                "success": False,
                "count": 0,
                "error": str(e),
            })
    
    def _test_orders(self, client):
        """测试委托数据"""
        self.log.info("\n" + "=" * 100)
        self.log.info("3. 委托数据")
        self.log.info("=" * 100)
        
        try:
            orders = client.query_orders()
            
            if orders:
                self.log.info(f"✓ 成功获取委托数据")
                self.log.info(f"  委托数量：{len(orders)}")
                
                # 显示第一个委托的完整字段
                first_order = orders[0]
                print(f"\n【委托完整字段】 (共 {len(orders)} 条委托)")
                print("-" * 100)
                print(f"字段数量：{len([attr for attr in dir(first_order) if not attr.startswith('_')])}")
                print("\n字段列表:")
                
                # 获取所有属性
                order_fields = {}
                for attr in dir(first_order):
                    if not attr.startswith('_') and not callable(getattr(first_order, attr)):
                        try:
                            value = getattr(first_order, attr)
                            order_fields[attr] = value
                        except:
                            pass
                
                for i, (field, value) in enumerate(order_fields.items(), 1):
                    print(f"  {i:2d}. {field:25s}: {value}")
                print("-" * 100)
                
                # 格式化表格输出
                print("\n【委托汇总表】")
                order_data = []
                for order in orders[:20]:  # 只显示前 20 条
                    try:
                        order_data.append({
                            "合同编号": order.order_id,
                            "证券代码": order.stock_code,
                            "证券名称": getattr(order, "stock_name", ""),
                            "委托时间": self._format_time(getattr(order, "order_time", 0)),
                            "买卖标记": self._get_order_side(getattr(order, "order_type", 0)),
                            "委托状态": self._get_order_status(getattr(order, "order_status", 0)),
                            "委托数量": order.order_volume,
                            "成交数量": order.traded_volume,
                            "已撤数量": getattr(order, "canceled_volume", 0),
                            "委托价格": order.price,
                            "成交均价": order.traded_price,
                            "冻结金额": getattr(order, "frozen_amount", 0),
                            "备注": getattr(order, "order_remark", ""),
                            "策略名称": getattr(order, "strategy_name", ""),
                        })
                    except Exception as e:
                        self.log.debug(f"处理委托失败：{e}")
                
                if order_data:
                    df = pd.DataFrame(order_data)
                    print(df.to_string(index=False))
                print("-" * 100)
                
                self.test_results.append({
                    "name": "委托数据",
                    "success": True,
                    "count": len(orders),
                })
            else:
                self.log.info("✓ 当日无委托")
                self.test_results.append({
                    "name": "委托数据",
                    "success": True,
                    "count": 0,
                })
        except Exception as e:
            self.log.error(f"✗ 查询委托失败：{e}")
            import traceback
            traceback.print_exc()
            self.test_results.append({
                "name": "委托数据",
                "success": False,
                "count": 0,
                "error": str(e),
            })
    
    def _test_trades(self, client):
        """测试成交数据"""
        self.log.info("\n" + "=" * 100)
        self.log.info("4. 成交数据")
        self.log.info("=" * 100)
        
        try:
            trades = client.query_trades()
            
            if trades:
                self.log.info(f"✓ 成功获取成交数据")
                self.log.info(f"  成交数量：{len(trades)}")
                
                # 显示第一个成交的完整字段
                first_trade = trades[0]
                print(f"\n【成交完整字段】 (共 {len(trades)} 条成交)")
                print("-" * 100)
                
                # 使用 dir() 获取所有属性
                attrs = [attr for attr in dir(first_trade) if not attr.startswith('_')]
                print(f"字段数量：{len(attrs)}")
                print("\n字段列表:")
                valid_attrs = {}
                for i, attr in enumerate(attrs, 1):
                    try:
                        value = getattr(first_trade, attr)
                        if not callable(value):
                            print(f"  {i:2d}. {attr:25s}: {value}")
                            valid_attrs[attr] = value
                    except Exception as e:
                        print(f"  {i:2d}. {attr:25s}: 读取失败 - {e}")
                print("-" * 100)
                self.log.info(f"实际可用字段：{list(valid_attrs.keys())}")
                
                # 格式化表格输出 - 只使用确实存在的字段
                print("\n【成交汇总表】")
                trade_data = []
                for trade in trades[:20]:  # 只显示前 20 条
                    row = {
                        "成交时间": self._format_time(getattr(trade, 'traded_time', 0)),
                        "合同编号": getattr(trade, 'order_id', ''),
                        "证券代码": getattr(trade, 'stock_code', ''),
                    }
                    # 只添加存在的字段
                    if 'stock_name' in valid_attrs:
                        row["证券名称"] = getattr(trade, 'stock_name', 'N/A')
                    if 'order_type' in valid_attrs:
                        row["买卖标记"] = "买入" if getattr(trade, 'order_type', 0) == 23 else "卖出"
                    if 'traded_price' in valid_attrs:
                        row["成交价格"] = getattr(trade, 'traded_price', 0)
                    if 'traded_volume' in valid_attrs:
                        row["成交数量"] = getattr(trade, 'traded_volume', 0)
                    if 'traded_amount' in valid_attrs:
                        row["成交金额"] = getattr(trade, 'traded_amount', 0)
                    if 'traded_id' in valid_attrs:
                        row["成交编号"] = getattr(trade, 'traded_id', '')
                    if 'account_name' in valid_attrs:
                        row["账号名称"] = getattr(trade, 'account_name', 'N/A')
                    if 'remark' in valid_attrs:
                        row["备注"] = getattr(trade, 'remark', '')
                    if 'strategy_name' in valid_attrs:
                        row["策略名称"] = getattr(trade, 'strategy_name', '')
                    
                    trade_data.append(row)
                
                if trade_data:
                    df = pd.DataFrame(trade_data)
                    print(df.to_string(index=False))
                print("-" * 100)
                
                self.test_results.append({
                    "name": "成交数据",
                    "success": True,
                    "count": len(trades),
                })
            else:
                self.log.info("✓ 当日无成交")
                self.test_results.append({
                    "name": "成交数据",
                    "success": True,
                    "count": 0,
                })
        except Exception as e:
            self.log.error(f"✗ 查询成交失败：{e}")
            import traceback
            traceback.print_exc()
            self.test_results.append({
                "name": "成交数据",
                "success": False,
                "count": 0,
                "error": str(e),
            })
    
    @staticmethod
    def _format_time(raw_time) -> str:
        """格式化时间"""
        try:
            ts = int(raw_time)
            if ts <= 0:
                return ""
            if ts > 1e15:
                ts = ts / 1e9
            elif ts > 1e12:
                ts = ts / 1e3
            return datetime.fromtimestamp(ts).strftime("%H:%M:%S")
        except:
            return str(raw_time)
    
    @staticmethod
    def _get_order_side(order_type: int) -> str:
        """获取买卖标记"""
        if order_type == 23:
            return "买入"
        elif order_type == 24:
            return "卖出"
        else:
            return str(order_type)
    
    @staticmethod
    def _get_order_status(status: int) -> str:
        """获取状态标签"""
        labels = {
            48: "未报",
            49: "待报",
            50: "已报",
            51: "已报待撤",
            52: "部成待撤",
            53: "部撤",
            54: "已撤",
            55: "部成",
            56: "已成",
            57: "废单",
            255: "未知",
        }
        return labels.get(status, str(status))


# ============================================================================
# 主函数
# ============================================================================

def main():
    """主函数"""
    load_dotenv()
    
    # 从环境变量读取配置
    miniqmt_path = os.environ.get(
        "MINIQMT_PATH",
        r"D:\迅投极速策略交易系统交易终端 华福证券 QMT 仿真\userdata_mini"
    )
    account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
    account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")
    
    # 验证 MiniQMT 路径
    if not Path(miniqmt_path).exists():
        print(f"✗ 错误：MiniQMT 路径不存在：{miniqmt_path}")
        print("请设置环境变量 MINIQMT_PATH 或修改脚本中的路径")
        return
    
    print("=" * 100)
    print("ThinkTrader 完整数据字段测试")
    print("=" * 100)
    print(f"MiniQMT 路径：{miniqmt_path}")
    print(f"资金账号：{account_id}")
    print(f"账号类型：{account_type}")
    print(f"测试时间：{datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 100)
    
    # 配置交易节点
    trader_id = TraderId(f"TESTER-{random.randint(1000, 9999)}")
    
    instrument_provider = ThinkTraderInstrumentProviderConfig()
    
    config = TradingNodeConfig(
        trader_id=trader_id,
        logging=LoggingConfig(log_level="INFO"),
        data_engine=LiveDataEngineConfig(),
        exec_engine=LiveExecEngineConfig(),
        data_clients={
            TT: ThinkTraderDataClientConfig(
                miniqmt_path=miniqmt_path,
                session_id=random.randint(100000, 999999),
                instrument_provider=instrument_provider,
            ),
        },
        exec_clients={
            TT: ThinkTraderExecClientConfig(
                miniqmt_path=miniqmt_path,
                session_id=random.randint(100000, 999999),
                account_id=account_id,
                account_type=account_type,
                instrument_provider=instrument_provider,
                routing=RoutingConfig(default=True),
            ),
        },
    )
    
    # 创建策略实例
    strategy = FullDataTestStrategy(
        FullDataTestConfig(
            query_delay_secs=3.0,
            auto_stop_secs=30.0,
        ),
    )
    
    # 创建并启动节点
    print("\n正在初始化交易节点...")
    node = TradingNode(config=config)
    
    # 添加策略
    node.trader.add_strategy(strategy)
    
    # 注册工厂
    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    
    # 构建节点
    node.build()
    
    try:
        node.run()
    except KeyboardInterrupt:
        print("\n\n用户中断，退出测试")
    finally:
        print("\n正在停止节点...")
        node.dispose()


if __name__ == "__main__":
    main()
