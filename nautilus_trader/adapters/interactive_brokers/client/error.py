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

from inspect import iscoroutinefunction
from typing import Final

from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.common.enums import LogColor
from nautilus_trader.model.identifiers import VenueOrderId


class InteractiveBrokersClientErrorMixin(BaseMixin):
    """
    处理 InteractiveBrokersClient 的错误和警告。

    该类旨在处理并记录 InteractiveBrokersClient 运行期间遇到的各种类型的错误消息和
    警告。它对不同的错误代码进行分类并管理相应的响应，包括日志记录和状态更新。

    参考资料：
    https://ibkrcampus.com/ibkr-api-page/tws-api-error-codes/#understanding-error-codes

    """

    WARNING_CODES: Final[set[int]] = {1101, 1102, 110, 165, 202, 399, 404, 434, 492, 10167}
    CLIENT_ERRORS: Final[set[int]] = {502, 503, 504, 10038, 10182, 1100, 2110}
    CONNECTIVITY_LOST_CODES: Final[set[int]] = {1100, 1300, 2110}
    CONNECTIVITY_RESTORED_CODES: Final[set[int]] = {1101, 1102}
    ORDER_REJECTION_CODES: Final[set[int]] = {201, 203, 321, 10289, 10293}
    SUPPRESS_ERROR_LOGGING_CODES: Final[set[int]] = {200}

    async def _log_message(
        self,
        error_code: int,
        req_id: int,
        error_string: str,
        is_warning: bool,
    ) -> None:
        """
        记录提供的错误或警告消息。

        参数
        ----------
        error_code : int
            与消息关联的错误代码。
        req_id : int
            与错误或警告关联的请求 ID。
        error_string : str
            错误或警告消息字符串。
        is_warning : bool
            指示消息是警告还是错误。

        """
        msg = f"{error_string} (code: {error_code}, {req_id=})"

        if error_code in self.SUPPRESS_ERROR_LOGGING_CODES:
            self._log.debug(msg)
        else:
            self._log.warning(msg) if is_warning else self._log.error(msg)

    async def process_error(
        self,
        *,
        req_id: int,
        error_time: int,
        error_code: int,
        error_string: str,
        advanced_order_reject_json: str = "",
    ) -> None:
        """
        根据错误代码、请求 ID 和消息处理错误。根据错误代码，此方法会委托给特定的
        错误处理程序或执行通用的错误处理。

        参数
        ----------
        req_id : int
            与错误关联的请求 ID。
        error_time : int
            错误发生时的时间戳。
        error_code : int
            错误代码。
        error_string : str
            错误消息字符串。
        advanced_order_reject_json : str
            高级订单拒绝的 JSON 字符串。

        """
        is_warning = error_code in self.WARNING_CODES or 2100 <= error_code < 2200
        error_string = error_string.replace("\n", " ")
        await self._log_message(error_code, req_id, error_string, is_warning)

        if req_id != -1:
            if self._subscriptions.get(req_id=req_id):
                await self._handle_subscription_error(req_id, error_code, error_string)
            elif self._requests.get(req_id=req_id):
                await self._handle_request_error(req_id, error_code, error_string)
            elif VenueOrderId(str(req_id)) in self._order_id_to_order_ref:
                await self._handle_order_error(req_id, error_code, error_string)
            else:
                self._log.warning(f"未处理的错误：代码 {error_code}，针对 req_id {req_id}")
        elif error_code in self.CLIENT_ERRORS or error_code in self.CONNECTIVITY_LOST_CODES:
            if self._is_ib_connected.is_set():
                self._log.debug(
                    f"在 `_process_error` 中代码 {error_code} 取消了 `_is_ib_connected` 状态",
                    LogColor.BLUE,
                )
                self._is_ib_connected.clear()
        elif error_code in self.CONNECTIVITY_RESTORED_CODES and not self._is_ib_connected.is_set():
            self._log.debug(
                f"在 `_process_error` 中代码 {error_code} 设置了 `_is_ib_connected` 状态",
                LogColor.BLUE,
            )
            self._is_ib_connected.set()

    async def _handle_subscription_error(
        self,
        req_id: int,
        error_code: int,
        error_string: str,
    ) -> None:
        """
        处理特定于数据订阅的错误。处理与订阅相关的错误并采取适当措施，
        例如取消订阅或清除标志。

        参数
        ----------
        req_id : int
            与订阅错误关联的请求 ID。
        error_code : int
            错误代码。
        error_string : str
            错误消息字符串。

        """
        subscription = self._subscriptions.get(req_id=req_id)

        if not subscription:
            return

        if error_code in [10189, 366, 102]:
            # 处理特定订阅相关的错误代码
            self._log.warning(f"{error_code}: {error_string}")
            subscription.cancel()

            if iscoroutinefunction(subscription.handle):
                self._create_task(subscription.handle())
            else:
                subscription.handle()
        elif error_code == 10182:
            # 处理断开连接错误
            self._log.warning(f"{error_code}: {error_string}")

            if self._is_ib_connected.is_set():
                self._log.info(
                    f"在 `_handle_subscription_error` 中 {subscription.name} 取消了 `_is_ib_connected` 状态",
                )
                self._is_ib_connected.clear()
            # 记录未知的订阅错误
            self._log.warning(
                f"未知订阅错误：代码 {error_code}，针对 req_id {req_id}",
            )

    async def _handle_request_error(self, req_id: int, error_code: int, error_string: str) -> None:
        """
        处理与常规请求相关的错误。记录错误并结束与给定请求 ID 关联的请求。

        参数
        ----------
        req_id : int
            与错误关联的请求 ID。
        error_code : int
            错误代码。
        error_string : str
            错误消息字符串。

        """
        request = self._requests.get(req_id=req_id)

        if error_code == 200:
            self._log.debug(f"{error_code}: {error_string}, {request}")
        else:
            self._log.warning(f"{error_code}: {error_string}, {request}")

        self._end_request(req_id, success=False)

    async def _handle_order_error(self, req_id: int, error_code: int, error_string: str) -> None:
        """
        处理订单相关的错误。管理各种订单相关的错误，包括拒绝和取消，并视情况
        记录或转发它们。

        参数
        ----------
        req_id : int
            与订单错误关联的请求 ID。
        error_code : int
            错误代码。
        error_string : str
            错误消息字符串。

        """
        # 使用 VenueOrderId 作为字典键（对于我们下的订单，req_id 是有效的 orderId）
        order_ref = self._order_id_to_order_ref.get(VenueOrderId(str(req_id)), None)

        if not order_ref:
            self._log.warning(f"未找到 req_id {req_id} 的订单引用")
            return

        name = f"orderStatus-{order_ref.account_id}"
        handler = self._event_subscriptions.get(name, None)

        if error_code in self.ORDER_REJECTION_CODES:
            # 处理各种订单拒绝情况
            if handler:
                handler(order_ref=order_ref.order_id, order_status="Rejected", reason=error_string)
        elif error_code == 202:
            # 处理订单取消警告
            if handler:
                handler(order_ref=order_ref.order_id, order_status="Cancelled", reason=error_string)
        else:
            # 记录未知的订单警告/错误
            self._log.warning(
                f"未处理的订单警告或错误代码：{error_code} (req_id {req_id}) - "
                f"{error_string}",
            )
