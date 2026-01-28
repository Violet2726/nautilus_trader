#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may not use this file except in compliance with the License.
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------

import time
from datetime import datetime
from decimal import Decimal

import pandas as pd

from nautilus_trader.backtest.config import BacktestEngineConfig
from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.backtest.models import FillModel
from nautilus_trader.backtest.modules import FXRolloverInterestConfig
from nautilus_trader.backtest.modules import FXRolloverInterestModule
from nautilus_trader.examples.strategies.volatility_market_maker import VolatilityMarketMaker
from nautilus_trader.examples.strategies.volatility_market_maker import VolatilityMarketMakerConfig
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.objects import Money
from nautilus_trader.persistence.wranglers import QuoteTickDataWrangler
from nautilus_trader.test_kit.providers import TestDataProvider
from nautilus_trader.test_kit.providers import TestInstrumentProvider


if __name__ == "__main__":
    # 配置回测引擎
    config = BacktestEngineConfig(
        trader_id=TraderId("BACKTESTER-001"),
    )

    # 构建回测引擎
    engine = BacktestEngine(config=config)

    # 可选的插件模块用于模拟展期利息（rollover interest），
    # 数据来源于封装好的测试数据。
    provider = TestDataProvider()
    interest_rate_data = provider.read_csv("short-term-interest.csv")
    config = FXRolloverInterestConfig(interest_rate_data)
    fx_rollover_interest = FXRolloverInterestModule(config=config)

    # 创建成交模型（可选）
    fill_model = FillModel(
        prob_fill_on_limit=0.2,  # 限价单成交概率
        prob_slippage=0.5,  # 滑点概率
        random_seed=42,  # 随机种子，用于结果可复现
    )

    # 添加交易场所（可以添加多个）
    SIM = Venue("SIM")
    engine.add_venue(
        venue=SIM,
        oms_type=OmsType.NETTING,
        account_type=AccountType.MARGIN,
        base_currency=USD,  # 标准单币种账户
        starting_balances=[Money(10_000_000, USD)],  # 单币种或多币种账户
        fill_model=fill_model,
        modules=[fx_rollover_interest],
        bar_execution=True,  # K 线数据是否驱动市场（默认为 True）
    )

    # 添加交易合约
    GBPUSD_SIM = TestInstrumentProvider.default_fx_ccy("GBP/USD", SIM)
    engine.add_instrument(GBPUSD_SIM)

    # 添加数据
    wrangler = QuoteTickDataWrangler(GBPUSD_SIM)
    ticks = wrangler.process_bar_data(
        bid_data=provider.read_csv_bars("fxcm/gbpusd-m1-bid-2012.csv"),
        ask_data=provider.read_csv_bars("fxcm/gbpusd-m1-ask-2012.csv"),
    )
    engine.add_data(ticks)

    # 配置策略
    strategy_config = VolatilityMarketMakerConfig(
        instrument_id=GBPUSD_SIM.id,
        bar_type=BarType.from_str("GBP/USD.SIM-5-MINUTE-BID-INTERNAL"),
        atr_period=20,  # ATR 周期
        atr_multiple=3.0,  # ATR 倍数
        trade_size=Decimal(500_000),  # 交易量
        emulation_trigger="NO_TRIGGER",  # 不使用仿真触发
    )
    # 实例化并添加策略
    strategy = VolatilityMarketMaker(config=strategy_config)
    engine.add_strategy(strategy=strategy)

    time.sleep(0.1)
    input("按下回车键继续...")

    # 运行引擎（从数据的开始到指定结束时间）
    engine.run(end=datetime(2012, 2, 10))

    # 可选：查看报告
    with pd.option_context(
        "display.max_rows",
        100,
        "display.max_columns",
        None,
        "display.width",
        300,
    ):
        print(engine.trader.generate_account_report(SIM))
        print(engine.trader.generate_order_fills_report())
        print(engine.trader.generate_positions_report())

    # 如需重复运行回测，请确保重置引擎
    engine.reset()

    # 完成后销毁对象是一个好习惯
    engine.dispose()
