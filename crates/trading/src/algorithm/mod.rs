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

//! 执行算法基础设施，用于订单拆分和执行优化。
//!
//! 此模块提供了 [`ExecutionAlgorithm`] trait 以及支持 TWAP（时间加权平均价格）和 VWAP
//! （成交量加权平均价格）等算法的基础设施，这些算法将大订单拆分为更小的子订单。
//!
//! # 架构
//!
//! 执行算法扩展了 [`DataActor`]（而不是 [`Strategy`](super::Strategy)），因为：
//! - 它们不持有仓位（父策略持有）。
//! - 生成的子订单携带的是父策略的 ID，而不是算法的 ID。
//! - 它们充当订单处理器/转换器，而不是仓位管理器。
//!
//! # 订单流程
//!
//! 1. 策略提交一个设置了 `exec_algorithm_id` 的订单。
//! 2. 订单被路由到算法的 `{id}.execute` 端点。
//! 3. 算法通过 `on_order()` 接收订单。
//! 4. 算法使用 `spawn_market()`、`spawn_limit()` 等生成子订单。
//! 5. 生成的子订单通过 RiskEngine 提交。
//! 6. 算法接收成交事件并管理剩余数量。

pub mod config;
pub mod core;
pub mod iceberg;
pub mod is;
pub mod pov;
pub mod twap;
pub mod vwap;

pub use core::{ExecutionAlgorithmCore, StrategyEventHandlers};

pub use config::ExecutionAlgorithmConfig;
use nautilus_common::{
    actor::{DataActor, registry::try_get_actor_unchecked},
    enums::ComponentState,
    logging::{CMD, EVT, RECV, SEND},
    messages::execution::{CancelOrder, ModifyOrder, SubmitOrder, TradingCommand},
    msgbus::{self, MessagingSwitchboard, TypedHandler},
    timer::TimeEvent,
};
use nautilus_core::{UUID4, UnixNanos};
use nautilus_model::{
    enums::{OrderStatus, TimeInForce, TriggerType},
    events::{
        OrderAccepted, OrderCancelRejected, OrderCanceled, OrderDenied, OrderEmulated,
        OrderEventAny, OrderExpired, OrderFilled, OrderInitialized, OrderModifyRejected,
        OrderPendingCancel, OrderPendingUpdate, OrderRejected, OrderReleased, OrderSubmitted,
        OrderTriggered, OrderUpdated, PositionChanged, PositionClosed, PositionEvent,
        PositionOpened,
    },
    identifiers::{ClientId, ExecAlgorithmId, PositionId, StrategyId},
    orders::{LimitOrder, MarketOrder, MarketToLimitOrder, Order, OrderAny, OrderList},
    types::{Price, Quantity},
};
pub use iceberg::{IcebergAlgorithm, IcebergAlgorithmConfig};
pub use is::{IsAlgorithm, IsAlgorithmConfig};
pub use pov::{PovAlgorithm, PovAlgorithmConfig};
pub use twap::{TwapAlgorithm, TwapAlgorithmConfig};
pub use vwap::{VwapAlgorithm, VwapAlgorithmConfig};
use ustr::Ustr;

/// 用于在 NautilusTrader 中实现执行算法的核心 trait。
///
/// 执行算法是专门的 [`DataActor`]，它们从策略接收订单并通过生成子订单来执行。
/// 它们用于 TWAP 和 VWAP 等订单拆分算法。
///
/// # 关键能力
///
/// - 所有 [`DataActor`] 的能力（数据订阅、事件处理、定时器）
/// - 订单生成（市价、限价、市价转限价）
/// - 订单生命周期管理（提交、修改、取消）
/// - 针对算法持有订单的事件过滤
///
/// # 实现
///
/// 用户算法应实现所需的方法并持有一个 [`ExecutionAlgorithmCore`] 成员。
/// 结构体应 `Deref` 和 `DerefMut` 到 `ExecutionAlgorithmCore`
/// （其本身又解引用到 `DataActorCore`）。
pub trait ExecutionAlgorithm: DataActor {
    /// 提供对内部 `ExecutionAlgorithmCore` 的可变访问。
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore;

    /// 返回执行算法 ID。
    fn id(&mut self) -> ExecAlgorithmId {
        self.core_mut().exec_algorithm_id
    }

    /// 执行交易命令。
    ///
    /// 这是路由到算法的命令的主要入口点。
    /// 根据命令类型分发到相应的处理器。
    ///
    /// 命令仅在算法处于 `Running` 状态时才会被处理。
    ///
    /// # Errors
    ///
    /// 如果命令处理失败，则返回错误。
    fn execute(&mut self, command: TradingCommand) -> anyhow::Result<()>
    where
        Self: 'static + std::fmt::Debug + Sized,
    {
        let core = self.core_mut();
        if core.config.log_commands {
            let id = &core.actor.actor_id;
            log::info!("{id} {RECV}{CMD} {command:?}");
        }

        if core.state() != ComponentState::Running {
            return Ok(());
        }

        match command {
            TradingCommand::SubmitOrder(cmd) => {
                self.subscribe_to_strategy_events(cmd.strategy_id);
                let order = self.core_mut().get_order(&cmd.client_order_id)?;
                self.on_order(order)
            }
            TradingCommand::SubmitOrderList(cmd) => {
                self.subscribe_to_strategy_events(cmd.strategy_id);
                self.on_order_list(cmd.order_list)
            }
            TradingCommand::CancelOrder(cmd) => self.handle_cancel_order(cmd),
            _ => {
                log::warn!("未处理的命令类型: {command:?}");
                Ok(())
            }
        }
    }

    /// 当接收到执行的主订单时被调用。
    ///
    /// 覆盖此方法以实现算法的订单拆分逻辑。
    ///
    /// # Errors
    ///
    /// 如果订单处理失败，则返回错误。
    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()>;

    /// 当接收到执行的订单列表时被调用。
    ///
    /// 覆盖此方法以处理订单列表。默认实现
    /// 会逐个处理每个订单。
    ///
    /// # Errors
    ///
    /// 如果订单列表处理失败，则返回错误。
    fn on_order_list(&mut self, order_list: OrderList) -> anyhow::Result<()> {
        for order in order_list.orders {
            self.on_order(order)?;
        }
        Ok(())
    }

    /// 处理算法管理订单的取消订单命令。
    ///
    /// 这会生成一个内部取消事件并发布它。订单
    /// 会在本地取消，而不会向执行引擎发送命令。
    ///
    /// # Errors
    ///
    /// 如果取消失败，则返回错误。
    fn handle_cancel_order(&mut self, command: CancelOrder) -> anyhow::Result<()> {
        let (mut order, is_pending_cancel) = {
            let cache = self.core_mut().cache();

            let Some(order) = cache.order(&command.client_order_id) else {
                log::warn!("无法取消订单：缓存中未找到 {}", command.client_order_id);
                return Ok(());
            };

            let is_pending = cache.is_order_pending_cancel_local(&command.client_order_id);
            (order.clone(), is_pending)
        };

        if is_pending_cancel {
            return Ok(());
        }

        if order.is_closed() {
            log::warn!("订单已关闭，针对命令：{command:?}");
            return Ok(());
        }

        let event = self.generate_order_canceled(&order);

        if let Err(e) = order.apply(OrderEventAny::Canceled(event)) {
            log::warn!("状态转换失败（InvalidStateTrigger）：{e}，未应用取消事件");
            return Ok(());
        }

        {
            let cache_rc = self.core_mut().cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&order)?;
        }

        let topic = format!("events.order.{}", order.strategy_id());
        msgbus::publish_order_event(topic.into(), &OrderEventAny::Canceled(event));

        Ok(())
    }

    /// 为订单生成 OrderCanceled 事件。
    fn generate_order_canceled(&mut self, order: &OrderAny) -> OrderCanceled {
        let ts_now = self.core_mut().clock().timestamp_ns();

        OrderCanceled::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            order.venue_order_id(),
            order.account_id(),
        )
    }

    /// 为订单生成 OrderPendingUpdate 事件。
    fn generate_order_pending_update(&mut self, order: &OrderAny) -> OrderPendingUpdate {
        let ts_now = self.core_mut().clock().timestamp_ns();

        OrderPendingUpdate::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            order
                .account_id()
                .expect("待处理更新的订单必须拥有 account_id"),
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            order.venue_order_id(),
        )
    }

    /// 为订单生成 OrderPendingCancel 事件。
    fn generate_order_pending_cancel(&mut self, order: &OrderAny) -> OrderPendingCancel {
        let ts_now = self.core_mut().clock().timestamp_ns();

        OrderPendingCancel::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            order
                .account_id()
                .expect("待处理取消的订单必须拥有 account_id"),
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            order.venue_order_id(),
        )
    }

    /// 从主订单生成市价单。
    ///
    /// 创建一个新的市价单，具有：
    /// - 唯一的客户订单 ID：`{primary_id}-E{sequence}`。
    /// - 主订单的交易员 ID、策略 ID 和仪表 ID。
    /// - 算法的 exec_algorithm_id。
    /// - exec_spawn_id 设置为主订单的客户订单 ID。
    ///
    /// 如果 `reduce_primary` 为真，主订单的数量将减少生成的数量。
    /// 如果生成的订单随后被拒或被驳回（在接受之前），扣除的数量会自动恢复到主订单。
    fn spawn_market(
        &mut self,
        primary: &mut OrderAny,
        quantity: Quantity,
        time_in_force: TimeInForce,
        reduce_only: bool,
        tags: Option<Vec<Ustr>>,
        reduce_primary: bool,
    ) -> MarketOrder {
        // Generate spawn ID first so we can track the reduction
        let core = self.core_mut();
        let client_order_id = core.spawn_client_order_id(&primary.client_order_id());
        let ts_init = core.clock().timestamp_ns();
        let exec_algorithm_id = core.exec_algorithm_id;

        if reduce_primary {
            self.reduce_primary_order(primary, quantity);
            self.core_mut()
                .track_pending_spawn_reduction(client_order_id, quantity);
        }

        MarketOrder::new(
            primary.trader_id(),
            primary.strategy_id(),
            primary.instrument_id(),
            client_order_id,
            primary.order_side(),
            quantity,
            time_in_force,
            UUID4::new(),
            ts_init,
            reduce_only,
            false, // quote_quantity
            primary.contingency_type(),
            primary.order_list_id(),
            primary.linked_order_ids().map(|ids| ids.to_vec()),
            primary.parent_order_id(),
            Some(exec_algorithm_id),
            primary.exec_algorithm_params().cloned(),
            Some(primary.client_order_id()),
            tags.or_else(|| primary.tags().map(|t| t.to_vec())),
        )
    }

    /// 从主订单生成限价单。
    ///
    /// 创建一个新的限价单，具有：
    /// - 唯一的客户订单 ID：`{primary_id}-E{sequence}`
    /// - 主订单的交易员 ID、策略 ID 和仪表 ID
    /// - 算法的 exec_algorithm_id
    /// - exec_spawn_id 设置为主订单的客户订单 ID
    ///
    /// 如果 `reduce_primary` 为真，主订单的数量将减少生成的数量。
    /// 如果生成的订单随后被拒或被驳回（在接受之前），扣除的数量会
    /// 自动恢复到主订单。
    #[allow(clippy::too_many_arguments)]
    fn spawn_limit(
        &mut self,
        primary: &mut OrderAny,     // 父订单（主订单），将被修改状态或减少数量
        quantity: Quantity,         // 新生成的限价单数量
        price: Price,               // 限价单的价格
        time_in_force: TimeInForce, // 订单时效策略（如 GTC, IOC, FOK）
        expire_time: Option<UnixNanos>, // (可选) 订单过期时间，通常配合 GTD 使用
        post_only: bool,            // 是否仅作为挂单（Maker），如果不成则撤单
        reduce_only: bool,          // 是否仅用于减仓（不会增加头寸）
        display_qty: Option<Quantity>, // (可选) 冰山订单的显示数量
        emulation_trigger: Option<TriggerType>, // (可选) 模拟触发条件
        tags: Option<Vec<Ustr>>,    // (可选) 订单标签，用于追踪或分类
        reduce_primary: bool,       // 标志位：是否从主订单中扣除对应数量
    ) -> LimitOrder {
        // Generate spawn ID first so we can track the reduction
        let core = self.core_mut();
        let client_order_id = core.spawn_client_order_id(&primary.client_order_id());
        let ts_init = core.clock().timestamp_ns();
        let exec_algorithm_id = core.exec_algorithm_id;

        if reduce_primary {
            self.reduce_primary_order(primary, quantity);
            self.core_mut()
                .track_pending_spawn_reduction(client_order_id, quantity);
        }

        LimitOrder::new(
            primary.trader_id(),
            primary.strategy_id(),
            primary.instrument_id(),
            client_order_id,
            primary.order_side(),
            quantity,
            price,
            time_in_force,
            expire_time,
            post_only,
            reduce_only,
            false, // quote_quantity
            display_qty,
            emulation_trigger,
            None, // trigger_instrument_id
            primary.contingency_type(),
            primary.order_list_id(),
            primary.linked_order_ids().map(|ids| ids.to_vec()),
            primary.parent_order_id(),
            Some(exec_algorithm_id),
            primary.exec_algorithm_params().cloned(),
            Some(primary.client_order_id()),
            tags.or_else(|| primary.tags().map(|t| t.to_vec())),
            UUID4::new(),
            ts_init,
        )
    }

    /// 从主订单生成市价转限价单（Market-to-limit order）。
    ///
    /// 创建一个新的市价转限价单，具有：
    /// - 唯一的客户订单 ID：`{primary_id}-E{sequence}`
    /// - 主订单的交易员 ID、策略 ID 和仪表 ID
    /// - 算法的 exec_algorithm_id
    /// - exec_spawn_id 设置为主订单的客户订单 ID
    ///
    /// 如果 `reduce_primary` 为真，主订单的数量将减少生成的数量。
    /// 如果生成的订单随后被拒或被驳回（在接受之前），扣除的数量会
    /// 自动恢复到主订单。
    #[allow(clippy::too_many_arguments)]
    fn spawn_market_to_limit(
        &mut self,
        primary: &mut OrderAny,
        quantity: Quantity,
        time_in_force: TimeInForce,
        expire_time: Option<UnixNanos>,
        reduce_only: bool,
        display_qty: Option<Quantity>,
        emulation_trigger: Option<TriggerType>,
        tags: Option<Vec<Ustr>>,
        reduce_primary: bool,
    ) -> MarketToLimitOrder {
        // Generate spawn ID first so we can track the reduction
        let core = self.core_mut();
        let client_order_id = core.spawn_client_order_id(&primary.client_order_id());
        let ts_init = core.clock().timestamp_ns();
        let exec_algorithm_id = core.exec_algorithm_id;

        if reduce_primary {
            self.reduce_primary_order(primary, quantity);
            self.core_mut()
                .track_pending_spawn_reduction(client_order_id, quantity);
        }

        let mut order = MarketToLimitOrder::new(
            primary.trader_id(),
            primary.strategy_id(),
            primary.instrument_id(),
            client_order_id,
            primary.order_side(),
            quantity,
            time_in_force,
            expire_time,
            false, // post_only
            reduce_only,
            false, // quote_quantity
            display_qty,
            primary.contingency_type(),
            primary.order_list_id(),
            primary.linked_order_ids().map(|ids| ids.to_vec()),
            primary.parent_order_id(),
            Some(exec_algorithm_id),
            primary.exec_algorithm_params().cloned(),
            Some(primary.client_order_id()),
            tags.or_else(|| primary.tags().map(|t| t.to_vec())),
            UUID4::new(),
            ts_init,
        );

        if emulation_trigger.is_some() {
            order.set_emulation_trigger(emulation_trigger);
        }

        order
    }

    /// 按生成的子订单数量减少主订单的数量。
    ///
    /// 生成一个 `OrderUpdated` 事件并将其应用于主订单，
    /// 然后更新缓存中的订单。
    ///
    /// # Panics
    ///
    /// 如果 `spawn_qty` 超过了主订单的 `leaves_qty`（剩余待执行数量），则触发 Panic。
    fn reduce_primary_order(&mut self, primary: &mut OrderAny, spawn_qty: Quantity) {
        let leaves_qty = primary.leaves_qty();
        assert!(
            leaves_qty >= spawn_qty,
            "生成的子订单数量 {spawn_qty} 超过了主订单的剩余待执行数量 {leaves_qty}"
        );

        let primary_qty = primary.quantity();
        let new_qty = Quantity::from_raw(primary_qty.raw - spawn_qty.raw, primary_qty.precision);

        let core = self.core_mut();
        let ts_now = core.clock().timestamp_ns();

        let updated = OrderUpdated::new(
            primary.trader_id(),
            primary.strategy_id(),
            primary.instrument_id(),
            primary.client_order_id(),
            new_qty,
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            primary.venue_order_id(),
            primary.account_id(),
            None, // price
            None, // trigger_price
            None, // protection_price
        );

        primary
            .apply(OrderEventAny::Updated(updated))
            .expect("无法应用 OrderUpdated 事件");

        let cache_rc = core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache.update_order(primary).expect("更新缓存中的订单失败");
    }

    /// 在生成的子订单被拒或被驳回后恢复主订单的数量。
    ///
    /// 当生成的子订单在被接受之前失败时调用。从主订单中
    /// 扣除的数量会被恢复（最高恢复到子订单的 leaves_qty，以处理部分成交的情况）。
    fn restore_primary_order_quantity(&mut self, order: &OrderAny) {
        let Some(exec_spawn_id) = order.exec_spawn_id() else {
            return;
        };

        let reduction_qty = {
            let core = self.core_mut();
            core.take_pending_spawn_reduction(&order.client_order_id())
        };

        let Some(reduction_qty) = reduction_qty else {
            return;
        };

        let primary = {
            let cache = self.core_mut().cache();
            cache.order(&exec_spawn_id).cloned()
        };

        let Some(mut primary) = primary else {
            log::warn!("无法恢复主订单数量：未找到主订单 {exec_spawn_id}",);
            return;
        };

        // Cap restore amount by leaves_qty to handle partial fills before rejection
        let restore_raw = std::cmp::min(reduction_qty.raw, order.leaves_qty().raw);
        if restore_raw == 0 {
            return;
        }

        let restored_qty = Quantity::from_raw(
            primary.quantity().raw + restore_raw,
            primary.quantity().precision,
        );

        let core = self.core_mut();
        let ts_now = core.clock().timestamp_ns();

        let updated = OrderUpdated::new(
            primary.trader_id(),
            primary.strategy_id(),
            primary.instrument_id(),
            primary.client_order_id(),
            restored_qty,
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            primary.venue_order_id(),
            primary.account_id(),
            None, // price
            None, // trigger_price
            None, // protection_price
        );

        if let Err(e) = primary.apply(OrderEventAny::Updated(updated)) {
            log::warn!("数量恢复时应用 OrderUpdated 失败：{e}");
            return;
        }

        {
            let cache_rc = core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            if let Err(e) = cache.update_order(&primary) {
                log::warn!("在缓存中更新主订单失败：{e}");
                return;
            }
        }

        log::info!(
            "在生成的子订单 {} 被拒/驳回后，已将主订单 {} 的数量恢复至 {}",
            primary.client_order_id(),
            restored_qty,
            order.client_order_id()
        );
    }

    /// 通过风险引擎向执行引擎提交订单。
    ///
    /// # Errors
    ///
    /// 如果订单提交失败，则返回错误。
    fn submit_order(
        &mut self,
        order: OrderAny,
        position_id: Option<PositionId>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let ts_init = core.clock().timestamp_ns();

        // For spawned orders, use the parent's strategy ID
        let strategy_id = order.strategy_id();

        {
            let cache_rc = core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), position_id, client_id, true)?;
        }

        let command = SubmitOrder::new(
            trader_id,
            client_id,
            strategy_id,
            order.instrument_id(),
            order.client_order_id(),
            order.init_event().clone(),
            order.exec_algorithm_id(),
            position_id,
            None, // params
            UUID4::new(),
            ts_init,
        );

        if core.config.log_commands {
            let id = &core.actor.actor_id;
            log::info!("{id} {SEND}{CMD} {command:?}");
        }

        msgbus::send_trading_command(
            MessagingSwitchboard::risk_engine_execute(),
            TradingCommand::SubmitOrder(command),
        );

        Ok(())
    }

    /// 修改订单。
    ///
    /// # Errors
    ///
    /// 如果订单修改失败，则返回错误。
    fn modify_order(
        &mut self,
        order: &mut OrderAny,
        quantity: Option<Quantity>,
        price: Option<Price>,
        trigger_price: Option<Price>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        let qty_changing = quantity.is_some_and(|q| q != order.quantity());
        let price_changing = price.is_some() && price != order.price();
        let trigger_changing = trigger_price.is_some() && trigger_price != order.trigger_price();

        if !qty_changing && !price_changing && !trigger_changing {
            log::error!("无法创建 ModifyOrder 命令：数量、价格和触发价均为 None 或与现有值相同。");
            return Ok(());
        }

        if order.is_closed() || order.is_pending_cancel() {
            log::warn!(
                "无法创建 ModifyOrder 命令：状态为 {:?}, {order:?}",
                order.status()
            );
            return Ok(());
        }

        let core = self.core_mut();
        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = order.strategy_id();

        if !order.is_active_local() {
            let event = self.generate_order_pending_update(order);
            if let Err(e) = order.apply(OrderEventAny::PendingUpdate(event)) {
                log::warn!("状态转换失败（InvalidStateTrigger）：{e}，未应用待处理更新事件");
                return Ok(());
            }

            {
                let cache_rc = self.core_mut().cache_rc();
                let mut cache = cache_rc.borrow_mut();
                cache.update_order(order).ok();
            }

            let topic = format!("events.order.{strategy_id}");
            msgbus::publish_order_event(topic.into(), &OrderEventAny::PendingUpdate(event));
        }

        let ts_init = self.core_mut().clock().timestamp_ns();
        let command = ModifyOrder::new(
            trader_id,
            client_id,
            strategy_id,
            order.instrument_id(),
            order.client_order_id(),
            order.venue_order_id(),
            quantity,
            price,
            trigger_price,
            UUID4::new(),
            ts_init,
            None, // params
        );

        if self.core_mut().config.log_commands {
            let id = &self.core_mut().actor.actor_id;
            log::info!("{id} {SEND}{CMD} {command:?}");
        }

        let has_emulation_trigger = order
            .emulation_trigger()
            .is_some_and(|t| t != TriggerType::NoTrigger);

        if order.is_emulated() || has_emulation_trigger {
            msgbus::send_trading_command(
                MessagingSwitchboard::order_emulator_execute(),
                TradingCommand::ModifyOrder(command),
            );
        } else {
            msgbus::send_trading_command(
                MessagingSwitchboard::risk_engine_execute(),
                TradingCommand::ModifyOrder(command),
            );
        }

        Ok(())
    }

    /// 在本地修改 INITIALIZED（已初始化）或 RELEASED（已释放）状态的订单，而不发送命令。
    ///
    /// 这对于在提交前调整订单参数非常有用。通过应用 `OrderUpdated`
    /// 事件并更新缓存来本地更新订单。
    ///
    /// 至少有一个参数必须与当前订单值不同。
    ///
    /// # Errors
    ///
    /// 如果订单状态不是 INITIALIZED 或 RELEASED，
    /// 或者没有任何参数发生变化，则返回错误。
    fn modify_order_in_place(
        &mut self,
        order: &mut OrderAny,
        quantity: Option<Quantity>,
        price: Option<Price>,
        trigger_price: Option<Price>,
    ) -> anyhow::Result<()> {
        // Validate order status
        let status = order.status();
        if status != OrderStatus::Initialized && status != OrderStatus::Released {
            anyhow::bail!(
                "无法在本地修改订单：状态为 {status:?}，预期为已初始化（INITIALIZED）或已释放（RELEASED）"
            );
        }

        // Validate order type compatibility
        if price.is_some() && order.price().is_none() {
            anyhow::bail!(
                "无法在本地修改订单：{} 类型订单没有限价（LIMIT price）",
                order.order_type()
            );
        }

        if trigger_price.is_some() && order.trigger_price().is_none() {
            anyhow::bail!(
                "无法在本地修改订单：{} 类型订单没有止损触发价（STOP trigger price）",
                order.order_type()
            );
        }

        // Check if any value would actually change
        let qty_changing = quantity.is_some_and(|q| q != order.quantity());
        let price_changing = price.is_some() && price != order.price();
        let trigger_changing = trigger_price.is_some() && trigger_price != order.trigger_price();

        if !qty_changing && !price_changing && !trigger_changing {
            anyhow::bail!("无法在本地修改订单：没有参数与当前值不同");
        }

        let core = self.core_mut();
        let ts_now = core.clock().timestamp_ns();

        let updated = OrderUpdated::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            quantity.unwrap_or_else(|| order.quantity()),
            UUID4::new(),
            ts_now,
            ts_now,
            false, // reconciliation
            order.venue_order_id(),
            order.account_id(),
            price,
            trigger_price,
            None, // protection_price
        );

        order
            .apply(OrderEventAny::Updated(updated))
            .map_err(|e| anyhow::anyhow!("应用 OrderUpdated 失败：{e}"))?;

        let cache_rc = core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache.update_order(order)?;

        Ok(())
    }

    /// 取消订单。
    ///
    /// # Errors
    ///
    /// 如果取消订单失败，则返回错误。
    fn cancel_order(
        &mut self,
        order: &mut OrderAny,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        if order.is_closed() || order.is_pending_cancel() {
            log::warn!("无法取消订单：状态为 {:?}, {order:?}", order.status());
            return Ok(());
        }

        let core = self.core_mut();
        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = order.strategy_id();

        if !order.is_active_local() {
            let event = self.generate_order_pending_cancel(order);
            if let Err(e) = order.apply(OrderEventAny::PendingCancel(event)) {
                log::warn!("状态转换失败（InvalidStateTrigger）：{e}，未应用待处理取消事件");
                return Ok(());
            }

            {
                let cache_rc = self.core_mut().cache_rc();
                let mut cache = cache_rc.borrow_mut();
                cache.update_order(order).ok();
            }

            let topic = format!("events.order.{strategy_id}");
            msgbus::publish_order_event(topic.into(), &OrderEventAny::PendingCancel(event));
        }

        let ts_init = self.core_mut().clock().timestamp_ns();
        let command = CancelOrder::new(
            trader_id,
            client_id,
            strategy_id,
            order.instrument_id(),
            order.client_order_id(),
            order.venue_order_id(),
            UUID4::new(),
            ts_init,
            None, // params
        );

        if self.core_mut().config.log_commands {
            let id = &self.core_mut().actor.actor_id;
            log::info!("{id} {SEND}{CMD} {command:?}");
        }

        let has_emulation_trigger = order
            .emulation_trigger()
            .is_some_and(|t| t != TriggerType::NoTrigger);

        if order.is_emulated() || order.status() == OrderStatus::Released || has_emulation_trigger {
            msgbus::send_trading_command(
                MessagingSwitchboard::order_emulator_execute(),
                TradingCommand::CancelOrder(command),
            );
        } else {
            msgbus::send_trading_command(
                MessagingSwitchboard::exec_engine_execute(),
                TradingCommand::CancelOrder(command),
            );
        }

        Ok(())
    }

    /// 订阅来自策略的事件。
    ///
    /// 当从策略接收到第一个订单时会自动调用此方法。
    fn subscribe_to_strategy_events(&mut self, strategy_id: StrategyId)
    where
        Self: 'static + std::fmt::Debug + Sized,
    {
        let core = self.core_mut();
        if core.is_strategy_subscribed(&strategy_id) {
            return;
        }

        let actor_id = core.actor.actor_id.inner();

        let order_topic = format!("events.order.{strategy_id}");
        let order_actor_id = actor_id;
        let order_handler = TypedHandler::from(move |event: &OrderEventAny| {
            if let Some(mut algo) = try_get_actor_unchecked::<Self>(&order_actor_id) {
                algo.handle_order_event(event.clone());
            } else {
                log::error!("执行算法 {order_actor_id} 未找到，无法进行订单事件处理");
            }
        });
        msgbus::subscribe_order_events(order_topic.clone().into(), order_handler.clone(), None);

        let position_topic = format!("events.position.{strategy_id}");
        let position_handler = TypedHandler::from(move |event: &PositionEvent| {
            if let Some(mut algo) = try_get_actor_unchecked::<Self>(&actor_id) {
                algo.handle_position_event(event.clone());
            } else {
                log::error!("执行算法 {actor_id} 未找到，无法进行持仓事件处理");
            }
        });
        msgbus::subscribe_position_events(
            position_topic.clone().into(),
            position_handler.clone(),
            None,
        );

        let handlers = StrategyEventHandlers {
            order_topic,
            order_handler,
            position_topic,
            position_handler,
        };
        core.store_strategy_event_handlers(strategy_id, handlers);

        core.add_subscribed_strategy(strategy_id);
        log::info!("已订阅策略 {strategy_id} 的事件");
    }

    /// 取消订阅所有策略事件处理器。
    ///
    /// 应在重置前调用此方法，以正确清理消息总线（msgbus）订阅。
    fn unsubscribe_all_strategy_events(&mut self) {
        let handlers = self.core_mut().take_strategy_event_handlers();
        for (strategy_id, h) in handlers {
            msgbus::unsubscribe_order_events(h.order_topic.into(), &h.order_handler);
            msgbus::unsubscribe_position_events(h.position_topic.into(), &h.position_handler);
            log::info!("已取消订阅策略 {strategy_id} 的事件");
        }
        self.core_mut().clear_subscribed_strategies();
    }

    /// 处理订单事件，并过滤属于该算法持有的订单。
    fn handle_order_event(&mut self, event: OrderEventAny) {
        if self.core_mut().state() != ComponentState::Running {
            return;
        }

        let order = {
            let cache = self.core_mut().cache();
            cache.order(&event.client_order_id()).cloned()
        };

        let Some(order) = order else {
            return;
        };

        let Some(order_algo_id) = order.exec_algorithm_id() else {
            return;
        };

        if order_algo_id != self.id() {
            return;
        }

        {
            let core = self.core_mut();
            if core.config.log_events {
                let id = &core.actor.actor_id;
                log::info!("{id} {RECV}{EVT} {event}");
            }
        }

        match &event {
            OrderEventAny::Initialized(e) => self.on_order_initialized(e.clone()),
            OrderEventAny::Denied(e) => {
                self.restore_primary_order_quantity(&order);
                self.on_order_denied(*e);
            }
            OrderEventAny::Emulated(e) => self.on_order_emulated(*e),
            OrderEventAny::Released(e) => self.on_order_released(*e),
            OrderEventAny::Submitted(e) => self.on_order_submitted(*e),
            OrderEventAny::Rejected(e) => {
                self.restore_primary_order_quantity(&order);
                self.on_order_rejected(*e);
            }
            OrderEventAny::Accepted(e) => {
                // Commit reduction - order accepted by venue
                self.core_mut()
                    .take_pending_spawn_reduction(&order.client_order_id());
                self.on_order_accepted(*e);
            }
            OrderEventAny::Canceled(e) => {
                self.core_mut()
                    .take_pending_spawn_reduction(&order.client_order_id());
                self.on_algo_order_canceled(*e);
            }
            OrderEventAny::Expired(e) => {
                self.core_mut()
                    .take_pending_spawn_reduction(&order.client_order_id());
                self.on_order_expired(*e);
            }
            OrderEventAny::Triggered(e) => self.on_order_triggered(*e),
            OrderEventAny::PendingUpdate(e) => self.on_order_pending_update(*e),
            OrderEventAny::PendingCancel(e) => self.on_order_pending_cancel(*e),
            OrderEventAny::ModifyRejected(e) => self.on_order_modify_rejected(*e),
            OrderEventAny::CancelRejected(e) => self.on_order_cancel_rejected(*e),
            OrderEventAny::Updated(e) => self.on_order_updated(*e),
            OrderEventAny::Filled(e) => self.on_algo_order_filled(*e),
        }

        self.on_order_event(event);
    }

    /// 处理持仓事件。
    fn handle_position_event(&mut self, event: PositionEvent) {
        if self.core_mut().state() != ComponentState::Running {
            return;
        }

        {
            let core = self.core_mut();
            if core.config.log_events {
                let id = &core.actor.actor_id;
                log::info!("{id} {RECV}{EVT} {event:?}");
            }
        }

        match &event {
            PositionEvent::PositionOpened(e) => self.on_position_opened(e.clone()),
            PositionEvent::PositionChanged(e) => self.on_position_changed(e.clone()),
            PositionEvent::PositionClosed(e) => self.on_position_closed(e.clone()),
            PositionEvent::PositionAdjusted(_) => {}
        }

        self.on_position_event(event);
    }

    /// 当算法启动时调用。
    ///
    /// 覆盖此方法以实现自定义初始化逻辑。
    ///
    /// # Errors
    ///
    /// 如果启动失败，则返回错误。
    fn on_start(&mut self) -> anyhow::Result<()> {
        let id = self.id();
        log::info!("正在启动 {id}");
        Ok(())
    }

    /// 当算法停止时调用。
    ///
    /// # Errors
    ///
    /// 如果停止失败，则返回错误。
    fn on_stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 当算法重置时调用。
    ///
    /// # Errors
    ///
    /// 如果重置失败，则返回错误。
    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.unsubscribe_all_strategy_events();
        self.core_mut().reset();
        Ok(())
    }

    /// 当接收到时间事件时调用。
    ///
    /// 对于像 TWAP 这样基于定时器的算法，请覆盖此方法。
    ///
    /// # Errors
    ///
    /// 如果时间事件处理失败，则返回错误。
    fn on_time_event(&mut self, _event: &TimeEvent) -> anyhow::Result<()> {
        Ok(())
    }

    /// 当订单已初始化时调用。
    #[allow(unused_variables)]
    fn on_order_initialized(&mut self, event: OrderInitialized) {}

    /// 当订单被拒绝（Denied）时调用。
    #[allow(unused_variables)]
    fn on_order_denied(&mut self, event: OrderDenied) {}

    /// 当订单被模拟（Emulated）时调用。
    #[allow(unused_variables)]
    fn on_order_emulated(&mut self, event: OrderEmulated) {}

    /// 当订单从模拟状态释放（Released）时调用。
    #[allow(unused_variables)]
    fn on_order_released(&mut self, event: OrderReleased) {}

    /// 当订单已提交时调用。
    #[allow(unused_variables)]
    fn on_order_submitted(&mut self, event: OrderSubmitted) {}

    /// 当订单被驳回（Rejected）时调用。
    #[allow(unused_variables)]
    fn on_order_rejected(&mut self, event: OrderRejected) {}

    /// 当订单被接受时调用。
    #[allow(unused_variables)]
    fn on_order_accepted(&mut self, event: OrderAccepted) {}

    /// 当订单被取消时调用。
    #[allow(unused_variables)]
    fn on_algo_order_canceled(&mut self, event: OrderCanceled) {}

    /// 当订单过期时调用。
    #[allow(unused_variables)]
    fn on_order_expired(&mut self, event: OrderExpired) {}

    /// 当订单触发时调用。
    #[allow(unused_variables)]
    fn on_order_triggered(&mut self, event: OrderTriggered) {}

    /// 当订单修改处于待处理状态时调用。
    #[allow(unused_variables)]
    fn on_order_pending_update(&mut self, event: OrderPendingUpdate) {}

    /// 当订单取消处于待处理状态时调用。
    #[allow(unused_variables)]
    fn on_order_pending_cancel(&mut self, event: OrderPendingCancel) {}

    /// 当订单修改被驳回时调用。
    #[allow(unused_variables)]
    fn on_order_modify_rejected(&mut self, event: OrderModifyRejected) {}

    /// 当订单取消被驳回时调用。
    #[allow(unused_variables)]
    fn on_order_cancel_rejected(&mut self, event: OrderCancelRejected) {}

    /// 当订单已更新时调用。
    #[allow(unused_variables)]
    fn on_order_updated(&mut self, event: OrderUpdated) {}

    /// 当订单已成交（Filled）时调用。
    #[allow(unused_variables)]
    fn on_algo_order_filled(&mut self, event: OrderFilled) {}

    /// 对任何订单事件调用（在特定处理器之后）。
    #[allow(unused_variables)]
    fn on_order_event(&mut self, event: OrderEventAny) {}

    /// 当持仓开启时调用。
    #[allow(unused_variables)]
    fn on_position_opened(&mut self, event: PositionOpened) {}

    /// 当持仓发生变化时调用。
    #[allow(unused_variables)]
    fn on_position_changed(&mut self, event: PositionChanged) {}

    /// 当持仓关闭时调用。
    #[allow(unused_variables)]
    fn on_position_closed(&mut self, event: PositionClosed) {}

    /// 对任何持仓事件调用（在特定处理器之后）。
    #[allow(unused_variables)]
    fn on_position_event(&mut self, event: PositionEvent) {}
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        ops::{Deref, DerefMut},
        rc::Rc,
    };

    use nautilus_common::{
        actor::{DataActor, DataActorCore},
        cache::Cache,
        clock::TestClock,
        component::Component,
        enums::ComponentTrigger,
    };
    use nautilus_model::{
        enums::OrderSide,
        events::{OrderAccepted, OrderCanceled, OrderDenied, OrderRejected},
        identifiers::{
            AccountId, ClientOrderId, ExecAlgorithmId, InstrumentId, StrategyId, TraderId,
            VenueOrderId,
        },
        orders::{LimitOrder, MarketOrder, OrderAny, stubs::TestOrderStubs},
        types::{Price, Quantity},
    };
    use rstest::rstest;

    use super::*;

    #[derive(Debug)]
    struct TestAlgorithm {
        core: ExecutionAlgorithmCore,
        on_order_called: bool,
        last_order_client_id: Option<ClientOrderId>,
    }

    impl TestAlgorithm {
        fn new(config: ExecutionAlgorithmConfig) -> Self {
            Self {
                core: ExecutionAlgorithmCore::new(config),
                on_order_called: false,
                last_order_client_id: None,
            }
        }
    }

    impl Deref for TestAlgorithm {
        type Target = DataActorCore;
        fn deref(&self) -> &Self::Target {
            &self.core.actor
        }
    }

    impl DerefMut for TestAlgorithm {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.core.actor
        }
    }

    impl DataActor for TestAlgorithm {}

    impl ExecutionAlgorithm for TestAlgorithm {
        fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
            &mut self.core
        }

        fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
            self.on_order_called = true;
            self.last_order_client_id = Some(order.client_order_id());
            Ok(())
        }
    }

    fn create_test_algorithm() -> TestAlgorithm {
        // 使用唯一 ID 以避免在并行测试中出现线程局部的注册表/消息总线冲突
        let unique_id = format!("TEST-{}", UUID4::new());
        let config = ExecutionAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        TestAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut TestAlgorithm) {
        let trader_id = TraderId::from("TRADER-001");
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));

        algo.core.register(trader_id, clock, cache).unwrap();

        // 为了测试切换到 Running 状态
        algo.transition_state(ComponentTrigger::Initialize).unwrap();
        algo.transition_state(ComponentTrigger::Start).unwrap();
        algo.transition_state(ComponentTrigger::StartCompleted)
            .unwrap();
    }

    #[rstest]
    fn test_algorithm_creation() {
        let algo = create_test_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("TEST-"));
        assert!(!algo.on_order_called);
        assert!(algo.last_order_client_id.is_none());
    }

    #[rstest]
    fn test_algorithm_registration() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        assert!(algo.core.trader_id().is_some());
        assert_eq!(algo.core.trader_id(), Some(TraderId::from("TRADER-001")));
    }

    #[rstest]
    fn test_algorithm_id() {
        let mut algo = create_test_algorithm();
        assert!(algo.id().inner().starts_with("TEST-"));
    }

    #[rstest]
    fn test_algorithm_spawn_market_creates_valid_order() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false, // reduce_only
            false, // quote_quantity
            None,  // contingency_type
            None,  // order_list_id
            None,  // linked_order_ids
            None,  // parent_order_id
            None,  // exec_algorithm_id
            None,  // exec_algorithm_params
            None,  // exec_spawn_id
            None,  // tags
        ));

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Ioc,
            false,
            None,  // tags
            false, // reduce_primary
        );

        assert_eq!(spawned.client_order_id.as_str(), "O-001-E1");
        assert_eq!(spawned.instrument_id, instrument_id);
        assert_eq!(spawned.order_side(), OrderSide::Buy);
        assert_eq!(spawned.quantity, Quantity::from("0.5"));
        assert_eq!(spawned.time_in_force, TimeInForce::Ioc);
        assert_eq!(spawned.exec_algorithm_id, Some(algo.id()));
        assert_eq!(spawned.exec_spawn_id, Some(ClientOrderId::from("O-001")));
    }

    #[rstest]
    fn test_algorithm_spawn_increments_sequence() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        let spawned1 = algo.spawn_market(
            &mut primary,
            Quantity::from("0.25"),
            TimeInForce::Ioc,
            false,
            None,
            false,
        );
        let spawned2 = algo.spawn_market(
            &mut primary,
            Quantity::from("0.25"),
            TimeInForce::Ioc,
            false,
            None,
            false,
        );
        let spawned3 = algo.spawn_market(
            &mut primary,
            Quantity::from("0.25"),
            TimeInForce::Ioc,
            false,
            None,
            false,
        );

        assert_eq!(spawned1.client_order_id.as_str(), "O-001-E1");
        assert_eq!(spawned2.client_order_id.as_str(), "O-001-E2");
        assert_eq!(spawned3.client_order_id.as_str(), "O-001-E3");
    }

    #[rstest]
    fn test_algorithm_default_handlers_do_not_panic() {
        let mut algo = create_test_algorithm();

        algo.on_order_initialized(Default::default());
        algo.on_order_denied(Default::default());
        algo.on_order_emulated(Default::default());
        algo.on_order_released(Default::default());
        algo.on_order_submitted(Default::default());
        algo.on_order_rejected(Default::default());
        algo.on_order_accepted(Default::default());
        algo.on_algo_order_canceled(Default::default());
        algo.on_order_expired(Default::default());
        algo.on_order_triggered(Default::default());
        algo.on_order_pending_update(Default::default());
        algo.on_order_pending_cancel(Default::default());
        algo.on_order_modify_rejected(Default::default());
        algo.on_order_cancel_rejected(Default::default());
        algo.on_order_updated(Default::default());
        algo.on_algo_order_filled(Default::default());
    }

    #[rstest]
    fn test_strategy_subscription_tracking() {
        let mut algo = create_test_algorithm();
        let strategy_id = StrategyId::from("STRAT-001");

        assert!(!algo.core.is_strategy_subscribed(&strategy_id));

        algo.subscribe_to_strategy_events(strategy_id);
        assert!(algo.core.is_strategy_subscribed(&strategy_id));

        // Second call should be idempotent
        algo.subscribe_to_strategy_events(strategy_id);
        assert!(algo.core.is_strategy_subscribed(&strategy_id));
    }

    #[rstest]
    fn test_algorithm_reset() {
        let mut algo = create_test_algorithm();
        let strategy_id = StrategyId::from("STRAT-001");
        let primary_id = ClientOrderId::new("O-001");

        let _ = algo.core.spawn_client_order_id(&primary_id);
        algo.core.add_subscribed_strategy(strategy_id);

        assert!(algo.core.spawn_sequence(&primary_id).is_some());
        assert!(algo.core.is_strategy_subscribed(&strategy_id));

        ExecutionAlgorithm::on_reset(&mut algo).unwrap();

        assert!(algo.core.spawn_sequence(&primary_id).is_none());
        assert!(!algo.core.is_strategy_subscribed(&strategy_id));
    }

    #[rstest]
    fn test_algorithm_spawn_limit_creates_valid_order() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        let price = Price::from("50000.0");
        let spawned = algo.spawn_limit(
            &mut primary,
            Quantity::from("0.5"),
            price,
            TimeInForce::Gtc,
            None,  // expire_time
            false, // post_only
            false, // reduce_only
            None,  // display_qty
            None,  // emulation_trigger
            None,  // tags
            false, // reduce_primary
        );

        assert_eq!(spawned.client_order_id.as_str(), "O-001-E1");
        assert_eq!(spawned.instrument_id, instrument_id);
        assert_eq!(spawned.order_side(), OrderSide::Buy);
        assert_eq!(spawned.quantity, Quantity::from("0.5"));
        assert_eq!(spawned.price, price);
        assert_eq!(spawned.time_in_force, TimeInForce::Gtc);
        assert_eq!(spawned.exec_algorithm_id, Some(algo.id()));
        assert_eq!(spawned.exec_spawn_id, Some(ClientOrderId::from("O-001")));
    }

    #[rstest]
    fn test_algorithm_spawn_market_to_limit_creates_valid_order() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        let spawned = algo.spawn_market_to_limit(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Gtc,
            None,  // expire_time
            false, // reduce_only
            None,  // display_qty
            None,  // emulation_trigger
            None,  // tags
            false, // reduce_primary
        );

        assert_eq!(spawned.client_order_id.as_str(), "O-001-E1");
        assert_eq!(spawned.instrument_id, instrument_id);
        assert_eq!(spawned.order_side(), OrderSide::Buy);
        assert_eq!(spawned.quantity, Quantity::from("0.5"));
        assert_eq!(spawned.time_in_force, TimeInForce::Gtc);
        assert_eq!(spawned.exec_algorithm_id, Some(algo.id()));
        assert_eq!(spawned.exec_spawn_id, Some(ClientOrderId::from("O-001")));
    }

    #[rstest]
    fn test_algorithm_spawn_market_with_tags() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        let tags = vec![ustr::Ustr::from("TAG1"), ustr::Ustr::from("TAG2")];
        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Ioc,
            false,
            Some(tags.clone()),
            false, // reduce_primary
        );

        assert_eq!(spawned.tags, Some(tags));
    }

    #[rstest]
    fn test_algorithm_reduce_primary_order() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let order = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        // 设置为已接受状态，以便可以应用 OrderUpdated
        let mut primary = TestOrderStubs::make_accepted_order(&order);

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawn_qty = Quantity::from("0.3");
        algo.reduce_primary_order(&mut primary, spawn_qty);

        assert_eq!(primary.quantity(), Quantity::from("0.7"));
    }

    #[rstest]
    fn test_algorithm_spawn_market_with_reduce_primary() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let order = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        // 设置为已接受状态，以便可以应用 OrderUpdated
        let mut primary = TestOrderStubs::make_accepted_order(&order);

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.4"),
            TimeInForce::Ioc,
            false,
            None,
            true, // reduce_primary = true
        );

        assert_eq!(spawned.quantity, Quantity::from("0.4"));
        assert_eq!(primary.quantity(), Quantity::from("0.6"));
    }

    #[rstest]
    fn test_algorithm_generate_order_canceled() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let order = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ));

        let event = algo.generate_order_canceled(&order);

        assert_eq!(event.trader_id, TraderId::from("TRADER-001"));
        assert_eq!(event.strategy_id, StrategyId::from("STRAT-001"));
        assert_eq!(event.instrument_id, InstrumentId::from("BTC/USDT.BINANCE"));
        assert_eq!(event.client_order_id, ClientOrderId::from("O-001"));
    }

    #[rstest]
    fn test_algorithm_modify_order_in_place_updates_quantity() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let mut order = OrderAny::Limit(LimitOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            Price::from("50000.0"),
            TimeInForce::Gtc,
            None,  // expire_time
            false, // post_only
            false, // reduce_only
            false, // quote_quantity
            None,  // display_qty
            None,  // emulation_trigger
            None,  // trigger_instrument_id
            None,  // contingency_type
            None,  // order_list_id
            None,  // linked_order_ids
            None,  // parent_order_id
            None,  // exec_algorithm_id
            None,  // exec_algorithm_params
            None,  // exec_spawn_id
            None,  // tags
            UUID4::new(),
            0.into(),
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false).unwrap();
        }

        let new_qty = Quantity::from("0.5");
        algo.modify_order_in_place(&mut order, Some(new_qty), None, None)
            .unwrap();

        assert_eq!(order.quantity(), new_qty);
    }

    #[rstest]
    fn test_algorithm_modify_order_in_place_rejects_no_changes() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let mut order = OrderAny::Limit(LimitOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            Price::from("50000.0"),
            TimeInForce::Gtc,
            None,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            UUID4::new(),
            0.into(),
        ));

        // 尝试使用相同的数量进行修改 - 应该失败
        let result =
            algo.modify_order_in_place(&mut order, Some(Quantity::from("1.0")), None, None);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("没有参数与当前值不同")
        );
    }

    #[rstest]
    fn test_spawned_order_denied_restores_primary_quantity() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        assert_eq!(primary.quantity(), Quantity::from("0.5"));

        let mut spawned_order = OrderAny::Market(spawned);
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache
                .add_order(spawned_order.clone(), None, None, false)
                .unwrap();
        }

        let denied = OrderDenied::new(
            spawned_order.trader_id(),
            spawned_order.strategy_id(),
            spawned_order.instrument_id(),
            spawned_order.client_order_id(),
            "TEST_DENIAL".into(),
            UUID4::new(),
            0.into(),
            0.into(),
        );

        spawned_order.apply(OrderEventAny::Denied(denied)).unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Denied(denied));

        let restored_primary = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(restored_primary.quantity(), Quantity::from("1.0"));
    }

    #[rstest]
    fn test_spawned_order_rejected_restores_primary_quantity() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        assert_eq!(primary.quantity(), Quantity::from("0.5"));

        let mut spawned_order = OrderAny::Market(spawned);
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache
                .add_order(spawned_order.clone(), None, None, false)
                .unwrap();
        }

        let rejected = OrderRejected::new(
            spawned_order.trader_id(),
            spawned_order.strategy_id(),
            spawned_order.instrument_id(),
            spawned_order.client_order_id(),
            AccountId::from("BINANCE-001"),
            "TEST_REJECTION".into(),
            UUID4::new(),
            0.into(),
            0.into(),
            false,
            false,
        );

        spawned_order
            .apply(OrderEventAny::Rejected(rejected))
            .unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Rejected(rejected));

        let restored_primary = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(restored_primary.quantity(), Quantity::from("1.0"));
    }

    #[rstest]
    fn test_spawned_order_with_reduce_primary_false_does_not_restore() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Fok,
            false,
            None,
            false,
        );

        assert_eq!(primary.quantity(), Quantity::from("1.0"));

        let mut spawned_order = OrderAny::Market(spawned);
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache
                .add_order(spawned_order.clone(), None, None, false)
                .unwrap();
        }

        let denied = OrderDenied::new(
            spawned_order.trader_id(),
            spawned_order.strategy_id(),
            spawned_order.instrument_id(),
            spawned_order.client_order_id(),
            "TEST_DENIAL".into(),
            UUID4::new(),
            0.into(),
            0.into(),
        );

        spawned_order.apply(OrderEventAny::Denied(denied)).unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Denied(denied));

        let final_primary = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(final_primary.quantity(), Quantity::from("1.0"));
    }

    #[rstest]
    fn test_multiple_spawns_with_one_denied_restores_correctly() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned1 = algo.spawn_market(
            &mut primary,
            Quantity::from("0.3"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        let spawned2 = algo.spawn_market(
            &mut primary,
            Quantity::from("0.4"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        assert_eq!(primary.quantity(), Quantity::from("0.3"));

        let spawned_order1 = OrderAny::Market(spawned1);
        let mut spawned_order2 = OrderAny::Market(spawned2);
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(spawned_order1, None, None, false).unwrap();
            cache
                .add_order(spawned_order2.clone(), None, None, false)
                .unwrap();
        }

        let denied = OrderDenied::new(
            spawned_order2.trader_id(),
            spawned_order2.strategy_id(),
            spawned_order2.instrument_id(),
            spawned_order2.client_order_id(),
            "TEST_DENIAL".into(),
            UUID4::new(),
            0.into(),
            0.into(),
        );

        spawned_order2.apply(OrderEventAny::Denied(denied)).unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order2).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Denied(denied));

        let restored_primary = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(restored_primary.quantity(), Quantity::from("0.7"));
    }

    #[rstest]
    fn test_spawned_order_accepted_prevents_restoration() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let spawned = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        assert_eq!(primary.quantity(), Quantity::from("0.5"));

        let mut spawned_order = OrderAny::Market(spawned);
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache
                .add_order(spawned_order.clone(), None, None, false)
                .unwrap();
        }

        let accepted = OrderAccepted::new(
            spawned_order.trader_id(),
            spawned_order.strategy_id(),
            spawned_order.instrument_id(),
            spawned_order.client_order_id(),
            VenueOrderId::from("V-123"),
            AccountId::from("BINANCE-001"),
            UUID4::new(),
            0.into(),
            0.into(),
            false,
        );

        spawned_order
            .apply(OrderEventAny::Accepted(accepted))
            .unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Accepted(accepted));

        let primary_after_accept = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(primary_after_accept.quantity(), Quantity::from("0.5"));

        // 接受后取消 - 不应发生恢复
        let canceled = OrderCanceled::new(
            spawned_order.trader_id(),
            spawned_order.strategy_id(),
            spawned_order.instrument_id(),
            spawned_order.client_order_id(),
            UUID4::new(),
            0.into(),
            0.into(),
            false,
            Some(VenueOrderId::from("V-123")),
            Some(AccountId::from("BINANCE-001")),
        );

        spawned_order
            .apply(OrderEventAny::Canceled(canceled))
            .unwrap();
        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&spawned_order).unwrap();
        }

        algo.handle_order_event(OrderEventAny::Canceled(canceled));

        let final_primary = {
            let cache = algo.core.cache();
            cache.order(&ClientOrderId::from("O-001")).cloned().unwrap()
        };
        assert_eq!(final_primary.quantity(), Quantity::from("0.5"));
    }

    #[rstest]
    #[should_panic(expected = "超过了主订单的剩余待执行数量")]
    fn test_spawn_quantity_exceeds_leaves_qty_panics() {
        let mut algo = create_test_algorithm();
        register_algorithm(&mut algo);

        let instrument_id = InstrumentId::from("BTC/USDT.BINANCE");
        let exec_algorithm_id = algo.id();

        let mut primary = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            instrument_id,
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(exec_algorithm_id),
            None,
            None,
            None,
        ));

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(primary.clone(), None, None, false).unwrap();
        }

        let _ = algo.spawn_market(
            &mut primary,
            Quantity::from("0.8"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary).unwrap();
        }

        assert_eq!(primary.quantity(), Quantity::from("0.2"));
        assert_eq!(primary.leaves_qty(), Quantity::from("0.2"));

        // 应该触发 Panic - 当只剩 0.2 的 leaves_qty 时生成 0.5
        let _ = algo.spawn_market(
            &mut primary,
            Quantity::from("0.5"),
            TimeInForce::Fok,
            false,
            None,
            true,
        );
    }
}
