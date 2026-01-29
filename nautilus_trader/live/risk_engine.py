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

from nautilus_trader.cache.base import CacheFacade
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.enums import LogColor
from nautilus_trader.config import LiveRiskEngineConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.message import Command
from nautilus_trader.core.message import Event
from nautilus_trader.live.enqueue import ThrottledEnqueuer
from nautilus_trader.portfolio.base import PortfolioFacade
from nautilus_trader.risk.engine import RiskEngine


class LiveRiskEngine(RiskEngine):
    """
    提供高性能的异步实时风险引擎。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        引擎的事件循环。
    portfolio : PortfolioFacade
        引擎的投资组合。
    msgbus : MessageBus
        引擎的消息总线。
    cache : CacheFacade
        引擎的只读缓存。
    clock : LiveClock
        引擎的时钟。
    config : LiveRiskEngineConfig
        实例的配置。

    引发
    ------
    TypeError
        如果 `config` 不是 `LiveRiskEngineConfig` 类型。

    """

    _sentinel: Final[None] = None

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        portfolio: PortfolioFacade,
        msgbus: MessageBus,
        cache: CacheFacade,
        clock: LiveClock,
        config: LiveRiskEngineConfig | None = None,
    ) -> None:
        if config is None:
            config = LiveRiskEngineConfig()
        PyCondition.type(config, LiveRiskEngineConfig, "config")
        super().__init__(
            portfolio=portfolio,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop: asyncio.AbstractEventLoop = loop
        self._cmd_queue: asyncio.Queue = Queue(maxsize=config.qsize)
        self._evt_queue: asyncio.Queue = Queue(maxsize=config.qsize)

        self._cmd_enqueuer: ThrottledEnqueuer[Command] = ThrottledEnqueuer(
            qname="cmd_queue",
            queue=self._cmd_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )
        self._evt_enqueuer: ThrottledEnqueuer[Event] = ThrottledEnqueuer(
            qname="evt_queue",
            queue=self._evt_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )

        # 异步任务
        self._cmd_queue_task: asyncio.Task | None = None
        self._evt_queue_task: asyncio.Task | None = None
        self._kill: bool = False

        # 配置
        self.graceful_shutdown_on_exception: bool = config.graceful_shutdown_on_exception
        self._shutdown_initiated: bool = False
        self._log.info(f"{config.graceful_shutdown_on_exception=}", LogColor.BLUE)

    def get_cmd_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎的内部命令队列任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._cmd_queue_task

    def get_evt_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎的内部事件队列任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._evt_queue_task

    def cmd_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `Command` 消息数量。

        返回
        -------
        int

        """
        return self._cmd_queue.qsize()

    def evt_qsize(self) -> int:
        """
        返回内部队列中缓冲的 `Event` 消息数量。

        返回
        -------
        int

        """
        return self._evt_queue.qsize()

    # -- 命令 -----------------------------------------------------------------------------------------

    def kill(self) -> None:
        """
        通过强制取消队列任务并调用停止来终止引擎。
        """
        self._log.warning("正在终止引擎 (Killing engine)")
        self._kill = True
        self.stop()
        if self._cmd_queue_task:
            self._log.debug(f"正在取消任务 '{self._cmd_queue_task.get_name()}'")
            self._cmd_queue_task.cancel()
            self._cmd_queue_task = None
        if self._evt_queue_task:
            self._log.debug(f"正在取消任务 '{self._evt_queue_task.get_name()}'")
            self._evt_queue_task.cancel()
            self._evt_queue_task = None

    def execute(self, command: Command) -> None:
        """
        执行给定的命令。

        如果内部队列已满或接近容量上限，将记录一条警告（限流），
        如果不满则调度异步的 `put()` 操作。这确保所有消息最终都能入队处理，
        而在队列已满时不会阻塞调用者。

        参数
        ----------
        command : Command
            要执行的命令。

        警告
        --------
        此方法不是线程安全的，应仅从运行事件循环的同一线程调用。
        从不同线程调用可能会导致意外行为。

        """
        self._cmd_enqueuer.enqueue(command)

    def process(self, event: Event) -> None:
        """
        处理给定的事件。

        如果内部队列已满或接近容量上限，将记录一条警告（限流），
        如果不满则调度异步的 `put()` 操作。这确保所有消息最终都能入队处理，
        而在队列已满时不会阻塞调用者。

        参数
        ----------
        event : Event
            要处理的事件。

        """
        self._evt_enqueuer.enqueue(event)

    # -- 内部 -----------------------------------------------------------------------------------------

    def _handle_queue_exception(self, e: Exception, queue_name: str) -> None:
        self._log.exception(
            f"{queue_name} 队列处理中发生意外异常: {e!r}",
            e,
        )
        if self.graceful_shutdown_on_exception:
            if not self._shutdown_initiated:
                self._log.warning(
                    "因意外异常正在启动优雅停机",
                )
                self.shutdown_system(
                    f"{queue_name} 队列处理中发生意外异常: {e!r}",
                )
                self._shutdown_initiated = True
        else:
            self._log.error(
                "系统将立即终止以防止在降级状态下运行",
            )
            os._exit(1)  # 立即崩溃

    def _enqueue_sentinel(self) -> None:
        self._loop.call_soon_threadsafe(self._cmd_queue.put_nowait, self._sentinel)
        self._loop.call_soon_threadsafe(self._evt_queue.put_nowait, self._sentinel)
        self._log.debug("哨兵消息已放入队列")

    def _on_start(self) -> None:
        if not self._loop.is_running():
            self._log.warning("在循环未运行时启动")

        self._cmd_queue_task = self._loop.create_task(self._run_cmd_queue(), name="cmd_queue")
        self._evt_queue_task = self._loop.create_task(self._run_evt_queue(), name="evt_queue")

        self._log.debug(f"已调度任务 '{self._cmd_queue_task.get_name()}'")
        self._log.debug(f"已调度任务 '{self._evt_queue_task.get_name()}'")

    def _on_stop(self) -> None:
        if self._kill:
            return  # 避免排队冗余的哨兵消息
        # 这将在看到哨兵消息时立即停止队列处理
        self._enqueue_sentinel()

    async def _run_cmd_queue(self) -> None:
        self._log.debug(
            f"命令消息队列处理中 (qsize={self.cmd_qsize()})",
        )
        try:
            while True:
                try:
                    command: Command | None = await self._cmd_queue.get()
                    if command is self._sentinel:
                        break

                    self._execute_command(command)
                except asyncio.CancelledError:
                    self._log.warning("已取消任务 'run_cmd_queue'")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "command")
        finally:
            stopped_msg = "命令消息队列已停止"
            if not self._cmd_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.cmd_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    async def _run_evt_queue(self) -> None:
        self._log.debug(
            f"事件消息队列处理开始 (qsize={self.evt_qsize()})",
        )
        try:
            while True:
                try:
                    event: Event | None = await self._evt_queue.get()
                    if event is self._sentinel:
                        break

                    self._handle_event(event)
                except asyncio.CancelledError:
                    self._log.warning("已取消任务 'run_evt_queue'")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "event")
        finally:
            stopped_msg = "事件消息队列已停止"
            if not self._evt_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.evt_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)
