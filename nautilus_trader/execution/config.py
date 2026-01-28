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

from typing import Any

import msgspec

from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.config import PositiveFloat
from nautilus_trader.common.config import msgspec_encoding_hook
from nautilus_trader.common.config import resolve_config_path
from nautilus_trader.common.config import resolve_path
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.model.identifiers import ClientId
from nautilus_trader.model.identifiers import ExecAlgorithmId


class ExecEngineConfig(NautilusConfig, frozen=True):
    """
    ``ExecutionEngine``（执行引擎）实例的配置。

    参数
    ----------
    load_cache : bool, 默认 True
        初始化时是否加载缓存。
    manage_own_order_books : bool, 默认 False
        执行引擎是否根据命令和事件维护自己的/用户的订单簿。
    snapshot_orders : bool, 默认 False
        订单状态快照列表是否持久化到后端数据库。
        快照将在每次订单状态更新时（应用事件时）拍摄。
    snapshot_positions : bool, 默认 False
        持仓状态快照列表是否持久化到后端数据库。
        快照将在持仓开仓、变更和平仓时（应用事件时）拍摄。
        要在快照中包含未实现盈亏，缓存中必须有该持仓合约的报价数据。
    snapshot_positions_interval_secs : PositiveFloat, 可选
        *额外*持仓状态快照持久化到后端数据库的间隔时间（秒）。
        如果为 ``None``，则不会拍摄额外的快照。
        要在这些快照中包含未实现盈亏，缓存中必须有该持仓合约的报价数据。
    convert_quote_qty_to_base : bool, 默认 True
        以报价货币计价的订单数量在提交前是否应转换为基础货币单位。
        已弃用：未来版本将移除此自动转换。设置为 ``False`` 以保持与预期
        报价货币计价数量的交易所行为一致。
    external_clients : list[ClientId], 可选
        代表外部执行流的客户端 ID 列表。
        带有这些客户端 ID 的命令将仅发布到消息总线；
        执行引擎不会尝试将它们转发到本地的 `ExecutionClient`。
    allow_overfills : bool, 默认 False
        如果为 True，允许超过原始订单数量的成交。
        当检测到超额成交时，订单的 ``overfill_qty`` 将被设置并记录警告。
        如果为 False（默认），为了向后兼容将抛出 ValueError。
    debug : bool, 默认 False
        调试模式是否激活（将提供额外的调试日志）。

    """

    load_cache: bool = True
    manage_own_order_books: bool = False
    convert_quote_qty_to_base: bool = True
    snapshot_orders: bool = False
    snapshot_positions: bool = False
    snapshot_positions_interval_secs: PositiveFloat | None = None
    external_clients: list[ClientId] | None = None
    allow_overfills: bool = False
    debug: bool = False


class ExecAlgorithmConfig(NautilusConfig, kw_only=True, frozen=True):
    """
    所有执行算法配置的基础模型。

    参数
    ----------
    exec_algorithm_id : ExecAlgorithmId, 可选
        执行算法的唯一 ID。
        如果不为 ``None``，则将成为执行算法 ID。
    log_events : bool, 默认 True
        执行算法是否记录事件日志。
        如果为 False，则只记录警告及以上级别的事件。
    log_commands : bool, 默认 True
        执行算法是否记录命令日志。

    """

    exec_algorithm_id: ExecAlgorithmId | None = None
    log_events: bool = True
    log_commands: bool = True


class ImportableExecAlgorithmConfig(NautilusConfig, frozen=True):
    """
    执行算法实例的配置。

    参数
    ----------
    exec_algorithm_path : str
        执行算法类的完全限定名。
    config_path : str
        配置类的完全限定名。
    config : dict[str, Any]
        执行算法配置。

    """

    exec_algorithm_path: str
    config_path: str
    config: dict[str, Any]


class ExecAlgorithmFactory:
    """
    提供从可导入配置创建执行算法的功能。
    """

    @staticmethod
    def create(config: ImportableExecAlgorithmConfig):
        """
        从给定配置创建执行算法。

        参数
        ----------
        config : ImportableExecAlgorithmConfig
            构建步骤的配置。

        返回
        -------
        ExecAlgorithm

        抛出
        ------
        TypeError
            如果 `config` 的类型不是 `ImportableExecAlgorithmConfig`。

        """
        PyCondition.type(config, ImportableExecAlgorithmConfig, "config")
        exec_algorithm_cls = resolve_path(config.exec_algorithm_path)
        config_cls = resolve_config_path(config.config_path)
        json = msgspec.json.encode(config.config, enc_hook=msgspec_encoding_hook)
        config = config_cls.parse(json)
        return exec_algorithm_cls(config=config)

