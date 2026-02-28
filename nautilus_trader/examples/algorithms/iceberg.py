"""
冰山订单 (Iceberg) 执行算法。

该算法将大订单隐藏在市场深处，每次只展示一小部分（"冰山一角"），
当该部分成交后再自动提交下一个切片，直到全部数量执行完毕。
"""

from __future__ import annotations

import random
from decimal import ROUND_DOWN, Decimal

from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.enums import OrderSide, OrderType
from nautilus_trader.model.events.order import OrderFilled
from nautilus_trader.model.identifiers import ClientOrderId, ExecAlgorithmId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import LimitOrder, MarketOrder, Order


class IcebergExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    Iceberg 执行算法配置类。

    该配置类定义了冰山订单 (Iceberg) 执行算法所需的参数。
    冰山算法将一个大订单隐藏在市场深处，每次只向市场展示一小部分（即"露出水面"的冰山一角），
    当该部分成交后再自动提交下一个切片，直到全部数量执行完毕。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId, optional
        执行算法 ID（默认："Iceberg"）。

    Notes
    -----
    此配置类定义了 Iceberg 执行算法所需的参数。
    该算法旨在将大订单拆分为多个小切片，每次只执行一个切片。
    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("Iceberg")


class IcebergExecAlgorithm(ExecAlgorithm):
    """
    冰山订单 (Iceberg) 执行算法。

    算法特点：
    - 将大订单拆分为多个固定大小的切片 (slice)
    - 每次只向市场提交一个切片，隐藏真实订单规模
    - 成交驱动型：完全由子订单的 OrderFilled 事件触发下一个切片
    - 支持数量随机化（降低被识别为算法订单的概率）
    - 支持市场深度检查（限制订单占市场深度的比例）

    与 TWAP/VWAP/POV 等基于时间或成交量驱动的算法不同，冰山订单是**成交驱动型**的：
    - 不依赖定时器或市场成交量数据
    - 在当前切片未完全成交前，不会提交新的切片

    算法工作流程：
    1. 接收主订单后，根据 slice_qty 计算切片大小
    2. 立即提交第一个切片（生成子订单）
    3. 监听 OrderFilled 事件，当子订单完全成交后提交下一个切片
    4. 最后一个切片直接提交剩余的主订单

    Parameters
    ----------
    config : IcebergExecAlgorithmConfig, optional
        算法配置实例。

    Notes
    -----
    可选的随机化功能可以让每个切片的大小在 [slice_qty * (1 - randomize_pct),
    slice_qty * (1 + randomize_pct)] 范围内随机浮动，以降低被市场对手方识别为
    算法订单的概率。
    """

    def __init__(self, config: IcebergExecAlgorithmConfig | None = None) -> None:
        """
        初始化 Iceberg 执行算法。

        Parameters
        ----------
        config : IcebergExecAlgorithmConfig, optional
            算法配置实例。
        """
        if config is None:
            config = IcebergExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单的剩余待执行数量
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的切片大小 (基准值)
        self._slice_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的随机化百分比 (0.0 ~ 0.5)
        self._randomize_pct: dict[ClientOrderId, float] = {}
        # 当前正在执行的子订单 ID -> 主订单 ID 的映射
        self._active_spawns: dict[ClientOrderId, ClientOrderId] = {}
        # 市场深度检查参数
        self._max_display_ratios: dict[ClientOrderId, float] = {}

    def on_start(self) -> None:
        """
        算法启动时执行的操作。

        Notes
        -----
        此方法在算法组件启动时被调用，可用于初始化资源。
        """
        # 可选实现

    def on_stop(self) -> None:
        """
        算法停止时执行的操作。

        Notes
        -----
        冰山算法不使用定时器，无需取消。
        """
        # 冰山算法不使用定时器，无需取消

    def on_reset(self) -> None:
        """
        算法重置时执行的操作。

        Notes
        -----
        此方法在算法组件重置时被调用，负责清空所有内部状态字典。
        """
        self._remaining_qty.clear()
        self._slice_qty.clear()
        self._randomize_pct.clear()
        self._active_spawns.clear()
        self._max_display_ratios.clear()

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
        return {}  # 可选实现

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        加载算法状态。

        Parameters
        ----------
        state : dict[str, bytes]
            算法状态字典。

        Notes
        -----
        此方法用于从持久化存储中恢复算法状态，当前实现为空操作。
        """
        # 可选实现

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
            交易工具。
        proposed_qty : Quantity
            提议的订单数量。
        max_display_ratio : float
            最大显示比例（如 0.1 表示 10%）。
        order_side : OrderSide
            订单方向（买/卖）。

        Returns
        -------
        Quantity
            经过市场深度限制调整后的数量。

        Notes
        -----
        此方法通过比较订单数量与订单簿深度，确保单笔订单不会对市场造成过大冲击。
        限制规则：订单数量不超过订单簿最佳报价数量的指定比例。
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
        算法运行中接收到订单时执行的操作。

        此方法处理主订单的初始化，包括：
        - 验证订单类型和参数
        - 解析执行参数（切片大小、随机化百分比等）
        - 验证参数合理性
        - 初始化跟踪状态
        - 立即提交第一个切片

        Parameters
        ----------
        order : Order
            主订单对象。

        Notes
        -----
        该方法负责整个 Iceberg 执行流程的初始化阶段。
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

        # 获取切片大小参数
        slice_qty = exec_params.get("slice_qty")
        if not slice_qty:
            self.log.error(
                f"无法执行订单：`exec_algorithm_params` {exec_params} 中未找到 `slice_qty`",
            )
            return

        slice_qty = Decimal(str(slice_qty))
        if slice_qty <= 0:
            self.log.error(
                f"无法执行订单：`slice_qty` 必须大于 0，当前值为 {slice_qty}",
            )
            return

        # 获取随机化百分比参数（可选，默认为 0，即不随机化）
        randomize_pct = exec_params.get("randomize_pct", 0.0)
        randomize_pct = float(randomize_pct)
        if randomize_pct < 0 or randomize_pct > 0.5:
            self.log.error(
                f"无法执行订单：`randomize_pct` 必须在 [0, 0.5] 范围内，当前值为 {randomize_pct}",
            )
            return

        # 获取市场深度检查参数（可选，默认 0.1 即 10%）
        max_display_ratio = exec_params.get("max_display_ratio", 0.1)

        order_qty = order.quantity.as_decimal()

        # 如果切片大小大于等于订单总量，则直接提交整个订单
        if slice_qty >= order_qty:
            self.log.warning(
                f"切片大小 {slice_qty} >= 订单总量 {order_qty}，直接提交整个订单",
            )
            self.submit_order(order)
            return  # 完成

        # 检查切片大小是否满足合约的最小数量要求
        slice_qty_rounded = self.round_decimal_down(slice_qty, instrument.size_precision)
        qty_check = instrument.make_qty(slice_qty_rounded)
        if (
            qty_check < instrument.size_increment
            or (instrument.min_quantity and qty_check < instrument.min_quantity)
        ):
            self.log.warning(
                f"切片大小 {qty_check} 低于合约最小数量要求，直接提交整个订单",
            )
            self.submit_order(order)
            return  # 完成

        # 初始化跟踪状态
        self._remaining_qty[order.client_order_id] = order_qty
        self._slice_qty[order.client_order_id] = slice_qty
        self._randomize_pct[order.client_order_id] = randomize_pct
        self._max_display_ratios[order.client_order_id] = max_display_ratio

        self.log.info(
            f"已启动 Iceberg 执行 {order.client_order_id}："
            f"total_qty={order_qty}, slice_qty={slice_qty}, randomize_pct={randomize_pct}, "
            f"max_display_ratio={max_display_ratio}",
            LogColor.BLUE,
        )

        # 立即提交第一个切片
        self._submit_next_slice(order, instrument)

    def on_order_filled(self, event: OrderFilled) -> None:
        """
        接收到订单成交事件时执行的操作。

        当子订单成交后，检查是否需要提交下一个切片。

        Parameters
        ----------
        event : OrderFilled
            接收到的订单成交事件。

        Notes
        -----
        该方法在每次子订单成交时被调用，负责：
        - 更新剩余数量
        - 检查是否完全成交
        - 提交下一个切片（如需要）
        """
        filled_order_id = event.client_order_id

        # 检查成交的是否是我们跟踪的子订单
        primary_order_id = self._active_spawns.get(filled_order_id)
        if primary_order_id is None:
            return  # 不是本算法管理的子订单

        # 获取成交的子订单，检查是否完全成交
        filled_order: Order = self.cache.order(filled_order_id)
        if filled_order is None:
            self.log.error(f"未找到已成交的订单 {filled_order_id}")
            return

        if not filled_order.is_closed:
            # 子订单尚未完全成交（可能是部分成交），等待继续成交
            self.log.info(
                f"子订单 {filled_order_id} 部分成交, "
                f"leaves_qty={filled_order.leaves_qty}",
                LogColor.YELLOW,
            )
            return

        # 子订单已完全成交，从活跃映射中移除
        self._active_spawns.pop(filled_order_id, None)

        # 更新剩余数量
        remaining = self._remaining_qty.get(primary_order_id)
        if remaining is None:
            self.log.error(f"未找到 {primary_order_id=} 的剩余数量")
            return

        # 注意：spawn_market 内部已经 reduce_primary，
        # 这里我们需要从自己的跟踪器中减去已成交的量
        filled_qty = filled_order.filled_qty.as_decimal()
        remaining -= filled_qty
        self._remaining_qty[primary_order_id] = remaining

        self.log.info(
            f"子订单 {filled_order_id} 已完全成交 (qty={filled_qty})，"
            f"剩余待执行: {remaining}",
            LogColor.BLUE,
        )

        if remaining <= 0:
            # 所有数量已执行完毕
            self.complete_sequence(primary_order_id)
            return

        # 获取主订单和合约信息，提交下一个切片
        primary: Order = self.cache.order(primary_order_id)
        if not primary:
            self.log.error(f"未找到主订单 {primary_order_id}")
            return

        if primary.is_closed:
            self.complete_sequence(primary_order_id)
            return

        instrument: Instrument = self.cache.instrument(primary.instrument_id)
        if not instrument:
            self.log.error(
                f"无法执行订单：未找到合约 {primary.instrument_id}",
            )
            return

        self._submit_next_slice(primary, instrument)

    def _submit_next_slice(self, primary: Order, instrument: Instrument) -> None:
        """
        计算并提交下一个冰山切片。

        如果剩余数量小于等于切片大小，则直接提交主订单完成执行序列。
        否则，根据切片大小（可选随机化）生成并提交一个子订单。

        Parameters
        ----------
        primary : Order
            主订单。
        instrument : Instrument
            合约信息。

        Notes
        -----
        该方法负责生成并提交下一个子订单，包括：
        - 计算当前切片数量（考虑随机化）
        - 应用市场深度限制
        - 生成适当的订单类型（限价单或市价单）
        - 提交订单并更新内部状态
        """
        primary_id = primary.client_order_id
        remaining = self._remaining_qty.get(primary_id, Decimal(0))
        base_slice = self._slice_qty.get(primary_id, Decimal(0))
        randomize_pct = self._randomize_pct.get(primary_id, 0.0)

        if remaining <= 0:
            self.complete_sequence(primary_id)
            return

        # 获取最小可执行数量
        min_qty_decimal = instrument.size_increment.as_decimal()
        if instrument.min_quantity:
            min_qty_decimal = max(min_qty_decimal, instrument.min_quantity.as_decimal())

        # 计算本次切片大小
        current_slice = self._calculate_slice_qty(
            base_slice,
            randomize_pct,
            remaining,
            min_qty_decimal,
            instrument.size_precision,
        )

        # 应用市场深度限制
        max_display_ratio = self._max_display_ratios.get(primary.client_order_id, 0.1)
        if max_display_ratio > 0:
            proposed_qty = instrument.make_qty(current_slice)
            limited_qty = self.check_market_depth_limit(
                instrument, proposed_qty, max_display_ratio, primary.side
            )
            current_slice = limited_qty.as_decimal()

        # 如果剩余数量小于等于本次切片，直接提交主订单
        if remaining <= current_slice or (remaining - current_slice) < min_qty_decimal:
            self.log.info(
                f"剩余数量 {remaining} 接近/等于切片大小，提交主订单完成执行",
                LogColor.BLUE,
            )
            self.submit_order(primary)
            self.complete_sequence(primary_id)
            return

        quantity: Quantity = instrument.make_qty(current_slice)

        # 生成并提交子订单
        if primary.order_type == OrderType.LIMIT:
            spawned_order = self.spawn_limit(
                primary=primary,
                quantity=quantity,
                price=primary.price,
                time_in_force=primary.time_in_force,
                reduce_only=primary.is_reduce_only,
                tags=primary.tags,
            )
        else:
            spawned_order: MarketOrder = self.spawn_market(
                primary=primary,
                quantity=quantity,
                time_in_force=primary.time_in_force,
                reduce_only=primary.is_reduce_only,
                tags=primary.tags,
            )

        # 记录子订单到主订单的映射
        self._active_spawns[spawned_order.client_order_id] = primary_id

        self.submit_order(spawned_order)

        self.log.info(
            f"提交冰山切片 {spawned_order.client_order_id}: "
            f"qty={quantity}, remaining={remaining - current_slice}",
            LogColor.BLUE,
        )

    def _calculate_slice_qty(
        self,
        base_slice: Decimal,
        randomize_pct: float,
        remaining: Decimal,
        min_qty: Decimal,
        size_precision: int,
    ) -> Decimal:
        """
        计算当前切片的数量。

        如果启用了随机化，切片大小将在 [base_slice * (1 - randomize_pct),
        base_slice * (1 + randomize_pct)] 范围内随机浮动。

        Parameters
        ----------
        base_slice : Decimal
            基准切片大小。
        randomize_pct : float
            随机化百分比 (0.0 ~ 0.5)。
        remaining : Decimal
            剩余待执行数量。
        min_qty : Decimal
            最小可执行数量。
        size_precision : int
            合约数量精度。

        Returns
        -------
        Decimal
            计算后的切片数量（已取整，且不超过剩余数量）。

        Notes
        -----
        该方法执行以下步骤：
        - 应用随机化（如启用）
        - 向下取整到合约精度
        - 确保不低于最小数量
        - 确保不超过剩余数量
        """
        if randomize_pct > 0:
            # 在 [1 - pct, 1 + pct] 范围内随机生成乘数
            lower = float(1 - Decimal(str(randomize_pct)))
            upper = float(1 + Decimal(str(randomize_pct)))
            multiplier = Decimal(str(random.uniform(lower, upper)))
            slice_qty = base_slice * multiplier
        else:
            slice_qty = base_slice

        # 向下取整到合约精度
        slice_qty = self.round_decimal_down(slice_qty, size_precision)

        # 确保不低于最小数量
        if slice_qty < min_qty:
            slice_qty = min_qty

        # 确保不超过剩余数量
        if slice_qty > remaining:
            slice_qty = remaining

        return slice_qty

    def complete_sequence(self, exec_spawn_id: ClientOrderId) -> None:
        """
        完成一个执行序列。

        清理与该订单相关的所有跟踪状态。

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            要完成的执行序列的客户端订单 ID。

        Notes
        -----
        此方法在订单执行序列完成时被调用，负责：
        - 清理所有与该订单相关的内部状态字典
        - 清理该主订单下的所有活跃子订单映射
        """
        # 清理跟踪状态
        self._remaining_qty.pop(exec_spawn_id, None)
        self._slice_qty.pop(exec_spawn_id, None)
        self._randomize_pct.pop(exec_spawn_id, None)
        self._max_display_ratios.pop(exec_spawn_id, None)

        # 清理该主订单下的所有活跃子订单映射
        spawn_ids_to_remove = [
            spawn_id
            for spawn_id, primary_id in self._active_spawns.items()
            if primary_id == exec_spawn_id
        ]
        for spawn_id in spawn_ids_to_remove:
            self._active_spawns.pop(spawn_id, None)

        self.log.info(f"已完成 Iceberg 执行 {exec_spawn_id}", LogColor.BLUE)
