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

from nautilus_trader.common.component import LogColor
from nautilus_trader.config import ExecAlgorithmConfig
from nautilus_trader.execution.algorithm import ExecAlgorithm
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.orders import Order
from nautilus_trader.model.orders import OrderList


class MyExecAlgorithmConfig(ExecAlgorithmConfig, frozen=True):
    """
    ``MyExecAlgorithm`` 实例的配置。

    参数
    ----------
    exec_algorithm_id : str | ExecAlgorithmId, 可选
        执行算法 ID（将覆盖默认的类名）。

    """

    exec_algorithm_id: ExecAlgorithmId | None = None


class MyExecAlgorithm(ExecAlgorithm):
    """
    一个空白模板执行算法。

    参数
    ----------
    config : MyExecAlgorithmConfig
        该实例的配置。

    """

    def __init__(self, config: MyExecAlgorithmConfig) -> None:
        super().__init__(config)
        # 可选：实现进一步的初始化

    def on_start(self) -> None:
        """
        算法组件启动时执行的操作。
        """
        # 可选实现

    def on_stop(self) -> None:
        """
        算法组件停止时执行的操作。
        """
        # 可选实现

    def on_reset(self) -> None:
        """
        算法组件重置时执行的操作。
        """
        # 可选实现

    def on_dispose(self) -> None:
        """
        算法组件注销时执行的操作。

        在此处清理策略使用的任何资源。

        """
        # 可选实现

    def on_save(self) -> dict[str, bytes]:
        """
        算法组件保存时执行的操作。

        创建并返回要保存的状态字典。

        返回
        -------
        dict[str, bytes]
            策略状态字典。

        """
        return {}  # 可选实现

    def on_load(self, state: dict[str, bytes]) -> None:
        """
        算法组件加载时执行的操作。

        保存的状态值将包含在给定的状态字典中。

        参数
        ----------
        state : dict[str, bytes]
            策略状态字典。

        """
        # 可选实现

    def on_order(self, order: Order) -> None:
        """
        运行时接收到订单时执行的操作。

        参数
        ----------
        order : Order
            待处理的订单。

        警告
        --------
        系统方法（不应由用户代码调用）。

        """
        self.log.info(repr(order), LogColor.CYAN)
        # 可选实现

    def on_order_list(self, order_list: OrderList) -> None:
        """
        运行时接收到订单列表时执行的操作。

        参数
        ----------
        order_list : OrderList
            待处理的订单列表。

        警告
        --------
        系统方法（不应由用户代码调用）。

        """
        self.log.info(repr(order_list), LogColor.CYAN)
        # 可选实现
