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

import pandas as pd
from strategy import DemoStrategy

from nautilus_trader.backtest.engine import BacktestEngine
from nautilus_trader.config import BacktestEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.model import TraderId
from nautilus_trader.model.currencies import USD
from nautilus_trader.model.data import Bar
from nautilus_trader.model.data import BarType
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.objects import Money
from nautilus_trader.persistence.wranglers import BarDataWrangler
from nautilus_trader.test_kit.providers import TestInstrumentProvider


if __name__ == "__main__":
    # 步骤 1: 配置并创建回测引擎
    engine_config = BacktestEngineConfig(
        trader_id=TraderId("BACKTEST_TRADER-001"),
        logging=LoggingConfig(
            log_level="DEBUG",  # 将控制台日志级别设置为 DEBUG 以在日志中查看加载的 K 线数据
        ),
    )
    engine = BacktestEngine(config=engine_config)

    # 步骤 2: 定义交易所并将其添加到引擎
    XCME = Venue("XCME")
    engine.add_venue(
        venue=XCME,
        oms_type=OmsType.NETTING,  # 订单管理系统类型 (净额结算)
        account_type=AccountType.MARGIN,  # 交易账户类型 (保证金账户)
        starting_balances=[Money(1_000_000, USD)],  # 初始账户余额
        base_currency=USD,  # 账户基础货币
        default_leverage=Decimal(1),  # 账户不使用杠杆
    )

    # 步骤 3: 创建交易品种定义并将其添加到引擎
    EURUSD_FUTURES_INSTRUMENT = TestInstrumentProvider.eurusd_future(
        expiry_year=2024,
        expiry_month=3,
        venue_name="XCME",
    )
    engine.add_instrument(EURUSD_FUTURES_INSTRUMENT)

    # ==========================================================================================
    # 重点关注：从 CSV 加载 K 线数据
    # ------------------------------------------------------------------------------------------

    # 步骤 4a: 从 CSV 文件加载 K 线数据 -> 进入 pandas DataFrame
    from pathlib import Path
    csv_file_path = Path(__file__).parent / "6EH4.XCME_1min_bars.csv"
    df = pd.read_csv(csv_file_path, sep=";", decimal=".", header=0, index_col=False)

    # 步骤 4b: 重构 DataFrame 为所需结构，以便传递给 `BarDataWrangler`
    #   - 5 列：'open', 'high', 'low', 'close', 'volume'（成交量是可选的）
    #   - 'timestamp' 作为索引

    # 更改列顺序
    df = df.reindex(columns=["timestamp_utc", "open", "high", "low", "close", "volume"])
    # 将字符串时间戳转换为 datetime 对象
    df["timestamp_utc"] = pd.to_datetime(df["timestamp_utc"], format="%Y-%m-%d %H:%M:%S")
    # 将列重命名为所需名称
    df = df.rename(columns={"timestamp_utc": "timestamp"})
    # 设置 `timestamp` 列为索引
    df = df.set_index("timestamp")

    # 步骤 4c: 定义加载的 K 线类型
    EURUSD_FUTURES_1MIN_BARTYPE = BarType.from_str(
        f"{EURUSD_FUTURES_INSTRUMENT.id}-1-MINUTE-LAST-EXTERNAL",
    )

    # 步骤 4d: `BarDataWrangler` 将每一行转换为 `Bar` 类型的对象
    wrangler = BarDataWrangler(EURUSD_FUTURES_1MIN_BARTYPE, EURUSD_FUTURES_INSTRUMENT)
    eurusd_1min_bars_list: list[Bar] = wrangler.process(df)

    # 步骤 4e: 将加载的数据添加到引擎
    engine.add_data(eurusd_1min_bars_list)

    # ------------------------------------------------------------------------------------------
    # 结束关注点
    # ==========================================================================================

    # 步骤 5: 创建策略并将其添加到引擎
    strategy = DemoStrategy(primary_bar_type=EURUSD_FUTURES_1MIN_BARTYPE)
    engine.add_strategy(strategy)

    # 步骤 6: 运行引擎 = 运行回测
    engine.run()

    # 步骤 7: 释放系统资源
    engine.dispose()
