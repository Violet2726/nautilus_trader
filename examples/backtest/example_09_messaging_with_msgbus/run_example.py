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

from decimal import Decimal

from strategy import DemoStrategy
from strategy import DemoStrategyConfig

from examples.utils.data_provider import prepare_demo_data_eurusd_futures_1min
from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.config import BacktestEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.model import Bar
from nautilus_trader.model import TraderId
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.instruments.base import Instrument
from nautilus_trader.model.objects import Money


if __name__ == "__main__":
    # ----------------------------------------------------------------------------------
    # 1. 配置并创建回测引擎
    # ----------------------------------------------------------------------------------

    engine_config = BacktestEngineConfig(
        trader_id=TraderId("BACKTEST-EVENTS-001"),  # 此回测的唯一标识符
        logging=LoggingConfig(log_level="INFO"),
    )
    engine = BacktestEngine(config=engine_config)

    # ----------------------------------------------------------------------------------
    # 2. 准备市场数据
    # ----------------------------------------------------------------------------------

    prepared_data: dict = prepare_demo_data_eurusd_futures_1min()
    venue_name: str = prepared_data["venue_name"]
    eurusd_instrument: Instrument = prepared_data["instrument"]
    eurusd_1min_bartype = prepared_data["bar_type"]
    eurusd_1min_bars: list[Bar] = prepared_data["bars_list"]

    # ----------------------------------------------------------------------------------
    # 3. 配置交易环境
    # ----------------------------------------------------------------------------------

    # 设置带保证金账户的交易场所
    engine.add_venue(
        venue=Venue(venue_name),
        oms_type=OmsType.NETTING,  # 使用净额结算订单管理系统
        account_type=AccountType.MARGIN,  # 使用保证金交易账户
        starting_balances=[Money(1_000_000, USD)],  # 设置初始资本
        base_currency=USD,  # 账户币种
        default_leverage=Decimal(1),  # 无杠杆 (1:1)
    )

    # 注册交易合约
    engine.add_instrument(eurusd_instrument)

    # 加载历史市场数据
    engine.add_data(eurusd_1min_bars)

    # ----------------------------------------------------------------------------------
    # 4. 配置并运行策略
    # ----------------------------------------------------------------------------------

    # 创建策略配置
    strategy_config = DemoStrategyConfig(
        instrument=eurusd_instrument,
        bar_type=eurusd_1min_bartype,
    )

    # 创建并注册策略
    strategy = DemoStrategy(config=strategy_config)
    engine.add_strategy(strategy)

    # 执行回测
    engine.run()

    # 清理资源
    engine.dispose()
