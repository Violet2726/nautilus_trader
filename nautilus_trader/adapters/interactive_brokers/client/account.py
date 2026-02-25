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

from ibapi.account_summary_tags import AccountSummaryTags
from ibapi.contract import Contract

from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.adapters.interactive_brokers.client.common import IBPosition
from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.common.enums import LogColor
from nautilus_trader.model.position import Position


class InteractiveBrokersClientAccountMixin(BaseMixin):
    """
    处理 InteractiveBrokersClient 的各种账户和持仓相关请求。

    参数
    ----------
    client : InteractiveBrokersClient
        用于与 TWS API 通信的客户端实例。

    """

    def accounts(self) -> set[str]:
        """
        返回此实例管理的账户标识符集合。

        返回
        -------
        set[str]

        """
        return self._account_ids.copy()

    def subscribe_account_summary(self) -> None:
        """
        订阅所有账户的账户摘要（Account Summary）。

        它向 Interactive Brokers 发送请求以检索账户摘要信息。

        """
        name = "accountSummary"

        if not (subscription := self._subscriptions.get(name=name)):
            req_id = self._next_req_id()
            subscription = self._subscriptions.add(
                req_id=req_id,
                name=name,
                handle=functools.partial(
                    self._eclient.reqAccountSummary,
                    reqId=req_id,
                    groupName="All",
                    tags=AccountSummaryTags.AllTags,
                ),
                cancel=functools.partial(
                    self._eclient.cancelAccountSummary,
                    reqId=req_id,
                ),
            )

        # 即使已经订阅，如果请求获取所有标签，也允许调用 handle
        if not subscription:
            return

        subscription.handle()

    def subscribe_positions(self) -> None:
        """
        订阅所有账户的实时持仓更新。

        这将启用对期权行权和其他外部事件引起的持仓变化的自动检测。

        """
        name = "PositionUpdates"

        if not (subscription := self._subscriptions.get(name=name)):
            subscription = self._subscriptions.add(
                req_id=self._next_req_id(),
                name=name,
                handle=self._eclient.reqPositions,
                cancel=self._eclient.cancelPositions,
            )

        if not subscription:
            return

        subscription.handle()

    def unsubscribe_positions(self) -> None:
        """
        取消订阅实时持仓更新。
        """
        name = "PositionUpdates"

        if subscription := self._subscriptions.get(name=name):
            self._subscriptions.remove(subscription.req_id)
            self._eclient.cancelPositions()

    def unsubscribe_account_summary(self, account_id: str) -> None:
        """
        取消订阅指定账户的账户摘要。此方法尚未实现。

        参数
        ----------
        account_id : str
            要取消订阅的账户标识符。

        """
        name = "accountSummary"

        if subscription := self._subscriptions.get(name=name):
            self._subscriptions.remove(subscription.req_id)
            self._eclient.cancelAccountSummary(reqId=subscription.req_id)
            self._log.debug(f"已取消订阅 {subscription}")
        else:
            self._log.debug(f"订阅 {name} 不存在")

    async def get_positions(self, account_id: str) -> list[Position]:
        """
        获取指定账户的未平仓持仓。

        参数
        ----------
        account_id: str
            要获取持仓的账户标识符。

        返回
        -------
        list[Position]

        """
        self._log.debug(f"正在请求 {account_id} 的未平仓持仓")
        name = "OpenPositions"

        if not (request := self._requests.get(name=name)):
            request = self._requests.add(
                req_id=self._next_req_id(),
                name=name,
                handle=self._eclient.reqPositions,
            )

            if not request:
                return []

            request.handle()
            all_positions = await self._await_request(request, self._request_timeout_secs)
        else:
            all_positions = await self._await_request(request, self._request_timeout_secs)

        if not all_positions:
            return []

        positions = []

        for position in all_positions:
            if position.account_id == account_id:
                positions.append(position)

        return positions

    async def process_account_summary(
        self,
        *,
        req_id: int,
        account_id: str,
        tag: str,
        value: str,
        currency: str,
    ) -> None:
        """
        接收账户信息。
        """
        name = f"accountSummary-{account_id}"

        if handler := self._event_subscriptions.get(name, None):
            handler(tag, value, currency)

    async def process_managed_accounts(self, *, accounts_list: str) -> None:
        """
        接收包含托管账户 ID 的逗号分隔字符串。

        在初始 API 客户端连接时自动发生。

        """
        self._account_ids = {a for a in accounts_list.split(",") if a}
        self._log.debug(
            f"受管理账户已设置：{self._account_ids}，下一个有效订单 ID：{self._next_valid_order_id}",
        )

        # 如果有下一个有效订单 ID，则设置连接标志
        # 在某些情况下账户可能为空，但 nextValidId 是必需的
        if self._next_valid_order_id >= 0 and not self._is_ib_connected.is_set():
            self._log.debug("在 `managedAccounts` 中设置了 `_is_ib_connected` 标志", LogColor.BLUE)
            self._is_ib_connected.set()

    async def process_position(
        self,
        *,
        account_id: str,
        contract: Contract,
        position: Decimal,
        avg_cost: float,
    ) -> None:
        """
        提供投资组合的未平仓持仓。
        """
        if request := self._requests.get(name="OpenPositions"):
            # 处理请求的持仓更新 (get_positions)
            ib_contract = IBContract(**contract.__dict__)
            request.result.append(IBPosition(account_id, ib_contract, position, avg_cost))
        elif self._subscriptions.get(name="PositionUpdates"):
            # 处理来自订阅的实时持仓更新
            ib_contract = IBContract(**contract.__dict__)
            ib_position = IBPosition(account_id, ib_contract, position, avg_cost)

            # 为注册客户端发送持仓更新事件
            if handler := self._event_subscriptions.get(f"positionUpdate-{account_id}", None):
                handler(ib_position)

    async def process_position_end(self) -> None:
        """
        指示所有持仓均已传输完毕。
        """
        if request := self._requests.get(name="OpenPositions"):
            self._end_request(request.req_id)
