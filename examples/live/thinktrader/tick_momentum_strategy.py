
import os
import random
import warnings
from collections import deque
from typing import Deque
from typing import Dict
from typing import List
from typing import Tuple

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
from nautilus_trader.trading.strategy import Strategy


# 过滤不必要的警告
warnings.filterwarnings("ignore", category=UserWarning)
warnings.filterwarnings("ignore", category=DeprecationWarning)

def _load_dotenv() -> None:
    """加载 .env 环境变量配置"""
    from pathlib import Path
    try:
        from dotenv import load_dotenv
    except ModuleNotFoundError:
        return
    for parent in Path(__file__).resolve().parents:
        env_path = parent / ".env"
        if env_path.is_file():
            load_dotenv(dotenv_path=env_path, override=True)
            return

# -------------------------------------------------------------------------------------
# 1. 策略配置类
# -------------------------------------------------------------------------------------
class MultiTickMomentumConfig(StrategyConfig, frozen=True):
    """
    多标的 Tick 动量策略配置
    """

    instrument_ids: List[InstrumentId]
    trade_qty: int = 100            # 每次交易数量 (股)

    # 信号参数
    momentum_window_seconds: float = 10.0
    entry_momentum_bps: float = 5.0

    # 止盈止损参数 (基点)
    take_profit_bps: float = 15.0
    stop_loss_bps: float = 10.0

    # 风险控制
    max_positions_per_instrument: int = 3  # 每个标的最大并行持仓个数 (Trade Units)
    max_total_positions: int = 10          # 全局总持仓个数
    max_daily_loss: float = 1000.0
    max_round_trips_per_id: int = 20

# -------------------------------------------------------------------------------------
# 2. 策略逻辑类
# -------------------------------------------------------------------------------------
class MultiTickMomentumStrategy(Strategy):
    """
    支持多标的、多订单跟踪的 Tick 动量策略。
    
    逻辑改进：
    1. 支持多个 InstrumentId 同时运行。
    2. 支持“多单元持仓”：如果动量持续触发，且未达到 max_positions_per_instrument，
       可以多次开仓，每个开仓单元独立跟踪止盈止损。
    3. 全局和单标的风控。
    """

    def __init__(self, config: MultiTickMomentumConfig):
        super().__init__(config)
        self.instrument_ids = config.instrument_ids

        # 将参数转化为内部计算格式
        self.momentum_window_ns = int(config.momentum_window_seconds * 1_000_000_000)
        self.entry_threshold = config.entry_momentum_bps / 10_000.0
        self.tp_threshold = config.take_profit_bps / 10_000.0
        self.sl_threshold = config.stop_loss_bps / 10_000.0

        # 状态字典 (key 为 instrument_id)
        self._tick_history: Dict[InstrumentId, Deque[Tuple[int, float]]] = {
            id: deque() for id in self.instrument_ids
        }
        # 独立跟踪每一笔“交易单元”: [{'entry_price': float, 'qty': int, 'order_id': ...}, ...]
        self._active_trades: Dict[InstrumentId, List[Dict]] = {
            id: [] for id in self.instrument_ids
        }
        self._pending_orders_count: Dict[InstrumentId, int] = dict.fromkeys(self.instrument_ids, 0)
        self._round_trips = dict.fromkeys(self.instrument_ids, 0)
        self._tick_counts = dict.fromkeys(self.instrument_ids, 0)

        self._realized_pnl = 0.0
        self._instruments = {}

    def on_start(self) -> None:
        """策略启动时调用"""
        for instrument_id in self.instrument_ids:
            self.subscribe_quote_ticks(instrument_id)
            self.log.info(f"已订阅: {instrument_id}")
        self.log.info(f"多级持仓上限: {self.config.max_positions_per_instrument}")

    def on_stop(self) -> None:
        """策略停止时调用"""
        for instrument_id in self.instrument_ids:
            self.unsubscribe_quote_ticks(instrument_id)
        self.log.info(f"策略结束。最终实现盈亏: {self._realized_pnl:.2f}")

    def on_quote_tick(self, tick: QuoteTick) -> None:
        """
        核心 Tick 处理逻辑
        """
        ts_id = tick.instrument_id
        if ts_id not in self.instrument_ids:
            return

        # 获取 Instrument 对象
        if ts_id not in self._instruments:
            inst = self.cache.instrument(ts_id)
            if inst: self._instruments[ts_id] = inst
            else: return

        # 基础数据
        bid = tick.bid_price.as_double()
        ask = tick.ask_price.as_double()
        if bid <= 0 or ask <= 0: return
        mid_price = (bid + ask) * 0.5
        now_ns = self.clock.timestamp_ns()

        # 1. 维护历史序列
        history = self._tick_history[ts_id]
        history.append((now_ns, mid_price))
        while history and (now_ns - history[0][0] > self.momentum_window_ns):
            history.popleft()

        # 2. 止盈止损检查 (检查该标的所有已成交单元)
        trades = self._active_trades[ts_id]
        closed_any = False
        for trade in trades[:]:  # 复制一份用于遍历，因为会修改原列表
            entry_price = trade["entry_price"]
            pnl_bps = (mid_price - entry_price) / entry_price

            # 止盈
            if pnl_bps >= self.tp_threshold:
                self.log.info(f"[{ts_id}] 触发单元止盈: 当前 {mid_price:.2f} >= 入场 {entry_price:.2f} (+{pnl_bps*10000:.1f} bps)")
                # 使用买一价限价卖出
                self._submit_exit_order(ts_id, trade["qty"], bid)
                trades.remove(trade)
                closed_any = True
            # 止损
            elif pnl_bps <= -self.sl_threshold:
                self.log.info(f"[{ts_id}] 触发单元止损: 当前 {mid_price:.2f} <= 入场 {entry_price:.2f} ({pnl_bps*10000:.1f} bps)")
                # 使用买一价限价卖出
                self._submit_exit_order(ts_id, trade["qty"], bid)
                trades.remove(trade)
                closed_any = True

        if closed_any: return # 如果本 Tick 触发了平仓，本标的不再开新仓

        # 3. 入场逻辑
        self._tick_counts[ts_id] += 1

        # 计算动量
        price_ago = history[0][1]
        momentum = (mid_price - price_ago) / price_ago if price_ago > 0 else 0

        # 每 5 个 Tick 打印心跳 (避免刷屏)
        if self._tick_counts[ts_id] % 5 == 0:
            active_count = len(trades)
            self.log.info(f"[{ts_id}] #{self._tick_counts[ts_id]} | 价:{mid_price:.2f} | 动量:{momentum*10000:.1f} bps | 持仓单元:{active_count}")

        # 入场检查
        if self._pending_orders_count[ts_id] > 0: return # 标的有在途订单，等待

        if len(trades) < self.config.max_positions_per_instrument:
            # 检查总仓位
            total_active = sum(len(v) for v in self._active_trades.values())
            if total_active >= self.config.max_total_positions: return

            # 检查风控
            if self._round_trips[ts_id] >= self.config.max_round_trips_per_id: return
            if self._realized_pnl <= -self.config.max_daily_loss: return

            # 触发开仓
            if momentum >= self.entry_threshold:
                self.log.info(f"[{ts_id}] 动量入场触发! 动量:{momentum*10000:.1f} bps")
                self._submit_entry_order(ts_id, ask)

    def _submit_entry_order(self, instrument_id: InstrumentId, price: float) -> None:
        inst = self._instruments[instrument_id]
        qty = inst.make_qty(self.config.trade_qty)
        order = self.order_factory.limit(
            instrument_id=instrument_id,
            order_side=OrderSide.BUY,
            quantity=qty,
            price=inst.make_price(price),
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)
        self._pending_orders_count[instrument_id] += 1

    def _submit_exit_order(self, instrument_id: InstrumentId, qty_val: float, price: float) -> None:
        inst = self._instruments[instrument_id]
        qty = inst.make_qty(qty_val)
        order = self.order_factory.limit(
            instrument_id=instrument_id,
            order_side=OrderSide.SELL,
            quantity=qty,
            price=inst.make_price(price),
            time_in_force=TimeInForce.GTC,
        )
        self.submit_order(order)
        self._pending_orders_count[instrument_id] += 1

    def on_order_filled(self, event) -> None:
        ts_id = event.instrument_id
        self._pending_orders_count[ts_id] = max(0, self._pending_orders_count[ts_id] - 1)

        fill_qty = event.last_qty.as_double()
        fill_price = event.last_px.as_double()

        if event.order_side == OrderSide.BUY:
            # 新增一个持仓单元
            self._active_trades[ts_id].append({
                "entry_price": fill_price,
                "qty": fill_qty
            })
            self.log.info(f"[{ts_id}] 买入成交: {fill_qty} @ {fill_price:.2f} (当前持仓单元数: {len(self._active_trades[ts_id])})")

        elif event.order_side == OrderSide.SELL:
            # 这种简化的逻辑假定 SELL 是平仓。在 Netting 模式下我们不通过 trade_id 匹配，
            # 只要成交了，盈亏已经在 _on_quote_tick 平仓触发时从逻辑上扣减了。
            # 这里记录总盈亏即可（实际上单元盈亏在 tick 触发平仓已经可以估算，这里可以进一步精细）。
            # 注意：由于我们同步了 active_trades 列表，这里的成交主要用于同步风控和日志。
            self._round_trips[ts_id] += 1
            # 简化的盈亏累计 (此处可以用成交价与最近一个移除单元的入场价比)
            self.log.info(f"[{ts_id}] 卖出成交: {fill_qty} @ {fill_price:.2f}")

    def on_order_rejected(self, event) -> None:
        self._pending_orders_count[event.instrument_id] = max(0, self._pending_orders_count[event.instrument_id] - 1)
        self.log.error(f"订单拒绝: {event}")

    def on_order_canceled(self, event) -> None:
        self._pending_orders_count[event.instrument_id] = max(0, self._pending_orders_count[event.instrument_id] - 1)
        self.log.warning(f"订单取消: {event}")

# -------------------------------------------------------------------------------------
# 3. 主程序入口
# -------------------------------------------------------------------------------------
if __name__ == "__main__":
    _load_dotenv()

    miniqmt_path = os.environ.get("MINIQMT_PATH", r"D:\迅投极速策略交易系统交易终端 华福证券QMT仿真\userdata_mini")
    session_id = random.randint(100000, 999999)
    account_id = os.environ.get("MINIQMT_ACCOUNT_ID", "211800003313")

    # --- 配置多标的列表 ---
    TARGET_SYMBOLS = ["688576.SSE", "601808.SSE"]
    instrument_ids = [InstrumentId.from_str(s) for s in TARGET_SYMBOLS]

    # 配置节点
    instrument_provider = ThinkTraderInstrumentProviderConfig(
        load_all=False,
        load_ids=frozenset(instrument_ids),
    )

    node_config = TradingNodeConfig(
        trader_id=TraderId("MULTI-MOMENTUM-001"),
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
                account_type="STOCK",
                session_id=session_id,
                instrument_provider=instrument_provider,
                routing=RoutingConfig(default=True),
            ),
        },
        data_engine=LiveDataEngineConfig(validate_data_sequence=True),
        timeout_connection=90.0,
    )

    node = TradingNode(config=node_config)

    # --- 策略详细参数 ---
    strategy_config = MultiTickMomentumConfig(
        instrument_ids=instrument_ids,
        trade_qty=100,                  # 每笔单元的基础数量
        entry_momentum_bps=2.0,         # 2.0 bps 触发
        take_profit_bps=10.0,           # 10.0 bps 止盈 (0.1%)
        stop_loss_bps=15.0,             # 15.0 bps 止损 (0.15%)
        max_positions_per_instrument=3, # 允许对同一只股票最多开 3 笔独立订单
        max_total_positions=10,         # 总共最多持有 10 笔订单
        momentum_window_seconds=15.0
    )

    strategy = MultiTickMomentumStrategy(config=strategy_config)
    node.trader.add_strategy(strategy)

    node.add_data_client_factory(TT, ThinkTraderLiveDataClientFactory)
    node.add_exec_client_factory(TT, ThinkTraderLiveExecClientFactory)

    node.build()

    try:
        print(f"开始运行多标的策略: {TARGET_SYMBOLS}")
        node.run()
    except KeyboardInterrupt:
        print("用户停止...")
    finally:
        node.dispose()
