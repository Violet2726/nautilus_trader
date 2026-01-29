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

import msgspec

from nautilus_trader.common import Environment
from nautilus_trader.common.config import ActorConfig
from nautilus_trader.common.config import InstrumentProviderConfig
from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.config import NonNegativeInt
from nautilus_trader.common.config import PositiveFloat
from nautilus_trader.common.config import PositiveInt
from nautilus_trader.common.config import resolve_config_path
from nautilus_trader.common.config import resolve_path
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.data.config import DataEngineConfig
from nautilus_trader.execution.config import ExecEngineConfig
from nautilus_trader.model.identifiers import ClientOrderId
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.risk.config import RiskEngineConfig
from nautilus_trader.system.config import NautilusKernelConfig
from nautilus_trader.trading.config import ImportableControllerConfig


class LiveDataEngineConfig(DataEngineConfig, frozen=True):
    """
    ``LiveDataEngine`` 实例的配置。

    参数
    ----------
    qsize : PositiveInt, 默认 100_000
        引擎内部队列缓冲区的队列大小。
    graceful_shutdown_on_exception : bool, 默认 False
        当消息队列处理过程中发生意外异常时，系统是否应执行优雅停机（不包括用户 Actor/策略异常）。

    """

    qsize: PositiveInt = 100_000
    graceful_shutdown_on_exception: bool = False


class LiveRiskEngineConfig(RiskEngineConfig, frozen=True):
    """
    ``LiveRiskEngine`` 实例的配置。

    参数
    ----------
    qsize : PositiveInt, 默认 100_000
        引擎内部队列缓冲区的队列大小。
    graceful_shutdown_on_exception : bool, 默认 False
        当消息队列处理过程中发生意外异常时，系统是否应执行优雅停机（不包括用户 Actor/策略异常）。

    """

    qsize: PositiveInt = 100_000
    graceful_shutdown_on_exception: bool = False


class LiveExecEngineConfig(ExecEngineConfig, frozen=True):
    """
    ``LiveExecEngine`` 实例的配置。

    处理中（in-flight）订单检查的目的是为了实盘对账。从场地发出的事件可能在某些时候丢失，
    导致订单处于中间状态，该检查可以通过状态报告恢复这些事件。

    参数
    ----------
    reconciliation : bool, 默认 True
        启动时是否激活执行对账。
    reconciliation_lookback_mins : NonNegativeInt, 可选
        对账执行状态的最大回溯分钟数。
        如果为 ``None`` 或 0，将使用场地提供的最大回溯时间。
    reconciliation_instrument_ids : list[InstrumentId], 可选
        用于执行对账的标的 ID 包含列表。
        如果提供，则仅对这些标的进行对账。
        如果为 ``None`` 或为空，则对所有标的进行对账。
    filter_unclaimed_external_orders : bool, 默认 False
        是否过滤/丢弃策略 ID 为 EXTERNAL 的未认领订单事件。
    filter_position_reports : bool, 默认 False
        是否在对账中过滤掉仓位状态报告。
        当其他节点在同一账户上交易相同标的时，这可能适用，因为这可能导致仓位状态冲突。
    filtered_client_order_ids : list[ClientOrderId], 可选
        要从对账中过滤掉的客户端订单 ID 列表。
    generate_missing_orders : bool, 默认 True
        如果在对账期间内部和外部仓位不一致，是否生成 MARKET 订单事件以对齐差异。
    inflight_check_interval_ms : NonNegativeInt, 默认 2_000
        检查处理中订单是否超过其处理时间阈值的间隔（毫秒）。
        此值不应设置为小于 `inflight_check_threshold_ms`。
    inflight_check_threshold_ms : NonNegativeInt, 默认 5_000
        向场地检查处理中订单状态的阈值（毫秒）。
        作为经验法则，除非你与场地处于同机房（以避免潜在的竞态条件），否则你不应考虑减少此设置。
    inflight_check_retries : NonNegativeInt, 默认 5
        如果初始尝试失败，引擎将为验证处理中订单与场地的状态而进行的重试次数。
    own_books_audit_interval_secs : NonNegativeFloat, 可选
        对所有自有委托单簿（own books）与公共委托单簿进行审计的间隔（秒）。
        审计将确保所有订单状态保持同步，并且没有已关闭的订单残留在自有委托单簿中。所有失败都记录为错误。
    open_check_interval_secs : PositiveFloat, 可选
        检场地上未平仓订单的间隔（秒）。
        如果存在差异，则生成订单状态报告并进行对账。
        建议设置为 5-10 秒，需考虑 API 速率限制和额外的请求权重。如果未指定值，则不启动未平仓订单检查任务。
    open_check_open_only : bool, 默认 True
        如果为 True，**check_open_orders** 请求仅从场地请求当前未完成的订单。
        如果为 False，它请求整个订单历史记录，这可能是一个繁重的 API 调用。
        此参数仅在 **check_open_orders** 任务运行时生效。
    open_check_lookback_mins : PositiveInt, 默认 60
        连续对账期间订单状态轮询的回溯窗口（分钟）。
        仅在此时间窗口内修改的订单才会被考虑进行对账。
    open_check_threshold_ms : NonNegativeInt, 默认 5_000
        未平仓订单检查在根据场地差异（缺失、状态偏移等）采取行动之前，距离订单最后一次缓存事件的最小间隔时间（毫秒）。
    open_check_missing_retries : NonNegativeInt, 默认 5
        在结清缓存中开启但在场地上未找到的订单之前的最大重试次数。这可以防止由于网络延迟或场地处理时间过快而导致的竞态条件。
    max_single_order_queries_per_cycle : PositiveInt, 默认 10
        每个对账周期执行的最大单笔订单查询次数。防止在许多订单批量查询检查失败时耗尽速率限制。
    single_order_query_delay_ms : NonNegativeInt, 默认 100
        单笔订单查询之间的延迟（毫秒），以防止耗尽速率限制。
    position_check_interval_secs : PositiveFloat, 可选
        检查缓存与场地之间仓位差异的间隔（秒）。
        当检测到差异时，系统会查询可能已丢失的成交（fills）。
        建议设置为 30-60 秒。如果未指定值，则不启动仓位检查。
    position_check_lookback_mins : PositiveInt, 默认 60
        检测到仓位差异时查询成交报告的回溯窗口（分钟）。
        将仅从场地请求此窗口内的成交记录。
    position_check_threshold_ms : NonNegativeInt, 默认 5_000
        仓位检查在根据差异采取行动之前，距离仓位最后一次本地活动的最小间隔时间（毫秒）。这可以防止与处理中的成交流生竞态条件。
    reconciliation_startup_delay_secs : PositiveFloat, 默认 10.0
        在启动对账完成后，开始连续对账循环之前的额外延迟（秒）。这为初始对账后的系统进一步稳定提供了时间。
    purge_closed_orders_interval_mins : PositiveInt, 可选
        从内存缓存中清除已关闭订单的间隔（分钟），**不会从数据库中清除**。如果为 None，则已关闭订单将**不会**被自动清除。对于高频交易（HFT），建议设置为 10-15 分钟。
    purge_closed_orders_buffer_mins : NonNegativeInt, 可选
        订单关闭后到可以被清除之间的时间缓冲（分钟）。
        仅关闭至少达到此时间的订单才会被清除。对于高频交易（HFT），建议设置为 60 分钟。
    purge_closed_positions_interval_mins : PositiveInt, 可选
        从内存缓存中清除已关闭仓位的间隔（分钟），**不会从数据库中清除**。如果为 None，则已关闭仓位将**不会**被自动清除。对于高频交易（HFT），建议设置为 10-15 分钟。
    purge_closed_positions_buffer_mins : NonNegativeInt, 可选
        仓位关闭后到可以被清除之间的时间缓冲（分钟）。
        仅关闭至少达到此时间的仓位才会被清除。对于高频交易（HFT），建议设置为 60 分钟。
    purge_account_events_interval_mins : PositiveInt, 可选
        从内存缓存中清除账户事件的间隔（分钟），**不会从数据库中清除**。如果为 None，则账户事件将**不会**被自动清除。对于高频交易（HFT），建议设置为 10-15 分钟。
    purge_account_events_lookback_mins : NonNegativeInt, 可选
        账户事件发生后到可以被清除之间的时间缓冲（分钟）。
        仅回溯窗口之外的事件才会被清除。对于高频交易（HFT），建议设置为 60 分钟。
    purge_from_database : bool, default False
        清除操作是否也从后端数据库中删除（除了从内存缓存中删除外）。
        **注意：** 目前账户事件尚未从数据库中清除 - 等待重新实现。
    qsize : PositiveInt, 默认 100_000
        引擎内部队列缓冲区的队列大小。
    graceful_shutdown_on_exception : bool, 默认 False
        当消息队列处理过程中发生意外异常时，系统是否应执行优雅停机（不包括用户 Actor/策略异常）。

    """

    reconciliation: bool = True
    reconciliation_lookback_mins: NonNegativeInt | None = None
    reconciliation_instrument_ids: list[InstrumentId] | None = None
    filter_unclaimed_external_orders: bool = False
    filter_position_reports: bool = False
    filtered_client_order_ids: list[ClientOrderId] | None = None
    generate_missing_orders: bool = True
    inflight_check_interval_ms: NonNegativeInt = 2_000
    inflight_check_threshold_ms: NonNegativeInt = 5_000
    inflight_check_retries: NonNegativeInt = 5
    own_books_audit_interval_secs: PositiveFloat | None = None
    open_check_interval_secs: PositiveFloat | None = None
    open_check_open_only: bool = True
    open_check_lookback_mins: PositiveInt = 60
    open_check_threshold_ms: NonNegativeInt = 5_000
    open_check_missing_retries: NonNegativeInt = 5
    max_single_order_queries_per_cycle: PositiveInt = 10
    single_order_query_delay_ms: NonNegativeInt = 100
    position_check_interval_secs: PositiveFloat | None = None
    position_check_lookback_mins: PositiveInt = 60
    position_check_threshold_ms: NonNegativeInt = 5_000
    reconciliation_startup_delay_secs: PositiveFloat = 10.0
    purge_closed_orders_interval_mins: PositiveInt | None = None
    purge_closed_orders_buffer_mins: NonNegativeInt | None = None
    purge_closed_positions_interval_mins: PositiveInt | None = None
    purge_closed_positions_buffer_mins: NonNegativeInt | None = None
    purge_account_events_interval_mins: PositiveInt | None = None
    purge_account_events_lookback_mins: NonNegativeInt | None = None
    purge_from_database: bool = False
    qsize: PositiveInt = 100_000
    graceful_shutdown_on_exception: bool = False


class RoutingConfig(NautilusConfig, frozen=True):
    """
    实盘客户端消息路由配置。

    参数
    ----------
    default : bool
        是否应将客户端注册为默认路由客户端（当找不到特定场地的路由时）。
    venues : list[str], 可选
        要注册路由的场地。

    """

    default: bool = False
    venues: frozenset[str] | None = None


class LiveDataClientConfig(NautilusConfig, frozen=True):
    """
    ``LiveDataClient`` 实例的配置。

    参数
    ----------
    handle_revised_bars : bool
        当新 K 线开启时，DataClient 是否会发出 K 线更新。
    instrument_provider : InstrumentProviderConfig
        客户端的标的提供者（instrument provider）配置。
    routing : RoutingConfig
        客户端的消息路由配置。

    """

    handle_revised_bars: bool = False
    instrument_provider: InstrumentProviderConfig = InstrumentProviderConfig()
    routing: RoutingConfig = RoutingConfig()


class LiveExecClientConfig(NautilusConfig, frozen=True):
    """
    ``LiveExecutionClient`` 实例的配置。

    参数
    ----------
    instrument_provider : InstrumentProviderConfig
        客户端的标的提供者配置。
    routing : RoutingConfig
        客户端的消息路由配置。

    """

    instrument_provider: InstrumentProviderConfig = InstrumentProviderConfig()
    routing: RoutingConfig = RoutingConfig()


class ControllerConfig(ActorConfig, kw_only=True, frozen=True):
    """
    所有控制器（controller）配置的基础模型。
    """


class ControllerFactory:
    """
    提供从可导入配置中创建控制器的功能。
    """

    @staticmethod
    def create(
        config: ImportableControllerConfig,
        trader,
    ):
        from nautilus_trader.trading.trader import Trader

        PyCondition.type(trader, Trader, "trader")
        controller_cls = resolve_path(config.controller_path)
        config_cls = resolve_config_path(config.config_path)
        config = config_cls.parse(msgspec.json.encode(config.config))
        return controller_cls(config=config, trader=trader)


class TradingNodeConfig(NautilusKernelConfig, frozen=True):
    """
    ``TradingNode`` 实例的配置。

    参数
    ----------
    trader_id : TraderId, 默认 "TRADER-001"
        节点的交易者 ID（必须是由连字符分隔的名称和 ID 标签）。
    cache : CacheConfig, 可选
        缓存配置。
    data_engine : LiveDataEngineConfig, 可选
        实盘数据引擎配置。
    risk_engine : LiveRiskEngineConfig, 可选
        实盘风控引擎配置。
    exec_engine : LiveExecEngineConfig, 可选
        实盘执行引擎配置。
    data_clients : dict[str, ImportableConfig | LiveDataClientConfig], 可选
        数据客户端配置。
    exec_clients : dict[str, ImportableConfig | LiveExecClientConfig], 可选
        执行客户端配置。

    """

    environment: Environment = Environment.LIVE
    trader_id: TraderId = "TRADER-001"
    data_engine: LiveDataEngineConfig = LiveDataEngineConfig()
    risk_engine: LiveRiskEngineConfig = LiveRiskEngineConfig()
    exec_engine: LiveExecEngineConfig = LiveExecEngineConfig()
    data_clients: dict[str, LiveDataClientConfig] = {}
    exec_clients: dict[str, LiveExecClientConfig] = {}

    def __post_init__(self):
        if isinstance(self.trader_id, str):
            msgspec.structs.force_setattr(self, "trader_id", TraderId(self.trader_id))
