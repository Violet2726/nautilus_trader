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

//! 用于异步事件调度的实盘执行事件发射器 (emitter)。
//!
//! 此模块提供了 [`ExecutionEventEmitter`]，它将事件生成（通过 [`OrderEventFactory`]）
//! 与异步调度相结合。适配器可以使用 `emit_*` 便捷方法在一个调用中生成并发送事件。
//!
//! # 架构
//!
//! ```text
//! Adapter (适配器)
//! ├── core: ExecutionClientCore    (身份标识 + 连接状态)
//! └── emitter: ExecutionEventEmitter   (事件生成 + 异步调度)
//!     ├── factory: OrderEventFactory
//!     └── sender: Option<Sender>   (在 start() 中设置)
//! ```

use nautilus_common::{
    factories::OrderEventFactory,
    messages::{ExecutionEvent, ExecutionReport},
};
use nautilus_core::{UUID4, UnixNanos, time::AtomicTime};
use nautilus_model::{
    enums::{AccountType, LiquiditySide},
    events::{
        AccountState, OrderCancelRejected, OrderEventAny, OrderModifyRejected, OrderRejected,
    },
    identifiers::{
        AccountId, ClientOrderId, InstrumentId, PositionId, StrategyId, TradeId, TraderId,
        VenueOrderId,
    },
    orders::OrderAny,
    reports::{FillReport, OrderStatusReport, PositionStatusReport},
    types::{AccountBalance, Currency, MarginBalance, Money, Price, Quantity},
};

/// 实盘交易的事件发射器 —— 将事件生成与异步调度相结合。
///
/// 此结构体封装了用于构造事件的 [`OrderEventFactory`] 以及用于异步调度的无界通道发送器。
/// 它提供了 `emit_*` 便捷方法，可以在单次调用中生成并发送事件。
///
/// 发送器是在适配器的 `start()` 阶段通过 [`set_sender`](Self::set_sender) 设置的。
#[derive(Debug, Clone)]
pub struct ExecutionEventEmitter {
    clock: &'static AtomicTime,
    factory: OrderEventFactory,
    sender: Option<tokio::sync::mpsc::UnboundedSender<ExecutionEvent>>,
}

impl ExecutionEventEmitter {
    /// 创建一个新的 [`ExecutionEventEmitter`]，初始不带发送器。
    ///
    /// 请在适配器的 `start()` 方法中调用 [`set_sender`](Self::set_sender)。
    #[must_use]
    pub fn new(
        clock: &'static AtomicTime,
        trader_id: TraderId,
        account_id: AccountId,
        account_type: AccountType,
        base_currency: Option<Currency>,
    ) -> Self {
        Self {
            clock,
            factory: OrderEventFactory::new(trader_id, account_id, account_type, base_currency),
            sender: None,
        }
    }

    fn ts_init(&self) -> UnixNanos {
        self.clock.get_time_ns()
    }

    /// 设置发送器。在适配器的 `start()` 中调用。
    pub fn set_sender(&mut self, sender: tokio::sync::mpsc::UnboundedSender<ExecutionEvent>) {
        self.sender = Some(sender);
    }

    /// 如果发送器已初始化，则返回 true。
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.sender.is_some()
    }

    /// 返回交易员 ID。
    #[must_use]
    pub fn trader_id(&self) -> TraderId {
        self.factory.trader_id()
    }

    /// 返回账户 ID。
    #[must_use]
    pub fn account_id(&self) -> AccountId {
        self.factory.account_id()
    }

    /// 生成并发出账户状态事件。
    pub fn emit_account_state(
        &self,
        balances: Vec<AccountBalance>,
        margins: Vec<MarginBalance>,
        reported: bool,
        ts_event: UnixNanos,
    ) {
        let state = self.factory.generate_account_state(
            balances,
            margins,
            reported,
            ts_event,
            self.ts_init(),
        );
        self.send_account_state(state);
    }

    /// 生成并发出订单拒绝 (denied) 事件。
    pub fn emit_order_denied(&self, order: &OrderAny, reason: &str) {
        let event = self
            .factory
            .generate_order_denied(order, reason, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单已提交 (submitted) 事件。
    pub fn emit_order_submitted(&self, order: &OrderAny) {
        let event = self.factory.generate_order_submitted(order, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单驳回 (rejected) 事件。
    pub fn emit_order_rejected(
        &self,
        order: &OrderAny,
        reason: &str,
        ts_event: UnixNanos,
        due_post_only: bool,
    ) {
        let event = self.factory.generate_order_rejected(
            order,
            reason,
            ts_event,
            self.ts_init(),
            due_post_only,
        );
        self.send_order_event(event);
    }

    /// 生成并发出订单已受理 (accepted) 事件。
    pub fn emit_order_accepted(
        &self,
        order: &OrderAny,
        venue_order_id: VenueOrderId,
        ts_event: UnixNanos,
    ) {
        let event =
            self.factory
                .generate_order_accepted(order, venue_order_id, ts_event, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单修改驳回 (modify rejected) 事件。
    pub fn emit_order_modify_rejected(
        &self,
        order: &OrderAny,
        venue_order_id: Option<VenueOrderId>,
        reason: &str,
        ts_event: UnixNanos,
    ) {
        let event = self.factory.generate_order_modify_rejected(
            order,
            venue_order_id,
            reason,
            ts_event,
            self.ts_init(),
        );
        self.send_order_event(event);
    }

    /// 生成并发出订单取消驳回 (cancel rejected) 事件。
    pub fn emit_order_cancel_rejected(
        &self,
        order: &OrderAny,
        venue_order_id: Option<VenueOrderId>,
        reason: &str,
        ts_event: UnixNanos,
    ) {
        let event = self.factory.generate_order_cancel_rejected(
            order,
            venue_order_id,
            reason,
            ts_event,
            self.ts_init(),
        );
        self.send_order_event(event);
    }

    /// 生成并发出订单已更新 (updated) 事件。
    #[allow(clippy::too_many_arguments)]
    pub fn emit_order_updated(
        &self,
        order: &OrderAny,
        venue_order_id: VenueOrderId,
        quantity: Quantity,
        price: Option<Price>,
        trigger_price: Option<Price>,
        protection_price: Option<Price>,
        ts_event: UnixNanos,
    ) {
        let event = self.factory.generate_order_updated(
            order,
            venue_order_id,
            quantity,
            price,
            trigger_price,
            protection_price,
            ts_event,
            self.ts_init(),
        );
        self.send_order_event(event);
    }

    /// 生成并发出订单已取消 (canceled) 事件。
    pub fn emit_order_canceled(
        &self,
        order: &OrderAny,
        venue_order_id: Option<VenueOrderId>,
        ts_event: UnixNanos,
    ) {
        let event =
            self.factory
                .generate_order_canceled(order, venue_order_id, ts_event, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单已触发 (triggered) 事件。
    pub fn emit_order_triggered(
        &self,
        order: &OrderAny,
        venue_order_id: Option<VenueOrderId>,
        ts_event: UnixNanos,
    ) {
        let event =
            self.factory
                .generate_order_triggered(order, venue_order_id, ts_event, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单已过期 (expired) 事件。
    pub fn emit_order_expired(
        &self,
        order: &OrderAny,
        venue_order_id: Option<VenueOrderId>,
        ts_event: UnixNanos,
    ) {
        let event =
            self.factory
                .generate_order_expired(order, venue_order_id, ts_event, self.ts_init());
        self.send_order_event(event);
    }

    /// 生成并发出订单已成交 (filled) 事件。
    #[allow(clippy::too_many_arguments)]
    pub fn emit_order_filled(
        &self,
        order: &OrderAny,
        venue_order_id: VenueOrderId,
        venue_position_id: Option<PositionId>,
        trade_id: TradeId,
        last_qty: Quantity,
        last_px: Price,
        quote_currency: Currency,
        commission: Option<Money>,
        liquidity_side: LiquiditySide,
        ts_event: UnixNanos,
    ) {
        let event = self.factory.generate_order_filled(
            order,
            venue_order_id,
            venue_position_id,
            trade_id,
            last_qty,
            last_px,
            quote_currency,
            commission,
            liquidity_side,
            ts_event,
            self.ts_init(),
        );
        self.send_order_event(event);
    }

    /// 从原始字段构造并发出订单驳回事件。
    #[allow(clippy::too_many_arguments)]
    pub fn emit_order_rejected_event(
        &self,
        strategy_id: StrategyId,
        instrument_id: InstrumentId,
        client_order_id: ClientOrderId,
        reason: &str,
        ts_event: UnixNanos,
        due_post_only: bool,
    ) {
        let event = OrderRejected::new(
            self.factory.trader_id(),
            strategy_id,
            instrument_id,
            client_order_id,
            self.factory.account_id(),
            reason.into(),
            UUID4::new(),
            ts_event,
            self.ts_init(),
            false,
            due_post_only,
        );
        self.send_order_event(OrderEventAny::Rejected(event));
    }

    /// 从原始字段构造并发出订单修改驳回事件。
    #[allow(clippy::too_many_arguments)]
    pub fn emit_order_modify_rejected_event(
        &self,
        strategy_id: StrategyId,
        instrument_id: InstrumentId,
        client_order_id: ClientOrderId,
        venue_order_id: Option<VenueOrderId>,
        reason: &str,
        ts_event: UnixNanos,
    ) {
        let event = OrderModifyRejected::new(
            self.factory.trader_id(),
            strategy_id,
            instrument_id,
            client_order_id,
            reason.into(),
            UUID4::new(),
            ts_event,
            self.ts_init(),
            false,
            venue_order_id,
            Some(self.factory.account_id()),
        );
        self.send_order_event(OrderEventAny::ModifyRejected(event));
    }

    /// 从原始字段构造并发出订单取消驳回事件。
    #[allow(clippy::too_many_arguments)]
    pub fn emit_order_cancel_rejected_event(
        &self,
        strategy_id: StrategyId,
        instrument_id: InstrumentId,
        client_order_id: ClientOrderId,
        venue_order_id: Option<VenueOrderId>,
        reason: &str,
        ts_event: UnixNanos,
    ) {
        let event = OrderCancelRejected::new(
            self.factory.trader_id(),
            strategy_id,
            instrument_id,
            client_order_id,
            reason.into(),
            UUID4::new(),
            ts_event,
            self.ts_init(),
            false,
            venue_order_id,
            Some(self.factory.account_id()),
        );
        self.send_order_event(OrderEventAny::CancelRejected(event));
    }

    /// 发出订单事件。
    pub fn send_order_event(&self, event: OrderEventAny) {
        if let Some(sender) = &self.sender {
            if let Err(e) = sender.send(ExecutionEvent::Order(event)) {
                log::warn!("发送订单事件失败：{e}");
            }
        } else {
            log::warn!("无法发送订单事件：发送器未初始化");
        }
    }

    /// 发出账户状态事件。
    pub fn send_account_state(&self, state: AccountState) {
        if let Some(sender) = &self.sender {
            if let Err(e) = sender.send(ExecutionEvent::Account(state)) {
                log::warn!("发送账户状态失败：{e}");
            }
        } else {
            log::warn!("无法发送账户状态：发送器未初始化");
        }
    }

    /// 发出执行报告。
    pub fn send_execution_report(&self, report: ExecutionReport) {
        if let Some(sender) = &self.sender {
            if let Err(e) = sender.send(ExecutionEvent::Report(report)) {
                log::warn!("发送执行报告失败：{e}");
            }
        } else {
            log::warn!("无法发送执行报告：发送器未初始化");
        }
    }

    /// 发出订单状态报告。
    pub fn send_order_status_report(&self, report: OrderStatusReport) {
        self.send_execution_report(ExecutionReport::Order(Box::new(report)));
    }

    /// 发出成交报告。
    pub fn send_fill_report(&self, report: FillReport) {
        self.send_execution_report(ExecutionReport::Fill(Box::new(report)));
    }

    /// 发出持仓状态报告。
    pub fn send_position_report(&self, report: PositionStatusReport) {
        self.send_execution_report(ExecutionReport::Position(Box::new(report)));
    }
}
