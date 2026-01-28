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

pub mod config;
pub mod core;

pub use core::StrategyCore;

pub use config::StrategyConfig;
use indexmap::IndexMap;
use nautilus_common::{
    actor::DataActor,
    logging::{EVT, RECV},
    messages::execution::{
        BatchCancelOrders, CancelAllOrders, CancelOrder, ModifyOrder, QueryAccount, QueryOrder,
        SubmitOrder, SubmitOrderList, TradingCommand,
    },
    msgbus,
    timer::TimeEvent,
};
use nautilus_core::UUID4;
use nautilus_model::{
    enums::{OrderSide, OrderStatus, PositionSide, TimeInForce, TriggerType},
    events::{
        OrderAccepted, OrderCancelRejected, OrderDenied, OrderEmulated, OrderEventAny,
        OrderExpired, OrderInitialized, OrderModifyRejected, OrderPendingCancel,
        OrderPendingUpdate, OrderRejected, OrderReleased, OrderSubmitted, OrderTriggered,
        OrderUpdated, PositionChanged, PositionClosed, PositionEvent, PositionOpened,
    },
    identifiers::{AccountId, ClientId, ClientOrderId, InstrumentId, PositionId, StrategyId},
    orders::{Order, OrderAny, OrderCore, OrderList},
    position::Position,
    types::{Price, Quantity},
};
use ustr::Ustr;

/// NautilusTrader 中实现交易策略的核心 trait。
///
/// 策略是专门的 [`DataActor`]，它结合了数据摄入能力与
/// 全面的订单和持仓管理功能。通过实现此 trait，
/// 自定义策略可以访问包括订单提交、修改、取消和持仓管理在内的
/// 完整交易执行栈。
///
/// # 关键能力
///
/// - 所有 [`DataActor`] 能力（数据订阅、事件处理、定时器）。
/// - 订单生命周期管理（提交、修改、取消）。
/// - 持仓管理（开仓、平仓、监控）。
/// - 访问交易缓存和投资组合。
/// - 将事件路由到订单管理器和模拟器。
///
/// # 实现
///
/// 用户策略应实现 [`Strategy::core_mut`] 方法以提供
/// 对其内部 [`StrategyCore`] 的访问，该核心处理与
/// 交易引擎的集成。所有订单和持仓管理方法均作为默认实现提供。
pub trait Strategy: DataActor {
    /// 提供对内部 `StrategyCore` 的可变访问。
    ///
    /// 此方法必须由用户的策略结构体实现，通常是
    /// 返回其 `StrategyCore` 成员的可变引用。
    fn core_mut(&mut self) -> &mut StrategyCore;

    /// 返回此策略的外部订单认领声明。
    ///
    /// 这些是在对账期间，其外部订单应被此策略认领的交易工具 ID。
    fn external_order_claims(&self) -> Option<Vec<InstrumentId>> {
        None
    }

    /// 提交订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单提交失败，则返回错误。
    fn submit_order(
        &mut self,
        order: OrderAny,
        position_id: Option<PositionId>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        self.submit_order_with_params(order, position_id, client_id, IndexMap::new())
    }

    /// 提交带特定适配器参数的订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单提交失败，则返回错误。
    fn submit_order_with_params(
        &mut self,
        order: OrderAny,
        position_id: Option<PositionId>,
        client_id: Option<ClientId>,
        params: IndexMap<String, String>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();

        let params = if params.is_empty() {
            None
        } else {
            Some(params)
        };

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
            params,
            UUID4::new(),
            ts_init,
        );

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        if matches!(order.emulation_trigger(), Some(trigger) if trigger != TriggerType::NoTrigger) {
            manager.send_emulator_command(TradingCommand::SubmitOrder(command));
        } else if order.exec_algorithm_id().is_some() {
            manager.send_algo_command(command, order.exec_algorithm_id().unwrap());
        } else {
            manager.send_risk_command(TradingCommand::SubmitOrder(command));
        }

        self.set_gtd_expiry(&order)?;
        Ok(())
    }

    /// 提交订单列表。
    ///
    /// # Errors
    ///
    /// 如果策略未注册、订单列表无效或订单列表提交失败，则返回错误。
    fn submit_order_list(
        &mut self,
        order_list: OrderList,
        position_id: Option<PositionId>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();
        {
            let cache_rc = core.cache_rc();
            let cache = cache_rc.borrow();
            if cache.order_list_exists(&order_list.id) {
                anyhow::bail!("拒绝订单列表：重复的 {}", order_list.id);
            }

            for order in &order_list.orders {
                if order.status() != OrderStatus::Initialized {
                    anyhow::bail!(
                        "拒绝列表中的订单：{} 状态无效，预期为 INITIALIZED",
                        order.client_order_id()
                    );
                }
                if cache.order_exists(&order.client_order_id()) {
                    anyhow::bail!("拒绝列表中的订单：重复的 {}", order.client_order_id());
                }
            }
        }

        {
            let cache_rc = core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order_list(order_list.clone())?;
            for order in &order_list.orders {
                cache.add_order(order.clone(), position_id, client_id, true)?;
            }
        }

        let first_order = order_list.orders.first();
        let exec_algorithm_id = first_order.and_then(|o| o.exec_algorithm_id());

        let command = SubmitOrderList::new(
            trader_id,
            client_id,
            strategy_id,
            order_list.instrument_id,
            order_list.clone(),
            exec_algorithm_id,
            position_id,
            None, // params
            UUID4::new(),
            ts_init,
        );

        let has_emulated_order = order_list.orders.iter().any(|o| {
            matches!(o.emulation_trigger(), Some(trigger) if trigger != TriggerType::NoTrigger)
                || o.is_emulated()
        });

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        if has_emulated_order {
            manager.send_emulator_command(TradingCommand::SubmitOrderList(command));
        } else if let Some(algo_id) = exec_algorithm_id {
            let endpoint = format!("{algo_id}.execute");
            msgbus::send_any(endpoint.into(), &TradingCommand::SubmitOrderList(command));
        } else {
            manager.send_risk_command(TradingCommand::SubmitOrderList(command));
        }

        for order in &order_list.orders {
            self.set_gtd_expiry(order)?;
        }

        Ok(())
    }

    /// 提交带特定适配器参数的订单列表。
    ///
    /// # Errors
    ///
    /// 如果策略未注册、订单列表无效或订单列表提交失败，则返回错误。
    fn submit_order_list_with_params(
        &mut self,
        order_list: OrderList,
        position_id: Option<PositionId>,
        client_id: Option<ClientId>,
        params: IndexMap<String, String>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();
        {
            let cache_rc = core.cache_rc();
            let cache = cache_rc.borrow();
            if cache.order_list_exists(&order_list.id) {
                anyhow::bail!("拒绝订单列表：重复的 {}", order_list.id);
            }

            for order in &order_list.orders {
                if order.status() != OrderStatus::Initialized {
                    anyhow::bail!(
                        "拒绝列表中的订单：{} 状态无效，预期为 INITIALIZED",
                        order.client_order_id()
                    );
                }
                if cache.order_exists(&order.client_order_id()) {
                    anyhow::bail!("拒绝列表中的订单：重复的 {}", order.client_order_id());
                }
            }
        }

        {
            let cache_rc = core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order_list(order_list.clone())?;
            for order in &order_list.orders {
                cache.add_order(order.clone(), position_id, client_id, true)?;
            }
        }

        let params_opt = if params.is_empty() {
            None
        } else {
            Some(params)
        };

        let first_order = order_list.orders.first();
        let exec_algorithm_id = first_order.and_then(|o| o.exec_algorithm_id());

        let command = SubmitOrderList::new(
            trader_id,
            client_id,
            strategy_id,
            order_list.instrument_id,
            order_list.clone(),
            exec_algorithm_id,
            position_id,
            params_opt,
            UUID4::new(),
            ts_init,
        );

        let has_emulated_order = order_list.orders.iter().any(|o| {
            matches!(o.emulation_trigger(), Some(trigger) if trigger != TriggerType::NoTrigger)
                || o.is_emulated()
        });

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        if has_emulated_order {
            manager.send_emulator_command(TradingCommand::SubmitOrderList(command));
        } else if let Some(algo_id) = exec_algorithm_id {
            let endpoint = format!("{algo_id}.execute");
            msgbus::send_any(endpoint.into(), &TradingCommand::SubmitOrderList(command));
        } else {
            manager.send_risk_command(TradingCommand::SubmitOrderList(command));
        }

        for order in &order_list.orders {
            self.set_gtd_expiry(order)?;
        }

        Ok(())
    }

    /// 修改订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单修改失败，则返回错误。
    fn modify_order(
        &mut self,
        order: OrderAny,
        quantity: Option<Quantity>,
        price: Option<Price>,
        trigger_price: Option<Price>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        self.modify_order_with_params(
            order,
            quantity,
            price,
            trigger_price,
            client_id,
            IndexMap::new(),
        )
    }

    /// 修改带特定适配器参数的订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单修改失败，则返回错误。
    fn modify_order_with_params(
        &mut self,
        order: OrderAny,
        quantity: Option<Quantity>,
        price: Option<Price>,
        trigger_price: Option<Price>,
        client_id: Option<ClientId>,
        params: IndexMap<String, String>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();

        let params = if params.is_empty() {
            None
        } else {
            Some(params)
        };

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
            params,
        );

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        if matches!(order.emulation_trigger(), Some(trigger) if trigger != TriggerType::NoTrigger) {
            manager.send_emulator_command(TradingCommand::ModifyOrder(command));
        } else if order.exec_algorithm_id().is_some() {
            manager.send_risk_command(TradingCommand::ModifyOrder(command));
        } else {
            manager.send_exec_command(TradingCommand::ModifyOrder(command));
        }
        Ok(())
    }

    /// 取消订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单取消失败，则返回错误。
    fn cancel_order(&mut self, order: OrderAny, client_id: Option<ClientId>) -> anyhow::Result<()> {
        self.cancel_order_with_params(order, client_id, IndexMap::new())
    }

    /// 取消带特定适配器参数的订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单取消失败，则返回错误。
    fn cancel_order_with_params(
        &mut self,
        order: OrderAny,
        client_id: Option<ClientId>,
        params: IndexMap<String, String>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();

        let params = if params.is_empty() {
            None
        } else {
            Some(params)
        };

        let command = CancelOrder::new(
            trader_id,
            client_id,
            strategy_id,
            order.instrument_id(),
            order.client_order_id(),
            order.venue_order_id(),
            UUID4::new(),
            ts_init,
            params,
        );

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        if matches!(order.emulation_trigger(), Some(trigger) if trigger != TriggerType::NoTrigger)
            || order.is_emulated()
        {
            manager.send_emulator_command(TradingCommand::CancelOrder(command));
        } else if let Some(algo_id) = order.exec_algorithm_id() {
            let endpoint = format!("{algo_id}.execute");
            msgbus::send_any(endpoint.into(), &TradingCommand::CancelOrder(command));
        } else {
            manager.send_exec_command(TradingCommand::CancelOrder(command));
        }
        Ok(())
    }

    /// 批量取消同一交易工具的多个订单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册、订单跨越多个交易工具、或包含模拟/本地订单，则返回错误。
    fn cancel_orders(
        &mut self,
        mut orders: Vec<OrderAny>,
        client_id: Option<ClientId>,
        params: Option<IndexMap<String, String>>,
    ) -> anyhow::Result<()> {
        if orders.is_empty() {
            anyhow::bail!("无法批量取消空订单列表");
        }

        let core = self.core_mut();
        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        let first = orders.remove(0);
        let instrument_id = first.instrument_id();

        if first.is_emulated() || first.is_active_local() {
            anyhow::bail!("批量取消中不能包含模拟或本地订单");
        }

        let mut cancels = Vec::with_capacity(orders.len() + 1);
        cancels.push(CancelOrder::new(
            trader_id,
            client_id,
            strategy_id,
            instrument_id,
            first.client_order_id(),
            first.venue_order_id(),
            UUID4::new(),
            ts_init,
            params.clone(),
        ));

        for order in orders {
            if order.instrument_id() != instrument_id {
                anyhow::bail!(
                    "无法批量取消不同交易工具的订单：{} vs {}",
                    instrument_id,
                    order.instrument_id()
                );
            }

            if order.is_emulated() || order.is_active_local() {
                anyhow::bail!("批量取消中不能包含模拟或本地订单");
            }

            cancels.push(CancelOrder::new(
                trader_id,
                client_id,
                strategy_id,
                instrument_id,
                order.client_order_id(),
                order.venue_order_id(),
                UUID4::new(),
                ts_init,
                params.clone(),
            ));
        }

        let command = BatchCancelOrders::new(
            trader_id,
            client_id,
            strategy_id,
            instrument_id,
            cancels,
            UUID4::new(),
            ts_init,
            params,
        );

        manager.send_exec_command(TradingCommand::BatchCancelOrders(command));
        Ok(())
    }

    /// 取消给定交易工具的所有挂单。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单取消失败，则返回错误。
    fn cancel_all_orders(
        &mut self,
        instrument_id: InstrumentId,
        order_side: Option<OrderSide>,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        self.cancel_all_orders_with_params(instrument_id, order_side, client_id, IndexMap::new())
    }

    /// 取消给定交易工具的所有挂单，带特定适配器参数。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或订单取消失败，则返回错误。
    fn cancel_all_orders_with_params(
        &mut self,
        instrument_id: InstrumentId,
        order_side: Option<OrderSide>,
        client_id: Option<ClientId>,
        params: IndexMap<String, String>,
    ) -> anyhow::Result<()> {
        let params = if params.is_empty() {
            None
        } else {
            Some(params)
        };
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();
        let cache = core.cache();

        let open_orders = cache.orders_open(
            None,
            Some(&instrument_id),
            Some(&strategy_id),
            None,
            order_side,
        );

        let emulated_orders = cache.orders_emulated(
            None,
            Some(&instrument_id),
            Some(&strategy_id),
            None,
            order_side,
        );

        let inflight_orders = cache.orders_inflight(
            None,
            Some(&instrument_id),
            Some(&strategy_id),
            None,
            order_side,
        );

        let exec_algorithm_ids = cache.exec_algorithm_ids();
        let mut algo_orders = Vec::new();

        for algo_id in &exec_algorithm_ids {
            let orders = cache.orders_for_exec_algorithm(
                algo_id,
                None,
                Some(&instrument_id),
                Some(&strategy_id),
                None,
                order_side,
            );
            algo_orders.extend(orders.iter().map(|o| (*o).clone()));
        }

        let open_count = open_orders.len();
        let emulated_count = emulated_orders.len();
        let inflight_count = inflight_orders.len();
        let algo_count = algo_orders.len();

        drop(cache);

        if open_count == 0 && emulated_count == 0 && inflight_count == 0 && algo_count == 0 {
            let side_str = order_side.map(|s| format!(" {s}")).unwrap_or_default();
            log::info!("没有 {instrument_id} 的活跃或模拟{side_str} 订单可取消");
            return Ok(());
        }

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        let side_str = order_side.map(|s| format!(" {s}")).unwrap_or_default();

        if open_count > 0 {
            log::info!("正在取消 {open_count} 个活跃的{side_str} {instrument_id} 订单",);

        if inflight_count > 0 {
            log::info!(
                "Canceling {inflight_count} inflight{side_str} {instrument_id} order{}",
                if inflight_count == 1 { "" } else { "s" }
            );
        }

        if open_count > 0 || inflight_count > 0 {
            let command = CancelAllOrders::new(
                trader_id,
                client_id,
                strategy_id,
                instrument_id,
                order_side.unwrap_or(OrderSide::NoOrderSide),
                UUID4::new(),
                ts_init,
                params.clone(),
            );

            manager.send_exec_command(TradingCommand::CancelAllOrders(command));
        }

        if emulated_count > 0 {
            log::info!("正在取消 {emulated_count} 个模拟的{side_str} {instrument_id} 订单",);

            let command = CancelAllOrders::new(
                trader_id,
                client_id,
                strategy_id,
                instrument_id,
                order_side.unwrap_or(OrderSide::NoOrderSide),
                UUID4::new(),
                ts_init,
                params,
            );

            manager.send_emulator_command(TradingCommand::CancelAllOrders(command));
        }

        for order in algo_orders {
            self.cancel_order(order, client_id)?;
        }

        Ok(())
    }

    /// 通过提交反向市价单来平仓。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或平仓失败，则返回错误。
    fn close_position(
        &mut self,
        position: &Position,
        client_id: Option<ClientId>,
        tags: Option<Vec<Ustr>>,
        time_in_force: Option<TimeInForce>,
        reduce_only: Option<bool>,
        quote_quantity: Option<bool>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();
        let Some(order_factory) = &mut core.order_factory else {
            anyhow::bail!("Strategy not registered: OrderFactory missing");
        };

        if position.is_closed() {
            log::warn!("无法平仓（已平仓）：{}", position.id);
            return Ok(());
        }

        let closing_side = OrderCore::closing_side(position.side);

        let order = order_factory.market(
            position.instrument_id,
            closing_side,
            position.quantity,
            time_in_force,
            reduce_only.or(Some(true)),
            quote_quantity,
            None,
            None,
            tags,
            None,
        );

        self.submit_order(order, Some(position.id), client_id)
    }

    /// 平掉给定交易工具的所有持仓。
    ///
    /// # Errors
    ///
    /// 如果策略未注册或平仓失败，则返回错误。
    #[allow(clippy::too_many_arguments)]
    fn close_all_positions(
        &mut self,
        instrument_id: InstrumentId,
        position_side: Option<PositionSide>,
        client_id: Option<ClientId>,
        tags: Option<Vec<Ustr>>,
        time_in_force: Option<TimeInForce>,
        reduce_only: Option<bool>,
        quote_quantity: Option<bool>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let cache = core.cache();

        let positions_open = cache.positions_open(
            None,
            Some(&instrument_id),
            Some(&strategy_id),
            None,
            position_side,
        );

        let side_str = position_side.map(|s| format!(" {s}")).unwrap_or_default();

        if positions_open.is_empty() {
            log::info!("没有 {instrument_id} 的活跃{side_str} 持仓可平配");
            return Ok(());
        }

        let count = positions_open.len();
        log::info!("正在平掉 {count} 个活跃的{side_str} 持仓",);

        let positions_data: Vec<_> = positions_open
            .iter()
            .map(|p| (p.id, p.instrument_id, p.side, p.quantity, p.is_closed()))
            .collect();

        drop(cache);

        for (pos_id, pos_instrument_id, pos_side, pos_quantity, is_closed) in positions_data {
            if is_closed {
                continue;
            }

            let core = self.core_mut();
            let Some(order_factory) = &mut core.order_factory else {
                anyhow::bail!("Strategy not registered: OrderFactory missing");
            };

            let closing_side = OrderCore::closing_side(pos_side);
            let order = order_factory.market(
                pos_instrument_id,
                closing_side,
                pos_quantity,
                time_in_force,
                reduce_only.or(Some(true)),
                quote_quantity,
                None,
                None,
                tags.clone(),
                None,
            );

            self.submit_order(order, Some(pos_id), client_id)?;
        }

        Ok(())
    }

    /// 从执行客户端查询账户状态。
    ///
    /// 创建一个 [`QueryAccount`] 命令并发送到执行引擎，
    /// 执行引擎会向执行客户端请求当前账户状态。
    ///
    /// # Errors
    ///
    /// 如果策略未注册，则返回错误。
    fn query_account(
        &mut self,
        account_id: AccountId,
        client_id: Option<ClientId>,
    ) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let ts_init = core.clock().timestamp_ns();

        let command = QueryAccount::new(trader_id, client_id, account_id, UUID4::new(), ts_init);

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        manager.send_exec_command(TradingCommand::QueryAccount(command));
        Ok(())
    }

    /// 从执行客户端查询订单状态。
    ///
    /// 创建一个 [`QueryOrder`] 命令并发送到执行引擎，
    /// 执行引擎会向执行客户端请求当前订单状态。
    ///
    /// # Errors
    ///
    /// 如果策略未注册，则返回错误。
    fn query_order(&mut self, order: &OrderAny, client_id: Option<ClientId>) -> anyhow::Result<()> {
        let core = self.core_mut();

        let trader_id = core.trader_id().expect("未设置交易员 ID");
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let ts_init = core.clock().timestamp_ns();

        let command = QueryOrder::new(
            trader_id,
            client_id,
            strategy_id,
            order.instrument_id(),
            order.client_order_id(),
            order.venue_order_id(),
            UUID4::new(),
            ts_init,
        );

        let Some(manager) = &mut core.order_manager else {
            anyhow::bail!("Strategy not registered: OrderManager missing");
        };

        manager.send_exec_command(TradingCommand::QueryOrder(command));
        Ok(())
    }

    /// 处理订单事件，分发到相应的处理器，并路由至订单管理器。
    fn handle_order_event(&mut self, event: OrderEventAny) {
        {
            let core = self.core_mut();
            if core.config.log_events {
                let id = &core.actor.actor_id;
                log::info!("{id} {RECV}{EVT} {event}");
            }
        }

        let client_order_id = event.client_order_id();
        let is_terminal = matches!(
            &event,
            OrderEventAny::Filled(_)
                | OrderEventAny::Canceled(_)
                | OrderEventAny::Rejected(_)
                | OrderEventAny::Expired(_)
                | OrderEventAny::Denied(_)
        );

        match &event {
            OrderEventAny::Initialized(e) => self.on_order_initialized(e.clone()),
            OrderEventAny::Denied(e) => self.on_order_denied(*e),
            OrderEventAny::Emulated(e) => self.on_order_emulated(*e),
            OrderEventAny::Released(e) => self.on_order_released(*e),
            OrderEventAny::Submitted(e) => self.on_order_submitted(*e),
            OrderEventAny::Rejected(e) => self.on_order_rejected(*e),
            OrderEventAny::Accepted(e) => self.on_order_accepted(*e),
            OrderEventAny::Canceled(e) => {
                let _ = DataActor::on_order_canceled(self, e);
            }
            OrderEventAny::Expired(e) => self.on_order_expired(*e),
            OrderEventAny::Triggered(e) => self.on_order_triggered(*e),
            OrderEventAny::PendingUpdate(e) => self.on_order_pending_update(*e),
            OrderEventAny::PendingCancel(e) => self.on_order_pending_cancel(*e),
            OrderEventAny::ModifyRejected(e) => self.on_order_modify_rejected(*e),
            OrderEventAny::CancelRejected(e) => self.on_order_cancel_rejected(*e),
            OrderEventAny::Updated(e) => self.on_order_updated(*e),
            OrderEventAny::Filled(e) => {
                let _ = DataActor::on_order_filled(self, e);
            }
        }

        if is_terminal {
            self.cancel_gtd_expiry(&client_order_id);
        }

        let core = self.core_mut();
        if let Some(manager) = &mut core.order_manager {
            manager.handle_event(event);
        }
    }

    /// 处理持仓事件，分发到相应的处理器。
    fn handle_position_event(&mut self, event: PositionEvent) {
        {
            let core = self.core_mut();
            if core.config.log_events {
                let id = &core.actor.actor_id;
                log::info!("{id} {RECV}{EVT} {event:?}");
            }
        }

        match event {
            PositionEvent::PositionOpened(e) => self.on_position_opened(e),
            PositionEvent::PositionChanged(e) => self.on_position_changed(e),
            PositionEvent::PositionClosed(e) => self.on_position_closed(e),
            PositionEvent::PositionAdjusted(_) => {
                // 尚未为此事件实现处理器
            }
        }
    }

    // -- LIFECYCLE METHODS -----------------------------------------------------------------------

    /// 当策略启动时调用。
    ///
    /// 覆盖此方法以实现自定义初始化逻辑。
    /// 如果启用了 `manage_gtd_expiry`，默认实现会重新激活 GTD 定时器。
    ///
    /// # Errors
    ///
    /// 如果策略初始化失败，则返回错误。
    fn on_start(&mut self) -> anyhow::Result<()> {
        let core = self.core_mut();
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        log::info!("正在启动 {strategy_id}");

        if core.config.manage_gtd_expiry {
            self.reactivate_gtd_timers();
        }

        Ok(())
    }

    /// 当接收到时间事件时调用。
    ///
    /// 将 GTD 过期定时器事件路由到过期处理器。
    ///
    /// # Errors
    ///
    /// 如果时间事件处理失败，则返回错误。
    fn on_time_event(&mut self, event: &TimeEvent) -> anyhow::Result<()> {
        if event.name.starts_with("GTD-EXPIRY:") {
            self.expire_gtd_order(event.clone());
        }
        Ok(())
    }

    // -- EVENT HANDLERS --------------------------------------------------------------------------

    /// 当订单初始化时调用。
    ///
    /// 覆盖此方法以实现当订单首次创建时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_initialized(&mut self, event: OrderInitialized) {}

    /// 当订单被系统拒绝时调用。
    ///
    /// 覆盖此方法以实现当订单在提交前被拒绝时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_denied(&mut self, event: OrderDenied) {}

    /// 当订单被模拟时调用。
    ///
    /// 覆盖此方法以实现当订单被模拟器接管时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_emulated(&mut self, event: OrderEmulated) {}

    /// 当订单从模拟中释放时调用。
    ///
    /// 覆盖此方法以实现当模拟订单被释放时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_released(&mut self, event: OrderReleased) {}

    /// 当订单提交到交易场所时调用。
    ///
    /// 覆盖此方法以实现当订单提交时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_submitted(&mut self, event: OrderSubmitted) {}

    /// 当订单被交易场所驳回时调用。
    ///
    /// 覆盖此方法以实现当订单被驳回时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_rejected(&mut self, event: OrderRejected) {}

    /// 当订单被交易场所接受时调用。
    ///
    /// 覆盖此方法以实现当订单被接受时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_accepted(&mut self, event: OrderAccepted) {}

    /// 当订单过期时调用。
    ///
    /// 覆盖此方法以实现当订单过期时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_expired(&mut self, event: OrderExpired) {}

    /// 当订单触发时调用。
    ///
    /// 覆盖此方法以实现当止损或条件订单触发时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_triggered(&mut self, event: OrderTriggered) {}

    /// 当订单修改处于待处理状态时调用。
    ///
    /// 覆盖此方法以实现当订单等待修改时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_pending_update(&mut self, event: OrderPendingUpdate) {}

    /// 当订单取消处于待处理状态时调用。
    ///
    /// 覆盖此方法以实现当订单等待取消时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_pending_cancel(&mut self, event: OrderPendingCancel) {}

    /// 当订单修改被驳回时调用。
    ///
    /// 覆盖此方法以实现当订单修改被驳回时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_modify_rejected(&mut self, event: OrderModifyRejected) {}

    /// 当订单取消被驳回时调用。
    ///
    /// 覆盖此方法以实现当订单取消被驳回时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_cancel_rejected(&mut self, event: OrderCancelRejected) {}

    /// 当订单更新时调用。
    ///
    /// 覆盖此方法以实现当订单修改时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_order_updated(&mut self, event: OrderUpdated) {}

    // Note: on_order_filled 继承自 DataActor trait

    /// 当持仓开启时调用。
    ///
    /// 覆盖此方法以实现当持仓开启时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_position_opened(&mut self, event: PositionOpened) {}

    /// 当持仓变更（数量或价格更新）时调用。
    ///
    /// 覆盖此方法以实现当持仓变更时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_position_changed(&mut self, event: PositionChanged) {}

    /// 当持仓关闭时调用。
    ///
    /// 覆盖此方法以实现当持仓关闭时的自定义逻辑。
    #[allow(unused_variables)]
    fn on_position_closed(&mut self, event: PositionClosed) {}

    // -- GTD EXPIRY MANAGEMENT -------------------------------------------------------------------

    /// 为订单设置 GTD 过期定时器。
    ///
    /// 创建一个在订单过期时自动取消订单的定时器。
    ///
    /// # Errors
    ///
    /// 如果定时器创建失败，则返回错误。
    fn set_gtd_expiry(&mut self, order: &OrderAny) -> anyhow::Result<()> {
        let core = self.core_mut();

        if !core.config.manage_gtd_expiry || order.time_in_force() != TimeInForce::Gtd {
            return Ok(());
        }

        let Some(expire_time) = order.expire_time() else {
            return Ok(());
        };

        let client_order_id = order.client_order_id();
        let timer_name = format!("GTD-EXPIRY:{client_order_id}");

        let current_time_ns = {
            let clock = core.clock();
            clock.timestamp_ns()
        };

        if current_time_ns >= expire_time.as_u64() {
            log::info!("GTD 订单 {client_order_id} 已过期，立即取消");
            return self.cancel_order(order.clone(), None);
        }

        {
            let mut clock = core.clock();
            clock.set_time_alert_ns(&timer_name, expire_time, None, None)?;
        }

        core.gtd_timers
            .insert(client_order_id, Ustr::from(&timer_name));

        log::debug!("为 {client_order_id} 设置 GTD 过期定时器，时间：{expire_time}");
        Ok(())
    }

    /// 取消订单的 GTD 过期定时器。
    fn cancel_gtd_expiry(&mut self, client_order_id: &ClientOrderId) {
        let core = self.core_mut();

        if let Some(timer_name) = core.gtd_timers.remove(client_order_id) {
            core.clock().cancel_timer(timer_name.as_str());
            log::debug!("取消了 {client_order_id} 的 GTD 过期定时器");
        }
    }

    /// 检查是否存在订单的 GTD 过期定时器。
    fn has_gtd_expiry_timer(&mut self, client_order_id: &ClientOrderId) -> bool {
        let core = self.core_mut();
        core.gtd_timers.contains_key(client_order_id)
    }

    /// 处理 GTD 订单过期，自动取消订单。
    ///
    /// 当 GTD 定时器触发时调用此方法。
    fn expire_gtd_order(&mut self, event: TimeEvent) {
        let timer_name = event.name.to_string();
        let Some(client_order_id_str) = timer_name.strip_prefix("GTD-EXPIRY:") else {
            log::error!("无效的 GTD 定时器名称格式: {timer_name}");
            return;
        };

        let client_order_id = ClientOrderId::from(client_order_id_str);

        let core = self.core_mut();
        core.gtd_timers.remove(&client_order_id);

        let cache = core.cache();
        let Some(order) = cache.order(&client_order_id) else {
            log::warn!("在缓存中未找到 GTD 订单 {client_order_id}");
            return;
        };

        let order = order.clone();
        drop(cache);

        log::info!("GTD 订单 {client_order_id} 已过期");

        if let Err(e) = self.cancel_order(order, None) {
            log::error!("未能取消已过期的 GTD 订单 {client_order_id}: {e}");
        }
    }

    /// 在策略启动时为活跃订单重新激活 GTD 定时器。
    ///
    /// 查询缓存中所有未过期的活跃 GTD 订单并为其创建定时器。
    /// 已经过期的订单将立即取消。
    fn reactivate_gtd_timers(&mut self) {
        let core = self.core_mut();
        let strategy_id = StrategyId::from(core.actor_id().inner().as_str());
        let current_time_ns = core.clock().timestamp_ns();
        let cache = core.cache();

        let open_orders = cache.orders_open(None, None, Some(&strategy_id), None, None);

        let gtd_orders: Vec<_> = open_orders
            .iter()
            .filter(|o| o.time_in_force() == TimeInForce::Gtd)
            .map(|o| (*o).clone())
            .collect();

        drop(cache);

        for order in gtd_orders {
            let Some(expire_time) = order.expire_time() else {
                continue;
            };

            let expire_time_ns = expire_time.as_u64();
            let client_order_id = order.client_order_id();

            if current_time_ns >= expire_time_ns {
                log::info!("GTD 订单 {client_order_id} 已过期，立即取消");
                if let Err(e) = self.cancel_order(order, None) {
                    log::error!("未能取消已过期的 GTD 订单 {client_order_id}: {e}");
                }
            } else if let Err(e) = self.set_gtd_expiry(&order) {
                log::error!("未能为 {client_order_id} 设置 GTD 过期定时器: {e}");
            }
        }
    }
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
    };
    use nautilus_model::{
        enums::{LiquiditySide, OrderSide, OrderType, PositionSide},
        events::OrderRejected,
        identifiers::{
            AccountId, ClientOrderId, InstrumentId, PositionId, StrategyId, TradeId, TraderId,
            VenueOrderId,
        },
        stubs::TestDefault,
        types::Currency,
    };
    use nautilus_portfolio::portfolio::Portfolio;
    use rstest::rstest;

    use super::*;

    #[derive(Debug)]
    struct TestStrategy {
        core: StrategyCore,
        on_order_rejected_called: bool,
        on_position_opened_called: bool,
    }

    impl TestStrategy {
        fn new(config: StrategyConfig) -> Self {
            Self {
                core: StrategyCore::new(config),
                on_order_rejected_called: false,
                on_position_opened_called: false,
            }
        }
    }

    impl Deref for TestStrategy {
        type Target = DataActorCore;
        fn deref(&self) -> &Self::Target {
            &self.core.actor
        }
    }

    impl DerefMut for TestStrategy {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.core.actor
        }
    }

    impl DataActor for TestStrategy {}

    impl Strategy for TestStrategy {
        fn core_mut(&mut self) -> &mut StrategyCore {
            &mut self.core
        }

        fn on_order_rejected(&mut self, _event: OrderRejected) {
            self.on_order_rejected_called = true;
        }

        fn on_position_opened(&mut self, _event: PositionOpened) {
            self.on_position_opened_called = true;
        }
    }

    fn create_test_strategy() -> TestStrategy {
        let config = StrategyConfig {
            strategy_id: Some(StrategyId::from("TEST-001")),
            order_id_tag: Some("001".to_string()),
            ..Default::default()
        };
        TestStrategy::new(config)
    }

    fn register_strategy(strategy: &mut TestStrategy) {
        let trader_id = TraderId::from("TRADER-001");
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));
        let portfolio = Rc::new(RefCell::new(Portfolio::new(
            cache.clone(),
            clock.clone(),
            None,
        )));

        strategy
            .core
            .register(trader_id, clock, cache, portfolio)
            .unwrap();
    }

    #[rstest]
    fn test_strategy_creation() {
        let strategy = create_test_strategy();
        assert_eq!(
            strategy.core.config.strategy_id,
            Some(StrategyId::from("TEST-001"))
        );
        assert!(!strategy.on_order_rejected_called);
        assert!(!strategy.on_position_opened_called);
    }

    #[rstest]
    fn test_strategy_registration() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        assert!(strategy.core.order_manager.is_some());
        assert!(strategy.core.order_factory.is_some());
        assert!(strategy.core.portfolio.is_some());
    }

    #[rstest]
    fn test_handle_order_event_dispatches_to_handler() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let event = OrderEventAny::Rejected(OrderRejected {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            client_order_id: ClientOrderId::from("O-001"),
            account_id: AccountId::from("ACC-001"),
            reason: "Test rejection".into(),
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
            reconciliation: 0,
            due_post_only: 0,
        });

        strategy.handle_order_event(event);

        assert!(strategy.on_order_rejected_called);
    }

    #[rstest]
    fn test_handle_position_event_dispatches_to_handler() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let event = PositionEvent::PositionOpened(PositionOpened {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            position_id: PositionId::test_default(),
            account_id: AccountId::from("ACC-001"),
            opening_order_id: ClientOrderId::from("O-001"),
            entry: OrderSide::Buy,
            side: PositionSide::Long,
            signed_qty: 1.0,
            quantity: Default::default(),
            last_qty: Default::default(),
            last_px: Default::default(),
            currency: Currency::from("USD"),
            avg_px_open: 0.0,
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
        });

        strategy.handle_position_event(event);

        assert!(strategy.on_position_opened_called);
    }

    #[rstest]
    fn test_strategy_default_handlers_do_not_panic() {
        let mut strategy = create_test_strategy();

        strategy.on_order_initialized(Default::default());
        strategy.on_order_denied(Default::default());
        strategy.on_order_emulated(Default::default());
        strategy.on_order_released(Default::default());
        strategy.on_order_submitted(Default::default());
        strategy.on_order_rejected(Default::default());
        let _ = DataActor::on_order_canceled(&mut strategy, &Default::default());
        strategy.on_order_expired(Default::default());
        strategy.on_order_triggered(Default::default());
        strategy.on_order_pending_update(Default::default());
        strategy.on_order_pending_cancel(Default::default());
        strategy.on_order_modify_rejected(Default::default());
        strategy.on_order_cancel_rejected(Default::default());
        strategy.on_order_updated(Default::default());
    }

    // -- GTD EXPIRY TESTS ----------------------------------------------------------------------------

    #[rstest]
    fn test_has_gtd_expiry_timer_when_timer_not_set() {
        let mut strategy = create_test_strategy();
        let client_order_id = ClientOrderId::from("O-001");

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_has_gtd_expiry_timer_when_timer_set() {
        let mut strategy = create_test_strategy();
        let client_order_id = ClientOrderId::from("O-001");

        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        assert!(strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_cancel_gtd_expiry_removes_timer() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");
        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        strategy.cancel_gtd_expiry(&client_order_id);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_cancel_gtd_expiry_when_timer_not_set() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");

        strategy.cancel_gtd_expiry(&client_order_id);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_handle_order_event_cancels_gtd_timer_on_filled() {
        use nautilus_model::events::OrderFilled;

        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");
        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        let event = OrderEventAny::Filled(OrderFilled {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            client_order_id,
            venue_order_id: VenueOrderId::test_default(),
            account_id: AccountId::from("ACC-001"),
            trade_id: TradeId::test_default(),
            position_id: None,
            order_side: OrderSide::Buy,
            order_type: OrderType::Market,
            last_qty: Default::default(),
            last_px: Default::default(),
            currency: Currency::from("USD"),
            liquidity_side: LiquiditySide::Taker,
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
            reconciliation: false,
            commission: None,
        });
        strategy.handle_order_event(event);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_handle_order_event_cancels_gtd_timer_on_canceled() {
        use nautilus_model::events::OrderCanceled;

        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");
        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        let event = OrderEventAny::Canceled(OrderCanceled {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            client_order_id,
            venue_order_id: Default::default(),
            account_id: Some(AccountId::from("ACC-001")),
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
            reconciliation: 0,
        });
        strategy.handle_order_event(event);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_handle_order_event_cancels_gtd_timer_on_rejected() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");
        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        let event = OrderEventAny::Rejected(OrderRejected {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            client_order_id,
            account_id: AccountId::from("ACC-001"),
            reason: "Test rejection".into(),
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
            reconciliation: 0,
            due_post_only: 0,
        });
        strategy.handle_order_event(event);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_handle_order_event_cancels_gtd_timer_on_expired() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let client_order_id = ClientOrderId::from("O-001");
        strategy
            .core
            .gtd_timers
            .insert(client_order_id, Ustr::from("GTD-EXPIRY:O-001"));

        let event = OrderEventAny::Expired(OrderExpired {
            trader_id: TraderId::from("TRADER-001"),
            strategy_id: StrategyId::from("TEST-001"),
            instrument_id: InstrumentId::from("BTCUSDT.BINANCE"),
            client_order_id,
            venue_order_id: Default::default(),
            account_id: Some(AccountId::from("ACC-001")),
            event_id: Default::default(),
            ts_event: Default::default(),
            ts_init: Default::default(),
            reconciliation: 0,
        });
        strategy.handle_order_event(event);

        assert!(!strategy.has_gtd_expiry_timer(&client_order_id));
    }

    #[rstest]
    fn test_on_start_calls_reactivate_gtd_timers_when_enabled() {
        let config = StrategyConfig {
            strategy_id: Some(StrategyId::from("TEST-001")),
            order_id_tag: Some("001".to_string()),
            manage_gtd_expiry: true,
            ..Default::default()
        };
        let mut strategy = TestStrategy::new(config);
        register_strategy(&mut strategy);

        let result = Strategy::on_start(&mut strategy);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_on_start_does_not_panic_when_gtd_disabled() {
        let config = StrategyConfig {
            strategy_id: Some(StrategyId::from("TEST-001")),
            order_id_tag: Some("001".to_string()),
            manage_gtd_expiry: false,
            ..Default::default()
        };
        let mut strategy = TestStrategy::new(config);
        register_strategy(&mut strategy);

        let result = Strategy::on_start(&mut strategy);
        assert!(result.is_ok());
    }

    // -- QUERY TESTS ---------------------------------------------------------------------------------

    #[rstest]
    fn test_query_account_when_registered() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let account_id = AccountId::from("ACC-001");

        let result = strategy.query_account(account_id, None);

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_query_account_with_client_id() {
        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let account_id = AccountId::from("ACC-001");
        let client_id = ClientId::from("BINANCE");

        let result = strategy.query_account(account_id, Some(client_id));

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_query_order_when_registered() {
        use nautilus_model::{orders::MarketOrder, stubs::TestDefault};

        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let order = OrderAny::Market(MarketOrder::test_default());

        let result = strategy.query_order(&order, None);

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_query_order_with_client_id() {
        use nautilus_model::{orders::MarketOrder, stubs::TestDefault};

        let mut strategy = create_test_strategy();
        register_strategy(&mut strategy);

        let order = OrderAny::Market(MarketOrder::test_default());
        let client_id = ClientId::from("BINANCE");

        let result = strategy.query_order(&order, Some(client_id));

        assert!(result.is_ok());
    }
}
