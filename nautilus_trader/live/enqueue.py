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
from weakref import WeakSet

from nautilus_trader.common.component import Clock
from nautilus_trader.common.component import Logger
from nautilus_trader.core.nautilus_pyo3 import NANOSECONDS_IN_SECOND


class ThrottledEnqueuer[T]:
    """
    管理将类型为 T 的消息排入内部异步队列的操作。

    参数
    ----------
    qname : str
        内部队列的名称（例如 "data_queue"）。
    queue : asyncio.Queue
        要管理的内部 asyncio 队列。
    loop : asyncio.AbstractEventLoop
        用于调度队列操作的事件循环。
    clock : Clock
        用于限流日志消息的时钟。
    logger : Logger
        用于记录容量警告日志的记录器。

    """

    def __init__(
        self,
        qname: str,
        queue: asyncio.Queue,
        loop: asyncio.AbstractEventLoop,
        clock: Clock,
        logger: Logger,
    ) -> None:
        self._qname = qname
        self._queue = queue
        self._loop = loop
        self._clock = clock
        self._log = logger
        self._ts_last_logged: int = 0
        self._pending_tasks: WeakSet[asyncio.Task] = WeakSet()

    @property
    def qname(self) -> str:
        """
        返回内部队列的名称。

        返回
        -------
        str

        """
        return self._qname

    @property
    def size(self) -> int:
        """
        返回当前内部队列的大小。

        返回
        -------
        int

        """
        return self._queue.qsize()

    @property
    def capacity(self) -> int:
        """
        返回内部队列的最大容量。

        返回
        -------
        int

        """
        return self._queue.maxsize

    def enqueue(self, msg: T) -> None:
        """
        将消息排入队列，并在队列达到容量上限时记录限流警告。

        此方法确保消息始终被排入队列，即使队列暂时已满（它会调度一个异步 put 操作）。

        参数
        ----------
        msg : T
            要排入队列的消息。

        """
        # 不允许 None 通过（None 是停止队列的哨兵值）
        assert msg is not None, "预期应有值，但消息为 `None`"

        if self._queue.qsize() < self._queue.maxsize:
            self._loop.call_soon_threadsafe(self._enqueue_nowait_safely, self._queue, msg)
            return

        task = self._loop.create_task(self._queue.put(msg))
        task.add_done_callback(self._handle_task_exception)
        self._pending_tasks.add(task)

        # 限流日志记录，每秒最多一次
        now_ns = self._clock.timestamp_ns()
        if now_ns > self._ts_last_logged + NANOSECONDS_IN_SECOND:
            self._log.warning(
                f"{self._qname} 已达容量上限 ({self._queue.qsize():_}/{self._queue.maxsize})，"
                "已调度异步 put() 操作到队列",
            )
            self._ts_last_logged = now_ns

    def cancel_pending_tasks(self) -> None:
        """
        取消所有挂起的异步 put 任务。

        应在停机期间调用此方法，以防止出现 "Task was destroyed but it is pending!" 警告。

        """
        for task in list(self._pending_tasks):
            if not task.done():
                task.cancel()

    def _handle_task_exception(self, task: asyncio.Task) -> None:
        if task.cancelled():
            return

        exc = task.exception()
        if exc is not None:
            self._log.error(f"将消息放入 {self._qname} 时出错: {exc!r}")

    def _enqueue_nowait_safely(self, queue: asyncio.Queue, msg: T) -> None:
        # 尝试执行 put_nowait(msg)，如果队列已满，
        # 则调度异步 put() 作为后备方案。
        try:
            queue.put_nowait(msg)
        except asyncio.QueueFull:
            task = asyncio.create_task(queue.put(msg))
            task.add_done_callback(self._handle_task_exception)
            self._pending_tasks.add(task)
