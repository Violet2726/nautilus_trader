// -------------------------------------------------------------------------------------------------
//  版权所有 (C) 2015-2026 Nautech Systems Pty Ltd。保留所有权利。
//  https://nautechsystems.io
//
//  基于 GNU Lesser General Public License 3.0 版本（“许可证”）获得许可；
//  除非符合许可证，否则您不得使用此文件。
//  您可以在 https://www.gnu.org/licenses/lgpl-3.0.en.html 获取许可证副本。
//
//  除非适用法律要求或书面同意，
//  否则根据许可证分发的软件是基于“按原样”基础分发的，
//  不附带任何明示或暗示的保证或条件。
//  请参阅许可证以了解管理许可证下的权限和限制的具体语言。
// -------------------------------------------------------------------------------------------------

//! 执行算法的核心组件。

use std::{
    cell::RefCell,
    fmt::Debug,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use ahash::{AHashMap, AHashSet};
use nautilus_common::{
    actor::{DataActorConfig, DataActorCore},
    cache::Cache,
    clock::Clock,
    msgbus::TypedHandler,
};
use nautilus_model::{
    events::{OrderEventAny, PositionEvent},
    identifiers::{ActorId, ClientOrderId, ExecAlgorithmId, StrategyId, TraderId},
    orders::{OrderAny, OrderList},
    types::Quantity,
};

use super::config::ExecutionAlgorithmConfig;

/// 持有策略事件订阅的事件处理器。
#[derive(Clone, Debug)]
pub struct StrategyEventHandlers {
    /// 订单事件的主题字符串。
    pub order_topic: String,
    /// 订单事件的处理器。
    pub order_handler: TypedHandler<OrderEventAny>,
    /// 持仓事件的主题字符串。
    pub position_topic: String,
    /// 持仓事件的处理器。
    pub position_handler: TypedHandler<PositionEvent>,
}

/// [`ExecutionAlgorithm`](super::ExecutionAlgorithm) 的核心组件。
///
/// 该结构体管理执行算法的内部状态，包括
/// 生成 ID 跟踪和策略订阅。它封装了 [`DataActorCore`]
/// 以提供数据参与者（Actor）功能。
///
/// 用户算法应将其作为成员持有，并实现 `Deref`/`DerefMut`
/// 以满足 [`ExecutionAlgorithm`](super::ExecutionAlgorithm) 的 trait 约束。
pub struct ExecutionAlgorithmCore {
    /// 底层数据参与者核心。
    pub actor: DataActorCore,
    /// 执行算法配置。
    pub config: ExecutionAlgorithmConfig,
    /// 执行算法 ID。
    pub exec_algorithm_id: ExecAlgorithmId,
    /// 将主订单客户 ID 映射到其生成的序列计数器。
    exec_spawn_ids: AHashMap<ClientOrderId, u32>,
    /// 跟踪已订阅事件的策略。
    subscribed_strategies: AHashSet<StrategyId>,
    /// 跟踪待处理的生成削减，以便在被拒或驳回时恢复数量。
    pending_spawn_reductions: AHashMap<ClientOrderId, Quantity>,
    /// 将策略映射到其事件处理器，以便在重置时进行清理。
    strategy_event_handlers: AHashMap<StrategyId, StrategyEventHandlers>,
}

impl Debug for ExecutionAlgorithmCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(ExecutionAlgorithmCore))
            .field("actor", &self.actor)
            .field("config", &self.config)
            .field("exec_algorithm_id", &self.exec_algorithm_id)
            .field("exec_spawn_ids", &self.exec_spawn_ids.len())
            .field("subscribed_strategies", &self.subscribed_strategies.len())
            .field(
                "pending_spawn_reductions",
                &self.pending_spawn_reductions.len(),
            )
            .field(
                "strategy_event_handlers",
                &self.strategy_event_handlers.len(),
            )
            .finish()
    }
}

impl ExecutionAlgorithmCore {
    /// 创建一个新的 [`ExecutionAlgorithmCore`] 实例。
    ///
    /// # Panics
    ///
    /// 如果 `config.exec_algorithm_id` 为 `None` 则会触发 Panic。
    #[must_use]
    pub fn new(config: ExecutionAlgorithmConfig) -> Self {
        let exec_algorithm_id = config
            .exec_algorithm_id
            .expect("ExecutionAlgorithmConfig 必须设置 exec_algorithm_id");

        let actor_config = DataActorConfig {
            actor_id: Some(ActorId::from(exec_algorithm_id.inner().as_str())),
            log_events: config.log_events,
            log_commands: config.log_commands,
        };

        Self {
            actor: DataActorCore::new(actor_config),
            config,
            exec_algorithm_id,
            exec_spawn_ids: AHashMap::new(),
            subscribed_strategies: AHashSet::new(),
            pending_spawn_reductions: AHashMap::new(),
            strategy_event_handlers: AHashMap::new(),
        }
    }

    /// 向交易引擎组件注册执行算法。
    ///
    /// # Errors
    ///
    /// 如果与参与者核心注册失败，则返回错误。
    pub fn register(
        &mut self,
        trader_id: TraderId,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<()> {
        self.actor.register(trader_id, clock, cache)
    }

    /// 返回执行算法 ID。
    #[must_use]
    pub fn id(&self) -> ExecAlgorithmId {
        self.exec_algorithm_id
    }

    /// 为主订单生成下一个子订单（Spawn）客户订单 ID。
    ///
    /// 生成的 ID 遵循以下模式：`{primary_id}-E{sequence}`。
    #[must_use]
    pub fn spawn_client_order_id(&mut self, primary_id: &ClientOrderId) -> ClientOrderId {
        let sequence = self
            .exec_spawn_ids
            .entry(*primary_id)
            .and_modify(|s| *s += 1)
            .or_insert(1);

        ClientOrderId::new(format!("{primary_id}-E{sequence}"))
    }

    /// 返回主订单的当前生成序列（如果有）。
    #[must_use]
    pub fn spawn_sequence(&self, primary_id: &ClientOrderId) -> Option<u32> {
        self.exec_spawn_ids.get(primary_id).copied()
    }

    /// 检查策略是否已订阅事件。
    #[must_use]
    pub fn is_strategy_subscribed(&self, strategy_id: &StrategyId) -> bool {
        self.subscribed_strategies.contains(strategy_id)
    }

    /// 将策略标记为已订阅事件。
    pub fn add_subscribed_strategy(&mut self, strategy_id: StrategyId) {
        self.subscribed_strategies.insert(strategy_id);
    }

    /// 存储策略订阅的事件处理器。
    pub fn store_strategy_event_handlers(
        &mut self,
        strategy_id: StrategyId,
        handlers: StrategyEventHandlers,
    ) {
        self.strategy_event_handlers.insert(strategy_id, handlers);
    }

    /// 取出并返回所有存储的策略事件处理器，并清空内部映射。
    pub fn take_strategy_event_handlers(&mut self) -> AHashMap<StrategyId, StrategyEventHandlers> {
        std::mem::take(&mut self.strategy_event_handlers)
    }

    /// 清除所有生成 ID 跟踪状态。
    pub fn clear_spawn_ids(&mut self) {
        self.exec_spawn_ids.clear();
    }

    /// 清除所有策略订阅。
    pub fn clear_subscribed_strategies(&mut self) {
        self.subscribed_strategies.clear();
    }

    /// 跟踪待处理的生成削减，以便进行潜在恢复。
    pub fn track_pending_spawn_reduction(&mut self, spawn_id: ClientOrderId, quantity: Quantity) {
        self.pending_spawn_reductions.insert(spawn_id, quantity);
    }

    /// 移除并返回订单的待处理生成削减（如果有）。
    pub fn take_pending_spawn_reduction(&mut self, spawn_id: &ClientOrderId) -> Option<Quantity> {
        self.pending_spawn_reductions.remove(spawn_id)
    }

    /// 清开所有待处理的生成削减。
    pub fn clear_pending_spawn_reductions(&mut self) {
        self.pending_spawn_reductions.clear();
    }

    /// 将核心重置为初始状态。
    ///
    /// 注意：此操作会清除处理器存储，但不会取消订阅消息总线。
    /// 请先调用 `unsubscribe_all_strategy_events` 以正确取消订阅。
    pub fn reset(&mut self) {
        self.exec_spawn_ids.clear();
        self.subscribed_strategies.clear();
        self.pending_spawn_reductions.clear();
        self.strategy_event_handlers.clear();
    }

    /// 从缓存中返回给定客户订单 ID 的订单。
    ///
    /// # Errors
    ///
    /// 如果在缓存中未找到订单，则返回错误。
    pub fn get_order(&self, client_order_id: &ClientOrderId) -> anyhow::Result<OrderAny> {
        self.cache()
            .order(client_order_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("缓存中未找到订单 {client_order_id}"))
    }

    /// Returns all orders for the given order list from the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if any order is not found in the cache.
    pub fn get_orders_for_list(&self, order_list: &OrderList) -> anyhow::Result<Vec<OrderAny>> {
        order_list
            .client_order_ids
            .iter()
            .map(|id| self.get_order(id))
            .collect()
    }
}

impl Deref for ExecutionAlgorithmCore {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.actor
    }
}

impl DerefMut for ExecutionAlgorithmCore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.actor
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn create_test_config() -> ExecutionAlgorithmConfig {
        ExecutionAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new("TWAP")),
            ..Default::default()
        }
    }

    #[rstest]
    fn test_core_new() {
        let config = create_test_config();
        let core = ExecutionAlgorithmCore::new(config.clone());

        assert_eq!(core.exec_algorithm_id, ExecAlgorithmId::new("TWAP"));
        assert_eq!(core.config.log_events, config.log_events);
        assert!(core.exec_spawn_ids.is_empty());
        assert!(core.subscribed_strategies.is_empty());
    }

    #[rstest]
    fn test_spawn_client_order_id_sequence() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let primary_id = ClientOrderId::new("O-001");

        let spawn1 = core.spawn_client_order_id(&primary_id);
        assert_eq!(spawn1.as_str(), "O-001-E1");

        let spawn2 = core.spawn_client_order_id(&primary_id);
        assert_eq!(spawn2.as_str(), "O-001-E2");

        let spawn3 = core.spawn_client_order_id(&primary_id);
        assert_eq!(spawn3.as_str(), "O-001-E3");
    }

    #[rstest]
    fn test_spawn_client_order_id_different_primaries() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let primary1 = ClientOrderId::new("O-001");
        let primary2 = ClientOrderId::new("O-002");

        let spawn1_1 = core.spawn_client_order_id(&primary1);
        let spawn2_1 = core.spawn_client_order_id(&primary2);
        let spawn1_2 = core.spawn_client_order_id(&primary1);

        assert_eq!(spawn1_1.as_str(), "O-001-E1");
        assert_eq!(spawn2_1.as_str(), "O-002-E1");
        assert_eq!(spawn1_2.as_str(), "O-001-E2");
    }

    #[rstest]
    fn test_spawn_sequence() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let primary_id = ClientOrderId::new("O-001");

        assert_eq!(core.spawn_sequence(&primary_id), None);

        let _ = core.spawn_client_order_id(&primary_id);
        assert_eq!(core.spawn_sequence(&primary_id), Some(1));

        let _ = core.spawn_client_order_id(&primary_id);
        assert_eq!(core.spawn_sequence(&primary_id), Some(2));
    }

    #[rstest]
    fn test_strategy_subscription_tracking() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let strategy_id = StrategyId::new("TEST-001");

        assert!(!core.is_strategy_subscribed(&strategy_id));

        core.add_subscribed_strategy(strategy_id);
        assert!(core.is_strategy_subscribed(&strategy_id));
    }

    #[rstest]
    fn test_clear_spawn_ids() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let primary_id = ClientOrderId::new("O-001");
        let _ = core.spawn_client_order_id(&primary_id);

        assert!(core.spawn_sequence(&primary_id).is_some());

        core.clear_spawn_ids();
        assert!(core.spawn_sequence(&primary_id).is_none());
    }

    #[rstest]
    fn test_reset() {
        let config = create_test_config();
        let mut core = ExecutionAlgorithmCore::new(config);

        let primary_id = ClientOrderId::new("O-001");
        let strategy_id = StrategyId::new("TEST-001");

        let _ = core.spawn_client_order_id(&primary_id);
        core.add_subscribed_strategy(strategy_id);

        core.reset();

        assert!(core.spawn_sequence(&primary_id).is_none());
        assert!(!core.is_strategy_subscribed(&strategy_id));
    }

    #[rstest]
    fn test_deref_to_data_actor_core() {
        let config = create_test_config();
        let core = ExecutionAlgorithmCore::new(config);

        // 应该能够通过 Deref 访问 DataActorCore 方法
        assert!(core.trader_id().is_none());
    }
}
