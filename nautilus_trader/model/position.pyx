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

from libc.math cimport fabs
from libc.math cimport fmin

from nautilus_trader.core.correctness cimport Condition
from nautilus_trader.core.rust.model cimport InstrumentClass
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport PositionSide
from nautilus_trader.core.uuid cimport UUID4
from nautilus_trader.model.events.order cimport OrderFilled
from nautilus_trader.model.events.position cimport PositionAdjusted
from nautilus_trader.model.events.position cimport PositionAdjustmentType
from nautilus_trader.model.functions cimport order_side_to_str
from nautilus_trader.model.functions cimport position_side_to_str
from nautilus_trader.model.identifiers cimport TradeId
from nautilus_trader.model.instruments.base cimport Instrument
from nautilus_trader.model.instruments.currency_pair cimport CurrencyPair
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.objects cimport Quantity


cdef class Position:
    """
    表示市场中的仓位。

    仓位 ID 可以由交易平台分配，也可以由系统生成，具体取决于策略的 OMS（订单管理系统）设置。

    参数
    ----------
    instrument : Instrument
        该仓位的交易标的。
    fill : OrderFilled
        开启仓位的订单成交事件。

    引发
    ------
    ValueError
        如果 `instrument.id` 不等于 `fill.instrument_id`。
    ValueError
        如果 `fill.position_id` 为 ``None``。
    """

    def __init__(
        self,
        Instrument instrument not None,
        OrderFilled fill not None,
    ) -> None:
        Condition.equal(instrument.id, fill.instrument_id, "instrument.id", "fill.instrument_id")
        Condition.not_none(fill.position_id, "fill.position_id")

        self._events: list[OrderFilled] = []
        self._adjustments: list = []
        self._trade_ids: list[TradeId] = []
        self._buy_qty = Quantity.zero_c(precision=instrument.size_precision)
        self._sell_qty = Quantity.zero_c(precision=instrument.size_precision)
        self._commissions = {}

        # 标识符
        self.trader_id = fill.trader_id
        self.strategy_id = fill.strategy_id
        self.instrument_id = fill.instrument_id
        self.id = fill.position_id
        self.account_id = fill.account_id
        self.opening_order_id = fill.client_order_id
        self.closing_order_id = None

        # 属性
        self.entry = fill.order_side
        self.side = Position.side_from_order_side(fill.order_side)
        self.signed_qty = 0.0
        self.quantity = Quantity.zero_c(precision=instrument.size_precision)
        self.peak_qty = Quantity.zero_c(precision=instrument.size_precision)
        self.ts_init = fill.ts_init
        self.ts_opened = fill.ts_event
        self.ts_last = fill.ts_event
        self.ts_closed = 0
        self.duration_ns = 0
        self.avg_px_open = fill.last_px.as_f64_c()
        self.avg_px_close = 0.0
        self.price_precision = instrument.price_precision
        self.size_precision = instrument.size_precision
        self.multiplier = instrument.multiplier
        self.is_inverse = instrument.is_inverse
        self.is_spot_currency = isinstance(instrument, CurrencyPair)
        self.instrument_class = instrument.instrument_class
        self.quote_currency = instrument.quote_currency
        self.base_currency = instrument.get_base_currency()  # 可能是 None
        self.settlement_currency = instrument.get_cost_currency()  # 待定：处理 quanto 的情况

        self.realized_return = 0.0
        self.realized_pnl = None

        self.apply(fill)

    def __eq__(self, Position other) -> bool:
        if other is None:
            return False
        return self.id == other.id

    def __hash__(self) -> int:
        return hash(self.id)

    def __repr__(self) -> str:
        return f"{type(self).__name__}({self.info()}, id={self.id})"

    def purge_events_for_order(self, ClientOrderId client_order_id) -> None:
        """
        清除给定客户端订单 ID 的所有订单事件。

        清除后，仓位将根据剩余的成交数据重新构建。如果没有剩余的成交数据，
        则仓位会被重置为一个清除所有历史记录（包括时间戳）的空壳，使其
        有资格立即进行缓存清理。

        参数
        ----------
        client_order_id : ClientOrderId
            要清除事件的客户端订单 ID。

        """
        Condition.not_none(client_order_id, "client_order_id")

        cdef list[OrderFilled] remaining_events = [
            event for event in self._events
            if event.client_order_id != client_order_id
        ]

        # 保留非佣金调整（资金费用、手动调整等）
        # 佣金调整将在重放成交数据时自动重新创建
        cdef list preserved_adjustments = [
            adj for adj in self._adjustments
            if adj.adjustment_type != PositionAdjustmentType.COMMISSION
        ]

        self._events.clear()
        self._trade_ids.clear()
        self._adjustments.clear()

        # 如果没有剩余成交，则重置为 FLAt（持平）状态，清除所有历史记录
        if not remaining_events:
            self._buy_qty = Quantity.zero_c(precision=self.size_precision)
            self._sell_qty = Quantity.zero_c(precision=self.size_precision)
            self._commissions = {}
            self.signed_qty = 0.0
            self.quantity = Quantity.zero_c(precision=self.size_precision)
            self.side = PositionSide.FLAT
            self.avg_px_close = 0.0
            self.realized_pnl = None
            self.realized_return = 0.0
            self.ts_opened = 0
            self.ts_last = 0
            self.ts_closed = 0
            self.duration_ns = 0
            return

        self.side = PositionSide.FLAT
        self.signed_qty = 0.0

        # 重新应用所有剩余的成交以重建状态
        cdef OrderFilled event
        for event in remaining_events:
            self.apply(event)

        # 重新应用保留的调整以维持完整状态
        cdef PositionAdjusted adjustment
        for adjustment in preserved_adjustments:
            self.apply_adjustment(adjustment)

    cpdef str info(self):
        """
        返回仓位的摘要描述。

        返回
        -------
        str

        """
        cdef str quantity = " " if self.quantity._mem.raw == 0 else f" {self.quantity.to_formatted_str()} "
        return f"{position_side_to_str(self.side)}{quantity}{self.instrument_id}"

    cpdef dict to_dict(self):
        """
        返回此对象的字典表示。

        返回
        -------
        dict[str, object]

        """
        return {
            "position_id": self.id.to_str(),
            "trader_id": self.trader_id.to_str(),
            "strategy_id": self.strategy_id.to_str(),
            "instrument_id": self.instrument_id.to_str(),
            "account_id": self.account_id.to_str(),
            "opening_order_id": self.opening_order_id.to_str(),
            "closing_order_id": self.closing_order_id.to_str() if self.closing_order_id is not None else None,
            "entry": order_side_to_str(self.entry),
            "side": position_side_to_str(self.side),
            "signed_qty": self.signed_qty,
            "quantity": str(self.quantity),
            "peak_qty": str(self.peak_qty),
            "ts_init": self.ts_init,
            "ts_opened": self.ts_opened,
            "ts_last": self.ts_last,
            "ts_closed": self.ts_closed if self.ts_closed > 0 else None,
            "duration_ns": self.duration_ns if self.duration_ns > 0 else None,
            "avg_px_open": self.avg_px_open,
            "avg_px_close": self.avg_px_close if self.avg_px_close > 0 else None,
            "quote_currency": self.quote_currency.code,
            "base_currency": self.base_currency.code if self.base_currency is not None else None,
            "settlement_currency": self.settlement_currency.code,
            "commissions": sorted([str(c) for c in self.commissions()]),
            "realized_return": round(self.realized_return, 5),
            "realized_pnl": str(self.realized_pnl),
        }

    cdef list client_order_ids_c(self):
        # 注意内部使用了集合 {} 以去重
        return sorted(list({fill.client_order_id for fill in self._events}))

    cdef list venue_order_ids_c(self):
        # 注意内部使用了集合 {} 以去重
        return sorted(list({fill.venue_order_id for fill in self._events}))

    cdef list trade_ids_c(self):
        # 在追加到事件之前已检查是否重复
        return [fill.trade_id for fill in self._events]

    cdef list events_c(self):
        return self._events.copy()

    cdef list adjustments_c(self):
        return self._adjustments.copy()

    cdef OrderFilled last_event_c(self):
        return self._events[-1] if self._events else None

    cdef TradeId last_trade_id_c(self):
        return self._events[-1].trade_id if self._events else None

    cdef bint has_trade_id_c(self, TradeId trade_id):
        Condition.not_none(trade_id, "trade_id")
        return trade_id in self._trade_ids

    cdef int event_count_c(self):
        return len(self._events)

    cdef bint is_open_c(self):
        return self.side != PositionSide.FLAT

    cdef bint is_closed_c(self):
        return self.side == PositionSide.FLAT

    cdef bint is_long_c(self):
        return self.side == PositionSide.LONG

    cdef bint is_short_c(self):
        return self.side == PositionSide.SHORT

    @property
    def symbol(self):
        """
        返回仓位的证券代码 (symbol)。

        返回
        -------
        Symbol

        """
        return self.instrument_id.symbol

    @property
    def venue(self):
        """
        返回仓位的交易平台 (venue)。

        返回
        -------
        Venue

        """
        return self.instrument_id.venue

    @property
    def client_order_ids(self):
        """
        返回与该仓位关联的客户端订单 ID。

        返回
        -------
        list[ClientOrderId]

        说明
        -----
        保证不包含重复的 ID。

        """
        return self.client_order_ids_c()

    @property
    def venue_order_ids(self):
        """
        返回与该仓位关联的平台订单 ID。

        返回
        -------
        list[VenueOrderId]

        说明
        -----
        保证不包含重复的 ID。

        """
        return self.venue_order_ids_c()

    @property
    def trade_ids(self):
        """
        返回与该仓位关联的交易撮合 ID (trade match ID)。

        返回
        -------
        list[TradeId]

        """
        return self.trade_ids_c()

    @property
    def events(self):
        """
        返回仓位的订单成交事件。

        返回
        -------
        list[Event]

        """
        return self.events_c()

    @property
    def adjustments(self):
        """
        返回仓位调整事件。

        返回
        -------
        list[PositionAdjusted]

        """
        return self.adjustments_c()

    @property
    def last_event(self):
        """
        返回最后一个订单成交事件（如果清除后仍有剩余）。

        返回
        -------
        OrderFilled 或 ``None``

        """
        return self.last_event_c()

    @property
    def last_trade_id(self):
        """
        返回仓位的最后一个交易撮合 ID（如果清除后仍有剩余）。

        返回
        -------
        TradeId 或 ``None``

        """
        return self.last_trade_id_c()

    @property
    def event_count(self):
        """
        返回应用于该仓位的订单成交事件数量。

        返回
        -------
        int

        """
        return self.event_count_c()

    @property
    def is_open(self):
        """
        返回仓位方向是否 **不为** ``FLAT``（持平）。

        返回
        -------
        bool

        """
        return self.is_open_c()

    @property
    def is_closed(self):
        """
        返回仓位方向是否为 ``FLAT``（持平）。

        返回
        -------
        bool

        """
        return self.is_closed_c()

    @property
    def is_long(self):
        """
        返回仓位方向是否为 ``LONG``（多头）。

        返回
        -------
        bool

        """
        return self.is_long_c()

    @property
    def is_short(self):
        """
        返回仓位方向是否为 ``SHORT``（空头）。

        返回
        -------
        bool

        """
        return self.is_short_c()

    @staticmethod
    cdef PositionSide side_from_order_side_c(OrderSide side):
        if side == OrderSide.BUY:
            return PositionSide.LONG
        elif side == OrderSide.SELL:
            return PositionSide.SHORT
        else:
            raise ValueError(  # pragma: no cover (design-time error)
                f"无效的 `OrderSide`，值为 {side}",  # pragma: no cover (设计时错误)
            )


    @staticmethod
    def side_from_order_side(OrderSide side):
        """
        返回给定订单方向（从 ``FLAT`` 开始）导致的仓位方向。

        参数
        ----------
        side : OrderSide {``BUY``, ``SELL``}
            订单方向

        返回
        -------
        PositionSide

        """
        return Position.side_from_order_side_c(side)

    cpdef OrderSide closing_order_side(self):
        """
        返回该仓位的平仓订单方向。

        如果仓位是 ``FLAT``，则返回 ``NO_ORDER_SIDE``。

        返回
        -------
        OrderSide

        """
        if self.side == PositionSide.LONG:
            return OrderSide.SELL
        elif self.side == PositionSide.SHORT:
            return OrderSide.BUY
        else:
            return OrderSide.NO_ORDER_SIDE

    cpdef signed_decimal_qty(self):
        """
        返回仓位数量的有符号十进制表示。

         - 如果仓位是 LONG，则值为正数（例如 Decimal('10.25')）
         - 如果仓位是 SHORT，则值为负数（例如 Decimal('-10.25')）
         - 如果仓位是 FLAT，则值为零（例如 Decimal('0')）

        返回
        -------
        Decimal

        """
        return Decimal(f"{self.signed_qty:.{self.size_precision}f}")

    cpdef bint is_opposite_side(self, OrderSide side):
        """
        返回一个值，指示给定的订单方向是否与当前仓位方向相反。

        参数
        ----------
        side : OrderSide {``BUY``, ``SELL``}

        返回
        -------
        bool
            如果方向相反则为 True，否则为 False。

        """
        return self.side != Position.side_from_order_side_c(side)

    cpdef void apply(self, OrderFilled fill):
        """
        将给定的订单成交事件应用于仓位。

        如果应用 `fill` 之前仓位是 FLAT，则在处理新成交之前，
        仓位状态将被重置（清除现有的事件、佣金等）。

        参数
        ----------
        fill : OrderFilled
            要应用的订单成交事件。

        引发
        ------
        KeyError
            如果 `fill.trade_id` 已应用于该仓位。

        """
        Condition.not_none(fill, "fill")
        self._check_duplicate_trade_id(fill)

        # 在平仓后重新开启仓位，重置为初始状态
        if self.side == PositionSide.FLAT:
            self._events.clear()
            self._trade_ids.clear()
            self._adjustments.clear()
            self._buy_qty = Quantity.zero_c(precision=self.size_precision)
            self._sell_qty = Quantity.zero_c(precision=self.size_precision)
            self._commissions = {}
            self.opening_order_id = fill.client_order_id
            self.closing_order_id = None
            self.peak_qty = Quantity.zero_c(precision=self.size_precision)
            self.ts_init = fill.ts_init
            self.ts_opened = fill.ts_event
            self.ts_closed = 0
            self.duration_ns = 0
            self.avg_px_open = fill.last_px.as_f64_c()
            self.avg_px_close = 0.0
            self.realized_return = 0.0
            self.realized_pnl = None

        self._events.append(fill)
        self._trade_ids.append(fill.trade_id)

        # 以其币种累积佣金
        cdef Currency currency = fill.commission.currency
        cdef Money commissions = self._commissions.get(currency)
        cdef double total_commissions = commissions.as_f64_c() if commissions is not None else 0.0
        self._commissions[currency] = Money(total_commissions + fill.commission.as_f64_c(), currency)

        if fill.order_side == OrderSide.BUY:
            self._handle_buy_order_fill(fill)
        elif fill.order_side == OrderSide.SELL:
            self._handle_sell_order_fill(fill)
        else:
            raise ValueError(  # pragma: no cover (design-time error)
                f"无效的 `OrderSide`，值为 {fill.order_side}",  # pragma: no cover (设计时错误)
            )

        # 对于 CurrencyPair 标的，当佣金计入基础币种时创建调整事件
        if (
            self.is_spot_currency
            and self.base_currency is not None
            and fill.commission is not None
            and fill.commission.currency == self.base_currency
        ):
            adjustment = PositionAdjusted(
                self.trader_id,
                self.strategy_id,
                self.instrument_id,
                self.id,
                self.account_id,
                PositionAdjustmentType.COMMISSION,
                fill.commission.as_decimal(),
                None,
                str(fill.client_order_id),
                UUID4(),
                fill.ts_event,
                fill.ts_init,
            )
            self.apply_adjustment(adjustment)

        # 更新数量、峰值数量和仓位方向
        self.quantity = Quantity(abs(self.signed_qty), self.size_precision)
        if self.quantity._mem.raw > self.peak_qty._mem.raw:
            self.peak_qty = self.quantity

        if self.signed_qty > 0.0:
            self.entry = OrderSide.BUY
            self.side = PositionSide.LONG
        elif self.signed_qty < 0.0:
            self.entry = OrderSide.SELL
            self.side = PositionSide.SHORT
        else:
            # 仓位已平
            self.side = PositionSide.FLAT
            self.closing_order_id = fill.client_order_id
            self.ts_closed = fill.ts_event
            self.duration_ns = self.ts_closed - self.ts_opened

        self.ts_last = fill.ts_event

    cpdef void apply_adjustment(self, PositionAdjusted adjustment):
        """
        应用仓位调整事件。

        此方法处理发生在正常订单成交之外的仓位数量或已实现盈亏的调整，例如：
        - 基础币种的佣金调整（加密货币现货市场）
        - 资金费支付（永续合约）

        调整事件存储在仓位的调整历史中，以提供完整的审计追踪。

        参数
        ----------
        adjustment : PositionAdjusted
            要应用的仓位调整事件。

        """
        Condition.not_none(adjustment, "adjustment")

        # 如果存在数量变化则应用
        if adjustment.quantity_change is not None:
            self.signed_qty += float(adjustment.quantity_change)

            self.quantity = Quantity(abs(self.signed_qty), self.size_precision)

            if self.quantity._mem.raw > self.peak_qty._mem.raw:
                self.peak_qty = self.quantity

        # 如果存在盈亏变化则应用
        cdef double current_pnl

        if adjustment.pnl_change is not None:
            current_pnl = self.realized_pnl.as_f64_c() if self.realized_pnl is not None else 0.0
            self.realized_pnl = Money(
                current_pnl + adjustment.pnl_change.as_f64_c(),
                self.settlement_currency,
            )

        # 根据新的有符号数量更新仓位状态
        if self.signed_qty > 0.0:
            self.side = PositionSide.LONG
            if self.entry == OrderSide.NO_ORDER_SIDE:
                self.entry = OrderSide.BUY
        elif self.signed_qty < 0.0:
            self.side = PositionSide.SHORT
            if self.entry == OrderSide.NO_ORDER_SIDE:
                self.entry = OrderSide.SELL
        else:
            self.side = PositionSide.FLAT

        self._adjustments.append(adjustment)
        self.ts_last = adjustment.ts_event

    cpdef Money notional_value(
        self,
        Price price,
        Currency target_currency=None,
        Price conversion_price=None,
    ):
        """
        返回仓位的当前名义价值 (notional value)，计算时使用参考价格
        （例如：买入价、卖出价、中间价、最后成交价或标记价格）。

        - 对于标准（非反向）合约，名义价值以报价币种返回。
        - 对于反向合约，名义价值以基础币种返回，计算时按 1 / 价格进行缩放。

        如果提供了 `target_currency` 和 `conversion_price`，
        则名义价值将转换为目标币种。

        参数
        ----------
        price : Price
            用于计算的参考价格。可以是最后价格、中间价、买入价、卖出价、
            逐日盯市价格或任何其他合适的代表值。
        target_currency : Currency, 可选
            转换的目标币种。
        conversion_price : Price, 可选
            用于币种转换的价格。

        返回
        -------
        Money
            对于标准合约以报价币种计价，如果是反向合约则以基础币种计价。

        """
        Condition.not_none(price, "price")

        cdef Money notional
        if self.is_inverse:
            notional = Money(
                self.quantity.as_f64_c() * self.multiplier.as_f64_c() * (1.0 / price.as_f64_c()),
                self.base_currency,
            )
        else:
            notional = Money(
                self.quantity.as_f64_c() * self.multiplier.as_f64_c() * price.as_f64_c(),
                self.quote_currency,
            )

        if target_currency is not None and conversion_price is not None:
            return Money(notional.as_f64_c() * conversion_price.as_f64_c(), target_currency)

        return notional

    cpdef Money cross_notional_value(
        self,
        Price price,
        Price quote_price,
        Price base_price,
        Currency target_currency,
    ):
        """
        以交叉/目标币种返回仓位的当前名义价值。

        `quote_price` 是“报价币种/目标币种”的转换价格，`base_price`
        是“基础币种/目标币种”的转换价格。

        参数
        ----------
        price : Price
            用于计算的参考价格。
        quote_price : Price
            报价币种/目标币种的转换价格。
        base_price : Price
            基础币种/目标币种的转换价格。
        target_currency : Currency
            转换的目标币种。

        返回
        -------
        Money

        """
        cdef Price conversion_price = base_price if self.is_inverse else quote_price
        return self.notional_value(
            price=price,
            target_currency=target_currency,
            conversion_price=conversion_price,
        )

    cpdef Money calculate_pnl(
        self,
        double avg_px_open,
        double avg_px_close,
        Quantity quantity,
    ):
        """
        以该标的的结算币种返回计算出的盈亏。

        参数
        ----------
        avg_px_open : double
            平均开仓价格。
        avg_px_close : double
            平均平仓价格。
        quantity : Quantity
            用于计算的数量。

        返回
        -------
        Money
            以结算币种计价。

        """
        cdef double pnl = self._calculate_pnl(
            avg_px_open=avg_px_open,
            avg_px_close=avg_px_close,
            quantity=quantity.as_f64_c(),
        )

        return Money(pnl, self.settlement_currency)

    cpdef Money unrealized_pnl(self, Price price):
        """
        返回仓位的未实现盈亏，计算时使用参考价格
        （例如：买入价、卖出价、中间价、最后成交价或标记价格）。

        参数
        ----------
        price : Price
            用于计算的参考价格。可以是最后价格、中间价、买入价、卖出价、
            逐日盯市价格或任何其他合适的代表值。

        返回
        -------
        Money
            以结算币种计价。

        """
        Condition.not_none(price, "price")

        if self.side == PositionSide.FLAT:
            return Money(0, self.settlement_currency)

        cdef double pnl = self._calculate_pnl(
            avg_px_open=self.avg_px_open,
            avg_px_close=price.as_f64_c(),
            quantity=self.quantity.as_f64_c(),
        )

        return Money(pnl, self.settlement_currency)

    cpdef Money total_pnl(self, Price price):
        """
        返回仓位的总盈亏，计算时使用参考价格
        （例如：买入价、卖出价、中间价、最后成交价或标记价格）。

        参数
        ----------
        price : Price
            用于计算的参考价格。可以是最后价格、中间价、买入价、卖出价、
            逐日盯市价格或任何其他合适的代表值。

        返回
        -------
        Money
            以结算币种计价。

        """
        Condition.not_none(price, "price")

        cdef double realized_pnl = self.realized_pnl.as_f64_c() if self.realized_pnl is not None else 0.0
        return Money(realized_pnl + self.unrealized_pnl(price).as_f64_c(), self.settlement_currency)

    cpdef list commissions(self):
        """
        返回该仓位产生的总佣金。

        返回
        -------
        list[Money]

        """
        return list(self._commissions.values())

    cdef void _check_duplicate_trade_id(self, OrderFilled fill):
        # 检查所有先前的成交数据，看是否有匹配的交易 ID 和复合键 (composite key)
        cdef:
            OrderFilled p_fill
        for p_fill in self._events:
            if fill.trade_id != p_fill.trade_id:
                continue
            if (
                fill.order_side == p_fill.order_side
                and fill.last_px == p_fill.last_px
                and fill.last_qty == p_fill.last_qty
            ):
                raise KeyError(f"Duplicate {fill.trade_id!r} in events {fill} {p_fill}")

    cdef void _handle_buy_order_fill(self, OrderFilled fill):
        cdef:
            double realized_pnl
            double last_px
            double last_qty
            Quantity last_qty_obj

        # 处理佣金可能为 None 或非结算币种的情况
        if fill.commission.currency == self.settlement_currency:
            realized_pnl = -fill.commission.as_f64_c()
        else:
            realized_pnl = 0.0

        last_px = fill.last_px.as_f64_c()
        last_qty = fill.last_qty.as_f64_c()
        last_qty_obj = fill.last_qty

        # 多头仓位 (LONG POSITION)
        if self.signed_qty > 0:
            self.avg_px_open = self._calculate_avg_px_open_px(last_px, last_qty)
        # 空头仓位 (SHORT POSITION)
        elif self.signed_qty < 0:
            self.avg_px_close = self._calculate_avg_px_close_px(last_px, last_qty)
            self.realized_return = self._calculate_return(self.avg_px_open, self.avg_px_close)
            realized_pnl += self._calculate_pnl(self.avg_px_open, last_px, last_qty)

        if self.realized_pnl is None:
            self.realized_pnl = Money(realized_pnl, self.settlement_currency)
        else:
            self.realized_pnl = Money(self.realized_pnl.as_f64_c() + realized_pnl, self.settlement_currency)

        self._buy_qty = self._buy_qty + last_qty_obj
        self.signed_qty += last_qty
        self.signed_qty = round(self.signed_qty, self.size_precision)

    cdef void _handle_sell_order_fill(self, OrderFilled fill):
        cdef:
            double realized_pnl
            double last_px
            double last_qty
            Quantity last_qty_obj

        # 处理佣金可能为 None 或非结算币种的情况
        if fill.commission.currency == self.settlement_currency:
            realized_pnl = -fill.commission.as_f64_c()
        else:
            realized_pnl = 0.0

        last_px = fill.last_px.as_f64_c()
        last_qty = fill.last_qty.as_f64_c()
        last_qty_obj = fill.last_qty

        # 空头仓位 (SHORT POSITION)
        if self.signed_qty < 0:
            self.avg_px_open = self._calculate_avg_px_open_px(last_px, last_qty)
        # 多头仓位 (LONG POSITION)
        elif self.signed_qty > 0:
            self.avg_px_close = self._calculate_avg_px_close_px(last_px, last_qty)
            self.realized_return = self._calculate_return(self.avg_px_open, self.avg_px_close)
            realized_pnl += self._calculate_pnl(self.avg_px_open, last_px, last_qty)

        if self.realized_pnl is None:
            self.realized_pnl = Money(realized_pnl, self.settlement_currency)
        else:
            self.realized_pnl = Money(self.realized_pnl.as_f64_c() + realized_pnl, self.settlement_currency)

        self._sell_qty = self._sell_qty + last_qty_obj
        self.signed_qty -= last_qty
        self.signed_qty = round(self.signed_qty, self.size_precision)

    cdef double _calculate_avg_px_open_px(self, double last_px, double last_qty):
        return self._calculate_avg_px(self.quantity.as_f64_c(), self.avg_px_open, last_px, last_qty)

    cdef double _calculate_avg_px_close_px(self, double last_px, double last_qty):
        if not self.avg_px_close:
            return last_px
        close_qty = self._sell_qty if self.side == PositionSide.LONG else self._buy_qty
        return self._calculate_avg_px(close_qty.as_f64_c(), self.avg_px_close, last_px, last_qty)

    cdef double _calculate_avg_px(
        self,
        double qty,
        double avg_px,
        double last_px,
        double last_qty,
    ):
        cdef double start_cost = avg_px * qty
        cdef double event_cost = last_px * last_qty
        return (start_cost + event_cost) / (qty + last_qty)

    cdef double _calculate_points(self, double avg_px_open, double avg_px_close):
        if self.side == PositionSide.LONG:
            return avg_px_close - avg_px_open
        elif self.side == PositionSide.SHORT:
            return avg_px_open - avg_px_close
        else:
            return 0.0  # 持平 (FLAT)

    cdef double _calculate_points_inverse(self, double avg_px_open, double avg_px_close):
        cdef double EPSILON = 1e-15

        # 针对零价格或接近零价格的防御性检查
        if fabs(avg_px_open) < EPSILON or fabs(avg_px_close) < EPSILON:
            return 0.0

        if self.side == PositionSide.LONG:
            return (1.0 / avg_px_open) - (1.0 / avg_px_close)
        elif self.side == PositionSide.SHORT:
            return (1.0 / avg_px_close) - (1.0 / avg_px_open)
        else:
            return 0.0  # 持平 (FLAT)

    cdef double _calculate_return(self, double avg_px_open, double avg_px_close):
        # 针对零开仓价格的防御性检查
        if avg_px_open == 0.0:
            return 0.0

        return self._calculate_points(avg_px_open, avg_px_close) / avg_px_open

    cdef double _calculate_pnl(
        self,
        double avg_px_open,
        double avg_px_close,
        double quantity,
    ):
        # 仅将未平仓数量计入盈亏（限制在实际仓位大小内）
        quantity = fmin(quantity, fabs(self.signed_qty))

        if self.is_inverse:
            # 以基础币种计
            return quantity * self.multiplier.as_f64_c() * self._calculate_points_inverse(avg_px_open, avg_px_close)
        else:
            # 以报价币种计
            return quantity * self.multiplier.as_f64_c() * self._calculate_points(avg_px_open, avg_px_close)
