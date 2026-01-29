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
`LiveDataClient` 类负责与特定 API 进行交互，该 API 可能由场地直接提供，
也可能通过经纪商中间商提供。

也可以为专门的数据提供商编写客户端。

"""

import asyncio
import functools
from asyncio import Task
from collections.abc import Callable
from collections.abc import Coroutine
from weakref import WeakSet

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.functions import format_utc_timerange
from nautilus_trader.common.providers import InstrumentProvider
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.data.client import DataClient
from nautilus_trader.data.client import MarketDataClient
from nautilus_trader.data.messages import RequestBars
from nautilus_trader.data.messages import RequestData
from nautilus_trader.data.messages import RequestFundingRates
from nautilus_trader.data.messages import RequestInstrument
from nautilus_trader.data.messages import RequestInstruments
from nautilus_trader.data.messages import RequestOrderBookDepth
from nautilus_trader.data.messages import RequestOrderBookSnapshot
from nautilus_trader.data.messages import RequestQuoteTicks
from nautilus_trader.data.messages import RequestTradeTicks
from nautilus_trader.data.messages import SubscribeBars
from nautilus_trader.data.messages import SubscribeData
from nautilus_trader.data.messages import SubscribeFundingRates
from nautilus_trader.data.messages import SubscribeIndexPrices
from nautilus_trader.data.messages import SubscribeInstrument
from nautilus_trader.data.messages import SubscribeInstrumentClose
from nautilus_trader.data.messages import SubscribeInstruments
from nautilus_trader.data.messages import SubscribeInstrumentStatus
from nautilus_trader.data.messages import SubscribeMarkPrices
from nautilus_trader.data.messages import SubscribeOrderBook
from nautilus_trader.data.messages import SubscribeQuoteTicks
from nautilus_trader.data.messages import SubscribeTradeTicks
from nautilus_trader.data.messages import UnsubscribeBars
from nautilus_trader.data.messages import UnsubscribeData
from nautilus_trader.data.messages import UnsubscribeFundingRates
from nautilus_trader.data.messages import UnsubscribeIndexPrices
from nautilus_trader.data.messages import UnsubscribeInstrument
from nautilus_trader.data.messages import UnsubscribeInstrumentClose
from nautilus_trader.data.messages import UnsubscribeInstruments
from nautilus_trader.data.messages import UnsubscribeInstrumentStatus
from nautilus_trader.data.messages import UnsubscribeMarkPrices
from nautilus_trader.data.messages import UnsubscribeOrderBook
from nautilus_trader.data.messages import UnsubscribeQuoteTicks
from nautilus_trader.data.messages import UnsubscribeTradeTicks
from nautilus_trader.live.cancellation import cancel_tasks_with_timeout
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import Venue


class LiveDataClient(DataClient):
    """
    所有实盘数据客户端的基类。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端的事件循环。
    client_id : ClientId
        客户端 ID。
    venue : Venue 或 ``None``
        客户端场地。如果是多场地，则可以为理论上的 ``None``。
    msgbus : MessageBus
        客户端的消息总线。
    cache : Cache
        客户端的缓存。
    clock : LiveClock
        客户端的时钟。
    config : NautilusConfig, 可选
        实例的配置。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client_id: ClientId,
        venue: Venue | None,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        config: NautilusConfig | None = None,
    ) -> None:
        super().__init__(
            client_id=client_id,
            venue=venue,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop = loop
        self._tasks: WeakSet[asyncio.Task] = WeakSet()

    async def run_after_delay(
        self,
        delay: float,
        coro: Coroutine,
    ) -> None:
        """
        延迟一定时间后运行给定的协程（coroutine）。

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
        success_color : LogColor, 默认 ``NORMAL``
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
                        f"在任务 '{task.get_name()}' 成功后触发动作 {actions.__name__} 失败",
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

    # -- SUBSCRIPTIONS ----------------------------------------------------------------------------

    def subscribe(self, command: SubscribeData) -> None:
        self._add_subscription(command.data_type)
        self.create_task(
            self._subscribe(command),
            log_msg=f"订阅: {command.data_type}",
            success_msg=f"已订阅 {command.data_type}",
            success_color=LogColor.BLUE,
        )

    def unsubscribe(self, command: UnsubscribeData) -> None:
        self._remove_subscription(command.data_type)
        self.create_task(
            self._unsubscribe(command),
            log_msg=f"取消订阅: {command.data_type}",
            success_msg=f"已取消订阅 {command.data_type}",
            success_color=LogColor.BLUE,
        )

    # -- REQUESTS ---------------------------------------------------------------------------------

    def request(self, request: RequestData) -> None:
        self._log.debug(f"请求 {request.data_type} {request.id}")
        self.create_task(
            self._request(request),
            log_msg=f"请求_{request.data_type}",
        )

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

    async def _subscribe(self, command: SubscribeData) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe` 协程",  # pragma: no cover
        )

    async def _unsubscribe(self, command: UnsubscribeBars) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe` 协程",  # pragma: no cover
        )

    async def _request(self, request: RequestData) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request` 协程",  # pragma: no cover
        )

    async def cancel_pending_tasks(self, timeout_secs: float = 5.0) -> None:
        """
        取消所有待处理任务并等待其取消。

        参数
        ----------
        timeout_secs : float, 默认 5.0
            等待任务取消的超时时间（秒）。

        """
        await cancel_tasks_with_timeout(self._tasks, self._log, timeout_secs)


class LiveMarketDataClient(MarketDataClient):
    """
    所有实盘行情数据客户端的基类。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端的事件循环。
    client_id : ClientId
        客户端 ID。
    venue : Venue 或 ``None``
        客户端场地。如果是多场地，则可以为理论上的 ``None``。
    msgbus : MessageBus
        客户端的消息总线。
    cache : Cache
        客户端的缓存。
    clock : LiveClock
        客户端的时钟。
    instrument_provider : InstrumentProvider
        客户端的标的提供者。
    config : NautilusConfig, 可选
        实例的配置。

    警告
    --------
    此类不应直接使用，而应通过具体的子类使用。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        client_id: ClientId,
        venue: Venue | None,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        instrument_provider: InstrumentProvider,
        config: NautilusConfig | None = None,
        is_sync: bool = False,
    ) -> None:
        PyCondition.type(instrument_provider, InstrumentProvider, "instrument_provider")

        super().__init__(
            client_id=client_id,
            venue=venue,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop = loop
        self._instrument_provider = instrument_provider
        self._is_sync = is_sync
        self._tasks: WeakSet[asyncio.Task] = WeakSet()
        self._disconnect_task: asyncio.Task | None = None

        if self._is_sync:
            self._log.warning(
                "客户端初始化为同步模式；"
                "如果在 Jupyter Notebook 等异步环境中运行，请确保调用了 nest_asyncio.apply()",
            )

    async def run_after_delay(
        self,
        delay: float,
        coro: Coroutine,
    ) -> None:
        """
        延迟一定时间后运行给定的协程（coroutine）。

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
    ) -> asyncio.Task | None:
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
        success_color : LogColor, 默认 ``NORMAL``
            动作成功完成后日志消息的颜色。

        返回
        -------
        asyncio.Task

        """
        task_name = log_msg or coro.__name__

        if self._is_sync:
            self._log.debug(f"正在同步运行协程 '{task_name}'...")
            result = None
            exception: BaseException | None = None

            try:
                result = asyncio.run(coro)
            except Exception as e:
                exception = e

            self._handle_completion(
                coro_name=task_name,
                actions=actions,
                success_msg=success_msg,
                success_color=success_color,
                exception=exception,
            )

            if exception:
                self._log.error(f"同步执行 '{task_name}' 失败")
                return None
            else:
                return result

        self._log.debug(f"正在创建异步任务 '{task_name}'")

        if not self._loop or not self._loop.is_running():
            self._log.error(f"异步任务 '{task_name}' 已创建，但事件循环未运行")
            return None

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
        coro_name = task.get_name()
        exception = None

        try:
            task.result()
        except asyncio.CancelledError:
            self._log.warning(f"任务 '{coro_name}' 已取消")
            return
        except Exception as e:
            exception = e

        self._handle_completion(
            coro_name=coro_name,
            actions=actions,
            success_msg=success_msg,
            success_color=success_color,
            exception=exception,
        )

    def _handle_completion(
        self,
        coro_name: str,
        actions: Callable[[], None] | None,
        success_msg: str | None,
        success_color: LogColor,
        exception: BaseException | None = None,
    ) -> None:
        if exception:
            self._log.exception(f"运行 '{coro_name}' 时出错", exception)
        else:
            self._log.debug(f"协程 '{coro_name}' 已完成")

            if actions:
                try:
                    actions()
                except Exception as e:
                    self._log.exception(
                        f"在 '{coro_name}' 成功完成之后触发动作 {getattr(actions, '__name__', 'N/A')} 失败",
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

        # 直接创建断开连接任务，不使用 create_task 辅助函数，
        # 这样它就不会被 cancel_pending_tasks() 取消
        self._loop.create_task(_disconnect_with_cleanup())

    # -- SUBSCRIPTIONS ----------------------------------------------------------------------------

    def subscribe(self, command: SubscribeData) -> None:
        self._add_subscription(command.data_type)
        self.create_task(
            self._subscribe(command),
            log_msg=f"订阅: {command.data_type}",
            success_msg=f"已订阅 {command.data_type}",
            success_color=LogColor.BLUE,
        )

    def subscribe_instruments(self, command: SubscribeInstruments) -> None:
        instrument_ids = list(self._instrument_provider.get_all().keys())
        [self._add_subscription_instrument(i) for i in instrument_ids]
        self.create_task(
            self._subscribe_instruments(command),
            log_msg=f"订阅: {self.venue} 标的",
            success_msg=f"已订阅 {self.venue} 标的",
            success_color=LogColor.BLUE,
        )

    def subscribe_instrument(self, command: SubscribeInstrument) -> None:
        self._add_subscription_instrument(command.instrument_id)
        self.create_task(
            self._subscribe_instrument(command),
            log_msg=f"订阅: 标的 {command.instrument_id}",
            success_msg=f"已订阅标的 {command.instrument_id}",
            success_color=LogColor.BLUE,
        )

    def subscribe_order_book_deltas(self, command: SubscribeOrderBook) -> None:
        self._add_subscription_order_book_deltas(command.instrument_id)
        self.create_task(
            self._subscribe_order_book_deltas(command),
            log_msg=f"订阅: 订单簿增量 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 订单簿增量; 深度={command.depth}",
            success_color=LogColor.BLUE,
        )

    def subscribe_order_book_depth(self, command: SubscribeOrderBook) -> None:
        self._add_subscription_order_book_depth(command.instrument_id)
        self.create_task(
            self._subscribe_order_book_depth(command),
            log_msg=f"订阅: 订单簿深度 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 订单簿深度; 深度={command.depth}",
            success_color=LogColor.BLUE,
        )

    def subscribe_quote_ticks(self, command: SubscribeQuoteTicks) -> None:
        self._add_subscription_quote_ticks(command.instrument_id)
        self.create_task(
            self._subscribe_quote_ticks(command),
            log_msg=f"订阅: 报价跳动 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 报价",
            success_color=LogColor.BLUE,
        )

    def subscribe_trade_ticks(self, command: SubscribeTradeTicks) -> None:
        self._add_subscription_trade_ticks(command.instrument_id)
        self.create_task(
            self._subscribe_trade_ticks(command),
            log_msg=f"订阅: 成交跳动 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 成交记录",
            success_color=LogColor.BLUE,
        )

    def subscribe_mark_prices(self, command: SubscribeMarkPrices) -> None:
        self._add_subscription_mark_prices(command.instrument_id)
        self.create_task(
            self._subscribe_mark_prices(command),
            log_msg=f"订阅: 标记价格 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 标记价格",
            success_color=LogColor.BLUE,
        )

    def subscribe_index_prices(self, command: SubscribeIndexPrices) -> None:
        self._add_subscription_index_prices(command.instrument_id)
        self.create_task(
            self._subscribe_index_prices(command),
            log_msg=f"订阅: 指数价格 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 指数价格",
            success_color=LogColor.BLUE,
        )

    def subscribe_funding_rates(self, command: SubscribeFundingRates) -> None:
        self._add_subscription_funding_rates(command.instrument_id)
        self.create_task(
            self._subscribe_funding_rates(command),
            log_msg=f"订阅: 资金费率 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 资金费率",
            success_color=LogColor.BLUE,
        )

    def subscribe_bars(self, command: SubscribeBars) -> None:
        PyCondition.is_true(
            command.bar_type.is_externally_aggregated(),
            "aggregation_source is not EXTERNAL",
        )

        self._add_subscription_bars(command.bar_type)
        self.create_task(
            self._subscribe_bars(command),
            log_msg=f"订阅: K 线 {command.bar_type}",
            success_msg=f"已订阅 {command.bar_type} K 线",
            success_color=LogColor.BLUE,
        )

    def subscribe_instrument_status(self, command: SubscribeInstrumentStatus) -> None:
        self._add_subscription_instrument_status(command.instrument_id)
        self.create_task(
            self._subscribe_instrument_status(command),
            log_msg=f"订阅: 标的状态 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 标的状态",
            success_color=LogColor.BLUE,
        )

    def subscribe_instrument_close(self, command: SubscribeInstrumentClose) -> None:
        self._add_subscription_instrument_close(command.instrument_id)
        self.create_task(
            self._subscribe_instrument_close(command),
            log_msg=f"订阅: 标的收盘 {command.instrument_id}",
            success_msg=f"已订阅 {command.instrument_id} 标的收盘",
            success_color=LogColor.BLUE,
        )

    def unsubscribe(self, command: UnsubscribeData) -> None:
        self._remove_subscription(command.data_type)
        self.create_task(
            self._unsubscribe(command),
            log_msg=f"取消订阅 {command.data_type}",
            success_msg=f"已取消订阅 {command.data_type}",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_instruments(self, command: UnsubscribeInstruments) -> None:
        instrument_ids = list(self._instrument_provider.get_all().keys())
        [self._remove_subscription_instrument(i) for i in instrument_ids]
        self.create_task(
            self._unsubscribe_instruments(command),
            log_msg=f"取消订阅: {self.venue} 标的",
            success_msg=f"已取消订阅 {self.venue} 标的",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_instrument(self, command: UnsubscribeInstrument) -> None:
        self._remove_subscription_instrument(command.instrument_id)
        self.create_task(
            self._unsubscribe_instrument(command),
            log_msg=f"取消订阅: 标的 {command.instrument_id}",
            success_msg=f"已取消订阅标的 {command.instrument_id}",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_order_book_deltas(self, command: UnsubscribeOrderBook) -> None:
        self._remove_subscription_order_book_deltas(command.instrument_id)
        self.create_task(
            self._unsubscribe_order_book_deltas(command),
            log_msg=f"取消订阅: 订单簿增量 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 订单簿增量",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_order_book_depth(self, command: UnsubscribeOrderBook) -> None:
        self._remove_subscription_order_book_depth(command.instrument_id)
        self.create_task(
            self._unsubscribe_order_book_depth(command),
            log_msg=f"取消订阅: 订单簿深度 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 订单簿深度",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_quote_ticks(self, command: UnsubscribeQuoteTicks) -> None:
        self._remove_subscription_quote_ticks(command.instrument_id)
        self.create_task(
            self._unsubscribe_quote_ticks(command),
            log_msg=f"取消订阅: 报价跳动 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 报价",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_trade_ticks(self, command: UnsubscribeTradeTicks) -> None:
        self._remove_subscription_trade_ticks(command.instrument_id)
        self.create_task(
            self._unsubscribe_trade_ticks(command),
            log_msg=f"取消订阅: 成交跳动 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 成交记录",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_mark_prices(self, command: UnsubscribeMarkPrices) -> None:
        self._remove_subscription_mark_prices(command.instrument_id)
        self.create_task(
            self._unsubscribe_mark_prices(command),
            log_msg=f"取消订阅: 标记价格 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 标记价格",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_index_prices(self, command: UnsubscribeIndexPrices) -> None:
        self._remove_subscription_index_prices(command.instrument_id)
        self.create_task(
            self._unsubscribe_index_prices(command),
            log_msg=f"取消订阅: 指数价格 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 指数价格",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_funding_rates(self, command: UnsubscribeFundingRates) -> None:
        self._remove_subscription_funding_rates(command.instrument_id)
        self.create_task(
            self._unsubscribe_funding_rates(command),
            log_msg=f"取消订阅: 资金费率 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 资金费率",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_bars(self, command: UnsubscribeBars) -> None:
        self._remove_subscription_bars(command.bar_type)
        self.create_task(
            self._unsubscribe_bars(command),
            log_msg=f"取消订阅: K 线 {command.bar_type}",
            success_msg=f"已取消订阅 {command.bar_type} K 线",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_instrument_status(self, command: UnsubscribeInstrumentStatus) -> None:
        self._remove_subscription_instrument_status(command.instrument_id)
        self.create_task(
            self._unsubscribe_instrument_status(command),
            log_msg=f"取消订阅: 标的状态 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 标的状态",
            success_color=LogColor.BLUE,
        )

    def unsubscribe_instrument_close(self, command: UnsubscribeInstrumentClose) -> None:
        self._remove_subscription_instrument_close(command.instrument_id)
        self.create_task(
            self._unsubscribe_instrument_close(command),
            log_msg=f"取消订阅: 标的收盘 {command.instrument_id}",
            success_msg=f"已取消订阅 {command.instrument_id} 标的收盘",
            success_color=LogColor.BLUE,
        )

    # -- REQUESTS ---------------------------------------------------------------------------------

    def request(self, request: RequestData) -> None:
        self._log.info(f"请求 {request.data_type}", LogColor.BLUE)
        self.create_task(
            self._request(request),
            log_msg=f"请求: {request.data_type}",
        )

    def request_instrument(self, request: RequestInstrument) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        self._log.info(f"请求 {request.instrument_id} 标的{time_range_str}", LogColor.BLUE)
        self.create_task(
            self._request_instrument(request),
            log_msg=f"请求: 标的 {request.instrument_id}",
        )

    def request_instruments(self, request: RequestInstruments) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        self._log.info(
            f"请求 {request.venue} 标的列表，范围{time_range_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_instruments(request),
            log_msg=f"请求: {request.venue} 标的列表",
        )

    def request_quote_ticks(self, request: RequestQuoteTicks) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        self._log.info(
            f"请求 {request.instrument_id} 报价{time_range_str}{limit_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_quote_ticks(request),
            log_msg=f"请求: 报价 {request.instrument_id}",
        )

    def request_trade_ticks(self, request: RequestTradeTicks) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        self._log.info(
            f"请求 {request.instrument_id} 成交记录{time_range_str}{limit_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_trade_ticks(request),
            log_msg=f"请求: 成交记录 {request.instrument_id}",
        )

    def request_funding_rates(self, request: RequestFundingRates) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        self._log.info(
            f"请求 {request.instrument_id} 资金费率{time_range_str}{limit_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_funding_rates(request),
            log_msg=f"请求: 资金费率 {request.instrument_id}",
        )

    def request_bars(self, request: RequestBars) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        self._log.info(f"请求 {request.bar_type} K 线{time_range_str}{limit_str}", LogColor.BLUE)
        self.create_task(
            self._request_bars(request),
            log_msg=f"请求: K 线 {request.bar_type}",
        )

    def request_order_book_snapshot(self, request: RequestOrderBookSnapshot) -> None:
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        self._log.info(
            f"请求 {request.instrument_id} 订单簿快照{limit_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_order_book_snapshot(request),
            log_msg=f"请求: 订单簿快照 {request.instrument_id}",
        )

    def request_order_book_depth(self, request: RequestOrderBookDepth) -> None:
        time_range_str = format_utc_timerange(request.start, request.end)
        limit_str = f" limit={request.limit}" if request.limit != 0 else ""
        depth_str = f" depth={request.depth}"
        self._log.info(
            f"请求 {request.instrument_id} 订单簿深度{time_range_str}{limit_str}{depth_str}",
            LogColor.BLUE,
        )
        self.create_task(
            self._request_order_book_depth(request),
            log_msg=f"请求: 订单簿深度 {request.instrument_id}",
        )

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

    async def _subscribe(self, command: SubscribeData) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe` 协程",  # pragma: no cover
        )

    async def _subscribe_instruments(self, command: SubscribeInstruments) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_instruments` 协程",  # pragma: no cover
        )

    async def _subscribe_instrument(self, command: SubscribeInstrument) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_instrument` 协程",  # pragma: no cover
        )

    async def _subscribe_order_book_deltas(self, command: SubscribeOrderBook) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_order_book_deltas` 协程",  # pragma: no cover
        )

    async def _subscribe_order_book_depth(self, command: SubscribeOrderBook) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_order_book_depth` 协程",  # pragma: no cover
        )

    async def _subscribe_quote_ticks(self, command: SubscribeQuoteTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_quote_ticks` 协程",  # pragma: no cover
        )

    async def _subscribe_trade_ticks(self, command: SubscribeTradeTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_trade_ticks` 协程",  # pragma: no cover
        )

    async def _subscribe_mark_prices(self, command: SubscribeMarkPrices) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_mark_prices` 协程",  # pragma: no cover
        )

    async def _subscribe_index_prices(self, command: SubscribeIndexPrices) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_index_prices` 协程",  # pragma: no cover
        )

    async def _subscribe_funding_rates(self, command: SubscribeFundingRates) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_funding_rates` 协程",  # pragma: no cover
        )

    async def _subscribe_bars(self, command: SubscribeBars) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_bars` 协程",  # pragma: no cover
        )

    async def _subscribe_instrument_status(self, command: SubscribeInstrumentStatus) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_instrument_status` 协程",  # pragma: no cover
        )

    async def _subscribe_instrument_close(self, command: SubscribeInstrumentClose) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_subscribe_instrument_close` 协程",  # pragma: no cover
        )

    async def _unsubscribe(self, command: UnsubscribeData) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe` 协程",  # pragma: no cover
        )

    async def _unsubscribe_instruments(self, command: UnsubscribeInstruments) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_instruments` 协程",  # pragma: no cover
        )

    async def _unsubscribe_instrument(self, command: UnsubscribeInstrument) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_instrument` 协程",  # pragma: no cover
        )

    async def _unsubscribe_order_book_deltas(self, command: UnsubscribeOrderBook) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_order_book_deltas` 协程",  # pragma: no cover
        )

    async def _unsubscribe_order_book_depth(self, command: UnsubscribeOrderBook) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_order_book_depth` 协程",  # pragma: no cover
        )

    async def _unsubscribe_quote_ticks(self, command: UnsubscribeQuoteTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_quote_ticks` 协程",  # pragma: no cover
        )

    async def _unsubscribe_trade_ticks(self, command: UnsubscribeTradeTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_trade_ticks` 协程",  # pragma: no cover
        )

    async def _unsubscribe_mark_prices(self, command: UnsubscribeMarkPrices) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_mark_prices` 协程",  # pragma: no cover
        )

    async def _unsubscribe_index_prices(self, command: UnsubscribeIndexPrices) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_index_prices` 协程",  # pragma: no cover
        )

    async def _unsubscribe_funding_rates(self, command: UnsubscribeFundingRates) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_funding_rates` 协程",  # pragma: no cover
        )

    async def _unsubscribe_bars(self, command: UnsubscribeBars) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_bars` 协程",  # pragma: no cover
        )

    async def _unsubscribe_instrument_status(self, command: UnsubscribeInstrumentStatus) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_instrument_status` 协程",  # pragma: no cover
        )

    async def _unsubscribe_instrument_close(self, command: UnsubscribeInstrumentClose) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_unsubscribe_instrument_close` 协程",  # pragma: no cover
        )

    async def _request(self, request: RequestData) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request` 协程",  # pragma: no cover
        )

    async def _request_instrument(self, request: RequestInstrument) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_instrument` 协程",  # pragma: no cover
        )

    async def _request_instruments(self, request: RequestInstruments) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_instruments` 协程",  # pragma: no cover
        )

    async def _request_quote_ticks(self, request: RequestQuoteTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_quote_ticks` 协程",  # pragma: no cover
        )

    async def _request_trade_ticks(self, request: RequestTradeTicks) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_trade_ticks` 协程",  # pragma: no cover
        )

    async def _request_funding_rates(self, request: RequestFundingRates) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_funding_rates` 协程",  # pragma: no cover
        )

    async def _request_bars(self, request: RequestBars) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_bars` 协程",  # pragma: no cover
        )

    async def _request_order_book_snapshot(self, request: RequestOrderBookSnapshot) -> None:
        raise NotImplementedError(
            "请实现 `_request_order_book_snapshot` 协程",  # pragma: no cover
        )

    async def _request_order_book_depth(self, request: RequestOrderBookDepth) -> None:
        raise NotImplementedError(  # pragma: no cover
            "请实现 `_request_order_book_depth` 协程",  # pragma: no cover
        )

    async def cancel_pending_tasks(self, timeout_secs: float = 5.0) -> None:
        """
        取消所有待处理任务并等待其取消。

        参数
        ----------
        timeout_secs : float, 默认 5.0
            等待任务取消的超时时间（秒）。

        """
        await cancel_tasks_with_timeout(self._tasks, self._log, timeout_secs)
