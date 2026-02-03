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
from collections.abc import Awaitable
from collections.abc import Callable
from random import randint

from nautilus_trader.common.component import Logger


def get_exponential_backoff(
    num_attempts: int,
    delay_initial_ms: int = 500,
    delay_max_ms: int = 2_000,
    backoff_factor: int = 2,
    jitter: bool = True,
) -> int:
    """
    使用指数退避（exponential backoff）和抖动（jitter）计算退避延迟时间。

    参数
    ----------
    num_attempts : int, 默认 1
        已经尝试过的次数。
    delay_initial_ms : int, 默认 500
        第一次尝试时的休眠时间（毫秒）。
    delay_max_ms : int, 默认 2_000
        最大延迟（毫秒）。
    backoff_factor : int, 默认 2
        延迟的指数退避因子。
    jitter : bool, 默认 True
        是否应用抖动。

    注意
    -----
    参考：https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/

    返回
    -------
    int
        以毫秒为单位的延迟时间。

    """
    delay = min(delay_max_ms, delay_initial_ms * backoff_factor ** (num_attempts - 1))

    if jitter:
        return randint(delay_initial_ms, delay)  # noqa: S311

    return delay


class RetryManager[T]:
    """
    为 HTTP 请求提供重试状态管理。

    此类对 `T` 是泛型的，其中 `T` 是传递给 `run` 方法的函数的返回类型。

    参数
    ----------
    max_retries : int
        失败前的最大重试次数。
    delay_initial_ms : int
        重试的初始延迟（毫秒）。
    delay_max_ms : int
        指数退避的最大延迟（毫秒）。
    backoff_factor : int
        重试延迟的指数退避因子。
    logger : Logger
        管理器的日志记录器。
    exc_types : tuple[Type[BaseException], ...]
        重试时要处理的异常类型。
    retry_check : Callable[[BaseException], None], 可选
        对异常执行额外检查的函数。
        如果该函数返回 `False`，则不会尝试重试。
    error_logger : Callable[[str, BaseException | None], None], 可选
        用于替代默认 `logger.error` 的自定义错误日志记录函数。

    """

    def __init__(
        self,
        max_retries: int,
        delay_initial_ms: int,
        delay_max_ms: int,
        backoff_factor: int,
        logger: Logger,
        exc_types: tuple[type[BaseException], ...],
        retry_check: Callable[[BaseException], bool] | None = None,
        error_logger: Callable[[str, BaseException | None], None] | None = None,
    ) -> None:
        self.max_retries = max_retries
        self.delay_initial_ms = delay_initial_ms
        self.delay_max_ms = delay_max_ms
        self.backoff_factor = backoff_factor
        self.retries = 0
        self.exc_types = exc_types
        self.retry_check = retry_check
        self.error_logger = error_logger
        self.cancel_event = asyncio.Event()
        self.log = logger

        self.name: str | None = None
        self.details: list[object] | None = None
        self.details_str: str | None = None
        self.result: bool = False
        self.message: str | None = None
        self.last_exception: BaseException | None = None

    def __repr__(self) -> str:
        return f"<{type(self).__name__}(name='{self.name}', details={self.details}) at {hex(id(self))}>"

    async def run(
        self,
        name: str,
        details: list[object] | None,
        func: Callable[..., Awaitable[T]],
        *args,
        **kwargs,
    ) -> T | None:
        """
        通过重试管理执行给定的 `func`。

        如果抛出 `self.exc_types` 中的异常，则记录警告，并在延迟后重试该函数，
        直到达到最大重试次数，届时将记录错误。

        参数
        ----------
        name : str
            要运行的操作名称。
        details : list[object], 可选
            标识符等操作详情。
        func : Callable[..., Awaitable[T]]
            要执行的函数。
        args : Any
            要传递给函数 `func` 的位置参数。
        kwargs : Any
            要传递给函数 `func` 的关键字参数。

        返回
        -------
        T | None
            执行函数的结果，如果重试失败则返回 ``None``。

        """
        self.name = name
        self.details = details

        try:
            while True:
                if self.cancel_event.is_set():
                    self._cancel()
                    return None

                try:
                    response = await func(*args, **kwargs)
                    self.result = True
                    return response  # 请求成功
                except self.exc_types as e:
                    self.last_exception = e

                    if (
                        (self.retry_check and not self.retry_check(e))
                        or not self.max_retries
                        or self.retries >= self.max_retries
                    ):
                        self._log_error()
                        self.result = False
                        self.message = str(e)
                        return None  # 操作失败
                    self.retries += 1
                    retry_delay_ms = get_exponential_backoff(
                        delay_initial_ms=self.delay_initial_ms,
                        delay_max_ms=self.delay_max_ms,
                        backoff_factor=self.backoff_factor,
                        num_attempts=self.retries,
                        jitter=True,
                    )
                    self._log_retry(retry_delay_ms=retry_delay_ms)
                    await asyncio.sleep(retry_delay_ms / 1000)
        except asyncio.CancelledError:
            self._cancel()
            return None

    def cancel(self) -> None:
        """
        取消重试操作。
        """
        self.log.debug(f"正在取消 {self!r}")
        self.cancel_event.set()

    def clear(self) -> None:
        """
        清除此重试管理器的所有状态。
        """
        self.retries = 0
        self.name = None
        self.details = None
        self.details_str = None
        self.result = False
        self.message = None
        self.last_exception = None

    def _cancel(self) -> None:
        self.log.warning(f"已取消 '{self.name}' 的重试")
        self.result = False
        self.message = "重试已取消"

    def _log_retry(self, retry_delay_ms: int) -> None:
        self.log.warning(
            f"正在重试 '{self.name}' ({self.retries}/{self.max_retries})，"
            f"延迟 {retry_delay_ms / 1000} 秒{self._details_str()}",
        )

    def _log_error(self) -> None:
        message = f"执行 {self.name} 失败{self._details_str()}"
        if self.error_logger:
            self.error_logger(message, self.last_exception)
        else:
            self.log.error(message)

    def _details_str(self) -> str:
        self.details_str = " " + ", ".join([repr(x) for x in self.details]) if self.details else ""
        self.details_str += f": {self.last_exception!r}" if self.last_exception else ""
        return self.details_str


class RetryManagerPool[T]:
    """
    提供 `RetryManager` 对象池。

    参数
    ----------
    pool_size : int
        重试管理器池的大小。
    max_retries : int
        失败前的最大重试次数。
    delay_initial_ms : int
        重试的初始延迟（毫秒）。
    delay_max_ms : int
        指数退避的最大延迟（毫秒）。
    backoff_factor : int
        重试延迟的指数退避因子。
    logger : Logger
        用于重试管理器的日志记录器。
    exc_types : tuple[Type[BaseException], ...]
        重试时要处理的异常类型。
    retry_check : Callable[[BaseException], None], 可选
        对异常执行额外检查的函数。
        如果该函数返回 `False`，则不会尝试重试。
    error_logger : Callable[[str, BaseException | None], None], 可选
        用于替代默认 `logger.error` 的自定义错误日志记录函数。

    """

    def __init__(
        self,
        pool_size: int,
        max_retries: int,
        delay_initial_ms: int,
        delay_max_ms: int,
        backoff_factor: int,
        logger: Logger,
        exc_types: tuple[type[BaseException], ...],
        retry_check: Callable[[BaseException], bool] | None = None,
        error_logger: Callable[[str, BaseException | None], None] | None = None,
    ) -> None:
        self.max_retries = max_retries
        self.delay_initial_ms = delay_initial_ms
        self.delay_max_ms = delay_max_ms
        self.backoff_factor = backoff_factor
        self.logger = logger
        self.exc_types = exc_types
        self.retry_check = retry_check
        self.error_logger = error_logger
        self.pool_size = pool_size
        self._pool: list[RetryManager[T]] = [self._create_manager() for _ in range(pool_size)]
        self._lock = asyncio.Lock()
        self._active_managers: set[RetryManager[T]] = set()

    def _create_manager(self) -> RetryManager:
        return RetryManager(
            max_retries=self.max_retries,
            delay_initial_ms=self.delay_initial_ms,
            delay_max_ms=self.delay_max_ms,
            backoff_factor=self.backoff_factor,
            logger=self.logger,
            exc_types=self.exc_types,
            retry_check=self.retry_check,
            error_logger=self.error_logger,
        )

    def shutdown(self) -> None:
        """
        优雅地关闭重试管理器池，确保所有活动的重试管理器都被取消。

        当使用该池的组件停止时，应调用此方法，以确保所有资源有序释放。

        """
        self.logger.info("正在关闭重试管理器池")
        for retry_manager in self._active_managers:
            retry_manager.cancel()
        self._active_managers.clear()

    async def acquire(self) -> RetryManager:
        """
        从池中获取一个 `RetryManager`，如果池为空则创建一个新的。

        返回
        -------
        RetryManager

        """
        async with self._lock:
            if self._pool:
                # 弹出最近使用的管理器并清除其状态
                retry_manager = self._pool.pop()
                retry_manager.clear()
            else:
                # 如果池为空，则创建新的管理器
                retry_manager = self._create_manager()

            self._active_managers.add(retry_manager)
            self.logger.debug(f"已获取 {retry_manager!r} (活动中: {len(self._active_managers)})")
            return retry_manager

    async def release(self, retry_manager: RetryManager) -> None:
        """
        将给定的 `retry_manager` 释放回池中。

        如果池已满，该 `retry_manager` 将被丢弃。

        参数
        ----------
        retry_manager : RetryManager
            要返回池中的管理器。

        """
        async with self._lock:
            self._active_managers.discard(retry_manager)

            if len(self._pool) < self.pool_size:
                # 将管理器附加到池中而不清除其状态，
                # 状态在获取时被清除，以避免潜在的竞态条件。
                self._pool.append(retry_manager)
                self.logger.debug(
                    f"已将 {retry_manager!r} 释放回池中 (活动中: {len(self._active_managers)})",
                )
            else:
                # 池已满
                self.logger.debug(
                    f"丢弃多余的 {retry_manager!r} (活动中: {len(self._active_managers)})",
                )
