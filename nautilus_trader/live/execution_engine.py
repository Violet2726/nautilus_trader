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
import math
import os
from asyncio import Queue
from collections import Counter
from collections.abc import Iterable
from decimal import Decimal
from typing import Any
from typing import Final
from typing import cast

import pandas as pd

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.enums import LogLevel
from nautilus_trader.config import LiveExecEngineConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.datetime import dt_to_unix_nanos
from nautilus_trader.core.datetime import millis_to_nanos
from nautilus_trader.core.datetime import secs_to_nanos
from nautilus_trader.core.fsm import InvalidStateTrigger
from nautilus_trader.core.message import Command
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.execution.client import ExecutionClient
from nautilus_trader.execution.engine import ExecutionEngine
from nautilus_trader.execution.messages import GenerateExecutionMassStatus
from nautilus_trader.execution.messages import GenerateFillReports
from nautilus_trader.execution.messages import GenerateOrderStatusReport
from nautilus_trader.execution.messages import GenerateOrderStatusReports
from nautilus_trader.execution.messages import GeneratePositionStatusReports
from nautilus_trader.execution.messages import QueryOrder
from nautilus_trader.execution.reports import ExecutionMassStatus
from nautilus_trader.execution.reports import ExecutionReport
from nautilus_trader.execution.reports import FillReport
from nautilus_trader.execution.reports import OrderStatusReport
from nautilus_trader.execution.reports import PositionStatusReport
from nautilus_trader.live.enqueue import ThrottledEnqueuer
from nautilus_trader.live.reconciliation import adjust_fills_for_partial_window
from nautilus_trader.live.reconciliation import calculate_reconciliation_price
from nautilus_trader.live.reconciliation import create_inferred_order_filled_event
from nautilus_trader.live.reconciliation import create_order_accepted_event
from nautilus_trader.live.reconciliation import create_order_canceled_event
from nautilus_trader.live.reconciliation import create_order_expired_event
from nautilus_trader.live.reconciliation import create_order_filled_event
from nautilus_trader.live.reconciliation import create_order_rejected_event
from nautilus_trader.live.reconciliation import create_order_triggered_event
from nautilus_trader.live.reconciliation import create_order_updated_event
from nautilus_trader.live.reconciliation import get_existing_fill_for_trade_id
from nautilus_trader.live.reconciliation import is_within_single_unit_tolerance
from nautilus_trader.model.book import py_should_handle_own_book_order
from nautilus_trader.model.enums import OrderSide
from nautilus_trader.model.enums import OrderStatus
from nautilus_trader.model.enums import OrderType
from nautilus_trader.model.enums import TimeInForce
from nautilus_trader.model.enums import TriggerType
from nautilus_trader.model.enums import trailing_offset_type_to_str
from nautilus_trader.model.enums import trigger_type_to_str
from nautilus_trader.model.events import OrderEvent
from nautilus_trader.model.events import OrderFilled
from nautilus_trader.model.events import OrderInitialized
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import PositionId
from nautilus_trader.model.identifiers import StrategyId
from nautilus_trader.model.identifiers import TradeId
from nautilus_trader.model.identifiers import VenueOrderId
from nautilus_trader.model.instruments import CurrencyPair
from nautilus_trader.model.instruments import Instrument
from nautilus_trader.model.objects import Price
from nautilus_trader.model.objects import Quantity
from nautilus_trader.model.orders import Order
from nautilus_trader.model.orders import OrderUnpacker
from nautilus_trader.model.position import Position


class LiveExecutionEngine(ExecutionEngine):
    """
    提供高性能的异步实盘执行引擎。

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
    config : LiveExecEngineConfig, 可选
        实例的配置。

    异常
    ------
    TypeError
        如果 `config` 的类型不是 `LiveExecEngineConfig`。

    """

    _sentinel: Final[None] = None

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        config: LiveExecEngineConfig | None = None,
    ) -> None:
        if config is None:
            config = LiveExecEngineConfig()
        PyCondition.type(config, LiveExecEngineConfig, "config")
        super().__init__(
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            config=config,
        )

        self._loop: asyncio.AbstractEventLoop = loop
        self._cmd_queue: asyncio.Queue = Queue(maxsize=config.qsize)
        self._evt_queue: asyncio.Queue = Queue(maxsize=config.qsize)

        # 对账
        self._recon_check_retries: Counter[ClientOrderId] = Counter()
        self._ts_last_query: dict[ClientOrderId, int] = {}
        self._order_local_activity_ns: dict[ClientOrderId, int] = {}
        self._position_local_activity_ns: dict[InstrumentId, int] = {}
        self._recent_fills_cache: dict[TradeId, int] = {}  # TradeId -> timestamp_ns (TTL 缓存)
        self._inferred_fill_ts: dict[ClientOrderId, int] = {}
        self._fill_application_audit: dict[ClientOrderId, list[tuple[TradeId, str, int]]] = {}
        self._startup_reconciliation_event: asyncio.Event = asyncio.Event()
        self._filtered_external_orders_count: int = 0

        self._cmd_enqueuer: ThrottledEnqueuer[Command] = ThrottledEnqueuer(
            qname="cmd_queue",
            queue=self._cmd_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )
        self._evt_enqueuer: ThrottledEnqueuer[OrderEvent] = ThrottledEnqueuer(
            qname="evt_queue",
            queue=self._evt_queue,
            loop=self._loop,
            clock=self._clock,
            logger=self._log,
        )

        # 异步任务
        self._cmd_queue_task: asyncio.Task | None = None
        self._evt_queue_task: asyncio.Task | None = None
        self._reconciliation_task: asyncio.Task | None = None
        self._own_books_audit_task: asyncio.Task | None = None
        self._purge_closed_orders_task: asyncio.Task | None = None
        self._purge_closed_positions_task: asyncio.Task | None = None
        self._purge_account_events_task: asyncio.Task | None = None
        self._is_shutting_down: bool = False
        self._kill: bool = False

        # 配置
        self._reconciliation: bool = config.reconciliation
        self.reconciliation_lookback_mins: int = config.reconciliation_lookback_mins or 0
        self.reconciliation_instrument_ids: list[InstrumentId] = (
            config.reconciliation_instrument_ids or []
        )
        self.filter_unclaimed_external_orders: bool = config.filter_unclaimed_external_orders
        self.filter_position_reports: bool = config.filter_position_reports
        self.filtered_client_order_ids: list[ClientOrderId] = config.filtered_client_order_ids or []
        self.generate_missing_orders: bool = config.generate_missing_orders
        self.inflight_check_interval_ms: int = config.inflight_check_interval_ms
        self.inflight_check_threshold_ms: int = config.inflight_check_threshold_ms
        self.inflight_check_max_retries: int = config.inflight_check_retries
        self.own_books_audit_interval_secs: float | None = config.own_books_audit_interval_secs
        self.open_check_interval_secs: float | None = config.open_check_interval_secs
        self.open_check_open_only: bool = config.open_check_open_only
        self.open_check_lookback_mins: int = config.open_check_lookback_mins
        self.open_check_threshold_ms: int = config.open_check_threshold_ms
        self.open_check_missing_retries: int = config.open_check_missing_retries
        self.max_single_order_queries_per_cycle: int = config.max_single_order_queries_per_cycle
        self.single_order_query_delay_ms: int = config.single_order_query_delay_ms
        self.position_check_interval_secs: float | None = config.position_check_interval_secs
        self.position_check_lookback_mins: int = config.position_check_lookback_mins
        self.position_check_threshold_ms: int = config.position_check_threshold_ms
        self.reconciliation_startup_delay_secs: float = config.reconciliation_startup_delay_secs
        self.purge_closed_orders_interval_mins = config.purge_closed_orders_interval_mins
        self.purge_closed_orders_buffer_mins = config.purge_closed_orders_buffer_mins
        self.purge_closed_positions_interval_mins = config.purge_closed_positions_interval_mins
        self.purge_closed_positions_buffer_mins = config.purge_closed_positions_buffer_mins
        self.purge_account_events_interval_mins = config.purge_account_events_interval_mins
        self.purge_account_events_lookback_mins = config.purge_account_events_lookback_mins
        self.purge_from_database = config.purge_from_database
        self.graceful_shutdown_on_exception: bool = config.graceful_shutdown_on_exception

        self._log.info(f"{config.reconciliation=}", LogColor.BLUE)
        self._log.info(f"{config.reconciliation_lookback_mins=}", LogColor.BLUE)
        self._log.info(f"{config.reconciliation_instrument_ids=}", LogColor.BLUE)
        self._log.info(f"{config.filter_unclaimed_external_orders=}", LogColor.BLUE)
        self._log.info(f"{config.filter_position_reports=}", LogColor.BLUE)
        self._log.info(f"{config.filtered_client_order_ids=}", LogColor.BLUE)
        self._log.info(f"{config.inflight_check_interval_ms=}", LogColor.BLUE)
        self._log.info(f"{config.inflight_check_threshold_ms=}", LogColor.BLUE)
        self._log.info(f"{config.inflight_check_retries=}", LogColor.BLUE)
        self._log.info(f"{config.own_books_audit_interval_secs=}", LogColor.BLUE)
        self._log.info(f"{config.open_check_interval_secs=}", LogColor.BLUE)
        self._log.info(f"{config.open_check_open_only=}", LogColor.BLUE)
        self._log.info(f"{config.open_check_lookback_mins=}", LogColor.BLUE)
        self._log.info(f"{config.open_check_threshold_ms=}", LogColor.BLUE)
        self._log.info(f"{config.open_check_missing_retries=}", LogColor.BLUE)
        self._log.info(f"{config.max_single_order_queries_per_cycle=}", LogColor.BLUE)
        self._log.info(f"{config.single_order_query_delay_ms=}", LogColor.BLUE)
        self._log.info(f"{config.position_check_interval_secs=}", LogColor.BLUE)
        self._log.info(f"{config.position_check_lookback_mins=}", LogColor.BLUE)
        self._log.info(f"{config.position_check_threshold_ms=}", LogColor.BLUE)
        self._log.info(f"{config.reconciliation_startup_delay_secs=}", LogColor.BLUE)
        self._log.info(f"{config.purge_closed_orders_interval_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_closed_orders_buffer_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_closed_positions_interval_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_closed_positions_buffer_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_account_events_interval_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_account_events_lookback_mins=}", LogColor.BLUE)
        self._log.info(f"{config.purge_from_database=}", LogColor.BLUE)
        self._log.info(f"{config.graceful_shutdown_on_exception=}", LogColor.BLUE)

        self._inflight_check_threshold_ns: int = millis_to_nanos(self.inflight_check_threshold_ms)
        self._open_check_threshold_ns: int = millis_to_nanos(self.open_check_threshold_ms)
        self._position_check_threshold_ns: int = millis_to_nanos(self.position_check_threshold_ms)

        # Register endpoints
        self._msgbus.register(
            endpoint="ExecEngine.reconcile_execution_report",
            handler=self.reconcile_execution_report,
        )
        self._msgbus.register(
            endpoint="ExecEngine.reconcile_execution_mass_status",
            handler=self.reconcile_execution_mass_status,
        )

    @property
    def reconciliation(self) -> bool:
        """
        返回对账过程是否将在启动时运行。

        返回
        -------
        bool

        """
        return self._reconciliation

    # -- LIFECYCLE ---------------------------------------------------------------------------------

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

    def get_evt_queue_task(self) -> asyncio.Task | None:
        """
        返回引擎内部事件队列的任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._evt_queue_task

    def get_own_books_audit_task(self) -> asyncio.Task | None:
        """
        返回引擎的自有订单簿审计任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._own_books_audit_task

    def get_reconciliation_task(self) -> asyncio.Task | None:
        """
        返回引擎的持续对账任务。

        返回
        -------
        asyncio.Task 或 ``None``

        """
        return self._reconciliation_task

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

    def _on_start(self) -> None:
        if not self._loop.is_running():
            self._log.warning("启动时循环（loop）未运行")

        # 清除对账事件以开始新的启动周期
        self._startup_reconciliation_event.clear()
        self._is_shutting_down = False

        self._cmd_queue_task = self._loop.create_task(self._run_cmd_queue(), name="cmd_queue")
        self._evt_queue_task = self._loop.create_task(self._run_evt_queue(), name="evt_queue")
        self._log.debug(f"已调度任务 '{self._cmd_queue_task.get_name()}'")
        self._log.debug(f"已调度任务 '{self._evt_queue_task.get_name()}'")

        # 如果配置了任何检查，则启动对账任务
        if (
            self.inflight_check_interval_ms
            or self.open_check_interval_secs
            or self.position_check_interval_secs
        ) and not self._reconciliation_task:
            self._reconciliation_task = self._loop.create_task(
                self._continuous_reconciliation_loop(),
                name="continuous_reconciliation",
            )
            self._log.debug(f"已调度任务 '{self._reconciliation_task.get_name()}'")
            self._log.info("已启动对账任务", LogColor.BLUE)

        if self.own_books_audit_interval_secs and not self._own_books_audit_task:
            self._own_books_audit_task = self._loop.create_task(
                self._own_books_audit_loop(self.own_books_audit_interval_secs),
                name="own_books_audit",
            )

        if self.purge_closed_orders_interval_mins and not self._purge_closed_orders_task:
            self._purge_closed_orders_task = self._loop.create_task(
                self._purge_closed_orders_loop(self.purge_closed_orders_interval_mins),
                name="purge_closed_orders",
            )

        if self.purge_closed_positions_interval_mins and not self._purge_closed_positions_task:
            self._purge_closed_positions_task = self._loop.create_task(
                self._purge_closed_positions_loop(self.purge_closed_positions_interval_mins),
                name="purge_closed_positions",
            )

        if self.purge_account_events_interval_mins and not self._purge_account_events_task:
            self._purge_account_events_task = self._loop.create_task(
                self._purge_account_events_loop(self.purge_account_events_interval_mins),
                name="purge_account_events",
            )

    async def _purge_closed_positions_loop(self, interval_mins: int) -> None:
        interval_secs = interval_mins * 60
        buffer_mins = self.purge_closed_positions_buffer_mins or 0
        buffer_secs = buffer_mins * 60

        try:
            while True:
                await asyncio.sleep(interval_secs)
                ts_now = self._clock.timestamp_ns()
                self._cache.purge_closed_positions(
                    ts_now=ts_now,
                    buffer_secs=buffer_secs,
                    purge_from_database=self.purge_from_database,
                )
        except asyncio.CancelledError:
            self._log.debug("任务 'purge_closed_positions' 已取消")
        except Exception as e:
            self._log.exception("清除已关闭持仓时出错", e)

    async def _purge_closed_orders_loop(self, interval_mins: int) -> None:
        interval_secs = interval_mins * 60
        buffer_mins = self.purge_closed_orders_buffer_mins or 0
        buffer_secs = buffer_mins * 60

        try:
            while True:
                await asyncio.sleep(interval_secs)
                ts_now = self._clock.timestamp_ns()
                self._cache.purge_closed_orders(
                    ts_now=ts_now,
                    buffer_secs=buffer_secs,
                    purge_from_database=self.purge_from_database,
                )
        except asyncio.CancelledError:
            self._log.debug("任务 'purge_closed_orders' 已取消")
        except Exception as e:
            self._log.exception("清除已关闭订单时出错", e)

    def _on_stop(self) -> None:
        self._is_shutting_down = True

        if self._reconciliation_task:
            self._log.debug(f"正在取消任务 '{self._reconciliation_task.get_name()}'")
            self._reconciliation_task.cancel()
            self._reconciliation_task = None

        if self._own_books_audit_task:
            self._log.debug(f"正在取消任务 '{self._own_books_audit_task.get_name()}'")
            self._own_books_audit_task.cancel()
            self._own_books_audit_task = None

        if self._purge_closed_orders_task:
            self._log.debug(f"正在取消任务 '{self._purge_closed_orders_task.get_name()}'")
            self._purge_closed_orders_task.cancel()
            self._purge_closed_orders_task = None

        if self._filtered_external_orders_count > 0:
            self._log.info(
                f"运行期间过滤了 {self._filtered_external_orders_count:,} 个未认领的外部（EXTERNAL）订单",
                LogColor.BLUE,
            )

        if self._purge_closed_positions_task:
            self._log.debug(f"正在取消任务 '{self._purge_closed_positions_task.get_name()}'")
            self._purge_closed_positions_task.cancel()
            self._purge_closed_positions_task = None

        if self._purge_account_events_task:
            self._log.debug(f"正在取消任务 '{self._purge_account_events_task.get_name()}'")
            self._purge_account_events_task.cancel()
            self._purge_account_events_task = None

        if self._kill:
            return  # 避免排队冗余的哨兵消息

        # 这将在队列看到哨兵消息时停止队列处理
        self._enqueue_sentinel()

    def _enqueue_sentinel(self) -> None:
        # 信号通知队列停止处理
        self._loop.call_soon_threadsafe(self._cmd_queue.put_nowait, self._sentinel)
        self._loop.call_soon_threadsafe(self._evt_queue.put_nowait, self._sentinel)
        self._log.debug("哨兵消息已放入队列")

    # -- 命令 ---------------------------------------------------------------------------------------

    def kill(self) -> None:
        """
        通过强行取消队列任务并调用 stop 来停止引擎。
        """
        self._log.warning("正在停止引擎（Kill）")
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

        如果内部队列已满，则记录警告并阻塞，直到队列大小减小。

        参数
        ----------
        command : Command
            要执行的命令。

        """
        self._cmd_enqueuer.enqueue(command)

    def process(self, event: OrderEvent) -> None:
        """
        处理给定的事件消息。

        如果内部队列已满或接近满，它将记录一条警告（限流）并调度异步 `put()` 操作。
        这确保了所有消息最终都会排入队列并得到处理，而不会在队列已满时阻塞调用者。

        参数
        ----------
        event : OrderEvent
            要处理的事件。

        """
        self._record_local_activity(event)
        self._evt_enqueuer.enqueue(event)

    # -- 队列处理 -----------------------------------------------------------------------------------

    async def _run_cmd_queue(self) -> None:
        self._log.debug(
            f"Command 消息队列处理开始 (qsize={self.cmd_qsize()})",
        )
        try:
            while True:
                try:
                    command: Command | None = await self._cmd_queue.get()
                    if command is self._sentinel:
                        break

                    self._execute_command(command)
                except asyncio.CancelledError:
                    self._log.warning("任务 'run_cmd_queue' 已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "command")
        finally:
            stopped_msg = "Command 消息队列已停止"

            if not self._cmd_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.cmd_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    async def _run_evt_queue(self) -> None:
        self._log.debug(
            f"Event 消息队列处理开始 (qsize={self.evt_qsize()})",
        )
        try:
            while True:
                try:
                    event: OrderEvent | None = await self._evt_queue.get()
                    if event is self._sentinel:
                        break

                    self._handle_event_with_tracking(event)
                except asyncio.CancelledError:
                    self._log.warning("任务 'run_evt_queue' 已取消")
                    break
                except Exception as e:
                    self._handle_queue_exception(e, "event")
        finally:
            stopped_msg = "Event 消息队列已停止"

            if not self._evt_queue.empty():
                self._log.warning(f"{stopped_msg}，队列中仍有 {self.evt_qsize()} 条消息")
            else:
                self._log.debug(stopped_msg)

    def _handle_queue_exception(self, e: Exception, queue_name: str) -> None:
        self._log.exception(
            f"{queue_name} 队列处理中出现意外异常: {e!r}",
            e,
        )

        if self.graceful_shutdown_on_exception:
            if not self._is_shutting_down:
                self._log.warning(
                    "由于意外异常，正在启动优雅停机",
                )
                self.shutdown_system(
                    f"{queue_name} 队列处理中出现意外异常: {e!r}",
                )
                self._is_shutting_down = True
        else:
            self._log.error(
                "系统将立即终止，以防止在降级状态下运行",
            )
            os._exit(1)  # 立即崩溃

    # -- 持续监控 -----------------------------------------------------------------------------------

    async def _own_books_audit_loop(self, interval_secs: float) -> None:
        try:
            while True:
                await asyncio.sleep(interval_secs)
                self._cache.audit_own_order_books()
        except asyncio.CancelledError:
            self._log.debug("任务 'own_books_audit' 已取消")
        except Exception as e:
            self._log.exception("审计自有订单簿时出错", e)

    # ruff: noqa: C901
    async def _continuous_reconciliation_loop(self) -> None:
        try:
            # 将间隔转换为纳秒（处理 None 值）
            inflight_check_interval_ns = (
                millis_to_nanos(self.inflight_check_interval_ms)
                if self.inflight_check_interval_ms > 0
                else 0
            )
            consistency_check_interval_ns = (
                secs_to_nanos(self.open_check_interval_secs) if self.open_check_interval_secs else 0
            )
            position_check_interval_ns = (
                secs_to_nanos(self.position_check_interval_secs)
                if self.position_check_interval_secs
                else 0
            )
            cache_prune_interval_ns = secs_to_nanos(60.0)

            # 确定最小休眠间隔（秒）
            intervals_secs: list[float] = []

            if self.inflight_check_interval_ms > 0:
                intervals_secs.append(self.inflight_check_interval_ms / 1000)

            if self.open_check_interval_secs:
                intervals_secs.append(self.open_check_interval_secs)

            if self.position_check_interval_secs:
                intervals_secs.append(self.position_check_interval_secs)

            min_interval_secs = min(intervals_secs) if intervals_secs else 1.0

            self._log.info(
                f"启动持续对账，间隔设置如下： "
                f"inflight={self.inflight_check_interval_ms}ms, "
                f"consistency={self.open_check_interval_secs}s, "
                f"position={self.position_check_interval_secs}s",
                LogColor.BLUE,
            )

            # 仅在启用对账时才等待（否则事件永远不会设置）
            if self.reconciliation:
                self._log.info(
                    "在开始持续检查前，等待启动对账完成",
                    LogColor.BLUE,
                )
                await self._startup_reconciliation_event.wait()
                self._log.info("启动对账已完成", LogColor.GREEN)

                # 在对账完成后应用额外的启动延迟
                if self.reconciliation_startup_delay_secs > 0:
                    self._log.info(
                        f"应用对账后的启动延迟 "
                        f"({self.reconciliation_startup_delay_secs}秒)",
                        LogColor.BLUE,
                    )
                    await asyncio.sleep(self.reconciliation_startup_delay_secs)
            else:
                self._log.info(
                    "启动对账已禁用，继续执行持续检查",
                    LogColor.BLUE,
                )

            # 将时间戳初始化为当前时间，以便第一次检查等待完整间隔，
            # 给执行客户端时间完成其连接初始化
            ts_now_init = self._clock.timestamp_ns()
            ts_last_inflight_check = ts_now_init
            ts_last_consistency_check = ts_now_init
            ts_last_position_check = ts_now_init
            ts_last_cache_prune = ts_now_init

            while True:
                if self._is_shutting_down:
                    self._log.debug("由于停止信号，对账循环退出")
                    break

                ts_now = self._clock.timestamp_ns()

                # 检查在途（In-flight）订单
                if (
                    inflight_check_interval_ns > 0
                    and ts_now - ts_last_inflight_check >= inflight_check_interval_ns
                ):
                    # 在开始检查前确认停止信号
                    if self._is_shutting_down:
                        break
                    try:
                        await self._check_inflight_orders()
                        ts_last_inflight_check = ts_now
                    except Exception as e:
                        self._log.exception("check_inflight_orders 失败", e)

                # 检查开仓订单的一致性
                if (
                    consistency_check_interval_ns > 0
                    and ts_now - ts_last_consistency_check >= consistency_check_interval_ns
                ):
                    # 在开始检查前确认停止信号
                    if self._is_shutting_down:
                        break
                    try:
                        await self._check_orders_consistency()
                        ts_last_consistency_check = ts_now
                    except Exception as e:
                        self._log.exception("check_orders_consistency 失败", e)

                # 检查持仓一致性
                if (
                    position_check_interval_ns > 0
                    and ts_now - ts_last_position_check >= position_check_interval_ns
                ):
                    # 在开始检查前确认停止信号
                    if self._is_shutting_down:
                        break
                    try:
                        await self._check_positions_consistency()
                        ts_last_position_check = ts_now
                    except Exception as e:
                        self._log.exception("check_positions_consistency 失败", e)

                if ts_now - ts_last_cache_prune >= cache_prune_interval_ns:
                    try:
                        self._prune_recent_fills_cache()
                        ts_last_cache_prune = ts_now
                    except Exception as e:
                        self._log.exception("prune_recent_fills_cache 失败", e)

                await asyncio.sleep(min_interval_secs)
        except asyncio.CancelledError:
            self._log.debug("任务 'continuous_reconciliation' 已取消")

    async def _check_inflight_orders(self) -> None:
        if self._is_shutting_down:
            self._log.debug("由于停止信号，跳过在途（In-flight）订单检查")
            return

        self._log.debug("正在检查在途订单状态")

        delayed_orders: list[Order] = []
        inflight_orders: list[Order] = self._cache.orders_inflight()

        ts_now = self._clock.timestamp_ns()

        for order in inflight_orders:
            if ts_now > order.last_event.ts_event + self._inflight_check_threshold_ns:
                delayed_orders.append(order)

        if delayed_orders:
            self._log.debug(
                f"检测到 {len(delayed_orders)} 个延迟的在途订单",
            )

        # 查询并可能解决每个不一致的订单
        for order in delayed_orders:
            if not order.is_inflight:
                self._clear_recon_tracking(order.client_order_id, drop_last_query=False)
                continue

            last_query_ts = self._ts_last_query.get(order.client_order_id)
            if last_query_ts and ts_now - last_query_ts < self._inflight_check_threshold_ns:
                self._log.debug(
                    f"跳过 {order.client_order_id!r} 的重新查询 - 正在等待之前的响应",
                )
                continue

            retries = self._recon_check_retries[order.client_order_id]
            if retries >= self.inflight_check_max_retries:
                backlog = self.evt_qsize()
                if backlog > 0:
                    self._log.debug(
                        f"推迟 {order.client_order_id!r} 的在途对账 - 事件队列积压 {backlog}",
                    )
                    continue

                self._log.warning(
                    f"订单 {order.client_order_id!r} 超过最大在途重试次数 ({retries})，"
                    f"将其解析为失败状态",
                    LogColor.YELLOW,
                )
                self._resolve_inflight_order(order)
            else:
                self._log.debug(f"正在向柜台查询 {order}...")
                query_ts = self._clock.timestamp_ns()
                query = QueryOrder(
                    trader_id=order.trader_id,
                    strategy_id=order.strategy_id,
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    command_id=UUID4(),
                    ts_init=query_ts,
                )
                self._execute_command(query)
                self._ts_last_query[order.client_order_id] = query_ts
                self._recon_check_retries[order.client_order_id] = retries + 1

    def _resolve_inflight_order(self, order: Order) -> None:
        if not order.is_inflight:
            self._log.debug(
                f"跳过 {order.client_order_id!r} 的在途对账 - 当前状态为 {order.status_string()}",
            )
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            return

        ts_now = self._clock.timestamp_ns()

        if order.status == OrderStatus.SUBMITTED:
            account_id = order.account_id or self._cache.account_id(order.instrument_id.venue)
            if account_id is None:
                account = self._cache.account_for_venue(venue=order.instrument_id.venue)
                if account is not None:
                    account_id = account.id
            if account_id is None:
                accounts = self._cache.accounts()
                if accounts and len(accounts) == 1:
                    account_id = accounts[0].id
            if account_id is None:
                self._log.warning(
                    f"无法为 {order.client_order_id!r} 解析 account_id，跳过 REJECTED 事件",
                    LogColor.YELLOW,
                )
                self._clear_recon_tracking(order.client_order_id)
                self._order_local_activity_ns.pop(order.client_order_id, None)
                return
            rejected = create_order_rejected_event(
                order=order,
                ts_now=ts_now,
                reason="UNKNOWN",
                account_id=account_id,
            )
            self._log.debug(f"生成了 {rejected}")
            self._handle_event_with_tracking(rejected)
        elif order.status in (OrderStatus.PENDING_UPDATE, OrderStatus.PENDING_CANCEL):
            canceled = create_order_canceled_event(
                order=order,
                ts_now=ts_now,
            )
            self._log.debug(f"生成了 {canceled}")
            self._handle_event_with_tracking(canceled)
        else:
            raise RuntimeError(f"在途订单状态无效，当前为 '{order.status_string()}'")

        self._clear_recon_tracking(order.client_order_id)
        self._order_local_activity_ns.pop(order.client_order_id, None)

    async def _check_positions_consistency(self) -> None:
        if self._is_shutting_down:
            self._log.debug("由于停止信号，跳过持仓一致性检查")
            return

        self._log.debug("正在检查缓存状态与柜台之间的持仓一致性")

        open_positions = self._cache.positions_open()

        if self.reconciliation_instrument_ids:
            open_positions = [
                p for p in open_positions if p.instrument_id in self.reconciliation_instrument_ids
            ]

        # 按交易标的 ID 对持仓进行分组（用于净额对账）
        positions_by_instrument: dict[InstrumentId, list[Position]] = {}

        for position in open_positions:
            if position.instrument_id not in positions_by_instrument:
                positions_by_instrument[position.instrument_id] = []

            positions_by_instrument[position.instrument_id].append(position)

        self._log.debug(
            f"发现 {len(positions_by_instrument)} 个具有开仓持仓的唯一交易标的",
        )

        if not self._clients:
            self._log.debug("没有执行客户端可用于检查持仓一致性，提前返回")
            return

        venue_positions = await self._query_position_status_reports()

        await self._process_cached_position_discrepancies(
            positions_by_instrument,
            venue_positions,
        )

        await self._process_venue_reported_positions(
            positions_by_instrument,
            venue_positions,
        )

    async def _query_position_status_reports(self) -> dict[InstrumentId, PositionStatusReport]:
        clients = self._clients.values()

        tasks = [
            c.generate_position_status_reports(
                GeneratePositionStatusReports(
                    instrument_id=None,  # 获取所有持仓
                    start=None,  # 不设时间过滤 - 我们需要所有开仓和已平仓的持仓
                    end=None,
                    command_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                    log_receipt_level=LogLevel.DEBUG,
                ),
            )
            for c in clients
        ]

        try:
            position_reports_all = await asyncio.gather(*tasks, return_exceptions=True)
        except Exception as e:
            self._log.error(f"无法获取持仓状态报告：{e}")
            return {}

        # 建立映射：instrument_id -> venue report
        venue_positions: dict[InstrumentId, PositionStatusReport] = {}
        for reports_or_exception in position_reports_all:
            if isinstance(reports_or_exception, Exception):
                self._log.error(
                    f"无法生成持仓状态报告：{reports_or_exception}",
                )
                continue

            reports = cast("list[PositionStatusReport]", reports_or_exception)
            for report in reports:
                venue_positions[report.instrument_id] = report

        return venue_positions

    async def _process_cached_position_discrepancies(
        self,
        positions_by_instrument: dict[InstrumentId, list[Position]],
        venue_positions: dict[InstrumentId, PositionStatusReport],
    ) -> None:
        clients = self._clients.values()

        for instrument_id, cached_positions in positions_by_instrument.items():
            venue_report = venue_positions.get(instrument_id)

            has_discrepancy = self._check_position_discrepancy(
                cached_positions,
                venue_report,
                instrument_id,
            )

            if not has_discrepancy:
                continue

            last_activity_ts = self._position_local_activity_ns.get(instrument_id)
            if last_activity_ts:
                ts_now = self._clock.timestamp_ns()
                if ts_now - last_activity_ts < self._position_check_threshold_ns:
                    self._log.debug(
                        f"跳过 {instrument_id} 的持仓对账： "
                        f"近期活动在阈值范围内 ({self.position_check_threshold_ms}ms)",
                    )
                    continue

            cached_qty = sum(p.signed_decimal_qty() for p in cached_positions)
            venue_qty = venue_report.signed_decimal_qty if venue_report else Decimal(0)

            self._log.warning(
                f"检测到 {instrument_id} 的持仓不一致： "
                f"cached_qty={cached_qty}, venue_qty={venue_qty}；正在查询缺失的成交...",
                LogColor.YELLOW,
            )

            missing_fills = await self._query_and_find_missing_fills(instrument_id, clients)
            await self._reconcile_missing_fills(missing_fills, instrument_id)

            if not missing_fills and has_discrepancy:
                self._log.warning(
                    f"{instrument_id} 的持仓不一致仍然存在，但未发现缺失的成交。 "
                    f"可能原因：成交超出回溯窗口 ({self.position_check_lookback_mins}分钟)， "
                    f"柜台持仓错误，或内部计算错误。",
                    LogColor.YELLOW,
                )

    def _check_position_discrepancy(
        self,
        cached_positions: list[Position],
        venue_report: PositionStatusReport | None,
        instrument_id: InstrumentId,
    ) -> bool:
        # 计算缓存的持仓数量
        cached_qty = Decimal(0)
        for position in cached_positions:
            cached_qty += position.signed_decimal_qty()

        # 处理场地没有持仓报告的情况
        if venue_report is None:
            # 我们认为有持仓，但场地显示为平仓（或无报告）
            if cached_qty != 0:
                instrument = self._cache.instrument(instrument_id)
                if instrument is not None:
                    if is_within_single_unit_tolerance(
                        cached_qty,
                        Decimal(0),
                        instrument.size_precision,
                    ):
                        return False
                else:
                    self._log.debug(
                        f"无法对 {instrument_id} 应用容差检查：缓存中未找到该标的",
                    )

                self._log.warning(
                    f"{instrument_id} 的持仓不一致： "
                    f"cached_qty={cached_qty}，柜台无持仓报告",
                    LogColor.YELLOW,
                )
                return True
            # 两者都平仓 - 无差异
            return False

        # 检查数量是否匹配（两者都可能为零）
        venue_qty = venue_report.signed_decimal_qty
        if cached_qty == venue_qty:
            return False

        instrument = self._cache.instrument(instrument_id)
        if instrument is not None:
            if is_within_single_unit_tolerance(
                cached_qty,
                venue_qty,
                instrument.size_precision,
            ):
                return False
        else:
            self._log.debug(
                f"无法对 {instrument_id} 应用容差检查：缓存中未找到该标的",
            )

        return True

    async def _process_venue_reported_positions(
        self,
        positions_by_instrument: dict[InstrumentId, list[Position]],
        venue_positions: dict[InstrumentId, PositionStatusReport],
    ) -> None:
        clients = self._clients.values()

        for instrument_id, venue_report in venue_positions.items():
            if instrument_id in positions_by_instrument:
                continue  # Already checked above

            # Apply instrument filter
            if (
                self.reconciliation_instrument_ids
                and instrument_id not in self.reconciliation_instrument_ids
            ):
                continue

            # 场地有持仓但我们没有 - 这是一个差异
            if venue_report.signed_decimal_qty == 0:
                continue  # 两者均平仓，无差异

            # 阈值检查
            last_activity_ts = self._position_local_activity_ns.get(instrument_id)
            if last_activity_ts:
                ts_now = self._clock.timestamp_ns()
                if ts_now - last_activity_ts < self._position_check_threshold_ns:
                    self._log.debug(
                        f"跳过 {instrument_id} 的持仓对账： "
                        f"近期活动在阈值内 ({self.position_check_threshold_ms}ms)",
                    )
                    continue

            self._log.warning(
                f"检测到 {instrument_id} 的持仓不一致： "
                f"cached_qty=0 (无持仓), venue_qty={venue_report.signed_decimal_qty}。正在查询缺失的成交...",
                LogColor.YELLOW,
            )

            missing_fills = await self._query_and_find_missing_fills(instrument_id, clients)
            await self._reconcile_missing_fills(missing_fills, instrument_id)

            if not missing_fills:
                self._log.warning(
                    f"{instrument_id} 的持仓不一致仍然存在，但未发现缺失的成交。 "
                    f"可能原因：成交超出回溯窗口 ({self.position_check_lookback_mins}分钟)， "
                    f"柜台持仓错误，或内部计算错误。",
                    LogColor.YELLOW,
                )

    async def _query_and_find_missing_fills(
        self,
        instrument_id: InstrumentId,
        clients: Iterable[ExecutionClient],
    ) -> list[FillReport]:
        fill_lookback_start = self._clock.utc_now() - pd.Timedelta(
            minutes=self.position_check_lookback_mins,
        )

        fill_tasks = [
            c.generate_fill_reports(
                GenerateFillReports(
                    instrument_id=instrument_id,
                    venue_order_id=None,
                    start=fill_lookback_start,
                    end=None,
                    command_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                ),
            )
            for c in clients
        ]

        fill_reports_all = await asyncio.gather(*fill_tasks, return_exceptions=True)

        venue_fills: list[FillReport] = []
        for fills_or_exception in fill_reports_all:
            if isinstance(fills_or_exception, Exception):
                self._log.error(
                    f"无法为 {instrument_id} 生成成交报告：{fills_or_exception}",
                )
                continue

            fills = cast("list[FillReport]", fills_or_exception)
            venue_fills.extend(fills)

        cached_fill_trade_ids: set[TradeId] = set()
        for order in self._cache.orders(instrument_id=instrument_id):
            for event in order.events:
                if isinstance(event, OrderFilled):
                    cached_fill_trade_ids.add(event.trade_id)

        # 查找缺失的成交（不在缓存中，也不在最近成交缓存中）
        missing_fills = [
            fill
            for fill in venue_fills
            if fill.trade_id not in cached_fill_trade_ids
            and fill.trade_id not in self._recent_fills_cache
        ]

        return missing_fills

    async def _reconcile_missing_fills(
        self,
        missing_fills: list[FillReport],
        instrument_id: InstrumentId,
    ) -> None:
        if not missing_fills:
            return

        self._log.warning(
            f"发现 {instrument_id} 有 {len(missing_fills)} 个缺失的成交",
            LogColor.YELLOW,
        )

        for fill_report in missing_fills:
            try:
                result = self._reconcile_fill_report_single(fill_report)
                if not result:
                    await self._try_reconcile_order_for_fill(fill_report)
                    result = self._reconcile_fill_report_single(fill_report)
                if result:
                    self._position_local_activity_ns[instrument_id] = self._clock.timestamp_ns()
                else:
                    self._log.warning(
                        f"无法对账 {instrument_id} 的成交 {fill_report.trade_id}： "
                        f"订单尚未缓存或缺少其他前提条件。 "
                        f"成交将在下一个持仓检查周期中重试。",
                        LogColor.YELLOW,
                    )
            except Exception as e:
                self._log.error(
                    f"对账 {instrument_id} 缺失的成交 {fill_report.trade_id} 时出现异常：{e}",
                )

    async def _try_reconcile_order_for_fill(self, report: FillReport) -> bool:
        if not self._clients:
            return False

        if report.client_order_id is None and report.venue_order_id is None:
            return False

        candidate_client_order_id = report.client_order_id
        if candidate_client_order_id is None and report.venue_order_id is not None:
            candidate_client_order_id = self._cache.client_order_id(report.venue_order_id)

        if (
            candidate_client_order_id is not None
            and self._cache.order(candidate_client_order_id) is not None
        ):
            return True

        query_ts = self._clock.timestamp_ns()
        tasks = [
            c.generate_order_status_report(
                GenerateOrderStatusReport(
                    instrument_id=report.instrument_id,
                    client_order_id=report.client_order_id,
                    venue_order_id=report.venue_order_id,
                    command_id=UUID4(),
                    ts_init=query_ts,
                ),
            )
            for c in self._clients.values()
        ]

        results = await asyncio.gather(*tasks, return_exceptions=True)
        for result in results:
            if isinstance(result, Exception):
                self._log.warning(
                    f"通过成交报告查询订单状态时出错：{result}",
                )
                continue

            order_report = cast("OrderStatusReport | None", result)
            if order_report is None:
                continue

            self._log.info(
                f"通过成交报告查询找到订单 {order_report.client_order_id!r}",
                LogColor.BLUE,
            )
            self._reconcile_order_report(order_report, trades=[])
            return True

        return False

    def _prune_recent_fills_cache(self, ttl_secs: float = 60.0) -> None:
        # 从缓存中移除过期的成交（默认 TTL：60 秒）
        ts_now = self._clock.timestamp_ns()
        ttl_ns = secs_to_nanos(ttl_secs)
        expired_trade_ids = [
            trade_id
            for trade_id, ts_cached in self._recent_fills_cache.items()
            if ts_now - ts_cached > ttl_ns
        ]
        for trade_id in expired_trade_ids:
            self._recent_fills_cache.pop(trade_id, None)

    async def _check_orders_consistency(self) -> None:
        try:
            if self._is_shutting_down:
                self._log.debug("由于停止信号，跳过订单一致性检查")
                return

            self._log.debug("正在检查缓存状态与柜台之间的订单一致性")

            open_order_ids: set[ClientOrderId] = self._cache.client_order_ids_open()
            open_orders: list[Order] = self._cache.orders_open()

            if self.reconciliation_instrument_ids:
                open_orders = [
                    o for o in open_orders if o.instrument_id in self.reconciliation_instrument_ids
                ]
                open_order_ids = {o.client_order_id for o in open_orders}

            open_len = len(open_orders)
            self._log.debug(f"发现缓存中有 {open_len} 个订单处于开仓状态")

            if not self._clients:
                self._log.debug("没有执行客户端可用于检查订单一致性，提前返回")
                return

            all_order_reports, venue_reported_ids = await self._query_order_status_reports()

            self._reconcile_order_reports(all_order_reports, open_order_ids)

            if self.open_check_open_only:
                missing_orders = open_order_ids - venue_reported_ids
                if missing_orders:
                    self._log.debug(
                        f"{len(missing_orders)} 个缓存中的开仓订单不在柜台当前的响应中 - "
                        f"可能已于近期成交/取消（柜台在查询开仓订单时可能包含近期已关闭订单）：",
                    )

                    for order_id in missing_orders:
                        self._log.debug(f"- {order_id}")

                return  # Can't reliably resolve missing orders in open_only mode

            await self._handle_missing_orders_at_venue(open_order_ids, venue_reported_ids)

            self._validate_open_orders_consistency()
        except Exception as e:
            self._log.exception("check_order_consistency 出现错误", e)

    def _validate_open_orders_consistency(self) -> None:
        for order in self._cache.orders_open():
            computed_filled = sum(e.last_qty for e in order.events if isinstance(e, OrderFilled))
            if computed_filled != order.filled_qty:
                self._log.error(
                    f"不一致：{order.client_order_id} "
                    f"计算的成交值={computed_filled} 与 缓存的成交值={order.filled_qty} 不符",
                )

    async def _handle_missing_orders_at_venue(
        self,
        open_order_ids: set[ClientOrderId],
        venue_reported_ids: set[ClientOrderId],
    ) -> None:
        missing_at_venue: set[ClientOrderId] = open_order_ids - venue_reported_ids
        ts_now = self._clock.timestamp_ns()

        targeted_queries_count = 0
        logged_limit_warning = False

        for client_order_id in missing_at_venue:
            order = self._cache.order(client_order_id)
            if order is None:
                self._log.error(f"柜台缺失 {client_order_id!r} 且缓存中未找到")
                continue

            # Check if order is too recent to reconcile (avoid race conditions)
            ts_last = order.ts_last
            if (ts_now - ts_last) < self._open_check_threshold_ns:
                self._log.debug(
                    f"跳过 {client_order_id!r} 的对账 - 订单过新 "
                    f"(时长={(ts_now - ts_last) / 1_000_000:.0f}ms < 阈值={self.open_check_threshold_ms}ms)",
                )
                continue

            local_activity = self._order_local_activity_ns.get(client_order_id)
            if local_activity and (ts_now - local_activity) < self._open_check_threshold_ns:
                self._log.debug(
                    f"跳过 {client_order_id!r} 的对账；"
                    f"待处理的本地活动 ({(ts_now - local_activity) / 1_000_000:.0f}ms < 阈值={self.open_check_threshold_ms}ms)",
                )
                continue

            retries = self._recon_check_retries.get(client_order_id, 0)
            if retries >= self.open_check_missing_retries:
                if targeted_queries_count >= self.max_single_order_queries_per_cycle:
                    self._recon_check_retries[client_order_id] = retries + 1

                    if not logged_limit_warning:
                        # Count how many orders at threshold are being deferred
                        orders_at_threshold_remaining = (
                            sum(
                                1
                                for cid in missing_at_venue
                                if self._recon_check_retries.get(cid, 0)
                                >= self.open_check_missing_retries
                            )
                            - targeted_queries_count
                        )
                        self._log.warning(
                            f"Reached max single-order queries ({self.max_single_order_queries_per_cycle}) "
                            f"this cycle, deferring {orders_at_threshold_remaining} order(s) at threshold to next cycle",
                            LogColor.YELLOW,
                        )
                        logged_limit_warning = True

                    continue  # 跳过查询但继续处理其他订单

                self._log.warning(
                    f"重试 {retries} 次后柜台仍未找到订单 {client_order_id!r}，正在执行单笔订单查询",
                    LogColor.YELLOW,
                )
                self._clear_recon_tracking(client_order_id, drop_last_query=False)
                await self._resolve_order_not_found_at_venue(order)
                targeted_queries_count += 1

                # Add delay between single-order queries (skip after final query)
                if (
                    targeted_queries_count < self.max_single_order_queries_per_cycle
                    and self.single_order_query_delay_ms > 0
                ):
                    await asyncio.sleep(self.single_order_query_delay_ms / 1000.0)
            else:
                self._recon_check_retries[client_order_id] = retries + 1
                self._log.debug(
                    f"柜台未找到订单 {client_order_id!r}，重试中 {retries + 1}/{self.open_check_missing_retries}",
                )

    async def _resolve_order_not_found_at_venue(self, order: Order) -> None:
        ts_now = self._clock.timestamp_ns()

        self._log.debug(
            f"在将 {order.client_order_id!r} 标记为 REJECTED 前对其执行单笔订单查询",
            LogColor.BLUE,
        )

        client_id = self._cache.client_id(order.client_order_id)
        if client_id is None:
            self._log.warning(
                f"未找到 {order.client_order_id!r} 的 client_id，跳过针对性查询",
            )
            # Skip targeted query but proceed with resolution
        else:
            client = self._clients.get(client_id)

            try:
                query_ts = self._clock.timestamp_ns()
                command = GenerateOrderStatusReport(
                    instrument_id=order.instrument_id,
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                    command_id=UUID4(),
                    ts_init=query_ts,
                )

                self._ts_last_query[order.client_order_id] = query_ts
                report = await client.generate_order_status_report(command)
                if report is not None:
                    self._log.info(
                        f"通过针对性查询找到 {order.client_order_id!r}：{report.order_status}",
                        LogColor.BLUE,
                    )
                    self._reconcile_order_report(report, trades=[])
                    return  # 订单已找到并对账，无需标记为拒绝
            except Exception as e:
                self._log.warning(f"对 {order.client_order_id!r} 执行针对性查询时出错：{e}")

        if not order.is_open:
            self._log.debug(
                f"跳过 {order.client_order_id!r} 的对账 - 状态已为 {order.status_string()}",
            )
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            return

        if order.status == OrderStatus.ACCEPTED:
            self._log.warning(
                f"正在对账 {order.client_order_id!r}：柜台未找到 ACCEPTED 订单，标记为 REJECTED",
                LogColor.YELLOW,
            )
            account_id = order.account_id or self._cache.account_id(order.instrument_id.venue)
            if account_id is None:
                account = self._cache.account_for_venue(venue=order.instrument_id.venue)
                if account is not None:
                    account_id = account.id
            if account_id is None:
                accounts = self._cache.accounts()
                if accounts and len(accounts) == 1:
                    account_id = accounts[0].id
            if account_id is None:
                self._log.warning(
                    f"无法为 {order.client_order_id!r} 解析 account_id，跳过 REJECTED 事件",
                    LogColor.YELLOW,
                )
                self._clear_recon_tracking(order.client_order_id)
                self._order_local_activity_ns.pop(order.client_order_id, None)
                return
            rejected = create_order_rejected_event(
                order=order,
                ts_now=ts_now,
                reason="ORDER_NOT_FOUND_AT_VENUE",
                account_id=account_id,
            )
            self._handle_event_with_tracking(rejected)
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            return

        if order.status == OrderStatus.PARTIALLY_FILLED:
            self._log.warning(
                f"正在对账 {order.client_order_id!r}：PARTIALLY_FILLED "
                f"订单未在柜台找到，标记为 CANCELED（保留 {order.filled_qty} 成交数量）",
                LogColor.YELLOW,
            )
            canceled = create_order_canceled_event(
                order=order,
                ts_now=ts_now,
            )
            self._handle_event_with_tracking(canceled)
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            return

        if order.is_inflight:
            self._log.debug(
                f"推迟 {order.client_order_id!r} 的对账 - 仍处于在途（In-flight）状态 {order.status_string()}",
            )
            self._clear_recon_tracking(order.client_order_id, drop_last_query=False)
            self._ts_last_query[order.client_order_id] = ts_now
            return

        if order.is_closed:
            if order.status == OrderStatus.FILLED:
                self._log.debug(
                    f"{order.client_order_id!r} 已成交（FILLED）且柜台未找到（符合预期）",
                )
            else:
                self._log.warning(
                    f"订单 {order.client_order_id!r} 已作为 {order.status_string()} 关闭，"
                    "跳过缺失订单对账",
                )
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            return

        self._log.warning(
            f"未在柜台找到的订单 {order.client_order_id!r} 状态异常：{order.status_string()}",
        )
        self._clear_recon_tracking(order.client_order_id)
        self._order_local_activity_ns.pop(order.client_order_id, None)

    async def _query_order_status_reports(
        self,
    ) -> tuple[list[OrderStatusReport], set[ClientOrderId]]:
        order_status_start = self._clock.utc_now() - pd.Timedelta(
            minutes=self.open_check_lookback_mins,
        )

        clients = self._clients.values()

        tasks = [
            c.generate_order_status_reports(
                GenerateOrderStatusReports(
                    instrument_id=None,
                    start=order_status_start,
                    end=None,
                    open_only=self.open_check_open_only,
                    command_id=UUID4(),
                    ts_init=self._clock.timestamp_ns(),
                    log_receipt_level=LogLevel.DEBUG,
                ),
            )
            for c in clients
        ]

        order_reports_all = await asyncio.gather(*tasks, return_exceptions=True)
        all_order_reports: list[OrderStatusReport] = []

        for reports_or_exception in order_reports_all:
            if isinstance(reports_or_exception, Exception):
                self._log.error(
                    f"无法生成订单状态报告：{reports_or_exception}",
                )
                continue

            reports = cast(list[OrderStatusReport], reports_or_exception)
            all_order_reports.extend(reports)

        venue_reported_ids: set[ClientOrderId] = {
            report.client_order_id
            for report in all_order_reports
            if report.client_order_id is not None
        }

        return all_order_reports, venue_reported_ids

    def _reconcile_order_reports(
        self,
        all_order_reports: list[OrderStatusReport],
        open_order_ids: set[ClientOrderId],
    ) -> None:
        ts_now = self._clock.timestamp_ns()

        for report in all_order_reports:
            is_in_open_ids = report.client_order_id in open_order_ids

            # 清除成功查询订单的重试次数
            if report.client_order_id:
                self._clear_recon_tracking(report.client_order_id)
            elif report.venue_order_id:
                # 尝试将仅有场地 ID 的订单映射到客户端订单 ID，并清除该重试计数器
                mapped_client_id = self._cache.client_order_id(report.venue_order_id)
                if mapped_client_id:
                    self._clear_recon_tracking(mapped_client_id)

            # 检查是否应该对账此订单
            should_reconcile = False
            reconcile_reason = ""

            if report.is_open != is_in_open_ids:
                should_reconcile = True
                reconcile_reason = f"venue_open={report.is_open}, cache_open={is_in_open_ids}"
            elif report.client_order_id:
                order = self._cache.order(report.client_order_id)
                if order:
                    # 检查成交数量是否匹配，将 None 视为零
                    report_filled = (
                        report.filled_qty
                        if report.filled_qty is not None
                        else Quantity.zero(order.quantity.precision)
                    )
                    if order.filled_qty != report_filled:
                        should_reconcile = True
                        reconcile_reason = (
                            f"filled_qty mismatch: venue={report_filled}, cache={order.filled_qty}"
                        )

            if should_reconcile:
                # 对账前应用包含（include）过滤器
                if not self._consider_for_reconciliation(report.instrument_id):
                    self._log.debug(
                        f"跳过 {report.client_order_id!r} 的对账： "
                        f"交易标的 {report.instrument_id} 不在包含列表中",
                    )
                    continue

                # 检查近期本地活动，以避免与在途（In-flight）成交产生竞态条件
                local_activity = self._order_local_activity_ns.get(report.client_order_id)
                if local_activity and (ts_now - local_activity) < self._open_check_threshold_ns:
                    self._log.info(
                        f"推迟 {report.client_order_id!r} 的对账： "
                        f"近期本地活动 ({(ts_now - local_activity) / 1_000_000:.0f}ms < "
                        f"阈值={self.open_check_threshold_ms}ms)， "
                        f"原因为：{reconcile_reason}",
                    )
                    continue

                self._log.debug(
                    f"正在对账 {report.client_order_id!r}：{reconcile_reason}",
                    LogColor.BLUE,
                )
                self._reconcile_order_report(report, trades=[])

    async def _purge_account_events_loop(self, interval_mins: int) -> None:
        interval_secs = interval_mins * 60
        lookback_mins = self.purge_account_events_lookback_mins or 0
        lookback_secs = lookback_mins * 60

        try:
            while True:
                await asyncio.sleep(interval_secs)
                ts_now = self._clock.timestamp_ns()
                self._cache.purge_account_events(
                    ts_now=ts_now,
                    lookback_secs=lookback_secs,
                    purge_from_database=self.purge_from_database,
                )
        except asyncio.CancelledError:
            self._log.debug("任务 'purge_account_events' 已取消")
        except Exception as e:
            self._log.exception("清除账户事件时出错", e)

    # -- 请求处理器 ----------------------------------------------------------------------------------

    def generate_execution_mass_status(self, command: GenerateExecutionMassStatus) -> None:
        """
        处理生成执行批量状态的请求，触发启动对账。
        """
        self._log.info(f"收到 {command!r}", LogColor.BLUE)
        self._loop.create_task(self.reconcile_execution_state())

    async def reconcile_execution_state(
        self,
        timeout_secs: float = 10.0,
    ) -> bool:
        """
        对账执行状态，作为启动对账的主要入口点，协调所有执行客户端的对账工作。
        """
        PyCondition.positive(timeout_secs, "timeout_secs")

        try:
            for client_id in self._external_clients:
                command = GenerateExecutionMassStatus(
                    trader_id=self.trader_id,
                    client_id=client_id,
                    command_id=UUID4(),
                    venue=None,
                    ts_init=self._clock.timestamp_ns(),
                )
                self._log.info(
                    f"正在向 {client_id} 请求执行批量状态",
                    LogColor.BLUE,
                )
                self._msgbus.publish(
                    topic=f"commands.trading.{client_id}",
                    msg=command,
                )

            if not self._clients:
                self._log.debug("没有用于对账的执行客户端")
                # 即使没有客户端，也发出完成信号
                return True

            results: list[bool] = []

            # 向客户端请求执行批量状态报告
            reconciliation_lookback_mins: int | None = (
                self.reconciliation_lookback_mins if self.reconciliation_lookback_mins > 0 else None
            )
            mass_status_coros = [
                c.generate_mass_status(reconciliation_lookback_mins) for c in self._clients.values()
            ]
            mass_status_all = await asyncio.gather(*mass_status_coros, return_exceptions=True)

            # 将每个批量状态与执行引擎进行对账
            for mass_status_or_exception in mass_status_all:
                if isinstance(mass_status_or_exception, BaseException):
                    self._log.error(f"无法生成批量状态：{mass_status_or_exception}")
                    results.append(False)
                    continue

                if mass_status_or_exception is None:
                    self._log.warning(
                        "没有可用于对账的执行批量状态 "
                        "（可能是由于生成报告时适配器客户端出错）",
                    )
                    results.append(False)
                    continue

                mass_status = cast("ExecutionMassStatus", mass_status_or_exception)
                client_id = mass_status.client_id
                # venue = mass_status.venue
                result = self._reconcile_execution_mass_status(mass_status)

                if not result and self.filter_position_reports:
                    self._log_reconciliation_result(client_id, result)
                    results.append(result)
                    self._log.warning(
                        "已启用 `filter_position_reports`，跳过后续对账",
                    )
                    continue

                client = self._clients[client_id]

                # 检查内部和外部持仓对账
                report_tasks: list[asyncio.Task] = []

                # 对于具有路由功能的券商，场地可能与标的场地不同（例如：IB 客户端场地对应
                # NYSE 标的场地），因此按 account_id 进行过滤，而不是按 venue
                for position in self._cache.positions_open(
                    venue=None,
                    account_id=client.account_id,
                ):
                    instrument_id = position.instrument_id
                    if instrument_id in mass_status.position_reports:
                        self._log.debug(
                            f"已为 {client_id} 对账标的 {instrument_id} 的持仓",
                        )
                        continue  # 已完成对账

                    self._log.info(f"{position} 等待对账")
                    position_status_command = GeneratePositionStatusReports(
                        instrument_id=instrument_id,
                        start=None,
                        end=None,
                        command_id=UUID4(),
                        ts_init=self._clock.timestamp_ns(),
                    )
                    report_tasks.append(
                        client.generate_position_status_reports(position_status_command),
                    )

                if report_tasks:
                    # 对账特定的内部开仓持仓
                    self._log.info(f"正在等待 {client_id} 的 {len(report_tasks)} 份持仓报告")

                    position_results: list[bool] = []
                    for task_result_or_exception in await asyncio.gather(
                        *report_tasks,
                        return_exceptions=True,
                    ):
                        if isinstance(task_result_or_exception, Exception):
                            self._log.error(
                                f"无法生成持仓状态报告：{task_result_or_exception}",
                            )
                            position_results.append(False)
                            continue

                        task_result = cast("list[PositionStatusReport]", task_result_or_exception)
                        for report in task_result:
                            position_result = self._reconcile_position_report(report)
                            self._log_reconciliation_result(report.instrument_id, position_result)
                            position_results.append(position_result)

                    result = result and all(position_results)

                self._log_reconciliation_result(client_id, result)
                results.append(result)

                self._msgbus.publish(
                    topic=f"reports.execution.{mass_status.venue}",
                    msg=mass_status,
                )

            return all(results)
        finally:
            # 始终发出完成信号，以防止持续循环中的信号等待挂起
            self._startup_reconciliation_event.set()

    def _log_reconciliation_result(self, value: ClientId | InstrumentId, result: bool) -> None:
        if result:
            self._log.info(f"{value} 的对账成功", LogColor.GREEN)
        else:
            self._log.warning(f"{value} 的对账失败")

    def reconcile_execution_report(self, report: ExecutionReport) -> bool:
        """
        对收到的单个执行报告进行运行时对账，根据报告类型路由到相应的对账方法。
        """
        self._log.debug(f"<--[RPT] {report}")
        self.report_count += 1

        if not self._consider_for_reconciliation(report.instrument_id):
            self._log_skipping_reconciliation_on_instrument_id(report)
            return True  # 已过滤

        self._log.debug(f"正在对账 {report}", color=LogColor.BLUE)

        if isinstance(report, OrderStatusReport):
            result = self._reconcile_order_report(report, [])  # 无成交需要对账
        elif isinstance(report, FillReport):
            result = self._reconcile_fill_report_single(report)
        elif isinstance(report, PositionStatusReport):
            result = self._reconcile_position_report(report)
        else:
            self._log.error(  # pragma: no cover (design-time error)
                f"无法处理未识别的报告：{report}",  # pragma: no cover (design-time error)
            )
            return False

        self._msgbus.publish(
            topic=f"reports.execution.{report.instrument_id.venue}.{report.instrument_id.symbol}",
            msg=report,
        )

        return result

    # -- 对账 ---------------------------------------------------------------------------------------

    def reconcile_execution_mass_status(self, report: ExecutionMassStatus) -> None:
        """
        批量状态对账的入口点。
        """
        self._reconcile_execution_mass_status(report)

    def _reconcile_execution_mass_status(
        self,
        mass_status: ExecutionMassStatus,
    ) -> bool:
        self._log.debug(f"<--[RPT] {mass_status}")
        self.report_count += 1

        self._log.info(
            f"正在为 {mass_status.venue} 对账执行批量状态",
            color=LogColor.BLUE,
        )

        # 为首个生命周期不完整的标的调整成交
        self._adjust_mass_status_fills(mass_status)

        # 对批量状态中的订单进行去重
        self._deduplicate_mass_status_orders(mass_status)

        results: list[bool] = []
        reconciled_orders: set[ClientOrderId] = set()
        reconciled_trades: set[TradeId] = set()

        # 对账所有报告的订单
        for venue_order_id, order_report in mass_status.order_reports.items():
            trades = mass_status.fill_reports.get(venue_order_id, [])

            if not self._consider_for_reconciliation(order_report.instrument_id):
                self._log_skipping_reconciliation_on_instrument_id(order_report)
                continue  # 已过滤

            client_order_id = order_report.client_order_id

            if client_order_id is not None and client_order_id in self.filtered_client_order_ids:
                self._log.debug(
                    f"跳过 {order_report.client_order_id!r} 的 {type(order_report).__name__} 对账： "
                    f"在 `filtered_client_order_ids` 列表中",
                    LogColor.MAGENTA,
                )
                continue

            # 检查重复的 trade ID
            for fill_report in trades:
                if fill_report.trade_id in reconciled_trades:
                    self._log.warning(
                        f"检测到重复的 {fill_report.trade_id!r}：{fill_report}",
                    )

                reconciled_trades.add(fill_report.trade_id)

            try:
                # 应用所有成交 - 让持仓通过所有生命周期自然循环
                result = self._reconcile_order_report(order_report, trades)
            except InvalidStateTrigger as e:
                self._log.error(str(e))
                result = False

            results.append(result)

            if order_report.client_order_id is not None:
                # 仅追踪标的已加载的订单（其他订单已被过滤）
                instrument = self._cache.instrument(order_report.instrument_id)
                if instrument is not None:
                    reconciled_orders.add(order_report.client_order_id)

                    if result and order_report.venue_order_id is not None:
                        self._ensure_venue_order_id_indexed(
                            client_order_id=order_report.client_order_id,
                            venue_order_id=order_report.venue_order_id,
                        )

        if not self.filter_position_reports:
            position_reports: list[PositionStatusReport]

            # 对账所有报告的持仓
            for position_reports in mass_status.position_reports.values():
                for report in position_reports:
                    if not self._consider_for_reconciliation(report.instrument_id):
                        self._log_skipping_reconciliation_on_instrument_id(report)
                        continue  # 已过滤

                    result = self._reconcile_position_report(report)
                    results.append(result)

        # 发布批量状态
        self._msgbus.publish(
            topic=f"reports.execution.{mass_status.venue}",
            msg=mass_status,
        )

        # 验证对账状态一致性
        self._validate_reconciliation_state(mass_status, reconciled_orders)

        return all(results)

    def _adjust_mass_status_fills(self, mass_status: ExecutionMassStatus) -> None:
        # 为首个生命周期不完整的标的调整成交
        # 从原始订单和成交开始
        final_orders = dict(mass_status._order_reports)
        final_fills = dict(mass_status._fill_reports)

        reconciliation_instruments: list[Instrument] = []
        for instrument_id, position_reports in mass_status.position_reports.items():
            # 跳过双向持仓模式（具有 venue_position_id）的标的，因为部分窗口
            # 调整假设每个标的只有一个净持仓
            is_hedge_mode = any(r.venue_position_id is not None for r in position_reports)
            if is_hedge_mode:
                self._log.debug(
                    f"Skipping fill adjustment for {instrument_id}: "
                    f"hedge mode (has venue_position_id)",
                )
                continue

            # Respect reconciliation_instrument_ids filter
            if not self._consider_for_reconciliation(instrument_id):
                self._log.debug(
                    f"Skipping fill adjustment for {instrument_id}: "
                    f"not in `reconciliation_instrument_ids` include list",
                )
                continue

            instrument = self._cache.instrument(instrument_id)
            if not instrument:
                self._log.debug(
                    f"Skipping fill adjustment for {instrument_id}: instrument not found in cache",
                )
                continue

            reconciliation_instruments.append(instrument)

        self._log.info(
            f"正在尝试为 {len(reconciliation_instruments)} 个交易标的调整成交",
            LogColor.BLUE,
        )
        adjusted_results = adjust_fills_for_partial_window(
            mass_status,
            reconciliation_instruments,
            self._log,
        )
        self._log.info(
            f"正在为 {len(reconciliation_instruments)} 个交易标的更新已调整的成交",
            LogColor.BLUE,
        )

        for instrument_id, (
            adjusted_orders_for_instrument,
            adjusted_fills_for_instrument,
        ) in adjusted_results.items():
            # Remove old orders and fills for this instrument
            for venue_order_id in list(final_orders.keys()):
                order = final_orders[venue_order_id]
                if order.instrument_id == instrument_id:
                    del final_orders[venue_order_id]

            for venue_order_id in list(final_fills.keys()):
                fills = final_fills[venue_order_id]
                if fills and fills[0].instrument_id == instrument_id:
                    del final_fills[venue_order_id]

            # 为此标的更新已调整的成交
            final_orders.update(adjusted_orders_for_instrument)
            final_fills.update(adjusted_fills_for_instrument)

        # 一次性应用所有调整
        mass_status._order_reports = final_orders
        mass_status._fill_reports = final_fills
        self._log.info(
            f"最终的 order_reports 包含 {len(final_orders)} 个订单，fill_reports 包含所有标的的 {len(final_fills)} 个成交",
            LogColor.BLUE,
        )

    def _deduplicate_mass_status_orders(self, mass_status: ExecutionMassStatus) -> None:
        # 移除批量状态报告中的重复项
        seen_client_order_ids: dict[ClientOrderId, VenueOrderId] = {}
        duplicate_venue_order_ids: list[VenueOrderId] = []
        orders_to_skip: list[VenueOrderId] = []

        # 第一遍：当前报告内去重
        for venue_order_id, order_report in mass_status._order_reports.items():
            if order_report.client_order_id is not None:
                if order_report.client_order_id in seen_client_order_ids:
                    # 在当前报告中发现重复项 - 标记以移除
                    duplicate_venue_order_ids.append(venue_order_id)
                    self._log.warning(
                        f"正在对订单进行去重：{order_report.client_order_id} "
                        f"（venue_order_id={venue_order_id}，"
                        f"保留首次出现的 {seen_client_order_ids[order_report.client_order_id]}）",
                    )
                else:
                    # 第一次出现 - 追踪
                    seen_client_order_ids[order_report.client_order_id] = venue_order_id

        # 第二遍：检查缓存订单以防止重复
        # 仅当订单完全匹配（状态、成交数量等相同）时才跳过
        # 这样可以在防止重复创建的同时，仍然允许对差异进行对账
        for venue_order_id, order_report in mass_status._order_reports.items():
            if venue_order_id in duplicate_venue_order_ids:
                continue  # 已标记为重复

            # 通过 client_order_id 检查此订单是否已存在于缓存中
            if order_report.client_order_id is not None:
                cached_order = self._cache.order(order_report.client_order_id)
                if cached_order is not None:
                    # 跳过已关闭的对账订单，以防止重启时生成重复的推断成交
                    if (
                        cached_order.is_closed
                        and cached_order.tags is not None
                        and "RECONCILIATION" in cached_order.tags
                    ):
                        orders_to_skip.append(venue_order_id)
                        self._log.debug(
                            f"跳过已关闭的对账订单 {order_report.client_order_id}： "
                            "来自上个周期的合成持仓调整",
                        )
                        continue

                    # 订单已存在于缓存中 - 检查其是否为完全重复的
                    # 只有在完全匹配时才跳过（防止重复创建）
                    # 但如果有任何差异，仍需进行对账
                    report_filled = (
                        order_report.filled_qty
                        if order_report.filled_qty is not None
                        else Quantity.zero(cached_order.quantity.precision)
                    )

                    # 检查是否完全匹配 - 具有相同的状态、成交数量和交易标的
                    is_exact_match = (
                        cached_order.status == order_report.order_status
                        and cached_order.filled_qty == report_filled
                        and cached_order.instrument_id == order_report.instrument_id
                        and cached_order.side == order_report.order_side
                    )

                    if is_exact_match:
                        # 完全重复 - 跳过以防止重复创建
                        orders_to_skip.append(venue_order_id)
                        self._log.debug(
                            f"跳过完全重复的订单 {order_report.client_order_id}： "
                            "缓存中已存在状态相同的订单",
                        )
                        continue
                    # 如果不是完全匹配，继续执行对账以修正差异

            # 如果根据 client_order_id 查找失败或未提供，则还需尝试通过 venue_order_id 检查
            if order_report.venue_order_id is not None and order_report.client_order_id is None:
                cached_client_id = self._cache.client_order_id(order_report.venue_order_id)
                if cached_client_id is not None:
                    cached_order = self._cache.order(cached_client_id)
                    if cached_order is not None:
                        # 对报告进行更新，使用缓存的 client_order_id 以保持一致性
                        order_report.client_order_id = cached_client_id
                        self._log.debug(
                            f"通过 venue_order_id {order_report.venue_order_id} 找到缓存中的订单 {cached_client_id}，"
                             "正在更新报告以使用缓存的 client_order_id",
                        )
                        # 不要跳过 - 如果存在差异仍需进行对账

        # Remove duplicates and orders to skip
        orders_to_remove = set(duplicate_venue_order_ids) | set(orders_to_skip)
        for venue_order_id in orders_to_remove:
            del mass_status._order_reports[venue_order_id]

            # 同时也移除关联的成交
            if venue_order_id in mass_status._fill_reports:
                del mass_status._fill_reports[venue_order_id]

        if orders_to_remove:
            self._log.debug(
                f"从对账中移除了 {len(orders_to_remove)} 个重复/跳过的订单 "
                f"（{len(duplicate_venue_order_ids)} 个重复，{len(orders_to_skip)} 个已在缓存中）",
                LogColor.YELLOW,
            )

    def _validate_reconciliation_state(
        self,
        mass_status: ExecutionMassStatus,
        reconciled_orders: set[ClientOrderId],
    ) -> None:
        venue_order_ids_seen: set[VenueOrderId] = set()
        issues: list[str] = []

        for order_report in mass_status._order_reports.values():
            if order_report.venue_order_id is None:
                continue

            # 跳过已过滤的订单（例如，交易标的未加载）
            if order_report.client_order_id not in reconciled_orders:
                self._log.debug(
                    f"跳过 {order_report.client_order_id}（venue_order_id={order_report.venue_order_id}）的验证 - 不在 reconciled_orders 中",
                )
                continue

            if order_report.venue_order_id in venue_order_ids_seen:
                issues.append(
                    f"批量状态中存在重复的 venue_order_id {order_report.venue_order_id}",
                )

            venue_order_ids_seen.add(order_report.venue_order_id)

            # 检查 venue_order_id 是否已正确建立索引
            if order_report.client_order_id:
                cached_client_id = self._cache.client_order_id(order_report.venue_order_id)
                if cached_client_id is None:
                    issues.append(
                        f"柜台订单 ID {order_report.venue_order_id} 在缓存中未索引，"
                        f"对应 client_order_id {order_report.client_order_id}",
                    )
                elif cached_client_id != order_report.client_order_id:
                    issues.append(
                        f"柜台订单 ID {order_report.venue_order_id} 索引不匹配："
                        f"预期为 {order_report.client_order_id}，实际找到 {cached_client_id}",
                    )

        if issues:
            self._log.warning(
                f"对账状态验证发现 {len(issues)} 个问题：\n"
                + "\n".join(f"  - {issue}" for issue in issues),
            )
        else:
            self._log.debug(
                f"{len(mass_status._order_reports)} 个订单的对账状态验证通过",
            )

    # -- 成交对账 -----------------------------------------------------------------------------------

    def _reconcile_fill_report_single(self, report: FillReport) -> bool:
        if self._is_shutting_down:
            return True  # 停机期间跳过对账

        if not self._consider_for_reconciliation(report.instrument_id):
            self._log_skipping_reconciliation_on_instrument_id(report)
            return True  # 已过滤

        client_order_id: ClientOrderId | None = None
        if report.venue_order_id is not None:
            client_order_id = self._cache.client_order_id(report.venue_order_id)
        if client_order_id is None and report.client_order_id is not None:
            client_order_id = report.client_order_id
            if report.venue_order_id is not None:
                self._ensure_venue_order_id_indexed(
                    client_order_id=client_order_id,
                    venue_order_id=report.venue_order_id,
                    log_context="来自成交报告",
                )

        if client_order_id is None:
            self._log.warning(
                f"在收到 OrderStatusReport 之前收到了 {report.venue_order_id!r} 的 FillReport，"
                "推迟对账 - 这可能需要一个合成订单",
            )
            return False  # 失败

        order: Order | None = self._cache.order(client_order_id)

        if order is None:
            # 如果根据 client_order_id 查询失败，尝试根据 venue_order_id 查找订单
            # 这处理了外部订单可能尚未完全被索引的情况
            if report.venue_order_id is not None:
                order = self._find_order_by_venue_order_id(
                    venue_order_id=report.venue_order_id,
                    instrument_id=report.instrument_id,
                    order_side=None,  # 不按方向过滤，查找任何匹配的订单
                )
                if order is not None:
                    self._log.debug(
                        f"通过 venue_order_id {report.venue_order_id} 找到订单 {order.client_order_id}，"
                        "用于成交报告",
                    )
                    # 确保映射已建立索引
                    self._ensure_venue_order_id_indexed(
                        client_order_id=order.client_order_id,
                        venue_order_id=report.venue_order_id,
                        log_context="用于成交报告 (for fill report)",
                    )
            if order is None:
                self._log.warning(
                    f"在订单缓存之前收到了 {client_order_id!r} 的 FillReport"
                    f"（venue_order_id={report.venue_order_id!r}），推迟对账",
                )
                return False  # 失败

        # 记录外部订单处理情况以提高可见性
        if order.strategy_id.value == "EXTERNAL":
            self._log.debug(
                f"正在处理外部订单 {order.client_order_id} 的成交 "
                f"（venue_order_id={order.venue_order_id}）",
            )

        instrument: Instrument | None = self._cache.instrument(order.instrument_id)
        if instrument is None:
            self._log.debug(
                f"无法为 {order.client_order_id!r} 对账订单： "
                f"未找到交易标的 {order.instrument_id}",
            )
            return True  # 标的已过滤或未加载

        return self._reconcile_fill_report(order, report, instrument)

    def _fill_reports_equal(self, cached_fill: OrderFilled, report: FillReport) -> bool:
        # 来自某些场地/路径的报告中可能缺失手续费信息；进行安全比较
        if cached_fill.commission is None and report.commission is None:
            commissions_equal = True
        elif cached_fill.commission is None or report.commission is None:
            commissions_equal = False
        else:
            commissions_equal = (
                cached_fill.commission.currency == report.commission.currency
                and cached_fill.commission == report.commission
            )

        return (
            cached_fill.last_qty == report.last_qty
            and cached_fill.last_px == report.last_px
            and commissions_equal
            and cached_fill.liquidity_side == report.liquidity_side
            and cached_fill.ts_event == report.ts_event
        )

    def _rollback_fill_audit_entry(
        self,
        client_order_id: ClientOrderId,
        audit_entry: tuple[TradeId, str, int],
    ) -> None:
        # 当成交应用失败时，移除审计分录
        if audit_entry in self._fill_application_audit.get(client_order_id, []):
            self._fill_application_audit[client_order_id].remove(audit_entry)

    def _create_order_status_report_from_cached_order(
        self,
        cached_order: Order,
        instrument_id: InstrumentId,
        account_id: AccountId,
        order_side: OrderSide,
        quantity: Quantity,
        filled_qty: Quantity,
        price: Price | None,
        avg_px: Decimal | None,
        ts_now: int,
        venue_position_id: PositionId | None = None,
    ) -> OrderStatusReport:
        return OrderStatusReport(
            instrument_id=instrument_id,
            account_id=account_id,
            venue_order_id=cached_order.venue_order_id or VenueOrderId(str(UUID4())),
            venue_position_id=venue_position_id,
            order_side=order_side,
            order_type=cached_order.order_type,
            time_in_force=cached_order.time_in_force,
            order_status=OrderStatus.FILLED,
            price=price,
            quantity=quantity,
            filled_qty=filled_qty,
            avg_px=avg_px,
            report_id=UUID4(),
            ts_accepted=ts_now,
            ts_last=ts_now,
            ts_init=ts_now,
            client_order_id=cached_order.client_order_id,
        )

    # -- 持仓对账 -----------------------------------------------------------------------------------

    def _reconcile_position_report(self, report: PositionStatusReport) -> bool:
        if self._is_shutting_down:
            return True  # 停机期间跳过对账

        if not self._consider_for_reconciliation(report.instrument_id):
            self._log_skipping_reconciliation_on_instrument_id(report)
            return True  # 已过滤

        if report.venue_position_id is not None:
            return self._reconcile_position_report_hedging(report)
        else:
            return self._reconcile_position_report_netting(report)

    def _consider_for_reconciliation(self, instrument_id: InstrumentId) -> bool:
        if self.reconciliation_instrument_ids:
            return instrument_id in self.reconciliation_instrument_ids

        return True

    def _log_skipping_reconciliation_on_instrument_id(self, report: ExecutionReport) -> None:
        self._log.debug(
            f"跳过 {report.instrument_id} 的 {type(report).__name__} 对账： "
            "不在 `reconciliation_instrument_ids` 包含列表中",
            LogColor.MAGENTA,
        )

    def _reconcile_position_report_hedging(self, report: PositionStatusReport) -> bool:
        self._log.info(
            f"正在为 {report.instrument_id} 对账对冲（HEDGE）持仓，venue_position_id={report.venue_position_id}",
            LogColor.BLUE,
        )

        position: Position | None = self._cache.position(report.venue_position_id)

        if position is None:
            if report.signed_decimal_qty == 0:
                return True  # 均为平仓，没有问题

            if not self.generate_missing_orders:
                self._log.error(
                    f"无法对账持仓：未找到 {report.venue_position_id!r} "
                    "且已禁用 `generate_missing_orders`",
                )
                return False

            return self._reconcile_missing_hedge_position(report)

        position_signed_decimal_qty: Decimal = position.signed_decimal_qty()

        if position_signed_decimal_qty != report.signed_decimal_qty:
            if not self.generate_missing_orders:
                self._log.error(
                    f"无法对账 {report.instrument_id} {report.venue_position_id!r}： "
                    f"持仓净数量 {position_signed_decimal_qty} != 报告的净数量 "
                    f"{report.signed_decimal_qty} 且已禁用 `generate_missing_orders`",
                )
                return False

            return self._reconcile_hedge_position_discrepancy(
                report=report,
                position=position,
                position_signed_decimal_qty=position_signed_decimal_qty,
            )

        return True  # 已对账

    def _reconcile_hedge_position_discrepancy(
        self,
        report: PositionStatusReport,
        position: Position,
        position_signed_decimal_qty: Decimal,
    ) -> bool:
        instrument = self._cache.instrument(report.instrument_id)
        if instrument is None:
            self._log.debug(
                f"无法为 {report.instrument_id} 对账持仓：未找到交易标的",
            )
            return True  # 标的已过滤或未加载

        diff = abs(position_signed_decimal_qty - report.signed_decimal_qty)
        diff_quantity = Quantity(diff, instrument.size_precision)

        if diff_quantity == 0:
            self._log.debug(
                f"{instrument.id} 的差额数量四舍五入后为零，跳过",
            )
            return True

        self._log.warning(
            f"{report.instrument_id} {report.venue_position_id!r} 的对冲持仓不一致： "
            f"缓存={position_signed_decimal_qty}，柜台={report.signed_decimal_qty}，正在生成对账订单",
            LogColor.YELLOW,
        )

        current_avg_px = Decimal(str(position.avg_px_open)) if position.avg_px_open else None

        diff_report = self._create_position_reconciliation_report(
            report=report,
            instrument=instrument,
            position_signed_decimal_qty=position_signed_decimal_qty,
            diff_quantity=diff_quantity,
            current_avg_px=current_avg_px,
        )

        if diff_report:
            self._reconcile_order_report(diff_report, trades=[], is_external=False)

        return True

    def _reconcile_missing_hedge_position(self, report: PositionStatusReport) -> bool:
        instrument = self._cache.instrument(report.instrument_id)
        if instrument is None:
            self._log.debug(
                f"无法为 {report.instrument_id} 对账持仓：未找到交易标的",
            )
            return True  # 标的已过滤或未加载

        quantity = Quantity(abs(report.signed_decimal_qty), instrument.size_precision)

        if quantity == 0:
            return True

        self._log.warning(
            f"{report.instrument_id} {report.venue_position_id!r} 缺失对冲持仓： "
            f"柜台报告为 {report.signed_decimal_qty}，正在生成对账订单",
            LogColor.YELLOW,
        )

        diff_report = self._create_position_reconciliation_report(
            report=report,
            instrument=instrument,
            position_signed_decimal_qty=Decimal(0),
            diff_quantity=quantity,
            current_avg_px=None,
        )

        if diff_report:
            self._reconcile_order_report(diff_report, trades=[], is_external=False)

        return True

    def _reconcile_position_report_netting(
        self,
        report: PositionStatusReport,
    ) -> bool:
        self._log.info(f"正在为 {report.instrument_id} 对账净额（NET）持仓", LogColor.BLUE)

        instrument = self._cache.instrument(report.instrument_id)
        if instrument is None:
            self._log.debug(
                f"无法为 {report.instrument_id} 对账持仓：未找到交易标的",
            )
            return True  # 标的已过滤或未加载

        positions_open: list[Position] = self._cache.positions_open(
            venue=None,  # 更快的查询过滤
            instrument_id=report.instrument_id,
        )

        position_signed_decimal_qty: Decimal = Decimal()

        for position in positions_open:
            position_signed_decimal_qty += position.signed_decimal_qty()

        self._log.info(f"{report.signed_decimal_qty=}", LogColor.BLUE)
        self._log.info(f"{position_signed_decimal_qty=}", LogColor.BLUE)

        # 检查数量是否匹配
        quantities_match = position_signed_decimal_qty == report.signed_decimal_qty

        if not quantities_match:
            if not self.generate_missing_orders:
                self._log.warning(
                    f"当禁用 `generate_missing_orders` 时，{report.instrument_id} 持仓出现不一致，跳过进一步对账",
                )
                return True

            diff = abs(position_signed_decimal_qty - report.signed_decimal_qty)
            diff_quantity = Quantity(diff, instrument.size_precision)
            self._log.info(f"{diff_quantity=}", LogColor.BLUE)

            if diff_quantity == 0:
                self._log.debug(
                    f"{instrument.id} 的差额数量四舍五入后为零，跳过订单生成",
                )
                return True

            # 如果可用，计算当前持仓平均价格（对账所需）
            current_avg_px = None
            if positions_open:
                # 计算当前持仓的加权平均价格
                total_value = Decimal(0)
                total_qty = Decimal(0)

                for pos in positions_open:
                    qty = abs(pos.signed_decimal_qty())

                    if pos.avg_px_open and qty > 0:
                        total_value += Decimal(str(pos.avg_px_open)) * qty
                        total_qty += qty

                if total_qty > 0:
                    current_avg_px = total_value / total_qty

            # 检查持仓是否跨越零点（从多头转为空头，或反之亦然）
            crosses_zero = (
                position_signed_decimal_qty != 0
                and report.signed_decimal_qty != 0
                and (
                    (position_signed_decimal_qty > 0 and report.signed_decimal_qty < 0)
                    or (position_signed_decimal_qty < 0 and report.signed_decimal_qty > 0)
                )
            )

            if crosses_zero:
                return self._reconcile_cross_zero_position(
                    report=report,
                    instrument=instrument,
                    position_signed_decimal_qty=position_signed_decimal_qty,
                    current_avg_px=current_avg_px,
                )

            diff_report = self._create_position_reconciliation_report(
                report=report,
                instrument=instrument,
                position_signed_decimal_qty=position_signed_decimal_qty,
                diff_quantity=diff_quantity,
                current_avg_px=current_avg_px,
            )
            if diff_report:
                self._reconcile_order_report(diff_report, trades=[], is_external=False)
        elif quantities_match and report.avg_px_open is not None:
            # 数量匹配，但需验证 avg_px_open 是否也匹配
            current_avg_px = None
            if positions_open:
                # 计算当前持仓的加权平均价格
                total_value = Decimal(0)
                total_qty = Decimal(0)

                for pos in positions_open:
                    qty = abs(pos.signed_decimal_qty())

                    if pos.avg_px_open and qty > 0:
                        total_value += Decimal(str(pos.avg_px_open)) * qty
                        total_qty += qty

                if total_qty > 0:
                    current_avg_px = total_value / total_qty

            if current_avg_px is not None:
                # 检查 avg_px 在容差范围内是否匹配
                avg_px_diff = abs(current_avg_px - report.avg_px_open)
                relative_diff = avg_px_diff / report.avg_px_open if report.avg_px_open != 0 else 0

                if relative_diff > Decimal("0.0001"):  # 0.01% 容差
                    self._log.warning(
                        f"{report.instrument_id} 对账后持仓平均价格（avg_px）不匹配： "
                        f"内部={current_avg_px}, 柜台={report.avg_px_open}, "
                        f"差异={avg_px_diff} ({relative_diff * 100:.4f}%)。 "
                        "这表明来自柜台的对账数据不完整。",
                        LogColor.YELLOW,
                    )
                else:
                    self._log.info(
                        f"已验证 {report.instrument_id} 的持仓平均价格（avg_px）： "
                        f"内部={current_avg_px}, 柜台={report.avg_px_open}",
                        LogColor.BLUE,
                    )

        return True  # 已完成对账

    def _reconcile_cross_zero_position(
        self,
        report: PositionStatusReport,
        instrument: Instrument,
        position_signed_decimal_qty: Decimal,
        current_avg_px: Decimal | None,
    ) -> bool:
        self._log.info(
            f"{report.instrument_id} 的持仓跨越零点（反向）： "
            f"当前={position_signed_decimal_qty}, 目标={report.signed_decimal_qty}。 "
            "将对账拆分为两次成交：首先平掉现有持仓，然后开设新持仓",
            LogColor.BLUE,
        )

        now = self._clock.timestamp_ns()

        # 第一笔成交：平掉现有持仓（归零）
        close_qty_decimal = abs(position_signed_decimal_qty)
        close_quantity = Quantity(close_qty_decimal, instrument.size_precision)
        close_side = OrderSide.BUY if position_signed_decimal_qty < 0 else OrderSide.SELL

        # 使用当前持仓平均价进行平仓
        close_price = None
        if current_avg_px is not None:
            close_price = instrument.make_price(current_avg_px)
        else:
            quote = self._cache.quote_tick(report.instrument_id)
            if quote:
                close_price = quote.ask_price if close_side == OrderSide.BUY else quote.bid_price

        close_result = False
        if close_price:
            # 修正 2：在创建合成订单前检查匹配的缓存订单
            close_avg_px = close_price.as_decimal()
            matching_close_order = self._find_matching_cached_order(
                instrument_id=report.instrument_id,
                order_side=close_side,
                quantity=close_quantity,
                price=close_price,
                avg_px=close_avg_px,
            )

            if matching_close_order:
                self._log.debug(
                    f"找到匹配的缓存订单 {matching_close_order.client_order_id} "
                    f"用于平仓 {report.instrument_id}，重用该订单而不是创建合成订单",
                )
                close_report = self._create_order_status_report_from_cached_order(
                    cached_order=matching_close_order,
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    order_side=close_side,
                    quantity=close_quantity,
                    filled_qty=close_quantity,
                    price=close_price,
                    avg_px=close_avg_px,
                    ts_now=now,
                    venue_position_id=report.venue_position_id,
                )
            else:
                close_report = OrderStatusReport(
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    venue_order_id=VenueOrderId(str(UUID4())),
                    venue_position_id=report.venue_position_id,
                    order_side=close_side,
                    order_type=OrderType.LIMIT,
                    time_in_force=TimeInForce.GTC,
                    order_status=OrderStatus.FILLED,
                    price=close_price,
                    quantity=close_quantity,
                    filled_qty=close_quantity,
                    avg_px=close_avg_px,
                    report_id=UUID4(),
                    ts_accepted=now,
                    ts_last=now,
                    ts_init=now,
                )
            close_result = self._reconcile_order_report(
                close_report,
                trades=[],
                is_external=False,
            )

        # 第二笔成交：反向开设新持仓
        open_qty_decimal = abs(report.signed_decimal_qty)
        open_quantity = Quantity(open_qty_decimal, instrument.size_precision)
        open_side = OrderSide.BUY if report.signed_decimal_qty > 0 else OrderSide.SELL

        # 使用场地报告的平均价作为新持仓价格
        open_price = None
        if report.avg_px_open is not None:
            open_price = instrument.make_price(report.avg_px_open)
        else:
            quote = self._cache.quote_tick(report.instrument_id)
            if quote:
                open_price = quote.ask_price if open_side == OrderSide.BUY else quote.bid_price
            elif close_price:
                # 仅允许对 CurrencyPair 进行备选处理，因为现货资产持仓可能缺少成本依据
                is_currency_pair = isinstance(instrument, CurrencyPair)

                if is_currency_pair:
                    open_price = close_price
                    self._log.warning(
                        f"在 {report.instrument_id} 的跨零对账中，使用平仓价格 {close_price} 作为开仓位置的备选； "
                        "柜台持仓报告缺少 avg_px_open（没有成本依据的现货资产持仓）",
                    )
                else:
                    self._log.error(
                        f"无法确定 {report.instrument_id} 的开仓价格： "
                        "柜台持仓报告缺少 avg_px_open 且没有可用的报价（quote tick）； "
                        "此备选方案仅允许用于 CurrencyPair（现货资产）持仓",
                    )
            else:
                self._log.error(
                    f"无法确定 {report.instrument_id} 的开仓价格： "
                    "没有可用的平仓价格（现有持仓缺少 avg_px）， "
                    "柜台持仓报告缺少 avg_px_open，且没有可用的报价",
                )

        open_result = False
        if open_price:
            # 修正 2：在创建合成订单前检查匹配的缓存订单
            open_avg_px = open_price.as_decimal()
            matching_open_order = self._find_matching_cached_order(
                instrument_id=report.instrument_id,
                order_side=open_side,
                quantity=open_quantity,
                price=open_price,
                avg_px=open_avg_px,
            )

            if matching_open_order:
                self._log.debug(
                    f"找到匹配的缓存订单 {matching_open_order.client_order_id} "
                    f"用于开仓 {report.instrument_id}，重用该订单而不是创建合成订单",
                )
                open_report = self._create_order_status_report_from_cached_order(
                    cached_order=matching_open_order,
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    order_side=open_side,
                    quantity=open_quantity,
                    filled_qty=open_quantity,
                    price=open_price,
                    avg_px=open_avg_px,
                    ts_now=now,
                    venue_position_id=report.venue_position_id,
                )
            else:
                open_report = OrderStatusReport(
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    venue_order_id=VenueOrderId(str(UUID4())),
                    venue_position_id=report.venue_position_id,
                    order_side=open_side,
                    order_type=OrderType.LIMIT,
                    time_in_force=TimeInForce.GTC,
                    order_status=OrderStatus.FILLED,
                    price=open_price,
                    quantity=open_quantity,
                    filled_qty=open_quantity,
                    avg_px=open_avg_px,
                    report_id=UUID4(),
                    ts_accepted=now,
                    ts_last=now,
                    ts_init=now,
                )
            open_result = self._reconcile_order_report(
                open_report,
                trades=[],
                is_external=False,
            )

        # 检查两笔成交是否均成功
        if not (close_result and open_result):
            self._log.error(
                f"对账 {report.instrument_id} 的跨零持仓失败： "
                f"平仓={close_result}, 开仓={open_result}",
            )
            return False

        return True  # 通过拆分成两笔成交完成对账

    def _create_position_reconciliation_report(
        self,
        report: PositionStatusReport,
        instrument: Instrument,
        position_signed_decimal_qty: Decimal,
        diff_quantity: Quantity,
        current_avg_px: Decimal | None,
    ) -> OrderStatusReport | None:
        order_side = (
            OrderSide.BUY
            if report.signed_decimal_qty > position_signed_decimal_qty
            else OrderSide.SELL
        )

        # 计算对账价格
        reconciliation_price = calculate_reconciliation_price(
            current_position_qty=position_signed_decimal_qty,
            current_position_avg_px=current_avg_px,
            target_position_qty=report.signed_decimal_qty,
            target_position_avg_px=report.avg_px_open,
            instrument=instrument,
        )

        # 如果无法计算价格，使用合理的备选方案
        if reconciliation_price is None:
            # 如果 avg_px_open 为 None，我们无法计算精确的对账价格，
            # 将回退到市价。
            self._log.warning(
                f"无法计算 {report.instrument_id} 的精确对账价格： "
                "持仓报告缺少平均价格信息，使用最后报价作为备选",
            )

            quote = self._cache.quote_tick(report.instrument_id)

            if quote:
                if order_side == OrderSide.BUY:
                    reconciliation_price = quote.ask_price
                else:  # OrderSide.SELL
                    reconciliation_price = quote.bid_price
            else:
                # 如果没有市场数据，使用持仓的当前平均价格作为备选
                if current_avg_px is not None:
                    reconciliation_price = instrument.make_price(current_avg_px)

        now = self._clock.timestamp_ns()

        if reconciliation_price:
            # 生成一笔带有计算得出的对账价格的限价（LIMIT）订单
            avg_px = reconciliation_price.as_decimal()

            # 仅在净额模式下重用缓存订单 - 对冲模式持仓是单独跟踪的，
            # 重用订单可能会匹配到错误的持仓
            matching_diff_order = None
            if report.venue_position_id is None:
                matching_diff_order = self._find_matching_cached_order(
                    instrument_id=report.instrument_id,
                    order_side=order_side,
                    quantity=diff_quantity,
                    price=reconciliation_price,
                    avg_px=avg_px,
                )

            if matching_diff_order:
                self._log.debug(
                    f"找到匹配的缓存订单 {matching_diff_order.client_order_id} "
                    f"用于持仓对账 {report.instrument_id}，重用该订单而不是创建合成订单",
                )
                return self._create_order_status_report_from_cached_order(
                    cached_order=matching_diff_order,
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    order_side=order_side,
                    quantity=diff_quantity,
                    filled_qty=diff_quantity,
                    price=reconciliation_price,
                    avg_px=avg_px,
                    ts_now=now,
                    venue_position_id=report.venue_position_id,
                )
            else:
                return OrderStatusReport(
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    venue_order_id=VenueOrderId(str(UUID4())),
                    venue_position_id=report.venue_position_id,
                    order_side=order_side,
                    order_type=OrderType.LIMIT,
                    time_in_force=TimeInForce.GTC,
                    order_status=OrderStatus.FILLED,
                    price=reconciliation_price,
                    quantity=diff_quantity,
                    filled_qty=diff_quantity,
                    avg_px=avg_px,
                    report_id=UUID4(),
                    ts_accepted=now,
                    ts_last=now,
                    ts_init=now,
                )
        else:
            # 无价格信息，回退到生成的市价（MARKET）订单
            avg_px = None
            self._log.warning(
                f"无法确定 {report.instrument_id} 的对账价格， "
                "正在为持仓对账生成市价（MARKET）订单 "
                f"（当前：{position_signed_decimal_qty}，目标：{report.signed_decimal_qty}）",
            )

            # 仅对净额模式重用缓存订单
            matching_diff_order = None
            if report.venue_position_id is None:
                matching_diff_order = self._find_matching_cached_order(
                    instrument_id=report.instrument_id,
                    order_side=order_side,
                    quantity=diff_quantity,
                    price=None,
                    avg_px=None,
                )

            if matching_diff_order:
                self._log.debug(
                    f"找到匹配的缓存订单 {matching_diff_order.client_order_id} "
                    f"用于持仓对账 {report.instrument_id}，重用该订单而不是创建合成订单",
                )
                return self._create_order_status_report_from_cached_order(
                    cached_order=matching_diff_order,
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    order_side=order_side,
                    quantity=diff_quantity,
                    filled_qty=diff_quantity,
                    price=None,
                    avg_px=avg_px,
                    ts_now=now,
                    venue_position_id=report.venue_position_id,
                )
            else:
                return OrderStatusReport(
                    instrument_id=report.instrument_id,
                    account_id=report.account_id,
                    venue_order_id=VenueOrderId(str(UUID4())),
                    venue_position_id=report.venue_position_id,
                    order_side=order_side,
                    order_type=OrderType.MARKET,
                    time_in_force=TimeInForce.IOC,
                    order_status=OrderStatus.FILLED,
                    quantity=diff_quantity,
                    filled_qty=diff_quantity,
                    avg_px=avg_px,
                    report_id=UUID4(),
                    ts_accepted=now,
                    ts_last=now,
                    ts_init=now,
                )

    def _reconcile_order_report(
        self,
        report: OrderStatusReport,
        trades: list[FillReport],
        is_external: bool = True,
    ) -> bool:
        if self._is_shutting_down:
            return True  # 停机期间跳过对账

        client_order_id = self._resolve_client_order_id(report)

        # 重置重试计数
        self._clear_recon_tracking(client_order_id)

        self._log.debug(f"正在为 {client_order_id!r} 对账订单", LogColor.MAGENTA)
        order: Order = self._cache.order(client_order_id)

        if order is None:
            instrument = self._cache.instrument(report.instrument_id)
            if instrument is None:
                self._log.debug(
                    f"无法为 {client_order_id!r} 对账订单： "
                    f"未找到交易标的 {report.instrument_id}",
                )
                return True  # 标的已过滤或未加载

            order = self._generate_order(report, is_external)

            if order is None:
                # 外部订单已丢弃
                return True  # 不再进行进一步对账

            # 加入缓存时最初不确定任何持仓 ID
            self._cache.add_order(order)

            # 为外部订单显式建立 venue_order_id 索引，以确保在随后的对账过程中
            # 能够通过 venue_order_id 找到它们
            if order.venue_order_id is not None:
                self._ensure_venue_order_id_indexed(
                    client_order_id=order.client_order_id,
                    venue_order_id=order.venue_order_id,
                )

            if self.manage_own_order_books and py_should_handle_own_book_order(order):
                self._add_own_book_order(order)

        else:
            # 订单已存在，检查交易标的
            instrument = self._cache.instrument(order.instrument_id)
            if instrument is None:
                self._log.debug(
                    f"无法为 {order.client_order_id!r} 对账订单： "
                    f"未找到交易标的 {order.instrument_id}",
                )
                return True  # 标的已过滤或未加载

        # 处理订单状态转换
        status_result = self._handle_order_status_transitions(order, report, trades, instrument)
        if status_result is not None:
            return status_result

        # 对账所有成交
        for trade in trades:
            self._reconcile_fill_report(order, trade, instrument)

        if report.avg_px is None:
            self._log.warning("预期有值时 report.avg_px 为 `None`")

        # 处理成交数量不匹配的情况
        return self._handle_fill_quantity_mismatch(order, report, instrument, client_order_id)

    def _resolve_client_order_id(self, report: OrderStatusReport) -> ClientOrderId:
        client_order_id: ClientOrderId | None = report.client_order_id
        if client_order_id is None:
            client_order_id = self._cache.client_order_id(report.venue_order_id)
            if client_order_id is None and report.venue_order_id is not None:
                # 检查具有此 venue_order_id 的外部订单是否已存在
                # 通过搜索缓存的订单（处理索引可能尚未建立的情况）
                cached_order = self._find_order_by_venue_order_id(
                    venue_order_id=report.venue_order_id,
                    instrument_id=report.instrument_id,
                    order_side=report.order_side,
                )
                if cached_order is not None:
                    client_order_id = cached_order.client_order_id
                    self._log.debug(
                        f"通过 venue_order_id {report.venue_order_id} 找到现有的外部订单 {client_order_id}， "
                        "正在重用",
                    )
                    # 确保映射已建立索引
                    self._ensure_venue_order_id_indexed(
                        client_order_id=client_order_id,
                        venue_order_id=report.venue_order_id,
                    )

            if client_order_id is None:
                # 生成外部客户端订单 ID
                client_order_id = ClientOrderId(UUID4().value)

            # 分配给报告
            report.client_order_id = client_order_id

        return client_order_id

    def _ensure_venue_order_id_indexed(
        self,
        client_order_id: ClientOrderId,
        venue_order_id: VenueOrderId,
        log_context: str = "",
    ) -> None:
        # 在缓存中为 venue_order_id 建立索引以便查询
        try:
            self._cache.add_venue_order_id(
                client_order_id,
                venue_order_id,
                overwrite=False,
            )
        except ValueError:
            # 映射已存在或冲突 - 如果订单在
            # 之前已建立索引，或存在冲突（应该很少见），这是符合预期的
            self._log.debug(
                f"柜台订单 ID {venue_order_id} 已为 "
                f"{client_order_id}{' ' + log_context if log_context else ''} 建立索引，跳过",
            )

    def _handle_fill_quantity_mismatch(
        self,
        order: Order,
        report: OrderStatusReport,
        instrument: Instrument,
        client_order_id: ClientOrderId,
    ) -> bool:
        if report.filled_qty < order.filled_qty:
            # 收集诊断信息
            fill_history = [
                (event.trade_id, event.last_qty, event.ts_event)
                for event in order.events
                if isinstance(event, OrderFilled)
            ]

            self._log.error(
                f"report.filled_qty {report.filled_qty} < order.filled_qty {order.filled_qty}， "
                "这可能是由于成交重复或缓存状态损坏引起的； "
                f"order_id={order.client_order_id}, venue_order_id={order.venue_order_id}, "
                f"total_fills_applied={len(fill_history)}, "
                f"fill_trade_ids={order.trade_ids}, "
                f"inferred_fill={'yes' if client_order_id in self._inferred_fill_ts else 'no'}, "
                f"order_status={order.status}, report_status={report.order_status}",
            )

            # 记录每笔成交以便法庭调查（forensics）
            for trade_id, qty, ts in fill_history:
                self._log.error(f"  成交：{trade_id}，数量={qty}，时间={ts}")

            return False  # 失败

        if report.filled_qty > order.filled_qty:
            # 检查订单是否已关闭，以避免重复生成推断成交
            if order.is_closed:
                # 使用更高精度进行容差检查
                precision = max(report.filled_qty.precision, order.filled_qty.precision)
                if is_within_single_unit_tolerance(
                    report.filled_qty.as_decimal(),
                    order.filled_qty.as_decimal(),
                    precision,
                ):
                    return True

                # 注意：在初始开发阶段后，可以将日志级别降至 debug
                self._log.debug(
                    f"{order.instrument_id} {order.client_order_id!r} 已处于 {order.status_string()} 状态，但 "
                    "报告的成交数量有差异： "
                    f"报告={report.filled_qty}，缓存={order.filled_qty}， "
                    "跳过为已关闭订单生成推断成交",
                )
                return True  # 视为已完成对账以避免无限循环

            # 这是由于成交报告缺失引起的，如果达到报告状态前
            # 发生了多次成交，或者手续费与默认值不同，现在可能会有一些信息丢失。
            try:
                fill: OrderFilled = self._generate_inferred_fill(order, report, instrument)
                self._handle_event_with_tracking(fill)
            except ValueError as e:
                self._log.error(
                    f"无法为 {order.client_order_id} 生成推断成交：{e}。 "
                    "该订单的对账失败。",
                )
                return False  # 失败

            if (
                report.avg_px is not None
                and order.avg_px is not None
                and not math.isclose(float(report.avg_px), float(order.avg_px))
            ):
                self._log.warning(
                    f"report.avg_px {report.avg_px} != order.avg_px {order.avg_px}， "
                    "这可能是由于推断成交导致信息丢失引起的",
                )

        return True  # 已完成对账

    def _handle_order_status_transitions(
        self,
        order: Order,
        report: OrderStatusReport,
        trades: list[FillReport],
        instrument: Instrument,
    ) -> bool | None:
        if report.order_status == OrderStatus.REJECTED:
            if order.status != OrderStatus.REJECTED:
                self._generate_order_rejected(order, report)

            return True  # 已完成对账

        if report.order_status == OrderStatus.ACCEPTED:
            if order.status != OrderStatus.ACCEPTED:
                self._generate_order_accepted(order, report)

            return True  # 已完成对账

        # 从此点开始订单必须已被接受
        if order.status in (OrderStatus.INITIALIZED, OrderStatus.SUBMITTED):
            self._generate_order_accepted(order, report)

        # 更新订单数量和价格差异
        if self._should_update(order, report):
            self._generate_order_updated(order, report)

        if report.order_status == OrderStatus.TRIGGERED:
            if order.status != OrderStatus.TRIGGERED:
                self._generate_order_triggered(order, report)

            return True  # 已完成对账

        if report.order_status == OrderStatus.CANCELED:
            if order.status != OrderStatus.CANCELED and order.is_open:
                if report.ts_triggered > 0:
                    self._generate_order_triggered(order, report)

                # 对账所有成交
                for trade in trades:
                    self._reconcile_fill_report(order, trade, instrument)

                self._generate_order_canceled(order, report)

            return True  # 已完成对账

        if report.order_status == OrderStatus.EXPIRED:
            if order.status != OrderStatus.EXPIRED and order.is_open:
                if report.ts_triggered > 0:
                    self._generate_order_triggered(order, report)

                # 在过期事件前对账所有成交（与取消相同）
                for trade in trades:
                    self._reconcile_fill_report(order, trade, instrument)

                self._generate_order_expired(order, report)

            return True  # 已完成对账

        return None  # 继续处理成交对账

    def _should_update(self, order: Order, report: OrderStatusReport) -> bool:
        if report.quantity != order.quantity and report.quantity >= order.filled_qty:
            return True  # 有效的数量更新

        match order.order_type:
            case OrderType.LIMIT:
                return report.price != order.price
            case OrderType.STOP_MARKET | OrderType.TRAILING_STOP_MARKET:
                return report.trigger_price != order.trigger_price
            case OrderType.STOP_LIMIT | OrderType.TRAILING_STOP_LIMIT:
                return report.trigger_price != order.trigger_price or report.price != order.price
            case _:
                return False

    def _reconcile_fill_report(
        self,
        order: Order,
        report: FillReport,
        instrument: Instrument,
    ) -> bool:
        # 检查是否应跳过此成交（早于推断成交或者是重复成交）
        skip_result = self._check_and_skip_duplicate_fill(order, report)
        if skip_result is not None:
            return skip_result

        # 检查成交是否会导致超额成交
        potential_filled_qty = order.filled_qty + report.last_qty
        if potential_filled_qty > order.quantity:
            if not self.allow_overfills:
                self._log.warning(
                    f"拒绝会导致 {order.client_order_id!r} 超额成交的成交报告： "
                    f"订单数量={order.quantity}，已成交数量={order.filled_qty}， "
                    f"成交数量={report.last_qty}，将导致成交总量={potential_filled_qty}",
                )
                return False  # 拒绝成交以防止超额成交
            # allow_overfills=True: 记录警告但允许成交通过
            self._log.warning(
                f"允许在对账期间为 {order.client_order_id!r} 进行超额成交： "
                f"订单数量={order.quantity}，已成交数量={order.filled_qty}， "
                f"成交数量={report.last_qty}，将导致成交总量={potential_filled_qty}",
            )

        # 在应用之前，验证总成交的一致性
        current_total = sum(
            event.last_qty for event in order.events if isinstance(event, OrderFilled)
        )
        if current_total != order.filled_qty:
            self._log.error(
                f"应用成交前检测到不一致： "
                f"{order.client_order_id} 的成交总和 sum(fills)={current_total} != 已成交数量 order.filled_qty={order.filled_qty}",
            )

        # 最终检查：在生成成交前确保 trade_id 不存在
        # 这防止了在 _apply_event_to_order 中抛出 KeyError
        existing_fill = get_existing_fill_for_trade_id(order, report.trade_id)
        if report.trade_id in order.trade_ids or existing_fill is not None:
            self._log.debug(
                f"订单 {order.client_order_id} 已存在 trade_id 为 {report.trade_id} 的成交，跳过重复项",
            )
            return True  # 成交已存在，视为成功

        # 在生成成交之前跟踪审计路径中的成交应用
        # 如果此成交平掉了订单，这确保了关闭时的清理操作仍然有效
        if order.client_order_id not in self._fill_application_audit:
            self._fill_application_audit[order.client_order_id] = []

        audit_entry = (report.trade_id, "reconciliation", self._clock.timestamp_ns())
        self._fill_application_audit[order.client_order_id].append(audit_entry)

        try:
            self._generate_order_filled(order, report, instrument)
        except InvalidStateTrigger as e:
            self._rollback_fill_audit_entry(order.client_order_id, audit_entry)
            self._log.error(str(e))
            return False
        except ValueError as e:
            self._rollback_fill_audit_entry(order.client_order_id, audit_entry)
            # 处理负 leaves_qty 错误
            self._log.exception(
                f"向 {order.client_order_id!r} 应用成交时出现 ValueError：{e}",
                e,
            )
            return False

        # 检查成交的顺序是否正确
        if report.ts_event < order.ts_last:
            self._log.warning(
                f"来自 {report} 的 OrderFilled 未按时间顺序应用",
            )
        return True

    def _check_and_skip_duplicate_fill(
        self,
        order: Order,
        report: FillReport,
    ) -> bool | None:
        # 检查此成交是否早于推断的对账成交
        # 这防止了历史成交被应用在推断成交之上
        client_order_id = order.client_order_id
        if client_order_id in self._inferred_fill_ts:
            earliest_inferred_ts = self._inferred_fill_ts[client_order_id]
            if report.ts_event < earliest_inferred_ts:
                self._log.debug(
                    f"跳过 {client_order_id!r} 的历史成交 {report.trade_id}（ts_event={report.ts_event}）， "
                    f"因为它早于推断的对账成交（ts={earliest_inferred_ts}）； "
                    "推断成交中已包含此成交",
                )
                return True  # 跳过此成交，推断成交中已包含它

        # 检查 trade_id 是否重复 - 同时检查 trade_ids 集合和事件
        # 这处理了从缓存加载订单且 trade_ids 可能未被完全填充的情况
        existing_fill = get_existing_fill_for_trade_id(order, report.trade_id)
        if report.trade_id in order.trade_ids or existing_fill is not None:
            # 成交已应用；检查数据是否一致
            # 现有的成交可能在启动时源自缓存，
            # 或者在触发对账时已存在于内存中
            # 记录关于其首次应用时间的详细信息
            if order.client_order_id in self._fill_application_audit:
                audit = self._fill_application_audit[order.client_order_id]
                previous = [a for a in audit if a[0] == report.trade_id]
                if previous:
                    self._log.debug(
                        f"检测到重复成交；{report.trade_id} 已在 "
                        f"ts={previous[0][2]} 应用，来源为 {previous[0][1]}",
                    )

            if existing_fill and not self._fill_reports_equal(existing_fill, report):
                differences: list[str] = []

                # 成交数量
                if existing_fill.last_qty != report.last_qty:
                    differences.append(f"qty: {existing_fill.last_qty} vs {report.last_qty}")

                # 成交价格
                if existing_fill.last_px != report.last_px:
                    differences.append(f"px: {existing_fill.last_px} vs {report.last_px}")

                # 手续费
                if existing_fill.commission is None and report.commission is not None:
                    differences.append(f"commission: None vs {report.commission}")
                elif existing_fill.commission is not None and report.commission is None:
                    differences.append(f"commission: {existing_fill.commission} vs None")
                elif existing_fill.commission is not None and report.commission is not None:
                    if existing_fill.commission.currency != report.commission.currency:
                        differences.append(
                            f"commission currency: {existing_fill.commission.currency} vs {report.commission.currency}",
                        )
                    elif existing_fill.commission != report.commission:
                        differences.append(
                            f"commission: {existing_fill.commission} vs {report.commission}",
                        )

                # 流动性方向
                if existing_fill.liquidity_side != report.liquidity_side:
                    differences.append(
                        f"liquidity: {existing_fill.liquidity_side} vs {report.liquidity_side}",
                    )

                # 时间戳
                if existing_fill.ts_event != report.ts_event:
                    differences.append(
                        f"ts_event: {existing_fill.ts_event} vs {report.ts_event}",
                    )

                self._log.warning(
                    f"trade_id {report.trade_id} 的成交报告数据与现有数据不同， "
                    f"差异如下：{', '.join(differences)}；为保持一致性，保留缓存数据",
                )

            # 如果 trade_id 在 order.trade_ids 中，或者我们找到了现有的成交，跳过此成交
            # 这防止了重复应用成交
            return True  # 成交已应用，继续使用现有数据

        return None  # 不是重复成交，继续处理

    def _generate_inferred_fill(
        self,
        order: Order,
        report: OrderStatusReport,
        instrument: Instrument,
    ) -> OrderFilled:
        filled = create_inferred_order_filled_event(
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
            instrument=instrument,
        )
        self._log.info(f"已生成推断成交 {filled}", LogColor.BLUE)

        return filled

    # -- 订单和事件生成 -----------------------------------------------------------------------------

    def _generate_order(
        self,
        report: OrderStatusReport,
        is_external: bool = True,
    ) -> Order | None:
        self._log.debug(f"正在生成订单 {report.client_order_id!r}", color=LogColor.MAGENTA)

        options: dict[str, Any] = {}

        if report.price is not None:
            options["price"] = str(report.price)

        if report.trigger_price is not None:
            options["trigger_price"] = str(report.trigger_price)

        if report.trigger_type is not None:
            options["trigger_type"] = trigger_type_to_str(report.trigger_type)

        if report.limit_offset is not None:
            options["limit_offset"] = str(report.limit_offset)
            options["trailing_offset_type"] = trailing_offset_type_to_str(
                report.trailing_offset_type,
            )

        if report.trailing_offset is not None:
            options["trailing_offset"] = str(report.trailing_offset)
            options["trailing_offset_type"] = trailing_offset_type_to_str(
                report.trailing_offset_type,
            )

        if report.display_qty is not None:
            options["display_qty"] = str(report.display_qty)

        options["expire_time_ns"] = (
            0 if report.expire_time is None else dt_to_unix_nanos(report.expire_time)
        )

        # 检查是否有任何策略已认领此交易标的的外部订单
        # 这允许策略在重启时恢复管理现有订单
        strategy_id = self.get_external_order_claim(report.instrument_id)

        if strategy_id is None:
            # 所有的未认领对账使用 EXTERNAL 策略 ID
            # 标签用于区分过滤用途的来源
            strategy_id = StrategyId("EXTERNAL")
            if is_external:
                # 在场地上发现的实际外部订单
                tags = ["VENUE"]
            else:
                # 内部持仓差异对齐（合成成交）
                tags = ["RECONCILIATION"]
        else:
            # 通过 external_order_claims 配置被策略认领的外部订单
            # 此订单将由认领策略管理
            tags = None
            self._log.info(
                f"{report.instrument_id} 的外部订单 {report.client_order_id} "
                f"被策略 {strategy_id} 认领",
                LogColor.BLUE,
            )

        # 过滤未认领的外部订单（但不对账成交）
        if self.filter_unclaimed_external_orders and tags and "VENUE" in tags:
            self._filtered_external_orders_count += 1

            if self._filtered_external_orders_count == 1:
                self._log.warning("正在过滤未认领的外部（EXTERNAL）订单", LogColor.BLUE)

            return None  # 不再进行进一步对账

        initialized = OrderInitialized(
            trader_id=self.trader_id,
            strategy_id=strategy_id,
            instrument_id=report.instrument_id,
            client_order_id=report.client_order_id,
            order_side=report.order_side,
            order_type=report.order_type,
            quantity=report.quantity,
            time_in_force=report.time_in_force,
            post_only=report.post_only,
            reduce_only=report.reduce_only,
            quote_quantity=False,
            options=options,
            emulation_trigger=TriggerType.NO_TRIGGER,
            trigger_instrument_id=None,
            contingency_type=report.contingency_type,
            order_list_id=report.order_list_id,
            linked_order_ids=report.linked_order_ids,
            parent_order_id=report.parent_order_id,
            exec_algorithm_id=None,
            exec_algorithm_params=None,
            exec_spawn_id=None,
            tags=tags,
            event_id=UUID4(),
            ts_init=self._clock.timestamp_ns(),
            reconciliation=True,
        )

        order: Order = OrderUnpacker.from_init(initialized)
        self._log.debug(f"已生成 {initialized}")

        return order

    def _generate_order_rejected(self, order: Order, report: OrderStatusReport) -> None:
        rejected = create_order_rejected_event(
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {rejected}")
        self._handle_event_with_tracking(rejected)

    def _generate_order_accepted(self, order: Order, report: OrderStatusReport) -> None:
        # 当订单转换为已接受（ACCEPTED）时，清除所有重试计数
        self._clear_recon_tracking(order.client_order_id)

        # 同时也尝试通过柜台订单 ID 映射清除
        if report.venue_order_id:
            mapped_client_id = self._cache.client_order_id(report.venue_order_id)
            if mapped_client_id:
                self._clear_recon_tracking(mapped_client_id)

        accepted = create_order_accepted_event(
            trader_id=self.trader_id,
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {accepted}")
        self._handle_event_with_tracking(accepted)

    def _generate_order_triggered(self, order: Order, report: OrderStatusReport) -> None:
        triggered = create_order_triggered_event(
            trader_id=self.trader_id,
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {triggered}")
        self._handle_event_with_tracking(triggered)

    def _generate_order_updated(self, order: Order, report: OrderStatusReport) -> None:
        updated = create_order_updated_event(
            trader_id=self.trader_id,
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {updated}")
        self._handle_event_with_tracking(updated)

    def _generate_order_canceled(self, order: Order, report: OrderStatusReport) -> None:
        canceled = create_order_canceled_event(
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {canceled}")
        self._handle_event_with_tracking(canceled)

    def _generate_order_expired(self, order: Order, report: OrderStatusReport) -> None:
        expired = create_order_expired_event(
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
        )
        self._log.debug(f"已生成 {expired}")
        self._handle_event_with_tracking(expired)

    def _generate_order_filled(
        self,
        order: Order,
        report: FillReport,
        instrument: Instrument,
    ) -> None:
        filled = create_order_filled_event(
            order=order,
            ts_now=self._clock.timestamp_ns(),
            report=report,
            instrument=instrument,
        )
        self._log.debug(f"已生成 {filled}")
        self._handle_event_with_tracking(filled)

    # -- 内部 ---------------------------------------------------------------------------------------

    def _clear_recon_tracking(
        self,
        client_order_id: ClientOrderId,
        *,
        drop_last_query: bool = True,
    ) -> None:
        self._recon_check_retries.pop(client_order_id, None)

        if drop_last_query:
            self._ts_last_query.pop(client_order_id, None)

    def _handle_event_with_tracking(self, event: OrderEvent) -> None:
        # 处理带有活动追踪的订单事件，在缓存中记录成交并
        # 为已关闭订单清理追踪数据
        self._record_local_activity(event)

        if isinstance(event, OrderFilled):
            self._recent_fills_cache[event.trade_id] = self._clock.timestamp_ns()
            self._position_local_activity_ns[event.instrument_id] = event.ts_event

            # 追踪推断成交的时间戳，以防止重复的历史成交
            if event.reconciliation:
                client_order_id = event.client_order_id
                if client_order_id not in self._inferred_fill_ts:
                    self._inferred_fill_ts[client_order_id] = event.ts_event

        self._handle_event(event)

        if event.client_order_id is None:
            return

        order = self._cache.order(event.client_order_id)
        if order and order.is_closed:
            self._clear_recon_tracking(order.client_order_id)
            self._order_local_activity_ns.pop(order.client_order_id, None)
            self._inferred_fill_ts.pop(order.client_order_id, None)
            self._fill_application_audit.pop(order.client_order_id, None)

    def _record_local_activity(self, event: OrderEvent | None) -> None:
        if event is None:
            return

        client_order_id = event.client_order_id
        if client_order_id is None:
            return

        # 使用接收时间（当前时钟时间）而不是场地时间（ts_event）
        # 以准确追踪该订单最后一次处理活动的时间。
        # 这避免了由于网络/队列延迟导致事件到达时看起来像“旧”的竞态条件。
        self._order_local_activity_ns[client_order_id] = self._clock.timestamp_ns()

    def _find_matching_cached_order(
        self,
        instrument_id: InstrumentId,
        order_side: OrderSide,
        quantity: Quantity,
        price: Price | None,
        avg_px: Decimal | None,
    ) -> Order | None:
        # 在缓存中搜索匹配对账参数的现有订单
        cached_orders = self._cache.orders(
            instrument_id=instrument_id,
            venue=None,
            side=order_side,
        )

        for cached_order in cached_orders:
            # 检查订单是否已成交并匹配参数
            if cached_order.status != OrderStatus.FILLED:
                continue

            # 匹配数量
            if cached_order.filled_qty != quantity:
                continue

            # 如果提供了价格，则匹配价格（市价单没有价格）
            if price is not None and cached_order.has_price and cached_order.price != price:
                continue

            # 如果提供了 avg_px，则进行匹配
            if avg_px is not None and cached_order.avg_px is not None:
                cached_avg_px = Decimal(str(cached_order.avg_px))
                if cached_avg_px != avg_px:
                    continue

            # 找到匹配项
            return cached_order

        return None

    def _find_order_by_venue_order_id(
        self,
        venue_order_id: VenueOrderId,
        instrument_id: InstrumentId,
        order_side: OrderSide | None = None,
    ) -> Order | None:
        # 当 venue_order_id 索引未建立时的回退搜索
        cached_orders = self._cache.orders(
            venue=instrument_id.venue,
            instrument_id=instrument_id,
            side=order_side,
        )

        for cached_order in cached_orders:
            if cached_order.venue_order_id == venue_order_id:
                return cached_order

        return None
