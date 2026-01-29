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

from libc.stdint cimport uint8_t
from libc.stdint cimport uint64_t

from nautilus_trader.core.rust.model cimport InstrumentClass
from nautilus_trader.core.rust.model cimport OrderSide
from nautilus_trader.core.rust.model cimport PositionSide
from nautilus_trader.model.events.order cimport OrderFilled
from nautilus_trader.model.events.position cimport PositionAdjusted
from nautilus_trader.model.identifiers cimport AccountId
from nautilus_trader.model.identifiers cimport ClientOrderId
from nautilus_trader.model.identifiers cimport InstrumentId
from nautilus_trader.model.identifiers cimport PositionId
from nautilus_trader.model.identifiers cimport StrategyId
from nautilus_trader.model.identifiers cimport TradeId
from nautilus_trader.model.identifiers cimport TraderId
from nautilus_trader.model.objects cimport Currency
from nautilus_trader.model.objects cimport Money
from nautilus_trader.model.objects cimport Price
from nautilus_trader.model.objects cimport Quantity


cdef class Position:
    cdef list _events
    cdef list _adjustments
    cdef list _trade_ids
    cdef Quantity _buy_qty
    cdef Quantity _sell_qty
    cdef dict _commissions

    cdef readonly TraderId trader_id
    """与仓位关联的交易员 ID。\n\n:returns: `TraderId`"""
    cdef readonly StrategyId strategy_id
    """与仓位关联的策略 ID。\n\n:returns: `StrategyId`"""
    cdef readonly InstrumentId instrument_id
    """仓位的标的 ID。\n\n:returns: `InstrumentId`"""
    cdef readonly PositionId id
    """仓位 ID。\n\n:returns: `PositionId`"""
    cdef readonly AccountId account_id
    """与仓位关联的账户 ID。\n\n:returns: `AccountId`"""
    cdef readonly ClientOrderId opening_order_id
    """开启仓位的订单的客户订单 ID。\n\n:returns: `ClientOrderId`"""
    cdef readonly ClientOrderId closing_order_id
    """关闭仓位的订单的客户订单 ID。\n\n:returns: `ClientOrderId` 或 ``None``"""
    cdef readonly OrderSide entry
    """仓位入场订单方向。\n\n:returns: `OrderSide`"""
    cdef readonly PositionSide side
    """当前仓位方向。\n\n:returns: `PositionSide`"""
    cdef readonly double signed_qty
    """当前带符号数量（多头仓位为正，空头为负）。\n\n:returns: `double`"""
    cdef readonly Quantity quantity
    """当前未平仓数量。\n\n:returns: `Quantity`"""
    cdef readonly Quantity peak_qty
    """仓位达到的峰值定向数量。\n\n:returns: `Quantity`"""
    cdef readonly uint8_t price_precision
    """仓位的价格精度。\n\n:returns: `uint8`"""
    cdef readonly uint8_t size_precision
    """仓位的数量精度。\n\n:returns: `uint8`"""
    cdef readonly Quantity multiplier
    """仓位对应标的的乘数。\n\n:returns: `Quantity`"""
    cdef readonly bint is_inverse
    """数量是否以报价币种表示。\n\n:returns: `bool`"""
    cdef readonly bint is_spot_currency
    """标的是否为现货货币对。\n\n:returns: `bool`"""
    cdef readonly InstrumentClass instrument_class
    """仓位的标的类别。\n\n:returns: `InstrumentClass`"""
    cdef readonly Currency quote_currency
    """仓位报价币种。\n\n:returns: `Currency`"""
    cdef readonly Currency base_currency
    """仓位基准币种（如果适用）。\n\n:returns: `Currency` 或 ``None``"""
    cdef readonly Currency settlement_currency
    """仓位结算币种（用于盈亏计算）。\n\n:returns: `Currency`"""
    cdef readonly uint64_t ts_init
    """对象初始化时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly uint64_t ts_opened
    """仓位开启时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly uint64_t ts_last
    """发生最后一次事件时的 UNIX 时间戳（纳秒）。\n\n:returns: `uint64_t`"""
    cdef readonly uint64_t ts_closed
    """仓位关闭时的 UNIX 时间戳（纳秒，未关闭则为零）。\n\n:returns: `uint64_t`"""
    cdef readonly uint64_t duration_ns
    """总持仓时长（纳秒，未关闭则为零）。\n\n:returns: `uint64_t`"""
    cdef readonly double avg_px_open
    """平均开仓价格。\n\n:returns: `double`"""
    cdef readonly double avg_px_close
    """平均平仓价格。\n\n:returns: `double`"""
    cdef readonly double realized_return
    """仓当前已实现收益率。\n\n:returns: `double`"""
    cdef readonly Money realized_pnl
    """仓位当前已实现盈亏（包括佣金）。\n\n:returns: `Money` 或 ``None``"""

    cpdef str info(self)
    cpdef dict to_dict(self)

    cdef list client_order_ids_c(self)
    cdef list venue_order_ids_c(self)
    cdef list trade_ids_c(self)
    cdef list events_c(self)
    cdef list adjustments_c(self)
    cdef OrderFilled last_event_c(self)
    cdef TradeId last_trade_id_c(self)
    cdef bint has_trade_id_c(self, TradeId trade_id)
    cdef int event_count_c(self)
    cdef bint is_long_c(self)
    cdef bint is_short_c(self)
    cdef bint is_open_c(self)
    cdef bint is_closed_c(self)

    @staticmethod
    cdef PositionSide side_from_order_side_c(OrderSide side)
    cpdef OrderSide closing_order_side(self)
    cpdef signed_decimal_qty(self)
    cpdef bint is_opposite_side(self, OrderSide side)

    cpdef void apply(self, OrderFilled fill)
    cpdef void apply_adjustment(self, PositionAdjusted adjustment)

    cpdef Money notional_value(self, Price price, Currency target_currency=*, Price conversion_price=*)
    cpdef Money cross_notional_value(self, Price price, Price quote_price, Price base_price, Currency target_currency)
    cpdef Money calculate_pnl(self, double avg_px_open, double avg_px_close, Quantity quantity)
    cpdef Money unrealized_pnl(self, Price price)
    cpdef Money total_pnl(self, Price price)
    cpdef list commissions(self)

    cdef void _check_duplicate_trade_id(self, OrderFilled fill)
    cdef void _handle_buy_order_fill(self, OrderFilled fill)
    cdef void _handle_sell_order_fill(self, OrderFilled fill)
    cdef double _calculate_avg_px(self, double avg_px, double qty, double last_px, double last_qty)
    cdef double _calculate_avg_px_open_px(self, double last_px, double last_qty)
    cdef double _calculate_avg_px_close_px(self, double last_px, double last_qty)
    cdef double _calculate_points(self, double avg_px_open, double avg_px_close)
    cdef double _calculate_points_inverse(self, double avg_px_open, double avg_px_close)
    cdef double _calculate_return(self, double avg_px_open, double avg_px_close)
    cdef double _calculate_pnl(self, double avg_px_open, double avg_px_close, double quantity)
