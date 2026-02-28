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

import math
from datetime import timedelta
from decimal import ROUND_DOWN
from decimal import Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import LimitOrder
from nautilus_trader.model.orders import MarketOrder
from nautilus_trader.model.orders import Order


class TWAPExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    TWAPExecAlgorithm 实例的配置。

    此配置类定义了时间加权平均价格 (TWAP) 执行算法所需的参数。该算法旨在将订单在指定的时间范围内定期且均匀地分散执行。

    参数
    ----------
    exec_algorithm_id : ExecAlgorithmId
        执行算法 ID（将覆盖默认的类名）。

    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("TWAP")


class TWAPExecAlgorithm(ExecAlgorithm):
    """
    提供时间加权平均价格 (TWAP) 执行算法。

    TWAP 执行算法旨在通过将订单均匀分散在指定的时间范围内来执行。该算法接收一个代表总规模和方向的主订单，然后通过生成较小的子订单来对其进行拆分，这些子订单随后在整个时间范围内的固定间隔执行。

    这有助于通过最小化任何给定时间的交易规模集中度，来减少主订单全量对市场的影响。

    该算法将立即提交第一个订单，而最后一个提交的订单将是周期结束时的原始主订单。

    参数
    ----------
    config : TWAPExecAlgorithmConfig, 可选
        该实例的配置。

    """

    def __init__(self, config: TWAPExecAlgorithmConfig | None = None) -> None:
        if config is None:
            config = TWAPExecAlgorithmConfig()
        super().__init__(config)

        self._scheduled_sizes: dict[ClientOrderId, list[Quantity]] = {}
        self._active_spawned_orders: dict[ClientOrderId, ClientOrderId] = {}

    def on_start(self) -> None:
        """
        算法组件启动时执行的操作。
        """
        # 可选实现

    def on_stop(self) -> None:
        """
        算法组件停止时执行的操作。
        """
        self.clock.cancel_timers()

    def on_reset(self) -> None:
        """
        算法组件重置时执行的操作。
        """
        self._scheduled_sizes.clear()
        self._active_spawned_orders.clear()

    def on_save(self) -> dict[str, bytes]:
        """
        算法组件保存时执行的操作。

        创建并返回要保存的状态字典。

        返回
        -------
        dict[str, bytes]
            策略状态字典。

        """
        return {}  # 可选实现

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        算法组件加载时执行的操作。

        保存的状态值将包含在给定的状态字典中。

        参数
        ----------
        state : dict[str, bytes]
            算法组件状态字典。

        """
        # 可选实现

    def round_decimal_down(self, amount: Decimal, precision: int) -> Decimal:
        """将 Decimal 值向下取整到指定精度。"""
        return amount.quantize(Decimal(f"1e-{precision}"), rounding=ROUND_DOWN)

    def on_order(self, order: Order) -> None:
        """
        运行时接收到订单时执行的操作。

        参数
        ----------
        order : Order
            待处理的订单。

        警告
        --------
        系统方法（不应由用户代码调用）。

        """
        PyCondition.not_in(
            order.client_order_id,
            self._scheduled_sizes,
            "order.client_order_id",
            "self._scheduled_sizes",
        )
        self.log.info(repr(order), LogColor.CYAN)

        if order.order_type not in (OrderType.MARKET, OrderType.LIMIT):
            self.log.error(
                f"无法执行订单：仅支持市价单和限价单，{order.order_type=}",
            )
            return

        instrument = self.cache.instrument(order.instrument_id)
        if not instrument:
            self.log.error(
                f"无法执行订单：未找到合约 {order.instrument_id}",
            )
            return

        # 校验执行参数
        exec_params = order.exec_algorithm_params
        if not exec_params:
            self.log.error(
                f"无法执行订单：主订单 {order!r} 未找到 `exec_algorithm_params`",
            )
            return

        horizon_secs = exec_params.get("horizon_secs")
        if not horizon_secs:
            self.log.error(
                f"无法执行订单：在 `exec_algorithm_params` {exec_params} 中未找到 `horizon_secs`",
            )
            return

        interval_secs = exec_params.get("interval_secs")
        if not interval_secs:
            self.log.error(
                f"无法执行订单：在 `exec_algorithm_params` {exec_params} 中未找到 `interval_secs`",
            )
            return

        if horizon_secs < interval_secs:
            self.log.error(
                f"无法执行订单：{horizon_secs=} 小于 {interval_secs=}",
            )
            return

        # 计算间隔数量
        num_intervals: int = math.floor(horizon_secs / interval_secs)

        # 均匀分配订单数量并确定余量
        quotient = order.quantity.as_decimal() / num_intervals
        floored_quotient = self.round_decimal_down(quotient, instrument.size_precision)
        qty_quotient = instrument.make_qty(floored_quotient)
        qty_per_interval = instrument.make_qty(qty_quotient)
        qty_remainder = order.quantity.as_decimal() - (floored_quotient * num_intervals)

        if (
            qty_per_interval == order.quantity
            or qty_per_interval < instrument.size_increment
            or (instrument.min_quantity and qty_per_interval < instrument.min_quantity)
        ):
            # 立即提交全部规模的第一笔订单
            self.log.warning(f"为全部规模提交订单 {qty_per_interval=}, {order.quantity=}")
            self.submit_order(order)
            return  # 完成

        scheduled_sizes: list[Quantity] = [qty_per_interval] * num_intervals
        if qty_remainder:
            scheduled_sizes.append(instrument.make_qty(qty_remainder))

        assert sum(scheduled_sizes) == order.quantity
        self.log.info(f"订单执行规模计划：{scheduled_sizes}", LogColor.BLUE)

        self._scheduled_sizes[order.client_order_id] = scheduled_sizes
        first_qty: Quantity = scheduled_sizes.pop(0)

        spawned_order = None
        if order.order_type == OrderType.MARKET:
            spawned_order = self.spawn_market(
                primary=order,
                quantity=first_qty,
                time_in_force=order.time_in_force,
                reduce_only=order.is_reduce_only,
                tags=order.tags,
            )
        else:
            quote = self.cache.quote_tick(instrument.id)
            if not quote:
                self.log.error(f"无法执行首笔限价单：未找到合约 {instrument.id} 的行情")
                return
            price = quote.bid_price if order.side == OrderSide.BUY else quote.ask_price
            spawned_order = self.spawn_limit(
                primary=order,
                quantity=first_qty,
                price=price,
                time_in_force=order.time_in_force,
                reduce_only=order.is_reduce_only,
                tags=order.tags,
            )

        self._active_spawned_orders[order.client_order_id] = spawned_order.client_order_id
        self.submit_order(spawned_order)

        # 设置定时器
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timedelta(seconds=interval_secs),
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 {order.client_order_id} 的 TWAP 执行："
            f"{horizon_secs=}, {interval_secs=}",
            LogColor.BLUE,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        """
        算法接收到时间事件时执行的操作。

        参数
        ----------
        event : TimeEvent
            接收到的时间事件。

        """
        self.log.info(repr(event), LogColor.CYAN)

        exec_spawn_id = ClientOrderId(event.name)

        primary: Order = self.cache.order(exec_spawn_id)
        if not primary:
            self.log.error(f"无法找到 {exec_spawn_id=} 的主订单")
            return

        if primary.is_closed:
            self.complete_sequence(primary.client_order_id)
            return

        instrument: Instrument = self.cache.instrument(primary.instrument_id)
        if not instrument:
            self.log.error(
                f"无法执行订单：未找到合约 {primary.instrument_id}",
            )
            return

        scheduled_sizes = self._scheduled_sizes.get(exec_spawn_id)
        if scheduled_sizes is None:
            self.log.error(f"无法找到 {exec_spawn_id=} 的计划规模")
            return

        # 时间片结束，主动撤销未成交的上一笔限价单
        last_spawned_id = self._active_spawned_orders.get(exec_spawn_id)
        if last_spawned_id:
            last_order = self.cache.order(last_spawned_id)
            if last_order and not last_order.is_closed:
                self.log.info(f"时间片结束，主动撤销未成交订单：{last_spawned_id}")
                self.cancel_order(last_order)
            self._active_spawned_orders.pop(exec_spawn_id, None)

        if not scheduled_sizes:
            self.log.warning(f"{exec_spawn_id=} 没有更多的规模可执行")
            return

        quantity: Quantity = instrument.make_qty(scheduled_sizes.pop(0))
        if not scheduled_sizes:  # 最后一份数量
            if primary.order_type == OrderType.LIMIT:
                quote = self.cache.quote_tick(instrument.id)
                if quote:
                    price = quote.bid_price if primary.side == OrderSide.BUY else quote.ask_price
                    self.modify_order_in_place(primary, price=price)
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        spawned_order = None
        if primary.order_type == OrderType.MARKET:
            spawned_order = self.spawn_market(
                primary=primary,
                quantity=quantity,
                time_in_force=primary.time_in_force,
                reduce_only=primary.is_reduce_only,
                tags=primary.tags,
            )
        else:
            quote = self.cache.quote_tick(instrument.id)
            if not quote:
                self.log.error(f"无法执行限价单：未找到合约 {instrument.id} 的行情")
                return
            price = quote.bid_price if primary.side == OrderSide.BUY else quote.ask_price
            spawned_order = self.spawn_limit(
                primary=primary,
                quantity=quantity,
                price=price,
                time_in_force=primary.time_in_force,
                reduce_only=primary.is_reduce_only,
                tags=primary.tags,
            )

        self._active_spawned_orders[exec_spawn_id] = spawned_order.client_order_id
        self.submit_order(spawned_order)

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        完成一个执行序列。

        参数
        ----------
        exec_spawn_id : ClientOrderId
            要完成的执行生成 ID。

        """
        if exec_spawn_id.value in self.clock.timer_names:
            self.clock.cancel_timer(exec_spawn_id.value)
        self._scheduled_sizes.pop(exec_spawn_id, None)
        self._active_spawned_orders.pop(exec_spawn_id, None)
        self.log.info(f"已完成 {exec_spawn_id} 的 TWAP 执行", LogColor.BLUE)
