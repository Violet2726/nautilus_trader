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

    标准的主动（确定性） VWAP 执行算法的目标是根据该资产**历史上**的成交量分布轮廓（Profile）
    来执行订单。算法接收一个代表总数量和方向的主订单 (primary order)，并在 `exec_algorithm_params` 
    中接收一个 `volume_profile`（历史预测的各时间间隔成交量占比数组）。算法会在订单生命周期内，
    按照预先规划好的轮廓曲线切分好所有的子订单份额，并按照固定的时间节拍触发执行。

    与根据实时追踪来被动修正数量的模式不同，标准 VWAP 需要预先设定执行计划，无论市场当时
    如何波动，它都会极其明确地按照历史拟合的曲线去砸单。

    算法工作流程：
    1. 接收主订单后，提取执行参数中的 `volume_profile` 
    2. 计算每个分布切片应占的订单数量
    3. 在每个时间间隔结束时，按照预先定好的规模计划依次弹出并生成对应的子订单
    4. 最后一笔直接将剩余的主订单全量推向市场

    Parameters
    ----------
    config : VWAPExecAlgorithmConfig, optional
        该实例的配置。

    """

    def __init__(self, config: VWAPExecAlgorithmConfig | None = None) -> None:
        if config is None:
            config = VWAPExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单计划执行的子订单大小队列
        self._scheduled_sizes: dict[ClientOrderId, list[Quantity]] = {}

    def on_start(self) -> None:
        """
        算法组件启动时执行的操作。
        """
        pass

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

    def on_save(self) -> dict[str, bytes]:
        return {}  

    def on_load(self, state: dict[str, bytes]) -> None:
        pass

    def round_decimal_down(self, amount: Decimal, precision: int) -> Decimal:
        """将 Decimal 值向下取整到指定精度。"""
        return amount.quantize(Decimal(f"1e-{precision}"), rounding=ROUND_DOWN)

    def on_order(self, order: Order) -> None:
        """
        算法运行中接收到订单时执行的操作。
        """
        PyCondition.not_in(
            order.client_order_id,
            self._scheduled_sizes,
            "order.client_order_id",
            "self._scheduled_sizes",
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

        # 核心：获取历史曲线 Profile
        volume_profile = exec_params.get("volume_profile")
        if not volume_profile:
            self.log.error(f"无法执行订单：标准 VWAP 需要在 `exec_algorithm_params` 中提供 `volume_profile` (历史成交各间隔占比列表)")
            return
            
        if len(volume_profile) != num_intervals:
            self.log.error(f"无法执行订单：`volume_profile` 的长度 ({len(volume_profile)}) 与基于时间计算的总间隔数 ({num_intervals}) 不匹配")
            return
            
        total_profile = sum(volume_profile)
        if total_profile <= 0:
            self.log.error("无法执行订单：`volume_profile` 的比例各项合计必须大于 0")
            return
            
        normalized_profile = [Decimal(str(p)) / Decimal(str(total_profile)) for p in volume_profile]

        # 根据 Profile 乘积将主订单拆分为子订单序列规划
        total_qty_decimal = order.quantity.as_decimal()
        scheduled_sizes = []
        accumulated_qty = Decimal(0)
        
        for p in normalized_profile[:-1]:
            target_qty = total_qty_decimal * p
            target_qty_rounded = self.round_decimal_down(target_qty, instrument.size_precision)
            scheduled_sizes.append(target_qty_rounded)
            accumulated_qty += target_qty_rounded
            
        # 最后一笔切片容纳剩余由于精度流失引起的所有偏差量
        last_qty = total_qty_decimal - accumulated_qty
        scheduled_sizes.append(last_qty)

        self.log.info(f"VWAP 基于历史Profile产生的执行数量序列：{scheduled_sizes}", LogColor.BLUE)

        # 转换为安全的 Quantity 对象（保留占位0以保证发单时间间隔的一致性推进）
        qty_schedule = []
        for s in scheduled_sizes:
            if s > 0:
                qty_schedule.append(instrument.make_qty(s))
            else:
                qty_schedule.append(None)
        
        self._scheduled_sizes[order.client_order_id] = qty_schedule
        first_qty = self._scheduled_sizes[order.client_order_id].pop(0)

        # 发送首单切片
        if first_qty is not None:
            spawned_order: MarketOrder = self.spawn_market(
                primary=order,
                quantity=first_qty,
                time_in_force=order.time_in_force,
                reduce_only=order.is_reduce_only,
                tags=order.tags,
            )
            self.submit_order(spawned_order)

        # 注册定时器接管未来的队列扫描
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timedelta(seconds=interval_secs),
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 VWAP 执行 {order.client_order_id}：{horizon_secs=}, {interval_secs=}",
            LogColor.BLUE,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        """
        定时触发按计划派发下一笔子订单。
        """
        self.log.info(repr(event), LogColor.CYAN)

        exec_spawn_id = ClientOrderId(event.name)
        primary: Order = self.cache.order(exec_spawn_id)
        if not primary:
            self.log.error(f"未找到 {exec_spawn_id=} 对应的主订单")
            return

        if primary.is_closed:
            self.complete_sequence(primary.client_order_id)
            return

        # 调取挂载的预排期切片序列
        scheduled_sizes = self._scheduled_sizes.get(exec_spawn_id)
        if scheduled_sizes is None:
            self.log.error(f"无法找到 {exec_spawn_id=} 的计划规模")
            return

        if not scheduled_sizes:
            self.log.warning(f"{exec_spawn_id=} 没有更多的规模可执行")
            return

        # 弹出该时间片的预分配数量
        quantity_to_execute = scheduled_sizes.pop(0)

        # 发现已经是最后一个时间点，直接把已被切割过好几轮的剩余主订单本身发走以收尾
        if not scheduled_sizes:
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        # 如果这一时段刚好历史占比太小分配到的 quantity 为 0
        if quantity_to_execute is None:
            self.log.info(f"本时间段对应计划成交量被合约精度舍入为 0，跳过发单")
            return

        spawned_order: MarketOrder = self.spawn_market(
            primary=primary,
            quantity=quantity_to_execute,
            time_in_force=primary.time_in_force,
            reduce_only=primary.is_reduce_only,
            tags=primary.tags,
        )

        self.submit_order(spawned_order)

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        销毁完成对象的定时器并清理引用状态。
        """
        if exec_spawn_id.value in self.clock.timer_names:
            self.clock.cancel_timer(exec_spawn_id.value)

        self._scheduled_sizes.pop(exec_spawn_id, None)

        self.log.info(f"已完成 VWAP 执行 {exec_spawn_id}", LogColor.BLUE)
