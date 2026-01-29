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

from nautilus_trader.common.config import NautilusConfig
from nautilus_trader.common.config import PositiveInt


class PortfolioConfig(NautilusConfig, frozen=True):
    """
    ``Portfolio`` 实例的配置。

    参数
    ----------
    use_mark_prices : bool, 默认 False
        用于盈亏和净风险敞口计算的价格类型。
        如果为 False（默认），则优先使用买卖报价（如果可用）；否则，使用最新成交价
        （如果 `bar_updates` 为 True，则回退到 Bar 价格）。
        如果为 True，则使用标记价格。
    use_mark_xrates : bool, 默认 False
        用于盈亏和净风险敞口计算的汇率类型。
        如果为 False（默认），则使用买卖报价。
        如果为 True，则使用标记价格。
    bar_updates : bool, 默认 True
        计算时是否应考虑外部 Bar 价格。
    convert_to_account_base_currency : bool, 默认 True
        是否应将计算结果转换为每个账户的本位币。
        此设置仅对指定了本位币的账户有效。
    min_account_state_logging_interval_ms : PositiveInt, 可选
        同一账户记录账户状态事件之间的最小间隔（毫秒）。
        设置后，仅当距上次日志已过去这段时间时，才会记录账户状态更新。
        适用于高频交易 (HFT) 部署，以防止账户状态快速变化时产生过多的日志。
        默认值为 None（不限制）。
    debug : bool, 默认 False
        调试模式是否激活（将提供额外的调试日志记录）。

    """

    use_mark_prices: bool = False
    use_mark_xrates: bool = False
    bar_updates: bool = True
    convert_to_account_base_currency: bool = True
    min_account_state_logging_interval_ms: PositiveInt | None = None
    debug: bool = False
