// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Risk management engine implementation.

pub mod config;

use std::{cell::RefCell, fmt::Debug, rc::Rc};

use ahash::AHashMap;
use config::RiskEngineConfig;
use nautilus_common::messages::execution::{CancelAllOrders, CancelOrder};
use nautilus_common::{
    cache::Cache,
    clock::Clock,
    logging::{CMD, EVT, RECV},
    messages::execution::{ModifyOrder, SubmitOrder, SubmitOrderList, TradingCommand},
    msgbus,
    msgbus::{MessagingSwitchboard, TypedHandler, TypedIntoHandler},
    throttler::Throttler,
};
use nautilus_core::{UUID4, WeakCell};
use nautilus_execution::trailing::{
    trailing_stop_calculate_with_bid_ask, trailing_stop_calculate_with_last,
};
use nautilus_model::{
    accounts::{Account, AccountAny},
    data::{Data, InstrumentStatus},
    enums::{
        InstrumentClass, OrderSide, OrderStatus, PositionSide, TimeInForce, TradingState,
        TrailingOffsetType, TriggerType,
    },
    events::{OrderCancelRejected, OrderDenied, OrderEventAny, OrderModifyRejected},
    identifiers::{AccountId, InstrumentId},
    instruments::{Instrument, InstrumentAny},
    orders::{Order, OrderAny},
    types::{Currency, Money, Price, Quantity, quantity::QuantityRaw},
};
use nautilus_portfolio::{Portfolio, t1_ledger::T1Ledger};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use std::sync::Arc;
use ustr::Ustr;

use crate::rule::{RuleContext, RuleChain};

type SubmitOrderFn = Box<dyn Fn(SubmitOrder)>;
type ModifyOrderFn = Box<dyn Fn(ModifyOrder)>;

/// Central risk management engine that validates and controls trading operations.
///
/// The `RiskEngine` provides comprehensive pre-trade risk checks including order validation,
/// balance verification, position sizing limits, and trading state management. It acts as
/// a gateway between strategy orders and execution, ensuring all trades comply with
/// defined risk parameters and regulatory constraints.
#[allow(dead_code)]
pub struct RiskEngine {
    clock: Rc<RefCell<dyn Clock>>,
    cache: Rc<RefCell<Cache>>,
    portfolio: Portfolio,
    pub throttled_submit_order: Throttler<SubmitOrder, SubmitOrderFn>,
    pub throttled_modify_order: Throttler<ModifyOrder, ModifyOrderFn>,
    max_notional_per_order: AHashMap<InstrumentId, Decimal>,
    trading_state: TradingState,
    config: RiskEngineConfig,

    // ---- 规则链 ----
    ashare_rule_chain: Option<RuleChain>,
    session_provider: Option<Arc<dyn nautilus_common::session::SessionProvider>>,
    pub t1_ledger: Option<Arc<std::sync::RwLock<T1Ledger>>>,
    pub global_trade_throttler: Option<Rc<RefCell<Throttler<TradingCommand, Box<dyn Fn(TradingCommand)>>>>>,

    // ---- 符号级和账户级限流器 ----
    symbol_throttlers: AHashMap<InstrumentId, Throttler<SubmitOrder, SubmitOrderFn>>,
    account_throttlers: AHashMap<AccountId, Throttler<SubmitOrder, SubmitOrderFn>>,
}

impl Debug for RiskEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(RiskEngine)).finish()
    }
}

impl RiskEngine {
    /// Creates a new [`RiskEngine`] instance.
    pub fn new(
        config: RiskEngineConfig,
        portfolio: Portfolio,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> Self {
        let global_trade_throttler = config.max_trade_command.as_ref().map(|rate_limit| {
            Rc::new(RefCell::new(Self::create_global_trade_throttler(
                rate_limit.clone(),
                clock.clone(),
                cache.clone(),
            )))
        });

        let throttled_submit_order = Self::create_submit_order_throttler(
            &config,
            clock.clone(),
            cache.clone(),
            global_trade_throttler.clone(),
        );

        let throttled_modify_order = Self::create_modify_order_throttler(
            &config,
            clock.clone(),
            cache.clone(),
            global_trade_throttler.clone(),
        );

        let (ashare_rule_chain, t1_ledger, session_provider) = if let Some(ashare_config) = &config.ashare_rules {
            let ledger = if ashare_config.t1_enabled {
                Some(ashare_config.t1_ledger.as_ref().map(|l| Arc::new(std::sync::RwLock::new((**l).clone()))).unwrap_or_else(|| Arc::new(std::sync::RwLock::new(T1Ledger::new()))))
            } else {
                None
            };
            let chain = crate::rule::ashare::create_ashare_rule_chain_with_ledger(ashare_config, ledger.clone(), None, None);
            let session = ashare_config.session_provider.clone();
            (Some(chain), ledger, session)
        } else {
            (None, None, None)
        };

        Self {
            clock,
            cache,
            portfolio,
            throttled_submit_order,
            throttled_modify_order,
            max_notional_per_order: AHashMap::new(),
            trading_state: TradingState::Active,
            config,
            ashare_rule_chain,
            session_provider,
            t1_ledger,
            global_trade_throttler,
            symbol_throttlers: AHashMap::new(),
            account_throttlers: AHashMap::new(),
        }
    }

    /// Registers all message bus handlers for the risk engine.
    pub fn register_msgbus_handlers(engine: Rc<RefCell<Self>>) {
        let weak = WeakCell::from(Rc::downgrade(&engine));

        let weak_cmd = weak.clone();
        msgbus::register_trading_command_endpoint(
            MessagingSwitchboard::risk_engine_execute(),
            TypedIntoHandler::from(move |cmd: TradingCommand| {
                if let Some(rc) = weak_cmd.upgrade() {
                    rc.borrow_mut().execute(cmd);
                }
            }),
        );

        let weak2 = weak.clone();
        msgbus::subscribe_data(
            "data.status.*".into(),
            TypedHandler::from(move |data: &Data| {
                if let Data::InstrumentStatus(status) = data {
                    if let Some(rc) = weak2.upgrade() {
                        rc.borrow_mut().handle_instrument_status(status);
                    }
                }
            }),
            None,
        );

        let weak_order = weak.clone();
        msgbus::subscribe_order_events(
            "events.order.*".into(),
            TypedHandler::from(move |event: &OrderEventAny| {
                if let Some(rc) = weak_order.upgrade() {
                    rc.borrow_mut().handle_event(event);
                }
            }),
            None,
        );
    }

    fn create_submit_order_throttler(
        config: &RiskEngineConfig,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
        global_throttler: Option<Rc<RefCell<Throttler<TradingCommand, Box<dyn Fn(TradingCommand)>>>>>,
    ) -> Throttler<SubmitOrder, SubmitOrderFn> {
        let success_handler = {
            let global_throttler = global_throttler;
            Box::new(move |submit_order: SubmitOrder| {
                if let Some(gt) = &global_throttler {
                    gt.borrow_mut().send(TradingCommand::SubmitOrder(submit_order));
                } else {
                    let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
                    msgbus::send_trading_command(endpoint, TradingCommand::SubmitOrder(submit_order));
                }
            }) as Box<dyn Fn(SubmitOrder)>
        };

        let failure_handler = {
            let cache = cache;
            let clock = clock.clone();
            Box::new(move |submit_order: SubmitOrder| {
                let reason = "REJECTED BY THROTTLER";
                log::warn!(
                    "SubmitOrder for {} DENIED: {}",
                    submit_order.client_order_id,
                    reason
                );

                Self::handle_submit_order_cache(&cache, &submit_order);

                let denied = Self::create_order_denied(&submit_order, reason, &clock);

                let endpoint = MessagingSwitchboard::exec_engine_process();
                msgbus::send_order_event(endpoint, denied);
            }) as Box<dyn Fn(SubmitOrder)>
        };

        Throttler::new(
            config.max_order_submit.limit,
            config.max_order_submit.interval_ns,
            clock,
            "ORDER_SUBMIT_THROTTLER".to_string(),
            success_handler,
            Some(failure_handler),
            Ustr::from(UUID4::new().as_str()),
        )
    }

    fn create_modify_order_throttler(
        config: &RiskEngineConfig,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
        global_throttler: Option<Rc<RefCell<Throttler<TradingCommand, Box<dyn Fn(TradingCommand)>>>>>,
    ) -> Throttler<ModifyOrder, ModifyOrderFn> {
        let success_handler = {
            let global_throttler = global_throttler;
            Box::new(move |order: ModifyOrder| {
                if let Some(gt) = &global_throttler {
                    gt.borrow_mut().send(TradingCommand::ModifyOrder(order));
                } else {
                    let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
                    msgbus::send_trading_command(endpoint, TradingCommand::ModifyOrder(order));
                }
            }) as Box<dyn Fn(ModifyOrder)>
        };

        let failure_handler = {
            let cache = cache;
            let clock = clock.clone();
            Box::new(move |order: ModifyOrder| {
                let reason = "Exceeded MAX_ORDER_MODIFY_RATE";
                log::warn!(
                    "SubmitOrder for {} DENIED: {}",
                    order.client_order_id,
                    reason
                );

                let order = match Self::get_existing_order(&cache, &order) {
                    Some(order) => order,
                    None => return,
                };

                let rejected = Self::create_modify_rejected(&order, reason, &clock);

                let endpoint = MessagingSwitchboard::exec_engine_process();
                msgbus::send_order_event(endpoint, rejected);
            }) as Box<dyn Fn(ModifyOrder)>
        };

        Throttler::new(
            config.max_order_modify.limit,
            config.max_order_modify.interval_ns,
            clock,
            "ORDER_MODIFY_THROTTLER".to_string(),
            success_handler,
            Some(failure_handler),
            Ustr::from(UUID4::new().as_str()),
        )
    }

    fn handle_submit_order_cache(cache: &Rc<RefCell<Cache>>, submit_order: &SubmitOrder) {
        let cache = cache.borrow();
        if !cache.order_exists(&submit_order.client_order_id) {
            log::error!(
                "Order not found in cache for client_order_id: {}",
                submit_order.client_order_id
            );
        }
    }

    fn get_existing_order(cache: &Rc<RefCell<Cache>>, order: &ModifyOrder) -> Option<OrderAny> {
        let cache = cache.borrow();
        if let Some(order) = cache.order(&order.client_order_id) {
            Some(order.clone())
        } else {
            log::error!(
                "Order with command.client_order_id: {} not found",
                order.client_order_id
            );
            None
        }
    }

    fn create_order_denied(
        submit_order: &SubmitOrder,
        reason: &str,
        clock: &Rc<RefCell<dyn Clock>>,
    ) -> OrderEventAny {
        let timestamp = clock.borrow().timestamp_ns();
        OrderEventAny::Denied(OrderDenied::new(
            submit_order.trader_id,
            submit_order.strategy_id,
            submit_order.instrument_id,
            submit_order.client_order_id,
            reason.into(),
            UUID4::new(),
            timestamp,
            timestamp,
        ))
    }

    fn create_modify_rejected(
        order: &OrderAny,
        reason: &str,
        clock: &Rc<RefCell<dyn Clock>>,
    ) -> OrderEventAny {
        let timestamp = clock.borrow().timestamp_ns();
        OrderEventAny::ModifyRejected(OrderModifyRejected::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            reason.into(),
            UUID4::new(),
            timestamp,
            timestamp,
            false,
            order.venue_order_id(),
            None,
        ))
    }

    fn create_global_trade_throttler(
        rate_limit: nautilus_common::throttler::RateLimit,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> Throttler<TradingCommand, Box<dyn Fn(TradingCommand)>> {
        let success_handler = {
            Box::new(move |command: TradingCommand| {
                let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
                msgbus::send_trading_command(endpoint, command);
            }) as Box<dyn Fn(TradingCommand)>
        };

        let failure_handler = {
            let cache = cache;
            let clock = clock.clone();
            Box::new(move |command: TradingCommand| {
                let reason = "EXCEEDED MAX GLOBAL TRADE RATE LIMIT";
                match &command {
                    TradingCommand::SubmitOrder(submit_order) => {
                        log::warn!(
                            "SubmitOrder for {} DENIED: {}",
                            submit_order.client_order_id,
                            reason
                        );
                        Self::handle_submit_order_cache(&cache, submit_order);
                        let denied = Self::create_order_denied(submit_order, reason, &clock);
                        let endpoint = MessagingSwitchboard::exec_engine_process();
                        msgbus::send_order_event(endpoint, denied);
                    }
                    TradingCommand::CancelOrder(cancel_order) => {
                        log::warn!(
                            "CancelOrder for {} DENIED: {}",
                            cancel_order.client_order_id,
                            reason
                        );
                        let (venue_order_id, account_id) = {
                            let cache_borrow = cache.borrow();
                            if let Some(order) = cache_borrow.order(&cancel_order.client_order_id) {
                                (order.venue_order_id(), order.account_id())
                            } else {
                                (cancel_order.venue_order_id.clone(), None)
                            }
                        };
                        let timestamp = clock.borrow().timestamp_ns();
                        let rejected = OrderEventAny::CancelRejected(OrderCancelRejected::new(
                            cancel_order.trader_id,
                            cancel_order.strategy_id,
                            cancel_order.instrument_id,
                            cancel_order.client_order_id.clone(),
                            Ustr::from(reason),
                            UUID4::new(),
                            timestamp,
                            timestamp,
                            false,
                            venue_order_id,
                            account_id,
                        ));
                        let endpoint = MessagingSwitchboard::exec_engine_process();
                        msgbus::send_order_event(endpoint, rejected);
                    }
                    TradingCommand::ModifyOrder(modify_order) => {
                        log::warn!(
                            "ModifyOrder for {} DENIED: {}",
                            modify_order.client_order_id,
                            reason
                        );
                        let order = match Self::get_existing_order(&cache, modify_order) {
                            Some(order) => order,
                            None => return,
                        };
                        let rejected = Self::create_modify_rejected(&order, reason, &clock);
                        let endpoint = MessagingSwitchboard::exec_engine_process();
                        msgbus::send_order_event(endpoint, rejected);
                    }
                    _ => {
                        log::warn!("Command dropped by global throttler: {:?}", command);
                    }
                }
            }) as Box<dyn Fn(TradingCommand)>
        };

        Throttler::new(
            rate_limit.limit,
            rate_limit.interval_ns,
            clock,
            "GLOBAL_TRADE_THROTTLER".to_string(),
            success_handler,
            Some(failure_handler),
            Ustr::from(UUID4::new().as_str()),
        )
    }

    /// Executes a trading command through the risk management pipeline.
    pub fn execute(&mut self, command: TradingCommand) {
        // This will extend to other commands such as `RiskCommand`
        self.handle_command(command);
    }

    /// Processes an order event for risk monitoring and state updates.
    pub fn process(&mut self, event: &OrderEventAny) {
        // This will extend to other events such as `RiskEvent`
        self.handle_event(event);
    }

    /// Sets the trading state for risk control enforcement.
    pub fn set_trading_state(&mut self, state: TradingState) {
        if state == self.trading_state {
            log::warn!("No change to trading state: already set to {state:?}");
            return;
        }

        self.trading_state = state;

        let _ts_now = self.clock.borrow().timestamp_ns();

        // TODO: Create a new Event "TradingStateChanged" in OrderEventAny enum.
        // let event = OrderEventAny::TradingStateChanged(TradingStateChanged::new(..,self.trading_state,..));

        msgbus::publish_any("events.risk".into(), &"message"); // TODO: Send the new Event here

        log::info!("Trading state set to {state:?}");
    }

    /// Sets the maximum notional value per order for the specified instrument.
    pub fn set_max_notional_per_order(&mut self, instrument_id: InstrumentId, new_value: Decimal) {
        self.max_notional_per_order.insert(instrument_id, new_value);

        let new_value_str = new_value.to_string();
        log::info!("Set MAX_NOTIONAL_PER_ORDER: {instrument_id} {new_value_str}");
    }

    /// 加载 T+1 账本初始持仓，供早盘启动或盘中恢复时调用。
    pub fn t1_load_position(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        total_qty: f64,
        today_buy_qty: f64,
    ) {
        if let Some(ledger) = &self.t1_ledger {
            ledger.write().expect("T1 ledger lock poisoned").load_position(account_id, instrument_id, total_qty, today_buy_qty);
            log::info!(
                "T1_LEDGER Loaded: account={}, symbol={}, total={}, today_buy={}",
                account_id,
                instrument_id,
                total_qty,
                today_buy_qty
            );
        } else {
            log::warn!("t1_load_position called but T1 ledger is not enabled.");
        }
    }

    /// 执行 T+1 账本日切结算，将今日买入仓位解冻为可卖仓位（通常在盘后发信号调用）。
    pub fn t1_on_settlement(&mut self) {
        if let Some(ledger) = &self.t1_ledger {
            ledger.write().expect("T1 ledger lock poisoned").on_settlement();
            log::info!("T1_LEDGER settled for all accounts to unlock sellable balance.");
        } else {
            log::warn!("t1_on_settlement called but T1 ledger is not enabled.");
        }
    }

    /// Starts the risk engine.
    pub fn start(&mut self) {
        log::info!("Started");
    }

    /// Stops the risk engine.
    pub fn stop(&mut self) {
        log::info!("Stopped");
    }

    /// Resets the risk engine to its initial state.
    pub fn reset(&mut self) {
        self.throttled_submit_order.reset();
        self.throttled_modify_order.reset();
        if let Some(t) = self.global_trade_throttler.as_ref() {
            t.borrow_mut().reset();
        }
        self.max_notional_per_order.clear();
        self.trading_state = TradingState::Active;

        log::info!("Reset");
    }

    /// Disposes of the risk engine, releasing resources.
    pub fn dispose(&mut self) {
        log::info!("Disposed");
    }

    /// Returns a reference to the clock.
    #[must_use]
    pub fn clock(&self) -> &Rc<RefCell<dyn Clock>> {
        &self.clock
    }

    /// Returns a reference to the cache.
    #[must_use]
    pub fn cache(&self) -> &Rc<RefCell<Cache>> {
        &self.cache
    }

    /// Returns a reference to the configuration.
    #[must_use]
    pub const fn config(&self) -> &RiskEngineConfig {
        &self.config
    }

    /// Returns the current trading state.
    #[must_use]
    pub const fn trading_state(&self) -> TradingState {
        self.trading_state
    }

    /// Returns a reference to the max notional per order settings.
    #[must_use]
    pub const fn max_notional_per_order(&self) -> &AHashMap<InstrumentId, Decimal> {
        &self.max_notional_per_order
    }

    fn handle_command(&mut self, command: TradingCommand) {
        if self.config.debug {
            log::debug!("{CMD}{RECV} {command:?}");
        }

        match command {
            TradingCommand::SubmitOrder(submit_order) => self.handle_submit_order(submit_order),
            TradingCommand::SubmitOrderList(submit_order_list) => {
                self.handle_submit_order_list(submit_order_list);
            }
            TradingCommand::ModifyOrder(modify_order) => self.handle_modify_order(modify_order),
            TradingCommand::CancelOrder(cancel_order) => self.handle_cancel_order(cancel_order),
            TradingCommand::CancelAllOrders(cancel_all) => {
                self.handle_cancel_all_orders(cancel_all)
            }
            TradingCommand::BatchCancelOrders(batch) => {
                // A 股：BatchCancelOrders 直接转发（批量撤单阶段检查由各 CancelOrder 分别处理）
                self.send_to_execution(TradingCommand::BatchCancelOrders(batch));
            }
            TradingCommand::QueryAccount(query_account) => {
                self.send_to_execution(TradingCommand::QueryAccount(query_account));
            }
            _ => {
                log::error!("Cannot handle command: {command}");
            }
        }
    }

    /// 撤单命令处理（含交易时段检查）。
    fn handle_cancel_order(&mut self, command: CancelOrder) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::CancelOrder(command));
            return;
        }

        // ---- 交易时段检查 ----
        if let Some(ref session) = self.session_provider {
            let phase = session.phase_at(
                &command.instrument_id.venue,
                self.clock.borrow().timestamp_ns(),
            );
            if !phase.can_cancel_order() {
                let reason = format!("CANCEL_DENIED: phase={phase:?} does not allow cancellation");
                log::warn!(
                    "CancelOrder for {} DENIED: {reason}",
                    command.client_order_id
                );
                // 从缓存中查找已有订单获取 venue_order_id / account_id
                let (venue_order_id, account_id) = {
                    let cache = self.cache.borrow();
                    if let Some(order) = cache.order(&command.client_order_id) {
                        (order.venue_order_id(), order.account_id())
                    } else {
                        (command.venue_order_id, None)
                    }
                };
                let event = self.create_cancel_rejected(
                    command.trader_id,
                    command.strategy_id,
                    command.instrument_id,
                    command.client_order_id,
                    venue_order_id,
                    account_id,
                    &reason,
                );
                let endpoint = MessagingSwitchboard::exec_engine_process();
                msgbus::send_order_event(endpoint, event);
                return;
            }
        }
        self.send_to_execution(TradingCommand::CancelOrder(command));
    }

    /// 全部撤单命令处理（含交易时段检查）。
    /// `CancelAllOrders` 无对应的单笔 `OrderCancelRejected`，仅打日志拦截。
    fn handle_cancel_all_orders(&mut self, command: CancelAllOrders) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::CancelAllOrders(command));
            return;
        }

        // ---- 交易时段检查 ----
        if let Some(ref session) = self.session_provider {
            let phase = session.phase_at(
                &command.instrument_id.venue,
                self.clock.borrow().timestamp_ns(),
            );
            if !phase.can_cancel_order() {
                log::warn!(
                    "CancelAllOrders for {} DENIED: phase={phase:?} does not allow cancellation",
                    command.instrument_id
                );
                return;
            }
        }

        self.send_to_execution(TradingCommand::CancelAllOrders(command));
    }

    /// 构建 `OrderCancelRejected` 事件（A 股撤单被阶段规则拒绝时使用）。
    #[allow(clippy::too_many_arguments)]
    fn create_cancel_rejected(
        &self,
        trader_id: nautilus_model::identifiers::TraderId,
        strategy_id: nautilus_model::identifiers::StrategyId,
        instrument_id: InstrumentId,
        client_order_id: nautilus_model::identifiers::ClientOrderId,
        venue_order_id: Option<nautilus_model::identifiers::VenueOrderId>,
        account_id: Option<AccountId>,
        reason: &str,
    ) -> OrderEventAny {
        let ts_now = self.clock.borrow().timestamp_ns();
        OrderEventAny::CancelRejected(OrderCancelRejected::new(
            trader_id,
            strategy_id,
            instrument_id,
            client_order_id,
            Ustr::from(reason),
            UUID4::new(),
            ts_now,
            ts_now,
            false,
            venue_order_id,
            account_id,
        ))
    }

    fn handle_submit_order(&mut self, command: SubmitOrder) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::SubmitOrder(command));
            return;
        }

        let order = {
            let cache = self.cache.borrow();
            match cache.order(&command.client_order_id) {
                Some(order) => order.clone(),
                None => {
                    log::error!(
                        "Cannot handle submit order: order not found in cache for {}",
                        command.client_order_id
                    );
                    return;
                }
            }
        };

        if let Some(position_id) = command.position_id
            && order.is_reduce_only()
        {
            let position_exists = {
                let cache = self.cache.borrow();
                cache
                    .position(&position_id)
                    .map(|pos| (pos.side, pos.quantity))
            };

            if let Some((pos_side, pos_quantity)) = position_exists {
                if !order.would_reduce_only(pos_side, pos_quantity) {
                    self.deny_command(
                        TradingCommand::SubmitOrder(command),
                        &format!("Reduce only order would increase position {position_id}"),
                    );
                    return; // Denied
                }
            } else {
                self.deny_command(
                    TradingCommand::SubmitOrder(command),
                    &format!("Position {position_id} not found for reduce-only order"),
                );
                return;
            }
        }

        let instrument_exists = {
            let cache = self.cache.borrow();
            cache.instrument(&command.instrument_id).cloned()
        };

        let instrument = if let Some(instrument) = instrument_exists {
            instrument
        } else {
            self.deny_command(
                TradingCommand::SubmitOrder(command.clone()),
                &format!("Instrument for {} not found", command.instrument_id),
            );
            return; // Denied
        };

        // ---- 使用规则链进行 A 股相关检查 ----
        if let Some(ref rule_chain) = self.ashare_rule_chain {
            let account_id = order.account_id().or_else(|| {
                self.cache
                    .borrow()
                    .account_for_venue(&order.instrument_id().venue)
                    .map(|a| a.id())
            });

            let context = RuleContext::new(
                order.clone(),
                command.instrument_id,
                account_id,
                self.clock.borrow().timestamp_ns().as_u64(),
            );

            let rule_result = rule_chain.check(&context);
            if !rule_result.is_pass() {
                if let crate::rule::RuleCheckResult::Fail { reason } = rule_result {
                    self.deny_command(
                        TradingCommand::SubmitOrder(command),
                        &reason,
                    );
                    return;
                }
            }
        }

        if !self.check_order(instrument.clone(), order.clone()) {
            return; // Denied
        }

        // ---- 停牌/临停检查 ----
        {
            let cache = self.cache.borrow();
            if let Some(status) = cache.instrument_status(&command.instrument_id) {
                use nautilus_model::enums::MarketStatusAction;
                if matches!(
                    status.action,
                    MarketStatusAction::Halt
                        | MarketStatusAction::Suspend
                        | MarketStatusAction::NotAvailableForTrading
                ) {
                    self.deny_command(
                        TradingCommand::SubmitOrder(command),
                        &format!("INSTRUMENT_SUSPENDED: status={:?}", status.action),
                    );
                    return;
                }
            }
        }

        if !self.check_orders_risk(instrument.clone(), &[order]) {
            return; // Denied
        }

        // Route through execution gateway for TradingState checks & throttling
        self.execution_gateway(instrument, TradingCommand::SubmitOrder(command));
    }

    fn handle_submit_order_list(&mut self, command: SubmitOrderList) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::SubmitOrderList(command));
            return;
        }

        let instrument_exists = {
            let cache = self.cache.borrow();
            cache.instrument(&command.instrument_id).cloned()
        };

        let instrument = if let Some(instrument) = instrument_exists {
            instrument
        } else {
            self.deny_command(
                TradingCommand::SubmitOrderList(command.clone()),
                &format!("no instrument found for {}", command.instrument_id),
            );
            return; // Denied
        };

        let orders: Vec<OrderAny> = self
            .cache
            .borrow()
            .orders_for_ids(&command.order_list.client_order_ids, &command);

        if orders.len() != command.order_list.client_order_ids.len() {
            self.deny_order_list(
                &orders,
                &format!("Incomplete order list: missing orders in cache for {command}"),
            );
            return; // Denied
        }

        for order in orders.clone() {
            if !self.check_order(instrument.clone(), order) {
                return; // Denied
            }
        }

        // ---- 新增：停牌/临停检查 ----
        {
            let cache = self.cache.borrow();
            if let Some(status) = cache.instrument_status(&command.instrument_id) {
                use nautilus_model::enums::MarketStatusAction;
                if matches!(
                    status.action,
                    MarketStatusAction::Halt
                        | MarketStatusAction::Suspend
                        | MarketStatusAction::NotAvailableForTrading
                ) {
                    self.deny_order_list(
                        &orders,
                        &format!("INSTRUMENT_SUSPENDED: status={:?}", status.action),
                    );
                    return;
                }
            }
        }

        if !self.check_orders_risk(instrument.clone(), &orders) {
            self.deny_order_list(
                &orders,
                &format!("OrderList {} DENIED", command.order_list.id),
            );
            return; // Denied
        }

        self.execution_gateway(instrument, TradingCommand::SubmitOrderList(command));
    }

    fn handle_modify_order(&mut self, command: ModifyOrder) {
        let order_exists = {
            let cache = self.cache.borrow();
            cache.order(&command.client_order_id).cloned()
        };

        let order = if let Some(order) = order_exists {
            order
        } else {
            log::error!(
                "ModifyOrder DENIED: Order with command.client_order_id: {} not found",
                command.client_order_id
            );
            return;
        };

        if order.is_closed() {
            self.reject_modify_order(
                order,
                &format!(
                    "Order with command.client_order_id: {} already closed",
                    command.client_order_id
                ),
            );
            return;
        } else if order.status() == OrderStatus::PendingCancel {
            self.reject_modify_order(
                order,
                &format!(
                    "Order with command.client_order_id: {} is already pending cancel",
                    command.client_order_id
                ),
            );
            return;
        }

        let maybe_instrument = {
            let cache = self.cache.borrow();
            cache.instrument(&command.instrument_id).cloned()
        };

        let instrument = if let Some(instrument) = maybe_instrument {
            instrument
        } else {
            self.reject_modify_order(
                order,
                &format!("no instrument found for {:?}", command.instrument_id),
            );
            return; // Denied
        };

        // Check Price
        let mut risk_msg = self.check_price(&instrument, command.price);
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // Check Trigger
        risk_msg = self.check_price(&instrument, command.trigger_price);
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // Check Quantity
        risk_msg = self.check_quantity(&instrument, command.quantity, order.is_quote_quantity());
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // Check InstrumentStatus
        {
            let cache = self.cache.borrow();
            if let Some(status) = cache.instrument_status(&command.instrument_id) {
                use nautilus_model::enums::MarketStatusAction;
                if matches!(
                    status.action,
                    MarketStatusAction::Halt
                        | MarketStatusAction::Suspend
                        | MarketStatusAction::NotAvailableForTrading
                ) {
                    self.reject_modify_order(
                        order,
                        &format!("INSTRUMENT_SUSPENDED: status={:?}", status.action),
                    );
                    return;
                }
            }
        }

        // Check TradingState
        match self.trading_state {
            TradingState::Halted => {
                self.reject_modify_order(order, "TradingState is HALTED: Cannot modify order");
            }
            TradingState::Reducing => {
                if let Some(quantity) = command.quantity
                    && quantity > order.quantity()
                    && ((order.is_buy() && self.portfolio.is_net_long(&instrument.id()))
                        || (order.is_sell() && self.portfolio.is_net_short(&instrument.id())))
                {
                    self.reject_modify_order(
                        order,
                        &format!(
                            "TradingState is REDUCING and update will increase exposure {}",
                            instrument.id()
                        ),
                    );
                }
            }
            _ => {}
        }

        self.throttled_modify_order.send(command);
    }

    fn check_order(&self, instrument: InstrumentAny, order: OrderAny) -> bool {
        if order.time_in_force() == TimeInForce::Gtd {
            // SAFETY: GTD guarantees an expire time
            let expire_time = order.expire_time().unwrap();
            if expire_time <= self.clock.borrow().timestamp_ns() {
                self.deny_order(
                    order,
                    &format!("GTD {} already past", expire_time.to_rfc3339()),
                );
                return false; // Denied
            }
        }

        if !self.check_order_price(instrument.clone(), order.clone())
            || !self.check_order_quantity(instrument, order)
        {
            return false; // Denied
        }

        true
    }

    fn check_order_price(&self, instrument: InstrumentAny, order: OrderAny) -> bool {
        if let Some(order_price) = order.price() {
            let risk_msg = self.check_price(&instrument, Some(order_price));
            if let Some(risk_msg) = risk_msg {
                self.deny_order(order.clone(), &risk_msg);
                return false; // Denied
            }
        }

        if order.trigger_price().is_some() {
            let risk_msg = self.check_price(&instrument, order.trigger_price());
            if let Some(risk_msg) = risk_msg {
                self.deny_order(order, &risk_msg);
                return false; // Denied
            }
        }

        true
    }

    fn check_order_quantity(&self, instrument: InstrumentAny, order: OrderAny) -> bool {
        let risk_msg = self.check_quantity(
            &instrument,
            Some(order.quantity()),
            order.is_quote_quantity(),
        );
        if let Some(risk_msg) = risk_msg {
            self.deny_order(order.clone(), &risk_msg);
            return false; // Denied
        }

        true
    }

    fn check_orders_risk(&self, instrument: InstrumentAny, orders: &[OrderAny]) -> bool {
        let mut last_px: Option<Price> = None;
        let mut max_notional: Option<Money> = None;

        // Determine max notional
        let max_notional_setting = self.max_notional_per_order.get(&instrument.id());
        if let Some(max_notional_setting_val) = max_notional_setting.copied() {
            max_notional = Some(Money::new(
                max_notional_setting_val
                    .to_f64()
                    .expect("Invalid decimal conversion"),
                instrument.quote_currency(),
            ));
        }

        // Get account for risk checks
        let account_exists = {
            let cache = self.cache.borrow();
            cache.account_for_venue(&instrument.id().venue).cloned()
        };

        let account = if let Some(account) = account_exists {
            account
        } else {
            log::debug!("Cannot find account for venue {}", instrument.id().venue);
            return true; // TODO: Temporary early return until handling routing/multiple venues
        };
        let cash_account = match account {
            AccountAny::Cash(cash_account) => cash_account,
            AccountAny::Margin(_) => return true, // TODO: Determine risk controls for margin
        };
        let free = cash_account.balance_free(Some(instrument.quote_currency()));
        let allow_borrowing = cash_account.allow_borrowing;
        if self.config.debug {
            log::debug!("Free cash: {free:?}");
        }

        // Get net LONG position quantity for this instrument (for position-reducing sell checks),
        // accounting for already submitted (but unfilled) SELL orders to prevent overselling.
        let (net_long_qty_raw, pending_sell_qty_raw) = {
            let cache = self.cache.borrow();
            let long_qty: QuantityRaw = cache
                .positions_open(
                    None,
                    Some(&instrument.id()),
                    None,
                    None,
                    Some(PositionSide::Long),
                )
                .iter()
                .map(|pos| pos.quantity.raw)
                .sum();
            let pending_sells: QuantityRaw = cache
                .orders_open(
                    None,
                    Some(&instrument.id()),
                    None,
                    None,
                    Some(OrderSide::Sell),
                )
                .iter()
                .map(|ord| ord.leaves_qty().raw)
                .sum();
            (long_qty, pending_sells)
        };

        // Available quantity is long position minus pending sells
        let available_long_qty_raw = net_long_qty_raw.saturating_sub(pending_sell_qty_raw);

        if self.config.debug && net_long_qty_raw > 0 {
            log::debug!(
                "Net LONG qty (raw): {net_long_qty_raw}, pending sells: {pending_sell_qty_raw}, available: {available_long_qty_raw}"
            );
        }

        // Track cumulative sell quantity to determine position-reducing vs position-opening sells
        let mut cum_sell_qty_raw: QuantityRaw = 0;

        let mut cum_notional_buy: Option<Money> = None;
        let mut cum_notional_sell: Option<Money> = None;
        let mut base_currency: Option<Currency> = None;
        for order in orders {
            // Determine last price based on order type
            last_px = match order {
                OrderAny::Market(_) | OrderAny::MarketToLimit(_) => {
                    if last_px.is_none() {
                        let cache = self.cache.borrow();
                        if let Some(last_quote) = cache.quote(&instrument.id()) {
                            match order.order_side() {
                                OrderSide::Buy => Some(last_quote.ask_price),
                                OrderSide::Sell => Some(last_quote.bid_price),
                                _ => panic!("Invalid order side"),
                            }
                        } else {
                            let cache = self.cache.borrow();
                            let last_trade = cache.trade(&instrument.id());

                            if let Some(last_trade) = last_trade {
                                Some(last_trade.price)
                            } else {
                                log::warn!(
                                    "Cannot check MARKET order risk: no prices for {}",
                                    instrument.id()
                                );
                                continue;
                            }
                        }
                    } else {
                        last_px
                    }
                }
                OrderAny::StopMarket(_) | OrderAny::MarketIfTouched(_) => order.trigger_price(),
                OrderAny::TrailingStopMarket(_) | OrderAny::TrailingStopLimit(_) => {
                    if let Some(trigger_price) = order.trigger_price() {
                        Some(trigger_price)
                    } else {
                        // Validate trailing offset type is supported
                        let offset_type = order.trailing_offset_type().unwrap();
                        if !matches!(
                            offset_type,
                            TrailingOffsetType::Price
                                | TrailingOffsetType::BasisPoints
                                | TrailingOffsetType::Ticks
                        ) {
                            self.deny_order(
                                order.clone(),
                                &format!("UNSUPPORTED_TRAILING_OFFSET_TYPE: {offset_type:?}"),
                            );
                            return false;
                        }

                        let trigger_type = order.trigger_type().unwrap();
                        let cache = self.cache.borrow();

                        if trigger_type == TriggerType::BidAsk {
                            if let Some(quote) = cache.quote(&instrument.id()) {
                                match trailing_stop_calculate_with_bid_ask(
                                    instrument.price_increment(),
                                    order.trailing_offset_type().unwrap(),
                                    order.order_side_specified(),
                                    order.trailing_offset().unwrap(),
                                    quote.bid_price,
                                    quote.ask_price,
                                ) {
                                    Ok(calculated_trigger) => Some(calculated_trigger),
                                    Err(e) => {
                                        log::warn!(
                                            "Cannot check {} order risk: failed to calculate trigger price from trailing offset: {e}",
                                            order.order_type()
                                        );
                                        continue;
                                    }
                                }
                            } else {
                                log::warn!(
                                    "Cannot check {} order risk: no trigger price set and no bid/ask quotes available for {}",
                                    order.order_type(),
                                    instrument.id()
                                );
                                continue;
                            }
                        } else if let Some(last_trade) = cache.trade(&instrument.id()) {
                            match trailing_stop_calculate_with_last(
                                instrument.price_increment(),
                                order.trailing_offset_type().unwrap(),
                                order.order_side_specified(),
                                order.trailing_offset().unwrap(),
                                last_trade.price,
                            ) {
                                Ok(calculated_trigger) => Some(calculated_trigger),
                                Err(e) => {
                                    log::warn!(
                                        "Cannot check {} order risk: failed to calculate trigger price from trailing offset: {}",
                                        order.order_type(),
                                        e
                                    );
                                    continue;
                                }
                            }
                        } else if trigger_type == TriggerType::LastOrBidAsk {
                            // Fallback to bid/ask when no trade data available
                            if let Some(quote) = cache.quote(&instrument.id()) {
                                match trailing_stop_calculate_with_bid_ask(
                                    instrument.price_increment(),
                                    order.trailing_offset_type().unwrap(),
                                    order.order_side_specified(),
                                    order.trailing_offset().unwrap(),
                                    quote.bid_price,
                                    quote.ask_price,
                                ) {
                                    Ok(calculated_trigger) => Some(calculated_trigger),
                                    Err(e) => {
                                        log::warn!(
                                            "Cannot check {} order risk: failed to calculate trigger price from trailing offset: {e}",
                                            order.order_type()
                                        );
                                        continue;
                                    }
                                }
                            } else {
                                log::warn!(
                                    "Cannot check {} order risk: no trigger price set and no market data available for {}",
                                    order.order_type(),
                                    instrument.id()
                                );
                                continue;
                            }
                        } else {
                            log::warn!(
                                "Cannot check {} order risk: no trigger price set and no market data available for {}",
                                order.order_type(),
                                instrument.id()
                            );
                            continue;
                        }
                    }
                }
                _ => order.price(),
            };

            let last_px = if let Some(px) = last_px {
                px
            } else {
                log::error!("Cannot check order risk: no price available");
                continue;
            };

            // For quote quantity limit orders, use worst-case execution price
            let effective_price = if order.is_quote_quantity()
                && !instrument.is_inverse()
                && matches!(order, OrderAny::Limit(_) | OrderAny::StopLimit(_))
            {
                // Get current market price for worst-case execution
                let cache = self.cache.borrow();
                if let Some(quote_tick) = cache.quote(&instrument.id()) {
                    match order.order_side() {
                        // BUY: could execute at best ask if below limit (more quantity)
                        OrderSide::Buy => last_px.min(quote_tick.ask_price),
                        // SELL: could execute at best bid if above limit (but less quantity, so use limit)
                        OrderSide::Sell => last_px.max(quote_tick.bid_price),
                        _ => last_px,
                    }
                } else {
                    last_px // No market data, use limit price
                }
            } else {
                last_px
            };

            let effective_quantity = if order.is_quote_quantity() && !instrument.is_inverse() {
                instrument.calculate_base_quantity(order.quantity(), effective_price)
            } else {
                order.quantity()
            };

            // Check min/max quantity against effective quantity
            if let Some(max_quantity) = instrument.max_quantity()
                && effective_quantity > max_quantity
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "QUANTITY_EXCEEDS_MAXIMUM: effective_quantity={effective_quantity}, max_quantity={max_quantity}"
                    ),
                );
                return false; // Denied
            }

            if let Some(min_quantity) = instrument.min_quantity()
                && effective_quantity < min_quantity
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "QUANTITY_BELOW_MINIMUM: effective_quantity={effective_quantity}, min_quantity={min_quantity}"
                    ),
                );
                return false; // Denied
            }

            let notional =
                instrument.calculate_notional_value(effective_quantity, last_px, Some(true));

            if self.config.debug {
                log::debug!("Notional: {notional:?}");
            }

            // Check MAX notional per order limit
            if let Some(max_notional_value) = max_notional
                && notional > max_notional_value
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "NOTIONAL_EXCEEDS_MAX_PER_ORDER: max_notional={max_notional_value:?}, notional={notional:?}"
                        ),
                    );
                return false; // Denied
            }

            // Check MIN notional instrument limit
            if let Some(min_notional) = instrument.min_notional()
                && notional.currency == min_notional.currency
                && notional < min_notional
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "NOTIONAL_LESS_THAN_MIN_FOR_INSTRUMENT: min_notional={min_notional:?}, notional={notional:?}"
                        ),
                    );
                return false; // Denied
            }

            // // Check MAX notional instrument limit
            if let Some(max_notional) = instrument.max_notional()
                && notional.currency == max_notional.currency
                && notional > max_notional
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "NOTIONAL_GREATER_THAN_MAX_FOR_INSTRUMENT: max_notional={max_notional:?}, notional={notional:?}"
                        ),
                    );
                return false; // Denied
            }

            // Calculate OrderBalanceImpact (valid for CashAccount only)
            let notional = instrument.calculate_notional_value(effective_quantity, last_px, None);
            let order_balance_impact = match order.order_side() {
                OrderSide::Buy => Money::from_raw(-notional.raw, notional.currency),
                OrderSide::Sell => Money::from_raw(notional.raw, notional.currency),
                OrderSide::NoOrderSide => {
                    panic!("invalid `OrderSide`, was {}", order.order_side());
                }
            };

            if self.config.debug {
                log::debug!("Balance impact: {order_balance_impact}");
            }

            // Skip balance check when borrowing is enabled (e.g. spot margin trading)
            if !allow_borrowing
                && let Some(free_val) = free
                && (free_val.as_decimal() + order_balance_impact.as_decimal()) < Decimal::ZERO
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "NOTIONAL_EXCEEDS_FREE_BALANCE: free={free_val:?}, notional={notional:?}"
                    ),
                );
                return false;
            }

            if base_currency.is_none() {
                base_currency = instrument.base_currency();
            }
            if order.is_buy() {
                match cum_notional_buy.as_mut() {
                    Some(cum_notional_buy_val) => {
                        cum_notional_buy_val.raw += -order_balance_impact.raw;
                    }
                    None => {
                        cum_notional_buy = Some(Money::from_raw(
                            -order_balance_impact.raw,
                            order_balance_impact.currency,
                        ));
                    }
                }

                if self.config.debug {
                    log::debug!("Cumulative notional BUY: {cum_notional_buy:?}");
                }

                if !allow_borrowing
                    && let (Some(free), Some(cum_notional_buy)) = (free, cum_notional_buy)
                    && cum_notional_buy > free
                {
                    self.deny_order(order.clone(), &format!("CUM_NOTIONAL_EXCEEDS_FREE_BALANCE: free={free}, cum_notional={cum_notional_buy}"));
                    return false; // Denied
                }
            } else if order.is_sell() {
                let is_position_reducing_sell = order.is_reduce_only()
                    || (cum_sell_qty_raw + effective_quantity.raw) <= available_long_qty_raw;
                cum_sell_qty_raw += effective_quantity.raw;

                if is_position_reducing_sell {
                    if self.config.debug {
                        log::debug!("Position-reducing SELL skips balance check");
                    }
                    continue;
                }

                if cash_account.base_currency.is_some() {
                    match cum_notional_sell.as_mut() {
                        Some(cum_notional_buy_val) => {
                            cum_notional_buy_val.raw += order_balance_impact.raw;
                        }
                        None => {
                            cum_notional_sell = Some(Money::from_raw(
                                order_balance_impact.raw,
                                order_balance_impact.currency,
                            ));
                        }
                    }
                    if self.config.debug {
                        log::debug!("Cumulative notional SELL: {cum_notional_sell:?}");
                    }

                    if !allow_borrowing
                        && let (Some(free), Some(cum_notional_sell)) = (free, cum_notional_sell)
                        && cum_notional_sell > free
                    {
                        self.deny_order(order.clone(), &format!("CUM_NOTIONAL_EXCEEDS_FREE_BALANCE: free={free}, cum_notional={cum_notional_sell}"));
                        return false; // Denied
                    }
                }
                // Account is already of type Cash, so no check
                else if let Some(base_currency) = base_currency {
                    let cash_value = Money::from_raw(
                        effective_quantity
                            .raw
                            .try_into()
                            .map_err(|e| log::error!("Unable to convert Quantity to f64: {e}"))
                            .unwrap(),
                        base_currency,
                    );

                    if self.config.debug {
                        log::debug!("Cash value: {cash_value:?}");
                        log::debug!(
                            "Total: {:?}",
                            cash_account.balance_total(Some(base_currency))
                        );
                        log::debug!(
                            "Locked: {:?}",
                            cash_account.balance_locked(Some(base_currency))
                        );
                        log::debug!("Free: {:?}", cash_account.balance_free(Some(base_currency)));
                    }

                    match cum_notional_sell {
                        Some(mut value) => {
                            value.raw += cash_value.raw;
                            cum_notional_sell = Some(value);
                        }
                        None => cum_notional_sell = Some(cash_value),
                    }

                    if self.config.debug {
                        log::debug!("Cumulative notional SELL: {cum_notional_sell:?}");
                    }
                    if !allow_borrowing
                        && let (Some(free), Some(cum_notional_sell)) = (free, cum_notional_sell)
                        && cum_notional_sell.raw > free.raw
                    {
                        self.deny_order(order.clone(), &format!("CUM_NOTIONAL_EXCEEDS_FREE_BALANCE: free={free}, cum_notional={cum_notional_sell}"));
                        return false; // Denied
                    }
                }
            }
        }

        // Finally
        true // Passed
    }

    fn check_price(&self, instrument: &InstrumentAny, price: Option<Price>) -> Option<String> {
        let price_val = price?;

        if price_val.precision > instrument.price_precision() {
            return Some(format!(
                "price {} invalid (precision {} > {})",
                price_val,
                price_val.precision,
                instrument.price_precision()
            ));
        }

        if !matches!(
            instrument.instrument_class(),
            InstrumentClass::Option
                | InstrumentClass::FuturesSpread
                | InstrumentClass::OptionSpread
        ) && price_val.raw <= 0
        {
            return Some(format!("price {price_val} invalid (<= 0)"));
        }

        // ---- tick 对齐检查 ----
        if !price_val.is_on_tick(instrument.price_increment()) {
            return Some(format!(
                "PRICE_NOT_ON_TICK: price={price_val}, tick={}",
                instrument.price_increment()
            ));
        }

        // ---- 涨跌停价检查 ----
        if let Some(max_price) = instrument.max_price() {
            if price_val > max_price {
                return Some(format!(
                    "PRICE_ABOVE_UP_LIMIT: price={price_val}, up_limit={max_price}"
                ));
            }
        }
        if let Some(min_price) = instrument.min_price() {
            if price_val < min_price {
                return Some(format!(
                    "PRICE_BELOW_DOWN_LIMIT: price={price_val}, down_limit={min_price}"
                ));
            }
        }

        None
    }

    fn check_quantity(
        &self,
        instrument: &InstrumentAny,
        quantity: Option<Quantity>,
        is_quote_quantity: bool,
    ) -> Option<String> {
        let quantity_val = quantity?;

        // Check precision
        if quantity_val.precision > instrument.size_precision() {
            return Some(format!(
                "quantity {} invalid (precision {} > {})",
                quantity_val,
                quantity_val.precision,
                instrument.size_precision()
            ));
        }

        // Skip min/max checks for quote quantities (they will be checked in check_orders_risk using effective_quantity)
        if is_quote_quantity {
            return None;
        }

        // Check maximum quantity
        if let Some(max_quantity) = instrument.max_quantity()
            && quantity_val > max_quantity
        {
            return Some(format!(
                "quantity {quantity_val} invalid (> maximum trade size of {max_quantity})"
            ));
        }

        // Check minimum quantity
        if let Some(min_quantity) = instrument.min_quantity()
            && quantity_val < min_quantity
        {
            return Some(format!(
                "quantity {quantity_val} invalid (< minimum trade size of {min_quantity})"
            ));
        }

        None
    }

    fn deny_command(&self, command: TradingCommand, reason: &str) {
        match command {
            TradingCommand::SubmitOrder(command) => {
                let order = {
                    let cache = self.cache.borrow();
                    cache.order(&command.client_order_id).cloned()
                };
                if let Some(order) = order {
                    self.deny_order(order, reason);
                } else {
                    log::error!(
                        "Cannot deny order: not found in cache for {}",
                        command.client_order_id
                    );
                }
            }
            TradingCommand::SubmitOrderList(command) => {
                let orders: Vec<OrderAny> = self
                    .cache
                    .borrow()
                    .orders_for_ids(&command.order_list.client_order_ids, &command);
                self.deny_order_list(&orders, reason);
            }
            _ => {
                panic!("Cannot deny command {command}");
            }
        }
    }

    fn deny_order(&self, order: OrderAny, reason: &str) {
        log::warn!(
            "SubmitOrder for {} DENIED: {}",
            order.client_order_id(),
            reason
        );

        if order.status() != OrderStatus::Initialized {
            return;
        }

        // Scope the cache borrow to avoid RefCell conflict when sending to ExecEngine
        {
            let mut cache = self.cache.borrow_mut();
            if !cache.order_exists(&order.client_order_id()) {
                cache
                    .add_order(order.clone(), None, None, false)
                    .map_err(|e| {
                        log::error!("Cannot add order to cache: {e}");
                    })
                    .unwrap();
            }
        }

        let denied = OrderEventAny::Denied(OrderDenied::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            reason.into(),
            UUID4::new(),
            self.clock.borrow().timestamp_ns(),
            self.clock.borrow().timestamp_ns(),
        ));

        let endpoint = MessagingSwitchboard::exec_engine_process();
        msgbus::send_order_event(endpoint, denied);
    }

    fn deny_order_list(&self, orders: &[OrderAny], reason: &str) {
        for order in orders {
            if !order.is_closed() {
                self.deny_order(order.clone(), reason);
            }
        }
    }

    fn reject_modify_order(&self, order: OrderAny, reason: &str) {
        let ts_event = self.clock.borrow().timestamp_ns();
        let denied = OrderEventAny::ModifyRejected(OrderModifyRejected::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            reason.into(),
            UUID4::new(),
            ts_event,
            ts_event,
            false,
            order.venue_order_id(),
            order.account_id(),
        ));

        let endpoint = MessagingSwitchboard::exec_engine_process();
        msgbus::send_order_event(endpoint, denied);
    }

    fn execution_gateway(&mut self, instrument: InstrumentAny, command: TradingCommand) {
        // ---- 新增：撤单时段检查 ----
        if let TradingCommand::CancelOrder(ref cancel) = command {
            if let Some(ref session) = self.session_provider {
                let phase = session.phase_at(
                    &cancel.instrument_id.venue,
                    self.clock.borrow().timestamp_ns(),
                );
                if !phase.can_cancel_order() {
                    log::warn!("CancelOrder DENIED: phase={phase:?} does not allow cancellation");
                    return;
                }
            }
        }

        match self.trading_state {
            TradingState::Halted => match command {
                TradingCommand::SubmitOrder(submit_order) => {
                    let order = {
                        let cache = self.cache.borrow();
                        cache.order(&submit_order.client_order_id).cloned()
                    };
                    if let Some(order) = order {
                        self.deny_order(order, "TradingState::HALTED");
                    }
                }
                TradingCommand::SubmitOrderList(submit_order_list) => {
                    let orders: Vec<OrderAny> = self.cache.borrow().orders_for_ids(
                        &submit_order_list.order_list.client_order_ids,
                        &submit_order_list,
                    );
                    self.deny_order_list(&orders, "TradingState::HALTED");
                }
                _ => {}
            },
            TradingState::Reducing => match command {
                TradingCommand::SubmitOrder(submit_order) => {
                    let order = {
                        let cache = self.cache.borrow();
                        cache.order(&submit_order.client_order_id).cloned()
                    };
                    if let Some(order) = order {
                        if order.is_buy() && self.portfolio.is_net_long(&instrument.id()) {
                            self.deny_order(
                                order,
                                &format!(
                                    "BUY when TradingState::REDUCING and LONG {}",
                                    instrument.id()
                                ),
                            );
                        } else if order.is_sell() && self.portfolio.is_net_short(&instrument.id()) {
                            self.deny_order(
                                order,
                                &format!(
                                    "SELL when TradingState::REDUCING and SHORT {}",
                                    instrument.id()
                                ),
                            );
                        }
                    }
                }
                TradingCommand::SubmitOrderList(submit_order_list) => {
                    let orders: Vec<OrderAny> = self.cache.borrow().orders_for_ids(
                        &submit_order_list.order_list.client_order_ids,
                        &submit_order_list,
                    );
                    for order in &orders {
                        if order.is_buy() && self.portfolio.is_net_long(&instrument.id()) {
                            self.deny_order_list(
                                &orders,
                                &format!(
                                    "BUY when TradingState::REDUCING and LONG {}",
                                    instrument.id()
                                ),
                            );
                            return;
                        } else if order.is_sell() && self.portfolio.is_net_short(&instrument.id()) {
                            self.deny_order_list(
                                &orders,
                                &format!(
                                    "SELL when TradingState::REDUCING and SHORT {}",
                                    instrument.id()
                                ),
                            );
                            return;
                        }
                    }
                }
                _ => {}
            },
            TradingState::Active => match command {
                TradingCommand::SubmitOrder(submit_order) => {
                    // ---- 新增：符号级限流 ----
                    if let Some(rate_limit) = &self.config.max_order_submit_per_symbol {
                        let throttler = self
                            .symbol_throttlers
                            .entry(submit_order.instrument_id)
                            .or_insert_with(|| {
                                Self::create_submit_order_throttler_with_rate(
                                    rate_limit.clone(),
                                    self.clock.clone(),
                                    format!("SYMBOL_THROTTLER_{}", submit_order.instrument_id),
                                )
                            });
                        if throttler.used() >= 1.0 {
                            self.deny_command(
                                TradingCommand::SubmitOrder(submit_order.clone()),
                                "SYMBOL_RATE_LIMIT_EXCEEDED",
                            );
                            return;
                        } else {
                            throttler.send_msg(submit_order.clone());
                        }
                    }

                    // ---- 新增：账户级限流 ----
                    if let Some(rate_limit) = &self.config.max_order_submit_per_account {
                        let account_id_opt = self
                            .cache
                            .borrow()
                            .order(&submit_order.client_order_id)
                            .and_then(|o| o.account_id());
                        if let Some(account_id) = account_id_opt {
                            let throttler = self
                                .account_throttlers
                                .entry(account_id)
                                .or_insert_with(|| {
                                    Self::create_submit_order_throttler_with_rate(
                                        rate_limit.clone(),
                                        self.clock.clone(),
                                        format!("ACCOUNT_THROTTLER_{account_id}"),
                                    )
                                });
                            if throttler.used() >= 1.0 {
                                self.deny_command(
                                    TradingCommand::SubmitOrder(submit_order.clone()),
                                    "ACCOUNT_RATE_LIMIT_EXCEEDED",
                                );
                                return;
                            } else {
                                throttler.send_msg(submit_order.clone());
                            }
                        }
                    }

                    self.throttled_submit_order.send(submit_order);
                }
                TradingCommand::SubmitOrderList(submit_order_list) => {
                    // TODO: implement throttler for order lists
                    self.send_to_execution(TradingCommand::SubmitOrderList(submit_order_list));
                }
                _ => {}
            },
        }
    }

    fn send_to_execution(&self, command: TradingCommand) {
        if let Some(gt) = &self.global_trade_throttler {
            if matches!(
                command,
                TradingCommand::SubmitOrder(_)
                    | TradingCommand::CancelOrder(_)
                    | TradingCommand::ModifyOrder(_)
            ) {
                gt.borrow_mut().send(command);
                return;
            }
        }

        let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
        msgbus::send_trading_command(endpoint, command);
    }

    fn handle_event(&mut self, event: &OrderEventAny) {
        // We intend to extend the risk engine to be able to handle additional events.
        // For now we just log.
        if self.config.debug {
            log::debug!("{RECV}{EVT} {event:?}");
        }

        // ---- T1Ledger 更新 ----
        if let Some(ledger) = &self.t1_ledger {
            if let OrderEventAny::Filled(fill) = event {
                ledger.write().expect("T1 ledger lock poisoned").on_fill(
                    fill.account_id,
                    fill.instrument_id,
                    fill.order_side,
                    fill.last_qty.as_f64(),
                );
            }
        }
    }

    fn handle_instrument_status(&mut self, status: &InstrumentStatus) {
        log::debug!("Handling instrument status: {:?}", status);

        use nautilus_model::enums::MarketStatusAction;
        match status.action {
            MarketStatusAction::Halt
            | MarketStatusAction::Suspend
            | MarketStatusAction::NotAvailableForTrading => {
                log::warn!(
                    "标的 {} 已停牌/临停: action={:?}, reason={:?}",
                    status.instrument_id,
                    status.action,
                    status.reason
                );
            }
            MarketStatusAction::Resume => {
                log::info!(
                    "标的 {} 已复牌: action={:?}",
                    status.instrument_id,
                    status.action
                );
            }
            _ => {}
        }
    }

    fn create_submit_order_throttler_with_rate(
        rate_limit: nautilus_common::throttler::RateLimit,
        clock: Rc<RefCell<dyn Clock>>,
        name: String,
    ) -> Throttler<SubmitOrder, SubmitOrderFn> {
        let success_handler = { Box::new(move |_| {}) as Box<dyn Fn(SubmitOrder)> };
        Throttler::new(
            rate_limit.limit,
            rate_limit.interval_ns,
            clock,
            name,
            success_handler,
            None,
            Ustr::from(UUID4::new().as_str()),
        )
    }
}
