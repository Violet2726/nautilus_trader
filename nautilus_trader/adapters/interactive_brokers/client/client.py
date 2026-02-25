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
import functools
import os
import traceback
from collections.abc import Callable
from collections.abc import Coroutine
from inspect import iscoroutinefunction
from typing import Any

from ibapi import comm
from ibapi.client import EClient
from ibapi.commission_and_fees_report import CommissionAndFeesReport
from ibapi.common import PROTOBUF_MSG_ID
from ibapi.common import BarData
from ibapi.const import MAX_MSG_LEN
from ibapi.const import NO_VALID_ID
from ibapi.errors import BAD_LENGTH
from ibapi.execution import Execution
from ibapi.server_versions import MIN_SERVER_VER_PROTOBUF
from ibapi.utils import current_fn_name

from nautilus_trader.adapters.interactive_brokers.client.account import (
    InteractiveBrokersClientAccountMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.common import AccountOrderRef
from nautilus_trader.adapters.interactive_brokers.client.common import Request
from nautilus_trader.adapters.interactive_brokers.client.common import Requests
from nautilus_trader.adapters.interactive_brokers.client.common import Subscriptions
from nautilus_trader.adapters.interactive_brokers.client.connection import (
    InteractiveBrokersClientConnectionMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.contract import (
    InteractiveBrokersClientContractMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.error import (
    InteractiveBrokersClientErrorMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.market_data import (
    InteractiveBrokersClientMarketDataMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.order import (
    InteractiveBrokersClientOrderMixin,
)
from nautilus_trader.adapters.interactive_brokers.client.wrapper import InteractiveBrokersEWrapper
from nautilus_trader.adapters.interactive_brokers.common import IB_VENUE
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import Component
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.enums import LogColor
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import VenueOrderId


class InteractiveBrokersClient(
    Component,
    InteractiveBrokersClientConnectionMixin,
    InteractiveBrokersClientAccountMixin,
    InteractiveBrokersClientMarketDataMixin,
    InteractiveBrokersClientOrderMixin,
    InteractiveBrokersClientContractMixin,
    InteractiveBrokersClientErrorMixin,
):
    """
    与 Interactive Brokers TWS 或 Gateway 对接的客户端组件。

    此类集成了各种混入类（Mixins），提供连接管理、账户管理、行情数据和订单处理
    等功能。它继承自 `Component`，通过 `InteractiveBrokersEWrapper` 接收来自 IB 
    API 的异步回调，从而实现事件驱动的响应和自定义组件行为。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        host: str = "127.0.0.1",
        port: int = 7497,
        client_id: int = 1,
        fetch_all_open_orders: bool = False,
        request_timeout_secs: int = 60,
    ) -> None:
        super().__init__(
            clock=clock,
            component_id=ClientId(f"{IB_VENUE.value}-{client_id:03d}"),
            component_name=f"{type(self).__name__}-{client_id:03d}",
            msgbus=msgbus,
        )

        # 配置
        self._loop = loop
        self._cache = cache
        self._host = host
        self._port = port
        self._client_id = client_id
        self._fetch_all_open_orders = fetch_all_open_orders
        self._request_timeout_secs = request_timeout_secs

        # TWS API
        self._eclient: EClient = EClient(
            wrapper=InteractiveBrokersEWrapper(
                nautilus_logger=self._log,
                client=self,
            ),
        )

        # EClient 覆盖 (Overrides)
        self._eclient.sendMsg = self.sendMsg
        self._eclient.logRequest = self.logRequest

        # 任务 (Tasks)
        self._connection_watchdog_task: asyncio.Task | None = None
        self._tws_incoming_msg_reader_task: asyncio.Task | None = None
        self._internal_msg_queue_processor_task: asyncio.Task | None = None
        self._internal_msg_queue: asyncio.Queue = asyncio.Queue()
        self._msg_handler_processor_task: asyncio.Task | None = None
        self._msg_handler_task_queue: asyncio.Queue = asyncio.Queue()

        # 事件标志 (Event flags)
        self._is_client_ready: asyncio.Event = asyncio.Event()
        self._is_ib_connected: asyncio.Event = asyncio.Event()

        # 热缓存 (Hot caches)
        self.registered_nautilus_clients: set = set()
        self._event_subscriptions: dict[str, Callable] = {}

        # 订阅 (Subscriptions)
        self._requests = Requests()
        self._subscriptions = Subscriptions()

        # AccountMixin
        self._account_ids: set[str] = set()

        # ConnectionMixin
        self._connection_attempts: int = 0
        self._max_connection_attempts: int = int(os.getenv("IB_MAX_CONNECTION_ATTEMPTS", "0"))
        self._indefinite_reconnect: bool = not self._max_connection_attempts
        self._reconnect_delay: int = 5  # seconds
        self._last_disconnection_ns: int | None = None

        # MarketDataMixin
        self._bar_type_to_last_bar: dict[str, BarData | None] = {}
        self._bar_timeout_tasks: dict[
            str,
            asyncio.Task,
        ] = {}  # 跟踪每种 K 线类型的超时任务
        self._subscription_tick_data: dict[int, dict] = {}  # 按 req_id 存储逐笔行情数据
        self._subscription_start_times: dict[int, int] = {}  # 存储用于 K 线过滤的 start_ns

        # OrderMixin
        self._exec_id_details: dict[
            str,
            dict[str, Execution | (CommissionAndFeesReport | str)],
        ] = {}
        self._order_id_to_order_ref: dict[VenueOrderId, AccountOrderRef] = {}
        self._next_valid_order_id: int = -1

        # 工具提供者 (Instrument provider，在连接期间由数据/执行客户端设置)
        self._instrument_provider = None

        # 启动客户端
        self._request_id_seq: int = 10000

    def _start(self) -> None:
        """
        启动客户端。

        此方法在客户端首次初始化或重置时调用。它会设置客户端并启动连接监视器（watchdog）、
        传入消息读取器以及内部消息队列处理任务。

        """
        if not self._loop.is_running():
            self._log.warning("启动时事件循环未运行")
            self._loop.run_until_complete(self._start_async())
        else:
            self._create_task(self._start_async())

    async def _start_async(self):
        self._log.info(f"正在启动 InteractiveBrokersClient ({self._client_id})...")
        while not self._is_ib_connected.is_set():
            try:
                self._connection_attempts += 1
                if (
                    not self._indefinite_reconnect
                    and self._connection_attempts > self._max_connection_attempts
                ):
                    self._log.error("已达到最大连接尝试次数，连接失败")
                    self._stop()
                    break

                if self._connection_attempts > 1:
                    self._log.info(
                        f"第 {self._connection_attempts} 次尝试：尝试在 {self._reconnect_delay} 秒内重新连接...",
                    )
                    await asyncio.sleep(self._reconnect_delay)

                await self._connect()
                self._start_tws_incoming_msg_reader()
                self._start_internal_msg_queue_processor()
                self._eclient.startApi()
                # TWS/Gateway 在成功连接后会发送一条 managedAccounts 消息，
                # 随后将设置 `_is_ib_connected` 事件。这通常需要几秒钟，
                # 因此我们在这里等待。
                await asyncio.wait_for(self._is_ib_connected.wait(), 15)
                self._start_connection_watchdog()

                self._is_client_ready.set()
                self._log.debug("在 `_start_async` 中设置了 `_is_client_ready` 标志", LogColor.BLUE)
                self._connection_attempts = 0

            except TimeoutError:
                self._log.error("客户端初始化失败：连接超时")
            except Exception as e:
                self._log.exception("客户端启动中出现未处理的异常", e)
                self._stop()

    def _start_tws_incoming_msg_reader(self) -> None:
        """
        启动传入消息读取任务。
        """
        if self._tws_incoming_msg_reader_task:
            self._tws_incoming_msg_reader_task.cancel()

        self._tws_incoming_msg_reader_task = self._create_task(
            self._run_tws_incoming_msg_reader(),
        )

    def _start_internal_msg_queue_processor(self) -> None:
        """
        启动内部消息队列处理任务。
        """
        if self._internal_msg_queue_processor_task:
            self._internal_msg_queue_processor_task.cancel()

        self._internal_msg_queue_processor_task = self._create_task(
            self._run_internal_msg_queue_processor(),
        )

        if self._msg_handler_processor_task:
            self._msg_handler_processor_task.cancel()

        self._msg_handler_processor_task = self._create_task(
            self._run_msg_handler_processor(),
        )

    def _start_connection_watchdog(self) -> None:
        """
        启动连接监视器（watchdog）任务。
        """
        if self._connection_watchdog_task:
            self._connection_watchdog_task.cancel()

        self._connection_watchdog_task = self._create_task(
            self._run_connection_watchdog(),
        )

    def _stop(self) -> None:
        """
        停止客户端并取消正在运行的任务。
        """
        self._create_task(self._stop_async())

    async def _stop_async(self) -> None:
        self._log.info(f"正在停止 InteractiveBrokersClient ({self._client_id})...")

        if self._is_client_ready.is_set():
            self._is_client_ready.clear()
            self._log.debug("在 `_stop_async` 中取消了 `_is_client_ready` 状态", LogColor.BLUE)

        # 取消任务
        tasks = [
            self._connection_watchdog_task,
            self._tws_incoming_msg_reader_task,
            self._internal_msg_queue_processor_task,
            self._msg_handler_processor_task,
        ]
        for task in tasks:
            if task and not task.cancelled():
                task.cancel()

        try:
            tasks = [t for t in tasks if t is not None]
            await asyncio.gather(*tasks, return_exceptions=True)
            self._log.info("所有任务已成功取消。")
        except Exception as e:
            self._log.exception(f"取消任务时发生错误：{e}", e)

        self._eclient.disconnect()
        self._account_ids = set()
        self.registered_nautilus_clients = set()

    def _reset(self) -> None:
        """
        重启客户端。
        """

        async def _reset_async():
            self._log.info(f"正在充值 InteractiveBrokersClient ({self._client_id})...")
            await self._stop_async()
            await self._start_async()

        self._create_task(_reset_async())

    def _resume(self) -> None:
        """
        恢复客户端并重新订阅所有内容。
        """

        async def _resume_async():
            await self._is_client_ready.wait()
            self._log.info(f"正在恢复 InteractiveBrokersClient ({self._client_id})...")
            await self._resubscribe_all()

        self._create_task(_resume_async())

    def _degrade(self) -> None:
        """
        当连接丢失时降级客户端。
        """
        if not self.is_degraded:
            self._log.info(f"正在降级 InteractiveBrokersClient ({self._client_id})...")
            self._is_client_ready.clear()
            self._account_ids = set()

    async def _resubscribe_all(self) -> None:
        """
        取消并重新启动所有订阅。
        """
        subscriptions = self._subscriptions.get_all()
        subscription_names = ", ".join([str(subscription.name) for subscription in subscriptions])
        self._log.info(f"正在重新订阅 {len(subscriptions)} 个订阅：{subscription_names}")

        for subscription in self._subscriptions.get_all():
            self._log.info(f"正在重新订阅 {subscription.name} 订阅...")

            try:
                if iscoroutinefunction(subscription.handle):
                    await subscription.handle()
                else:
                    await asyncio.to_thread(subscription.handle)
            except Exception as e:
                self._log.exception(f"重新订阅 {subscription} 失败", e)

    async def wait_until_ready(self, timeout: int = 300) -> None:
        """
        在给定的超时时间内检查客户端是否正在运行且已就绪。

        参数
        ----------
        timeout : int, 默认 300
            等待客户端就绪的时间（秒）。

        """
        try:
            if not self._is_client_ready.is_set():
                await asyncio.wait_for(self._is_client_ready.wait(), timeout)
        except TimeoutError as e:
            self._log.error(f"客户端未就绪：{e}")

    async def _run_connection_watchdog(self) -> None:
        """
        运行监视器以监控和管理套接字连接的健康状况。

        持续检查连接状态，根据连接健康状况管理客户端状态，并在网络故障或
        强制 IB 连接重置时处理订阅管理。

        """
        try:
            while True:
                await asyncio.sleep(1)

                if not self._is_ib_connected.is_set() or not self._eclient.isConnected():
                    self._log.error("连接监视器检测到连接丢失")
                    await self._handle_disconnection()
        except asyncio.CancelledError:
            self._log.debug("客户端连接监视器任务已取消。")

    async def _handle_disconnection(self) -> None:
        """
        处理客户端从 TWS/Gateway 断开连接的情况。
        """
        if self.is_running:
            self._degrade()

        if self._is_ib_connected.is_set():
            self._log.debug("在 `_handle_disconnection` 中取消了 `_is_ib_connected` 状态", LogColor.BLUE)
            self._is_ib_connected.clear()

        self._last_disconnection_ns = self._clock.timestamp_ns()
        await asyncio.sleep(5)
        await self._handle_reconnect()

    def _create_task(
        self,
        coro: Coroutine,
        log_msg: str | None = None,
        actions: Callable | None = None,
        success: str | None = None,
    ) -> asyncio.Task:
        """
        创建一个具有错误处理和可选回调操作的 asyncio 任务。

        参数
        ----------
        coro : Coroutine
            要运行的协程。
        log_msg : str, 可选
            任务的日志消息。
        actions : Callable, 可选
            协程完成时要运行的操作回调。
        success : str, 可选
            操作成功时要写入的日志消息。

        返回
        -------
        asyncio.Task

        """
        log_msg = log_msg or coro.__name__
        self._log.debug(f"正在创建任务 '{log_msg}'")
        task = self._loop.create_task(
            coro,
            name=coro.__name__,
        )
        task.add_done_callback(
            functools.partial(
                self._on_task_completed,
                actions,
                success,
            ),
        )
        return task

    def _on_task_completed(
        self,
        actions: Callable | None,
        success: str | None,
        task: asyncio.Task,
    ) -> None:
        """
        处理任务的完成。

        参数
        ----------
        actions : Callable, 可选
            任务完成时执行的回调操作。
        success : str, 可选
            操作成功完成时显示的成功日志消息。
        task : asyncio.Task
            已完成的 asyncio 任务。

        """
        if task.exception():
            # exc = task.exception()
            # exc_type = type(exc)
            # exc_traceback = exc.__traceback__
            # stack_trace = traceback.format_exception(exc_type, exc, exc_traceback)
            # stack_trace_str = "".join(stack_trace)
            # self._log.error(
            #     f"Error on '{task.get_name()}': {exc!r}\n{stack_trace_str}",
            # )
            self._log.error(
                f"任务 '{task.get_name()}' 出错：{task.exception()!r}",
            )
        else:
            if actions:
                try:
                    actions()
                except Exception as e:
                    self._log.exception(
                        f"在 '{task.get_name()}' 上触发操作 {actions.__name__} 失败",
                        e,
                    )
            if success:
                self._log.info(success, LogColor.GREEN)

    def subscribe_event(self, name: str, handler: Callable) -> None:
        """
        将处理函数订阅到具名事件。

        参数
        ----------
        name : str
            要订阅的事件名称。
        handler : Callable
            事件发生时要调用的处理函数。

        """
        self._event_subscriptions[name] = handler

    def unsubscribe_event(self, name: str) -> None:
        """
        取消处理函数对具名事件的订阅。

        参数
        ----------
        name : str
            要取消订阅的事件名称。

        """
        self._event_subscriptions.pop(name)

    async def _await_request(
        self,
        request: Request,
        timeout: int,
        default_value: Any | None = None,
        suppress_timeout_warning: bool = False,
    ) -> Any:
        """
        在指定超时时间内等待请求完成。

        参数
        ----------
        request : Request
            要等待的请求对象。
        timeout : int
            等待请求完成的最大时间（秒）。
        default_value : Any, 可选
            如果请求超时或失败，则返回默认值。默认为 None。
        suppress_timeout_warning: bool, 可选
            是否抑制超时警告。默认为 False。

        返回
        -------
        Any
            请求的结果，如果请求超时或失败，则返回 default_value。

        """
        try:
            return await asyncio.wait_for(request.future, timeout)
        except TimeoutError as e:
            msg = f"请求 {request} 超时。正在结束请求。"
            self._log.debug(msg) if suppress_timeout_warning else self._log.warning(msg)
            self._end_request(request.req_id, success=False, exception=e)

            return default_value
        except ConnectionError as e:
            self._log.error(f"{request} 期间发生连接错误；正在结束请求")
            self._end_request(request.req_id, success=False, exception=e)

            return default_value

    def _end_request(
        self,
        req_id: int,
        success: bool = True,
        exception: type | BaseException | None = None,
    ) -> None:
        """
        以指定结果或异常结束请求。

        参数
        ----------
        req_id : int
            要结束的请求 ID。
        success : bool, 可选
            请求是否成功。默认为 True。
        exception : type | BaseException | None, 可选
            请求失败时要设置的异常。默认为 None。

        """
        if not (request := self._requests.get(req_id=req_id)):
            return

        if not request.future.done():
            if success:
                request.future.set_result(request.result)
            else:
                request.cancel()
                if exception:
                    request.future.set_exception(exception)

        self._requests.remove(req_id=req_id)

    async def _run_tws_incoming_msg_reader(self) -> None:
        """
        持续从 TWS/Gateway 读取消息，然后将其放入内部消息队列进行处理。
        """
        self._log.debug("客户端 TWS 传入消息读取器已启动")
        buf = b""

        try:
            while self._eclient.conn and self._eclient.conn.isConnected():
                data = await asyncio.to_thread(self._eclient.conn.recvMsg)
                buf += data

                while buf:
                    _, msg, buf = comm.read_msg(buf)
                    self._log.debug(f"收到消息缓冲区：{buf!s}")

                    if msg:
                        # 将消息放入内部队列进行处理
                        self._loop.call_soon_threadsafe(self._internal_msg_queue.put_nowait, msg)
                    else:
                        self._log.debug("需要更多传入的数据包")
                        break
        except asyncio.CancelledError:
            self._log.debug("客户端 TWS 传入消息读取器已取消")
        except Exception as e:
            self._log.exception("客户端 TWS 传入消息读取器中出现未处理的异常", e)
        finally:
            if self._is_ib_connected.is_set() and not self.is_disposed:
                self._log.debug(
                    "`_is_ib_connected` 已被 `_run_tws_incoming_msg_reader` 取消设置",
                    LogColor.BLUE,
                )
                self._is_ib_connected.clear()

            self._log.debug("客户端 TWS 传入消息读取器已停止")

    async def _run_internal_msg_queue_processor(self) -> None:
        """
        持续从内部传入消息队列中处理消息。
        """
        self._log.debug("客户端内部消息队列处理程序已启动")

        try:
            while (
                self._eclient.conn and self._eclient.conn.isConnected()
            ) or not self._internal_msg_queue.empty():
                msg = await self._internal_msg_queue.get()

                if not await self._process_message(msg):
                    break

                self._internal_msg_queue.task_done()
        except asyncio.CancelledError:
            log_msg = f"内部消息队列处理已取消。(队列大小={self._internal_msg_queue.qsize()})。"
            (
                self._log.warning(log_msg)
                if not self._internal_msg_queue.empty()
                else self._log.debug(
                    log_msg,
                )
            )
        finally:
            self._log.debug("内部消息队列处理程序已停止")

    async def _process_message(self, msg: bytes) -> bool:
        """
        处理来自 TWS/Gateway 的单个消息。

        参数
        ----------
        msg : bytes
            要处理的消息。

        返回
        -------
        bool

        """
        if len(msg) > MAX_MSG_LEN:
            await self.process_error(
                req_id=NO_VALID_ID,
                error_time=0,
                error_code=BAD_LENGTH.code(),
                error_string=f"{BAD_LENGTH.msg()}:{len(msg)}:{msg!r}",
            )

            return False

        if self._eclient.serverVersion() >= MIN_SERVER_VER_PROTOBUF:
            sMsgId = msg[:4]
            msgId = int.from_bytes(sMsgId, "big")
            msg = msg[4:]
        else:
            sMsgId = msg[: msg.index(b"\0")]
            msg = msg[msg.index(b"\0") + len(b"\0") :]
            msgId = int(sMsgId)

        if msgId > PROTOBUF_MSG_ID:
            msgId -= PROTOBUF_MSG_ID
            self._log.debug(f"收到消息 (Protobuf)：msgId={msgId}")
            # 使用 Protobuf 解码器识别消息类型并调用相应的 EWrapper 方法。
            # 在较新的 TWS API 版本中，Protobuf 编码被用于更高效的通信。
            await asyncio.to_thread(self._eclient.decoder.processProtoBuf, msg, msgId)
        else:
            fields: tuple[bytes] = comm.read_fields(msg)
            self._log.debug(f"收到消息：msgId={msgId} 字段={fields}")
            # 使用标准解码器根据 msgId 识别消息类型，并调用 EWrapper
            # 中的相应方法。这些方法中有许多在客户端管理器和处理程序类中
            # 被覆盖，以支持 Nautilus 所需的自定义处理。
            await asyncio.to_thread(self._eclient.decoder.interpret, fields, msgId)

        return True

    async def _run_msg_handler_processor(self):
        """
        异步处理来自消息处理任务队列的任务。

        持续从 `msg_handler_task_queue` 中获取并执行任务，这些任务通常是表示从
        ibapi wrapper 接收到的消息处理操作的偏函数（partial functions）。
        该方法会等待每个任务执行完毕，从而确保其被运行。任务执行后，它会在队列
        中将该任务标记为已完成。

        此方法被设计为无限循环运行，直到被外部取消，通常作为应用程序关闭或处理
        上下文需要停止操作的一部分。

        """
        try:
            while True:
                handler_task = await self._msg_handler_task_queue.get()
                try:
                    await handler_task()
                except Exception as e:
                    exc_type = type(e)
                    exc_traceback = e.__traceback__
                    stack_trace = traceback.format_exception(exc_type, e, exc_traceback)
                    stack_trace_str = "".join(stack_trace)
                    task_name = getattr(handler_task, "__name__", str(handler_task))
                    self._log.error(
                        f"Exception in message handler task '{task_name}': {e!r}\n{stack_trace_str}",
                    )
                self._msg_handler_task_queue.task_done()
        except asyncio.CancelledError:
            log_msg = f"处理程序任务处理已取消。(队列大小={self._msg_handler_task_queue.qsize()})。"
            (
                self._log.warning(log_msg)
                if not self._internal_msg_queue.empty()
                else self._log.debug(
                    log_msg,
                )
            )
        finally:
            self._log.debug("处理程序任务处理程序已停止")

    def submit_to_msg_handler_queue(self, task: Callable[..., Any]) -> None:
        """
        将任务提交到消息处理程序的队列进行处理。

        此方法将一个可调用任务放入消息处理任务队列，确保其根据队列顺序进行异步
        执行。操作是非阻塞的，在入队任务后立即返回。

        参数
        ----------
        task : Callable[..., Any]
            要入队的任务。此任务应为一个可调用对象，且符合消息处理程序处理任务
            的预期签名。

        """
        self._log.debug(f"正在向消息处理程序队列提交任务：{task}")
        asyncio.run_coroutine_threadsafe(self._msg_handler_task_queue.put(task), self._loop)

    def _next_req_id(self) -> int:
        """
        生成下一个顺序请求 ID。

        返回
        -------
        int

        """
        new_id = self._request_id_seq
        self._request_id_seq += 1

        return new_id

    # -- EClient overrides ------------------------------------------------------------------------

    def sendMsg(self, msgId, msg):
        """
        覆盖 ibapi EClient.sendMsg 的日志记录。
        """
        useRawIntMsgId = self._eclient.serverVersion() >= MIN_SERVER_VER_PROTOBUF
        full_msg = comm.make_msg(msgId, useRawIntMsgId, msg)
        self._log.debug(f"TWS API 请求已发送：函数={current_fn_name(1)} 消息={full_msg}")
        self._eclient.conn.sendMsg(full_msg)

    def logRequest(self, fnName, fnParams):
        """
        覆盖 ibapi EClient.logRequest 的日志记录。
        """
        if "self" in fnParams:
            prms = dict(fnParams)
            del prms["self"]
        else:
            prms = fnParams

        self._log.debug(f"TWS API 已准备好的请求：函数={fnName} 数据={prms}")
