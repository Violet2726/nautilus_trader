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

from nautilus_trader.adapters.interactive_brokers.client import InteractiveBrokersClient
from nautilus_trader.adapters.interactive_brokers.common import IB_VENUE
from nautilus_trader.adapters.interactive_brokers.config import DockerizedIBGatewayConfig
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersDataClientConfig
from nautilus_trader.adapters.interactive_brokers.config import InteractiveBrokersExecClientConfig
from nautilus_trader.adapters.interactive_brokers.config import (
    InteractiveBrokersInstrumentProviderConfig,
)
from nautilus_trader.adapters.interactive_brokers.data import InteractiveBrokersDataClient
from nautilus_trader.adapters.interactive_brokers.execution import InteractiveBrokersExecutionClient
from nautilus_trader.adapters.interactive_brokers.gateway import DockerizedIBGateway
from nautilus_trader.adapters.interactive_brokers.providers import (
    InteractiveBrokersInstrumentProvider,
)
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common.component import LiveClock
from nautilus_trader.common.component import MessageBus
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.live.factories import LiveDataClientFactory
from nautilus_trader.live.factories import LiveExecClientFactory
from nautilus_trader.model.identifiers import AccountId


GATEWAYS: dict[tuple, DockerizedIBGateway] = {}
IB_CLIENTS: dict[tuple, InteractiveBrokersClient] = {}
IB_INSTRUMENT_PROVIDERS: dict[tuple, InteractiveBrokersInstrumentProvider] = {}


def get_cached_ib_client(
    loop: asyncio.AbstractEventLoop,
    msgbus: MessageBus,
    cache: Cache,
    clock: LiveClock,
    host: str = "127.0.0.1",
    port: int | None = None,
    client_id: int = 1,
    dockerized_gateway: DockerizedIBGatewayConfig | None = None,
    fetch_all_open_orders: bool = False,
    request_timeout_secs: int = 60,
) -> InteractiveBrokersClient:
    """
    根据提供的键获取或创建一个缓存的 InteractiveBrokersClient。

    如果缓存中已存在具有相应键的客户端，该函数将返回该实例。注意，该键由 host、port 
    和 client_id 组合而成。

    当使用 DockerizedIBGatewayConfig 时，可以根据 trading_mode 创建并缓存多个网关。

    参数
    ----------
    loop: asyncio.AbstractEventLoop,
        事件循环
    msgbus: MessageBus,
        消息总线
    cache: Cache,
        缓存
    clock: LiveClock,
        时钟
    host: str
        要连接的 IB 主机地址。如果使用 DockerizedIBGatewayConfig，此参数可选，否则必填。
    port: int
        要连接的 IB 端口。如果使用 DockerizedIBGatewayConfig，此参数可选，否则必填。
    client_id: int
        TWS 或 Gateway 的唯一会话标识符。单个主机可以支持多个连接，但每个连接必须使用不同的 client_id。
    dockerized_gateway: DockerizedIBGatewayConfig, 可选
        Docker 化网关的配置。如果提供此参数，Nautilus 将监管 Docker 环境，并在其中运行 
        IB Gateway。可以根据 trading_mode 创建多个网关。
    fetch_all_open_orders : bool, 默认 False
        如果为 True，使用 reqAllOpenOrders 从所有 API 客户端和 TWS GUI 获取订单。
        如果为 False，使用 reqOpenOrders 仅获取当前客户端 ID 会话的订单。
    request_timeout_secs : int, 默认 60
        等待请求响应（合同详情等）的超时时间（秒）。

    返回
    -------
    InteractiveBrokersClient

    """
    if dockerized_gateway:
        PyCondition.equal(host, "127.0.0.1", "host", "127.0.0.1")
        PyCondition.none(port, "确保在使用 DockerizedIBGatewayConfig 时将 `port` 设置为 None。")

        # 根据 trading_mode 为网关创建一个唯一的键
        gateway_key = (dockerized_gateway.trading_mode,)

        if gateway_key not in GATEWAYS:
            gateway = DockerizedIBGateway(dockerized_gateway)
            gateway.safe_start(wait=dockerized_gateway.timeout)
            GATEWAYS[gateway_key] = gateway
            port = gateway.port
        else:
            port = GATEWAYS[gateway_key].port
    else:
        PyCondition.not_none(
            host,
            "请提供 IB TWS 或 Gateway 的 `host` IP 地址。",
        )
        PyCondition.not_none(port, "请提供 IB TWS 或 Gateway 的 `port` 端口。")

    client_key: tuple = (host, port, client_id)

    if client_key not in IB_CLIENTS:
        client = InteractiveBrokersClient(
            loop=loop,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            host=host,
            port=port,
            client_id=client_id,
            fetch_all_open_orders=fetch_all_open_orders,
            request_timeout_secs=request_timeout_secs,
        )
        client.start()
        IB_CLIENTS[client_key] = client
    elif fetch_all_open_orders:
        # 如果有要求，将现有客户端升级为获取所有未结订单
        # 这处理了先创建行情客户端（未设置标志），后创建执行客户端（设置了标志）的情况
        IB_CLIENTS[client_key]._fetch_all_open_orders = True

    return IB_CLIENTS[client_key]


def get_cached_interactive_brokers_instrument_provider(
    client: InteractiveBrokersClient,
    clock: LiveClock,
    config: InteractiveBrokersInstrumentProviderConfig,
) -> InteractiveBrokersInstrumentProvider:
    """
    缓存并返回一个 InteractiveBrokersInstrumentProvider。

    如果已存在缓存的提供者，则返回该缓存的提供者。
    缓存键基于客户端连接参数和配置哈希。

    参数
    ----------
    client : InteractiveBrokersClient
        工具提供者的客户端。
    clock : LiveClock
        提供者的时钟。
    config: InteractiveBrokersInstrumentProviderConfig
        工具提供者配置

    返回
    -------
    InteractiveBrokersInstrumentProvider

    """
    global IB_INSTRUMENT_PROVIDERS

    # 基于客户端连接信息和配置创建一个缓存键
    # 我们使用客户端的连接参数而不是客户端对象本身，
    # 以确保连接相同的不同客户端实例之间具有一致的缓存。
    client_key = (client._host, client._port, client._client_id)
    provider_key = (client_key, hash(config))

    if provider_key not in IB_INSTRUMENT_PROVIDERS:
        provider = InteractiveBrokersInstrumentProvider(client=client, clock=clock, config=config)
        IB_INSTRUMENT_PROVIDERS[provider_key] = provider

    return IB_INSTRUMENT_PROVIDERS[provider_key]


class InteractiveBrokersLiveDataClientFactory(LiveDataClientFactory):
    """
    提供 InteractiveBrokers 实时行情客户端工厂。
    """

    @staticmethod
    def create(  # type: ignore
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: InteractiveBrokersDataClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> InteractiveBrokersDataClient:
        """
        创建一个新的 InteractiveBrokers 行情客户端。

        参数
        ----------
        loop : asyncio.AbstractEventLoop
            客户端的事件循环。
        name : str
            自定义客户端 ID。
        config : dict
            配置字典。
        msgbus : MessageBus
            客户端的消息总线。
        cache : Cache
            客户端的缓存。
        clock : LiveClock
            客户端的时钟。

        返回
        -------
        InteractiveBrokersDataClient

        """
        client = get_cached_ib_client(
            loop=loop,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            host=config.ibg_host,
            port=config.ibg_port,
            client_id=config.ibg_client_id,
            dockerized_gateway=config.dockerized_gateway,
            request_timeout_secs=config.request_timeout_secs,
        )

        # 获取工具提供者单例
        provider = get_cached_interactive_brokers_instrument_provider(
            client=client,
            clock=clock,
            config=config.instrument_provider,
        )

        # 创建客户端
        data_client = InteractiveBrokersDataClient(
            loop=loop,
            client=client,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            ibg_client_id=config.ibg_client_id,
            config=config,
            name=name,
            connection_timeout=config.connection_timeout,
        )

        return data_client


class InteractiveBrokersLiveExecClientFactory(LiveExecClientFactory):
    """
    提供 InteractiveBrokers 实时执行客户端工厂。
    """

    @staticmethod
    def create(  # type: ignore
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: InteractiveBrokersExecClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> InteractiveBrokersExecutionClient:
        """
        创建一个新的 InteractiveBrokers 执行客户端。

        参数
        ----------
        loop : asyncio.AbstractEventLoop
            客户端的事件循环。
        name : str
            自定义客户端 ID。
        config : dict[str, object]
            客户端配置。
        msgbus : MessageBus
            客户端的消息总线。
        cache : Cache
            客户端的缓存。
        clock : LiveClock
            客户端的时钟。

        返回
        -------
        InteractiveBrokersSpotExecutionClient

        """
        client = get_cached_ib_client(
            loop=loop,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            host=config.ibg_host,
            port=config.ibg_port,
            client_id=config.ibg_client_id,
            dockerized_gateway=config.dockerized_gateway,
            fetch_all_open_orders=config.fetch_all_open_orders,
            request_timeout_secs=config.request_timeout_secs,
        )

        # 获取工具提供者单例
        provider = get_cached_interactive_brokers_instrument_provider(
            client=client,
            clock=clock,
            config=config.instrument_provider,
        )

        # 设置账户 ID
        ib_account = config.account_id or os.environ.get("TWS_ACCOUNT")
        assert ib_account, (
            f"必须传递 `{config.__class__.__name__}.account_id` 或设置 `TWS_ACCOUNT` 环境变量。"
        )

        # 如果提供了名称则使用名称，否则从账户字符串中使用 account_id 的 issuer
        # 这允许具有不同名称/账户的多个 IB 执行客户端
        # account_issuer 将被用作 client_id，并允许按 account_id 进行路由
        account_issuer = name or IB_VENUE.value
        account_id = AccountId(f"{account_issuer}-{ib_account}")

        # 创建客户端
        exec_client = InteractiveBrokersExecutionClient(
            loop=loop,
            client=client,
            account_id=account_id,
            msgbus=msgbus,
            cache=cache,
            clock=clock,
            instrument_provider=provider,
            config=config,
            name=name,
            connection_timeout=config.connection_timeout,
            track_option_exercise_from_position_update=config.track_option_exercise_from_position_update,
        )

        return exec_client
