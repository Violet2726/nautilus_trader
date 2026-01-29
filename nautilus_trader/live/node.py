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
import signal
import time
from collections.abc import Callable
from datetime import timedelta

from nautilus_trader.cache.base import CacheFacade
from nautilus_trader.common.component import Logger
from nautilus_trader.common.enums import LogColor
from nautilus_trader.common.functions import get_event_loop
from nautilus_trader.config import TradingNodeConfig
from nautilus_trader.core import nautilus_pyo3
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.live.factories import LiveDataClientFactory
from nautilus_trader.live.factories import LiveExecClientFactory
from nautilus_trader.live.node_builder import TradingNodeBuilder
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.portfolio.base import PortfolioFacade
from nautilus_trader.system.kernel import NautilusKernel
from nautilus_trader.trading.trader import Trader


class TradingNode:
    """
    提供一个用于实盘交易的异步网络节点。

    参数
    ----------
    config : TradingNodeConfig, 可选
        实例的配置。
    loop : asyncio.AbstractEventLoop, 可选
        节点的事件循环。
        如果为 ``None``，则会在内部获取运行中的事件循环。

    """

    def __init__(
        self,
        config: TradingNodeConfig | None = None,
        loop: asyncio.AbstractEventLoop | None = None,
    ) -> None:
        if config is None:
            config = TradingNodeConfig()
        PyCondition.not_none(config, "config")
        PyCondition.type(config, TradingNodeConfig, "config")

        self._config: TradingNodeConfig = config

        if loop is None:
            try:
                loop = asyncio.get_event_loop()
            except RuntimeError:
                loop = get_event_loop()

        self.kernel = NautilusKernel(
            name=type(self).__name__,
            config=config,
            loop=loop,
            loop_sig_callback=self._loop_sig_handler,
        )

        self._builder = TradingNodeBuilder(
            loop=loop,
            data_engine=self.kernel.data_engine,
            exec_engine=self.kernel.exec_engine,
            portfolio=self.kernel.portfolio,
            msgbus=self.kernel.msgbus,
            cache=self.kernel.cache,
            clock=self.kernel.clock,
            logger=self.kernel.logger,
        )

        has_cache_backing = bool(config.cache and config.cache.database)
        has_msgbus_backing = bool(config.message_bus and config.message_bus.database)
        self.kernel.logger.info(f"{has_cache_backing=}", LogColor.BLUE)
        self.kernel.logger.info(f"{has_msgbus_backing=}", LogColor.BLUE)

        self._stream_processors: list[Callable] = []

        # 异步任务
        self._task_streaming: asyncio.Future | None = None

        # 状态标志
        self._is_built = False

    @property
    def trader_id(self) -> TraderId:
        """
        返回节点的交易员 ID。

        返回
        -------
        TraderId

        """
        return self.kernel.trader_id

    @property
    def machine_id(self) -> str:
        """
        返回节点的机器 ID。

        返回
        -------
        str

        """
        return self.kernel.machine_id

    @property
    def instance_id(self) -> UUID4:
        """
        返回节点的实例 ID。

        返回
        -------
        UUID4

        """
        return self.kernel.instance_id

    @property
    def trader(self) -> Trader:
        """
        返回节点内部的交易员对象。

        返回
        -------
        Trader

        """
        return self.kernel.trader

    @property
    def cache(self) -> CacheFacade:
        """
        返回节点内部的只读缓存（facade）。

        返回
        -------
        CacheFacade

        """
        return self.kernel.cache

    @property
    def portfolio(self) -> PortfolioFacade:
        """
        返回节点内部的只读投资组合（facade）。

        返回
        -------
        PortfolioFacade

        """
        return self.kernel.portfolio

    def is_running(self) -> bool:
        """
        返回交易节点是否正在运行。

        返回
        -------
        bool

        """
        return self.kernel.is_running()

    def is_built(self) -> bool:
        """
        返回交易节点的客户端是否已构建。

        返回
        -------
        bool

        """
        return self._is_built

    def get_event_loop(self) -> asyncio.AbstractEventLoop | None:
        """
        返回交易节点的事件循环。

        返回
        -------
        asyncio.AbstractEventLoop 或 ``None``

        """
        return self.kernel.loop

    def get_logger(self) -> Logger:
        """
        返回交易节点的日志记录器（logger）。

        返回
        -------
        Logger

        """
        return self.kernel.logger

    def add_stream_processor(self, callback: Callable) -> None:
        """
        添加给定的流处理器回调。

        参数
        ----------
        callback : Callable
            要添加的回调函数。

        """
        self._stream_processors.append(callback)

    def add_data_client_factory(self, name: str, factory: type[LiveDataClientFactory]) -> None:
        """
        向节点添加给定的数据客户端工厂。

        参数
        ----------
        name : str
            客户端工厂的名称。
        factory : type[LiveDataClientFactory]
            要添加的工厂类。

        异常
        ------
        ValueError
            如果 `name` 不是有效的字符串。
        KeyError
            如果 `name` 已经添加过。

        """
        self._builder.add_data_client_factory(name, factory)

    def add_exec_client_factory(self, name: str, factory: type[LiveExecClientFactory]) -> None:
        """
        向节点添加给定的执行客户端工厂。

        参数
        ----------
        name : str
            客户端工厂的名称。
        factory : type[LiveExecutionClientFactory]
            要添加的工厂类。

        异常
        ------
        ValueError
            如果 `name` 不是有效的字符串。
        KeyError
            如果 `name` 已经添加过。

        """
        self._builder.add_exec_client_factory(name, factory)

    def build(self) -> None:
        """
        构建节点客户端。
        """
        if self._is_built:
            raise RuntimeError("交易节点的客户端已经构建。")

        self._builder.build_data_clients(self._config.data_clients)
        self._builder.build_exec_clients(self._config.exec_clients)
        self._is_built = True

    def run(self, raise_exception: bool = False) -> None:
        """
        启动并运行交易节点。

        参数
        ----------
        raise_exception : bool, 默认 False
            是否在记录日志的同时重新抛出运行时异常。

        """
        try:
            if self.kernel.loop.is_running():
                task = self.kernel.loop.create_task(self.run_async())
                task.add_done_callback(self._handle_run_task_result)
            else:
                self.kernel.loop.run_until_complete(self.run_async())
        except RuntimeError as e:
            self.kernel.logger.exception("运行出错", e)

            if raise_exception:
                raise e

    def publish_bus_message(self, bus_msg: nautilus_pyo3.BusMessage) -> None:
        """
        在内部消息总线上发布消息。

        注意：消息不会发布到外部。

        参数
        ----------
        bus_msg : nautilus_pyo3.BusMessage
            要发布的消息。

        """
        try:
            msg = self.kernel.msgbus_serializer.deserialize(bus_msg.payload)
        except Exception as e:
            self.kernel.logger.error(f"反序列化总线消息失败：{e}")
            return

        try:
            for processor in self._stream_processors:
                processor(msg)

            if not self.kernel.msgbus.is_streaming_type(type(msg)):
                return  # 类型尚未注册消息流
        except Exception as e:
            self.kernel.logger.error(f"处理总线消息失败：{e}")
            return

        try:
            self.kernel.msgbus.publish(bus_msg.topic, msg, external_pub=False)
        except Exception as e:
            self.kernel.logger.error(f"发布总线消息失败：{e}")

    async def run_async(self) -> None:
        """
        异步启动并运行交易节点。
        """
        try:
            if not self._is_built:
                raise RuntimeError(
                    "交易节点的客户端尚未构建。 "
                    "启动前请运行 `node.build()`。",
                )

            await self.kernel.start_async()

            if self.kernel.loop.is_running():
                self.kernel.logger.info("运行中 (RUNNING)")
            else:
                self.kernel.logger.warning("事件循环（event loop）未运行")

            # 在引擎运行时持续运行...
            tasks: list[asyncio.Task] = [
                self.kernel.data_engine.get_cmd_queue_task(),
                self.kernel.data_engine.get_req_queue_task(),
                self.kernel.data_engine.get_res_queue_task(),
                self.kernel.data_engine.get_data_queue_task(),
                self.kernel.risk_engine.get_cmd_queue_task(),
                self.kernel.risk_engine.get_evt_queue_task(),
                self.kernel.exec_engine.get_cmd_queue_task(),
                self.kernel.exec_engine.get_evt_queue_task(),
            ]

            if self._config.message_bus and self._config.message_bus.external_streams:
                streams = self._config.message_bus.external_streams
                self.kernel.logger.info("正在启动任务：外部消息流处理", LogColor.BLUE)
                self.kernel.logger.info(f"正在监听流：{streams}", LogColor.BLUE)
                self._task_streaming = asyncio.ensure_future(
                    self.kernel.msgbus_database.stream(self.publish_bus_message),
                )
                self._task_streaming.add_done_callback(self._handle_streaming_exception)

            await asyncio.gather(*tasks)
        except asyncio.CancelledError as e:
            self.kernel.logger.error(str(e))

    def stop(self) -> None:
        """
        优雅地停止交易节点。

        在指定的延迟之后，将检查内部 `Trader` 的残留状态。

        如果配置了保存策略，则会保存策略状态。

        """
        try:
            if self.kernel.loop.is_running():
                self.kernel.loop.create_task(self.stop_async())
            else:
                self.kernel.loop.run_until_complete(self.stop_async())
        except RuntimeError as e:
            self.kernel.logger.exception("停止出错", e)

    async def stop_async(self) -> None:
        """
        异步地优雅停止交易节点。

        在指定的延迟之后，将检查内部 `Trader` 的残留状态。

        如果配置了保存策略，则会保存策略状态。

        """
        await self.kernel.stop_async()

    def dispose(self) -> None:
        """
        清理并注销交易节点。

        优雅地关闭执行器和事件循环。

        """
        try:
            timeout = self.kernel.clock.utc_now() + timedelta(
                seconds=self._config.timeout_disconnection,
            )

            while self.kernel.is_running():
                time.sleep(0.1)

                if self.kernel.clock.utc_now() >= timeout:
                    self.kernel.logger.warning(
                        f"等待节点停止超时（{self._config.timeout_disconnection}s）"
                        f"\n状态"
                        f"\n------"
                        f"\nDataEngine.check_disconnected() == {self.kernel.data_engine.check_disconnected()}"
                        f"\nExecEngine.check_disconnected() == {self.kernel.exec_engine.check_disconnected()}",
                    )
                    break

            self.kernel.logger.debug("正在注销 (DISPOSING)")

            if self._task_streaming:
                self.kernel.logger.info("正在取消任务 'streaming'")
                self._task_streaming.cancel()
                self._task_streaming = None

            self.kernel.logger.debug(str(self.kernel.data_engine.get_cmd_queue_task()))
            self.kernel.logger.debug(str(self.kernel.data_engine.get_req_queue_task()))
            self.kernel.logger.debug(str(self.kernel.data_engine.get_res_queue_task()))
            self.kernel.logger.debug(str(self.kernel.data_engine.get_data_queue_task()))
            self.kernel.logger.debug(str(self.kernel.exec_engine.get_cmd_queue_task()))
            self.kernel.logger.debug(str(self.kernel.exec_engine.get_evt_queue_task()))
            self.kernel.logger.debug(str(self.kernel.risk_engine.get_cmd_queue_task()))
            self.kernel.logger.debug(str(self.kernel.risk_engine.get_evt_queue_task()))

            self.kernel.dispose()

            if self.kernel.executor:
                self.kernel.logger.info("正在关闭执行器 (executor)")
                self.kernel.executor.shutdown(wait=True, cancel_futures=True)

            loop = self.kernel.loop

            if not loop.is_closed():
                if loop.is_running():
                    self.kernel.logger.info("正在停止事件循环")
                    self.kernel.cancel_all_tasks()
                    loop.stop()
                else:
                    self.kernel.logger.info("正在关闭事件循环")
                    loop.close()
            else:
                self.kernel.logger.info("事件循环已关闭（asyncio.run 的正常情况）")
        except (asyncio.CancelledError, RuntimeError) as e:
            self.kernel.logger.exception("注销出错", e)
        finally:
            self.kernel.logger.info(f"loop.is_running={self.kernel.loop.is_running()}")
            self.kernel.logger.info(f"loop.is_closed={self.kernel.loop.is_closed()}")
            self.kernel.logger.info("已注销 (DISPOSED)")

    def _handle_run_task_result(self, task: asyncio.Task) -> None:
        try:
            task.result()
        except asyncio.CancelledError:
            return  # Normal control flow
        except BaseException as e:
            self.kernel.logger.exception("run_async 任务出错", e)

    def _handle_streaming_exception(self, task: asyncio.Future) -> None:
        try:
            task.result()
        except asyncio.CancelledError:
            return  # Normal control flow
        except BaseException as e:
            self.kernel.logger.exception("外部消息流任务出错", e)

    def _loop_sig_handler(self, sig: signal.Signals) -> None:
        self.kernel.logger.warning(f"收到 {sig.name}，正在关机")
        self.stop()
