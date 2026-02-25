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
"""
`LiveExecutionClient` 类负责与特定的 API 进行接口对接，该 API 可能由交易场（venue）直接提供，也可能通过经纪商（broker）中间机构提供。
"""

import asyncio
import functools
from asyncio import Task
from collections.abc import Callable
from collections.abc import Coroutine
from datetime import timedelta
from weakref import WeakSet

import pandas as pd

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.enums import LogLevel
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.nautilus_pyo3 import MILLISECONDS_IN_SECOND
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.execution.client import ExecutionClient
from nautilus_trader.execution.messages import BatchCancelOrders
from nautilus_trader.execution.messages import CancelAllOrders
from nautilus_trader.execution.messages import CancelOrder
from nautilus_trader.execution.messages import GenerateFillReports
from nautilus_trader.execution.messages import GenerateOrderStatusReport
from nautilus_trader.execution.messages import GenerateOrderStatusReports
from nautilus_trader.execution.messages import GeneratePositionStatusReports
from nautilus_trader.execution.messages import ModifyOrder
from nautilus_trader.execution.messages import QueryAccount
from nautilus_trader.execution.messages import QueryOrder
from nautilus_trader.execution.messages import SubmitOrder
from nautilus_trader.execution.messages import SubmitOrderList
from nautilus_trader.execution.reports import ExecutionMassStatus
from nautilus_trader.execution.reports import FillReport
from nautilus_trader.execution.reports import OrderStatusReport
from nautilus_trader.execution.reports import PositionStatusReport
from nautilus_trader.live.cancellation import cancel_tasks_with_timeout
from nautilus_trader.model.enums import AccountType
from nautilus_trader.model.enums import OmsType
from nautilus_trader.model.enums import order_side_to_str
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.model.objects import Currency


class LiveExecutionClient(ExecutionClient):
    """
    所有实盘执行客户端的基类。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端的事件循环。
    client_id : ClientId
        客户端 ID。
    venue : Venue 或 ``None``
        客户端场地。如果是多场地，则可以为 ``None``。
    instrument_provider : InstrumentProvider
        客户端的标的提供者。
    account_type : AccountType
        客户端的账户类型。
    base_currency : Currency, 可选
        客户端的账户基础货币。对于多货币账户，请使用 ``None``。
    msgbus : MessageBus
        客户端的消息总线。
    cache : Cache
        客户端的缓存。
    clock : LiveClock
        客户端的时钟。
    config : NautilusConfig, 可选
        实例的配置。

    异常
    ------
    ValueError
        如果 `oms_type` 为 ``UNSPECIFIED``（必须指定）。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client_id: ClientId,
        venue: Venue | None,
        oms_type: OmsType,
        account_type: AccountType,
        base_currency: Currency | None,
        instrument_provider: InstrumentProvider,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        config: NautilusConfig | None = None,
    ) -> None:
        PyCondition.type(instrument_provider, InstrumentProvider, "instrument_provider")

        super().__init__(
            client_id=client_id,
            venue=venue,
            oms_type=oms_type,
            account_type=account_type,
            base_currency=base_currency,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop = loop
        self._tasks: WeakSet[asyncio.Task] = WeakSet()
        self._instrument_provider = instrument_provider

        self.reconciliation_active = False

    async def run_after_delay(
        self,
        delay: float,
        coro: Coroutine,
    ) -> None:
        """
        延迟一定时间后运行给定的协程。

        参数
        ----------
        delay : float
            运行协程前的延迟（秒）。
        coro : Coroutine
            在初始延迟后运行的协程。

        """
        await asyncio.sleep(delay)
        return await coro

    def create_task(
        self,
        coro: Coroutine,
        log_msg: str | None = None,
        actions: Callable | None = None,
        success_msg: str | None = None,
        success_color: LogColor = LogColor.NORMAL,
    ) -> asyncio.Task:
        """
        运行给定的协程，并包含错误处理和完成后可选的回调动作。

        参数
        ----------
        coro : Coroutine
            要运行的协程。
        log_msg : str, 可选
            任务的日志消息。
        actions : Callable, 可选
            协程完成时要运行的回调动作。
        success_msg : str, 可选
            动作成功完成后要写入的日志消息。
        success_color : str, 默认 ``NORMAL``
            动作成功完成后日志消息的颜色。

        返回
        -------
        asyncio.Task

        """
        task_name = log_msg or getattr(coro, "__name__", None) or coro.__class__.__name__
        self._log.debug(f"正在创建任务 '{task_name}'")
        task = self._loop.create_task(
            coro,
            name=task_name,
        )
        task.add_done_callback(
            functools.partial(
                self._on_task_completed,
                actions,
                success_msg,
                success_color,
            ),
        )
        self._tasks.add(task)
        return task

    def _on_task_completed(
        self,
        actions: Callable | None,
        success_msg: str | None,
        success_color: LogColor,
        task: Task,
    ) -> None:
        try:
            e: BaseException | None = task.exception()
        except asyncio.CancelledError:
            self._log.warning(f"任务 '{task.get_name()}' 已取消")
            return

        if e:
            self._log.exception(f"任务 '{task.get_name()}' 出错", e)
        else:
            if actions:
                try:
                    actions()
                except Exception as e:
                    self._log.exception(
                        f"在任务 '{task.get_name()}' 后触发动作 {actions.__name__} 失败",
                        e,
                    )
            if success_msg:
                self._log.info(success_msg, success_color)

    def connect(self) -> None:
        """
        连接客户端。
        """
        self._log.info("正在连接...")
        self.create_task(
            self._connect(),
            actions=lambda: self._set_connected(True),
            success_msg="已连接",
            success_color=LogColor.GREEN,
        )

    def disconnect(self) -> None:
        """
        断开客户端连接。
        """
        self._log.info("正在断开连接...")

        async def _disconnect_with_cleanup():
            await self._disconnect()
            await self.cancel_pending_tasks()
            self._set_connected(False)
            self._log.info("已断开连接", LogColor.GREEN)

        self._loop.create_task(_disconnect_with_cleanup())

    async def cancel_pending_tasks(self, timeout_secs: float = 5.0) -> None:
        """
        取消所有待处理任务并等待其完成。

        参数
        ----------
        timeout_secs : float, 默认 5.0
            等待任务取消的超时时间（秒）。

        """
        await cancel_tasks_with_timeout(self._tasks, self._log, timeout_secs)

    def submit_order(self, command: SubmitOrder) -> None:
        self._log.info(f"提交 {command.order}", LogColor.BLUE)
        self.create_task(
            self._submit_order(command),
            log_msg=f"submit_order: {command}",
        )

    def submit_order_list(self, command: SubmitOrderList) -> None:
        self._log.info(f"提交 {command.order_list}", LogColor.BLUE)
        self.create_task(
            self._submit_order_list(command),
            log_msg=f"submit_order_list: {command}",
        )

    def modify_order(self, command: ModifyOrder) -> None:
        venue_order_id_str = (
            " " + repr(command.venue_order_id) if command.venue_order_id is not None else ""
        )
        self._log.info(f"修改 {command.client_order_id!r}{venue_order_id_str}", LogColor.BLUE)
        self.create_task(
            self._modify_order(command),
            log_msg=f"modify_order: {command}",
        )

    def cancel_order(self, command: CancelOrder) -> None:
        venue_order_id_str = (
            " " + repr(command.venue_order_id) if command.venue_order_id is not None else ""
        )
        self._log.info(f"取消 {command.client_order_id!r}{venue_order_id_str}", LogColor.BLUE)
        self.create_task(
            self._cancel_order(command),
            log_msg=f"cancel_order: {command}",
        )

    def cancel_all_orders(self, command: CancelAllOrders) -> None:
        side_str = f" {order_side_to_str(command.order_side)} " if command.order_side else " "
        self._log.info(f"取消所有{side_str}订单", LogColor.BLUE)
        self.create_task(
            self._cancel_all_orders(command),
            log_msg=f"cancel_all_orders: {command}",
        )

    def batch_cancel_orders(self, command: BatchCancelOrders) -> None:
        self._log.info(
            f"批量取消订单 {[repr(c.client_order_id) for c in command.cancels]}",
            LogColor.BLUE,
        )
        self.create_task(
            self._batch_cancel_orders(command),
            log_msg=f"batch_cancel_orders: {command}",
        )

    def query_account(self, command: QueryAccount) -> None:
        self._log.info(f"查询 {command.account_id!r}", LogColor.BLUE)
        self.create_task(
            self._query_account(command),
            log_msg=f"query_account: {command}",
        )

    def query_order(self, command: QueryOrder) -> None:
        self._log.info(f"查询 {command.client_order_id!r}", LogColor.BLUE)
        self.create_task(
            self._query_order(command),
            log_msg=f"query_order: {command}",
        )

    async def generate_order_status_report(
        self,
        command: GenerateOrderStatusReport,
    ) -> OrderStatusReport | None:
        """
        根据给定的订单标识参数生成 `OrderStatusReport`。

        如果未找到订单或发生错误，则记录日志并返回 ``None``。

        参数
        ----------
        command : GenerateOrderStatusReport
            生成报告的命令。

        返回
        -------
        OrderStatusReport 或 ``None``

        异常
        ------
        ValueError
            如果 `client_order_id` 和 `venue_order_id` 均为 ``None``。

        """
        raise NotImplementedError(
            "方法 `generate_order_status_report` 必须在子类中实现",
        )  # pragma: no cover

    async def generate_order_status_reports(
        self,
        command: GenerateOrderStatusReports,
    ) -> list[OrderStatusReport]:
        """
        生成带有可选查询过滤条件的 `OrderStatusReport` 列表。

        如果没有订单匹配给定参数，则返回的列表可能为空。

        参数
        ----------
        command : GenerateOrderStatusReports
            生成报告的命令。

        返回
        -------
        list[OrderStatusReport]

        """
        raise NotImplementedError(
            "方法 `generate_order_status_reports` 必须在子类中实现",
        )  # pragma: no cover

    async def generate_fill_reports(
        self,
        command: GenerateFillReports,
    ) -> list[FillReport]:
        """
        生成带有可选查询过滤条件的 `FillReport` 列表。

        如果没有成交匹配给定参数，则返回的列表可能为空。

        参数
        ----------
        command : GenerateFillReports
            生成报告的命令。

        返回
        -------
        list[FillReport]

        """
        raise NotImplementedError(
            "方法 `generate_fill_reports` 必须在子类中实现",
        )  # pragma: no cover

    async def generate_position_status_reports(
        self,
        command: GeneratePositionStatusReports,
    ) -> list[PositionStatusReport]:
        """
        生成带有可选查询过滤条件的 `PositionStatusReport` 列表。

        如果没有持仓匹配给定参数，则返回的列表可能为空。

        参数
        ----------
        command : GeneratePositionStatusReports
            生成持仓状态报告的命令。

        返回
        -------
        list[PositionStatusReport]

        """
        raise NotImplementedError(
            "方法 `generate_position_status_reports` 必须在子类中实现",
        )  # pragma: no cover

    async def generate_mass_status(
        self,
        lookback_mins: int | None = None,
    ) -> ExecutionMassStatus | None:
        """
        生成 `ExecutionMassStatus` 报告。

        参数
        ----------
        lookback_mins : int, 可选
            查询已关闭订单、成交和持仓时的最大回溯时间（分钟）。

        返回
        -------
        ExecutionMassStatus 或 ``None``

        """
        self._log.info("正在生成 ExecutionMassStatus...")

        self.reconciliation_active = True

        mass_status = ExecutionMassStatus(
            client_id=self.id,
            account_id=self.account_id,
            venue=self.venue,
            report_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )

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

            mass_status.add_order_reports(reports=reports[0])
            mass_status.add_fill_reports(reports=reports[1])
            mass_status.add_position_reports(reports=reports[2])

            self.reconciliation_active = False

            return mass_status
        except Exception as e:
            self._log.exception("无法对账执行状态", e)
        return None

    async def _query_order(self, command: QueryOrder) -> None:
        self._log.debug(f"正在同步订单状态 {command}")

        command = GenerateOrderStatusReport(
            instrument_id=command.instrument_id,
            client_order_id=command.client_order_id,
            venue_order_id=command.venue_order_id,
            command_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
        )
        report: OrderStatusReport | None = await self.generate_order_status_report(command)

        if report is None:
            self._log.warning("未收到请求返回的 `OrderStatusReport`")
            return

        self._send_order_status_report(report)

    async def _await_account_registered(
        self,
        timeout_secs: float = 30.0,
        log_registered: bool = True,
    ) -> None:
        # 此方法通过轮询缓存，以确保账户状态事件已被处理且账户可用。
        # 这可以防止启动期间的竞争条件，此时策略或组合计算可能会在账户注册之前尝试访问账户。

        if not self.account_id:
            self._log.warning("无法等待账户注册：account_id 未设置")
            return

        # 首先检查账户是否已经注册
        if self._cache.account(self.account_id):
            if log_registered:
                self._log_account_registered()
            return

        interval_ms = 10  # 每 10ms 检查一次
        interval_secs = interval_ms / MILLISECONDS_IN_SECOND
        max_attempts = int((timeout_secs * MILLISECONDS_IN_SECOND) / interval_ms)

        for _ in range(1, max_attempts + 1):
            if self._cache.account(self.account_id):
                if log_registered:
                    self._log_account_registered()
                return
            await asyncio.sleep(interval_secs)

        raise RuntimeError(
            f"账户 {self.account_id} 在 {timeout_secs} 秒超时后仍未在缓存中注册",
        )

    def _log_account_registered(self) -> None:
        self._log.info(f"账户 {self.account_id} 已在缓存中注册", LogColor.GREEN)

    def _log_report_error(self, e: BaseException, report_type: str) -> None:
        if isinstance(e, asyncio.CancelledError) or (
            isinstance(e, ValueError) and "request canceled" in str(e).lower()
        ):
            self._log.debug(f"{report_type} request cancelled during shutdown")
        else:
            self._log.exception(f"Failed to generate {report_type}", e)

    def _log_report_receipt(
        self,
        count: int,
        report_type: str,
        log_level: LogLevel,
        verb: str = "收到",
    ) -> None:
        plural = "" if count == 1 else "s"
        receipt_log = f"{verb} {count} {report_type}{plural}"

        if log_level == LogLevel.INFO:
            self._log.info(receipt_log)
        else:
            self._log.debug(receipt_log)

    ############################################################################
    # Coroutines to implement
    ############################################################################
    async def _connect(self) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_connect` 协程",  # pragma: no cover
        )

    async def _disconnect(self) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_disconnect` 协程",  # pragma: no cover
        )

    async def _submit_order(self, command: SubmitOrder) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_submit_order` 协程",  # pragma: no cover
        )

    async def _submit_order_list(self, command: SubmitOrderList) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_submit_order_list` 协程",  # pragma: no cover
        )

    async def _modify_order(self, command: ModifyOrder) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_modify_order` 协程",  # pragma: no cover
        )

    async def _cancel_order(self, command: CancelOrder) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_cancel_order` 协程",  # pragma: no cover
        )

    async def _cancel_all_orders(self, command: CancelAllOrders) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_cancel_all_orders` 协程",  # pragma: no cover
        )

    async def _batch_cancel_orders(self, command: BatchCancelOrders) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_batch_cancel_orders` 协程",  # pragma: no cover
        )
