# -------------------------------------------------------------------------------------------------
#  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
#  https://nautechsystems.io
#
#  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
#  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
#
#  Unless required by applicable law or agreed to in writing, software
#  distributed under the License is distributed on an "AS IS" BASIS,
#  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#  See the License for the specific language governing permissions and
#  limitations under the License.
# -------------------------------------------------------------------------------------------------
"""
`Trader` 类旨在管理平台运行实例中的交易策略集群。

运行实例可以是测试/回测或实盘实现 —— `Trader` 将以相同的方式运行。
"""

import asyncio
from collections.abc import Callable
from typing import Any

import pandas as pd

from nautilus_trader.analysis.reporter import ReportProvider
from nautilus_trader.cache.cache import Cache
from nautilus_trader.common import Environment
from nautilus_trader.common.actor import Actor
from nautilus_trader.common.component import Clock
from nautilus_trader.common.component import Component
from nautilus_trader.common.component import MessageBus
from nautilus_trader.common.component import deregister_component_clock
from nautilus_trader.common.component import register_component_clock
from nautilus_trader.common.component import remove_instance_component_clocks
from nautilus_trader.core.correctness import PyCondition
from nautilus_trader.core.uuid import UUID4
from nautilus_trader.data.engine import DataEngine
from nautilus_trader.model.identifiers import AccountId
from nautilus_trader.model.identifiers import ComponentId
from nautilus_trader.model.identifiers import ExecAlgorithmId
from nautilus_trader.model.identifiers import StrategyId
from nautilus_trader.model.identifiers import TraderId
from nautilus_trader.model.identifiers import Venue
from nautilus_trader.portfolio.portfolio import Portfolio
from nautilus_trader.risk.engine import RiskEngine
from nautilus_trader.trading.strategy import Strategy


class Trader(Component):
    """
    提供一个交易员，用于管理 Actor、执行算法和交易策略集群。

    参数
    ----------
    trader_id : TraderId
        交易员的 ID。
    instance_id : UUID4
        交易员的实例 ID。
    msgbus : MessageBus
        交易员的消息总线。
    cache : Cache
        交易员的缓存。
    portfolio : Portfolio
        交易员的投资组合。
    data_engine : DataEngine
        交易员的数据引擎。
    risk_engine : RiskEngine
        交易员的风险引擎。
    exec_engine : ExecutionEngine
        交易员的执行引擎。
    clock : Clock
        交易员的时钟。
    environment : Environment { ``BACKTEST``, ``SANDBOX``, ``LIVE`` }
        环境上下文。
    has_controller : bool, 默认为 False
        交易员是否拥有控制器。
    loop : asyncio.AbstractEventLoop, 可选
        交易员的事件循环。

    异常
    ------
    ValueError
        如果 `portfolio` 与 `exec_engine` 的投资组合不一致。
    ValueError
        如果 `strategies` 为 ``None``。
    ValueError
        如果 `strategies` 为空。
    TypeError
        如果 `strategies` 包含 `Strategy` 以外的类型。
    """

    def __init__(
        self,
        trader_id: TraderId,
        instance_id: UUID4,
        msgbus: MessageBus,
        cache: Cache,
        portfolio: Portfolio,
        data_engine: DataEngine,
        risk_engine: RiskEngine,
        exec_engine: Any,
        clock: Clock,
        environment: Environment,
        has_controller: bool = False,
        loop: asyncio.AbstractEventLoop | None = None,
    ) -> None:
        # 在此处导入以避免循环导入问题
        from nautilus_trader.execution.engine import ExecutionEngine

        PyCondition.type(exec_engine, ExecutionEngine, "exec_engine")
        super().__init__(
            clock=clock,
            component_id=trader_id,
            msgbus=msgbus,
        )

        self._instance_id = instance_id
        self._environment = environment
        self._loop = loop
        self._cache = cache
        self._portfolio = portfolio
        self._data_engine = data_engine
        self._risk_engine = risk_engine
        self._exec_engine = exec_engine

        self._actors: dict[ComponentId, Actor] = {}
        self._strategies: dict[StrategyId, Strategy] = {}
        self._exec_algorithms: dict[ExecAlgorithmId, Any] = {}
        self._has_controller: bool = has_controller

    @property
    def instance_id(self) -> UUID4:
        """
        返回交易员的实例 ID。

        返回
        -------
        UUID4
        """
        return self._instance_id

    def actors(self) -> list[Actor]:
        """
        返回加载到交易员中的 Actor 列表。

        返回
        -------
        list[Actor]
        """
        return list(self._actors.values())

    def strategies(self) -> list[Strategy]:
        """
        返回加载到交易员中的策略列表。

        返回
        -------
        list[Strategy]
        """
        return list(self._strategies.values())

    def exec_algorithms(self) -> list[Any]:  # ExecutionAlgorithm (循环导入问题)
        """
        返回加载到交易员中的执行算法列表。

        返回
        -------
        list[ExecAlgorithms]
        """
        return list(self._exec_algorithms.values())

    def actor_ids(self) -> list[ComponentId]:
        """
        返回加载到交易员中的 Actor ID 列表。

        返回
        -------
        list[ComponentId]
        """
        return sorted(self._actors.keys())

    def strategy_ids(self) -> list[StrategyId]:
        """
        返回加载到交易员中的策略 ID 列表。

        返回
        -------
        list[StrategyId]
        """
        return sorted(self._strategies.keys())

    def exec_algorithm_ids(self) -> list[ExecAlgorithmId]:
        """
        返回加载到交易员中的执行算法 ID 列表。

        返回
        -------
        list[ExecAlgorithmId]
        """
        return sorted(self._exec_algorithms.keys())

    def actor_states(self) -> dict[ComponentId, str]:
        """
        返回交易员中 Actor 的状态。

        返回
        -------
        dict[ComponentId, str]
        """
        return {k: v.state.name for k, v in self._actors.items()}

    def strategy_states(self) -> dict[StrategyId, str]:
        """
        返回交易员中策略的状态。

        返回
        -------
        dict[StrategyId, str]
        """
        return {k: v.state.name for k, v in self._strategies.items()}

    def exec_algorithm_states(self) -> dict[ExecAlgorithmId, str]:
        """
        返回交易员中执行算法的状态。

        返回
        -------
        dict[ExecAlgorithmId, str]
        """
        return {k: v.state.name for k, v in self._exec_algorithms.items()}

    # -- 动作实现 -----------------------------------------------------------------------

    def _start(self) -> None:
        is_backtest = self._environment == Environment.BACKTEST
        now_ns = self._clock.timestamp_ns()

        for actor in list(self._actors.values()):
            if is_backtest:
                actor.clock.set_time(now_ns)

            actor.start()

        for strategy in list(self._strategies.values()):
            if is_backtest:
                strategy.clock.set_time(now_ns)

            strategy.start()

        for exec_algorithm in list(self._exec_algorithms.values()):
            if is_backtest:
                exec_algorithm.clock.set_time(now_ns)

            exec_algorithm.start()

    def _stop(self) -> None:
        for actor in self._actors.values():
            if actor.is_running:
                actor.stop()
            else:
                self._log.warning(f"{actor} 已经停止")

        for strategy in self._strategies.values():
            if strategy.is_running:
                strategy.stop()
            else:
                self._log.warning(f"{strategy} 已经停止")

        for exec_algorithm in self._exec_algorithms.values():
            if exec_algorithm.is_running:
                exec_algorithm.stop()
            else:
                self._log.warning(f"{exec_algorithm} 已经停止")

    def _reset(self) -> None:
        for actor in self._actors.values():
            actor.reset()

        for strategy in self._strategies.values():
            strategy.reset()

        for exec_algorithm in self._exec_algorithms.values():
            exec_algorithm.reset()

        self._portfolio.reset()

    def _dispose(self) -> None:
        self.clear_actors()
        self.clear_strategies()
        self.clear_exec_algorithms()
        remove_instance_component_clocks(self._instance_id)

    # --------------------------------------------------------------------------------------------------

    def add_actor(self, actor: Actor) -> None:
        """
        将给定的自定义组件添加到交易员。

        参数
        ----------
        actor : Actor
            要添加并注册的 Actor。

        异常
        ------
        ValueError
            如果 `actor.state` 为 ``RUNNING`` 或 ``DISPOSED``。
        RuntimeError
            如果 `actor.id` 已经存在于交易员中。
        """
        PyCondition.is_true(not actor.is_running, "actor.state 为 RUNNING")
        PyCondition.is_true(not actor.is_disposed, "actor.state 为 DISPOSED")

        if self.is_running and not self._has_controller:
            self._log.error("无法向正在运行的交易员添加 Actor/组件")
            return

        if actor.id in self._actors:
            raise RuntimeError(
                f"已经注册了 ID 为 {actor.id} 的 Actor，"
                "请尝试指定不同的 Actor ID。",
            )

        clock = self._clock.__class__()  # 每个组件一个时钟
        register_component_clock(self._instance_id, clock)

        # 将组件接入交易员
        actor.register_base(
            portfolio=self._portfolio,
            msgbus=self._msgbus,
            cache=self._cache,
            clock=clock,
        )
        self._actors[actor.id] = actor
        self._log.info(f"已注册组件 {actor}")

    def add_actors(self, actors: list[Actor]) -> None:
        """
        将给定的 Actor 列表添加到交易员。

        参数
        ----------
        actors : list[Actor]
            要添加并注册的 Actor 列表。

        异常
        ------
        ValueError
            如果 `actors` 为 ``None`` 或为空。
        """
        PyCondition.not_empty(actors, "actors")

        for actor in actors:
            self.add_actor(actor)

    def add_strategy(self, strategy: Strategy) -> None:
        """
        将给定的交易策略添加到交易员。

        参数
        ----------
        strategy : Strategy
            要添加并注册的交易策略。

        异常
        ------
        ValueError
            如果 `strategy.state` 为 ``RUNNING`` 或 ``DISPOSED``。
        RuntimeError
            如果 `strategy.id` 已经存在于交易员中。
        """
        PyCondition.not_none(strategy, "strategy")
        PyCondition.is_true(not strategy.is_running, "strategy.state 为 RUNNING")
        PyCondition.is_true(not strategy.is_disposed, "strategy.state 为 DISPOSED")

        if self.is_running and not self._has_controller:
            self._log.error("无法向正在运行的交易员添加策略")
            return

        if strategy.id in self._strategies:
            raise RuntimeError(
                f"已经注册了 ID 为 {strategy.id} 的策略，"
                "请尝试指定不同的策略 ID。",
            )

        # 确认策略 ID
        order_id_tags: list[str] = [s.order_id_tag for s in self._strategies.values()]
        if strategy.order_id_tag in (None, str(None)):
            order_id_tag = f"{len(order_id_tags):03d}"
            # 分配策略 `order_id_tag`
            strategy_id = StrategyId(f"{strategy.id.value.partition('-')[0]}-{order_id_tag}")
            strategy.change_id(strategy_id)
            strategy.change_order_id_tag(order_id_tag)

        # 检查重复的 `order_id_tag`
        if strategy.order_id_tag in order_id_tags:
            raise RuntimeError(
                f"策略 `order_id_tag` 冲突: '{strategy.order_id_tag}'，"
                f"请在策略配置中明确定义所有 `order_id_tag` 值",
            )

        clock = self._clock.__class__()  # 每个组件一个时钟
        register_component_clock(self._instance_id, clock)

        # 将策略接入交易员
        strategy.register(
            trader_id=self.id,
            portfolio=self._portfolio,
            msgbus=self._msgbus,
            cache=self._cache,
            clock=clock,
        )

        self._exec_engine.register_oms_type(strategy)
        self._exec_engine.register_external_order_claims(strategy)
        self._strategies[strategy.id] = strategy

        self._log.info(f"已注册策略 {strategy}")

    def add_strategies(self, strategies: list[Strategy]) -> None:
        """
        将给定的交易策略列表添加到交易员。

        参数
        ----------
        strategies : list[Strategy]
            要添加并注册的交易策略列表。

        异常
        ------
        ValueError
            如果 `strategies` 为 ``None`` 或为空。
        """
        PyCondition.not_empty(strategies, "strategies")

        for strategy in strategies:
            self.add_strategy(strategy)

    def add_exec_algorithm(self, exec_algorithm: Any) -> None:
        """
        将给定的执行算法添加到交易员。

        参数
        ----------
        exec_algorithm : ExecAlgorithm
            要添加并注册的执行算法。

        异常
        ------
        KeyError
            如果 `exec_algorithm.id` 已经存在于交易员中。
        ValueError
            如果 `exec_algorithm.state` 为 ``RUNNING`` 或 ``DISPOSED``。
        """
        PyCondition.not_none(exec_algorithm, "exec_algorithm")
        PyCondition.is_true(not exec_algorithm.is_running, "exec_algorithm.state 为 RUNNING")
        PyCondition.is_true(not exec_algorithm.is_disposed, "exec_algorithm.state 为 DISPOSED")

        if self.is_running:
            self._log.error("无法向正在运行的交易员添加执行算法")
            return

        if exec_algorithm.id in self._exec_algorithms:
            raise RuntimeError(
                f"已经注册了 ID 为 {exec_algorithm.id} 的执行算法，"
                "请尝试指定不同的 `exec_algorithm_id`。",
            )

        clock = self._clock.__class__()  # 每个组件一个时钟
        register_component_clock(self._instance_id, clock)

        # 将执行算法接入交易员
        exec_algorithm.register(
            trader_id=self.id,
            portfolio=self._portfolio,
            msgbus=self._msgbus,
            cache=self._cache,
            clock=clock,
        )
        self._exec_algorithms[exec_algorithm.id] = exec_algorithm

        self._log.info(f"已注册执行算法 {exec_algorithm}")

    def add_exec_algorithms(self, exec_algorithms: list[Any]) -> None:
        """
        将给定的执行算法列表添加到交易员。

        参数
        ----------
        exec_algorithms : list[ExecAlgorithm]
            要添加并注册的执行算法列表。

        异常
        ------
        ValueError
            如果 `exec_algorithms` 为 ``None`` 或为空。
        """
        PyCondition.not_empty(exec_algorithms, "exec_algorithms")

        for exec_algorithm in exec_algorithms:
            self.add_exec_algorithm(exec_algorithm)

    def start_actor(self, actor_id: ComponentId) -> None:
        """
        启动具有给定 `actor_id` 的 Actor。

        参数
        ----------
        actor_id : ComponentId
            要启动的组件 ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `actor_id` 的 Actor。
        """
        PyCondition.not_none(actor_id, "actor_id")

        actor = self._actors.get(actor_id)

        if actor is None:
            raise ValueError(f"无法启动 Actor，未找到 {actor_id}。")

        if actor.is_running:
            self._log.warning(f"Actor {actor_id} 已经在运行")
            return

        actor.start()

    def start_strategy(self, strategy_id: StrategyId) -> None:
        """
        启动具有给定 `strategy_id` 的策略。

        参数
        ----------
        strategy_id : StrategyId
            要启动的策略 ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `strategy_id` 的策略。
        """
        PyCondition.not_none(strategy_id, "strategy_id")

        strategy = self._strategies.get(strategy_id)

        if strategy is None:
            raise ValueError(f"无法启动策略，未找到 {strategy_id}。")

        if strategy.is_running:
            self._log.warning(f"策略 {strategy_id} 已经在运行")
            return

        strategy.start()

    def stop_actor(self, actor_id: ComponentId) -> None:
        """
        停止具有给定 `actor_id` 的 Actor。

        参数
        ----------
        actor_id : ComponentId
            要停止的 Actor ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `actor_id` 的 Actor。
        """
        PyCondition.not_none(actor_id, "actor_id")

        actor = self._actors.get(actor_id)

        if actor is None:
            raise ValueError(f"无法停止 Actor，未找到 {actor_id}。")

        if not actor.is_running:
            self._log.warning(f"Actor {actor_id} 未在运行")
            return

        actor.stop()

    def stop_strategy(self, strategy_id: StrategyId) -> None:
        """
        停止具有给定 `strategy_id` 的策略。

        参数
        ----------
        strategy_id : StrategyId
            要停止的策略 ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `strategy_id` 的策略。
        """
        PyCondition.not_none(strategy_id, "strategy_id")

        strategy = self._strategies.get(strategy_id)

        if strategy is None:
            raise ValueError(f"无法停止策略，未找到 {strategy_id}。")

        if not strategy.is_running:
            self._log.warning(f"策略 {strategy_id} 未在运行")
            return

        strategy.stop()

    def remove_actor(self, actor_id: ComponentId) -> None:
        """
        移除具有给定 `actor_id` 的 Actor。

        如果状态为 ``RUNNING``，将先停止该 Actor。

        参数
        ----------
        actor_id : ComponentId
            要移除的 Actor ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `actor_id` 的 Actor。
        """
        PyCondition.not_none(actor_id, "actor_id")

        actor = self._actors.get(actor_id)

        if actor is None:
            raise ValueError(f"无法移除 Actor，未找到 {actor_id}。")

        if actor.is_running:
            actor.stop()

        self._actors.pop(actor_id)
        deregister_component_clock(self._instance_id, actor.clock)

    def remove_strategy(self, strategy_id: StrategyId) -> None:
        """
        移除具有给定 `strategy_id` 的策略。

        如果状态为 ``RUNNING``，将先停止该策略。

        参数
        ----------
        strategy_id : StrategyId
            要移除的策略 ID。

        异常
        ------
        ValueError
            如果未找到具有给定 `strategy_id` 的策略。
        """
        PyCondition.not_none(strategy_id, "strategy_id")

        strategy = self._strategies.get(strategy_id)

        if strategy is None:
            raise ValueError(f"无法移除策略，未找到 {strategy_id}。")

        if strategy.is_running:
            strategy.stop()

        self._strategies.pop(strategy_id)
        deregister_component_clock(self._instance_id, strategy.clock)

    def clear_actors(self) -> None:
        """
        释放并清除交易员持有的所有 Actor。

        异常
        ------
        ValueError
            如果状态为 ``RUNNING``。
        """
        if self.is_running:
            self._log.error("无法清除正在运行的交易员的 Actor")
            return

        for actor in self._actors.values():
            actor.dispose()
            deregister_component_clock(self._instance_id, actor.clock)

        self._actors.clear()
        self._log.info("已清除 Actor")

    def clear_strategies(self) -> None:
        """
        释放并清除交易员持有的所有策略。

        异常
        ------
        ValueError
            如果状态为 ``RUNNING``。
        """
        if self.is_running:
            self._log.error("无法清除正在运行的交易员的策略")
            return

        for strategy in self._strategies.values():
            strategy.dispose()
            deregister_component_clock(self._instance_id, strategy.clock)

        self._strategies.clear()
        self._log.info("已清除交易策略")

    def clear_exec_algorithms(self) -> None:
        """
        释放并清除交易员持有的所有执行算法。

        异常
        ------
        ValueError
            如果状态为 ``RUNNING``。
        """
        if self.is_running:
            self._log.error("无法清除正在运行的交易员的执行算法")
            return

        for exec_algorithm in self._exec_algorithms.values():
            exec_algorithm.dispose()
            deregister_component_clock(self._instance_id, exec_algorithm.clock)

        self._exec_algorithms.clear()
        self._log.info("已清除执行算法")

    def subscribe(self, topic: str, handler: Callable[[Any], None]) -> None:
        """
        使用给定的回调处理程序订阅给定的消息主题。

        参数
        ----------
        topic : str
            订阅的主题。可以包含通配符 glob 模式。
        handler : Callable[[Any], None]
            订阅的处理程序。
        """
        self._msgbus.subscribe(topic=topic, handler=handler)

    def unsubscribe(self, topic: str, handler: Callable[[Any], None]) -> None:
        """
        从给定的消息主题中取消订阅给定的处理程序。

        参数
        ----------
        topic : str, 可选
            要取消订阅的主题。可以包含通配符 glob 模式。
        handler : Callable[[Any], None]
            订阅的处理程序。
        """
        self._msgbus.unsubscribe(topic=topic, handler=handler)

    def save(self) -> None:
        """
        将所有 Actor 和策略状态保存到缓存。
        """
        for actor in self._actors.values():
            self._cache.update_actor(actor)

        for strategy in self._strategies.values():
            self._cache.update_strategy(strategy)

    def load(self) -> None:
        """
        从缓存加载所有 Actor 和策略状态。
        """
        for actor in self._actors.values():
            self._cache.load_actor(actor)

        for strategy in self._strategies.values():
            self._cache.load_strategy(strategy)

    def check_residuals(self) -> None:
        """
        检查残留的开放状态，例如未平仓订单或未平仓头寸。
        """
        self._exec_engine.check_residuals()

    def generate_orders_report(self) -> pd.DataFrame:
        """
        生成订单报告。

        返回
        -------
        pd.DataFrame
        """
        return ReportProvider.generate_orders_report(self._cache.orders())

    def generate_order_fills_report(self) -> pd.DataFrame:
        """
        生成订单成交报告。

        返回
        -------
        pd.DataFrame
        """
        return ReportProvider.generate_order_fills_report(self._cache.orders())

    def generate_fills_report(self) -> pd.DataFrame:
        """
        生成成交报告。

        返回
        -------
        pd.DataFrame
        """
        return ReportProvider.generate_fills_report(self._cache.orders())

    def generate_positions_report(self) -> pd.DataFrame:
        """
        生成头寸报告。

        返回
        -------
        pd.DataFrame
        """
        positions = self._cache.positions()
        snapshots = self._cache.position_snapshots()

        # 使用头寸和快照生成报告
        return ReportProvider.generate_positions_report(positions, snapshots)

    def generate_account_report(
        self,
        venue: Venue = None,
        account_id: AccountId = None,
    ) -> pd.DataFrame:
        """
        生成账户报告。

        参数
        ----------
        venue : Venue, 可选
            账户的场馆（如果未提供 account_id，则使用此参数）。
        account_id : AccountId, 可选
            账户 ID（如果同时提供了 venue 和 account_id，则优先使用此参数）。

        返回
        -------
        pd.DataFrame
        """
        if account_id is None and venue is None:
            raise ValueError("必须至少提供 'venue' 或 'account_id' 之一")

        account = self._cache.account_for_venue(venue=venue, account_id=account_id)

        if account is None:
            return pd.DataFrame()

        return ReportProvider.generate_account_report(account)
