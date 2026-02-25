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

from typing import Callable

from libc.stdint cimport uint64_t

from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.core.rust.model cimport LiquiditySide
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport OrderType
from nautilus_trader.core.rust.model cimport PriceRaw
from nautilus_trader.model.functions cimport order_type_to_str
from nautilus_trader.model.identifiers cimport ClientOrderId
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.orders.base cimport Order


cdef class MatchingCore:
    """
    提供一个通用的订单撮合核心。

    Parameters
    ----------
    instrument_id : InstrumentId
        撮合核心的合约 ID。
    price_increment : Price
        撮合核心的最小价格增量（Tick 大格）。
    trigger_stop_order : Callable[[Order], None]
        止损单触发时的回调函数。
    fill_market_order : Callable[[Order], None]
        市价单成交时的回调函数。
    fill_limit_order : Callable[[Order], None]
        限价单成交时的回调函数。
    """

    def __init__(
        self,
        InstrumentId instrument_id not None,
        Price price_increment not None,
        trigger_stop_order not None: Callable,
        fill_market_order not None: Callable,
        fill_limit_order not None: Callable,
    ):
        self._instrument_id = instrument_id
        self._price_increment = price_increment
        self._price_precision = price_increment.precision

        # 市场
        self.bid_raw = 0
        self.ask_raw = 0
        self.last_raw = 0
        self.is_bid_initialized = False
        self.is_ask_initialized = False
        self.is_last_initialized = False

        # 事件处理器
        self._trigger_stop_order = trigger_stop_order
        self._fill_market_order = fill_market_order
        self._fill_limit_order = fill_limit_order

        # 订单
        self._orders: dict[ClientOrderId, Order] = {}
        self._orders_bid: list[Order] = []
        self._orders_ask: list[Order] = []

    @property
    def instrument_id(self) -> InstrumentId:
        """
        返回撮合核心的合约 ID。

        Returns
        -------
        InstrumentId

        """
        return self._instrument_id

    @property
    def price_precision(self) -> int:
        """
        返回撮合核心的工具价格精度。

        Returns
        -------
        int

        """
        return self._price_increment.precision

    @property
    def price_increment(self) -> Price:
        """
        返回撮合核心的工具最小价格增量（Tick 大小）。

        Returns
        -------
        Price

        """
        return self._price_increment

    @property
    def bid(self) -> Price | None:
        """
        返回撮合核心当前的买入价。

        Returns
        -------
        Price 或 ``None``

        """
        if not self.is_bid_initialized:
            return None
        else:
            return Price.from_raw_c(self.bid_raw, self._price_precision)

    @property
    def ask(self) -> Price | None:
        """
        返回撮合核心当前的卖出价。

        Returns
        -------
        Price 或 ``None``

        """
        if not self.is_ask_initialized:
            return None
        else:
            return Price.from_raw_c(self.ask_raw, self._price_precision)

    @property
    def last(self) -> Price | None:
        """
        返回撮合核心当前的最新价。

        Returns
        -------
        Price 或 ``None``

        """
        if not self.is_last_initialized:
            return None
        else:
            return Price.from_raw_c(self.last_raw, self._price_precision)

# -- 查询 --------------------------------------------------------------------------------------

    cpdef Order get_order(self, ClientOrderId client_order_id):
        Condition.not_none(client_order_id, "client_order_id")
        return self._orders.get(client_order_id)

    cpdef bint order_exists(self, ClientOrderId client_order_id):
        Condition.not_none(client_order_id, "client_order_id")
        return client_order_id in self._orders

    cpdef list get_orders(self):
        return self._orders_bid + self._orders_ask

    cpdef list get_orders_bid(self):
        return self._orders_bid

    cpdef list get_orders_ask(self):
        return self._orders_ask

# -- 命令 -------------------------------------------------------------------------------------

    cdef void set_bid_raw(self, PriceRaw bid_raw):
        self.is_bid_initialized = True
        self.bid_raw = bid_raw

    cdef void set_ask_raw(self, PriceRaw ask_raw):
        self.is_ask_initialized = True
        self.ask_raw = ask_raw

    cdef void set_last_raw(self, PriceRaw last_raw):
        self.is_last_initialized = True
        self.last_raw = last_raw

    cpdef void reset(self):
        self._orders.clear()
        self._orders_bid.clear()
        self._orders_ask.clear()
        self.bid_raw = 0
        self.ask_raw = 0
        self.last_raw = 0
        self.is_bid_initialized = False
        self.is_ask_initialized = False
        self.is_last_initialized = False

    cpdef void add_order(self, Order order):
        Condition.not_none(order, "order")

        # 由于 cpdef 函数不支持闭包，所以需要此步骤
        self._add_order(order)

    cdef void _add_order(self, Order order):
        # 索引订单
        self._orders[order.client_order_id] = order

        if order.side == OrderSide.BUY:
            self._orders_bid.append(order)
            self.sort_bid_orders()
        elif order.side == OrderSide.SELL:
            self._orders_ask.append(order)
            self.sort_ask_orders()
        else:
            raise RuntimeError(f"无效的 `OrderSide`，原为 {order.side}")  # pragma: no cover (设计时错误)

    cdef void sort_bid_orders(self):
        self._orders_bid.sort(key=order_sort_key, reverse=True)

    cdef void sort_ask_orders(self):
        self._orders_ask.sort(key=order_sort_key)

    cpdef void delete_order(self, Order order):
        Condition.not_none(order, "order")

        self._orders.pop(order.client_order_id, None)

        if order.side == OrderSide.BUY:
            if order in self._orders_bid:
                self._orders_bid.remove(order)
        elif order.side == OrderSide.SELL:
            if order in self._orders_ask:
                self._orders_ask.remove(order)
        else:
            raise RuntimeError(f"无效的 `OrderSide`，原为 {order.side}")  # pragma: no cover (设计时错误)

    cpdef void iterate(self, uint64_t timestamp_ns):
        cdef Order order
        for order in self._orders_bid + self._orders_ask:  # 列表会被隐式复制
            if order.is_closed_c():
                continue  # 订单状态在迭代开始后发生了变化  # pragma: no cover
            self.match_order(order)

# -- 撮合 -------------------------------------------------------------------------------------

    cpdef void match_order(self, Order order, bint initial = False):
        """
        撮合给定的订单。

        Parameters
        ----------
        order : Order
            要撮合的订单。
        initial : bool, 默认 False
            这是否是初始撮合。

        Raises
        ------
        TypeError
            如果 `order.order_type` 对于核心来说是无效类型（例如 `MARKET`）。

        """
        Condition.not_none(order, "order")

        if (
            order.order_type == OrderType.LIMIT
            or order.order_type == OrderType.MARKET_TO_LIMIT
        ):
            self.match_limit_order(order)
        elif order.order_type == OrderType.STOP_LIMIT:
            self.match_stop_limit_order(order, initial)
        elif order.order_type == OrderType.STOP_MARKET:
            self.match_stop_market_order(order)
        elif order.order_type == OrderType.LIMIT_IF_TOUCHED:
            self.match_limit_if_touched_order(order, initial)
        elif order.order_type == OrderType.MARKET_IF_TOUCHED:
            self.match_market_if_touched_order(order)
        elif order.order_type == OrderType.TRAILING_STOP_LIMIT:
            self.match_trailing_stop_limit_order(order, initial)
        elif order.order_type == OrderType.TRAILING_STOP_MARKET:
            self.match_trailing_stop_market_order(order)
        else:
            raise TypeError(f"invalid `OrderType` was {order.order_type}")  # pragma: no cover (design-time error)

    cpdef void match_limit_order(self, Order order):
        Condition.not_none(order, "order")

        if self.is_limit_fillable(order.side, order.price):
            order.liquidity_side = LiquiditySide.MAKER
            self._fill_limit_order(order)

    cpdef void match_stop_market_order(self, Order order):
        Condition.not_none(order, "order")

        if self.is_stop_triggered(order.side, order.trigger_price):
            order.set_triggered_price_c(order.trigger_price)
            # 触发后的止损单作为市价单处理
            self._fill_market_order(order)

    cpdef void match_stop_limit_order(self, Order order, bint initial):
        Condition.not_none(order, "order")

        if order.is_triggered:
            if self.is_limit_fillable(order.side, order.price):
                order.liquidity_side = LiquiditySide.MAKER
                self._fill_limit_order(order)

            return

        cdef LiquiditySide liquidity_side
        if self.is_stop_triggered(order.side, order.trigger_price):
            order.set_triggered_price_c(order.trigger_price)
            order.liquidity_side = self._determine_order_liquidity(
                initial,
                order.side,
                order.price,
                order.trigger_price,
            )
            # 检查是否可立即成交
            self._trigger_stop_order(order)

    cpdef void match_market_if_touched_order(self, Order order):
        Condition.not_none(order, "order")

        if self.is_touch_triggered(order.side, order.trigger_price):
            order.set_triggered_price_c(order.trigger_price)
            # 触发后的止损单作为市价单处理
            self._fill_market_order(order)

    cpdef void match_limit_if_touched_order(self, Order order, bint initial):
        Condition.not_none(order, "order")

        if order.is_triggered:
            if self.is_limit_fillable(order.side, order.price):
                order.liquidity_side = LiquiditySide.MAKER
                self._fill_limit_order(order)

            return

        cdef LiquiditySide liquidity_side
        if self.is_touch_triggered(order.side, order.trigger_price):
            if not initial:
                order.set_triggered_price_c(order.trigger_price)
            order.liquidity_side = self._determine_order_liquidity(
                initial,
                order.side,
                order.price,
                order.trigger_price,
            )
            # 检查是否可立即成交
            self._trigger_stop_order(order)

    cpdef void match_trailing_stop_limit_order(self, Order order, bint initial):
        if order.is_activated:
            self.match_stop_limit_order(order, initial)

    cpdef void match_trailing_stop_market_order(self, Order order):
        if order.is_activated:
            self.match_stop_market_order(order)

    cpdef bint is_limit_fillable(self, OrderSide side, Price price):
        # True when order can be filled: crosses the spread or is inside the spread.
        return self.is_limit_marketable(side, price) or self._is_inside_spread(side, price)

    cpdef bint is_limit_marketable(self, OrderSide side, Price price):
        # True when order would take liquidity (crosses the spread). Used for post-only rejection.
        Condition.not_none(price, "price")

        if side == OrderSide.BUY:
            if not self.is_ask_initialized:
                return False  # 无行情
            return self.ask_raw <= price._mem.raw
        elif side == OrderSide.SELL:
            if not self.is_bid_initialized:
                return False  # 无行情
            return self.bid_raw >= price._mem.raw
        else:
            raise ValueError(f"无效的 `OrderSide`，原为 {side}")  # pragma: no cover (设计时错误)

    cdef bint _is_inside_spread(self, OrderSide side, Price price):
        # Check if a limit order is priced inside the bid-ask spread
        if price is None:
            return False
        if side == OrderSide.BUY:
            return self.is_bid_initialized and price._mem.raw >= self.bid_raw
        elif side == OrderSide.SELL:
            return self.is_ask_initialized and price._mem.raw <= self.ask_raw

        return False

    cpdef bint is_stop_triggered(self, OrderSide side, Price trigger_price):
        Condition.not_none(trigger_price, "trigger_price")

        if side == OrderSide.BUY:
            if not self.is_ask_initialized:
                return False  # 无行情
            return self.ask_raw >= trigger_price._mem.raw
        elif side == OrderSide.SELL:
            if not self.is_bid_initialized:
                return False  # 无行情
            return self.bid_raw <= trigger_price._mem.raw
        else:
            raise ValueError(f"无效的 `OrderSide`，原为 {side}")  # pragma: no cover (设计时错误)

    cpdef bint is_touch_triggered(self, OrderSide side, Price trigger_price):
        Condition.not_none(trigger_price, "trigger_price")

        if side == OrderSide.BUY:
            if not self.is_ask_initialized:
                return False  # 无行情
            return self.ask_raw <= trigger_price._mem.raw
        elif side == OrderSide.SELL:
            if not self.is_bid_initialized:
                return False  # 无行情
            return self.bid_raw >= trigger_price._mem.raw
        else:
            raise ValueError(f"无效的 `OrderSide`，原为 {side}")  # pragma: no cover (设计时错误)

    cdef LiquiditySide _determine_order_liquidity(
        self,
        bint initial,
        OrderSide side,
        Price price,
        Price trigger_price,
    ):
        if initial:
            return LiquiditySide.TAKER

        if side == OrderSide.BUY and trigger_price._mem.raw > price._mem.raw:
            return LiquiditySide.MAKER
        elif side == OrderSide.SELL and trigger_price._mem.raw < price._mem.raw:
            return LiquiditySide.MAKER

        return LiquiditySide.TAKER


cdef inline int64_t order_sort_key(Order order):
    cdef Price trigger_price
    cdef Price price
    if order.order_type == OrderType.LIMIT:
        price = order.price
        return price._mem.raw
    elif order.order_type == OrderType.MARKET_TO_LIMIT:
        price = order.price
        return price._mem.raw
    elif order.order_type == OrderType.STOP_MARKET:
        trigger_price = order.trigger_price
        return trigger_price._mem.raw
    elif order.order_type == OrderType.STOP_LIMIT:
        trigger_price = order.trigger_price
        price = order.price
        return price._mem.raw if order.is_triggered else trigger_price._mem.raw
    elif order.order_type == OrderType.MARKET_IF_TOUCHED:
        trigger_price = order.trigger_price
        return trigger_price._mem.raw
    elif order.order_type == OrderType.LIMIT_IF_TOUCHED:
        trigger_price = order.trigger_price
        price = order.price
        return price._mem.raw if order.is_triggered else trigger_price._mem.raw
    elif order.order_type == OrderType.TRAILING_STOP_MARKET:
        trigger_price = order.trigger_price or order.activation_price
        return trigger_price._mem.raw
    elif order.order_type == OrderType.TRAILING_STOP_LIMIT:
        trigger_price = order.trigger_price or order.activation_price
        price = order.price
        return price._mem.raw if order.is_triggered else trigger_price._mem.raw
    else:
        raise RuntimeError(  # pragma: no cover (设计时错误)
            f"排序订单簿时的订单类型无效，"  # pragma: no cover (设计时错误)
            f"原为 {order_type_to_str(order.order_type)}",  # pragma: no cover (设计时错误)
        )
