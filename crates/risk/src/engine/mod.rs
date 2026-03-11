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

//! 风险管理引擎的实现。

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
    identifiers::InstrumentId,
    instruments::{Instrument, InstrumentAny},
    orders::{Order, OrderAny},
    types::{Currency, Money, Price, Quantity, quantity::QuantityRaw},
};
use nautilus_portfolio::Portfolio;
use nautilus_rules::common::{RuleChain, RuleCheckResult, RuleContext};
use nautilus_rules::command::{CommandContext, CommandRuleChain, CommandRuleCheckResult};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use ustr::Ustr;

type SubmitOrderFn = Box<dyn Fn(SubmitOrder)>;
type ModifyOrderFn = Box<dyn Fn(ModifyOrder)>;

/// 核心风险管理引擎，用于验证和控制交易操作。
///
/// `RiskEngine` 提供全面的事前风险检查，包括订单验证、
/// 资金余额验证、持仓规模限制以及交易状态管理。它作为
/// 策略订单与执行系统之间的网关，确保所有交易均符合
/// 定义的风险参数和监管限制。
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

    pre_trade_rules: Option<RuleChain>,
    pre_trade_command_rules: Option<CommandRuleChain>,
}

impl Debug for RiskEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(RiskEngine)).finish()
    }
}

impl RiskEngine {
    /// 创建一个新的 [`RiskEngine`] 实例。
    pub fn new(
        config: RiskEngineConfig,
        portfolio: Portfolio,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> Self {
        let throttled_submit_order = Self::create_submit_order_throttler(
            &config,
            clock.clone(),
            cache.clone(),
        );

        let throttled_modify_order = Self::create_modify_order_throttler(
            &config,
            clock.clone(),
            cache.clone(),
        );

        Self {
            clock,
            cache,
            portfolio,
            throttled_submit_order,
            throttled_modify_order,
            max_notional_per_order: AHashMap::new(),
            trading_state: TradingState::Active,
            config,
            pre_trade_rules: None,
            pre_trade_command_rules: None,
        }
    }

    pub fn set_pre_trade_rules(&mut self, rules: Option<RuleChain>) {
        self.pre_trade_rules = rules;
    }

    pub fn set_pre_trade_command_rules(&mut self, rules: Option<CommandRuleChain>) {
        self.pre_trade_command_rules = rules;
    }

    fn check_pre_trade_command_rules(&self, command: TradingCommand) -> Option<String> {
        let rules = self.pre_trade_command_rules.as_ref()?;
        let instrument_id = match &command {
            TradingCommand::QueryAccount(_) => None,
            _ => Some(command.instrument_id()),
        };

        let account_id = match &command {
            TradingCommand::CancelOrder(c) => self
                .cache
                .borrow()
                .order(&c.client_order_id)
                .and_then(|o| o.account_id()),
            TradingCommand::ModifyOrder(m) => self
                .cache
                .borrow()
                .order(&m.client_order_id)
                .and_then(|o| o.account_id()),
            _ => None,
        };

        let ts = self.clock.borrow().timestamp_ns().as_u64();
        let ctx = CommandContext {
            command,
            instrument_id,
            account_id,
            timestamp_ns: ts,
        };

        match rules.check(&ctx) {
            CommandRuleCheckResult::Pass => None,
            CommandRuleCheckResult::Fail { reason } => Some(reason),
        }
    }

    /// 为风险引擎注册所有的消息总线处理器。
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
    ) -> Throttler<SubmitOrder, SubmitOrderFn> {
        let success_handler = {
            Box::new(move |submit_order: SubmitOrder| {
                let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
                msgbus::send_trading_command(endpoint, TradingCommand::SubmitOrder(submit_order));
            }) as Box<dyn Fn(SubmitOrder)>
        };

        let failure_handler = {
            let cache = cache;
            let clock = clock.clone();
            Box::new(move |submit_order: SubmitOrder| {
                let reason = "流控器（Throttler）拒绝";
                log::warn!(
                    "{} 的提交订单（SubmitOrder）被拒绝：{}",
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
    ) -> Throttler<ModifyOrder, ModifyOrderFn> {
        let success_handler = {
            Box::new(move |order: ModifyOrder| {
                let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
                msgbus::send_trading_command(endpoint, TradingCommand::ModifyOrder(order));
            }) as Box<dyn Fn(ModifyOrder)>
        };

        let failure_handler = {
            let cache = cache;
            let clock = clock.clone();
            Box::new(move |order: ModifyOrder| {
                let reason = "超过了最大订单修改速率限制（MAX_ORDER_MODIFY_RATE）";
                log::warn!(
                    "{} 的提交订单（SubmitOrder）被拒绝：{}",
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
                "缓存中未找到 client_order_id 为 {} 的订单",
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
                "未找到 client_order_id 为 {} 的订单",
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


    /// 通过风险管理流水线执行一项交易命令。
    pub fn execute(&mut self, command: TradingCommand) {
        // 这将扩展到其他命令，例如 `RiskCommand`
        self.handle_command(command);
    }

    /// 处理订单事件以进行风险监控和状态更新。
    pub fn process(&mut self, event: &OrderEventAny) {
        // 这将扩展到其他事件，例如 `RiskEvent`
        self.handle_event(event);
    }

    /// 设置用于风险控制执行的交易状态。
    pub fn set_trading_state(&mut self, state: TradingState) {
        if state == self.trading_state {
            log::warn!("交易状态无变化：已设置为 {state:?}");
            return;
        }

        self.trading_state = state;

        let _ts_now = self.clock.borrow().timestamp_ns();

        // TODO：在 OrderEventAny 枚举中创建一个新的事件 "TradingStateChanged"。
        // let event = OrderEventAny::TradingStateChanged(TradingStateChanged::new(..,self.trading_state,..));

        msgbus::publish_any("events.risk".into(), &"message"); // TODO：在这里发送新事件

        log::info!("交易状态已设置为 {state:?}");
    }

    /// 为指定的标的设置每笔订单的最大名义价值。
    pub fn set_max_notional_per_order(&mut self, instrument_id: InstrumentId, new_value: Decimal) {
        self.max_notional_per_order.insert(instrument_id, new_value);

        let new_value_str = new_value.to_string();
        log::info!("已设置每笔订单最大名义价值：{instrument_id} {new_value_str}");
    }

    /// 启动风险引擎。
    pub fn start(&mut self) {
        log::info!("已启动");
    }

    /// 停止风险引擎。
    pub fn stop(&mut self) {
        log::info!("已停止");
    }

    /// 将风险引擎重置为其初始状态。
    pub fn reset(&mut self) {
        self.throttled_submit_order.reset();
        self.throttled_modify_order.reset();
        self.max_notional_per_order.clear();
        self.trading_state = TradingState::Active;

        log::info!("已重置");
    }

    fn check_pre_trade_rules(&self, order: OrderAny, instrument_id: InstrumentId) -> Option<String> {
        let rules = self.pre_trade_rules.as_ref()?;

        let account_id = order.account_id();
        let timestamp_ns = self.clock.borrow().timestamp_ns().as_u64();
        let context = RuleContext::new(order, instrument_id, account_id, timestamp_ns);
        match rules.check(&context) {
            RuleCheckResult::Pass => None,
            RuleCheckResult::Fail { reason } => Some(reason),
        }
    }

    /// 释放风险引擎，释放资源。
    pub fn dispose(&mut self) {
        log::info!("已释放");
    }

    /// 返回时钟的引用。
    #[must_use]
    pub fn clock(&self) -> &Rc<RefCell<dyn Clock>> {
        &self.clock
    }

    /// 返回缓存的引用。
    #[must_use]
    pub fn cache(&self) -> &Rc<RefCell<Cache>> {
        &self.cache
    }

    /// 返回配置的引用。
    #[must_use]
    pub const fn config(&self) -> &RiskEngineConfig {
        &self.config
    }

    /// 返回当前的交易状态。
    #[must_use]
    pub const fn trading_state(&self) -> TradingState {
        self.trading_state
    }

    /// 返回每笔订单最大名义价值设置的引用。
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
                // BatchCancelOrders forwarded directly (individual cancel order checks handled by each CancelOrder)
                self.send_to_execution(TradingCommand::BatchCancelOrders(batch));
            }
            TradingCommand::QueryAccount(query_account) => {
                self.send_to_execution(TradingCommand::QueryAccount(query_account));
            }
            _ => {
                log::error!("无法处理命令：{command}");
            }
        }
    }

    /// 撤单命令处理。
    fn handle_cancel_order(&mut self, command: CancelOrder) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::CancelOrder(command));
            return;
        }

        let wrapped = TradingCommand::CancelOrder(command.clone());
        if let Some(reason) = self.check_pre_trade_command_rules(wrapped.clone()) {
            let ts_event = self.clock.borrow().timestamp_ns();
            let rejected = OrderEventAny::CancelRejected(OrderCancelRejected::new(
                command.trader_id,
                command.strategy_id,
                command.instrument_id,
                command.client_order_id,
                Ustr::from(&reason),
                UUID4::new(),
                ts_event,
                command.ts_init,
                false,
                command.venue_order_id,
                None,
            ));
            let endpoint = MessagingSwitchboard::exec_engine_process();
            msgbus::send_order_event(endpoint, rejected);
            return;
        }

        self.send_to_execution(TradingCommand::CancelOrder(command));
    }

    /// 全部撤单命令处理。
    fn handle_cancel_all_orders(&mut self, command: CancelAllOrders) {
        if self.config.bypass {
            self.send_to_execution(TradingCommand::CancelAllOrders(command));
            return;
        }

        let wrapped = TradingCommand::CancelAllOrders(command.clone());
        if let Some(reason) = self.check_pre_trade_command_rules(wrapped) {
            // CancelAllOrders 没有单个订单 ID；暂时发出一个通用的风险事件。
            msgbus::publish_any("events.risk".into(), &reason);
            return;
        }

        self.send_to_execution(TradingCommand::CancelAllOrders(command));
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
                        "无法处理下单：缓存中未找到 {} 的订单",
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
                        &format!("只减仓订单（Reduce only）会增加标的 {position_id} 的持仓"),
                    );
                    return; // 已拒绝
                }
            } else {
                self.deny_command(
                    TradingCommand::SubmitOrder(command),
                    &format!("未找到只减仓订单对应的仓位 {position_id}"),
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
                &format!("未找到 {} 对应的标的", command.instrument_id),
            );
            return; // Denied
        };

        if let Some(reason) = self.check_pre_trade_rules(order.clone(), command.instrument_id) {
            self.deny_command(TradingCommand::SubmitOrder(command), &reason);
            return;
        }

        if !self.check_order(instrument.clone(), order.clone()) {
            return; // Denied
        }

        if !self.check_orders_risk(instrument.clone(), &[order]) {
            return; // Denied
        }

        // 通过执行网关进行路由，以进行交易状态检查和流控限制
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
                &format!("未找到 {} 对应的标的", command.instrument_id),
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
                &format!("订单列表不完整：缓存中缺失 {} 的订单", command),
            );
            return; // Denied
        }

        for order in orders.clone() {
            if !self.check_order(instrument.clone(), order) {
                return; // Denied
            }
        }

        for order in &orders {
            if let Some(reason) = self.check_pre_trade_rules(order.clone(), command.instrument_id) {
                self.deny_order_list(&orders, &reason);
                return;
            }
        }

        if !self.check_orders_risk(instrument.clone(), &orders) {
            self.deny_order_list(
                &orders,
                &format!("订单列表（OrderList）{} 被拒绝", command.order_list.id),
            );
            return; // Denied
        }

        self.execution_gateway(instrument, TradingCommand::SubmitOrderList(command));
    }

    fn handle_modify_order(&mut self, command: ModifyOrder) {
        if let Some(reason) = self.check_pre_trade_command_rules(TradingCommand::ModifyOrder(command.clone())) {
            let order = match Self::get_existing_order(&self.cache, &command) {
                Some(order) => order,
                None => return,
            };
            let rejected = Self::create_modify_rejected(&order, &reason, &self.clock);
            let endpoint = MessagingSwitchboard::exec_engine_process();
            msgbus::send_order_event(endpoint, rejected);
            return;
        }

        let order_exists = {
            let cache = self.cache.borrow();
            cache.order(&command.client_order_id).cloned()
        };

        let order = if let Some(order) = order_exists {
            order
        } else {
            log::error!(
                "修改订单请求被拒绝：未找到 client_order_id 为 {} 的订单",
                command.client_order_id
            );
            return;
        };

        if order.is_closed() {
            self.reject_modify_order(
                order,
                &format!(
                    "client_order_id 为 {} 的订单已收盘/关闭",
                    command.client_order_id
                ),
            );
            return;
        } else if order.status() == OrderStatus::PendingCancel {
            self.reject_modify_order(
                order,
                &format!(
                    "client_order_id 为 {} 的订单已在等待撤单中",
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
                &format!("未找到 {:?} 对应的标的", command.instrument_id),
            );
            return; // Denied
        };

        // 检查价格
        let mut risk_msg = self.check_price(&instrument, command.price);
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // 检查触发价
        risk_msg = self.check_price(&instrument, command.trigger_price);
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // 检查数量
        risk_msg = self.check_quantity(&instrument, command.quantity, order.is_quote_quantity());
        if let Some(risk_msg) = risk_msg {
            self.reject_modify_order(order, &risk_msg);
            return; // Denied
        }

        // 检查标的状态
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

        // 检查交易状态
        match self.trading_state {
            TradingState::Halted => {
                self.reject_modify_order(order, "交易状态为停牌/停盘 (HALTED)：无法修改订单");
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
                            "交易状态为减仓 (REDUCING)，且更新将增加标的 {} 的敞口",
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
            // 安全性：GTD 保证具有过期时间
            let expire_time = order.expire_time().unwrap();
            if expire_time <= self.clock.borrow().timestamp_ns() {
                self.deny_order(
                    order,
                    &format!("GTD 订单已超过过期时间 {}", expire_time.to_rfc3339()),
                );
                return false; // 已拒绝
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
                return false; // 已拒绝
            }
        }

        if order.trigger_price().is_some() {
            let risk_msg = self.check_price(&instrument, order.trigger_price());
            if let Some(risk_msg) = risk_msg {
                self.deny_order(order, &risk_msg);
                return false; // 已拒绝
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

        // 确定最大名义价值
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
            log::debug!("未找到柜台 {} 的账户", instrument.id().venue);
            return true; // TODO：暂时提前返回直到处理路由/多柜台逻辑
        };
        let cash_account = match account {
            AccountAny::Cash(cash_account) => cash_account,
            AccountAny::Margin(_) => return true, // TODO：确定保证金账户的风险控制
        };
        let free = cash_account.balance_free(Some(instrument.quote_currency()));
        let allow_borrowing = cash_account.allow_borrowing;
        if self.config.debug {
            log::debug!("空闲现金：{free:?}");
        }

        // 获取该标的的净多头持仓数量（用于减仓卖出检查），
        // 考虑已提交（但未成交）的卖单，以防止超卖。
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

        // 可用数量等于多头持仓减去挂单中的卖单
        let available_long_qty_raw = net_long_qty_raw.saturating_sub(pending_sell_qty_raw);

        if self.config.debug && net_long_qty_raw > 0 {
            log::debug!(
                "净多头持仓（raw）：{net_long_qty_raw}，挂单卖出：{pending_sell_qty_raw}，可用：{available_long_qty_raw}"
            );
        }

        // 追踪累计卖出数量，以确定是减仓型卖出还是开仓型卖出
        let mut cum_sell_qty_raw: QuantityRaw = 0;

        let mut cum_notional_buy: Option<Money> = None;
        let mut cum_notional_sell: Option<Money> = None;
        let mut base_currency: Option<Currency> = None;
        for order in orders {
            // 根据订单类型确定最新价格
            last_px = match order {
                OrderAny::Market(_) | OrderAny::MarketToLimit(_) => {
                    if last_px.is_none() {
                        let cache = self.cache.borrow();
                        if let Some(last_quote) = cache.quote(&instrument.id()) {
                            match order.order_side() {
                                OrderSide::Buy => Some(last_quote.ask_price),
                                OrderSide::Sell => Some(last_quote.bid_price),
                                _ => panic!("无效的订单方向（OrderSide）"),
                            }
                        } else {
                            let cache = self.cache.borrow();
                            let last_trade = cache.trade(&instrument.id());

                            if let Some(last_trade) = last_trade {
                                Some(last_trade.price)
                            } else {
                                log::warn!(
                                    "无法检查市价单（MARKET）风险：{} 无报价",
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
                                &format!("不支持的追踪偏移类型（UNSUPPORTED_TRAILING_OFFSET_TYPE）：{offset_type:?}"),
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
                                            "无法检查 {} 订单风险：无法从追踪偏移量计算触发价：{e}",
                                            order.order_type()
                                        );
                                        continue;
                                    }
                                }
                            } else {
                                log::warn!(
                                    "无法检查 {} 订单风险：未设置触发价，且 {} 无可用报价数据",
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
                                        "无法检查 {} 订单风险：无法从追踪偏移量计算触发价：{}",
                                        order.order_type(),
                                        e
                                    );
                                    continue;
                                }
                            }
                        } else if trigger_type == TriggerType::LastOrBidAsk {
                            // 当没有成交数据可用时，回退到使用买卖报价
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
                                            "无法检查 {} 订单风险：无法从追踪偏移量计算触发价：{e}",
                                            order.order_type()
                                        );
                                        continue;
                                    }
                                }
                            } else {
                                log::warn!(
                                    "无法检查 {} 订单风险：未设置触发价，且 {} 无可用市场数据",
                                    order.order_type(),
                                    instrument.id()
                                );
                                continue;
                            }
                        } else {
                            log::warn!(
                                "无法检查 {} 订单风险：未设置触发价，且 {} 无可用市场数据",
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
                log::error!("无法检查订单风险：无可用的价格数据");
                continue;
            };

            // 对于报价币种数量的限价单，使用最坏情况下的执行价格
            let effective_price = if order.is_quote_quantity()
                && !instrument.is_inverse()
                && matches!(order, OrderAny::Limit(_) | OrderAny::StopLimit(_))
            {
                // 获取当前市场价格以进行最坏情况下的执行
                let cache = self.cache.borrow();
                if let Some(quote_tick) = cache.quote(&instrument.id()) {
                    match order.order_side() {
                        // 买入：如果低于限价，则可能以最佳卖价执行（更多数量）
                        OrderSide::Buy => last_px.min(quote_tick.ask_price),
                        // 卖出：如果高于限价，则可能以最佳买价执行（但数量较少，故限价执行）
                        OrderSide::Sell => last_px.max(quote_tick.bid_price),
                        _ => last_px,
                    }
                } else {
                    last_px // 无市场数据，使用限价
                }
            } else {
                last_px
            };

            let effective_quantity = if order.is_quote_quantity() && !instrument.is_inverse() {
                instrument.calculate_base_quantity(order.quantity(), effective_price)
            } else {
                order.quantity()
            };

            // 根据有效数量检查最小/最大数量限制
            if let Some(max_quantity) = instrument.max_quantity()
                && effective_quantity > max_quantity
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "数量超过最大限制：有效数量={effective_quantity}，最大数量={max_quantity}"
                    ),
                );
                return false; // 已拒绝
            }

            if let Some(min_quantity) = instrument.min_quantity()
                && effective_quantity < min_quantity
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "数量低于最小限制：有效数量={effective_quantity}，最小数量={min_quantity}"
                    ),
                );
                return false; // 已拒绝
            }

            let notional =
                instrument.calculate_notional_value(effective_quantity, last_px, Some(true));

            if self.config.debug {
                log::debug!("名义价值：{notional:?}");
            }

            // 检查每笔订单的最大名义价值限制
            if let Some(max_notional_value) = max_notional
                && notional > max_notional_value
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "名义价值超过每笔订单最大限制：最大限制={max_notional_value:?}，当前名义价值={notional:?}"
                        ),
                    );
                return false; // 已拒绝
            }

            // 检查标的的最小名义价值限制
            if let Some(min_notional) = instrument.min_notional()
                && notional.currency == min_notional.currency
                && notional < min_notional
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "名义价值低于标的最小限制：最小限制={min_notional:?}，当前名义价值={notional:?}"
                        ),
                    );
                return false; // 已拒绝
            }

            // 检查标地的最大名义价值限制
            if let Some(max_notional) = instrument.max_notional()
                && notional.currency == max_notional.currency
                && notional > max_notional
            {
                self.deny_order(
                        order.clone(),
                        &format!(
                            "名义价值超过标的最大限制：最大限制={max_notional:?}，当前名义价值={notional:?}"
                        ),
                    );
                return false; // 已拒绝
            }

            // 计算订单余额影响（仅对现金账户有效）
            let notional = instrument.calculate_notional_value(effective_quantity, last_px, None);
            let order_balance_impact = match order.order_side() {
                OrderSide::Buy => Money::from_raw(-notional.raw, notional.currency),
                OrderSide::Sell => Money::from_raw(notional.raw, notional.currency),
                OrderSide::NoOrderSide => {
                    panic!("无效的订单方向（OrderSide）：{}", order.order_side());
                }
            };

            if self.config.debug {
                log::debug!("余额影响：{order_balance_impact}");
            }

            // 当启用借贷（例如现货杠杆交易）时，跳过余额检查
            if !allow_borrowing
                && let Some(free_val) = free
                && (free_val.as_decimal() + order_balance_impact.as_decimal()) < Decimal::ZERO
            {
                self.deny_order(
                    order.clone(),
                    &format!(
                        "名义价值超过可用余额：可用余额={free_val:?}，当前名义价值={notional:?}"
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
                    log::debug!("累计买入名义价值：{cum_notional_buy:?}");
                }

                if !allow_borrowing
                    && let (Some(free), Some(cum_notional_buy)) = (free, cum_notional_buy)
                    && cum_notional_buy > free
                {
                    self.deny_order(order.clone(), &format!("累计名义价值超过可用余额：可用余额={free}，累计名义价值={cum_notional_buy}"));
                    return false; // 已拒绝
                }
            } else if order.is_sell() {
                let is_position_reducing_sell = order.is_reduce_only()
                    || (cum_sell_qty_raw + effective_quantity.raw) <= available_long_qty_raw;
                cum_sell_qty_raw += effective_quantity.raw;

                if is_position_reducing_sell {
                    if self.config.debug {
                        log::debug!("减仓型卖出（SELL）跳过余额检查");
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
                        log::debug!("累计卖出名义价值：{cum_notional_sell:?}");
                    }

                    if !allow_borrowing
                        && let (Some(free), Some(cum_notional_sell)) = (free, cum_notional_sell)
                        && cum_notional_sell > free
                    {
                        self.deny_order(order.clone(), &format!("累计名义价值超过可用余额：可用余额={free}，累计名义价值={cum_notional_sell}"));
                        return false; // 已拒绝
                    }
                }
                // 账户已经是现金类型，因此不进行检查
                else if let Some(base_currency) = base_currency {
                    let cash_value = Money::from_raw(
                        effective_quantity
                            .raw
                            .try_into()
                            .map_err(|e| log::error!("无法将 Quantity 转换为 f64：{e}"))
                            .unwrap(),
                        base_currency,
                    );

                    if self.config.debug {
                        log::debug!("现金价值：{cash_value:?}");
                        log::debug!(
                            "总额：{:?}",
                            cash_account.balance_total(Some(base_currency))
                        );
                        log::debug!(
                            "锁定：{:?}",
                            cash_account.balance_locked(Some(base_currency))
                        );
                        log::debug!("空闲：{:?}", cash_account.balance_free(Some(base_currency)));
                    }

                    match cum_notional_sell {
                        Some(mut value) => {
                            value.raw += cash_value.raw;
                            cum_notional_sell = Some(value);
                        }
                        None => cum_notional_sell = Some(cash_value),
                    }

                    if self.config.debug {
                        log::debug!("累计卖出名义价值：{cum_notional_sell:?}");
                    }
                    if !allow_borrowing
                        && let (Some(free), Some(cum_notional_sell)) = (free, cum_notional_sell)
                        && cum_notional_sell.raw > free.raw
                    {
                        self.deny_order(order.clone(), &format!("累计名义价值超过可用余额：可用余额={free}，累计名义价值={cum_notional_sell}"));
                        return false; // 已拒绝
                    }
                }
            }
        }

        // 已通过
        true // 已通过
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

        None
    }

    fn check_quantity(
        &self,
        instrument: &InstrumentAny,
        quantity: Option<Quantity>,
        is_quote_quantity: bool,
    ) -> Option<String> {
        let quantity_val = quantity?;

        // 检查精度
        if quantity_val.precision > instrument.size_precision() {
            return Some(format!(
                "quantity {} invalid (precision {} > {})",
                quantity_val,
                quantity_val.precision,
                instrument.size_precision()
            ));
        }

        // 对于报价币种数量，跳过最小/最大检查（这些将在 check_orders_risk 中使用有效数量进行检查）
        if is_quote_quantity {
            return None;
        }

        // 检查最大数量限制
        if let Some(max_quantity) = instrument.max_quantity()
            && quantity_val > max_quantity
        {
            return Some(format!(
                "quantity {quantity_val} invalid (> maximum trade size of {max_quantity})"
            ));
        }

        // 检查最小数量限制
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
                        "无法拒绝订单：缓存中未找到 {}",
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
                panic!("无法拒绝命令：{command}");
            }
        }
    }

    fn deny_order(&self, order: OrderAny, reason: &str) {
        log::warn!(
            "{} 的提交订单（SubmitOrder）被拒绝：{}",
            order.client_order_id(),
            reason
        );

        if order.status() != OrderStatus::Initialized {
            return;
        }

        // 作用域化缓存借用以避免发送到 ExecEngine 时出现 RefCell 冲突
        {
            let mut cache = self.cache.borrow_mut();
            if !cache.order_exists(&order.client_order_id()) {
                cache
                    .add_order(order.clone(), None, None, false)
                    .map_err(|e| {
                        log::error!("无法将订单添加到缓存：{e}");
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
        match self.trading_state {
            TradingState::Halted => match command {
                TradingCommand::SubmitOrder(submit_order) => {
                    let order = {
                        let cache = self.cache.borrow();
                        cache.order(&submit_order.client_order_id).cloned()
                    };
                    if let Some(order) = order {
                        self.deny_order(order, "交易状态：停牌/停盘 (TradingState::HALTED)");
                    }
                }
                TradingCommand::SubmitOrderList(submit_order_list) => {
                    let orders: Vec<OrderAny> = self.cache.borrow().orders_for_ids(
                        &submit_order_list.order_list.client_order_ids,
                        &submit_order_list,
                    );
                    self.deny_order_list(&orders, "交易状态：停牌/停盘 (TradingState::HALTED)");
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
                                    "交易状态为减仓 (REDUCING) 且处于多头头寸时的买入动作 {}",
                                    instrument.id()
                                ),
                            );
                        } else if order.is_sell() && self.portfolio.is_net_short(&instrument.id()) {
                            self.deny_order(
                                order,
                                &format!(
                                    "交易状态为减仓 (REDUCING) 且处于空头头寸时的卖出动作 {}",
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
                                    "交易状态为减仓 (REDUCING) 且处于多头头寸时的买入动作 {}",
                                    instrument.id()
                                ),
                            );
                            return;
                        } else if order.is_sell() && self.portfolio.is_net_short(&instrument.id()) {
                            self.deny_order_list(
                                &orders,
                                &format!(
                                    "交易状态为减仓 (REDUCING) 且处于空头头寸时的卖出动作 {}",
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
                    self.throttled_submit_order.send(submit_order);
                }
                TradingCommand::SubmitOrderList(submit_order_list) => {
                    // TODO：为订单列表实现流控器
                    self.send_to_execution(TradingCommand::SubmitOrderList(submit_order_list));
                }
                _ => {}
            },
        }
    }

    fn send_to_execution(&self, command: TradingCommand) {
        // 全局交易流控功能已移动到各个独立的流控器中

        let endpoint = MessagingSwitchboard::exec_engine_queue_execute();
        msgbus::send_trading_command(endpoint, command);
    }

    fn handle_event(&mut self, event: &OrderEventAny) {
        // 我们计划扩展风险引擎，使其能够处理额外的事件。
        // 目前我们仅进行日志记录。
        if self.config.debug {
            log::debug!("{RECV}{EVT} {event:?}");
        }
    }

    fn handle_instrument_status(&mut self, status: &InstrumentStatus) {
        log::debug!("正在处理标的状态：{:?}", status);

        use nautilus_model::enums::MarketStatusAction;
        match status.action {
            MarketStatusAction::Halt
            | MarketStatusAction::Suspend
            | MarketStatusAction::NotAvailableForTrading => {
                log::warn!(
                    "标的 {} 已停牌/临停：动作={:?}，原因={:?}",
                    status.instrument_id,
                    status.action,
                    status.reason
                );
            }
            MarketStatusAction::Resumed => {
                log::info!(
                    "标的 {} 已复牌：动作={:?}",
                    status.instrument_id,
                    status.action
                );
            }
            _ => {}
        }
    }

}
