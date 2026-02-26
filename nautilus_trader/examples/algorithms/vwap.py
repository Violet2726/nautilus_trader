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

from __future__ import annotations

import math
from datetime import timedelta
from decimal import ROUND_DOWN
from decimal import Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.data import TradeTick
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import MarketOrder
from nautilus_trader.model.orders import Order


class VWAPExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    ``VWAPExecAlgorithm`` 实例的配置类。

    该配置类定义了成交量加权平均价格 (VWAP) 执行算法所需的参数。
    VWAP 算法根据市场实际成交量的分布情况来分配子订单的大小，
    目标是使最终执行价格尽可能接近 VWAP 基准价格。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId
        执行算法 ID（将覆盖默认值，默认值为类名）。

    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("VWAP")


class VWAPExecAlgorithm(ExecAlgorithm):
    """
    提供成交量加权平均价格 (VWAP) 执行算法。

    VWAP 执行算法的目标是根据市场成交量分布来执行订单。算法接收一个代表总数量和方向的
    主订单 (primary order)，然后根据实时市场成交量的变化，在指定的时间范围内按比例
    生成较小的子订单 (child orders) 进行执行。

    与 TWAP (时间加权) 不同，VWAP 会在市场成交量较大时执行更多的数量，
    在成交量较小时执行较少的数量，从而减少对市场价格的冲击。

    算法工作流程：
    1. 接收主订单后，订阅该合约的逐笔成交 (TradeTick) 数据
    2. 按固定的检查间隔 (interval_secs) 采样市场成交量
    3. 在每个间隔结束时，根据该间隔内的市场成交量占总成交量的比例，
       计算应执行的子订单大小
    4. 在时间范围 (horizon_secs) 结束时提交剩余的主订单

    Parameters
    ----------
    config : VWAPExecAlgorithmConfig, optional
        该实例的配置。

    """

    def __init__(self, config: VWAPExecAlgorithmConfig | None = None) -> None:
        if config is None:
            config = VWAPExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单的剩余待执行数量
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单在当前采样间隔内累积的市场成交量
        self._interval_volume: dict[ClientOrderId, Decimal] = {}
        # 每个主订单自执行开始以来的总市场成交量
        self._total_volume: dict[ClientOrderId, Decimal] = {}
        # 每个主订单对应的合约 ID，用于在 on_trade_tick 中匹配
        self._order_instrument_ids: dict[ClientOrderId, InstrumentId] = {}
        # 每个主订单的总执行数量 (原始订单数量)
        self._total_order_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的已执行间隔数
        self._intervals_executed: dict[ClientOrderId, int] = {}
        # 每个主订单的总间隔数
        self._total_intervals: dict[ClientOrderId, int] = {}
        # 跟踪已订阅 TradeTick 的合约 ID
        self._subscribed_instruments: set[InstrumentId] = set()

    def on_start(self) -> None:
        """
        算法组件启动时执行的操作。
        """
        # 可选实现

    def on_stop(self) -> None:
        """
        算法组件停止时执行的操作。
        """
        # 取消所有活跃的定时器
        self.clock.cancel_timers()

    def on_reset(self) -> None:
        """
        算法组件重置时执行的操作。
        """
        self._remaining_qty.clear()
        self._interval_volume.clear()
        self._total_volume.clear()
        self._order_instrument_ids.clear()
        self._total_order_qty.clear()
        self._intervals_executed.clear()
        self._total_intervals.clear()
        self._subscribed_instruments.clear()

    def on_save(self) -> dict[str, bytes]:
        """
        算法组件保存时执行的操作。

        创建并返回要保存的状态字典。

        Returns
        -------
        dict[str, bytes]
            策略状态字典。

        """
        return {}  # 可选实现

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        算法组件加载时执行的操作。

        已保存的状态值将包含在给定的状态字典中。

        Parameters
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
        算法运行中接收到订单时执行的操作。

        Parameters
        ----------
        order : Order
            待处理的订单。

        Warnings
        --------
        系统方法（不应由用户代码直接调用）。

        """
        # 确保该订单尚未被调度
        PyCondition.not_in(
            order.client_order_id,
            self._remaining_qty,
            "order.client_order_id",
            "self._remaining_qty",
        )
        self.log.info(repr(order), LogColor.CYAN)

        # 仅支持市价单
        if order.order_type != OrderType.MARKET:
            self.log.error(
                f"无法执行订单：仅支持市价单，当前类型为 {order.order_type=}",
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

        # 获取时间范围参数（秒）
        horizon_secs = exec_params.get("horizon_secs")
        if not horizon_secs:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` {exec_params} 中未找到 `horizon_secs`",
            )
            return

        # 获取检查间隔参数（秒）
        interval_secs = exec_params.get("interval_secs")
        if not interval_secs:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` {exec_params} 中未找到 `interval_secs`",
            )
            return

        # 验证时间范围必须大于等于检查间隔
        if horizon_secs < interval_secs:
            self.log.error(
                f"无法执行订单：{horizon_secs=} 小于 {interval_secs=}",
            )
            return

        # 计算总间隔数
        num_intervals: int = math.floor(horizon_secs / interval_secs)

        # 计算每个间隔的均匀分配量（作为最小保底量）
        quotient = order.quantity.as_decimal() / num_intervals
        floored_quotient = self.round_decimal_down(quotient, instrument.size_precision)
        qty_per_interval = instrument.make_qty(floored_quotient)

        # 如果无法拆分（单笔量等于总量、低于最小增量或最小数量），则直接提交整个订单
        if (
            qty_per_interval == order.quantity
            or qty_per_interval < instrument.size_increment
            or (instrument.min_quantity and qty_per_interval < instrument.min_quantity)
        ):
            self.log.warning(f"直接提交全部数量 {qty_per_interval=}, {order.quantity=}")
            self.submit_order(order)
            return  # 完成

        # 初始化该订单的跟踪状态
        self._remaining_qty[order.client_order_id] = order.quantity.as_decimal()
        self._interval_volume[order.client_order_id] = Decimal(0)
        self._total_volume[order.client_order_id] = Decimal(0)
        self._order_instrument_ids[order.client_order_id] = order.instrument_id
        self._total_order_qty[order.client_order_id] = order.quantity.as_decimal()
        self._intervals_executed[order.client_order_id] = 0
        self._total_intervals[order.client_order_id] = num_intervals

        # 订阅该合约的逐笔成交数据（用于跟踪市场成交量）
        if order.instrument_id not in self._subscribed_instruments:
            self.subscribe_trade_ticks(order.instrument_id)
            self._subscribed_instruments.add(order.instrument_id)

        # 立即提交第一笔子订单（使用均匀分配量作为初始量）
        first_qty: Quantity = instrument.make_qty(floored_quotient)
        self._remaining_qty[order.client_order_id] -= floored_quotient

        spawned_order: MarketOrder = self.spawn_market(
            primary=order,
            quantity=first_qty,
            time_in_force=order.time_in_force,
            reduce_only=order.is_reduce_only,
            tags=order.tags,
        )

        self.submit_order(spawned_order)
        self._intervals_executed[order.client_order_id] = 1

        # 设置定时器，按固定间隔触发成交量采样检查
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timedelta(seconds=interval_secs),
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 VWAP 执行 {order.client_order_id}：{horizon_secs=}, {interval_secs=}",
            LogColor.BLUE,
        )

    def on_trade_tick(self, tick: TradeTick) -> None:
        """
        接收到逐笔成交数据时执行的操作。

        在每个采样间隔内累积市场成交量，用于计算下一个子订单的大小。

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
                self._total_volume[client_order_id] += tick_size

    def on_time_event(self, event: TimeEvent) -> None:
        """
        算法接收到定时事件时执行的操作。

        在每个间隔结束时，根据该间隔内的市场成交量占比计算应执行的子订单数量。

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

        # 更新已执行间隔计数
        self._intervals_executed[exec_spawn_id] += 1
        intervals_executed = self._intervals_executed[exec_spawn_id]
        total_intervals = self._total_intervals[exec_spawn_id]

        # 如果这是最后一个间隔，提交剩余的主订单
        if intervals_executed >= total_intervals:
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        # 根据成交量比例计算本次应执行的数量
        quantity_decimal = self._calculate_vwap_quantity(
            exec_spawn_id,
            instrument,
            intervals_executed,
            total_intervals,
        )

        # 确保计算的数量不超过剩余数量
        quantity_decimal = min(quantity_decimal, remaining)

        # 如果计算出的数量低于最小增量，跳过本次间隔
        min_qty_decimal = instrument.size_increment.as_decimal()
        if instrument.min_quantity:
            min_qty_decimal = max(min_qty_decimal, instrument.min_quantity.as_decimal())

        if quantity_decimal < min_qty_decimal:
            self.log.info(
                f"本间隔计算量 {quantity_decimal} 低于最小数量 {min_qty_decimal}，跳过",
                LogColor.YELLOW,
            )
            # 重置该间隔的成交量累积
            self._interval_volume[exec_spawn_id] = Decimal(0)
            return

        # 向下取整到合约精度
        quantity_decimal = self.round_decimal_down(quantity_decimal, instrument.size_precision)
        if quantity_decimal < min_qty_decimal:
            self._interval_volume[exec_spawn_id] = Decimal(0)
            return

        quantity: Quantity = instrument.make_qty(quantity_decimal)

        # 更新剩余数量
        self._remaining_qty[exec_spawn_id] -= quantity_decimal

        # 重置该间隔的成交量累积，为下一个间隔做准备
        self._interval_volume[exec_spawn_id] = Decimal(0)

        # 生成并提交子订单
        spawned_order: MarketOrder = self.spawn_market(
            primary=primary,
            quantity=quantity,
            time_in_force=primary.time_in_force,
            reduce_only=primary.is_reduce_only,
            tags=primary.tags,
        )

        self.submit_order(spawned_order)

    def _calculate_vwap_quantity(
        self,
        exec_spawn_id: ClientOrderId,
        instrument: Instrument,
        intervals_executed: int,
        total_intervals: int,
    ) -> Decimal:
        """
        根据 VWAP 策略计算当前间隔应执行的数量。

        计算逻辑：
        - 如果未观测到市场成交量（如盘前/流动性极低），
          则回退到均匀分配（类似 TWAP）
        - 否则按照 (当前间隔成交量 / 累积总成交量) 的比例，
          乘以剩余待执行数量来计算

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            执行订单的客户端订单 ID。
        instrument : Instrument
            合约信息。
        intervals_executed : int
            已执行的间隔数。
        total_intervals : int
            总间隔数。

        Returns
        -------
        Decimal
            当前间隔应执行的数量（未取整）。

        """
        interval_vol = self._interval_volume.get(exec_spawn_id, Decimal(0))
        total_vol = self._total_volume.get(exec_spawn_id, Decimal(0))
        remaining = self._remaining_qty.get(exec_spawn_id, Decimal(0))
        remaining_intervals = total_intervals - intervals_executed

        if remaining_intervals <= 0:
            return remaining

        # 如果没有观测到成交量数据，回退到均匀分配
        if total_vol <= 0 or interval_vol <= 0:
            self.log.info(
                f"未观测到市场成交量，回退到均匀分配: "
                f"remaining={remaining}, remaining_intervals={remaining_intervals}",
                LogColor.YELLOW,
            )
            return remaining / remaining_intervals

        # 基于成交量比例计算：当前间隔成交量占总成交量的比例
        volume_ratio = interval_vol / total_vol
        # 按比例分配剩余数量
        vwap_qty = remaining * volume_ratio

        self.log.info(
            f"VWAP 计算: interval_vol={interval_vol}, total_vol={total_vol}, "
            f"volume_ratio={volume_ratio:.4f}, vwap_qty={vwap_qty}",
            LogColor.BLUE,
        )

        return vwap_qty

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        完成一个执行序列。

        清理与该订单相关的所有跟踪状态，并取消定时器。

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
        self._total_volume.pop(exec_spawn_id, None)
        instrument_id = self._order_instrument_ids.pop(exec_spawn_id, None)
        self._total_order_qty.pop(exec_spawn_id, None)
        self._intervals_executed.pop(exec_spawn_id, None)
        self._total_intervals.pop(exec_spawn_id, None)

        # 如果没有其他订单使用该合约的 TradeTick，则取消订阅
        if instrument_id and not any(
            iid == instrument_id for iid in self._order_instrument_ids.values()
        ):
            self.unsubscribe_trade_ticks(instrument_id)
            self._subscribed_instruments.discard(instrument_id)

        self.log.info(f"已完成 VWAP 执行 {exec_spawn_id}", LogColor.BLUE)
