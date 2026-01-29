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
"""
实盘交易的对账函数。
"""

from decimal import Decimal

from nautilus_trader.cache.transformers import transform_instrument_to_pyo3
from nautilus_trader.common.component import Logger
from nautilus_trader.core import nautilus_pyo3
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.execution.reports import ExecutionMassStatus
from nautilus_trader.execution.reports import FillReport
from nautilus_trader.execution.reports import OrderStatusReport
from nautilus_trader.model.currencies import register_currency
from nautilus_trader.model.enums import LiquiditySide
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.events import OrderAccepted
from nautilus_trader.model.events import OrderCanceled
from nautilus_trader.model.events import OrderExpired
from nautilus_trader.model.events import OrderFilled
from nautilus_trader.model.events import OrderRejected
from nautilus_trader.model.events import OrderTriggered
from nautilus_trader.model.events import OrderUpdated
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import PositionId
from nautilus_trader.model.identifiers import TradeId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import Order


def is_within_single_unit_tolerance(
    value1: Decimal,
    value2: Decimal,
    precision: int,
) -> bool:
    """
    检查两个十进制数值是否在基于精度的单一单位容差范围内。

    处理来自交易场的舍入差异（例如，OKX 的 fillSz 与 accFillSz）。

    参数
    ----------
    value1 : Decimal
        要比较的第一个值。
    value2 : Decimal
        要比较的第二个值。
    precision : int
        用于计算容差的十进制精度。

    返回
    -------
    bool

    """
    # 仅对小数数量应用容忍度（精度 > 0）
    if precision == 0:
        return value1 == value2  # 整数数量要求完全匹配

    tolerance = Decimal(10) ** -precision

    return abs(value1 - value2) <= tolerance


def get_existing_fill_for_trade_id(
    order: Order,
    trade_id: TradeId,
) -> OrderFilled | None:
    """
    在订单的事件历史中查找特定成交 ID 的现有成交事件。

    参数
    ----------
    order : Order
        要搜索的订单。
    trade_id : TradeId
        要查找的成交 ID。

    返回
    -------
    OrderFilled 或 ``None``

    """
    for event in order.events:
        if isinstance(event, OrderFilled) and event.trade_id == trade_id:
            return event

    return None


def create_order_rejected_event(
    order: Order,
    ts_now: int,
    report: OrderStatusReport | None = None,
    reason: str | None = None,
) -> OrderRejected:
    """
    为对账创建一个 `OrderRejected`（订单被拒绝）事件。

    此函数统一了不同对账路径（带报告的启动对账，不带报告的持续对账）下
    `OrderRejected` 事件的创建。

    参数
    ----------
    order : Order
        要创建拒绝事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport, 可选
        来自场地的订单状态报告（如果有）。
    reason : str, 可选
        拒绝原因（在没有报告时使用）。

    返回
    -------
    OrderRejected

    """
    if report:
        # 当报告可用时使用其数据（启动对账）
        return OrderRejected(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            account_id=report.account_id,
            reason=report.cancel_reason or reason or "UNKNOWN",
            event_id=UUID4(),
            ts_event=report.ts_last,
            ts_init=ts_now,
            reconciliation=True,
        )
    else:
        # 使用当前时间戳和提供的拒绝原因（持续对账）
        return OrderRejected(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            account_id=order.account_id,
            reason=reason or "UNKNOWN",
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
            reconciliation=True,
        )


def create_order_canceled_event(
    order: Order,
    ts_now: int,
    report: OrderStatusReport | None = None,
) -> OrderCanceled:
    """
    为对账创建一个 `OrderCanceled`（订单已取消）事件。

    此函数统一了不同对账路径（带报告的启动对账，不带报告的持续对账）下
    `OrderCanceled` 事件的创建。

    参数
    ----------
    order : Order
        要创建取消事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport, 可选
        来自场地的订单状态报告（如果有）。

    返回
    -------
    OrderCanceled

    """
    if report:
        # 当报告可用时使用其数据（启动对账）
        return OrderCanceled(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=report.instrument_id,
            client_order_id=report.client_order_id,
            venue_order_id=report.venue_order_id,
            account_id=report.account_id,
            event_id=UUID4(),
            ts_event=report.ts_last,
            ts_init=ts_now,
            reconciliation=True,
        )
    else:
        # 使用当前时间戳（持续对账）
        return OrderCanceled(
            trader_id=order.trader_id,
            strategy_id=order.strategy_id,
            instrument_id=order.instrument_id,
            client_order_id=order.client_order_id,
            venue_order_id=order.venue_order_id,
            account_id=order.account_id,
            event_id=UUID4(),
            ts_event=ts_now,
            ts_init=ts_now,
            reconciliation=True,
        )


def create_order_expired_event(
    order: Order,
    ts_now: int,
    report: OrderStatusReport,
) -> OrderExpired:
    """
    为对账创建一个 `OrderExpired`（订单已过期）事件。

    参数
    ----------
    order : Order
        要创建过期事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport
        来自场地的订单状态报告。

    返回
    -------
    OrderExpired

    """
    return OrderExpired(
        trader_id=order.trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=report.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        event_id=UUID4(),
        ts_event=report.ts_last,
        ts_init=ts_now,
        reconciliation=True,
    )


def create_order_accepted_event(
    trader_id: TraderId,
    order: Order,
    ts_now: int,
    report: OrderStatusReport,
) -> OrderAccepted:
    """
    为对账创建一个 `OrderAccepted`（订单已受理）事件。

    参数
    ----------
    trader_id : TraderId
        订单的交易员 ID。
    order : Order
        要创建受理事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport
        来自场地的订单状态报告。

    返回
    -------
    OrderAccepted

    """
    return OrderAccepted(
        trader_id=trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=report.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        event_id=UUID4(),
        ts_event=report.ts_accepted,
        ts_init=ts_now,
        reconciliation=True,
    )


def create_order_triggered_event(
    trader_id: TraderId,
    order: Order,
    ts_now: int,
    report: OrderStatusReport,
) -> OrderTriggered:
    """
    为对账创建一个 `OrderTriggered`（订单已触发）事件。

    参数
    ----------
    trader_id : TraderId
        订单的交易员 ID。
    order : Order
        要创建触发事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport
        来自场地的订单状态报告。

    返回
    -------
    OrderTriggered

    """
    return OrderTriggered(
        trader_id=trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=report.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        event_id=UUID4(),
        ts_event=report.ts_triggered,
        ts_init=ts_now,
        reconciliation=True,
    )


def create_order_updated_event(
    trader_id: TraderId,
    order: Order,
    ts_now: int,
    report: OrderStatusReport,
) -> OrderUpdated:
    """
    为对账创建一个 `OrderUpdated`（订单已更新）事件。

    参数
    ----------
    trader_id : TraderId
        订单的交易员 ID。
    order : Order
        要创建更新事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport
        来自场地的订单状态报告。

    返回
    -------
    OrderUpdated

    """
    return OrderUpdated(
        trader_id=trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=report.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        quantity=report.quantity,
        price=report.price,
        trigger_price=report.trigger_price,
        event_id=UUID4(),
        ts_event=report.ts_last,
        ts_init=ts_now,
        reconciliation=True,
    )


def create_order_filled_event(
    order: Order,
    ts_now: int,
    report: FillReport,
    instrument: Instrument,
) -> OrderFilled:
    """
    为对账创建一个 `OrderFilled`（订单已成交）事件。

    参数
    ----------
    order : Order
        要创建成交事件的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : FillReport
        来自场地的成交报告。
    instrument : Instrument
        订单对应的交易标的。

    返回
    -------
    OrderFilled

    """
    return OrderFilled(
        trader_id=order.trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=order.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        trade_id=report.trade_id,
        position_id=report.venue_position_id,
        order_side=order.side,
        order_type=order.order_type,
        last_qty=report.last_qty,
        last_px=report.last_px,
        currency=instrument.quote_currency,
        commission=report.commission,
        liquidity_side=report.liquidity_side,
        event_id=UUID4(),
        ts_event=report.ts_event,
        ts_init=ts_now,
        reconciliation=True,
    )


def create_inferred_order_filled_event(
    order: Order,
    ts_now: int,
    report: OrderStatusReport,
    instrument: Instrument,
) -> OrderFilled:
    """
    为对账创建一个推断出的 `OrderFilled`（订单已成交）事件。

    此函数在成交详情缺失但可以通过显示已成交数量的订单状态报告进行推断时使用。

    参数
    ----------
    order : Order
        要为其创建推断成交的订单。
    ts_now : int
        以纳秒为单位的当前时间戳。
    report : OrderStatusReport
        显示已成交数量的订单状态报告。
    instrument : Instrument
        订单对应的交易标的。

    返回
    -------
    OrderFilled

    """
    # 推断流动性方向
    liquidity_side: LiquiditySide = LiquiditySide.NO_LIQUIDITY_SIDE

    if order.order_type in (
        OrderType.MARKET,
        OrderType.STOP_MARKET,
        OrderType.TRAILING_STOP_MARKET,
    ):
        liquidity_side = LiquiditySide.TAKER
    elif report.post_only:
        liquidity_side = LiquiditySide.MAKER

    # 计算上一次成交数量
    last_qty: Quantity = instrument.make_qty(report.filled_qty - order.filled_qty)

    # 计算上一次成交价格
    if order.avg_px is None:
        # 对于首笔成交，使用报告的平均成交价
        if report.avg_px:
            last_px: Price = instrument.make_price(report.avg_px)
        elif report.price is not None:
            # 如果没有平均成交价但有价格（例如来自限价单），则使用该价格
            last_px = report.price
        else:
            # 目前保留原始备选方案
            last_px = instrument.make_price(0.0)
    else:
        report_cost: float = float(report.avg_px or 0.0) * float(report.filled_qty)
        filled_cost = float(order.avg_px) * float(order.filled_qty)
        incremental_cost = report_cost - filled_cost

        if float(last_qty) > 0:
            last_px = instrument.make_price(incremental_cost / float(last_qty))
        else:
            last_px = instrument.make_price(report.avg_px)

    notional_value: Money = instrument.notional_value(last_qty, last_px)
    commission: Money = Money(notional_value * instrument.taker_fee, instrument.quote_currency)

    return OrderFilled(
        trader_id=order.trader_id,
        strategy_id=order.strategy_id,
        instrument_id=report.instrument_id,
        client_order_id=order.client_order_id,
        venue_order_id=report.venue_order_id,
        account_id=report.account_id,
        position_id=report.venue_position_id or PositionId(f"{instrument.id}-EXTERNAL"),
        trade_id=TradeId(UUID4().value),
        order_side=order.side,
        order_type=order.order_type,
        last_qty=last_qty,
        last_px=last_px,
        currency=instrument.quote_currency,
        commission=commission,
        liquidity_side=liquidity_side,
        event_id=UUID4(),
        ts_event=report.ts_last,
        ts_init=ts_now,
        reconciliation=True,
    )


def calculate_reconciliation_price(
    current_position_qty: Decimal,
    current_position_avg_px: Decimal | None,
    target_position_qty: Decimal,
    target_position_avg_px: Decimal | None,
    instrument: Instrument,
) -> Price | None:
    """
    计算对账订单所需的价格以达到目标持仓。

    这是一个纯函数，用于计算成交需要什么价格才能从当前持仓状态
    以正确的平均价格移动到目标持仓状态，同时考虑净额结算模拟逻辑。

    参数
    ----------
    current_position_qty : Decimal
        当前的带符号持仓数量（正数表示多头，负数表示空头）。
    current_position_avg_px : Decimal, optional
        当前的持仓平均价格（平仓时可为 None）。
    target_position_qty : Decimal
        目标带符号持仓数量。
    target_position_avg_px : Decimal, optional
        目标持仓平均价格。
    instrument : Instrument
        用于价格精度的交易标的。

    返回
    -------
    Price 或 ``None``

    Notes
    -----
    该函数处理三种情况：
    1. 平仓到持仓：reconciliation_px = target_avg_px
    2. 持仓反转（符号改变）：reconciliation_px = target_avg_px（由于模拟中值重置）
    3. 累积/减少：加权平均公式

    """
    result = nautilus_pyo3.calculate_reconciliation_price(
        current_position_qty,
        current_position_avg_px,
        target_position_qty,
        target_position_avg_px,
    )

    if result is None:
        return None

    return instrument.make_price(result)


def adjust_fills_for_partial_window_single(
    mass_status: ExecutionMassStatus,
    instrument: Instrument,
    logger: Logger | None = None,
) -> tuple[dict[VenueOrderId, OrderStatusReport], dict[VenueOrderId, list[FillReport]]]:
    """
    调整成交以说明窗口开始时持仓生命周期的不完整逻辑。
    """
    return adjust_fills_for_partial_window(mass_status, [instrument], logger)[instrument.id]


def adjust_fills_for_partial_window(
    mass_status: ExecutionMassStatus,
    instruments: list[Instrument],
    logger: Logger | None = None,
) -> dict[
    InstrumentId,
    tuple[dict[VenueOrderId, OrderStatusReport], dict[VenueOrderId, list[FillReport]]],
]:
    """
    调整成交以说明窗口开始时持仓生命周期的不完整逻辑。

    此函数分析回溯窗口（lookback window）内的成交报告，并对其进行调整，
    以确保模拟持仓与场地报告的持仓相匹配，考虑到以下场景：
    - 持仓生命周期在回溯窗口之前就开始了
    - 发生了多次持仓生命周期（伴随跨零点）
    - 来自旧生命周期的成交报告应被排除

    参数
    ----------
    mass_status : ExecutionMassStatus
        包含订单、成交和持仓报告的执行总量状态（mass status）。
    instruments : list[Instrument]
        需要调整成交的交易标的（总量状态中的所有标的）。
    logger : Logger, 可选
        用于诊断输出的日志记录器。

    返回
    -------
    tuple[dict[VenueOrderId, OrderStatusReport], dict[VenueOrderId, list[FillReport]]]
        由（调整后的订单报告, 调整后的成交报告）组成的元组，与交易场持仓匹配。

    """
    # 注册所有要求的手续费货币
    seen_currencies: set[Currency] = set()
    for fill_list in mass_status.fill_reports.values():
        for fill in fill_list:
            currency = fill.commission.currency
            if currency not in seen_currencies:
                register_currency(currency)
                if logger:
                    logger.debug(f"已注册货币：{currency}")
                seen_currencies.add(currency)

    pyo3_mass_status = mass_status.to_pyo3()

    pyo3_instruments = [transform_instrument_to_pyo3(instrument) for instrument in instruments]
    results: dict[
        InstrumentId,
        tuple[dict[VenueOrderId, OrderStatusReport], dict[VenueOrderId, list[FillReport]]],
    ] = {}

    for instrument, pyo3_instrument in zip(instruments, pyo3_instruments, strict=False):
        assert instrument.id.value == pyo3_instrument.id.value
        pyo3_orders, pyo3_fills = nautilus_pyo3.adjust_fills_for_partial_window(
            pyo3_mass_status,
            pyo3_instrument,
        )
        orders: dict[VenueOrderId, OrderStatusReport] = {}

        for venue_order_id_str, pyo3_order in pyo3_orders.items():
            venue_order_id = VenueOrderId(venue_order_id_str)
            order = OrderStatusReport.from_pyo3(pyo3_order)
            orders[venue_order_id] = order

        fills: dict[VenueOrderId, list[FillReport]] = {}

        for venue_order_id_str, pyo3_reports in pyo3_fills.items():
            venue_order_id = VenueOrderId(venue_order_id_str)
            reports = []

            for pyo3_report in pyo3_reports:
                report = FillReport.from_pyo3(pyo3_report)
                reports.append(report)

            fills[venue_order_id] = reports

            if logger:
                logger.debug(
                    f"已为 {instrument.id} 调整成交：{len(orders)} 个订单，{len(fills)} 个成交",
                )

        results[instrument.id] = (orders, fills)

    return results
