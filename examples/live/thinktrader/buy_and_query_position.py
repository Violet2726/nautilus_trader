#!/usr/bin/env python3
# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
# -------------------------------------------------------------------------------------------------

import os
import random
import warnings
from pathlib import Path


# Suppress annoying warnings
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
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return

    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return

class BuyAndQueryStrategyConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId

class BuyAndQueryStrategy(Strategy):
    def __init__(self, config: BuyAndQueryStrategyConfig):
        super().__init__(config)
        self.instrument_id = config.instrument_id
        self._has_ordered = False

    def on_start(self):
        self.subscribe_quote_ticks(self.instrument_id)
        self.log.info(f"Subscribed to {self.instrument_id}, waiting for quote to place order...")

    def on_stop(self):
        self.unsubscribe_quote_ticks(self.instrument_id)
        self.log.info(f"Unsubscribed from {self.instrument_id}")

    def on_quote_tick(self, tick: QuoteTick):
        if self._has_ordered:
            return

        # Simple logic: Buy at Ask to fill immediately (Crossing the spread)
        # Using a Limit order for safety, price = Ask Price
        price = tick.ask_price
        if price.as_double() == 0.0:
             # Fallback to bid ?? or just waitt
             return

        instrument = self.cache.instrument(self.instrument_id)
        if instrument is None:
            self.log.error(f"Could not find instrument for {self.instrument_id}")
            return

        # Round price to instrument tick size to avoid precision errors
        price = Price.from_str(str(round(price.as_double(), 2))) # HACK: Hardcoded for stocks, or use helper
        # Better: use quantize if available, or just re-create price with correct precision?
        # The instrument might have a tick size of 0.01.
        if instrument.price_precision:
             # This is a bit manual, but safe for this test.
             # Ideally: price = price.round(instrument.info.price_precision)
             pass

        # Simplified: Just formatting string to 2 decimals which is standard for CN stocks
        price = Price.from_str(f"{price.as_double():.2f}")

        qty = instrument.make_qty(100)



        # Add slippage to ensure fill in simulation
        raw_price = round(price.as_double() + 0.02, 2)
        price = Price.from_str(f"{raw_price:.2f}")

        self.log.info(f"Received Quote: {tick}. Placing BUY LIMIT Order for 100 shares at {price} (+0.02 slippage)...")

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
        self.log.info(f"Order Filled: {event}")
        self._print_position()

        if event.order_side == OrderSide.BUY:
            self.log.info("BUY Order Filled. Waiting 3s to place SELL order...")
            from datetime import timedelta
            self.clock.set_time_alert(
                name="place_sell_order",
                alert_time=self.clock.utc_now() + timedelta(seconds=3.0),
                callback=lambda e: self._place_sell_order(),
            )
        else:
            self.log.info("SELL Order Filled. Waiting 5s to stop node...")
            from datetime import timedelta
            self.clock.set_time_alert(
                name="stop_node",
                alert_time=self.clock.utc_now() + timedelta(seconds=5.0),
                callback=lambda e: self.stop(),
            )

    def _place_sell_order(self):
        tick = self.cache.quote_tick(self.instrument_id)
        if tick is None or tick.bid_price.as_double() <= 0:
            self.log.error("Cannot place sell order: No quote or invalid bid price.")
            self.stop()
            return

        instrument = self.cache.instrument(self.instrument_id)
        # Use bid price minus some slippage to ensure fill
        price_val = round(tick.bid_price.as_double() - 0.02, 2)
        price = Price.from_str(f"{max(price_val, 0.01):.2f}")
        qty = instrument.make_qty(100)

        self.log.info(f"Placing SELL LIMIT Order for 100 shares at {price}...")
        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            price=price,
            time_in_force=TimeInForce.DAY,
        )
        self.submit_order(order)

    def _print_position(self):
        # Use cache to find open positions by instrument_id
        try:
            positions = self.cache.positions_open(instrument_id=self.instrument_id)
            if positions:
                for position in positions:
                    self.log.info(f"Current Position for {self.instrument_id}: {position}")
                    self.log.info(f"  - Quantity: {position.quantity}")
                    self.log.info(f"  - Avg Px: {position.avg_px_open}")
            else:
                self.log.info(f"No open positions found in cache for {self.instrument_id}")
        except Exception as e:
            self.log.error(f"Error querying position from cache: {e}")

    def on_order_rejected(self, event):
        self.log.error(f"Order REJECTED: {event}")
        self.stop()

    def on_order_canceled(self, event):
        self.log.warn(f"Order CANCELED: {event}")
        self.stop()

# --- Configuration ---
_load_dotenv()

miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
session_id = random.randint(100000, 999999) # Random Session ID
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
ticker_str = "601808.SSE"

instrument_id = InstrumentId.from_str(ticker_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("BUY-TESTER-001"),
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

    strategy_config = BuyAndQueryStrategyConfig(
        instrument_id=instrument_id,
    )
    strategy = BuyAndQueryStrategy(config=strategy_config)

    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)
    node.build()

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("Stopped by user")
    finally:
        node.dispose()
