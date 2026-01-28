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
from decimal import Decimal

import pandas as pd

from nautilus_trader.backtest.config import BacktestEngineConfig
from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.backtest.modules import FXRolloverInterestConfig
from nautilus_trader.backtest.modules import FXRolloverInterestModule
from nautilus_trader.examples.strategies.ema_cross import EMACross
from nautilus_trader.examples.strategies.ema_cross import EMACrossConfig
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

    # 添加交易场所（可以添加多个）
    SIM = Venue("SIM")
    engine.add_venue(
        venue=SIM,
        oms_type=OmsType.HEDGING,  # 交易场所将生成持仓 ID
        account_type=AccountType.MARGIN,
        base_currency=USD,  # 标准单币种账户
        starting_balances=[Money(1_000_000, USD)],  # 单币种或多币种账户
        modules=[fx_rollover_interest],
    )

    # 添加交易合约
    AUDUSD_SIM = TestInstrumentProvider.default_fx_ccy("AUD/USD", SIM)
    engine.add_instrument(AUDUSD_SIM)

    # 添加数据
    wrangler = QuoteTickDataWrangler(instrument=AUDUSD_SIM)
    ticks = wrangler.process(provider.read_csv_ticks("truefx/audusd-ticks.csv"))
    engine.add_data(ticks)

    # 配置策略
    strategy_config = EMACrossConfig(
        instrument_id=AUDUSD_SIM.id,
        bar_type=BarType.from_str("AUD/USD.SIM-1-MINUTE-MID-INTERNAL"),
        fast_ema_period=10,
        slow_ema_period=20,
        trade_size=Decimal(1_000_000),
    )
    # 实例化并添加策略
    strategy = EMACross(config=strategy_config)
    engine.add_strategy(strategy=strategy)

    time.sleep(0.1)
    input("按下回车键继续...")

    # 运行引擎（从数据的开始到结束）
    engine.run()

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
