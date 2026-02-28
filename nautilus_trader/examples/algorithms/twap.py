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
时间加权平均价格 (TWAP) 执行算法。

该算法将订单均匀分散在指定的时间范围内执行，通过生成较小的子订单来减少市场影响。
"""

import math
import random
from datetime import timedelta
from decimal import ROUND_DOWN, Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.enums import OrderSide, OrderType
from nautilus_trader.model.identifiers import ClientOrderId, ExecAlgorithmId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import LimitOrder, MarketOrder, Order


class TWAPExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    TWAP 执行算法配置类。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId, optional
        执行算法 ID（默认："TWAP"）。

    Notes
    -----
    此配置类定义了时间加权平均价格 (TWAP) 执行算法所需的参数。
    该算法旨在将订单在指定的时间范围内定期且均匀地分散执行。
    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("TWAP")


class TWAPExecAlgorithm(ExecAlgorithm):
    """
    时间加权平均价格 (TWAP) 执行算法。

    算法特点：
    - 将大订单均匀分散在指定时间范围内执行
    - 通过生成较小的子订单减少市场影响
    - 支持时间随机化（±20% 抖动）
    - 支持数量随机化（±5% 扰动）
    - 支持市场冲击保护（限制订单占市场深度的比例）

    Parameters
    ----------
    config : TWAPExecAlgorithmConfig, optional
        算法配置实例。

    Notes
    -----
    该算法将立即提交第一个订单，最后一个提交的订单将是周期结束时的原始主订单。
    """

    def __init__(self, config: TWAPExecAlgorithmConfig | None = None) -> None:
        """初始化 TWAP 执行算法。"""
        if config is None:
            config = TWAPExecAlgorithmConfig()
        super().__init__(config)

        # 订单调度状态
        self._scheduled_sizes: dict[ClientOrderId, list[Quantity]] = {}
        self._active_spawned_orders: dict[ClientOrderId, ClientOrderId] = {}
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}

        # 随机化控制参数
        self._base_intervals: dict[ClientOrderId, float] = {}
        self._max_market_impact_ratios: dict[ClientOrderId, float] = {}
        self._randomization_enabled: dict[ClientOrderId, bool] = {}

    def on_start(self) -> None:
        """算法启动时的回调。"""
        pass

    def on_stop(self) -> None:
        """算法停止时的回调。"""
        self.clock.cancel_timers()

    def on_reset(self) -> None:
        """算法重置时的回调。"""
        self._scheduled_sizes.clear()
        self._active_spawned_orders.clear()
        self._remaining_qty.clear()
        self._base_intervals.clear()
        self._max_market_impact_ratios.clear()
        self._randomization_enabled.clear()

    def on_save(self) -> dict[str, bytes]:
        """
        保存算法状态。

        Returns
        -------
        dict[str, bytes]
            算法状态字典。
        """
        return {}

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        加载算法状态。

        Parameters
        ----------
        state : dict[str, bytes]
            算法状态字典。
        """
        pass

    @staticmethod
    def round_decimal_down(amount: Decimal, precision: int) -> Decimal:
        """
        将 Decimal 值向下取整到指定精度。

        Parameters
        ----------
        amount : Decimal
            待取整的数值。
        precision : int
            精度（小数位数）。

        Returns
        -------
        Decimal
            向下取整后的数值。
        """
        return amount.quantize(Decimal(f"1e-{precision}"), rounding=ROUND_DOWN)

    @staticmethod
    def apply_time_randomization(base_interval_secs: float) -> timedelta:
        """
        应用时间随机化（±20% 抖动）。

        Parameters
        ----------
        base_interval_secs : float
            基础间隔时间（秒）。

        Returns
        -------
        timedelta
            随机化后的时间间隔。

        Notes
        -----
        随机化范围：[0.8, 1.2] × 基础间隔，最小值为 1 秒。
        """
        jitter = 0.8 + random.uniform(0.0, 0.4)  # 范围：0.8 ~ 1.2
        randomized_secs = base_interval_secs * jitter
        return timedelta(seconds=max(randomized_secs, 1.0))

    def apply_quantity_randomization(
        self,
        base_qty: Quantity,
        remaining_qty: Quantity,
        is_final: bool,
    ) -> Quantity:
        """
        应用数量随机化（±5% 扰动）。

        Parameters
        ----------
        base_qty : Quantity
            基础数量。
        remaining_qty : Quantity
            剩余待执行数量。
        is_final : bool
            是否为最后一笔订单。

        Returns
        -------
        Quantity
            随机化后的数量。

        Notes
        -----
        随机化范围：[0.95, 1.05] × 基础数量。
        最后一笔订单或随机化后数量不超过剩余数量。
        """
        if is_final:
            return remaining_qty

        randomization_factor = 0.95 + random.uniform(0.0, 0.10)  # 范围：0.95 ~ 1.05
        randomized_raw = float(base_qty.as_double()) * randomization_factor
        
        instrument = self.cache.instrument(base_qty.instrument_id)
        randomized_qty = instrument.make_qty(Decimal(str(randomized_raw)))
        
        return min(randomized_qty, remaining_qty)

    def check_market_impact_limit(
        self,
        instrument: Instrument,
        proposed_qty: Quantity,
        max_impact_ratio: float,
        order_side: OrderSide,
    ) -> Quantity:
        """
        检查并应用市场冲击限制。

        Parameters
        ----------
        instrument : Instrument
            合约信息。
        proposed_qty : Quantity
            提议的订单数量。
        max_impact_ratio : float
            最大市场冲击比例（0.0 ~ 1.0）。
        order_side : OrderSide
            订单方向。

        Returns
        -------
        Quantity
            经过市场冲击限制后的数量。

        Notes
        -----
        根据订单方向检查最佳反向报价的深度，
        限制订单数量不超过该深度的指定比例。
        """
        book = self.cache.order_book(instrument.id)
        if book is None:
            return proposed_qty

        # 获取最佳反向报价深度
        depth_qty = (
            book.best_ask_size() if order_side == OrderSide.BUY
            else book.best_bid_size()
        )
        if depth_qty is None:
            return proposed_qty

        # 计算允许的最大数量
        max_allowed = depth_qty.as_decimal() * Decimal(str(max_impact_ratio))
        limited_qty = min(proposed_qty, instrument.make_qty(max_allowed))
        
        # 确保不低于最小交易单位
        if limited_qty.as_decimal() < instrument.size_increment.as_decimal():
            limited_qty = instrument.make_qty(instrument.size_increment.as_decimal())
            
        return limited_qty

    def on_order(self, order: Order) -> None:
        """
        处理新订单。

        Parameters
        ----------
        order : Order
            待处理的订单。

        Notes
        -----
        系统方法，不应由用户代码直接调用。
        """
        PyCondition.not_in(
            order.client_order_id,
            self._scheduled_sizes,
            "order.client_order_id",
            "self._scheduled_sizes",
        )
        self.log.info(repr(order), LogColor.CYAN)

        # 验证订单类型
        if order.order_type not in (OrderType.MARKET, OrderType.LIMIT):
            self.log.error(
                f"无法执行订单：仅支持市价单和限价单，{order.order_type=}",
            )
            return

        # 获取合约信息
        instrument = self.cache.instrument(order.instrument_id)
        if not instrument:
            self.log.error(f"无法执行订单：未找到合约 {order.instrument_id}")
            return

        # 验证执行参数
        exec_params = order.exec_algorithm_params
        if not exec_params:
            self.log.error(
                f"无法执行订单：主订单 {order!r} 未设置 `exec_algorithm_params`",
            )
            return

        horizon_secs = exec_params.get("horizon_secs")
        if not horizon_secs:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` 中未找到 `horizon_secs`",
            )
            return

        interval_secs = exec_params.get("interval_secs")
        if not interval_secs:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` 中未找到 `interval_secs`",
            )
            return

        if horizon_secs < interval_secs:
            self.log.error(
                f"无法执行订单：{horizon_secs=} 小于 {interval_secs=}",
            )
            return

        # 获取可选参数
        max_market_impact_ratio = exec_params.get("max_market_impact_ratio", 0.1)
        randomization_enabled = exec_params.get("randomization_enabled", True)

        # 计算间隔数量和每份数量
        num_intervals: int = math.floor(horizon_secs / interval_secs)
        quotient = order.quantity.as_decimal() / num_intervals
        floored_quotient = self.round_decimal_down(quotient, instrument.size_precision)
        qty_quotient = instrument.make_qty(floored_quotient)
        qty_per_interval = instrument.make_qty(qty_quotient)
        qty_remainder = order.quantity.as_decimal() - (floored_quotient * num_intervals)

        # 如果无法拆分，直接提交整个订单
        if (
            qty_per_interval == order.quantity
            or qty_per_interval < instrument.size_increment
            or (instrument.min_quantity and qty_per_interval < instrument.min_quantity)
        ):
            self.log.warning(f"为全部规模提交订单 {qty_per_interval=}, {order.quantity=}")
            self.submit_order(order)
            return

        # 创建执行计划
        scheduled_sizes: list[Quantity] = [qty_per_interval] * num_intervals
        if qty_remainder:
            scheduled_sizes.append(instrument.make_qty(qty_remainder))

        assert sum(scheduled_sizes) == order.quantity
        self.log.info(f"订单执行规模计划：{scheduled_sizes}", LogColor.BLUE)

        # 初始化状态跟踪
        self._base_intervals[order.client_order_id] = interval_secs
        self._max_market_impact_ratios[order.client_order_id] = max_market_impact_ratio
        self._randomization_enabled[order.client_order_id] = randomization_enabled
        self._remaining_qty[order.client_order_id] = order.quantity.as_decimal()
        self._scheduled_sizes[order.client_order_id] = scheduled_sizes

        # 准备并发送首单
        first_qty: Quantity = scheduled_sizes.pop(0)
        
        # 应用市场冲击限制
        if max_market_impact_ratio > 0:
            first_qty = self.check_market_impact_limit(
                instrument, first_qty, max_market_impact_ratio, order.side
            )

        # 创建子订单
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

        # 设置定时器（应用时间随机化）
        timer_interval = (
            self.apply_time_randomization(interval_secs)
            if randomization_enabled
            else timedelta(seconds=interval_secs)
        )
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timer_interval,
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 {order.client_order_id} 的 TWAP 执行："
            f"{horizon_secs=}, {interval_secs=}",
            LogColor.BLUE,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        """
        处理定时事件。

        Parameters
        ----------
        event : TimeEvent
            定时事件。
        """
        self.log.info(repr(event), LogColor.CYAN)

        exec_spawn_id = ClientOrderId(event.name)

        # 获取主订单
        primary: Order = self.cache.order(exec_spawn_id)
        if not primary:
            self.log.error(f"无法找到 {exec_spawn_id=} 的主订单")
            return

        if primary.is_closed:
            self.complete_sequence(primary.client_order_id)
            return

        # 获取合约信息
        instrument: Instrument = self.cache.instrument(primary.instrument_id)
        if not instrument:
            self.log.error(f"无法执行订单：未找到合约 {primary.instrument_id}")
            return

        # 获取执行计划
        scheduled_sizes = self._scheduled_sizes.get(exec_spawn_id)
        if scheduled_sizes is None:
            self.log.error(f"无法找到 {exec_spawn_id=} 的执行计划")
            return

        # 撤销上一时间片未成交的订单
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

        # 获取基准数量并应用随机化
        base_qty: Quantity = scheduled_sizes.pop(0)
        is_final = len(scheduled_sizes) == 0
        
        randomization_enabled = self._randomization_enabled.get(exec_spawn_id, True)
        if randomization_enabled:
            remaining = self._remaining_qty.get(exec_spawn_id, base_qty.as_decimal())
            remaining_qty = instrument.make_qty(remaining)
            quantity = self.apply_quantity_randomization(base_qty, remaining_qty, is_final)
        else:
            quantity = base_qty
        
        # 应用市场冲击限制
        max_impact_ratio = self._max_market_impact_ratios.get(exec_spawn_id, 0.1)
        if max_impact_ratio > 0:
            quantity = self.check_market_impact_limit(
                instrument, quantity, max_impact_ratio, primary.side
            )

        # 如果是最后一份，提交主订单
        if not scheduled_sizes:
            if primary.order_type == OrderType.LIMIT:
                quote = self.cache.quote_tick(instrument.id)
                if quote:
                    price = quote.bid_price if primary.side == OrderSide.BUY else quote.ask_price
                    self.modify_order_in_place(primary, price=price)
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        # 创建并提交子订单
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

        # 重置定时器（应用时间随机化）
        if randomization_enabled and scheduled_sizes:
            base_interval = self._base_intervals.get(exec_spawn_id, 60.0)
            timer_interval = self.apply_time_randomization(base_interval)
            self.clock.set_timer(
                name=exec_spawn_id.value,
                interval=timer_interval,
                callback=self.on_time_event,
            )

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        完成执行序列并清理状态。

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            执行序列的客户端订单 ID。
        """
        if exec_spawn_id.value in self.clock.timer_names:
            self.clock.cancel_timer(exec_spawn_id.value)
        
        self._scheduled_sizes.pop(exec_spawn_id, None)
        self._active_spawned_orders.pop(exec_spawn_id, None)
        self._base_intervals.pop(exec_spawn_id, None)
        self._max_market_impact_ratios.pop(exec_spawn_id, None)
        self._randomization_enabled.pop(exec_spawn_id, None)
        self._remaining_qty.pop(exec_spawn_id, None)
        
        self.log.info(f"已完成 {exec_spawn_id} 的 TWAP 执行", LogColor.BLUE)
