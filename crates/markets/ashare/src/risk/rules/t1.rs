// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! A股T+1规则
//!
//! 提供A股市场的T+1交收验证功能。

use crate::portfolio::T1Ledger;
use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use nautilus_model::enums::OrderSide;
use nautilus_model::orders::Order;
use std::sync::Arc;

/// 用于验证T+1交收限制的规则。
///
/// 此规则确保卖出订单不超过T+1交收规则下的可卖出数量。
#[derive(Debug)]
pub struct T1Rule {
    t1_ledger: Arc<std::sync::RwLock<T1Ledger>>,
    enabled: bool,
}

impl T1Rule {
    pub fn new(t1_ledger: Arc<std::sync::RwLock<T1Ledger>>) -> Self {
        Self {
            t1_ledger,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn ledger(&self) -> &std::sync::RwLock<T1Ledger> {
        &self.t1_ledger
    }

    pub fn on_order_accepted(&self, context: &RuleContext) {
        if context.order.order_side() != OrderSide::Sell {
            return;
        }

        let account_id = match &context.account_id {
            Some(id) => id,
            None => return,
        };

        let order_qty = context.metadata.quantity.unwrap_or(0.0);
        let mut ledger = self.t1_ledger.write().expect("T1 ledger lock poisoned");
        ledger.on_fill(
            *account_id,
            context.instrument_id,
            OrderSide::Sell,
            order_qty,
        );
    }
}

impl Rule for T1Rule {
    fn name(&self) -> &str {
        "T1Rule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        if context.order.order_side() != OrderSide::Sell {
            return RuleCheckResult::Pass;
        }

        let account_id = match &context.account_id {
            Some(id) => id,
            None => return RuleCheckResult::Pass,
        };

        let sellable = self.t1_ledger.read().expect("T1 ledger lock poisoned").sellable(account_id, &context.instrument_id);
        let order_qty = context.metadata.quantity.unwrap_or(0.0);

        if order_qty > sellable {
            return RuleCheckResult::Fail {
                reason: format!(
                    "EXCEEDS_SELLABLE: qty={:.0}, sellable={:.0}",
                    order_qty, sellable
                ),
            };
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::enums::{OrderSide, OrderType};
    use nautilus_model::identifiers::{AccountId, ClientOrderId, InstrumentId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::{Price, Quantity};

    fn create_test_context(
        qty: f64,
        side: OrderSide,
        account_id: Option<AccountId>,
    ) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(side)
            .quantity(Quantity::new(qty, 0))
            .price(Price::new(10.0, 2))
            .build();

        RuleContext::new(
            order,
            InstrumentId::from("600000.SH"),
            account_id,
            0,
        )
    }

    #[test]
    fn test_t1_rule_pass_buy() {
        let ledger = T1Ledger::new();
        let rule = T1Rule::new(Arc::new(std::sync::RwLock::new(ledger)));
        let context = create_test_context(100.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_t1_rule_pass_sell_within_limit() {
        let mut ledger = T1Ledger::new();
        ledger.load_position(
            AccountId::from("ACC-001"),
            InstrumentId::from("600000.SH"),
            1000.0,
            0.0,
        );

        let rule = T1Rule::new(Arc::new(std::sync::RwLock::new(ledger)));
        let context = create_test_context(
            100.0,
            OrderSide::Sell,
            Some(AccountId::from("ACC-001")),
        );
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_t1_rule_fail_sell_exceeds_limit() {
        let mut ledger = T1Ledger::new();
        ledger.load_position(
            AccountId::from("ACC-001"),
            InstrumentId::from("600000.SH"),
            1000.0,
            500.0,
        );

        let rule = T1Rule::new(Arc::new(std::sync::RwLock::new(ledger)));
        let context = create_test_context(
            600.0,
            OrderSide::Sell,
            Some(AccountId::from("ACC-001")),
        );
        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("EXCEEDS_SELLABLE"));
    }

    #[test]
    fn test_t1_rule_disabled() {
        let mut ledger = T1Ledger::new();
        ledger.load_position(
            AccountId::from("ACC-001"),
            InstrumentId::from("600000.SH"),
            1000.0,
            500.0,
        );
        let mut rule = T1Rule::new(Arc::new(std::sync::RwLock::new(ledger)));
        rule.set_enabled(false);
        let context = create_test_context(
            600.0,
            OrderSide::Sell,
            Some(AccountId::from("ACC-001")),
        );
        let result = rule.check(&context);
        assert!(result.is_pass());
    }
}
