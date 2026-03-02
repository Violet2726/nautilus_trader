"""
成交量加权平均价格 (VWAP) 执行算法。

该算法根据历史成交量分布轮廓 (Volume Profile) 来分配子订单，
目标是使最终执行价格尽可能接近 VWAP 基准价格。
"""

from __future__ import annotations

import math
import random
from datetime import timedelta
from decimal import ROUND_DOWN, Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.events import TimeEvent
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.identifiers import ClientOrderId, ExecAlgorithmId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import MarketOrder, Order


class VWAPExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    VWAP 执行算法配置类。

    该配置类定义了成交量加权平均价格 (VWAP) 执行算法所需的参数。
    VWAP 算法根据市场实际成交量的分布情况来分配子订单的大小，
    目标是使最终执行价格尽可能接近 VWAP 基准价格。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId, optional
        执行算法 ID（默认："VWAP"）。

    Notes
    -----
    此配置类定义了 VWAP 执行算法所需的参数。
    该算法旨在根据历史成交量分布轮廓 (Volume Profile) 来分配子订单。
    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("VWAP")


class VWAPExecAlgorithm(ExecAlgorithm):
    """
    成交量加权平均价格 (VWAP) 执行算法。

    算法特点：
    - 根据历史成交量分布轮廓 (Volume Profile) 分配子订单
    - 预先设定执行计划，按固定时间节拍触发执行
    - 支持时间随机化（±20% 抖动）
    - 支持数量随机化（±5% 扰动）
    - 支持市场冲击保护（限制订单占市场深度的比例）
    - 支持进度自适应调整（根据执行进度动态调整）

    算法工作流程：
    1. 接收主订单后，提取执行参数中的 `volume_profile`
    2. 计算每个分布切片应占的订单数量
    3. 在每个时间间隔结束时，按照预先定好的规模计划依次弹出并生成对应的子订单
    4. 最后一笔直接将剩余的主订单全量推向市场

    Parameters
    ----------
    config : VWAPExecAlgorithmConfig, optional
        算法配置实例。

    Notes
    -----
    与根据实时追踪来被动修正数量的模式不同，标准 VWAP 需要预先设定执行计划，
    无论市场当时如何波动，它都会极其明确地按照历史拟合的曲线去执行。
    """

    def __init__(self, config: VWAPExecAlgorithmConfig | None = None) -> None:
        """
        初始化 VWAP 执行算法。

        Parameters
        ----------
        config : VWAPExecAlgorithmConfig, optional
            算法配置实例。
        """
        if config is None:
            config = VWAPExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单计划执行的子订单大小队列
        self._scheduled_sizes: dict[ClientOrderId, list[Quantity]] = {}
        # 随机化控制参数
        self._randomization_enabled: dict[ClientOrderId, bool] = {}
        self._max_market_impact_ratios: dict[ClientOrderId, float] = {}
        self._base_intervals: dict[ClientOrderId, float] = {}
        # 进度跟踪
        self._executed_qty: dict[ClientOrderId, Decimal] = {}
        self._total_qty: dict[ClientOrderId, Decimal] = {}

    def on_start(self) -> None:
        """
        算法启动时执行的操作。

        Notes
        -----
        此方法在算法组件启动时被调用，可用于初始化资源。
        """
        pass

    def on_stop(self) -> None:
        """
        算法停止时执行的操作。

        Notes
        -----
        此方法在算法组件停止时被调用，负责清理定时器资源。
        """
        self.clock.cancel_timers()

    def on_reset(self) -> None:
        """
        算法重置时执行的操作。

        Notes
        -----
        此方法在算法组件重置时被调用，负责清空所有内部状态字典。
        """
        self._scheduled_sizes.clear()
        self._randomization_enabled.clear()
        self._max_market_impact_ratios.clear()
        self._base_intervals.clear()
        self._executed_qty.clear()
        self._total_qty.clear()

    def on_save(self) -> dict[str, bytes]:
        """
        保存算法状态。

        Returns
        -------
        dict[str, bytes]
            算法状态字典。

        Notes
        -----
        此方法用于持久化算法状态，当前实现返回空字典。
        """
        return {}  

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        加载算法状态。

        Parameters
        ----------
        state : dict[str, bytes]
            要加载的算法状态字典。

        Notes
        -----
        此方法用于从持久化存储中恢复算法状态，当前实现为空操作。
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

        Notes
        -----
        时间随机化范围：0.8x 到 1.2x（±20% 抖动）
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
            基础数量。
        remaining_qty : Quantity
            剩余数量。
        is_final : bool
            是否为最后一笔订单。

        Returns
        -------
        Quantity
            随机化后的数量。

        Notes
        -----
        数量随机化范围：0.95x 到 1.05x（±5% 扰动）
        最后一笔订单不进行随机化，直接返回剩余数量。
        """
        if is_final:
            return remaining_qty

        randomization_factor = 0.95 + random.uniform(0.0, 0.10)  # 0.95 to 1.05
        randomized_raw = float(base_qty.as_double()) * randomization_factor
        instrument = self.cache.instrument(base_qty.instrument_id)
        randomized_qty = instrument.make_qty(Decimal(str(randomized_raw)))
        
        # 确保数量在合理范围内
        return min(randomized_qty, remaining_qty)

    def check_market_impact_limit(
        self,
        instrument: Instrument,
        proposed_qty: Quantity,
        max_impact_ratio: float,
    ) -> Quantity:
        """
        检查市场冲击限制。

        Parameters
        ----------
        instrument : Instrument
            交易工具。
        proposed_qty : Quantity
            提议的订单数量。
        max_impact_ratio : float
            最大市场冲击比例（如 0.1 表示 10%）。

        Returns
        -------
        Quantity
            经过市场冲击限制调整后的数量。

        Notes
        -----
        此方法通过比较订单数量与订单簿深度，确保单笔订单不会对市场造成过大冲击。
        限制规则：订单数量不超过订单簿最佳报价数量的指定比例。
        """
        book = self.cache.order_book(instrument.id)
        if book is None:
            return proposed_qty

        # 获取双向报价数量
        bid_qty = book.best_bid_size()
        ask_qty = book.best_ask_size()
        
        if bid_qty is None or ask_qty is None:
            return proposed_qty

        # 使用较小的深度
        depth_qty = min(bid_qty, ask_qty)
        max_allowed = depth_qty.as_decimal() * Decimal(str(max_impact_ratio))
        limited_qty = min(proposed_qty, instrument.make_qty(max_allowed))
        
        # 确保至少有 1 单位的量
        if limited_qty.as_decimal() < instrument.size_increment.as_decimal():
            limited_qty = instrument.make_qty(instrument.size_increment.as_decimal())
            
        return limited_qty

    def adapt_progress(self, base_qty: Quantity, progress_ratio: float) -> Quantity:
        """
        根据执行进度自适应调整数量。

        Parameters
        ----------
        base_qty : Quantity
            基础数量。
        progress_ratio : float
            当前执行进度比例（0.0 到 1.0）。

        Returns
        -------
        Quantity
            调整后的数量。

        Notes
        -----
        如果进度落后于时间（进度比例 < 0.8），适当增加执行量以追赶进度。
        调整规则：最多增加 10%（当进度为 0 时）。
        """
        # 如果进度落后，适当增加执行量
        if progress_ratio < 0.8:  # 进度落后于时间
            adjustment_factor = 1.0 + (0.8 - progress_ratio) * 0.125  # 最多增加 10%
            adjusted_raw = float(base_qty.as_double()) * adjustment_factor
            instrument = self.cache.instrument(base_qty.instrument_id)
            return instrument.make_qty(Decimal(str(adjusted_raw)))
        return base_qty

    def on_order(self, order: Order) -> None:
        """
        算法运行中接收到订单时执行的操作。

        此方法处理主订单的初始化，包括：
        - 验证订单类型和参数
        - 解析执行参数（时间范围、检查间隔等）
        - 根据历史成交量分布轮廓 (Volume Profile) 拆分订单
        - 应用各种保护机制（随机化、市场冲击限制等）
        - 启动定时器以按计划执行子订单

        Parameters
        ----------
        order : Order
            主订单对象。

        Notes
        -----
        该方法负责整个 VWAP 执行流程的初始化阶段。
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

        # 获取可选参数
        max_market_impact_ratio = exec_params.get("max_market_impact_ratio", 0.1)  # 默认 10%
        randomization_enabled = exec_params.get("randomization_enabled", True)  # 默认启用

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

        self.log.info(f"VWAP 基于历史 Profile 产生的执行数量序列：{scheduled_sizes}", LogColor.BLUE)

        # 初始化新参数
        self._randomization_enabled[order.client_order_id] = randomization_enabled
        self._max_market_impact_ratios[order.client_order_id] = max_market_impact_ratio
        self._base_intervals[order.client_order_id] = interval_secs
        self._total_qty[order.client_order_id] = order.quantity.as_decimal()
        self._executed_qty[order.client_order_id] = Decimal(0)

        # 转换为安全的 Quantity 对象（保留占位 0 以保证发单时间间隔的一致性推进）
        qty_schedule = []
        for s in scheduled_sizes:
            if s > 0:
                base_qty = instrument.make_qty(s)
                # 应用市场冲击限制
                if max_market_impact_ratio > 0:
                    base_qty = self.check_market_impact_limit(
                        instrument, base_qty, max_market_impact_ratio
                    )
                qty_schedule.append(base_qty)
            else:
                qty_schedule.append(None)
        
        self._scheduled_sizes[order.client_order_id] = qty_schedule
        first_qty = self._scheduled_sizes[order.client_order_id].pop(0)

        # 应用数量随机化
        if randomization_enabled and first_qty is not None:
            remaining_qty = instrument.make_qty(self._total_qty[order.client_order_id])
            is_final = len(self._scheduled_sizes[order.client_order_id]) == 0
            first_qty = self.apply_quantity_randomization(first_qty, remaining_qty, is_final)

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
            self._executed_qty[order.client_order_id] += first_qty.as_decimal()

        # 注册定时器接管未来的队列扫描（应用时间随机化）
        timer_interval = self.apply_time_randomization(interval_secs) if randomization_enabled else timedelta(seconds=interval_secs)
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timer_interval,
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 VWAP 执行 {order.client_order_id}：{horizon_secs=}, {interval_secs=}",
            LogColor.BLUE,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        """
        定时触发按计划派发下一笔子订单。

        此方法负责：
        - 查找对应的主订单
        - 从预排期序列中弹出当前时间片的订单数量
        - 应用进度自适应和数量随机化
        - 生成并提交子订单

        Parameters
        ----------
        event : TimeEvent
            定时器触发事件。

        Notes
        -----
        该方法在每次定时器触发时被调用，负责按计划执行子订单。
        如果是最后一笔订单，将直接提交剩余的主订单以收尾。
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
        base_qty = scheduled_sizes.pop(0)

        # 发现已经是最后一个时间点，直接把已被切割过好几轮的剩余主订单本身发走以收尾
        if not scheduled_sizes:
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        # 如果这一时段刚好历史占比太小分配到的 quantity 为 0
        if base_qty is None:
            self.log.info(f"本时间段对应计划成交量被合约精度舍入为 0，跳过发单")
            return

        # 应用进度自适应
        total_qty = self._total_qty.get(exec_spawn_id, primary.quantity.as_decimal())
        executed_qty = self._executed_qty.get(exec_spawn_id, Decimal(0))
        progress_ratio = float(executed_qty / total_qty) if total_qty > 0 else 0.0
        
        quantity_to_execute = self.adapt_progress(base_qty, progress_ratio)

        # 应用数量随机化
        randomization_enabled = self._randomization_enabled.get(exec_spawn_id, True)
        if randomization_enabled:
            remaining_qty = self.cache.instrument(primary.instrument_id).make_qty(
                total_qty - executed_qty
            )
            is_final = len(scheduled_sizes) == 0
            quantity_to_execute = self.apply_quantity_randomization(
                quantity_to_execute, remaining_qty, is_final
            )

        spawned_order: MarketOrder = self.spawn_market(
            primary=primary,
            quantity=quantity_to_execute,
            time_in_force=primary.time_in_force,
            reduce_only=primary.is_reduce_only,
            tags=primary.tags,
        )

        self.submit_order(spawned_order)
        self._executed_qty[exec_spawn_id] += quantity_to_execute.as_decimal()

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
        销毁完成对象的定时器并清理引用状态。

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            执行算法的客户端订单 ID。

        Notes
        -----
        此方法在订单执行序列完成时被调用，负责：
        - 取消相关定时器
        - 清理所有与该订单相关的内部状态
        """
        if exec_spawn_id.value in self.clock.timer_names:
            self.clock.cancel_timer(exec_spawn_id.value)

        self._scheduled_sizes.pop(exec_spawn_id, None)
        self._randomization_enabled.pop(exec_spawn_id, None)
        self._max_market_impact_ratios.pop(exec_spawn_id, None)
        self._base_intervals.pop(exec_spawn_id, None)
        self._executed_qty.pop(exec_spawn_id, None)
        self._total_qty.pop(exec_spawn_id, None)

        self.log.info(f"已完成 VWAP 执行 {exec_spawn_id}", LogColor.BLUE)
