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
from nautilus_trader.common.config import msgspec_encoding_hook
from nautilus_trader.common.config import resolve_config_path
from nautilus_trader.common.config import resolve_path
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.model.identifiers import InstrumentId
from nautilus_trader.model.identifiers import StrategyId


class StrategyConfig(NautilusConfig, kw_only=True, frozen=True):
    """
    所有交易策略配置的基础模型。

    参数
    ----------
    strategy_id : StrategyId, 可选
        策略的唯一 ID。如果提供，将成为策略的 ID。
    order_id_tag : str, 可选
        策略的唯一订单 ID 标签。在同一个交易者 ID 下运行的所有策略中必须唯一。
    use_uuid_client_order_ids : bool, 默认为 False
        是否使用 UUID4 作为客户端订单 ID 值。
    use_hyphens_in_client_order_ids : bool, 默认为 True
        生成的客户端订单 ID 值中是否包含连字符。
    oms_type : OmsType, 可选
        策略的订单管理系统类型。这将决定 `ExecutionEngine` 如何处理持仓 ID。
    external_order_claims : list[InstrumentId], 可选
        外部订单认领的合约 ID 列表。匹配这些合约 ID 的外部订单将归属于（被认领）该策略。
    manage_contingent_orders : bool, 默认为 False
        策略是否应自动管理 OTO、OCO 和 OUO 等**处于开启状态**的条件订单。
        任何在本地活跃的仿真订单将由 `OrderEmulator` 管理。
    manage_gtd_expiry : bool, 默认为 False
        策略是否应管理所有 GTD（Good-Till-Date）时效订单的过期。
        如果为 True，将确保在启动时重新激活挂单的 GTD 定时器。
    log_events : bool, 默认为 True
        策略是否应记录事件。
        如果为 False，则仅记录警告及以上级别的事件。
    log_commands : bool, 默认为 True
        策略是否应记录命令。
    log_rejected_due_post_only_as_warning : bool, 默认为 True
        由于 `due_post_only` 导致的订单拒绝事件是否应记录为警告。

    """

    strategy_id: StrategyId | None = None
    order_id_tag: str | None = None
    use_uuid_client_order_ids: bool = False
    use_hyphens_in_client_order_ids: bool = True
    oms_type: str | None = None
    external_order_claims: list[InstrumentId] | None = None
    manage_contingent_orders: bool = False
    manage_gtd_expiry: bool = False
    log_events: bool = True
    log_commands: bool = True
    log_rejected_due_post_only_as_warning: bool = True


class ImportableStrategyConfig(NautilusConfig, frozen=True):
    """
    交易策略实例的配置。

    参数
    ----------
    strategy_path : str
        策略类的完全限定名（导入路径）。
    config_path : str
        配置类的完全限定名（导入路径）。
    config : dict[str, Any]
        策略的具体配置字典。

    """

    strategy_path: str
    config_path: str
    config: dict[str, Any]


class StrategyFactory:
    """
    提供从可导入配置创建策略的功能。
    """

    @staticmethod
    def create(config: ImportableStrategyConfig):
        """
        从给定的配置创建一个交易策略。

        参数
        ----------
        config : ImportableStrategyConfig
            用于构建步骤的配置。

        返回
        -------
        Strategy
            创建的策略实例。

        抛出
        ------
        TypeError
            如果 `config` 类型不是 `ImportableStrategyConfig`。

        """
        PyCondition.type(config, ImportableStrategyConfig, "config")
        strategy_cls = resolve_path(config.strategy_path)
        config_cls = resolve_config_path(config.config_path)
        json = msgspec.json.encode(config.config, enc_hook=msgspec_encoding_hook)
        config = config_cls.parse(json)
        return strategy_cls(config=config)


class ImportableControllerConfig(NautilusConfig, frozen=True):
    """
    控制器实例的配置。

    参数
    ----------
    controller_path : str
        控制器类的完全限定名（导入路径）。
    config_path : str
        配置类的完全限定名（导入路径）。
    config : dict[str, Any]
        控制器的具体配置字典。

    """

    controller_path: str
    config_path: str
    config: dict
