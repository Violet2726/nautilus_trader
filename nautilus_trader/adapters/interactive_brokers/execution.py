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

import asyncio
import json
from datetime import timedelta
from decimal import Decimal
from typing import Any

import pandas as pd
from ibapi.commission_and_fees_report import CommissionAndFeesReport
from ibapi.const import UNSET_DECIMAL
from ibapi.const import UNSET_DOUBLE
from ibapi.execution import Execution
from ibapi.execution import ExecutionFilter
from ibapi.order import Order as IBOrder
from ibapi.order_condition import ExecutionCondition
from ibapi.order_condition import MarginCondition
from ibapi.order_condition import OrderCondition
from ibapi.order_condition import PercentChangeCondition
from ibapi.order_condition import PriceCondition
from ibapi.order_condition import TimeCondition
from ibapi.order_condition import VolumeCondition
from ibapi.order_state import OrderState as IBOrderState
from ibapi.tag_value import TagValue

from nautilus_trader.adapters.interactive_brokers.client import InteractiveBrokersClient
from nautilus_trader.adapters.interactive_brokers.client.common import IBPosition
from nautilus_trader.adapters.interactive_brokers.client.common import get_venue_order_id
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.adapters.interactive_brokers.common import IBOrderTags
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersExecClientConfig
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_ORDER_ACTION
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_ORDER_FIELDS
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_ORDER_STATUS
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_ORDER_TYPE
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_TIME_IN_FORCE
from nautilus_trader.adapters.interactive_brokers.parsing.execution import MAP_TRIGGER_METHOD
from nautilus_trader.adapters.interactive_brokers.parsing.execution import (
    ORDER_SIDE_TO_ORDER_ACTION,
)
from nautilus_trader.adapters.interactive_brokers.parsing.execution import timestring_to_timestamp
from nautilus_trader.adapters.interactive_brokers.parsing.price_conversion import (
    ib_price_to_nautilus_price,
)
from nautilus_trader.adapters.interactive_brokers.parsing.price_conversion import (
    nautilus_price_to_ib_price,
)
from nautilus_trader.adapters.interactive_brokers.providers import (
    InteractiveBrokersInstrumentProvider,
)
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.enums import LogLevel
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.rust.common import LogColor
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.execution.messages import BatchCancelOrders
from nautilus_trader.execution.messages import CancelAllOrders
from nautilus_trader.execution.messages import CancelOrder
from nautilus_trader.execution.messages import GenerateFillReports
from nautilus_trader.execution.messages import GenerateOrderStatusReport
from nautilus_trader.execution.messages import GenerateOrderStatusReports
from nautilus_trader.execution.messages import GeneratePositionStatusReports
from nautilus_trader.execution.messages import ModifyOrder
from nautilus_trader.execution.messages import QueryAccount
from nautilus_trader.execution.messages import SubmitOrder
from nautilus_trader.execution.messages import SubmitOrderList
from nautilus_trader.execution.reports import ExecutionMassStatus
from nautilus_trader.execution.reports import FillReport
from nautilus_trader.execution.reports import OrderStatusReport
from nautilus_trader.execution.reports import PositionStatusReport
from nautilus_trader.live.execution_client import LiveExecutionClient
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import LiquiditySide
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderStatus
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.enums import PositionSide
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TrailingOffsetType
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model.enums import order_side_to_str
from nautilus_trader.model.enums import trailing_offset_type_to_str
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TradeId
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.identifiers import generic_spread_id_n_legs
from nautilus_trader.model.identifiers import generic_spread_id_to_list
from nautilus_trader.model.identifiers import is_generic_spread_id
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import AccountBalance
from nautilus_trader.model.objects import Currency
from nautilus_trader.model.objects import MarginBalance
from nautilus_trader.model.objects import Money
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders.base import Order
from nautilus_trader.model.orders.limit_if_touched import LimitIfTouchedOrder
from nautilus_trader.model.orders.market_if_touched import MarketIfTouchedOrder
from nautilus_trader.model.orders.stop_limit import StopLimitOrder
from nautilus_trader.model.orders.stop_market import StopMarketOrder
from nautilus_trader.model.orders.trailing_stop_limit import TrailingStopLimitOrder
from nautilus_trader.model.orders.trailing_stop_market import TrailingStopMarketOrder


# 对 PriceCondition.__str__ 进行猴子补丁（monkey patch），以修复 IB API 中该属性
# 并非方法而导致的 bug。这可以防止 IB API 尝试记录订单时出现
# TypeError: 'str' object is not callable。
def _price_condition_str(self):
    """
    修复 PriceCondition 的 __str__ 方法。
    """
    try:
        return f"price {'>=' if self.isMore else '<='} {self.price}"
    except Exception:
        return "PriceCondition"


# 应用猴子补丁
if hasattr(PriceCondition, "__str__") and not callable(PriceCondition.__str__):
    PriceCondition.__str__ = _price_condition_str


ib_to_nautilus_trigger_method = dict(
    zip(MAP_TRIGGER_METHOD.values(), MAP_TRIGGER_METHOD.keys(), strict=False),
)
ib_to_nautilus_time_in_force = dict(
    zip(MAP_TIME_IN_FORCE.values(), MAP_TIME_IN_FORCE.keys(), strict=False),
)
ib_to_nautilus_order_side = dict(
    zip(MAP_ORDER_ACTION.values(), MAP_ORDER_ACTION.keys(), strict=False),
)
ib_to_nautilus_order_type = dict(zip(MAP_ORDER_TYPE.values(), MAP_ORDER_TYPE.keys(), strict=False))


class InteractiveBrokersExecutionClient(LiveExecutionClient):
    """
    为 Interactive Brokers TWS API 提供执行客户端，允许检索账户信息和执行订单。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端的事件循环。
    client : InteractiveBrokersClient
        使用 ibapi 的 Nautilus InteractiveBrokersClient 实例。
    account_id: AccountId
        与此客户端关联的账户 ID。
    msgbus : MessageBus
        客户端的消息总线。
    cache : Cache
        客户端的缓存。
    clock : LiveClock
        客户端的时钟。
    instrument_provider : InteractiveBrokersInstrumentProvider
        工具提供者。
    config : InteractiveBrokersExecClientConfig, 可选
        实例的配置。
    name : str, 可选
        自定义客户端 ID。
    connection_timeout: int, 默认 300
        连接超时时间。
    track_option_exercise_from_position_update: bool, 默认 False
        如果为 True，订阅实时持仓更新以跟踪期权行权。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client: InteractiveBrokersClient,
        account_id: AccountId,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        instrument_provider: InteractiveBrokersInstrumentProvider,
        config: InteractiveBrokersExecClientConfig,
        name: str | None = None,
        connection_timeout: int = 300,
        track_option_exercise_from_position_update: bool = False,
    ) -> None:
        # 如果未提供名称，则从 account_id issuer 派生 client_id。
        # 这确保了 client_id 与 ExecutionClient 要求的 account_id issuer 相匹配。
        client_id_str = name or account_id.get_issuer()

        super().__init__(
            loop=loop,
            client_id=ClientId(client_id_str),
            venue=None,  # 多交易所经纪商 - 改为按 account_id 路由
            oms_type=OmsType.NETTING,
            instrument_provider=instrument_provider,
            account_type=AccountType.MARGIN,
            base_currency=None,  # IB 账户支持多币种
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._filter_sec_types = instrument_provider.filter_sec_types

        # 跟踪已知头寸以检测外部变化（如期权行权）
        self._known_positions: dict[int, Decimal] = {}  # conId -> 数量
        self._connection_timeout = connection_timeout
        self._track_option_exercise_from_position_update = (
            track_option_exercise_from_position_update
        )
        self._client: InteractiveBrokersClient = client
        self._set_account_id(account_id)
        self._account_summary_tags = {
            "NetLiquidation",
            "TotalCashValue",
            "FullAvailableFunds",
            "FullInitMarginReq",
            "FullMaintMarginReq",
        }
        self._account_summary_loaded: asyncio.Event = asyncio.Event()

        # 热缓存 (Hot caches)
        self._account_summary: dict[str, dict[str, Any]] = {}

        # 跟踪已处理的成交 ID
        self._spread_fill_tracking: dict[ClientOrderId, set[str]] = {}

        # 跟踪订单的平均成交价格
        self._order_avg_prices: dict[ClientOrderId, Price] = {}

        # 跟踪来自 orderStatus 回调的已成交数量（由 VenueOrderId 索引）
        # 这是必需的，因为 IB 的 openOrder 回调不包含准确的 filledQuantity
        self._order_filled_qty: dict[VenueOrderId, Decimal] = {}

    @property
    def instrument_provider(self) -> InteractiveBrokersInstrumentProvider:
        return self._instrument_provider  # type: ignore

    async def _connect(self):
        # 连接客户端
        await self._client.wait_until_ready(self._connection_timeout)
        await self.instrument_provider.initialize()

        # 在客户端设置工具提供者，以便获取价格乘数
        self._client._instrument_provider = self._instrument_provider

        # 使用 Account 验证是否连接到了预期的 TWS/Gateway
        if self.account_id.get_id() in self._client.accounts():
            self._log.info(
                f"Account `{self.account_id.get_id()}` found in the connected TWS/Gateway",
                LogColor.GREEN,
            )
        else:
            self.fault()
            raise ValueError(
                f"Account `{self.account_id.get_id()}` not found in the connected TWS/Gateway: "
                f"available accounts are {self._client.accounts()}",
            )

        # 事件钩子
        account = self.account_id.get_id()
        self._client.registered_nautilus_clients.add(self.id)
        self._client.subscribe_event(f"accountSummary-{account}", self._on_account_summary)
        self._client.subscribe_event(f"openOrder-{account}", self._on_open_order)
        self._client.subscribe_event(f"orderStatus-{account}", self._on_order_status)
        self._client.subscribe_event(f"execDetails-{account}", self._on_exec_details)

        if self._track_option_exercise_from_position_update:
            self._client.subscribe_event(f"positionUpdate-{account}", self._on_position_update)

        # 加载账户余额
        self._client.subscribe_account_summary()
        await self._account_summary_loaded.wait()

        # 初始化已知头寸跟踪，以避免来自 execDetails 的重复处理
        await self._initialize_position_tracking()

        # 为外部变化（期权行权）订阅实时持仓更新
        if self._track_option_exercise_from_position_update:
            self._client.subscribe_positions()

        self._set_connected(True)

    async def _disconnect(self):
        self._client.registered_nautilus_clients.discard(self.id)

        if self._client.is_running and self._track_option_exercise_from_position_update:
            self._client.unsubscribe_positions()

        if self._client.is_running and self._client.registered_nautilus_clients == set():
            self._client.stop()

        self._set_connected(False)

    async def _initialize_position_tracking(self) -> None:
        """
        初始化头寸跟踪，以避免处理来自 execDetails 的重复数据。
        """
        try:
            positions = await self._client.get_positions(self.account_id.get_id())

            if positions:
                for position in positions:
                    self._known_positions[position.contract.conId] = position.quantity

                self._log.info(f"已初始化 {len(positions)} 个现有持仓的跟踪")
        except Exception as e:
            self._log.warning(f"初始化持仓跟踪失败: {e}")

    async def generate_order_status_report(
        self,
        command: GenerateOrderStatusReport,
    ) -> OrderStatusReport | None:
        PyCondition.type_or_none(command.client_order_id, ClientOrderId, "client_order_id")
        PyCondition.type_or_none(command.venue_order_id, VenueOrderId, "venue_order_id")

        if not (command.client_order_id or command.venue_order_id):
            self._log.debug("`client_order_id` 和 `venue_order_id` 不能同时为 None")
            return None

        report = None
        ib_orders = await self._client.get_open_orders(self.account_id.get_id())

        for ib_order in ib_orders:
            if (command.client_order_id and command.client_order_id.value == ib_order.orderRef) or (
                command.venue_order_id
                and command.venue_order_id.value
                == str(
                    ib_order.orderId,
                )
            ):
                report = await self._parse_ib_order_to_order_status_report(ib_order)
                break

        if report is None:
            self._log.warning(
                f"未找到订单 {command.client_order_id=}, {command.venue_order_id}，正在取消",
            )
            self._on_order_status(
                order_ref=command.client_order_id.value,
                order_status="Cancelled",
                reason="在查询中未找到",
            )

        return report

    async def _parse_ib_order_to_order_status_report(self, ib_order: IBOrder) -> OrderStatusReport:
        self._log.debug(f"正在尝试为 {ib_order.__dict__} 生成 OrderStatusReport")
        instrument = await self.instrument_provider.get_instrument(ib_order.contract)
        total_qty = (
            Quantity.from_int(0)
            if ib_order.totalQuantity == UNSET_DECIMAL
            else Quantity.from_str(str(ib_order.totalQuantity))
        )

        # 首先检查我们是否缓存了来自 orderStatus 回调的已成交数量，
        # 因为 IB 的 openOrder 回调不包含准确的 filledQuantity。
        # 使用 venue_order_id 作为键，因为对于外部订单，orderRef 可能为空。
        venue_order_id = get_venue_order_id(ib_order.orderId, ib_order.permId)
        cached_filled = self._order_filled_qty.get(venue_order_id)
        if cached_filled is not None:
            filled_qty = Quantity.from_str(str(cached_filled))
        elif ib_order.filledQuantity == UNSET_DECIMAL:
            filled_qty = Quantity.from_int(0)
        else:
            filled_qty = Quantity.from_str(str(ib_order.filledQuantity))

        if total_qty.as_double() > filled_qty.as_double() > 0:
            order_status = OrderStatus.PARTIALLY_FILLED
        else:
            order_status = MAP_ORDER_STATUS[ib_order.order_state.status]

        ts_init = self._clock.timestamp_ns()

        price_magnifier = self.instrument_provider.get_price_magnifier(instrument.id)
        price = None

        if ib_order.lmtPrice != UNSET_DOUBLE:
            converted_price = ib_price_to_nautilus_price(ib_order.lmtPrice, price_magnifier)
            price = instrument.make_price(converted_price)

        expire_time = (
            timestring_to_timestamp(ib_order.goodTillDate) if ib_order.tif == "GTD" else None
        )
        mapped_order_type_info = ib_to_nautilus_order_type[ib_order.orderType]

        if isinstance(mapped_order_type_info, tuple):
            order_type, time_in_force = mapped_order_type_info
        else:
            order_type = mapped_order_type_info
            time_in_force = ib_to_nautilus_time_in_force[ib_order.tif]

        order_status = OrderStatusReport(
            account_id=self.account_id,
            instrument_id=instrument.id,
            venue_order_id=get_venue_order_id(ib_order.orderId, ib_order.permId),
            order_side=ib_to_nautilus_order_side[ib_order.action],
            order_type=order_type,
            time_in_force=time_in_force,
            order_status=order_status,
            quantity=total_qty,
            filled_qty=filled_qty,
            avg_px=Decimal(0),
            report_id=UUID4(),
            ts_accepted=ts_init,
            ts_last=ts_init,
            ts_init=ts_init,
            client_order_id=ClientOrderId(ib_order.orderRef),
            # order_list_id=,
            # contingency_type=,
            expire_time=expire_time,
            price=price,
            trigger_price=(
                instrument.make_price(
                    ib_price_to_nautilus_price(ib_order.auxPrice, price_magnifier),
                )
                if ib_order.auxPrice != UNSET_DOUBLE
                else None
            ),
            trigger_type=TriggerType.BID_ASK,
            # limit_offset=,
            # trailing_offset=,
        )
        self._log.debug(f"已收到 {order_status!r}")

        return order_status

    async def generate_order_status_reports(  # noqa: C901 (complexity due to position adjustment logic)
        self,
        command: GenerateOrderStatusReports,
    ) -> list[OrderStatusReport]:
        report = []

        # 首先获取未平仓订单 - 启动和定期对账都需要，
        # 并且用于计算合成订单调整的未平仓订单成交量
        ib_orders: list[IBOrder] = await self._client.get_open_orders(
            self.account_id.get_id(),
        )

        # 建立 instrument_id -> 来自未平仓订单的净带符号已成交数量的映射
        # 这用于调整合成持仓订单，以避免重复计算
        # 将由未平仓订单单独处理的部分成交（修复 #3476）
        open_order_fills: dict[InstrumentId, Decimal] = {}

        for ib_order in ib_orders:
            order_status = await self._parse_ib_order_to_order_status_report(ib_order)
            report.append(order_status)

            # 跟踪各工具的已成交数量，用于合成订单调整
            if not command.open_only and order_status.filled_qty.as_decimal() > 0:
                instrument_id = order_status.instrument_id
                filled_qty = order_status.filled_qty.as_decimal()

                # 根据订单方向转换为带符号数量
                if order_status.order_side == OrderSide.BUY:
                    signed_filled = filled_qty
                else:  # SELL
                    signed_filled = -filled_qty

                if instrument_id in open_order_fills:
                    open_order_fills[instrument_id] += signed_filled
                else:
                    open_order_fills[instrument_id] = signed_filled

        # 仅在启动对账期间（当 open_only=False 时）根据持仓创建合成已成交订单。
        # 在定期一致性检查期间（open_only=True），我们应该仅返回来自 IB 的
        # 实际未平仓订单，而不是基于当前持仓状态的合成已成交订单。
        # 在定期检查期间重新生成这些合成订单会导致 filled_qty 不匹配，
        # 因为持仓可能因离场订单的部分成交而发生了变化。
        if not command.open_only:
            positions: list[IBPosition] = await self._client.get_positions(
                self.account_id.get_id(),
            )

            ts_init = self._clock.timestamp_ns()

            for position in positions:
                instrument = await self.instrument_provider.get_instrument(position.contract)

                if instrument is None:
                    if position.contract.secType in self._filter_sec_types:
                        self._log.warning(
                            f"Skipping reconciliation for filtered contract: {position.contract}",
                        )
                    else:
                        self._log.error(
                            f"Cannot generate report: instrument not found for contract ID {position.contract.conId}",
                        )
                    continue

                # 计算合成订单的调整后的数量（修复 #3476）
                # 持仓数量表示净持仓（带符号：+ve=多头，-ve=空头）
                # 我们减去未平仓订单的已成交数量以避免重复计算
                # 示例：持仓=-1，未平仓订单成交=+4（买入成交 4）
                #   已调整 = -1 - (+4) = -5，即合成卖出 5
                #   然后：合成卖出 5 (-5) + 未平仓订单买入 4 (+4) = -1 ✓
                position_qty = position.quantity
                open_fills = open_order_fills.get(instrument.id, Decimal(0))
                adjusted_qty = position_qty - open_fills

                self._log.debug(
                    f"Infer OrderStatusReport from open position {position.contract}: "
                    f"position={position_qty}, open_fills={open_fills}, adjusted={adjusted_qty}",
                )

                if adjusted_qty == 0:
                    # 所有成交已通过未平仓订单体现，不需要合成订单
                    continue

                if adjusted_qty > 0:
                    order_side = OrderSide.BUY
                else:
                    order_side = OrderSide.SELL

                contract_details = self.instrument_provider.contract_details[instrument.id]
                avg_px = instrument.make_price(
                    position.avg_cost
                    / (instrument.multiplier.as_double() * contract_details.priceMagnifier),
                ).as_decimal()
                quantity = Quantity.from_str(str(abs(adjusted_qty)))
                order_status = OrderStatusReport(
                    account_id=self.account_id,
                    instrument_id=instrument.id,
                    venue_order_id=VenueOrderId(instrument.id.value),
                    order_side=order_side,
                    order_type=OrderType.MARKET,
                    time_in_force=TimeInForce.FOK,
                    order_status=OrderStatus.FILLED,
                    quantity=quantity,
                    filled_qty=quantity,
                    avg_px=avg_px,
                    report_id=UUID4(),
                    ts_accepted=ts_init,
                    ts_last=ts_init,
                    ts_init=ts_init,
                    client_order_id=ClientOrderId(instrument.id.value),
                )
                self._log.debug(f"已收到 {order_status!r}")
                report.append(order_status)

        return report

    async def generate_fill_reports(  # noqa: C901
        self,
        command: GenerateFillReports,
    ) -> list[FillReport]:
        self._log.debug("正在请求 FillReports...")
        reports: list[FillReport] = []

        try:
            # 创建基于命令参数的执行过滤器
            execution_filter = ExecutionFilter()
            execution_filter.acctCode = self.account_id.get_id()

            # 如果指定，应用工具过滤器
            if command.instrument_id is not None:
                # 将 Nautilus 工具 ID 转换为 IB 合约以获取正确的根代码
                # IB 执行过滤器需要根合约代码（例如 "ES" 对应 "ESM4"，"EUR" 对应 "EUR/USD"）
                ib_contract = await self.instrument_provider.instrument_id_to_ib_contract(
                    command.instrument_id,
                )

                if ib_contract is not None:
                    # 使用 IB 合约的代码进行过滤
                    execution_filter.symbol = ib_contract.symbol

                    # 如果可用，还需设置 secType 以使过滤器更精确
                    if hasattr(ib_contract, "secType") and ib_contract.secType:
                        execution_filter.secType = ib_contract.secType
                else:
                    # 如果转换失败，回退到原始代码
                    self._log.warning(
                        f"无法将工具 ID {command.instrument_id} 转换为 IB 合约，"
                        f"使用原始代码 {command.instrument_id.symbol.value}",
                    )
                    execution_filter.symbol = command.instrument_id.symbol.value

            # 如果指定，应用时间过滤器
            if command.start is not None:
                # IB 期望的时间格式为 'yyyymmdd-hh:mm:ss'
                start_time = command.start.strftime("%Y%m%d-%H:%M:%S")
                execution_filter.time = start_time

            # 从 IB 获取执行详情
            execution_details = await self._client.get_executions(
                account_id=self.account_id.get_id(),
                execution_filter=execution_filter,
            )

            ts_init = self._clock.timestamp_ns()

            for exec_detail in execution_details:
                execution = exec_detail.get("execution")
                contract = exec_detail.get("contract")
                commission_report = exec_detail.get("commission_report")

                if not all([execution, contract, commission_report]):
                    self._log.warning(f"执行详情不完整: {exec_detail}")
                    continue

                # 如果指定，则按结束时间过滤
                if command.end is not None:
                    exec_time = timestring_to_timestamp(execution.time)
                    if exec_time.value > command.end.value:
                        continue

                # 获取此次执行对应的工具
                instrument = await self.instrument_provider.get_instrument(contract)
                if instrument is None:
                    self._log.warning(
                        f"无法生成成交报告：未找到合约 {contract.conId} 对应的工具",
                    )
                    continue

                # 将 IB 执行转换为 Nautilus FillReport
                try:
                    fill_report = self._create_fill_report(
                        execution=execution,
                        contract=contract,
                        commission_report=commission_report,
                        instrument=instrument,
                        ts_init=ts_init,
                    )
                    reports.append(fill_report)
                    self._log.debug(f"已生成 {fill_report}")
                except Exception as e:
                    self._log.error(
                        f"为执行 {execution.execId} 创建成交报告失败: {e}",
                    )
                    continue

            self._log_report_receipt(len(reports), "FillReport", LogLevel.INFO, "已生成")

        except Exception as e:
            self._log.error(f"生成成交报告失败: {e}")

        return reports

    def _create_fill_report(
        self,
        execution: Execution,
        contract: IBContract,
        commission_report: CommissionAndFeesReport,
        instrument,
        ts_init: int,
    ) -> FillReport:
        """
        根据 IB 执行数据创建 FillReport。
        """
        # 使用价格放大器转换价格
        price_magnifier = self.instrument_provider.get_price_magnifier(instrument.id)
        converted_execution_price = ib_price_to_nautilus_price(execution.price, price_magnifier)

        # 确定订单方向
        order_side = OrderSide[ORDER_SIDE_TO_ORDER_ACTION[execution.side]]

        # 如果可用，从订单引用创建客户端订单 ID
        client_order_id = None
        if execution.orderRef:
            # 移除 IB 添加的订单 ID 后缀
            order_ref = execution.orderRef.rsplit(":", 1)[0]
            client_order_id = ClientOrderId(order_ref)

        # 创建交易所订单 ID
        venue_order_id = get_venue_order_id(execution.orderId, execution.permId)

        # 创建交易 ID
        trade_id = TradeId(execution.execId)

        # 创建数量和价格
        last_qty = Quantity(execution.shares, precision=instrument.size_precision)
        last_px = Price(converted_execution_price, precision=instrument.price_precision)

        # 创建佣金
        commission = Money(
            commission_report.commissionAndFees,
            Currency.from_str(commission_report.currency),
        )

        # 确定流动性方向（IB 不直接提供此信息，因此我们使用 NO_LIQUIDITY_SIDE）
        liquidity_side = LiquiditySide.NO_LIQUIDITY_SIDE

        # 将执行时间转换为时间戳
        ts_event = timestring_to_timestamp(execution.time).value

        # 生成报告 ID
        report_id = UUID4()

        return FillReport(
            account_id=self.account_id,
            instrument_id=instrument.id,
            venue_order_id=venue_order_id,
            trade_id=trade_id,
            order_side=order_side,
            last_qty=last_qty,
            last_px=last_px,
            commission=commission,
            liquidity_side=liquidity_side,
            report_id=report_id,
            ts_event=ts_event,
            ts_init=ts_init,
            client_order_id=client_order_id,
            venue_position_id=None,  # IB 不在执行中提供头寸 ID
        )

    async def generate_position_status_reports(
        self,
        command: GeneratePositionStatusReports,
    ) -> list[PositionStatusReport]:
        report = []
        positions: list[IBPosition] = await self._client.get_positions(
            self.account_id.get_id(),
        )

        # 处理请求了特定工具但未找到头寸的情况
        if command.instrument_id and not positions:
            now = self._clock.timestamp_ns()
            flat_report = PositionStatusReport(
                account_id=self.account_id,
                instrument_id=command.instrument_id,
                position_side=PositionSide.FLAT,
                quantity=Quantity.zero(),
                report_id=UUID4(),
                ts_last=now,
                ts_init=now,
            )
            self._log.debug(f"已为 {command.instrument_id} 生成 FLAT 报告")
            return [flat_report]

        if not positions:
            return []

        for position in positions:
            self._log.debug(f"正在尝试为 {position.contract.conId} 生成 PositionStatusReport")

            instrument = await self.instrument_provider.get_instrument(position.contract)

            if instrument is None:
                if position.contract.secType in self._filter_sec_types:
                    self._log.warning(
                        f"Skipping reconciliation for filtered contract: {position.contract}",
                    )
                else:
                    self._log.error(
                        f"Cannot generate report: instrument not found for contract ID {position.contract.conId}",
                    )
                continue

            if not self._cache.instrument(instrument.id):
                self._msgbus.send(endpoint="DataEngine.process", msg=instrument)

            # 确定持仓方向
            if position.quantity > 0:
                side = PositionSide.LONG
            elif position.quantity < 0:
                side = PositionSide.SHORT
            # 如果持仓数量为 0，生成 FLAT 报告
            else:
                side = PositionSide.FLAT

            # 如果可用，将 avg_cost 转换为 Price
            avg_px_open = self._convert_ib_avg_cost_to_price(position.avg_cost, instrument)

            position_status = PositionStatusReport(
                account_id=self.account_id,
                instrument_id=instrument.id,
                position_side=side,
                quantity=Quantity.from_str(str(abs(position.quantity))),
                avg_px_open=avg_px_open,
                report_id=UUID4(),
                ts_last=self._clock.timestamp_ns(),
                ts_init=self._clock.timestamp_ns(),
            )
            self._log.debug(f"已收到 {position_status!r}")
            report.append(position_status)

        return report

    async def generate_mass_status(
        self,
        lookback_mins: int | None = None,
    ) -> ExecutionMassStatus | None:
        """
        生成 `ExecutionMassStatus` 报告。

        覆盖基类实现，以便从报告中派生交易所，因为 IB 是多交易所经纪商。

        Parameters
        ----------
        lookback_mins : int, 可选
            查询已平仓订单、成交和持仓时的最大回溯时间。

        Returns
        -------
        ExecutionMassStatus 或 ``None``

        """
        self._log.info("正在生成 ExecutionMassStatus...")

        self.reconciliation_active = True

        since: pd.Timestamp | None = None
        if lookback_mins is not None:
            since = self._clock.utc_now() - timedelta(minutes=lookback_mins)

        order_status_command = GenerateOrderStatusReports(
            instrument_id=None,
            start=since,
            end=None,
            open_only=False,
            command_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )
        fill_reports_command = GenerateFillReports(
            instrument_id=None,
            venue_order_id=None,
            start=since,
            end=None,
            command_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )
        position_status_command = GeneratePositionStatusReports(
            instrument_id=None,
            start=since,
            end=None,
            command_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )

        try:
            reports = await asyncio.gather(
                self.generate_order_status_reports(order_status_command),
                self.generate_fill_reports(fill_reports_command),
                self.generate_position_status_reports(position_status_command),
            )

            order_reports = reports[0]
            fill_reports = reports[1]
            position_reports = reports[2]

            # 为交易所传递 None（多交易所经纪商）——在大规模状态报告中将按 account_id 路由
            mass_status = ExecutionMassStatus(
                client_id=self.id,
                account_id=self.account_id,
                venue=None,
                report_id=UUID4(),
                ts_init=self._clock.timestamp_ns(),
            )

            mass_status.add_order_reports(reports=order_reports)
            mass_status.add_fill_reports(reports=fill_reports)
            mass_status.add_position_reports(reports=position_reports)

            return mass_status
        except Exception as e:
            self._log.exception("无法协调执行状态", e)
            return None
        finally:
            self.reconciliation_active = False

    async def _query_account(self, _command: QueryAccount) -> None:
        # 此方法触发对账户摘要信息的全新请求，
        # 接收后将更新账户余额和保证金。
        self._log.debug("正在查询账户状态")

        # 清除账户摘要缓存以强制更新
        self._account_summary.clear()
        self._account_summary_loaded.clear()

        # 请求最新的账户摘要数据
        self._client.subscribe_account_summary()

        # 等待账户摘要加载完毕，设置超时以防止死锁
        try:
            await asyncio.wait_for(
                self._account_summary_loaded.wait(),
                timeout=self._connection_timeout,
            )
        except TimeoutError:
            self._log.error(
                f"等待账户摘要超时，等待时间：{self._connection_timeout}秒",
            )
            raise

    async def _submit_order(self, command: SubmitOrder) -> None:
        PyCondition.type(command, SubmitOrder, "command")

        try:
            ib_order: IBOrder = self._transform_order_to_ib_order(command.order)
            ib_order.orderId = self._client.next_order_id()
            self._client.place_order(ib_order)
            self._handle_order_event(status=OrderStatus.SUBMITTED, order=command.order)
        except ValueError as e:
            self._handle_order_event(
                status=OrderStatus.REJECTED,
                order=command.order,
                reason=str(e),
            )

    async def _submit_order_list(self, command: SubmitOrderList) -> None:
        PyCondition.type(command, SubmitOrderList, "command")

        order_id_map = {}
        client_id_to_orders = {}
        ib_orders = []

        # 转换订单
        for order in command.order_list.orders:
            order_id_map[order.client_order_id.value] = self._client.next_order_id()
            client_id_to_orders[order.client_order_id.value] = order

            try:
                ib_order = self._transform_order_to_ib_order(order)
                ib_order.transmit = False
                ib_order.orderId = order_id_map[order.client_order_id.value]
                ib_orders.append(ib_order)
            except ValueError as e:
                # 拒绝列表中的所有订单，以防止产生非预期的副作用
                for o in command.order_list.orders:
                    if o == order:
                        self._handle_order_event(
                            status=OrderStatus.REJECTED,
                            order=o,
                            reason=str(e),
                        )
                    else:
                        self._handle_order_event(
                            status=OrderStatus.REJECTED,
                            order=o,
                            reason=f"由于列表中的订单 {order.client_order_id!r} 被拒绝，该订单也被拒绝",
                        )

                return

        # 标记最后一个订单进行传输
        ib_orders[-1].transmit = True

        for ib_order in ib_orders:
            # 映射父订单 ID
            if parent_id := order_id_map.get(ib_order.parentId):
                ib_order.parentId = parent_id

            # 下单
            order_ref = ib_order.orderRef
            self._client.place_order(ib_order)
            self._handle_order_event(
                status=OrderStatus.SUBMITTED,
                order=client_id_to_orders[order_ref],
            )

    async def _modify_order(self, command: ModifyOrder) -> None:
        PyCondition.not_none(command, "command")
        if not (command.quantity or command.price or command.trigger_price):
            return

        nautilus_order: Order = self._cache.order(command.client_order_id)
        self._log.info(f"Nautilus 订单状态为 {nautilus_order.status_string()}")

        try:
            ib_order: IBOrder = self._transform_order_to_ib_order(nautilus_order)
        except ValueError as e:
            self._handle_order_event(
                status=OrderStatus.REJECTED,
                order=nautilus_order,
                reason=str(e),
            )
            return

        ib_order.orderId = int(command.venue_order_id.value)

        if ib_order.parentId:
            parent_nautilus_order = self._cache.order(ClientOrderId(ib_order.parentId))

            if parent_nautilus_order:
                ib_order.parentId = int(parent_nautilus_order.venue_order_id.value)
            else:
                ib_order.parentId = 0

        if command.quantity and command.quantity != ib_order.totalQuantity:
            ib_order.totalQuantity = command.quantity.as_double()

        price_magnifier = self.instrument_provider.get_price_magnifier(command.instrument_id)

        if command.price and command.price.as_double() != getattr(ib_order, "lmtPrice", None):
            converted_price = nautilus_price_to_ib_price(command.price.as_double(), price_magnifier)
            ib_order.lmtPrice = converted_price

        if command.trigger_price and command.trigger_price.as_double() != getattr(
            ib_order,
            "auxPrice",
            None,
        ):
            converted_trigger_price = nautilus_price_to_ib_price(
                command.trigger_price.as_double(),
                price_magnifier,
            )
            ib_order.auxPrice = converted_trigger_price

        self._log.info(f"正在下单 {ib_order!r}")
        self._client.place_order(ib_order)

    def _transform_order_to_ib_order(self, order: Order) -> IBOrder:  # noqa: C901
        if order.is_post_only:
            raise ValueError("Interactive Brokers 不支持 `post_only`")

        is_inverse = self.instrument_provider.find(order.instrument_id).is_inverse
        if order.is_quote_quantity and not is_inverse:
            raise ValueError("不支持的报价数量 (UNSUPPORTED_QUOTE_QUANTITY)")

        ib_order = IBOrder()
        time_in_force = order.time_in_force
        price_magnifier = self.instrument_provider.get_price_magnifier(order.instrument_id)

        for key, field, fn in MAP_ORDER_FIELDS:
            if value := getattr(order, key, None):
                if key == "order_type" and time_in_force == TimeInForce.AT_THE_CLOSE:
                    setattr(ib_order, field, fn((value, time_in_force)))
                elif key == "price" and value is not None:
                    converted_price = nautilus_price_to_ib_price(value.as_double(), price_magnifier)
                    setattr(ib_order, field, converted_price)
                else:
                    setattr(ib_order, field, fn(value))

        if self.instrument_provider.find(order.instrument_id).is_inverse:
            ib_order.cashQty = int(ib_order.totalQuantity)
            ib_order.totalQuantity = 0

        if isinstance(order, TrailingStopLimitOrder | TrailingStopMarketOrder):
            if order.trailing_offset_type != TrailingOffsetType.PRICE:
                raise ValueError(
                    f"不支持 `TrailingOffsetType` {trailing_offset_type_to_str(order.trailing_offset_type)}",
                )

            ib_order.auxPrice = float(order.trailing_offset)

            if order.trigger_price:
                converted_trigger_price = nautilus_price_to_ib_price(
                    order.trigger_price.as_double(),
                    price_magnifier,
                )
                ib_order.trailStopPrice = converted_trigger_price
                ib_order.triggerMethod = MAP_TRIGGER_METHOD[order.trigger_type]
        elif (
            isinstance(
                order,
                MarketIfTouchedOrder | LimitIfTouchedOrder | StopLimitOrder | StopMarketOrder,
            )
        ) and order.trigger_price:
            converted_aux_price = nautilus_price_to_ib_price(
                order.trigger_price.as_double(),
                price_magnifier,
            )
            ib_order.auxPrice = converted_aux_price

        if is_generic_spread_id(order.instrument_id):
            bag_contract = self.instrument_provider.contract.get(order.instrument_id)

            if not bag_contract:
                raise ValueError(
                    f"未找到价差工具 {order.instrument_id} 对应的 BAG 合约",
                )

            ib_order.contract = bag_contract
        else:
            details = self.instrument_provider.contract_details[order.instrument_id]
            ib_order.contract = details.contract

        ib_order.account = self.account_id.get_id()
        ib_order.clearingAccount = self.account_id.get_id()

        if order.tags:
            return self._attach_order_tags(ib_order, order)
        else:
            return ib_order

    def _attach_order_tags(self, ib_order: IBOrder, order: Order) -> IBOrder:  # noqa: C901
        """
        将所有订单标签（包括 OCA 设置）附加到 IB 订单上。
        """
        tags: dict = {}
        oca_group_from_tags = None
        oca_type_from_tags = None

        # 从订单标签中解析 IBOrderTags
        for ot in order.tags:
            if ot.startswith("IBOrderTags:"):
                try:
                    tags = IBOrderTags.parse(ot.replace("IBOrderTags:", "")).dict()
                    break
                except Exception as e:
                    self._log.warning(f"解析 IBOrderTags 失败: {e}")

        # 处理所有标签
        for tag in tags:
            if tag == "conditions":
                conditions = self._create_ib_conditions(tags[tag])
                self._log.debug(
                    f"正在为订单设置 {len(conditions)} 个条件：{[type(c).__name__ for c in conditions]}",
                )
                ib_order.conditions = conditions
            elif tag == "conditionsCancelOrder":
                ib_order.conditionsCancelOrder = tags[tag]
            elif tag == "ocaGroup":
                oca_group_from_tags = tags[tag]
            elif tag == "ocaType":
                oca_type_from_tags = tags[tag]
            elif tag == "smartComboRoutingParams":
                ib_order.smartComboRoutingParams = [
                    TagValue(tag=param["tag"], value=param["value"]) for param in tags[tag]
                ]
            elif tag == "algoParams":
                ib_order.algoParams = [
                    TagValue(tag=param["tag"], value=param["value"]) for param in tags[tag]
                ]
            elif tag == "orderMiscOptions":
                ib_order.orderMiscOptions = [
                    TagValue(tag=param["tag"], value=param["value"]) for param in tags[tag]
                ]
            else:
                setattr(ib_order, tag, tags[tag])

        # 处理 OCA (One-Cancels-All) 设置
        if oca_group_from_tags:
            ib_order.ocaGroup = oca_group_from_tags

            # 如果标签中显式设置了 ocaType（即使为 0），则使用它；否则默认为 1
            if oca_type_from_tags is not None and oca_type_from_tags > 0:
                ib_order.ocaType = oca_type_from_tags
            else:
                ib_order.ocaType = 1  # 默认为 1 以确保安全

            self._log.info(
                f"正在根据标签设置 OCA - 组：{oca_group_from_tags}，类型：{ib_order.ocaType}",
            )

        return ib_order

    def _create_ib_conditions(
        self,
        conditions_data: list[dict],
    ) -> list[OrderCondition]:
        """
        从条件字典中创建 IB 订单条件。

        Parameters
        ----------
        conditions_data : list[dict]
            包含条件参数的条件字典列表。

        Returns
        -------
        list[OrderCondition]
            IB 订单条件对象列表。

        """
        conditions = []

        for condition_dict in conditions_data:
            condition_type = condition_dict.get("type")

            if condition_type == "price":
                condition = self._create_price_condition(condition_dict)
            elif condition_type == "time":
                condition = self._create_time_condition(condition_dict)
            elif condition_type == "margin":
                condition = self._create_margin_condition(condition_dict)
            elif condition_type == "execution":
                condition = self._create_execution_condition(condition_dict)
            elif condition_type == "volume":
                condition = self._create_volume_condition(condition_dict)
            elif condition_type == "percent_change":
                condition = self._create_percent_change_condition(condition_dict)
            else:
                self._log.warning(f"未知条件类型：{condition_type}")
                continue

            if condition:
                # 设置连接方式 (AND/OR)
                # True = AND, False = OR
                condition.isConjunctionConnection = (
                    condition_dict.get("conjunction", "and").lower() == "and"
                )
                conditions.append(condition)

        return conditions

    def _create_price_condition(self, condition_dict: dict) -> PriceCondition | None:
        """
        从条件字典中创建价格条件。
        """
        try:
            condition = PriceCondition()
            condition.conId = condition_dict.get("conId", 0)
            condition.exchange = condition_dict.get("exchange", "SMART")
            condition.isMore = condition_dict.get("isMore", True)
            condition.price = condition_dict.get("price", 0.0)
            condition.triggerMethod = condition_dict.get("triggerMethod", 0)
            return condition
        except Exception as e:
            self._log.error(f"创建价格条件失败: {e}")
            return None

    def _create_time_condition(self, condition_dict: dict) -> TimeCondition | None:
        """
        从条件字典中创建时间条件。
        """
        try:
            condition = TimeCondition()
            condition.time = condition_dict.get("time", "")
            condition.isMore = condition_dict.get("isMore", True)
            return condition
        except Exception as e:
            self._log.error(f"创建时间条件失败: {e}")
            return None

    def _create_margin_condition(self, condition_dict: dict) -> MarginCondition | None:
        """
        从条件字典中创建保证金条件。
        """
        try:
            condition = MarginCondition()
            condition.percent = condition_dict.get("percent", 0)
            condition.isMore = condition_dict.get("isMore", True)
            return condition
        except Exception as e:
            self._log.error(f"创建保证金条件失败: {e}")
            return None

    def _create_execution_condition(self, condition_dict: dict) -> ExecutionCondition | None:
        """
        从条件字典中创建执行条件。
        """
        try:
            condition = ExecutionCondition()
            condition.symbol = condition_dict.get("symbol", "")
            condition.secType = condition_dict.get("secType", "STK")
            condition.exchange = condition_dict.get("exchange", "SMART")
            return condition
        except Exception as e:
            self._log.error(f"创建执行条件失败: {e}")
            return None

    def _create_volume_condition(self, condition_dict: dict) -> VolumeCondition | None:
        """
        从条件字典中创建成交量条件。
        """
        try:
            condition = VolumeCondition()
            condition.conId = condition_dict.get("conId", 0)
            condition.exchange = condition_dict.get("exchange", "SMART")
            condition.isMore = condition_dict.get("isMore", True)
            condition.volume = condition_dict.get("volume", 0)
            return condition
        except Exception as e:
            self._log.error(f"创建成交量条件失败: {e}")
            return None

    def _create_percent_change_condition(
        self,
        condition_dict: dict,
    ) -> PercentChangeCondition | None:
        """
        从条件字典中创建百分比价格变化条件。
        """
        try:
            condition = PercentChangeCondition()
            condition.conId = condition_dict.get("conId", 0)
            condition.exchange = condition_dict.get("exchange", "SMART")
            condition.isMore = condition_dict.get("isMore", True)
            condition.changePercent = condition_dict.get("changePercent", 0.0)
            return condition
        except Exception as e:
            self._log.error(f"创建百分比变化条件失败: {e}")
            return None

    async def _cancel_order(self, command: CancelOrder) -> None:
        PyCondition.not_none(command, "command")

        venue_order_id = command.venue_order_id

        if venue_order_id:
            self._client.cancel_order(int(venue_order_id.value))
        else:
            self._log.error(f"未找到 {command.client_order_id} 对应的 VenueOrderId")

    async def _cancel_all_orders(self, command: CancelAllOrders) -> None:
        if command.order_side != OrderSide.NO_ORDER_SIDE:
            self._log.warning(
                f"Interactive Brokers 不支持撤单全部订单时的方向 (order_side) 过滤；"
                f"忽略 order_side={order_side_to_str(command.order_side)} 并撤销所有订单",
            )

        for order in self._cache.orders_open(
            instrument_id=command.instrument_id,
        ):
            venue_order_id = order.venue_order_id

            if venue_order_id:
                self._client.cancel_order(int(venue_order_id.value))
            else:
                self._log.error(f"未找到 {order.client_order_id} 对应的 VenueOrderId")

    async def _batch_cancel_orders(self, command: BatchCancelOrders) -> None:
        for order in command.cancels:
            await self._cancel_order(order)

    def _on_account_summary(self, tag: str, value: str, currency: str) -> None:
        if not self._account_summary.get(currency):
            self._account_summary[currency] = {}

        try:
            self._account_summary[currency][tag] = float(value)
        except ValueError:
            self._account_summary[currency][tag] = value

        for currency in self._account_summary:
            if not currency:
                continue

            if self._account_summary_tags - set(self._account_summary[currency].keys()) == set():
                self._log.debug(f"{self._account_summary}", LogColor.GREEN)
                cur = Currency.from_str(currency)
                total = Money(self._account_summary[currency]["NetLiquidation"], cur)
                free = Money(self._account_summary[currency]["FullAvailableFunds"], cur)
                locked = total - free

                account_balance = AccountBalance(
                    total=total,
                    free=free,
                    locked=locked,
                )
                margin_balance = MarginBalance(
                    initial=Money(
                        self._account_summary[currency]["FullInitMarginReq"],
                        currency=Currency.from_str(currency),
                    ),
                    maintenance=Money(
                        self._account_summary[currency]["FullMaintMarginReq"],
                        currency=Currency.from_str(currency),
                    ),
                )
                self.generate_account_state(
                    balances=[account_balance],
                    margins=[margin_balance],
                    reported=True,
                    ts_event=self._clock.timestamp_ns(),
                    info={
                        "TotalCashValue": self._account_summary[currency]["TotalCashValue"],
                    },
                )

                # 将所有可用字段存储到缓存中（目前的临时方案，直到有永久解决方案）
                self._cache.add(
                    f"accountSummary:{self.account_id.get_id()}",
                    json.dumps(self._account_summary, default=str).encode("utf-8"),
                )

        self._account_summary_loaded.set()

    def _handle_order_event(  # noqa: C901
        self,
        status: OrderStatus,
        order: Order,
        ib_order: IBOrder | None = None,
        reason: str = "",
    ) -> None:
        if status == OrderStatus.SUBMITTED:
            self.generate_order_submitted(
                strategy_id=order.strategy_id,
                instrument_id=order.instrument_id,
                client_order_id=order.client_order_id,
                ts_event=self._clock.timestamp_ns(),
            )
        elif status == OrderStatus.ACCEPTED:
            # 如果订单已处于 ACCEPTED 或更晚的状态（PARTIALLY_FILLED, FILLED），则跳过
            # IB 在部分成交后仍会发送 openOrder 回调，这会导致
            # 如果我们尝试再次生成 OrderAccepted，会引起无效的状态转换
            if order.status not in (
                OrderStatus.ACCEPTED,
                OrderStatus.PARTIALLY_FILLED,
                OrderStatus.FILLED,
            ):
                self.generate_order_accepted(
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=get_venue_order_id(ib_order.orderId, ib_order.permId),
                    ts_event=self._clock.timestamp_ns(),
                )
            else:
                self._log.debug(f"订单 {order.client_order_id} 已受理")
        elif status == OrderStatus.FILLED:
            if order.status != OrderStatus.FILLED:
                # 待办: self.generate_order_filled
                self._log.debug(f"订单 {order.client_order_id} 已成交")
        elif status == OrderStatus.PENDING_CANCEL:
            # 待办: self.generate_order_pending_cancel
            self._log.warning(f"订单 {order.client_order_id} 状态为 {status.name}")
        elif status == OrderStatus.CANCELED:
            if order.status != OrderStatus.CANCELED:
                self.generate_order_canceled(
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    ts_event=self._clock.timestamp_ns(),
                )
        elif status == OrderStatus.REJECTED:
            if order.status != OrderStatus.REJECTED:
                self.generate_order_rejected(
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    reason=reason,
                    ts_event=self._clock.timestamp_ns(),
                )
        else:
            self._log.warning(
                f"订单 {order.client_order_id} 状态为 {status.name}，未知或"
                "尚未实现",
            )

    async def handle_order_status_report(self, ib_order: IBOrder) -> None:
        report = await self._parse_ib_order_to_order_status_report(ib_order)
        self._send_order_status_report(report)

    def _on_open_order(self, order_ref: str, order: IBOrder, order_state: IBOrderState) -> None:
        if not order.orderRef:
            self._log.warning(
                f"ClientOrderId 不可用，订单={order.__dict__}，状态={order_state.__dict__}",
            )
            return

        if not (nautilus_order := self._cache.order(ClientOrderId(order_ref))):
            self.create_task(self.handle_order_status_report(order))
            return

        if order.whatIf and order_state.status == "PreSubmitted":
            # 待办: 此用例是否有更好的方法？
            # 这显示了初始保证金和维持保证金的变化详情，用户可以通过设置 whatIf 标志来请求。
            # 订单将不会在 IB 中实际执行，而是返回模拟结果。
            # 例如：{'status': 'PreSubmitted', 'initMarginBefore': '52.88', ...}
            self._handle_order_event(
                status=OrderStatus.REJECTED,
                order=nautilus_order,
                reason=json.dumps({"whatIf": order_state.__dict__}, default=str),
            )
        elif order_state.status in [
            "PreSubmitted",
            "Submitted",
        ]:
            instrument = self.instrument_provider.find(nautilus_order.instrument_id)
            total_qty = (
                Quantity.from_int(0)
                if order.totalQuantity == UNSET_DECIMAL
                else Quantity.from_str(str(order.totalQuantity))
            )

            if total_qty <= 0.0:
                # 这可能是由部分成交的入场括号订单触发了止损（SL）导致的。
                self._log.warning(f"IB 订单 totalQuantity <= 0，跳过: {order.__dict__}")
                return

            price_magnifier = self.instrument_provider.get_price_magnifier(
                nautilus_order.instrument_id,
            )
            price = None

            if order.lmtPrice != UNSET_DOUBLE:
                converted_price = ib_price_to_nautilus_price(order.lmtPrice, price_magnifier)
                price = instrument.make_price(converted_price)

            trigger_price = None

            if order.auxPrice != UNSET_DOUBLE:
                converted_trigger_price = ib_price_to_nautilus_price(
                    order.auxPrice,
                    price_magnifier,
                )
                trigger_price = instrument.make_price(converted_trigger_price)

            venue_order_id_modified = bool(
                nautilus_order.venue_order_id is None
                or nautilus_order.venue_order_id != get_venue_order_id(order.orderId, order.permId),
            )

            if total_qty != nautilus_order.quantity or price or trigger_price:
                self.generate_order_updated(
                    strategy_id=nautilus_order.strategy_id,
                    instrument_id=nautilus_order.instrument_id,
                    client_order_id=nautilus_order.client_order_id,
                    venue_order_id=get_venue_order_id(order.orderId, order.permId),
                    quantity=total_qty,
                    price=price,
                    trigger_price=trigger_price,
                    ts_event=self._clock.timestamp_ns(),
                    venue_order_id_modified=venue_order_id_modified,
                )
            self._handle_order_event(
                status=OrderStatus.ACCEPTED,
                order=nautilus_order,
                ib_order=order,
            )

    def _on_order_status(  # noqa: C901 (complexity unavoidable due to IB status handling)
        self,
        order_ref: str,
        order_status: str,
        avg_fill_price: float = 0.0,
        filled: Decimal = Decimal(0),
        remaining: Decimal = Decimal(0),
        reason: str = "",
        venue_order_id: VenueOrderId | None = None,
    ) -> None:
        # 缓存已成交数量，以便在对账期间生成 OrderStatusReport 时使用。
        # IB 的 openOrder 回调不包含准确的 filledQuantity，但 orderStatus 包含。
        # 使用 venue_order_id 作为键，因为对于外部订单，orderRef 可能为空。
        # 防御性地转换为 Decimal，以防 IB API 将其作为字符串发送（IB API bug/极端情况）
        filled_decimal = Decimal(filled) if not isinstance(filled, Decimal) else filled
        if filled_decimal > 0 and venue_order_id is not None:
            self._order_filled_qty[venue_order_id] = filled_decimal

        if order_status in ["ApiCancelled", "Cancelled"]:
            status = OrderStatus.CANCELED
        elif order_status == "PendingCancel":
            status = OrderStatus.PENDING_CANCEL
        elif order_status == "Rejected":
            status = OrderStatus.REJECTED
        elif order_status == "Filled":
            status = OrderStatus.FILLED
        elif order_status == "Inactive":
            self._log.warning(
                f"由于订单无效或在 {order_ref=} 时触发了错误，订单状态为 'Inactive'",
            )
            return
        elif order_status in ["PendingSubmit", "PreSubmitted", "Submitted"]:
            self._log.debug(
                f"忽略 {order_status=} 的 `_on_order_status` 事件，该事件已在 `_on_open_order` 中处理",
            )
            return
        else:
            self._log.warning(
                f"在 `_on_order_status` 中收到了未知的 {order_status=}，对应 {order_ref=}",
            )
            return

        nautilus_order = self._cache.order(ClientOrderId(order_ref))

        if nautilus_order:
            # 如果提供了平均成交价且订单已全部成交，则更新订单
            if avg_fill_price and avg_fill_price > 0 and status == OrderStatus.FILLED:
                # 使用平均成交价生成订单更新事件
                instrument = self._cache.instrument(nautilus_order.instrument_id)
                if instrument:
                    price_magnifier = self.instrument_provider.get_price_magnifier(
                        nautilus_order.instrument_id,
                    )
                    converted_avg_price = ib_price_to_nautilus_price(
                        avg_fill_price,
                        price_magnifier,
                    )
                    avg_px = instrument.make_price(converted_avg_price)

                    # 存储平均价格以便后续在成交事件中使用
                    self._order_avg_prices[nautilus_order.client_order_id] = avg_px

                    self._log.debug(
                        f"已使用 avg_px={avg_px} 更新订单 {nautilus_order.client_order_id}",
                    )

            self._handle_order_event(
                status=status,
                order=nautilus_order,
                reason=reason,
            )

            if venue_order_id is not None and status in (
                OrderStatus.FILLED,
                OrderStatus.EXPIRED,
                OrderStatus.CANCELED,
                OrderStatus.REJECTED,
            ):
                self._order_filled_qty.pop(venue_order_id, None)
        else:
            self._log.warning(f"缓存中未找到 ClientOrderId {order_ref}")

    def _on_exec_details(
        self,
        order_ref: str,
        execution: Execution,
        commission_report: CommissionAndFeesReport,
        contract: IBContract,
    ) -> None:
        if not execution.orderRef:
            self._log.warning(f"ClientOrderId 不可用，执行详情={execution.__dict__}")
            return

        client_order_id = ClientOrderId(order_ref)
        venue_order_id = get_venue_order_id(execution.orderId, execution.permId)

        # 通过 client_order_id 或 venue_order_id 查找订单
        nautilus_order = self._find_order_for_execution(client_order_id, venue_order_id)

        if not nautilus_order:
            # 未找到订单 - 执行引擎将在对账期间处理此情况
            # 记录日志并尽早返回，以避免处理不完整的执行详情
            self._log.debug(
                f"在缓存中未找到该成交对应的订单 (order_ref={order_ref}, "
                f"venue_order_id={venue_order_id}, execId={execution.execId})。 "
                f"将在对账期间进行处理。",
            )
            return

        instrument = self.instrument_provider.find(nautilus_order.instrument_id)

        if not instrument:
            self._log.error(
                f"无法处理 {nautilus_order.instrument_id} 的执行详情：未找到工具",
            )
            return

        # 检查这是否为组合/价差订单并进行相应处理
        if is_generic_spread_id(nautilus_order.instrument_id):
            self._handle_spread_execution(
                nautilus_order,
                execution,
                contract,
                commission_report,
            )
            return

        # 常规单标的订单 - 准备成交数据
        price_magnifier = self.instrument_provider.get_price_magnifier(
            nautilus_order.instrument_id,
        )
        converted_execution_price = ib_price_to_nautilus_price(
            execution.price,
            price_magnifier,
        )

        # 如果存储了平均成交价 (avg_px)，则将其包含在 info 中
        info = {}
        if nautilus_order.client_order_id in self._order_avg_prices:
            info["avg_px"] = self._order_avg_prices[nautilus_order.client_order_id]

        self.generate_order_filled(
            strategy_id=nautilus_order.strategy_id,
            instrument_id=nautilus_order.instrument_id,
            client_order_id=nautilus_order.client_order_id,
            venue_order_id=nautilus_order.venue_order_id,
            venue_position_id=None,
            trade_id=TradeId(execution.execId),
            order_side=OrderSide[ORDER_SIDE_TO_ORDER_ACTION[execution.side]],
            order_type=nautilus_order.order_type,
            last_qty=Quantity(execution.shares, precision=instrument.size_precision),
            last_px=Price(converted_execution_price, precision=instrument.price_precision),
            quote_currency=instrument.quote_currency,
            commission=Money(
                commission_report.commissionAndFees,
                Currency.from_str(commission_report.currency),
            ),
            liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
            ts_event=timestring_to_timestamp(execution.time).value,
            info=info or None,
        )

        # 更新持仓跟踪以避免重复处理
        self._update_position_tracking_from_execution(contract, execution)

    def _find_order_for_execution(
        self,
        client_order_id: ClientOrderId,
        venue_order_id: VenueOrderId | None,
    ) -> Order | None:
        # 首先尝试 client_order_id
        order = self._cache.order(client_order_id)
        if order:
            return order

        # 回退到使用 venue_order_id 查找
        if venue_order_id:
            matched_client_id = self._cache.client_order_id(venue_order_id)
            if matched_client_id:
                order = self._cache.order(matched_client_id)
                if order:
                    self._log.debug(
                        f"通过 venue_order_id {venue_order_id} "
                        f"找到了对应的 client_order_id {client_order_id}",
                    )
                    return order

        return None

    def _handle_spread_execution(
        self,
        nautilus_order: Order,
        execution: Execution,
        contract: IBContract,
        commission_report: CommissionAndFeesReport,
    ) -> None:
        """
        处理价差单执行，将腿部成交映射到组合进度和各腿部成交。
        """
        try:
            trade_id = TradeId(execution.execId)
            fill_id = str(trade_id)
            client_order_id = nautilus_order.client_order_id
            self._log.info(
                f"正在处理价差单执行：client_order_id={client_order_id}，trade_id={trade_id}",
            )

            if client_order_id not in self._spread_fill_tracking:
                self._spread_fill_tracking[client_order_id] = set()

            if fill_id in self._spread_fill_tracking[client_order_id]:
                self._log.info(f"成交 {fill_id} 已处理，跳过")
                return

            self._spread_fill_tracking[client_order_id].add(fill_id)

            if len(self._spread_fill_tracking[client_order_id]) == 1:
                # 组合成交用于订单管理，每个组合仅生成一次
                self._generate_combo_fill(
                    nautilus_order,
                    execution,
                    contract,
                    commission_report,
                )

            # 腿部成交用于更新 Nautilus 中的腿部持仓
            self._generate_leg_fill(
                nautilus_order,
                execution,
                contract,
                commission_report,
            )
        except Exception as e:
            self._log.error(f"处理价差单执行时出错：{e}")

    def _generate_combo_fill(
        self,
        nautilus_order: Order,
        execution: Execution,
        contract: IBContract,
        commission_report: CommissionAndFeesReport,
    ) -> None:
        """
        根据腿部成交生成用于订单管理的组合成交。
        """
        try:
            spread_instrument = self._cache.instrument(nautilus_order.instrument_id)

            # 提取腿部工具 ID 和比率，以计算正确的组合数量
            leg_instrument_id, ratio = self._get_leg_instrument_id_and_ratio(
                nautilus_order.instrument_id,
                contract,
            )

            # 价格
            price_magnifier = self.instrument_provider.get_price_magnifier(
                nautilus_order.instrument_id,
            )
            converted_execution_price = ib_price_to_nautilus_price(execution.price, price_magnifier)
            combo_price = Price(
                converted_execution_price,
                precision=spread_instrument.price_precision,
            )

            # 组合数量
            combo_quantity_value = execution.shares / abs(ratio)
            combo_quantity = Quantity(
                combo_quantity_value,
                precision=spread_instrument.size_precision,
            )

            # 根据成交方向和比率确定订单方向
            execution_side_numeric = (
                1 if ORDER_SIDE_TO_ORDER_ACTION[execution.side] == "BUY" else -1
            )
            leg_side_numeric = 1 if ratio >= 0 else -1
            combo_order_side = (
                OrderSide.BUY if execution_side_numeric == leg_side_numeric else OrderSide.SELL
            )

            # 组合佣金根据组合的腿数进行缩放
            combo_commission = (
                commission_report.commissionAndFees
                * generic_spread_id_n_legs(nautilus_order.instrument_id)
                / abs(ratio)
            )
            commission = Money(combo_commission, Currency.from_str(commission_report.currency))

            # 生成带有组合商品 ID 的组合成交
            self._log.info(
                f"正在生成组合成交: instrument_id={nautilus_order.instrument_id}, client_order_id={nautilus_order.client_order_id}, "
                f"execution_side={execution.side}, ratio={ratio}, combo_side={combo_order_side}",
            )

            # 如果存储了平均成交价 (avg_px)，则将其包含在 info 中
            info = {}
            if nautilus_order.client_order_id in self._order_avg_prices:
                info["avg_px"] = self._order_avg_prices[nautilus_order.client_order_id]

            self.generate_order_filled(
                strategy_id=nautilus_order.strategy_id,
                instrument_id=nautilus_order.instrument_id,  # Keep spread ID
                client_order_id=nautilus_order.client_order_id,
                venue_order_id=nautilus_order.venue_order_id,
                venue_position_id=None,
                trade_id=TradeId(execution.execId),
                order_side=combo_order_side,
                order_type=nautilus_order.order_type,
                last_qty=combo_quantity,
                last_px=combo_price,
                quote_currency=spread_instrument.quote_currency,
                commission=commission,
                liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
                ts_event=timestring_to_timestamp(execution.time).value,
                info=info or None,
            )
        except Exception as e:
            self._log.error(f"生成组合成交时出错：{e}")

    def _generate_leg_fill(
        self,
        nautilus_order: Order,
        execution: Execution,
        contract: IBContract,
        commission_report: CommissionAndFeesReport,
    ) -> None:
        """
        生成用于投资组合更新的单个腿部成交。
        """
        try:
            leg_instrument_id, ratio = self._get_leg_instrument_id_and_ratio(
                nautilus_order.instrument_id,
                contract,
            )

            if not leg_instrument_id:
                self._log.warning(f"未找到合约 {contract} 对应的腿部工具 ID")
                return

            leg_instrument = self._cache.instrument(leg_instrument_id)

            if not leg_instrument:
                self._log.warning(f"缓存中未找到腿部工具：{leg_instrument_id}")
                return

            # 为腿部成交记录分配唯一的 client_order_id，以防与组合订单冲突
            leg_client_order_id = ClientOrderId(
                f"{nautilus_order.client_order_id.value}-LEG-{leg_instrument_id.symbol}",
            )

            # 为腿部成交记录分配唯一的成交 ID (trade ID)，以避免与组合成交冲突
            spread_legs = generic_spread_id_to_list(
                nautilus_order.instrument_id,
            )  # [(instrument_id, ratio), ...]
            spread_instrument_ids = [leg[0] for leg in spread_legs]
            leg_position = (
                spread_instrument_ids.index(leg_instrument_id)
                if leg_instrument_id in spread_instrument_ids
                else 0
            )
            leg_trade_id_str = f"{execution.execId}-{leg_position}"
            leg_trade_id = TradeId(leg_trade_id_str)

            # 为腿部成交分配唯一的 venue_order_id，基于父订单的 venue_order_id
            base_venue_order_id = nautilus_order.venue_order_id
            leg_venue_order_id = VenueOrderId(f"{base_venue_order_id.value}-LEG-{leg_position}")

            price_magnifier = self.instrument_provider.get_price_magnifier(leg_instrument_id)
            converted_execution_price = ib_price_to_nautilus_price(execution.price, price_magnifier)
            price = Price(converted_execution_price, precision=leg_instrument.price_precision)

            quantity = Quantity(execution.shares, precision=leg_instrument.size_precision)

            order_side = OrderSide[ORDER_SIDE_TO_ORDER_ACTION[execution.side]]

            commission = Money(
                commission_report.commissionAndFees,
                Currency.from_str(commission_report.currency),
            )

            # 如果存储了父订单的平均成交价 (avg_px)，则将其包含在 info 中
            info = {}
            if nautilus_order.client_order_id in self._order_avg_prices:
                info["avg_px"] = self._order_avg_prices[nautilus_order.client_order_id]

            self.generate_order_filled(
                strategy_id=nautilus_order.strategy_id,
                instrument_id=leg_instrument_id,
                client_order_id=leg_client_order_id,
                venue_order_id=leg_venue_order_id,
                venue_position_id=None,
                trade_id=leg_trade_id,
                order_side=order_side,
                order_type=nautilus_order.order_type,
                last_qty=quantity,
                last_px=price,
                quote_currency=leg_instrument.quote_currency,
                commission=commission,
                liquidity_side=LiquiditySide.NO_LIQUIDITY_SIDE,
                ts_event=timestring_to_timestamp(execution.time).value,
                info=info or None,
            )

            # 更新持仓跟踪以避免重复处理
            self._update_position_tracking_from_execution(contract, execution)
        except Exception as e:
            self._log.error(f"生成腿部成交时出错：{e}")

    def _get_leg_instrument_id_and_ratio(
        self,
        spread_instrument_id: InstrumentId,
        contract: IBContract,
    ) -> tuple[InstrumentId | None, int]:
        leg_instrument_id = self.instrument_provider.contract_id_to_instrument_id.get(
            contract.conId,
        )

        if leg_instrument_id:
            leg_tuples = generic_spread_id_to_list(spread_instrument_id)

            for leg_id, ratio in leg_tuples:
                if leg_id == leg_instrument_id:
                    return leg_instrument_id, ratio

        return None, 1

    def _update_position_tracking_from_execution(self, contract: IBContract, execution) -> None:
        """
        根据成交更新持仓跟踪，以避免重复处理。
        """
        try:
            contract_id = contract.conId

            if contract_id in self._known_positions:
                # 根据成交更新跟踪的数量
                side_multiplier = 1 if execution.side == "BOT" else -1
                quantity_change = Decimal(execution.shares) * side_multiplier
                self._known_positions[contract_id] += quantity_change
        except Exception as e:
            self._log.warning(f"更新持仓跟踪失败: {e}")

    def _on_position_update(self, ib_position) -> None:
        """
        处理来自 IB 的实时持仓更新。当持仓因期权行权、指派或其他外部事件发生变化时触发。
        """
        self.create_task(self._handle_position_update(ib_position))

    async def _handle_position_update(self, ib_position) -> None:
        """
        处理持仓更新并仅针对外部变化生成持仓状态报告。这会过滤掉由正常交易（execDetails）产生的持仓更新，仅处理期权行权等外部持仓变化。
        """
        try:
            contract_id = ib_position.contract.conId
            new_quantity = ib_position.quantity

            # 跳过零持仓（IB 可能会为已平仓的持仓发送这些信息）
            if new_quantity == 0:
                # 如果持仓已平，则从跟踪中移除
                self._known_positions.pop(contract_id, None)
                return

            # 检查这是否为外部持仓变化
            known_quantity = self._known_positions.get(contract_id, Decimal(0))

            # 如果数量匹配，这很可能来自正常交易 - 跳过
            if known_quantity == new_quantity:
                return

            # 这是外部持仓变化（很可能是期权行权）
            self._log.info(
                f"检测到外部持仓变化（很可能是期权行权）: "
                f"合约 {contract_id} ({ib_position.contract.secType})，数量变化: {known_quantity} -> {new_quantity}",
                LogColor.YELLOW,
            )

            # 获取此持仓的商品信息
            instrument = await self.instrument_provider.get_instrument(ib_position.contract)

            if instrument is None:
                self._log.warning(
                    f"无法处理持仓更新：合约 ID {contract_id} 找不到对应的商品信息",
                )
                return

            # 确保商品已在缓存中
            if not self._cache.instrument(instrument.id):
                self._msgbus.send(endpoint="DataEngine.process", msg=instrument)

            # 确定持仓方向
            side = PositionSide.LONG if new_quantity > 0 else PositionSide.SHORT

            # 如果可用，将 avg_cost 转换为 Price
            avg_px_open = self._convert_ib_avg_cost_to_price(ib_position.avg_cost, instrument)

            # 创建持仓状态报告
            position_report = PositionStatusReport(
                account_id=self.account_id,
                instrument_id=instrument.id,
                position_side=side,
                quantity=instrument.make_qty(new_quantity),
                avg_px_open=avg_px_open,
                report_id=UUID4(),
                ts_last=self._clock.timestamp_ns(),
                ts_init=self._clock.timestamp_ns(),
            )

            self._log.info(
                f"期权行权持仓已创建: {instrument.id} {side} {abs(new_quantity)} @ {ib_position.avg_cost}",
                LogColor.CYAN,
            )

            # 将持仓状态报告发送给执行引擎
            self._send_position_status_report(position_report)

            # 更新跟踪
            self._known_positions[contract_id] = new_quantity
        except Exception as e:
            self._log.error(f"处理持仓更新时出错：{e}")

    def _convert_ib_avg_cost_to_price(
        self,
        avg_cost: float,
        instrument: Instrument,
    ) -> Decimal | None:
        """
        将 IB 平均成本 (avg_cost) 转换为 Nautilus 价格，计入价格放大器和乘数。如果 avg_cost 无效（<= 0 或 None），则返回 None。
        """
        if not avg_cost or avg_cost <= 0:
            return None

        contract_details = self.instrument_provider.contract_details.get(instrument.id)
        if contract_details is None:
            self._log.warning(
                f"找不到 {instrument.id} 的合约详情，无法转换 avg_cost",
            )
            return None

        price_magnifier = contract_details.priceMagnifier
        multiplier = instrument.multiplier.as_double()
        converted_avg_cost = avg_cost / (multiplier * price_magnifier)

        return Decimal(f"{converted_avg_cost:.{instrument.price_precision}f}")
