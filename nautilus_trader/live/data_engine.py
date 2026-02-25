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
import os
from asyncio import Queue
from typing import Final

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import LiveDataEngineConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.data import Data
from nautilus_trader.data.engine import DataEngine
from nautilus_trader.data.messages import DataCommand
from nautilus_trader.data.messages import DataResponse
from nautilus_trader.data.messages import RequestData
from nautilus_trader.live.enqueue import ThrottledEnqueuer


class LiveDataEngine(DataEngine):
    """
    提供高性能的异步实盘数据引擎。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        引擎的事件循环。
    msgbus : MessageBus
        引擎的消息总线。
    cache : Cache
        引擎的缓存。
    clock : LiveClock
        引擎的时钟。
    config : LiveDataEngineConfig, 可选
        实例的配置。

    异常
    ------
    TypeError
        如果 `config` 的类型不是 `LiveDataEngineConfig`。

    """

    _sentinel: Final[None] = None

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        config: LiveDataEngineConfig | None = None,
    ) -> None:
        if config is None:
            config = LiveDataEngineConfig()
        PyCondition.type(config, LiveDataEngineConfig, "config")
        super().__init__(
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop: asyncio.AbstractEventLoop = loop
        self._cmd_queue: asyncio.Queue = Queue(maxsize=config.qsize)
        self._req_queue: asyncio.Queue = Queue(maxsize=config.qsize)
        self._res_queue: asyncio.Queue = Queue(maxsize=config.qsize)
        self._data_queue: asyncio.Queue = Queue(maxsize=config.qsize)

        self._cmd_enqueuer: ThrottledEnqueuer[DataCommand] = ThrottledEnqueuer(
            qname="cmd_queue",
            queue=self._cmd_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )
        self._req_enqueuer: ThrottledEnqueuer[RequestData] = ThrottledEnqueuer(
            qname="req_queue",
            queue=self._req_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )
        self._res_enqueuer: ThrottledEnqueuer[DataResponse] = ThrottledEnqueuer(
            qname="res_queue",
            queue=self._res_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )
        self._data_enqueuer: ThrottledEnqueuer[Data] = ThrottledEnqueuer(
            qname="data_queue",
            queue=self._data_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )

        # 异步任务
        self._cmd_queue_task: asyncio.Task | None = None
        self._req_queue_task: asyncio.Task | None = None
        self._res_queue_task: asyncio.Task | None = None
        self._data_queue_task: asyncio.Task | None = None
        self._kill: bool = False

        # 配置
        self.graceful_shutdown_on_exception: bool = config.graceful_shutdown_on_exception
        self._shutdown_initiated: bool = False
        self._log.info(f"{config.graceful_shutdown_on_exception=}", LogColor.BLUE)

    def connect(self) -> None:
        """
        通过调用所有注册客户端的 connect 方法来连接引擎。
        """
        if self._clients:
            self._log.info("正在连接所有客户端...")
        elif self._external_clients:
            self._log.info(
                f"已配置外部客户端：{self._external_clients}",
                LogColor.BLUE,
            )
        else:
            self._log.warning("没有可连接的客户端")
            return

        for client in self._clients.values():
            client.connect()

    def disconnect(self) -> None:
        """
        通过调用所有注册客户端的 disconnect 方法来断开引擎连接。
        """
        if self._clients:
            self._log.info("正在断开所有客户端的连接...")
        else:
            self._log.warning("没有可断开连接的客户端")
            return

        for client in self._clients.values():
            client.disconnect()

    def get_cmd_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎内部命令队列的任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._cmd_queue_task

    def get_req_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎内部请求队列的任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._req_queue_task

    def get_res_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎内部响应队列的任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._res_queue_task

    def get_data_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎内部数据队列的任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._data_queue_task

    def cmd_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `DataCommand` 对象数量。

        返回
        -------
        int

        """
        return self._cmd_queue.qsize()

    def req_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `RequestData` 对象数量。

        返回
        -------
        int

        """
        return self._req_queue.qsize()

    def res_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `DataResponse` 对象数量。

        返回
        -------
        int

        """
        return self._res_queue.qsize()

    def data_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `Data` 对象数量。

        返回
        -------
        int

        """
        return self._data_queue.qsize()

    def kill(self) -> None:
        """
        通过强行取消队列任务并调用 stop 来停止引擎。
        """
        self._log.warning("正在停止引擎（Kill）")
        self._kill = True
        self.stop()

        # 取消挂起的入队任务
        self._cmd_enqueuer.cancel_pending_tasks()
        self._req_enqueuer.cancel_pending_tasks()
        self._res_enqueuer.cancel_pending_tasks()
        self._data_enqueuer.cancel_pending_tasks()

        if self._cmd_queue_task:
            self._log.debug(f"正在取消任务 '{self._cmd_queue_task.get_name()}'")
            self._cmd_queue_task.cancel()
            self._cmd_queue_task = None
        if self._req_queue_task:
            self._log.debug(f"正在取消任务 '{self._req_queue_task.get_name()}'")
            self._req_queue_task.cancel()
            self._req_queue_task = None
        if self._res_queue_task:
            self._log.debug(f"正在取消任务 '{self._res_queue_task.get_name()}'")
            self._res_queue_task.cancel()
            self._res_queue_task = None
        if self._data_queue_task:
            self._log.debug(f"正在取消任务 '{self._data_queue_task.get_name()}'")
            self._data_queue_task.cancel()
            self._data_queue_task = None

    def execute(self, command: DataCommand) -> None:
        """
        执行给定的数据命令。

        如果内部队列容量已满或接近满，它将记录一条警告（限流）并调度异步 `put()` 操作。
        这确保了所有消息最终都会排入队列并得到处理，而不会在队列已满时阻塞调用者。

        参数
        ----------
        command : DataCommand
            要执行的命令。

        """
        self._cmd_enqueuer.enqueue(command)

    def request(self, request: RequestData) -> None:
        """
        处理给定的请求。

        如果内部队列容量已满或接近满，它将记录一条警告（限流）并调度异步 `put()` 操作。
        这确保了所有消息最终都会排入队列并得到处理，而不会在队列已满时阻塞调用者。

        参数
        ----------
        request : RequestData
            要处理的请求。

        """
        self._req_enqueuer.enqueue(request)

    def response(self, response: DataResponse) -> None:
        """
        处理给定的响应。

        如果内部队列容量已满或接近满，它将记录一条警告（限流）并调度异步 `put()` 操作。
        这确保了所有消息最终都会排入队列并得到处理，而不会在队列已满时阻塞调用者。

        参数
        ----------
        response : DataResponse
            要处理的响应。

        """
        self._res_enqueuer.enqueue(response)

    def process(self, data: Data) -> None:
        """
        处理给定的数据消息。

        如果内部队列容量已满或接近满，它将记录一条警告（限流）并调度异步 `put()` 操作。
        这确保了所有消息最终都会排入队列并得到处理，而不会在队列已满时阻塞调用者。

        参数
        ----------
        data : Data
            要处理的数据。

        警告
        --------
        此方法不是线程安全的，只能从运行事件循环的同一个线程调用。从不同线程调用可能会导致意外行为。

        """
        self._data_enqueuer.enqueue(data)

    # -- 内部 -----------------------------------------------------------------------------------------

    def _handle_queue_exception(self, e: Exception, queue_name: str) -> None:
        self._log.exception(
            f"{queue_name} 队列处理中出现意外异常: {e!r}",
            e,
        )
        if self.graceful_shutdown_on_exception:
            if not self._shutdown_initiated:
                self._log.warning(
                    "由于意外异常，正在启动优雅停机",
                )
                self.shutdown_system(
                    f"{queue_name} 队列处理中出现意外异常: {e!r}",
                )
                self._shutdown_initiated = True
        else:
            self._log.error(
                "系统将立即终止，以防止在降级状态下运行",
            )
            os._exit(1)  # 立即崩溃

    def _enqueue_sentinels(self) -> None:
        self._loop.call_soon_threadsafe(self._cmd_queue.put_nowait, self._sentinel)
        self._loop.call_soon_threadsafe(self._req_queue.put_nowait, self._sentinel)
        self._loop.call_soon_threadsafe(self._res_queue.put_nowait, self._sentinel)
        self._loop.call_soon_threadsafe(self._data_queue.put_nowait, self._sentinel)
        self._log.debug("哨兵消息已放入队列")

    def _on_start(self) -> None:
        if not self._loop.is_running():
            self._log.warning("启动时循环（loop）未运行")

        self._cmd_queue_task = self._loop.create_task(self._run_cmd_queue(), name="cmd_queue")
        self._req_queue_task = self._loop.create_task(self._run_req_queue(), name="req_queue")
        self._res_queue_task = self._loop.create_task(self._run_res_queue(), name="res_queue")
        self._data_queue_task = self._loop.create_task(self._run_data_queue(), name="data_queue")

        self._log.debug(f"已调度任务 '{self._cmd_queue_task.get_name()}'")
        self._log.debug(f"已调度任务 '{self._req_queue_task.get_name()}'")
        self._log.debug(f"已调度任务 '{self._res_queue_task.get_name()}'")
        self._log.debug(f"已调度任务 '{self._data_queue_task.get_name()}'")

    def _on_stop(self) -> None:
        if self._kill:
            return  # 避免排队冗余的哨兵消息

        # 取消挂起的入队任务
        self._cmd_enqueuer.cancel_pending_tasks()
        self._req_enqueuer.cancel_pending_tasks()
        self._res_enqueuer.cancel_pending_tasks()
        self._data_enqueuer.cancel_pending_tasks()

        # 这将在看到哨兵消息时立即停止队列处理
        self._enqueue_sentinels()

    async def _run_cmd_queue(self) -> None:
        self._log.debug(
            f"DataCommand 消息队列处理开始 (qsize={self.cmd_qsize()})",
        )
        try:
            while True:
                try:
                    command: DataCommand | None = await self._cmd_queue.get()
                    if command is self._sentinel:
                        break

                    self._execute_command(command)
                except asyncio.CancelledError:
                    self._log.warning("DataCommand 消息队列已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "DataCommand")
        finally:
            stopped_msg = "DataCommand 消息队列已停止"
            if not self._cmd_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.cmd_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    async def _run_req_queue(self) -> None:
        self._log.debug(
            f"RequestData 消息队列处理开始 (qsize={self.req_qsize()})",
        )
        try:
            while True:
                try:
                    request: RequestData | None = await self._req_queue.get()
                    if request is self._sentinel:
                        break

                    self._handle_request(request)
                except asyncio.CancelledError:
                    self._log.warning("RequestData 消息队列已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "RequestData")
        finally:
            stopped_msg = "RequestData 消息队列已停止"
            if not self._req_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.req_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    async def _run_res_queue(self) -> None:
        self._log.debug(
            f"DataResponse 消息队列处理开始 (qsize={self.res_qsize()})",
        )
        try:
            while True:
                try:
                    response: DataResponse | None = await self._res_queue.get()
                    if response is self._sentinel:
                        break

                    self._handle_response(response)
                except asyncio.CancelledError:
                    self._log.warning("DataResponse 消息队列已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "DataResponse")
        finally:
            stopped_msg = "DataResponse 消息队列已停止"
            if not self._res_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.res_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    async def _run_data_queue(self) -> None:
        self._log.debug(f"Data 消息队列处理开始 (qsize={self.data_qsize()})")
        try:
            while True:
                try:
                    data: Data | None = await self._data_queue.get()
                    if data is self._sentinel:
                        break

                    self._handle_data(data)
                except asyncio.CancelledError:
                    self._log.warning("Data 消息队列已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "Data")
        finally:
            stopped_msg = "Data 消息队列已停止"
            if not self._data_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.data_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)
