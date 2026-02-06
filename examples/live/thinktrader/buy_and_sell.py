#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
# -------------------------------------------------------------------------------------------------

import os
import random
import warnings
from pathlib import Path


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
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.objects import Price
from nautilus_trader.trading.strategy import Strategy


def _load_dotenv() -> None:
    """加载 .env 环境变量文件"""
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


class BuyAndSellStrategyConfig(StrategyConfig, frozen=True):
    """买入卖出策略配置"""
    instrument_id: InstrumentId


class BuyAndSellStrategy(Strategy):
    """
    买入卖出测试策略
    
    功能：
    1. 订阅行情
    2. 收到报价后立即买入100股
    3. 等待3秒后卖出100股
    4. 等待5秒后停止策略
    """

    def __init__(self, config: BuyAndSellStrategyConfig):
        super().__init__(config)
        self.instrument_id = config.instrument_id
        self._has_ordered = False

    def on_start(self):
        """策略启动"""
        self.subscribe_quote_ticks(self.instrument_id)
        self.log.info(f"已订阅 {self.instrument_id}，等待报价以下单...")

    def on_stop(self):
        """策略停止"""
        self.unsubscribe_quote_ticks(self.instrument_id)
        self.log.info(f"已取消订阅 {self.instrument_id}")

    def on_quote_tick(self, tick: QuoteTick):
        """收到报价时的处理"""
        if self._has_ordered:
            return

        # 简单逻辑：以卖一价买入以确保立即成交（跨越买卖价差）
        # 使用限价单以确保安全，价格 = 卖一价
        price = tick.ask_price
        if price.as_double() == 0.0:
             # 价格无效，等待下一个报价
             return

        instrument = self.cache.instrument(self.instrument_id)
        if instrument is None:
            self.log.error(f"无法找到合约信息: {self.instrument_id}")
            return

        # 将价格舍入到合约的最小变动单位以避免精度错误
        # 简化处理：格式化为2位小数（中国股票标准）
        price = Price.from_str(f"{price.as_double():.2f}")

        qty = instrument.make_qty(100)

        # 添加滑点以确保成交（模拟环境）
        raw_price = round(price.as_double() + 0.02, 2)
        price = Price.from_str(f"{raw_price:.2f}")

        self.log.info(f"收到报价: {tick}. 正在提交买入限价单，100股 @ {price}（+0.02滑点）...")

        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(order)
        self._has_ordered = True

    def on_order_filled(self, event):
        """订单成交时的处理"""
        self.log.info(f"订单已成交: {event}")
        self._print_position()

        if event.order_side == OrderSide.BUY:
            self.log.info("买入订单已成交。等待3秒后提交卖出订单...")
            from datetime import timedelta
            self.clock.set_time_alert(
                name="place_sell_order",
                alert_time=self.clock.utc_now() + timedelta(seconds=3.0),
                callback=lambda e: self._place_sell_order(),
            )
        else:
            self.log.info("卖出订单已成交。等待5秒后停止节点...")
            from datetime import timedelta
            self.clock.set_time_alert(
                name="stop_node",
                alert_time=self.clock.utc_now() + timedelta(seconds=5.0),
                callback=lambda e: self.stop(),
            )

    def _place_sell_order(self):
        """提交卖出订单"""
        tick = self.cache.quote_tick(self.instrument_id)
        if tick is None or tick.bid_price.as_double() <= 0:
            self.log.error("无法提交卖出订单：没有报价或买一价无效")
            self.stop()
            return

        instrument = self.cache.instrument(self.instrument_id)
        # 使用买一价减去一些滑点以确保成交
        price_val = round(tick.bid_price.as_double() - 0.02, 2)
        price = Price.from_str(f"{max(price_val, 0.01):.2f}")
        qty = instrument.make_qty(100)

        self.log.info(f"正在提交卖出限价单，100股 @ {price}...")
        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(order)

    def _print_position(self):
        """打印当前持仓信息"""
        try:
            positions = self.cache.positions_open(instrument_id=self.instrument_id)
            if positions:
                for position in positions:
                    self.log.info(f"{self.instrument_id} 的当前持仓: {position}")
                    self.log.info(f"  - 数量: {position.quantity}")
                    self.log.info(f"  - 平均价格: {position.avg_px_open}")
            else:
                self.log.info(f"缓存中未找到 {self.instrument_id} 的持仓")
        except Exception as e:
            self.log.error(f"查询持仓时出错: {e}")

    def on_order_rejected(self, event):
        """订单被拒绝时的处理"""
        self.log.error(f"订单被拒绝: {event}")
        self.stop()

    def on_order_canceled(self, event):
        """订单被撤销时的处理"""
        self.log.warn(f"订单已撤销: {event}")
        self.stop()


# --- 配置参数 ---
_load_dotenv()

miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
session_id = random.randint(100000, 999999)  # 随机会话ID
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
ticker_str = "601808.SSE"

instrument_id = InstrumentId.from_str(ticker_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("BUY-SELL-TESTER-001"),
    logging=LoggingConfig(log_level="INFO"),
    data_clients={
        TT: ThinkTraderDataClientConfig(
            miniqmt_path=miniqmt_path,
            session_id=session_id,
            instrument_provider=instrument_provider,
        ),
    },
    exec_clients={
        TT: ThinkTraderExecClientConfig(
            miniqmt_path=miniqmt_path,
            account_id=account_id,
            account_type=account_type,
            session_id=session_id,
            instrument_provider=instrument_provider,
            routing=RoutingConfig(default=True),
        ),
    },
    data_engine=LiveDataEngineConfig(
        validate_data_sequence=True,
    ),
    timeout_connection=90.0,
    timeout_reconciliation=5.0,
    timeout_portfolio=5.0,
    timeout_disconnection=5.0,
    timeout_post_stop=2.0,
)

if __name__ == "__main__":
    node = TradingNode(config=config_node)

    strategy_config = BuyAndSellStrategyConfig(
        instrument_id=instrument_id,
    )
    strategy = BuyAndSellStrategy(config=strategy_config)

    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    node.build()

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户中断")
    finally:
        node.dispose()
