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


class RiskEngineConfig(NautilusConfig, frozen=True):
    """
    ``RiskEngine`` 实例的配置。

    参数
    ----------
    bypass : bool, 默认 False
        如果为 True，则将跳过所有盘前风控检查和频率限制（但仍会检查重复 ID）。
    max_order_submit_rate : str, 默认 100/00:00:01
        每个时间间隔内提交订单命令的最大频率。
    max_order_modify_rate : str, 默认 100/00:00:01
        每个时间间隔内修改订单命令的最大频率。
    max_notional_per_order : dict[str, int], 默认空字典
        每个标的 ID 的订单最大名义价值。
        该值应为有效的 Decimal 格式。
    debug : bool, 默认 False
        调试模式是否激活（将提供额外的调试日志记录）。
    t1_enabled : bool, 默认 False
        是否启用 T+1 交易限制检查。
    session_enabled : bool | None, 默认 None
        是否启用 A 股交易时段检查（报单/撤单）。
        当为 ``None`` 时，兼容历史行为：跟随 ``t1_enabled``。
    price_tick_enabled : bool | None, 默认 None
        是否启用 A 股价格最小变动单位（tick）检查。
        当为 ``None`` 时，兼容历史行为：跟随 ``t1_enabled``。
    price_limit_enabled : bool | None, 默认 None
        是否启用 A 股涨跌停价检查。
        当为 ``None`` 时，兼容历史行为：跟随 ``t1_enabled``。
    lot_size_enabled : bool | None, 默认 None
        是否启用 A 股买入手数整数倍及卖出零股规则检查。
        当为 ``None`` 时，兼容历史行为：跟随 ``t1_enabled``。
    price_cage_enabled : bool, 默认 False
        是否启用价格笼子（Price Cage）检查。
    price_cage_pct : float, 默认 0.02
        价格笼子的偏离比例（如 0.02 代表 2%）。
    max_trade_command_rate : str | None, 默认 None
        包含申报(Submit)、撤单(Cancel)、改单(Modify)等所有交易请求的全局发送频率。
        格式例如 "500/00:00:01" 代表每秒最多处理500个操作指令。

    """

    bypass: bool = False
    max_order_submit_rate: str = "100/00:00:01"
    max_order_modify_rate: str = "100/00:00:01"
    max_notional_per_order: dict[str, int] = {}
    debug: bool = False
    t1_enabled: bool = False
    session_enabled: bool | None = None
    price_tick_enabled: bool | None = None
    price_limit_enabled: bool | None = None
    lot_size_enabled: bool | None = None
    price_cage_enabled: bool = False
    price_cage_pct: float = 0.02
    max_trade_command_rate: str | None = None

