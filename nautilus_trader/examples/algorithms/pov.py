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
from nautilus_trader.model.orders import LimitOrder
from nautilus_trader.model.orders import MarketOrder
from nautilus_trader.model.orders import Order


class POVExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    ``POVExecAlgorithm`` 实例的配置类。

    该配置类定义了参与比例 (Percentage of Volume, POV) 执行算法所需的参数。
    POV 算法以市场实际成交量的固定百分比为目标来提交子订单，
    确保算法的执行速率与市场活跃度保持同步。

    Parameters
    ----------
    exec_algorithm_id : ExecAlgorithmId
        执行算法 ID（将覆盖默认值，默认值为类名）。

    """

    exec_algorithm_id: ExecAlgorithmId | None = ExecAlgorithmId("POV")


class POVExecAlgorithm(ExecAlgorithm):
    """
    提供参与比例 (Percentage of Volume, POV) 执行算法。

    POV 执行算法的目标是按照市场成交量的固定百分比来执行订单。算法接收一个代表总数量
    和方向的主订单 (primary order)，然后通过订阅逐笔成交 (TradeTick) 数据来实时
    跟踪市场成交量，并按指定的目标参与率 (pov_rate) 来生成子订单。

    与 TWAP/VWAP 不同，POV 是一种**被动跟随型**算法：
    - TWAP 按固定时间均匀分配
    - VWAP 按历史/实时成交量分布分配
    - POV 则严格按照市场成交量的固定百分比跟随执行

    例如：若 pov_rate 设为 0.10 (10%)，当市场成交 1000 手时，算法将执行 100 手。

    算法工作流程：
    1. 接收主订单后，订阅该合约的逐笔成交 (TradeTick) 数据
    2. 按固定的检查间隔 (interval_secs) 触发定时器
    3. 在每个间隔结束时，将该间隔内累积的市场成交量乘以 pov_rate，
       作为本次应执行的子订单大小
    4. 当累计已执行数量达到主订单总量时，自动完成执行序列
    5. 如果到达最大执行时间 (max_horizon_secs) 仍有剩余数量，
       则提交剩余的主订单以确保完全执行

    Parameters
    ----------
    config : POVExecAlgorithmConfig, optional
        该实例的配置。

    """

    def __init__(self, config: POVExecAlgorithmConfig | None = None) -> None:
        if config is None:
            config = POVExecAlgorithmConfig()
        super().__init__(config)

        # 每个主订单的剩余待执行数量
        self._remaining_qty: dict[ClientOrderId, Decimal] = {}
        # 每个主订单在当前检查间隔内累积的市场成交量
        self._interval_volume: dict[ClientOrderId, Decimal] = {}
        # 每个主订单的目标参与率 (0 < pov_rate <= 1)
        self._pov_rates: dict[ClientOrderId, Decimal] = {}
        # 每个主订单对应的合约 ID，用于匹配 TradeTick
        self._order_instrument_ids: dict[ClientOrderId, InstrumentId] = {}
        # 每个主订单的最大执行时间（秒），超时后强制提交剩余量
        self._max_horizon_secs: dict[ClientOrderId, float] = {}
        # 每个主订单的启动时间戳（纳秒），用于判断是否超时
        self._start_time_ns: dict[ClientOrderId, int] = {}
        # 限制每笔子订单的最大发单量，防冲击
        self._max_slice_qty: dict[ClientOrderId, Decimal] = {}
        # 跟踪当前活跃的子订单，以便在下个切片时撤销旧单
        self._active_spawned_orders: dict[ClientOrderId, ClientOrderId] = {}
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
        self._pov_rates.clear()
        self._order_instrument_ids.clear()
        self._max_horizon_secs.clear()
        self._start_time_ns.clear()
        self._max_slice_qty.clear()
        self._active_spawned_orders.clear()
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

        # 支持市价单和限价单
        if order.order_type not in (OrderType.MARKET, OrderType.LIMIT):
            self.log.error(
                f"无法执行订单：不支持的订单类型 {order.order_type=}",
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

        # 初始化该订单的跟踪状态
        self._remaining_qty[order.client_order_id] = order.quantity.as_decimal()
        self._interval_volume[order.client_order_id] = Decimal(0)
        self._pov_rates[order.client_order_id] = pov_rate
        self._order_instrument_ids[order.client_order_id] = order.instrument_id
        self._max_horizon_secs[order.client_order_id] = float(max_horizon_secs)
        self._start_time_ns[order.client_order_id] = self.clock.timestamp_ns()
        if max_slice_qty:
            self._max_slice_qty[order.client_order_id] = Decimal(str(max_slice_qty))

        # 订阅该合约的逐笔成交数据（用于跟踪市场成交量）
        if order.instrument_id not in self._subscribed_instruments:
            self.subscribe_trade_ticks(order.instrument_id)
            self._subscribed_instruments.add(order.instrument_id)

        # 设置定时器，按固定间隔触发成交量检查
        self.clock.set_timer(
            name=order.client_order_id.value,
            interval=timedelta(seconds=interval_secs),
            callback=self.on_time_event,
        )
        self.log.info(
            f"已启动 POV 执行 {order.client_order_id}："
            f"pov_rate={pov_rate}, {interval_secs=}, {max_horizon_secs=}",
            LogColor.BLUE,
        )

    def on_trade_tick(self, tick: TradeTick) -> None:
        """
        接收到逐笔成交数据时执行的操作。

        在每个检查间隔内累积市场成交量，用于计算下一个子订单的大小。

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
        算法接收到定时事件时执行的操作。

        在每个间隔结束时，将该间隔内累积的市场成交量乘以目标参与率，
        作为本次应执行的子订单数量。

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
        max_slice = self._max_slice_qty.get(exec_spawn_id)
        if max_slice and target_qty > max_slice:
            self.log.info(f"POV 计算量 {target_qty} 超过限制 {max_slice}，进行截断", LogColor.YELLOW)
            target_qty = max_slice

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
        self._pov_rates.pop(exec_spawn_id, None)
        instrument_id = self._order_instrument_ids.pop(exec_spawn_id, None)
        self._max_horizon_secs.pop(exec_spawn_id, None)
        self._start_time_ns.pop(exec_spawn_id, None)
        self._max_slice_qty.pop(exec_spawn_id, None)
        self._active_spawned_orders.pop(exec_spawn_id, None)

        # 如果没有其他订单使用该合约的 TradeTick，则取消订阅
        if instrument_id and not any(
            iid == instrument_id for iid in self._order_instrument_ids.values()
        ):
            self.unsubscribe_trade_ticks(instrument_id)
            self._subscribed_instruments.discard(instrument_id)

        self.log.info(f"已完成 POV 执行 {exec_spawn_id}", LogColor.BLUE)
