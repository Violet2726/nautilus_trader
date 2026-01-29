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
为实盘组件提供任务取消工具。
"""

import asyncio
from weakref import WeakSet

from nautilus_trader.common.component import Logger


# 取消 Future 的默认超时时间（比任务短，因为它们代表外部连接）
DEFAULT_FUTURE_CANCELLATION_TIMEOUT: float = 2.0

# 取消常规任务的默认超时时间
DEFAULT_TASK_CANCELLATION_TIMEOUT: float = 5.0


async def cancel_tasks_with_timeout(
    tasks: WeakSet[asyncio.Task] | set[asyncio.Task | asyncio.Future],
    logger: Logger | None = None,
    timeout_secs: float = DEFAULT_TASK_CANCELLATION_TIMEOUT,
) -> None:
    """
    取消所有待处理任务，并等待它们在超时时间内完成。

    此函数会对任务进行强引用快照，以确保它们在取消过程中不被垃圾回收。
    它会取消所有待处理任务，并等待它们在指定的超时时间内完成。

    参数
    ----------
    tasks : WeakSet[asyncio.Task] | set[asyncio.Task | asyncio.Future]
        要取消的任务集合。可以是 WeakSet（用于正常操作）或常规集合（用于 future）。
    logger : Logger | None, 可选
        用于调试和警告消息的记录器。
    timeout_secs : float, 默认 5.0
        等待任务完成取消的最大时间。

    注意
    -----
    - 采用强引用快照，防止任务在取消期间被垃圾回收（GC）。
    - 使用 return_exceptions=True 以防止“异常未被检索”的警告。
    - 如果任务未在指定超时内完成，则记录超时警告。

    """
    # 获取强引用快照，防止任务在取消期间消失
    pending_tasks = [task for task in tasks if not task.done()]

    if not pending_tasks:
        if logger:
            logger.debug("没有待取消的任务")
        return

    if logger:
        logger.debug(f"正在取消 {len(pending_tasks)} 个待处理任务")

    # 取消所有任务
    for task in pending_tasks:
        task.cancel()

    # 使用我们捕获的强引用进行等待
    try:
        await asyncio.wait_for(
            asyncio.gather(*pending_tasks, return_exceptions=True),
            timeout=timeout_secs,
        )
        if logger:
            logger.debug(f"已成功取消 {len(pending_tasks)} 个任务")
    except TimeoutError:
        if logger:
            logger.warning(
                f"等待 {len(pending_tasks)} 个任务取消超时 ({timeout_secs}s)",
            )
            _log_still_pending_tasks(pending_tasks, logger)


def _log_still_pending_tasks(
    pending_tasks: list[asyncio.Task | asyncio.Future],
    logger: Logger,
) -> None:
    still_pending = [task for task in pending_tasks if not task.done()]
    if not still_pending:
        return

    for task in still_pending:
        # Task 有 get_name()，Future 没有
        if hasattr(task, "get_name"):
            logger.warning(f"任务仍未完成: {task.get_name()} (id={id(task)})")
        else:
            logger.warning(f"Future 仍未完成: id={id(task)}")
