
import datetime
import os
import random
import warnings
from decimal import Decimal
from pathlib import Path

# Suppress annoying warnings from dependencies
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

import pandas as pd
from nautilus_trader.adapters.thinktrader.common import TT
from nautilus_trader.adapters.thinktrader.config import ThinkTraderDataClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderExecClientConfig
from nautilus_trader.adapters.thinktrader.config import ThinkTraderInstrumentProviderConfig
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveDataClientFactory
from nautilus_trader.adapters.thinktrader.factories import ThinkTraderLiveExecClientFactory
from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.config import LoggingConfig
from nautilus_trader.config import NonNegativeFloat
from nautilus_trader.config import PositiveFloat
from nautilus_trader.config import RoutingConfig
from nautilus_trader.config import StrategyConfig
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.book import OrderBook
from nautilus_trader.model.data import OrderBookDeltas
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import BookType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import book_type_from_str
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
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


_load_dotenv()

# --- 修改后的策略类 ---

class OrderBookImbalanceConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    max_trade_size: Decimal
    trigger_min_size: PositiveFloat = 100.0
    trigger_imbalance_ratio: PositiveFloat = 0.20
    min_seconds_between_triggers: NonNegativeFloat = 1.0
    book_type: str = "L2_MBP"
    use_quote_ticks: bool = False
    dry_run: bool = False


class OrderBookImbalance(Strategy):
    """
    修改版 OrderBookImbalance 策略。
    当 use_quote_ticks=True 时，直接使用 QuoteTick 触发，而不强依赖 Cache 中的 OrderBook 对象。
    """

    def __init__(self, config: OrderBookImbalanceConfig) -> None:
        assert 0 < config.trigger_imbalance_ratio < 1
        super().__init__(config)
        self.instrument: Instrument | None = None
        if self.config.use_quote_ticks:
            assert self.config.book_type == "L1_MBP"
        self.book_type: BookType = book_type_from_str(self.config.book_type)
        self._last_trigger_timestamp: datetime.datetime | None = None
        # 用于保存最新的 QuoteTick
        self._last_tick: QuoteTick | None = None

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.config.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.config.instrument_id}")
            self.stop()
            return

        if self.config.use_quote_ticks:
            self.book_type = BookType.L1_MBP
            self.subscribe_quote_ticks(self.instrument.id)
        else:
            self.book_type = book_type_from_str(self.config.book_type)
            self.subscribe_order_book_deltas(self.instrument.id, self.book_type)

        self._last_trigger_timestamp = None

    def on_order_book_deltas(self, deltas: OrderBookDeltas) -> None:
        # 传统模式：使用 Book 触发
        self.check_trigger_from_book()

    def on_order_book(self, order_book: OrderBook) -> None:
        # 传统模式：使用 Book 触发
        self.check_trigger_from_book()

    def on_quote_tick(self, tick: QuoteTick) -> None:
        # 新模式：直接使用 Tick 触发
        self._last_tick = tick
        self.check_trigger_from_tick(tick)

    def check_trigger_from_book(self) -> None:
        """从缓存的 OrderBook 检查触发条件"""
        if not self.instrument:
            return
        book = self.cache.order_book(self.config.instrument_id)
        if not book:
            # 只有在非 QuoteTick 模式下才报错
            if not self.config.use_quote_ticks:
                self.log.error("No book being maintained")
            return
        
        if not book.spread():
            return
            
        self._process_trigger(book.best_bid_size(), book.best_ask_size(), book.best_bid_price(), book.best_ask_price())

    def check_trigger_from_tick(self, tick: QuoteTick) -> None:
        """直接从 QuoteTick 检查触发条件"""
        if not self.instrument:
            return
        
        # QuoteTick 可能也是空的或者单边的
        bid_size = tick.bid_size
        ask_size = tick.ask_size
        bid_price = tick.bid_price
        ask_price = tick.ask_price
        
        self._process_trigger(bid_size, ask_size, bid_price, ask_price)

    def _process_trigger(self, bid_size, ask_size, bid_price, ask_price) -> None:
        """统一的触发逻辑处理"""
        if (bid_size is None or bid_size <= 0) or (ask_size is None or ask_size <= 0):
            # self.log.warning("No market yet") # 减少日志噪音
            return

        smaller = min(bid_size, ask_size)
        larger = max(bid_size, ask_size)
        
        # 避免除以零
        if larger == 0:
            return
            
        ratio = smaller / larger
        
        # Log 太多会刷屏，可以适当减少
        # self.log.info(f"Market: {bid_price} @ {ask_price} ({ratio=:0.2f})")

        if self._last_trigger_timestamp is not None:
            seconds_since_last_trigger = (
                self.clock.utc_now() - self._last_trigger_timestamp
            ).total_seconds()
        else:
            seconds_since_last_trigger = float("inf")

        if larger > self.config.trigger_min_size and ratio < self.config.trigger_imbalance_ratio:
            self.log.info(
                f"Imbalance Triggered! Ratio: {ratio:.2f}, Bid: {bid_size}@{bid_price}, Ask: {ask_size}@{ask_price}"
            )
            
            if len(self.cache.orders_inflight(strategy_id=self.id)) > 0:
                self.log.info("Already have orders in flight - skipping.")
            elif seconds_since_last_trigger < self.config.min_seconds_between_triggers:
                self.log.info("Time since last order < min_seconds_between_triggers - skipping")
            elif bid_size > ask_size:
                # 买方力量大 -> 此时应该买还是卖？
                # 原策略逻辑：bid_size > ask_size (买单多)，则买入？
                # 原策略代码：
                # if bid_size > ask_size: ... order_side=OrderSide.BUY ... price=book.best_ask_price()
                # 这种逻辑是：买单堆积，可能会推高价格，所以吃掉卖单（Taker Buy）
                
                # Round down trade quantity to nearest 100
                trade_qty = min(ask_size, Quantity.from_str(str(self.config.max_trade_size)))
                trade_qty = (int(trade_qty) // 100) * 100
                if trade_qty < 100:
                    self.log.info(f"Trade quantity {trade_qty} < 100, skipping")
                    return

                order = self.order_factory.limit(
                    instrument_id=self.instrument.id,
                    price=self.instrument.make_price(ask_price),
                    order_side=OrderSide.BUY,
                    quantity=self.instrument.make_qty(trade_qty),
                    post_only=False,
                    time_in_force=TimeInForce.FOK,
                )
                self._last_trigger_timestamp = self.clock.utc_now()
                self.log.info(f"Hitting ASK! {order=}", color=LogColor.BLUE)
                if self.config.dry_run:
                    self.log.warning("Dry run mode - skipping")
                    return
                self.submit_order(order)
            else:
                # 卖方力量大 -> 卖出
                # Round down trade quantity to nearest 100
                trade_qty = min(bid_size, Quantity.from_str(str(self.config.max_trade_size)))
                trade_qty = (int(trade_qty) // 100) * 100
                if trade_qty < 100:
                    self.log.info(f"Trade quantity {trade_qty} < 100, skipping")
                    return

                order = self.order_factory.limit(
                    instrument_id=self.instrument.id,
                    price=self.instrument.make_price(bid_price),
                    order_side=OrderSide.SELL,
                    quantity=self.instrument.make_qty(trade_qty),
                    post_only=False,
                    time_in_force=TimeInForce.FOK,
                )
                self._last_trigger_timestamp = self.clock.utc_now()
                self.log.info(f"Hitting BID! {order=}", color=LogColor.BLUE)
                if self.config.dry_run:
                    self.log.warning("Dry run mode - skipping")
                    return
                self.submit_order(order)

    def on_reset(self) -> None:
        self._last_trigger_timestamp = None

    def on_stop(self) -> None:
        if self.instrument is None:
            return
        self.cancel_all_orders(self.instrument.id)
        self.close_all_positions(self.instrument.id)


# --- 配置部分 ---

# MiniQMT 路径与账户配置
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
session_id = random.randint(100000, 999999)

instrument_ids_str = ["600916.SSE"] # "000547.SZSE", 
instrument_ids = [InstrumentId.from_str(i) for i in instrument_ids_str]

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset(instrument_ids_str),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-IMB-001"),
    logging=LoggingConfig(
        log_level="INFO",
        log_component_levels={"Strategy": "INFO"} 
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

# --- 启动流程 ---

if __name__ == "__main__":
    node = TradingNode(config=config_node)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    for instrument_id in instrument_ids:
        strategy_config = OrderBookImbalanceConfig(
            instrument_id=instrument_id,
            max_trade_size=Decimal("100"),    
            trigger_min_size=100.0,          
            trigger_imbalance_ratio=0.3,      
            min_seconds_between_triggers=5.0, 
            book_type="L1_MBP",               
            use_quote_ticks=True,             
            dry_run=False,                   
        )
        
        strategy = OrderBookImbalance(config=strategy_config)
        node.trader.add_strategy(strategy)
        print(f"已添加策略: {instrument_id} (Imbalance)")

    node.build()

    print(f"正在启动多标的策略...")
    print(f"数据路径: {miniqmt_path}")
    print(f"账户 ID: {account_id}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
