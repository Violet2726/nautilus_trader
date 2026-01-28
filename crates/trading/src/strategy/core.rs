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

use std::{
    cell::RefCell,
    fmt::Debug,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use ahash::AHashMap;
use nautilus_common::{
    actor::{DataActorConfig, DataActorCore},
    cache::Cache,
    clock::Clock,
    factories::OrderFactory,
};
use nautilus_execution::order_manager::manager::OrderManager;
use nautilus_model::identifiers::{ActorId, ClientOrderId, StrategyId, TraderId};
use nautilus_portfolio::portfolio::Portfolio;
use ustr::Ustr;

use super::config::StrategyConfig;

/// [`Strategy`](super::Strategy) 的核心组件，管理数据、订单和状态。
///
/// 该结构体旨在作为成员持有在用户的自定义策略结构体中。
/// 用户的结构体应通过 `Deref` 和 `DerefMut` 解引用到此 `StrategyCore` 实例，
/// 以满足 [`Strategy`](super::Strategy) 和
/// [`DataActor`](nautilus_common::actor::data_actor::DataActor) 的 trait 约束。
pub struct StrategyCore {
    /// 底层数据参与者（Actor）核心。
    pub actor: DataActorCore,
    /// 策略配置。
    pub config: StrategyConfig,
    /// 订单管理器。
    pub order_manager: Option<OrderManager>,
    /// 订单工厂。
    pub order_factory: Option<OrderFactory>,
    /// 投资组合。
    pub portfolio: Option<Rc<RefCell<Portfolio>>>,
    /// 将客户订单 ID 映射到 GTD 过期定时器名称。
    pub gtd_timers: AHashMap<ClientOrderId, Ustr>,
}

impl Debug for StrategyCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(StrategyCore))
            .field("actor", &self.actor)
            .field("config", &self.config)
            .field("order_manager", &self.order_manager)
            .field("order_factory", &self.order_factory)
            .finish()
    }
}

impl StrategyCore {
    /// 创建一个新的 [`StrategyCore`] 实例。
    pub fn new(config: StrategyConfig) -> Self {
        let actor_config = DataActorConfig {
            actor_id: config
                .strategy_id
                .map(|id| ActorId::from(id.inner().as_str())),
            log_events: config.log_events,
            log_commands: config.log_commands,
        };

        Self {
            actor: DataActorCore::new(actor_config),
            config,
            order_manager: None,
            order_factory: None,
            portfolio: None,
            gtd_timers: AHashMap::new(),
        }
    }

    /// 向交易引擎组件注册策略。
    ///
    /// 这通常由框架在策略添加到引擎时调用。
    ///
    /// # Errors
    ///
    /// 如果与参与者核心注册失败，则返回错误。
    pub fn register(
        &mut self,
        trader_id: TraderId,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
        portfolio: Rc<RefCell<Portfolio>>,
    ) -> anyhow::Result<()> {
        self.actor
            .register(trader_id, clock.clone(), cache.clone())?;

        let strategy_id = StrategyId::from(self.actor.actor_id.inner().as_str());

        self.order_factory = Some(OrderFactory::new(
            trader_id,
            strategy_id,
            None,
            None,
            clock.clone(),
            self.config.use_uuid_client_order_ids,
            self.config.use_hyphens_in_client_order_ids,
        ));

        self.order_manager = Some(OrderManager::new(clock, cache, false, None, None, None));

        self.portfolio = Some(portfolio);

        Ok(())
    }
}

impl Deref for StrategyCore {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.actor
    }
}

impl DerefMut for StrategyCore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.actor
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use nautilus_common::{cache::Cache, clock::TestClock};
    use nautilus_model::identifiers::{StrategyId, TraderId};
    use nautilus_portfolio::portfolio::Portfolio;
    use rstest::rstest;

    use super::*;

    fn create_test_config() -> StrategyConfig {
        StrategyConfig {
            strategy_id: Some(StrategyId::from("TEST-001")),
            order_id_tag: Some("001".to_string()),
            ..Default::default()
        }
    }

    #[rstest]
    fn test_strategy_core_new() {
        let config = create_test_config();
        let core = StrategyCore::new(config.clone());

        assert_eq!(core.config.strategy_id, config.strategy_id);
        assert_eq!(core.config.order_id_tag, config.order_id_tag);
        assert!(core.order_manager.is_none());
        assert!(core.order_factory.is_none());
        assert!(core.portfolio.is_none());
    }

    #[rstest]
    fn test_strategy_core_register() {
        let config = create_test_config();
        let mut core = StrategyCore::new(config);

        let trader_id = TraderId::from("TRADER-001");
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));
        let portfolio = Rc::new(RefCell::new(Portfolio::new(
            cache.clone(),
            clock.clone(),
            None,
        )));

        let result = core.register(trader_id, clock, cache, portfolio);
        assert!(result.is_ok());

        assert!(core.order_manager.is_some());
        assert!(core.order_factory.is_some());
        assert!(core.portfolio.is_some());
        assert_eq!(core.trader_id(), Some(trader_id));
    }

    #[rstest]
    fn test_strategy_core_deref() {
        let config = create_test_config();
        let core = StrategyCore::new(config);

        assert!(core.trader_id().is_none());
    }

    #[rstest]
    fn test_strategy_core_debug() {
        let config = create_test_config();
        let core = StrategyCore::new(config);

        let debug_str = format!("{core:?}");
        assert!(debug_str.contains("StrategyCore"));
    }
}
