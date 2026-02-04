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


import os
from pathlib import Path

from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.examples.strategies.subscribe import SubscribeStrategy
from nautilus_trader.examples.strategies.subscribe import SubscribeStrategyConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId


def _load_dotenv() -> None:
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return


# *** THIS IS A TEST STRATEGY WITH NO ALPHA ADVANTAGE WHATSOEVER. ***
# *** IT IS NOT INTENDED TO BE USED TO TRADE LIVE WITH REAL MONEY. ***


_load_dotenv()

miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
session_id = int(os.environ.get("MINIQMT_SESSION_ID", "123456"))
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"

instrument_id = InstrumentId.from_str(
    os.environ.get("XT_LIVE_INSTRUMENT_ID", "000547.SZSE"),
)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-001"),
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

node = TradingNode(config=config_node)

strategy_config = SubscribeStrategyConfig(
    instrument_id=instrument_id,
    trade_ticks=False,
    quote_ticks=True,
    bars=True,
)
strategy = SubscribeStrategy(config=strategy_config)

node.trader.add_strategy(strategy)

node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
node.build()


if __name__ == "__main__":
    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
