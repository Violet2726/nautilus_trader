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
参与比例 (POV) 执行算法。

该算法按照市场成交量的固定百分比来执行订单，通过订阅逐笔成交数据实时跟踪市场成交量，
并按指定的目标参与率生成子订单，确保算法执行速率与市场活跃度保持同步。
"""

import random
from datetime import timedelta
from decimal import ROUND_DOWN, Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import OrderSide, OrderType
from nautilus_trader.model.identifiers import ClientOrderId, ExecAlgorithmId, InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import LimitOrder, MarketOrder, Order


class POVExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    POV 执行算法配置类。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId, optional
        执行算法 ID（默认："POV"）。

    Notes
    -----
    此配置类定义了参与比例 (POV) 执行算法所需的参数。
    该算法旨在按照市场成交量的固定百分比来提交子订单。
    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("POV")


class POVExecAlgorithm(ExecAlgorithm):
    """
    参与比例 (Percentage of Volume, POV) 执行算法。

    算法特点：
    - 被动跟随：根据实际市场成交量动态调整执行节奏
    - 风险控制：内置随机化、市场深度限制等保护机制
    - 灵活配置：支持多种可调参数以适应不同市场环境

    算法工作流程：
    1. 接收主订单后订阅对应合约的逐笔成交数据
    2. 按固定间隔触发定时器，累积该间隔内的市场成交量
    3. 将累积成交量乘以目标参与率作为本次子订单数量
    4. 应用各种风险控制措施后提交子订单
    5. 重复直到完成全部执行或达到最大执行时间

    Parameters
    ----------
    config : POVExecAlgorithmConfig, optional
        算法配置实例。

    Notes
    -----
    与 TWAP/VWAP 不同，POV 是一种被动跟随型算法：
    - TWAP 按固定时间均匀分配
    - VWAP 按历史/实时成交量分布分配
    - POV 则严格按照市场成交量的固定百分比跟随执行
    """

    def __init__(self, config: POVExecAlgorithmConfig | None = None) -> None:
        """
        初始化 POV 执行算法。

        Parameters
        ----------
        config : POVExecAlgorithmConfig, optional
            算法配置实例。
        """
        if config is None:
            config = POVExecAlgorithmConfig()
        super().__init__(config)

        # 订单跟踪状态
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}
        self._interval_volume: dict[ClientOrderId, Decimal] = {}
        self._pov_rates: dict[ClientOrderId, Decimal] = {}
        self._order_instrument_ids: dict[ClientOrderId, InstrumentId] = {}
        self._max_horizon_secs: dict[ClientOrderId, float] = {}
        self._start_time_ns: dict[ClientOrderId, int] = {}
        self._max_slice_qty: dict[ClientOrderId, Decimal] = {}
        self._active_spawned_orders: dict[ClientOrderId, ClientOrderId] = {}
        self._subscribed_instruments: set[InstrumentId] = set()

        # 随机化和市场深度控制参数
        self._randomization_enabled: dict[ClientOrderId, bool] = {}
        self._max_display_ratios: dict[ClientOrderId, float] = {}
        self._base_intervals: dict[ClientOrderId, float] = {}

    def on_start(self) -> None:
        """算法启动时的回调。"""
        pass

    def on_stop(self) -> None:
        """算法停止时的回调。"""
        self.clock.cancel_timers()

    def on_reset(self) -> None:
        """算法重置时的回调。"""
        self._remaining_qty.clear()
        self._interval_volume.clear()
        self._pov_rates.clear()
        self._order_instrument_ids.clear()
        self._max_horizon_secs.clear()
        self._start_time_ns.clear()
        self._max_slice_qty.clear()
        self._active_spawned_orders.clear()
        self._subscribed_instruments.clear()
        self._randomization_enabled.clear()
        self._max_display_ratios.clear()
        self._base_intervals.clear()

    def on_save(self) -> dict[str, bytes]:
        """
        算法保存时的回调。

        Returns
        -------
        dict[str, bytes]
            策略状态字典。
        """
        return {}

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        算法加载时的回调。

        Parameters
        ----------
        state : dict[str, bytes]
            策略状态字典。
        """
        pass

    def round_decimal_down(self, amount: Decimal, precision: int) -> Decimal:
        """
        将 Decimal 值向下取整到指定精度。

        Parameters
        ----------
        amount : Decimal
            要取整的数值。
        precision : int
            精度（小数位数）。

        Returns
        -------
        Decimal
            向下取整后的结果。
        """
        return amount.quantize(Decimal(f"1e-{precision}"), rounding=ROUND_DOWN)

    def apply_time_randomization(self, base_interval_secs: float) -> timedelta:
        """
        应用时间随机性（±20% 抖动）。

        Parameters
        ----------
        base_interval_secs : float
            基础时间间隔（秒）。

        Returns
        -------
        timedelta
            随机化后的时间间隔。
        """
        jitter = 0.8 + random.uniform(0.0, 0.4)  # 0.8 to 1.2
        randomized_secs = base_interval_secs * jitter
        return timedelta(seconds=max(randomized_secs, 1.0))

    def apply_quantity_randomization(
        self,
        base_qty: Quantity,
        remaining_qty: Quantity,
        is_final: bool,
    ) -> Quantity:
        """
        应用数量随机性（±5% 扰动）。

        Parameters
        ----------
        base_qty : Quantity
            基准数量。
        remaining_qty : Quantity
            剩余数量。
        is_final : bool
            是否为最后一个切片。

        Returns
        -------
        Quantity
            随机化后的数量。
        """
        if is_final:
            return remaining_qty

        randomization_factor = 0.95 + random.uniform(0.0, 0.10)  # 0.95 to 1.05
        randomized_raw = float(base_qty.as_double()) * randomization_factor
        instrument = self.cache.instrument(base_qty.instrument_id)
        randomized_qty = instrument.make_qty(Decimal(str(randomized_raw)))
        
        # 确保数量在合理范围内
        return min(randomized_qty, remaining_qty)

    def check_market_depth_limit(
        self,
        instrument: Instrument,
        proposed_qty: Quantity,
        max_display_ratio: float,
        order_side: OrderSide,
    ) -> Quantity:
        """
        检查市场深度限制。

        Parameters
        ----------
        instrument : Instrument
            合约信息。
        proposed_qty : Quantity
            提议的订单数量。
        max_display_ratio : float
            最大显示比例（相对于市场深度）。
        order_side : OrderSide
            订单方向。

        Returns
        -------
        Quantity
            经过市场深度限制后的数量。
        """
        book = self.cache.order_book(instrument.id)
        if book is None:
            return proposed_qty

        # 获取最佳反向报价数量
        depth_qty = book.best_ask_size() if order_side == OrderSide.BUY else book.best_bid_size()
        if depth_qty is None:
            return proposed_qty

        max_allowed = depth_qty.as_decimal() * Decimal(str(max_display_ratio))
        limited_qty = min(proposed_qty, instrument.make_qty(max_allowed))
        
        # 确保至少有 1 单位的量
        if limited_qty.as_decimal() < instrument.size_increment.as_decimal():
            limited_qty = instrument.make_qty(instrument.size_increment.as_decimal())
            
        return limited_qty

    def on_order(self, order: Order) -> None:
        """
        接收到订单时的回调。

        Parameters
        ----------
        order : Order
            待处理的订单。
        """
        # 确保该订单尚未被调度
        PyCondition.not_in(
            order.client_order_id,
            self._remaining_qty,
            "order.client_order_id",
            "self._remaining_qty",
        )
        self.log.info(repr(order), LogColor.CYAN)

        # 支持市价单和限价单
        if order.order_type not in (OrderType.MARKET, OrderType.LIMIT):
            self.log.error(
                f"无法执行订单：仅支持市价单和限价单，当前类型为 {order.order_type=}",
            )
            return

        # 获取合约信息
        instrument = self.cache.instrument(order.instrument_id)
        if not instrument:
            self.log.error(
                f"无法执行订单：未找到合约 {order.instrument_id}",
            )
            return

        # 验证执行参数
        exec_params = order.exec_algorithm_params
        if not exec_params:
            self.log.error(
                f"无法执行订单：主订单 {order!r} 未设置 `exec_algorithm_params`",
            )
            return

        # 获取目标参与率 (0 < pov_rate <= 1)
        pov_rate = exec_params.get("pov_rate")
        if not pov_rate:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` {exec_params} 中未找到 `pov_rate`",
            )
            return

        pov_rate = Decimal(str(pov_rate))
        if pov_rate <= 0 or pov_rate > 1:
            self.log.error(
                f"无法执行订单：`pov_rate` 必须在 (0, 1] 范围内，当前值为 {pov_rate}",
            )
            return

        # 获取检查间隔参数（秒）
        interval_secs = exec_params.get("interval_secs")
        if not interval_secs:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` {exec_params} 中未找到 `interval_secs`",
            )
            return

        # 获取最大执行时间（秒），可选参数，默认无限制
        max_horizon_secs = exec_params.get("max_horizon_secs", 0)

        # 获取最大切片限制（可选参数），防止突然放量导致发单过大冲击盘口
        max_slice_qty = exec_params.get("max_slice_qty")

        # 获取可选参数
        max_display_ratio = exec_params.get("max_display_ratio", 0.1)  # 默认 10%
        randomization_enabled = exec_params.get("randomization_enabled", True)  # 默认启用

        # 初始化该订单的跟踪状态
        self._remaining_qty[order.client_order_id] = order.quantity.as_decimal()
        self._interval_volume[order.client_order_id] = Decimal(0)
        self._pov_rates[order.client_order_id] = pov_rate
        self._order_instrument_ids[order.client_order_id] = order.instrument_id
        self._max_horizon_secs[order.client_order_id] = float(max_horizon_secs)
        self._start_time_ns[order.client_order_id] = self.clock.timestamp_ns()
        if max_slice_qty:
            self._max_slice_qty[order.client_order_id] = Decimal(str(max_slice_qty))
        self._randomization_enabled[order.client_order_id] = randomization_enabled
        self._max_display_ratios[order.client_order_id] = max_display_ratio
        self._base_intervals[order.client_order_id] = interval_secs

        # 订阅该合约的逐笔成交数据（用于跟踪市场成交量）
        if order.instrument_id not in self._subscribed_instruments:
            self.subscribe_trade_ticks(order.instrument_id)
            self._subscribed_instruments.add(order.instrument_id)

        # 设置定时器，按固定间隔触发成交量检查（应用时间随机化）
        timer_interval = self.apply_time_randomization(interval_secs) if randomization_enabled else timedelta(seconds=interval_secs)
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timer_interval,
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 POV 执行 {order.client_order_id}："
            f"pov_rate={pov_rate}, {interval_secs=}, {max_horizon_secs=}",
            LogColor.BLUE,
        )

    def on_trade_tick(self, tick: TradeTick) -> None:
        """
        接收到逐笔成交数据时的回调。

        Parameters
        ----------
        tick : TradeTick
            接收到的逐笔成交数据。
        """
        # 遍历所有正在执行的订单，累积与该合约匹配的成交量
        tick_size = tick.size.as_decimal()
        for client_order_id, instrument_id in self._order_instrument_ids.items():
            if tick.instrument_id == instrument_id:
                self._interval_volume[client_order_id] += tick_size

    def on_time_event(self, event: TimeEvent) -> None:
        """
        接收到定时事件时的回调。

        Parameters
        ----------
        event : TimeEvent
            接收到的定时事件。
        """
        self.log.info(repr(event), LogColor.CYAN)

        exec_spawn_id = ClientOrderId(event.name)

        # 查找主订单
        primary: Order = self.cache.order(exec_spawn_id)
        if not primary:
            self.log.error(f"未找到 {exec_spawn_id=} 对应的主订单")
            return

        # 如果主订单已关闭，完成执行序列
        if primary.is_closed:
            self.complete_sequence(primary.client_order_id)
            return

        # 获取合约信息
        instrument: Instrument = self.cache.instrument(primary.instrument_id)
        if not instrument:
            self.log.error(
                f"无法执行订单：未找到合约 {primary.instrument_id}",
            )
            return

        # 获取剩余待执行数量
        remaining = self._remaining_qty.get(exec_spawn_id)
        if remaining is None:
            self.log.error(f"未找到 {exec_spawn_id=} 的剩余数量")
            return

        if remaining <= 0:
            self.log.warning(f"{exec_spawn_id=} 没有更多的数量可执行")
            self.complete_sequence(primary.client_order_id)
            return

        # 撤销上一轮未成交完的子订单
        if exec_spawn_id in self._active_spawned_orders:
            prev_spawn_id = self._active_spawned_orders[exec_spawn_id]
            prev_order = self.cache.order(prev_spawn_id)
            if prev_order and not prev_order.is_closed:
                self.log.info(f"由于新的时间片到达，撤销尚未完全成交的上一轮订单: {prev_spawn_id}")
                self.cancel_order(prev_order)
            self._active_spawned_orders.pop(exec_spawn_id)

        # 检查是否已超过最大执行时间
        max_horizon = self._max_horizon_secs.get(exec_spawn_id, 0)
        if max_horizon > 0:
            elapsed_ns = self.clock.timestamp_ns() - self._start_time_ns.get(exec_spawn_id, 0)
            elapsed_secs = elapsed_ns / 1_000_000_000
            if elapsed_secs >= max_horizon:
                self.log.warning(
                    f"已达到最大执行时间 {max_horizon}s，提交剩余数量 {remaining}",
                )
                if primary.order_type == OrderType.LIMIT:
                    quote = self.cache.quote_tick(primary.instrument_id)
                    if quote:
                        price = quote.bid_price if primary.side == OrderSide.BUY else quote.ask_price
                        self.modify_order_in_place(primary, quantity=primary.quantity, price=price)
                self.submit_order(primary)
                self.complete_sequence(primary.client_order_id)
                return

        # 获取本间隔内的市场成交量和目标参与率
        interval_vol = self._interval_volume.get(exec_spawn_id, Decimal(0))
        pov_rate = self._pov_rates.get(exec_spawn_id, Decimal("0.1"))

        # 重置该间隔的成交量累积，为下一个间隔做准备
        self._interval_volume[exec_spawn_id] = Decimal(0)

        # 如果本间隔没有市场成交量，跳过（POV 的核心特性：无量则不执行）
        if interval_vol <= 0:
            self.log.info(
                f"本间隔无市场成交量，跳过执行 (POV 被动跟随)",
                LogColor.YELLOW,
            )
            return

        # 计算目标执行量 = 市场成交量 × 参与率
        target_qty = interval_vol * pov_rate

        # 确保不超过剩余数量
        target_qty = min(target_qty, remaining)

        # 防冲击：应用最大单笔限制
        max_slice = self._max_slice_qty.get(exec_spawn_id, None)
        if max_slice and target_qty > max_slice:
            self.log.info(f"POV 计算量 {target_qty} 超过限制 {max_slice}，进行截断", LogColor.YELLOW)
            target_qty = max_slice

        # 应用市场深度限制
        max_display_ratio = self._max_display_ratios.get(exec_spawn_id, 0.1)
        if max_display_ratio > 0:
            proposed_qty = instrument.make_qty(target_qty)
            limited_qty = self.check_market_depth_limit(
                instrument, proposed_qty, max_display_ratio, primary.side
            )
            target_qty = limited_qty.as_decimal()

        # 获取最小可执行数量
        min_qty_decimal = instrument.size_increment.as_decimal()
        if instrument.min_quantity:
            min_qty_decimal = max(min_qty_decimal, instrument.min_quantity.as_decimal())

        # 如果计算出的数量低于最小数量，跳过本次间隔
        if target_qty < min_qty_decimal:
            self.log.info(
                f"POV 计算量 {target_qty} 低于最小数量 {min_qty_decimal}，跳过",
                LogColor.YELLOW,
            )
            return

        # 向下取整到合约精度
        target_qty = self.round_decimal_down(target_qty, instrument.size_precision)
        if target_qty < min_qty_decimal:
            return

        quantity: Quantity = instrument.make_qty(target_qty)

        # 应用数量随机化
        randomization_enabled = self._randomization_enabled.get(exec_spawn_id, True)
        if randomization_enabled:
            remaining_qty = instrument.make_qty(remaining)
            is_final = (remaining - target_qty) <= min_qty_decimal  # 如果剩余量少于最小单位，视为最后一批
            quantity = self.apply_quantity_randomization(quantity, remaining_qty, is_final)
            target_qty = quantity.as_decimal()

        # 更新剩余数量
        self._remaining_qty[exec_spawn_id] -= target_qty

        # 检查更新后的剩余数量：如果不足以再下一笔单，则直接提交主订单完成
        new_remaining = self._remaining_qty[exec_spawn_id]
        if new_remaining <= 0 or new_remaining < min_qty_decimal:
            # 剩余量不足或归零，将剩余量合并到本次，直接提交主订单
            if primary.order_type == OrderType.LIMIT:
                quote = self.cache.quote_tick(primary.instrument_id)
                if quote:
                    price = quote.bid_price if primary.side == OrderSide.BUY else quote.ask_price
                    self.modify_order_in_place(primary, quantity=primary.quantity, price=price)
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        self.log.info(
            f"POV 执行: interval_vol={interval_vol}, pov_rate={pov_rate}, "
            f"target_qty={quantity}, remaining={new_remaining}",
            LogColor.BLUE,
        )

        # 生成并提交子订单
        if primary.order_type == OrderType.LIMIT:
            quote = self.cache.quote_tick(primary.instrument_id)
            if not quote:
                self.log.warning(f"无法获取合约 {primary.instrument_id} 的最新盘口数据，跳过本次限价单执行")
                self._remaining_qty[exec_spawn_id] += target_qty # 退回数量
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
        else:
            spawned_order = self.spawn_market(
                primary=primary,
                quantity=quantity,
                time_in_force=primary.time_in_force,
                reduce_only=primary.is_reduce_only,
                tags=primary.tags,
            )

        self._active_spawned_orders[exec_spawn_id] = spawned_order.client_order_id
        self.submit_order(spawned_order)

        # 重置定时器（应用时间随机化）
        if randomization_enabled:
            base_interval = self._base_intervals.get(exec_spawn_id, 60.0)
            timer_interval = self.apply_time_randomization(base_interval)
            self.clock.set_timer(
                name=exec_spawn_id.value,
                interval=timer_interval,
                callback=self.on_time_event,
            )

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        完成执行序列的回调。

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            要完成的执行序列的客户端订单 ID。
        """
        # 取消定时器
        if exec_spawn_id.value in self.clock.timer_names:
            self.clock.cancel_timer(exec_spawn_id.value)

        # 清理所有跟踪状态
        self._remaining_qty.pop(exec_spawn_id, None)
        self._interval_volume.pop(exec_spawn_id, None)
        self._pov_rates.pop(exec_spawn_id, None)
        instrument_id = self._order_instrument_ids.pop(exec_spawn_id, None)
        self._max_horizon_secs.pop(exec_spawn_id, None)
        self._start_time_ns.pop(exec_spawn_id, None)
        self._max_slice_qty.pop(exec_spawn_id, None)
        self._active_spawned_orders.pop(exec_spawn_id, None)
        self._randomization_enabled.pop(exec_spawn_id, None)
        self._max_display_ratios.pop(exec_spawn_id, None)
        self._base_intervals.pop(exec_spawn_id, None)

        # 如果没有其他订单使用该合约的 TradeTick，则取消订阅
        if instrument_id and not any(
            iid == instrument_id for iid in self._order_instrument_ids.values()
        ):
            self.unsubscribe_trade_ticks(instrument_id)
            self._subscribed_instruments.discard(instrument_id)

        self.log.info(f"已完成 POV 执行 {exec_spawn_id}", LogColor.BLUE)
