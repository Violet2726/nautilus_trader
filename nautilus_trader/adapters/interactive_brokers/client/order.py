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

import functools
from decimal import Decimal

from ibapi.commission_and_fees_report import CommissionAndFeesReport
from ibapi.contract import Contract
from ibapi.execution import Execution
from ibapi.execution import ExecutionFilter
from ibapi.order import Order as IBOrder
from ibapi.order_cancel import OrderCancel as IBOrderCancel
from ibapi.order_state import OrderState as IBOrderState

from nautilus_trader.adapters.interactive_brokers.client.common import AccountOrderRef
from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.adapters.interactive_brokers.client.common import get_venue_order_id
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.common.enums import LogColor
from nautilus_trader.model.identifiers import VenueOrderId


class InteractiveBrokersClientOrderMixin(BaseMixin):
    """
    为 InteractiveBrokersClient 管理订单。

    此类负责执行和管理交易。它维护内部状态，跟踪 Nautilus 订单与 IB API 订单
    之间的关系，确保下单、修改和取消等操作能正确反映在两个系统中。

    """

    _fetch_all_open_orders: bool

    def place_order(self, order: IBOrder) -> None:
        """
        通过 EClient 下单。

        参数
        ----------
        order : IBOrder
            包含订单详情（如订单 ID、合约详情和具体订单参数）的订单对象。

        """
        # 对于我们下的订单，orderId 是有效的（permId 尚未分配）
        venue_order_id = VenueOrderId(str(order.orderId))
        self._order_id_to_order_ref[venue_order_id] = AccountOrderRef(
            account_id=order.account,
            order_id=order.orderRef.rsplit(":", 1)[0],
        )
        order.orderRef = f"{order.orderRef}:{order.orderId}"
        self._eclient.placeOrder(order.orderId, order.contract, order)

    def place_order_list(self, orders: list[IBOrder]) -> None:
        """
        通过 EClient 下达一组订单。

        参数
        ----------
        orders : list[IBOrder]
            要下达的订单对象列表。

        """
        for order in orders:
            order.orderRef = f"{order.orderRef}:{order.orderId}"
            self._eclient.placeOrder(order.orderId, order.contract, order)

    def cancel_order(self, order_id: int, order_cancel: IBOrderCancel = None) -> None:
        """
        通过 EClient 取消订单。

        参数
        ----------
        order_id : int
            要取消订单的唯一标识符。
        order_cancel : OrderCancel 对象, 可选。
            根据 CME Rule 576 规定取消订单时的参数。

        """
        if order_cancel is None:
            order_cancel = IBOrderCancel()

        self._eclient.cancelOrder(order_id, order_cancel)

    def cancel_all_orders(self) -> None:
        """
        通过 EClient 请求取消所有未平仓订单。
        """
        self._log.warning(
            "正在取消所有未平仓订单，无论它们最初是如何下达的",
        )
        self._eclient.reqGlobalCancel()

    async def get_open_orders(self, account_id: str) -> list[IBOrder]:
        """
        检索特定账户的未平仓订单列表。请求完成后，将调用 openOrderEnd()。

        其行为取决于 `fetch_all_open_orders` 配置：
        - 如果为 True：使用 reqAllOpenOrders() 从所有 API 客户端、TWS/IB Gateway GUI
          和其他交易界面获取订单。
        - 如果为 False：使用 reqOpenOrders() 仅获取当前客户端 ID 会话的订单。

        参数
        ----------
        account_id : str
            检索未平仓订单的账户标识符。

        返回
        -------
        list[IBOrder]
            按指定账户 ID 过滤的未平仓订单列表。

        """
        self._log.debug(f"正在请求 {account_id} 的未结订单")
        name = "OpenOrders"

        if not (request := self._requests.get(name=name)):
            # 根据配置选择合适的处理程序
            if self._fetch_all_open_orders:
                handle = self._eclient.reqAllOpenOrders
            else:
                handle = self._eclient.reqOpenOrders

            request = self._requests.add(
                req_id=self._next_req_id(),
                name=name,
                handle=handle,
            )

            if not request:
                return []

            request.handle()

        all_orders: list[IBOrder] | None = await self._await_request(request, 30)

        if all_orders:
            orders: list[IBOrder] = [order for order in all_orders if order.account == account_id]
        else:
            orders = []

        return orders

    async def get_executions(
        self,
        account_id: str,
        execution_filter: ExecutionFilter | None = None,
    ) -> list[dict]:
        """
        检索特定账户的成交执行报告。

        参数
        ----------
        account_id : str
            检索成交记录的账户标识符。
        execution_filter : ExecutionFilter, 可选
            成交记录的过滤标准。如果为 None，将使用该账户的默认过滤器。

        返回
        -------
        list[dict]
            包含关联合约和佣金报告的成交详情列表。
            每个字典包含 'execution'、'contract' 和 'commission_report' 键。

        """
        self._log.debug(f"正在请求 {account_id} 的成交记录")
        name = f"Executions-{account_id}"

        if not (request := self._requests.get(name=name)):
            # 如果未提供，则创建成交过滤器
            if execution_filter is None:
                execution_filter = ExecutionFilter()
                execution_filter.acctCode = account_id

            req_id = self._next_req_id()
            request = self._requests.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqExecutions,
                    reqId=req_id,
                    execFilter=execution_filter,
                ),
                cancel=lambda: None,  # 成交详情请求没有取消方法
            )

            if not request:
                return []

            request.handle()

        # 等待收集成交详情
        execution_details: list[dict] | None = await self._await_request(request, 30)

        if execution_details:
            # 如果需要，按账户过滤（以防过滤器工作不完美）
            filtered_executions = [
                exec_detail
                for exec_detail in execution_details
                if exec_detail.get("execution")
                and exec_detail["execution"].acctNumber == account_id
            ]
        else:
            filtered_executions = []

        return filtered_executions

    def next_order_id(self) -> int:
        """
        检索用于新订单的下一个有效订单 ID。

        返回
        -------
        int

        """
        order_id: int = self._next_valid_order_id
        self._next_valid_order_id += 1
        self._eclient.reqIds(-1)

        return order_id

    async def process_next_valid_id(self, *, order_id: int) -> None:
        """
        接收下一个有效订单 ID。

        在 API 客户端成功连接后，或者调用 EClient::reqIds 后自动调用。
        重要提示：下一个有效订单 ID 仅在收到时有效。

        """
        self._next_valid_order_id = max(self._next_valid_order_id, order_id, 101)
        self._log.debug(
            f"下一个有效订单 ID 已设置：{self._next_valid_order_id}，账户列表：{self.accounts()}",
        )

        # 一旦有了下一个有效订单 ID 且账户信息已就位，就设置连接标志
        if (
            self._next_valid_order_id >= 0
            and self.accounts()
            and not self._is_ib_connected.is_set()
        ):
            self._log.debug(f"在 `nextValidId` 中设置了 `_is_ib_connected` 标志", LogColor.BLUE)
            self._is_ib_connected.set()

    async def process_open_order(
        self,
        *,
        order_id: int,
        contract: Contract,
        order: IBOrder,
        order_state: IBOrderState,
    ) -> None:
        """
        传入当前的未平仓订单。
        """
        order.contract = IBContract(**contract.__dict__)
        order.order_state = order_state
        order.orderRef = order.orderRef.rsplit(":", 1)[0]

        # 处理按需发起的请求响应
        if request := self._requests.get(name="OpenOrders"):
            request.result.append(order)

            # 验证并添加反向映射（如果不存在）
            venue_order_id = get_venue_order_id(order.orderId, order.permId)
            if order_ref := self._order_id_to_order_ref.get(venue_order_id):
                if not (
                    order_ref.account_id == order.account and order_ref.order_id == order.orderRef
                ):
                    self._log.warning(
                        f"在订单中发现不一致，预期为 {order_ref}，"
                        f"实际为 (account={order.account}, order_id={order.orderRef})",
                    )
            else:
                self._order_id_to_order_ref[venue_order_id] = AccountOrderRef(
                    account_id=order.account,
                    order_id=order.orderRef,
                )
            return

        # 处理基于事件的回调响应
        name = f"openOrder-{order.account}"

        if handler := self._event_subscriptions.get(name, None):
            handler(
                order_ref=order.orderRef.rsplit(":", 1)[0],
                order=order,
                order_state=order_state,
            )

    async def process_open_order_end(self) -> None:
        """
        通知未平仓订单接收结束。
        """
        if request := self._requests.get(name="OpenOrders"):
            self._end_request(request.req_id)

    async def process_order_status(
        self,
        *,
        order_id: int,
        status: str,
        filled: Decimal,
        remaining: Decimal,
        avg_fill_price: float,
        perm_id: int,
        parent_id: int,
        last_fill_price: float,
        client_id: int,
        why_held: str,
        mkt_cap_price: float,
    ) -> None:
        """
        每次订单发生变化时，获取该订单的最新信息。

        注意：经常会有重复的 orderStatus 消息。

        """
        venue_order_id = get_venue_order_id(order_id, perm_id)
        order_ref = self._order_id_to_order_ref.get(venue_order_id, None)

        if order_ref:
            name = f"orderStatus-{order_ref.account_id}"

            if handler := self._event_subscriptions.get(name, None):
                handler(
                    venue_order_id=venue_order_id,
                    order_ref=order_ref.order_id,
                    order_status=status,
                    avg_fill_price=avg_fill_price,
                    filled=filled,
                    remaining=remaining,
                )

    async def process_exec_details(
        self,
        *,
        req_id: int,
        contract: Contract,
        execution: Execution,
    ) -> None:
        """
        提供过去 24 小时内发生的成交执行。
        """
        if not (cache := self._exec_id_details.get(execution.execId, None)):
            self._exec_id_details[execution.execId] = {}
            cache = self._exec_id_details[execution.execId]

        cache["execution"] = execution
        cache["contract"] = IBContract(**contract.__dict__)
        cache["order_ref"] = execution.orderRef.rsplit(":", 1)[0]
        cache["req_id"] = req_id

        # 检查这是否是为了 get_executions 请求发出的响应
        execution_request_name = f"Executions-{execution.acctNumber}"

        if (
            (request := self._requests.get(name=execution_request_name))
            and request.req_id == req_id
            and cache.get("commission_report")
        ):
            # 将完整的成交详情添加到请求结果中
            execution_detail = {
                "execution": cache["execution"],
                "contract": cache["contract"],
                "commission_report": cache["commission_report"],
            }
            request.result.append(execution_detail)
            # 暂时不要从缓存中移除，等待 execDetailsEnd

        # 处理实时成交的基于事件的回调响应
        name = f"execDetails-{execution.acctNumber}"
        if (handler := self._event_subscriptions.get(name, None)) and cache.get(
            "commission_report",
        ):
            handler(
                order_ref=cache["order_ref"],
                execution=cache["execution"],
                commission_report=cache["commission_report"],
                contract=cache["contract"],
            )

            # 只有当不属于某个请求时，才从缓存中移除
            if not self._requests.get(name=execution_request_name):
                self._exec_id_details.pop(execution.execId, None)

    async def process_commission_report(
        self,
        *,
        commission_report: CommissionAndFeesReport,
    ) -> None:
        """
        提供某个 Execution（成交执行）的佣金和费用报告。
        """
        if not (cache := self._exec_id_details.get(commission_report.execId, None)):
            self._exec_id_details[commission_report.execId] = {}
            cache = self._exec_id_details[commission_report.execId]

        cache["commission_report"] = commission_report

        if cache.get("execution") and (account := getattr(cache["execution"], "acctNumber", None)):
            # 检查这是否是为了 get_executions 请求发出的响应
            execution_request_name = f"Executions-{account}"
            if request := self._requests.get(name=execution_request_name):
                req_id = cache.get("req_id")
                if req_id == request.req_id:
                    # 将完整的成交详情添加到请求结果中
                    execution_detail = {
                        "execution": cache["execution"],
                        "contract": cache["contract"],
                        "commission_report": cache["commission_report"],
                    }
                    request.result.append(execution_detail)
                    # 暂时不要从缓存中移除，等待 execDetailsEnd

            # 处理实时成交的基于事件的回调响应
            name = f"execDetails-{account}"
            if handler := self._event_subscriptions.get(name, None):
                handler(
                    order_ref=cache["order_ref"],
                    execution=cache["execution"],
                    commission_report=cache["commission_report"],
                    contract=cache.get("contract"),
                )

                # 只有当不属于某个请求时，才从缓存中移除
                if not self._requests.get(name=execution_request_name):
                    self._exec_id_details.pop(commission_report.execId, None)

    async def process_exec_details_end(self, req_id: int) -> None:
        """
        处理请求的所有成交已发送完毕的情况。
        """
        # 如果请求存在，则结束该请求
        if self._requests.get(req_id=req_id):
            self._end_request(req_id)
