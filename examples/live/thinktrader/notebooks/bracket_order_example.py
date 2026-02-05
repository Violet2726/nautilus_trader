import datetime
import os
import random
import sys
from pathlib import Path

import pandas as pd

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
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model import BarType
from nautilus_trader.model import TraderId
from nautilus_trader.model.enums import ContingencyType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.orders import Order
from nautilus_trader.model.orders import OrderList
from nautilus_trader.trading import Strategy
from nautilus_trader.trading.config import StrategyConfig


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


class ThinkTraderBracketConfig(StrategyConfig, frozen=True):
    tradable_instrument_id: str | None = None


class ThinkTraderBracketStrategy(Strategy):
    def __init__(self, config: ThinkTraderBracketConfig) -> None:
        super().__init__(config)
        self.bar_type_m1: dict[InstrumentId, BarType] = {}
        self.tradable_instrument_id = config.tradable_instrument_id

    def on_start(self) -> None:
        self.log.info(f"instrument_id in cache : {self.cache.instrument_ids()}")
        self.log.info(f"instruments in cache : {self.cache.instruments()}")

        for instrument in self.cache.instruments():
            if str(instrument.id) == self.tradable_instrument_id:
                self.bar_type_m1[instrument.id] = BarType.from_str(
                    f"{instrument.id}-1-MINUTE-LAST-EXTERNAL",
                )
                self.log.info(f"subscribing to : {self.bar_type_m1[instrument.id]}")

                self.buy_bracket(instrument.id)

                self.clock.set_time_alert(
                    "sl",
                    self.clock.utc_now() + pd.Timedelta(seconds=10),
                    lambda event, instrument_id=instrument.id: self.modify_sl(
                        instrument_id,
                    ),
                )

    def buy_bracket(self, instrument_id: InstrumentId) -> None:
        instrument = self.cache.instrument(instrument_id)

        if not instrument:
            self.log.error(f"No instrument loaded for instrument id : {instrument_id}")
            sys.exit(1)

        base_price = 10.0
        sl_price = instrument.make_price(base_price - 0.5)
        tp_price = instrument.make_price(base_price + 1.0)

        order_list: OrderList = self.order_factory.bracket(
            instrument_id=instrument_id,
            order_side=OrderSide.BUY,
            quantity=instrument.make_qty(1),
            time_in_force=TimeInForce.GTC,
            entry_post_only=False,
            contingency_type=ContingencyType.OCO,
            sl_trigger_price=sl_price,
            tp_order_type=OrderType.LIMIT,
            tp_price=tp_price,
            tp_post_only=False,
            entry_order_type=OrderType.MARKET,
            emulation_trigger=TriggerType.NO_TRIGGER,
        )
        self.log.info(f"orderlist : {order_list}")

    def modify_sl(self, instrument_id: InstrumentId) -> None:
        instrument = self.cache.instrument(instrument_id)
        if not instrument:
            self.log.error(f"No instrument loaded for instrument id : {instrument_id}")
            sys.exit(1)

        base_price = 10.0
        new_sl_price = instrument.make_price(base_price - 0.2)

        sl_order = self.get_sl_order(instrument_id)
        if sl_order is None:
            self.log.error(f"No SL order found for instrument {instrument_id}")
            return

        self.log.info(f"modifying sl order for {instrument_id} to : {new_sl_price}")
        self.modify_order(sl_order, trigger_price=new_sl_price)

    def get_sl_order(self, instrument_id: InstrumentId) -> Order | None:
        list_orders_for_instrument = self.cache.orders(instrument_id=instrument_id)

        for order in list_orders_for_instrument:
            if order.is_open and order.order_type == OrderType.STOP_MARKET:
                return order

        self.log.error(
            f"Error : sl not found for instrument {instrument_id}\n list of orders found : {list_orders_for_instrument}",
        )
        return None


_load_dotenv()

miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
session_id = random.randint(100000, 999999) # Random Session ID
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = os.environ.get("MINIQMT_ACCOUNT_TYPE", "STOCK")

instrument_id_str = (
    os.environ.get("XT_LIVE_INSTRUMENT_ID", "000547.SZSE")
)
instrument_id = InstrumentId.from_str(instrument_id_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TT-BRACKET-TESTER-001"),
    logging=LoggingConfig(
        log_level="INFO",
        log_level_file="INFO",
        log_file_name=datetime.datetime.strftime(
            datetime.datetime.now(tz=datetime.UTC),
            "%Y-%m-%d_%H-%M",
        )
        + "_tt_bracket_order.log",
        log_directory="./logs/",
        print_config=True,
    ),
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
            routing=RoutingConfig(
                default=True,
            ),
        ),
    },
    data_engine=LiveDataEngineConfig(
        time_bars_timestamp_on_close=False,
        validate_data_sequence=True,
        time_bars_build_with_no_updates=False,
    ),
)

strat_config = ThinkTraderBracketConfig(tradable_instrument_id=instrument_id_str)
strategy = ThinkTraderBracketStrategy(config=strat_config)

node = TradingNode(config=config_node)
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
