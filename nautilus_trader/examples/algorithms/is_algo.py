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
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import MarketOrder
from nautilus_trader.model.orders import Order


class ISExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    ``ISExecAlgorithm`` 实例的配置类。

    该配置类定义了实现缺口 (Implementation Shortfall, IS) 执行算法所需的参数。
    IS 算法也称为"到达价格算法" (Arrival Price Algorithm)，其目标是最小化
    实际执行价格与"到达价格"（即算法收到订单时的市场价格）之间的差距。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId
        执行算法 ID（将覆盖默认值，默认值为类名）。

    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("IS")


class ISExecAlgorithm(ExecAlgorithm):
    """
    提供实现缺口 (Implementation Shortfall, IS) 执行算法。

    IS 执行算法的核心目标是在**市场冲击成本**与**时间风险成本**之间取得最优平衡：
    - 执行过快 → 市场冲击大，抬高/压低价格
    - 执行过慢 → 价格可能向不利方向漂移，增加时间风险

    算法通过 `urgency` (紧迫度) 参数来控制这一权衡：
    - urgency → 1.0：高度紧迫，大量前置执行（类似一次性市价单）
    - urgency → 0.0：低紧迫度，更均匀分散执行（类似 TWAP）

    此外，算法实时监控市场价格相对于到达价格的漂移情况：
    - 如果价格向**不利方向**漂移（买单价格上涨、卖单价格下跌），
      算法会自适应加速执行，减少进一步的不利影响
    - 如果价格向**有利方向**漂移，算法维持正常节奏

    算法工作流程：
    1. 接收主订单时，记录当前市场价格作为"到达价格" (arrival_price)
    2. 根据 urgency 参数生成前倾/均匀的执行时间表
    3. 按固定检查间隔 (interval_secs) 触发定时器
    4. 每个间隔内，根据基准调度量 + 价格漂移自适应调整量确定子订单大小
    5. 在时间范围结束时提交剩余的主订单

    Parameters
    ----------
    config : ISExecAlgorithmConfig, optional
        该实例的配置。

    """

    def __init__(self, config: ISExecAlgorithmConfig | None = None) -> None:
        if config is None:
            config = ISExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单的预计算调度数量列表
        self._scheduled_sizes: dict[ClientOrderId, list[Quantity]] = {}
        # 每个主订单的到达价格 (Decimal)
        self._arrival_prices: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的剩余待执行数量
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的订单方向
        self._order_sides: dict[ClientOrderId, OrderSide] = {}
        # 每个主订单对应的合约 ID
        self._order_instrument_ids: dict[ClientOrderId, InstrumentId] = {}
        # 每个主订单的紧迫度参数
        self._urgency: dict[ClientOrderId, float] = {}
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
        self._scheduled_sizes.clear()
        self._arrival_prices.clear()
        self._remaining_qty.clear()
        self._order_sides.clear()
        self._order_instrument_ids.clear()
        self._urgency.clear()
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

        # 获取紧迫度参数 (0.0 ~ 1.0)，默认 0.5
        urgency = exec_params.get("urgency", 0.5)
        urgency = float(urgency)
        if urgency < 0.0 or urgency > 1.0:
            self.log.error(
                f"无法执行订单：`urgency` 必须在 [0.0, 1.0] 范围内，当前值为 {urgency}",
            )
            return

        # 计算总间隔数
        num_intervals: int = math.floor(horizon_secs / interval_secs)

        # 获取到达价格 —— 算法收到订单瞬间的市场最新成交价
        arrival_price = self._get_arrival_price(order.instrument_id)
        if arrival_price is None:
            self.log.warning(
                "未获取到到达价格（无市场数据），将使用 TWAP 调度作为后备",
            )

        # 根据紧迫度生成执行调度
        scheduled_sizes = self._generate_urgency_schedule(
            total_qty=order.quantity.as_decimal(),
            num_intervals=num_intervals,
            urgency=urgency,
            instrument=instrument,
        )

        if not scheduled_sizes:
            # 无法拆分，直接提交整个订单
            self.log.warning(f"无法生成执行调度，直接提交整个订单 {order.quantity=}")
            self.submit_order(order)
            return  # 完成

        self.log.info(f"IS 执行调度: {scheduled_sizes}", LogColor.BLUE)

        # 初始化跟踪状态
        self._scheduled_sizes[order.client_order_id] = scheduled_sizes
        self._arrival_prices[order.client_order_id] = arrival_price if arrival_price else Decimal(0)
        self._remaining_qty[order.client_order_id] = order.quantity.as_decimal()
        self._order_sides[order.client_order_id] = order.side
        self._order_instrument_ids[order.client_order_id] = order.instrument_id
        self._urgency[order.client_order_id] = urgency

        # 订阅逐笔成交数据（用于监控价格漂移）
        if order.instrument_id not in self._subscribed_instruments:
            self.subscribe_trade_ticks(order.instrument_id)
            self._subscribed_instruments.add(order.instrument_id)

        # 立即提交第一个切片
        first_qty: Quantity = scheduled_sizes.pop(0)
        self._remaining_qty[order.client_order_id] -= first_qty.as_decimal()

        spawned_order: MarketOrder = self.spawn_market(
            primary=order,
            quantity=first_qty,
            time_in_force=order.time_in_force,
            reduce_only=order.is_reduce_only,
            tags=order.tags,
        )

        self.submit_order(spawned_order)

        # 设置定时器
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timedelta(seconds=interval_secs),
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 IS 执行 {order.client_order_id}："
            f"{horizon_secs=}, {interval_secs=}, {urgency=}, "
            f"arrival_price={arrival_price}",
            LogColor.BLUE,
        )

    def on_time_event(self, event: TimeEvent) -> None:
        """
        算法接收到定时事件时执行的操作。

        在每个间隔结束时，根据基准调度量和价格漂移自适应调整执行数量。

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

        # 获取调度列表
        scheduled_sizes = self._scheduled_sizes.get(exec_spawn_id)
        if scheduled_sizes is None:
            self.log.error(f"未找到 {exec_spawn_id=} 的调度列表")
            return

        if not scheduled_sizes:
            self.log.warning(f"{exec_spawn_id=} 没有更多的调度切片")
            return

        # 获取基准调度量
        base_qty: Quantity = scheduled_sizes.pop(0)
        base_qty_decimal = base_qty.as_decimal()

        # 计算价格漂移自适应调整量
        adjusted_qty_decimal = self._apply_drift_adjustment(
            exec_spawn_id,
            base_qty_decimal,
            instrument,
        )

        # 确保调整后的数量不超过剩余量
        remaining = self._remaining_qty.get(exec_spawn_id, Decimal(0))
        adjusted_qty_decimal = min(adjusted_qty_decimal, remaining)

        # 获取最小可执行数量
        min_qty_decimal = instrument.size_increment.as_decimal()
        if instrument.min_quantity:
            min_qty_decimal = max(min_qty_decimal, instrument.min_quantity.as_decimal())

        # 如果调整后的数量低于最小数量，跳过本间隔
        if adjusted_qty_decimal < min_qty_decimal:
            self.log.info(
                f"调整后数量 {adjusted_qty_decimal} 低于最小数量 {min_qty_decimal}，跳过",
                LogColor.YELLOW,
            )
            return

        # 向下取整到合约精度
        adjusted_qty_decimal = self.round_decimal_down(
            adjusted_qty_decimal,
            instrument.size_precision,
        )
        if adjusted_qty_decimal < min_qty_decimal:
            return

        # 如果这是最后一个切片，直接提交主订单
        if not scheduled_sizes:
            self.submit_order(primary)
            self.complete_sequence(primary.client_order_id)
            return

        quantity: Quantity = instrument.make_qty(adjusted_qty_decimal)

        # 更新剩余数量
        self._remaining_qty[exec_spawn_id] -= adjusted_qty_decimal

        # 生成并提交子订单
        spawned_order: MarketOrder = self.spawn_market(
            primary=primary,
            quantity=quantity,
            time_in_force=primary.time_in_force,
            reduce_only=primary.is_reduce_only,
            tags=primary.tags,
        )

        self.submit_order(spawned_order)

    def _get_arrival_price(self, instrument_id: InstrumentId) -> Decimal | None:
        """
        获取到达价格 —— 算法收到订单瞬间的市场最新成交价。

        Parameters
        ----------
        instrument_id : InstrumentId
            合约 ID。

        Returns
        -------
        Decimal or None
            到达价格，如果无市场数据则返回 None。

        """
        # 优先从最新逐笔成交获取
        if self.cache.has_trade_ticks(instrument_id):
            last_trade: TradeTick = self.cache.trade_tick(instrument_id, 0)
            if last_trade:
                return last_trade.price.as_decimal()

        # 回退到报价中间价
        if self.cache.has_quote_ticks(instrument_id):
            last_quote = self.cache.quote_tick(instrument_id, 0)
            if last_quote:
                bid = last_quote.bid_price.as_decimal()
                ask = last_quote.ask_price.as_decimal()
                return (bid + ask) / 2

        return None

    def _generate_urgency_schedule(
        self,
        total_qty: Decimal,
        num_intervals: int,
        urgency: float,
        instrument: Instrument,
    ) -> list[Quantity]:
        """
        根据紧迫度参数生成前倾或均匀的执行调度。

        使用指数衰减模型：权重 w_i = exp(-urgency_factor * i)
        - urgency = 0.0 → urgency_factor = 0 → 均匀分配（类似 TWAP）
        - urgency = 1.0 → urgency_factor 很大 → 高度前倾（大部分量集中在前几个间隔）

        Parameters
        ----------
        total_qty : Decimal
            订单总数量。
        num_intervals : int
            总间隔数。
        urgency : float
            紧迫度参数 (0.0 ~ 1.0)。
        instrument : Instrument
            合约信息。

        Returns
        -------
        list[Quantity]
            每个间隔的目标执行数量列表。

        """
        if num_intervals <= 0:
            return []

        # 将 urgency [0, 1] 映射到衰减因子 [0, 3]
        # urgency=0 → 完全均匀分配，urgency=1 → 强烈前倾
        urgency_factor = urgency * 3.0

        # 计算每个间隔的指数衰减权重
        weights: list[float] = []
        for i in range(num_intervals):
            w = math.exp(-urgency_factor * i / max(num_intervals - 1, 1))
            weights.append(w)

        # 归一化权重
        total_weight = sum(weights)
        if total_weight <= 0:
            # 安全回退到均匀分配
            weights = [1.0] * num_intervals
            total_weight = float(num_intervals)

        normalized_weights = [w / total_weight for w in weights]

        # 根据归一化权重分配数量
        scheduled_sizes: list[Quantity] = []
        allocated = Decimal(0)

        for i, nw in enumerate(normalized_weights):
            if i == num_intervals - 1:
                # 最后一个间隔分配所有剩余数量，避免精度损失
                qty_decimal = total_qty - allocated
            else:
                qty_decimal = total_qty * Decimal(str(nw))
                qty_decimal = self.round_decimal_down(qty_decimal, instrument.size_precision)

            allocated += qty_decimal

            # 检查数量是否满足最小要求
            min_qty_decimal = instrument.size_increment.as_decimal()
            if instrument.min_quantity:
                min_qty_decimal = max(min_qty_decimal, instrument.min_quantity.as_decimal())

            if qty_decimal < min_qty_decimal:
                qty_decimal = min_qty_decimal

            scheduled_sizes.append(instrument.make_qty(qty_decimal))

        # 验证总量 - 如果因取整导致超量则调整最后一个
        total_scheduled = sum(q.as_decimal() for q in scheduled_sizes)
        if total_scheduled > total_qty:
            # 重新分配最后一个切片以确保不超量
            excess = total_scheduled - total_qty
            last_qty = scheduled_sizes[-1].as_decimal() - excess
            if last_qty > 0:
                scheduled_sizes[-1] = instrument.make_qty(
                    self.round_decimal_down(last_qty, instrument.size_precision),
                )
            else:
                scheduled_sizes.pop()

        # 如果每个切片大小等于总量（无法拆分），返回空列表
        if len(scheduled_sizes) <= 1:
            return []

        return scheduled_sizes

    def _apply_drift_adjustment(
        self,
        exec_spawn_id: ClientOrderId,
        base_qty: Decimal,
        instrument: Instrument,
    ) -> Decimal:
        """
        根据价格漂移情况对基准调度量进行自适应调整。

        价格漂移 = (当前价格 - 到达价格) / 到达价格

        对于买单：
        - 价格上涨（不利漂移）→ 加速执行（增加数量）
        - 价格下跌（有利漂移）→ 维持/减缓执行

        对于卖单：
        - 价格下跌（不利漂移）→ 加速执行（增加数量）
        - 价格上涨（有利漂移）→ 维持/减缓执行

        调整公式：adjusted_qty = base_qty × (1 + drift_impact)
        - drift_impact 范围被限制在 [-0.5, 1.0]，
          避免过度调整（最多翻倍，最少减半）

        Parameters
        ----------
        exec_spawn_id : ClientOrderId
            执行订单的客户端订单 ID。
        base_qty : Decimal
            基准调度数量。
        instrument : Instrument
            合约信息。

        Returns
        -------
        Decimal
            调整后的执行数量。

        """
        arrival_price = self._arrival_prices.get(exec_spawn_id)
        if not arrival_price or arrival_price <= 0:
            # 无到达价格，不进行调整
            return base_qty

        # 获取当前市场价格
        current_price = self._get_arrival_price(
            self._order_instrument_ids.get(exec_spawn_id),
        )
        if current_price is None or current_price <= 0:
            return base_qty

        # 计算价格漂移比例
        drift_pct = float((current_price - arrival_price) / arrival_price)

        # 根据订单方向确定漂移方向的有利/不利
        order_side = self._order_sides.get(exec_spawn_id)
        if order_side == OrderSide.BUY:
            # 买单：价格上涨 (drift > 0) 是不利的，需要加速执行
            adverse_drift = drift_pct
        else:
            # 卖单：价格下跌 (drift < 0) 是不利的，需要加速执行
            adverse_drift = -drift_pct

        # 将不利漂移转化为调整系数
        # 不利漂移越大，加速越多；有利漂移时适度减缓
        # 使用 urgency 作为灵敏度放大器
        urgency = self._urgency.get(exec_spawn_id, 0.5)
        sensitivity = 5.0 * (0.5 + urgency)  # 范围 [2.5, 7.5]

        drift_impact = adverse_drift * sensitivity

        # 限制调整范围：最多翻倍 (+1.0)，最少减半 (-0.5)
        drift_impact = max(-0.5, min(1.0, drift_impact))

        adjusted_qty = base_qty * (1 + Decimal(str(drift_impact)))

        if abs(drift_impact) > 0.01:
            self.log.info(
                f"IS 价格漂移调整: arrival={arrival_price}, current={current_price}, "
                f"drift={drift_pct:.4%}, adverse={adverse_drift:.4%}, "
                f"impact={drift_impact:.4f}, base={base_qty} → adjusted={adjusted_qty}",
                LogColor.BLUE,
            )

        return adjusted_qty

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
        self._scheduled_sizes.pop(exec_spawn_id, None)
        self._arrival_prices.pop(exec_spawn_id, None)
        self._remaining_qty.pop(exec_spawn_id, None)
        self._order_sides.pop(exec_spawn_id, None)
        instrument_id = self._order_instrument_ids.pop(exec_spawn_id, None)
        self._urgency.pop(exec_spawn_id, None)

        # 如果没有其他订单使用该合约的 TradeTick，则取消订阅
        if instrument_id and not any(
            iid == instrument_id for iid in self._order_instrument_ids.values()
        ):
            self.unsubscribe_trade_ticks(instrument_id)
            self._subscribed_instruments.discard(instrument_id)

        self.log.info(f"已完成 IS 执行 {exec_spawn_id}", LogColor.BLUE)
