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
from nautilus_trader.common.component import MessageBus
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.config import LiveExecClientConfig
from nautilus_trader.live.data_client import LiveDataClient
from nautilus_trader.live.execution_client import LiveExecutionClient


class LiveDataClientFactory:
    """
    提供用于创建 `LiveDataClient` 实例的工厂。
    """

    @staticmethod
    def create(
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: LiveDataClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> LiveDataClient:
        """
        返回一个新的数据客户端。

        参数
        ----------
        loop : asyncio.AbstractEventLoop
            客户端使用的事件循环。
        name : str
            自定义客户端 ID。
        config : dict[str, object]
            客户端的配置。
        msgbus : MessageBus
            客户端的消息总线。
        cache : Cache
            客户端使用的缓存。
        clock : LiveClock
            客户端使用的时钟。

        返回
        -------
        LiveDataClient

        """
        raise NotImplementedError(
            "`create` 方法必须在子类中实现",
        )  # pragma: no cover


class LiveExecClientFactory:
    """
    提供用于创建 `LiveExecutionClient` 实例的工厂。
    """

    @staticmethod
    def create(
        loop: asyncio.AbstractEventLoop,
        name: str,
        config: LiveExecClientConfig,
        msgbus: MessageBus,
        cache: Cache,
        clock: LiveClock,
    ) -> LiveExecutionClient:
        """
        返回一个新的执行客户端。

        参数
        ----------
        loop : asyncio.AbstractEventLoop
            客户端使用的事件循环。
        name : str
            自定义客户端 ID。
        config : dict[str, object]
            客户端的配置。
        msgbus : MessageBus
            客户端的消息总线。
        cache : Cache
            客户端使用的缓存。
        clock : LiveClock
            客户端使用的时钟。

        返回
        -------
        LiveExecutionClient

        """
        raise NotImplementedError(
        )  # pragma: no cover
