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

from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import Logger
from nautilus_trader.common.component import MessageBus
from nautilus_trader.config import ImportableConfig
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.config import LiveExecClientConfig
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.live.data_engine import LiveDataEngine
from nautilus_trader.live.execution_engine import LiveExecutionEngine
from nautilus_trader.live.factories import LiveDataClientFactory
from nautilus_trader.live.factories import LiveExecClientFactory
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.portfolio.portfolio import Portfolio


class TradingNodeBuilder:
    """
    提供交易节点的构建服务。

    参数
    ----------
    loop : asyncio.AbstractEventLoop
        客户端使用的事件循环。
    data_engine : LiveDataEngine
        交易节点的数据引擎。
    exec_engine : LiveExecutionEngine
        交易节点的执行引擎。
    portfolio : Portfolio
        交易节点的投资组合。
    msgbus : MessageBus
        交易节点的消息总线。
    cache : Cache
        用于构建客户端的缓存。
    clock : LiveClock
        用于构建客户端的时钟。
    logger : Logger
        用于构建客户端的日志记录器。
    log : Logger
        交易节点的日志记录器。

    """

    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        data_engine: LiveDataEngine,
        exec_engine: LiveExecutionEngine,
        portfolio: Portfolio,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
        logger: Logger,
    ) -> None:
        self._msgbus = msgbus
        self._cache = cache
        self._clock = clock
        self._log = logger

        self._loop = loop
        self._data_engine = data_engine
        self._exec_engine = exec_engine
        self._portfolio = portfolio

        self._data_factories: dict[str, type[LiveDataClientFactory]] = {}
        self._exec_factories: dict[str, type[LiveExecClientFactory]] = {}

    def add_data_client_factory(self, name: str, factory: type[LiveDataClientFactory]) -> None:
        """
    向构建器添加给定的数据客户端工厂。

    参数
    ----------
    name : str
        客户端的名称。
    factory : type[LiveDataClientFactory]
        要添加的工厂。

    异常
    ------
    ValueError
        如果 `name` 不是有效的字符串。
    KeyError
        如果 `name` 已经添加过。

    """
        PyCondition.valid_string(name, "name")
        PyCondition.not_none(factory, "factory")
        PyCondition.not_in(name, self._data_factories, "name", "_data_factories")

        if not issubclass(factory, LiveDataClientFactory):
            self._log.error(f"工厂类型不是 `LiveDataClientFactory`，而是 {factory}")
            return

        self._data_factories[name] = factory

    def add_exec_client_factory(self, name: str, factory: type[LiveExecClientFactory]) -> None:
        """
    向构建器添加给定的执行客户端工厂。

    参数
    ----------
    name : str
        客户端的名称。
    factory : type[LiveExecClientFactory]
        要添加的工厂。

    异常
    ------
    ValueError
        如果 `name` 不是有效的字符串。
    KeyError
        如果 `name` 已经添加过。

    """
        PyCondition.valid_string(name, "name")
        PyCondition.not_none(factory, "factory")
        PyCondition.not_in(name, self._exec_factories, "name", "_exec_factories")

        if not issubclass(factory, LiveExecClientFactory):
            self._log.error(f"工厂类型不是 `LiveExecClientFactory`，而是 {factory}")
            return

        self._exec_factories[name] = factory

    def build_data_clients(
        self,
        config: dict[str, LiveDataClientConfig],
    ) -> None:
        """
    使用给定的配置构建数据客户端。

    参数
    ----------
    config : dict[str, ImportableConfig | LiveDataClientConfig]
        数据客户端的配置。

    """
        PyCondition.not_none(config, "config")

        if not config and not self._data_engine.get_external_client_ids():
            self._log.warning("未发现 `data_clients` 配置")

        for parts, cfg in config.items():
            name = parts.partition("-")[0]
            self._log.info(f"正在为 {name} 构建数据客户端")

            if isinstance(cfg, ImportableConfig):
                if name not in self._data_factories and cfg.factory is not None:
                    self._data_factories[name] = cfg.factory.create()

                client_config: LiveDataClientConfig = cfg.create()
            else:
                client_config: LiveDataClientConfig = cfg  # type: ignore

            if name not in self._data_factories:
                self._log.error(f"未注册用于 {name} 的 `LiveDataClientFactory`")
                continue

            factory = self._data_factories[name]
            client = factory.create(
                loop=self._loop,
                name=name,
                config=client_config,
                msgbus=self._msgbus,
                cache=self._cache,
                clock=self._clock,
            )
            self._data_engine.register_client(client)

            # Default client config
            if client_config.routing.default:
                self._data_engine.register_default_client(client)

            # Venue routing config
            venues: frozenset[str] = client_config.routing.venues or frozenset()

            for venue in venues:
                if not isinstance(venue, Venue):
                    venue = Venue(venue)

                self._data_engine.register_venue_routing(client, venue)

    def build_exec_clients(
        self,
        config: dict[str, LiveExecClientConfig],
    ) -> None:
        """
    使用给定的配置构建执行客户端。

    参数
    ----------
    config : dict[str, ImportableConfig | LiveExecClientConfig]
        执行客户端的配置。

    """
        PyCondition.not_none(config, "config")

        if not config and not self._exec_engine.get_external_client_ids():
            self._log.warning("未发现 `exec_clients` 配置")

        for parts, cfg in config.items():
            name = parts.partition("-")[0]
            self._log.info(f"正在为 {name} 构建执行客户端")

            if isinstance(cfg, ImportableConfig):
                if name not in self._exec_factories and cfg.factory is not None:
                    self._exec_factories[name] = cfg.factory.create()

                client_config: LiveExecClientConfig = cfg.create()
            else:
                client_config: LiveExecClientConfig = cfg  # type: ignore

            if name not in self._exec_factories:
                self._log.error(f"未注册用于 {name} 的 `LiveExecClientFactory`")
                continue

            factory = self._exec_factories[name]

            factory_kws = {
                "loop": self._loop,
                "name": name,
                "config": client_config,
                "msgbus": self._msgbus,
                "cache": self._cache,
                "clock": self._clock,
            }

            if factory.__name__ == "SandboxLiveExecClientFactory":
                factory_kws["portfolio"] = self._portfolio

            client = factory.create(**factory_kws)
            self._exec_engine.register_client(client)

            # Default client config
            if client_config.routing.default:
                self._exec_engine.register_default_client(client)

            # Venue routing config
            venues: frozenset[str] = client_config.routing.venues or frozenset()

            for venue in venues:
                if not isinstance(venue, Venue):
                    venue = Venue(venue)

                self._exec_engine.register_venue_routing(client, venue)
