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

from __future__ import annotations

from enum import Enum
from typing import Literal

from ibapi.common import MarketDataTypeEnum as IBMarketDataTypeEnum

from nautilus_trader.adapters.interactive_brokers.common import IBContract
from nautilus_trader.config import InstrumentProviderConfig
from nautilus_trader.config import LiveDataClientConfig
from nautilus_trader.config import LiveExecClientConfig
from nautilus_trader.config import NautilusConfig


class SymbologyMethod(Enum):
    IB_SIMPLIFIED = "simplified"
    IB_RAW = "raw"


class DockerizedIBGatewayConfig(NautilusConfig, frozen=True):
    """
    在使用容器化安装时，用于 `DockerizedIBGateway` 设置的配置。

    参数
    ----------
    username : str, 可选
        Interactive Brokers 账户用户名。
        如果为 ``None``，将从 `TWS_USERNAME` 环境变量中获取。
    password : str, 可选
        Interactive Brokers 账户密码。
        如果为 ``None``，将从 `TWS_PASSWORD` 环境变量中获取。
    trading_mode: str
        ``paper``（模拟）或 ``live``（实盘）。
    read_only_api: bool, 可选, 默认 True
        如果为 True，则不允许订单执行。设置 read_only_api=False 以允许执行实盘订单。
    timeout: int, 可选
        当 start=True 时，尝试启动 IBG Docker 容器的超时时间（秒）。
    container_image: str, 可选
        IB Gateway 使用的容器镜像引用。
    vnc_port: int | None, 可选, 默认 None
        容器的 VNC 端口。设置为 None 以禁用 VNC 访问。
        VNC 服务器提供对 IB Gateway 界面的远程桌面访问。
        示例：5900, 5901, 5902 等。

    """

    username: str | None = None
    password: str | None = None
    trading_mode: Literal["paper", "live"] = "paper"
    read_only_api: bool = True
    timeout: int = 300
    container_image: str = "ghcr.io/gnzsnz/ib-gateway:stable"
    vnc_port: int | None = None

    def __repr__(self):
        masked_username = self._mask_sensitive_info(self.username)

        return (
            f"DockerizedIBGatewayConfig(username={masked_username}, "
            f"password=********, trading_mode='{self.trading_mode}', "
            f"read_only_api={self.read_only_api}, timeout={self.timeout})"
        )

    @staticmethod
    def _mask_sensitive_info(value: str | None) -> str:
        if value is None:
            return "None"

        return value[0] + "*" * (len(value) - 2) + value[-1] if len(value) > 2 else "*" * len(value)


class InteractiveBrokersInstrumentProviderConfig(InstrumentProviderConfig, frozen=True):
    """
    `InteractiveBrokersInstrumentProvider` 实例的配置。

    指定 `load_ids`、`load_contracts` 或两者，以确定系统启动时加载哪些工具。
    需要注意的是，`InteractiveBrokersInstrumentProviderConfig` 并不限于初始加载的工具。
    工具可以在运行时根据需要动态请求和加载。

    参数
    ----------
    load_all : bool, 默认 False
        注意：InteractiveBrokersInstrumentProvider 不支持加载所有工具。
        因此，此参数不适用。
    load_ids : FrozenSet[InstrumentId], 可选
        应在启动期间加载的 `InstrumentId` 实例的冻结集合。这些代表提供者最初应加载的特定工具。
    load_contracts: FrozenSet[IBContract], 可选
        应在初始启动期间加载的 `IBContract` 对象的冻结集合。这些特定合约对应于提供者预加载的工具。
        需要注意的是，虽然 `load_ids` 选项可用于加载单个工具，但使用 `load_contracts` 
        可以更灵活地加载共享相同底层资产的多个相关工具，如期货和期权。
    symbology_method : SymbologyMethod, 可选
        指定用于识别金融工具的代码方法（symbology format）。可用选项有：
        - IB_RAW：使用 Interactive Brokers 提供的原始代码格式。工具代码遵循详细格式，
        如 `localSymbol=secType.exchange`（例如 `EUR.USD=CASH.IDEALPRO`）。
        虽然此格式可能缺乏视觉清晰度，但它非常稳健，支持来自任何地区的工具，尤其是那些代码不标准、
        简化解析可能会失败的工具。
        - IB_SIMPLIFIED：采用特定于 Interactive Brokers 的简化代码格式，使用交易所缩写。
        工具代码使用更简洁的记号，如 `ESZ28.CME` 或 `EUR/USD.IDEALPRO`。
        此格式优先考虑易读性和可用性，是默认选项。
    build_options_chain: bool (默认: None)
        搜索完整期权链。所有适用工具的全局设置。
    build_futures_chain: bool (默认: None)
        搜索完整期货链。所有适用工具的全局设置。
    min_expiry_days: int (默认: None)
        过滤到期天数不少于指定天数的期权链和期货链。所有适用工具的全局设置。
    max_expiry_days: int (默认: None)
        过滤到期天数不多于指定天数的期权链和期货链。所有适用工具的全局设置。
    convert_exchange_to_mic_venue: bool (默认: False)
        在将 IB 合约转换为工具 ID 时，是否将 IB 交易所转换为 MIC 交易所（MIC venues）。
    symbol_to_mic_venue: dict, 可选
        用于覆盖默认 MIC 交易所转换的字典。
        键是代码前缀（例如 ES 代表其所有期货和期权），值是要使用的 MIC 交易所。
    cache_validity_days: int (默认: None)
        默认为 None，将在 TradingNode 启动时请求新鲜拉取 [仅一次]。
        设置该值将在指定间隔拉取工具，适用于 TradingNode 运行多天的情况。
        示例：值设置为 1，即使 TradingNode 没有重启，InstrumentProvider 也会每天进行新鲜拉取。
    pickle_path: str (默认: None)
        如果提供了有效路径，将把 ContractDetails 存储为 pickle 文件，并在缓存有效期内使用。
    filter_sec_types: FrozenSet[str], 可选
        提供者应忽略的 IB `secType` 值集合。任何 `secType` 与其中一项匹配的合约都将被跳过，
        并在尝试对账之前发出警告。使用此选项可排除尚不支持的资产（例如 `WAR` 或 `IOPT`）。

    """

    def __eq__(self, other: object) -> bool:
        if other is None:
            return False
        if not isinstance(other, InteractiveBrokersInstrumentProviderConfig):
            return False

        return (
            self.load_ids == other.load_ids
            and self.load_contracts == other.load_contracts
            and self.min_expiry_days == other.min_expiry_days
            and self.max_expiry_days == other.max_expiry_days
            and self.build_options_chain == other.build_options_chain
            and self.build_futures_chain == other.build_futures_chain
            and self.filter_sec_types == other.filter_sec_types
        )

    def __hash__(self) -> int:
        return hash(
            (
                self.load_ids,
                self.load_contracts,
                self.build_options_chain,
                self.build_futures_chain,
                self.min_expiry_days,
                self.max_expiry_days,
                self.symbology_method,
                self.convert_exchange_to_mic_venue,
                (
                    tuple(sorted(self.symbol_to_mic_venue.items()))
                    if self.symbol_to_mic_venue
                    else None
                ),
                self.cache_validity_days,
                self.pickle_path,
                self.filter_sec_types,
            ),
        )

    symbology_method: SymbologyMethod = SymbologyMethod.IB_SIMPLIFIED
    load_contracts: frozenset[IBContract] | None = None
    build_options_chain: bool | None = None
    build_futures_chain: bool | None = None
    min_expiry_days: int | None = None
    max_expiry_days: int | None = None
    convert_exchange_to_mic_venue: bool = False
    symbol_to_mic_venue: dict = {}

    cache_validity_days: int | None = None
    pickle_path: str | None = None
    filter_sec_types: frozenset[str] = frozenset()


class InteractiveBrokersDataClientConfig(LiveDataClientConfig, frozen=True):
    """
    ``InteractiveBrokersDataClient`` 实例的配置。

    参数
    ----------
    ibg_host : str, 默认 "127.0.0.1"
        IB Gateway (IBG) 或 Trader Workstation (TWS) 的主机名或 IP 地址。
    ibg_port : int, 默认 None
        网关服务器的端口。（“模拟/实盘”默认值：IBG 4002/4001；TWS 7497/7496）
    ibg_client_id: int, 默认 1
        要传递给连接调用的 client_id。
    use_regular_trading_hours : bool
        如果为 True，将仅请求常规交易时段（RTH）的数据。
        仅适用于 K 线数据 - 对成交或逐笔行情数据流无影响。
        通常用于 'STK' 安全类型。请咨询 InteractiveBrokers 获取 RTH 信息。
    market_data_type : IBMarketDataTypeEnum, 默认 REALTIME
        设置 InteractiveBrokersClient 使用的 IBMarketDataTypeEnum。
        对于没有数据订阅的账户，请配置 `IBMarketDataTypeEnum.DELAYED_FROZEN`。
    ignore_quote_tick_size_updates : bool
        如果设置为 True，QuoteTick 订阅将排除仅大小发生变化而价格未变化的行情更新。
        这有助于减少行情数据量。当设置为 False（默认值）时，QuoteTick 更新将包含所有更新，
        包括仅大小发生变化的更新。
    dockerized_gateway : DockerizedIBGatewayConfig, 可选
        客户端的网关容器配置。
    connection_timeout : int, 默认 300
        等待客户端建立连接的超时时间（秒）。
    request_timeout : int, 默认 60
        等待历史数据响应的超时时间（秒）。

    """

    instrument_provider: InteractiveBrokersInstrumentProviderConfig = (
        InteractiveBrokersInstrumentProviderConfig()
    )

    ibg_host: str = "127.0.0.1"
    ibg_port: int | None = None
    ibg_client_id: int = 1
    use_regular_trading_hours: bool = True
    market_data_type: IBMarketDataTypeEnum = IBMarketDataTypeEnum.REALTIME
    ignore_quote_tick_size_updates: bool = False
    dockerized_gateway: DockerizedIBGatewayConfig | None = None
    connection_timeout: int = 300
    request_timeout: int = 60


class InteractiveBrokersExecClientConfig(LiveExecClientConfig, frozen=True):
    """
    ``InteractiveBrokersExecClient`` 实例的配置。

    参数
    ----------
    ibg_host : str, 默认 "127.0.0.1"
        IB Gateway (IBG) 或 Trader Workstation (TWS) 的主机名或 IP 地址。
    ibg_port : int
        网关服务器的端口。（“模拟/实盘”默认值：IBG 4002/4001；TWS 7497/7496）
    ibg_client_id: int, 默认 1
        要传递给连接调用的 client_id。
    account_id : str
        表示 TWS/Gateway 登录的 Interactive Brokers 账户 ID。
        account_id 必须与 TWS/Gateway 登录的账户一致，这一点至关重要。
        如果 account_id 为 `None`，系统将回退使用环境变量中的 `TWS_ACCOUNT`。
    dockerized_gateway : DockerizedIBGatewayConfig, 可选
        客户端的网关容器配置。
    connection_timeout : int, 默认 300
        等待客户端建立连接的超时时间（秒）。
    fetch_all_open_orders : bool, 默认 False
        如果为 True，使用 reqAllOpenOrders 获取来自所有 API 客户端和 TWS GUI 的订单。
        如果为 False，使用 reqOpenOrders 仅获取来自当前客户端 ID 会话的订单。
        注意：当使用客户端 ID 0 调用 reqAllOpenOrders 时，可以看到来自所有来源
        （包括 TWS GUI）的订单，但看不到来自其他非零客户端 ID 的订单。
    track_option_exercise_from_position_update : bool, 默认 False
        如果为 True，订阅实时持仓更新以跟踪期权行权。

    """

    instrument_provider: InteractiveBrokersInstrumentProviderConfig = (
        InteractiveBrokersInstrumentProviderConfig()
    )
    ibg_host: str = "127.0.0.1"
    ibg_port: int | None = None
    ibg_client_id: int = 1
    account_id: str | None = None
    dockerized_gateway: DockerizedIBGatewayConfig | None = None
    connection_timeout: int = 300
    fetch_all_open_orders: bool = False
    track_option_exercise_from_position_update: bool = False
