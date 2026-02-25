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
import functools

from ibapi import comm
from ibapi import decoder
from ibapi.client import EClient
from ibapi.connection import Connection
from ibapi.const import NO_VALID_ID
from ibapi.errors import CONNECT_FAIL
from ibapi.server_versions import MAX_CLIENT_VER
from ibapi.server_versions import MIN_CLIENT_VER
from ibapi.utils import currentTimeMillis

from nautilus_trader.adapters.interactive_brokers.client.common import BaseMixin
from nautilus_trader.common.enums import LogColor


class InteractiveBrokersClientConnectionMixin(BaseMixin):
    """
    管理 InteractiveBrokersClient 与 TWS/Gateway 的连接。

    该类负责建立和维护套接字连接、处理服务器通信、监控连接健康状况以及管理
    自动重连。当连接建立且客户端完成初始化时，会设置 `_is_ib_connected` 事件；
    如果连接丢失，则会清除 `_is_ib_connected` 事件。

    """

    async def _connect(self) -> None:
        """
        建立与 TWS/Gateway 的套接字连接。

        此方法负责初始化连接参数、连接套接字、发送和接收版本信息，然后设置连接
        已成功建立的标志。

        """
        try:
            self._initialize_connection_params()
            await self._connect_socket()
            self._eclient.setConnState(EClient.CONNECTING)
            await self._send_version_info()
            self._eclient.decoder = decoder.Decoder(
                wrapper=self._eclient.wrapper,
                serverVersion=self._eclient.serverVersion(),
            )
            await self._receive_server_info()
            self._eclient.setConnState(EClient.CONNECTED)
            conn_time_str = self._msgspec_decoding_hook(self._eclient.connTime)
            self._log.info(
                f"已连接到 Interactive Brokers (版本: {self._eclient.serverVersion_})，"
                f"连接时间：{conn_time_str}，地址：{self._host}:{self._port}，"
                f"客户端 ID：{self._client_id}",
            )
        except ConnectionError:
            self._log.error("连接失败")
            if self._eclient.wrapper:
                self._eclient.wrapper.error(
                    NO_VALID_ID,
                    currentTimeMillis(),
                    CONNECT_FAIL.code(),
                    CONNECT_FAIL.msg(),
                )
        except TimeoutError:
            self._log.warning("连接超时")
            if self._eclient.wrapper:
                self._eclient.wrapper.error(
                    NO_VALID_ID,
                    currentTimeMillis(),
                    CONNECT_FAIL.code(),
                    CONNECT_FAIL.msg(),
                )
        except asyncio.CancelledError:
            self._log.info("连接已取消")
        except Exception as e:
            self._log.exception("连接失败", e)
            if self._eclient.wrapper:
                self._eclient.wrapper.error(
                    NO_VALID_ID,
                    currentTimeMillis(),
                    CONNECT_FAIL.code(),
                    CONNECT_FAIL.msg(),
                )

    def _msgspec_decoding_hook(self, byte_data: bytes) -> str:
        """
        对来自服务器的连接时间进行解码。
        """
        for enc in ["utf-8", "gbk", "latin-1", "cp1252"]:
            try:
                return byte_data.decode(enc)
            except UnicodeDecodeError:
                continue
        return byte_data.decode("utf-8", errors="replace")

    async def _disconnect(self) -> None:
        """
        断开与 TWS/Gateway 的连接并清除 `_is_ib_connected` 标志。
        """
        try:
            self._eclient.disconnect()

            if self._is_ib_connected.is_set():
                self._log.debug("在 `_disconnect` 中取消了 `_is_ib_connected` 状态", LogColor.BLUE)
                self._is_ib_connected.clear()

            self._log.info("已断开与 Interactive Brokers API 的连接")
        except Exception as e:
            self._log.exception("断开连接失败", e)

    async def _handle_reconnect(self) -> None:
        """
        尝试重新连接到 TWS/Gateway。
        """
        self._reset()
        self._resume()

    def _initialize_connection_params(self) -> None:
        """
        在尝试连接之前初始化连接参数。

        设置 EClient 实例的主机、端口和客户端 ID，并递增连接尝试计数器。记录
        尝试连接的相关信息。

        """
        self._eclient.reset()
        self._eclient.host = self._host
        self._eclient.port = self._port
        self._eclient.clientId = self._client_id

    async def _connect_socket(self) -> None:
        """
        将套接字连接到 TWS / Gateway，并将连接状态更改为 CONNECTING。

        这是一个在事件循环执行器中运行的异步方法。

        """
        self._eclient.conn = Connection(self._host, self._port)
        self._log.info(
            f"正在连接到 {self._host}:{self._port}，客户端 ID：{self._client_id}",
        )
        await asyncio.to_thread(self._connect_socket_safe)

    def _connect_socket_safe(self) -> None:
        try:
            self._eclient.conn.connect()
        except Exception:
            raise ConnectionError("连接 TWS/Gateway 失败。")

    async def _send_version_info(self) -> None:
        """
        向 TWS / Gateway 发送 API 版本信息。

        构建并发送包含 API 版本前缀和客户端支持的版本范围的消息。这是与服务器
        进行初始握手过程的一部分。

        """
        v100prefix = "API\0"
        v100version = f"v{MIN_CLIENT_VER}..{MAX_CLIENT_VER}"

        if self._eclient.connectOptions:
            v100version += f" {self._eclient.connectOptions}"

        msg = comm.make_initial_msg(v100version)
        msg2 = str.encode(v100prefix, "ascii") + msg
        await asyncio.to_thread(functools.partial(self._eclient.conn.sendMsg, msg2))

    async def _receive_server_info(self) -> None:
        """
        接收并处理服务器版本信息。

        等待服务器发送其版本信息和连接时间。在指定的尝试次数内重试接收此信息。

        引发
        ------
        ConnectionError
            如果在分配的重试次数内未收到服务器版本信息。

        """
        retries_remaining = 5
        fields: list[str] = []

        while retries_remaining > 0:
            buf = await asyncio.to_thread(self._eclient.conn.recvMsg)

            if len(buf) > 0:
                _, msg, _ = comm.read_msg(buf)
                fields.extend(comm.read_fields(msg))
            else:
                self._log.debug("接收到空缓冲区")

            if len(fields) == 2:
                self._process_server_version(fields)
                break

            retries_remaining -= 1
            self._log.warning(
                "获取服务器版本信息失败，"
                f"剩余重试次数：{retries_remaining}",
            )
            await asyncio.sleep(1)

        if retries_remaining == 0:
            raise ConnectionError(
                "已达到最大重试次数。无法接收服务器版本信息。",
            )
            self._log.info("")

    def _process_server_version(self, fields: list[str]) -> None:
        """
        处理并记录服务器版本信息。从接收到的字段中提取并设置服务器版本和连接
        时间。记录服务器版本和连接时间。

        参数
        ----------
        fields : list[str]
            包含服务器版本和连接时间的字段。

        """
        server_version, conn_time = int(fields[0]), fields[1]
        self._eclient.connTime = conn_time
        self._eclient.serverVersion_ = server_version
        self._eclient.decoder.serverVersion = server_version

    def process_connection_closed(self) -> None:
        """
        指示 API 连接已关闭。

        在 API 与 TWS 之间的套接字连接断开后，此函数不会自动调用，而必须由 API
        客户端代码触发。

        """
        for future in self._requests.get_futures():
            if not future.done():
                future.set_exception(ConnectionError("套接字已断开。"))

        if self._is_ib_connected.is_set():
            self._log.debug("由 `connectionClosed` 取消了 `_is_ib_connected` 状态", LogColor.BLUE)
            self._is_ib_connected.clear()
