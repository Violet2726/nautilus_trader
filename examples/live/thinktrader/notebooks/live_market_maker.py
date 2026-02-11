
import os
import random
import warnings
from decimal import Decimal
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
from nautilus_trader.core.message import Event
from nautilus_trader.live.node import TradingNode
from nautilus_trader.model.data import QuoteTick
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import PositionSide
from nautilus_trader.model.events import PositionChanged
from nautilus_trader.model.events import PositionClosed
from nautilus_trader.model.events import PositionOpened
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Price
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

# --- 策略部分 (适配 ThinkTrader/QuoteTick) ---

class MarketMakerConfig(StrategyConfig, frozen=True):
    instrument_id: InstrumentId
    trade_size: Decimal = Decimal("100")
    max_size: Decimal = Decimal("1000")
    spread_pct: Decimal = Decimal("0.0") # 设为 0，紧贴买一/卖一价
    update_threshold_pct: Decimal = Decimal("0.0001") # 阈值，只要有微小变动就更新订单


class MarketMaker(Strategy):
    """
    MarketMaker 策略的适配版本。
    1. 支持 QuoteTick (因为 ThinkTrader 提供的是 QuoteTick 而非 OrderBookDeltas)。
    2. 确保下单数量为 100 的整数倍。
    3. 实现了基于库存的中间价调整 (Inventory skew)。
    """

    def __init__(self, config: MarketMakerConfig) -> None:
        super().__init__(config)

        self.instrument_id = config.instrument_id
        self.trade_size = config.trade_size
        self.max_size = config.max_size
        self.spread_pct = config.spread_pct
        self.update_threshold_pct = config.update_threshold_pct

        self.instrument: Instrument | None = None
        self._mid: Decimal | None = None
        self._adj = Decimal(0)

    def on_start(self) -> None:
        self.instrument = self.cache.instrument(self.instrument_id)
        if self.instrument is None:
            self.log.error(f"Could not find instrument for {self.instrument_id}")
            self.stop()
            return

        # 订阅 QuoteTick
        self.subscribe_quote_ticks(self.instrument_id)

    def on_quote_tick(self, tick: QuoteTick) -> None:
        """
        处理报价更新
        """
        bid_price = tick.bid_price
        ask_price = tick.ask_price
        
        if not bid_price or not ask_price:
            return

        # 计算中间价
        mid = (bid_price + ask_price) / 2
        mid_dec = Decimal(mid)

        # 只有当价格变动超过阈值时才调整订单
        should_update = False
        if self._mid is None:
            should_update = True
        else:
            # 计算变化百分比: abs(new_mid - old_mid) / old_mid
            pct_change = abs(mid_dec - self._mid) / self._mid
            if pct_change > self.update_threshold_pct:
                should_update = True
        
        if should_update:
            if self._mid is not None:
                self.log.info(f"价格变动显著 (Old Mid: {self._mid}, New Mid: {mid_dec}), 重新挂单...")
            
            # 撤销所有旧订单
            self.cancel_all_orders(self.instrument_id)
            
            self._mid = mid_dec
            
            # 库存调整后的基准价
            val = self._mid + self._adj
            
            # 计算买卖价格 (做市商模式：低买高卖)
            # Buy @ Val * (1 - spread)
            # Sell @ Val * (1 + spread)
            buy_price = val * (Decimal("1") - self.spread_pct)
            sell_price = val * (Decimal("1") + self.spread_pct)
            
            # 下单
            self.place_order(OrderSide.BUY, buy_price)
            self.place_order(OrderSide.SELL, sell_price)

    def on_event(self, event: Event) -> None:
        """
        根据仓位变化调整价格偏移 (_adj)
        """
        if isinstance(event, PositionOpened | PositionChanged):
            signed_qty = event.quantity.as_decimal()
            if event.side == PositionSide.SHORT:
                signed_qty = -signed_qty
            # 库存越多，_adj 越小(甚至为负)，买卖价下移，倾向于卖出
            # 库存越少(负库存)，_adj 越大，买卖价上移，倾向于买入
            self._adj = (signed_qty / self.max_size) * Decimal("0.01") * -1 
            # 注意：原版策略逻辑似乎是正向的？如果是正向 (signed_qty > 0 implies higher price)，
            # 那意味着持有越多越想要更高的价格？还是说是为了追涨？
            # 传统的Inventory Skew应该是：持有库存多 -> 降价卖出 -> Skew negative.
            # 原版代码: self._adj = (signed_qty / self.max_size) * Decimal("0.01")
            # 假设 Max=1000, Qty=500 -> Adj = 0.5 * 0.01 = 0.005. Price increases.
            # 价格升高意味着更容易卖出(Sell Limit更高?) 不，Sell Limit更高意味着更难卖出。
            # Wait, Sell Price = Mid + Adj + Spread.
            # 如果 Adj > 0 (持有库存)，Sell Price 提高 -> 更难卖出？这反了。
            # 通常：Inventory > 0 -> Quote Lower to attract Buyers and discourage Sellers.
            # 所以应该是 负相关。
            # 我这里加上 * -1 来符合通用做市逻辑。
            
        elif isinstance(event, PositionClosed):
            self._adj = Decimal(0)

    def place_order(self, side: OrderSide, price: Decimal) -> None:
        if not self.instrument:
            return

        # 数量处理：取整到100
        qty = self.trade_size
        qty = (int(qty) // 100) * 100
        
        if qty < 100:
            return

        # 价格处理：符合TickSize
        price_obj = self.instrument.make_price(price)
        
        # 数量对象
        qty_obj = self.instrument.make_qty(qty)

        order = self.order_factory.limit(
            instrument_id=self.instrument_id,
            order_side=side,
            price=price_obj,
            quantity=qty_obj,
        )
        self.submit_order(order)


# --- 配置部分 ---

# MiniQMT 路径与账户配置
miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")
account_type = "STOCK"
session_id = random.randint(100000, 999999)

# 标的: 000547.SZSE
instrument_id_str = "000547.SZSE"
instrument_id = InstrumentId.from_str(instrument_id_str)

instrument_provider = ThinkTraderInstrumentProviderConfig(
    load_all=False,
    load_ids=frozenset([instrument_id_str]),
)

config_node = TradingNodeConfig(
    trader_id=TraderId("TESTER-MM-002"),
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

    strategy_config = MarketMakerConfig(
        instrument_id=instrument_id,
        trade_size=Decimal("100"),
        max_size=Decimal("1000"),
        spread_pct=Decimal("0.005") # 0.5% Spread
    )
    
    strategy = MarketMaker(config=strategy_config)
    node.trader.add_strategy(strategy)

    node.build()

    print(f"正在启动 MarketMaker 策略: {instrument_id}...")
    print(f"数据路径: {miniqmt_path}")

    try:
        node.run()
    except KeyboardInterrupt:
        node.kernel.logger.info("用户停止运行 (Ctrl+C)")
    finally:
        node.dispose()
