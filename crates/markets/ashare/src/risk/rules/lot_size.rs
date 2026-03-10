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

//! A股手数规则
//!
//! 提供A股市场的手数验证功能。

use crate::portfolio::T1Ledger;
use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use nautilus_model::enums::OrderSide;
use nautilus_model::orders::Order;

/// 用于验证A股手数要求的规则。
///
/// 此规则确保订单符合A股手数规则：
/// - 主板：100股起，100的整数倍
/// - 科创板（688）：200股起，1股递增
/// - 创业板（30）：100股起，1股递增
/// - 零股卖出：必须卖出全部可卖余额
#[derive(Debug)]
pub struct LotSizeRule {
    t1_ledger: Option<T1Ledger>,
    enabled: bool,
}

impl LotSizeRule {
    pub fn new() -> Self {
        Self {
            t1_ledger: None,
            enabled: true,
        }
    }

    pub fn with_t1_ledger(mut self, ledger: T1Ledger) -> Self {
        self.t1_ledger = Some(ledger);
        self
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn check_lot_size(
        &self,
        symbol: &str,
        is_buy: bool,
        qty: f64,
        context: &RuleContext,
    ) -> RuleCheckResult {
        let mut sellable = None;
        if !is_buy {
            if let Some(ledger) = &self.t1_ledger {
                if let Some(account_id) = &context.account_id {
                    sellable = Some(ledger.sellable(account_id, &context.instrument_id));
                }
            }
        }

        if let Some(msg) = crate::risk::checks::check_ashare_lot_size_violation(
            symbol,
            is_buy,
            qty,
            sellable,
        ) {
            return RuleCheckResult::Fail { reason: msg };
        }

        RuleCheckResult::Pass
    }
}

impl Rule for LotSizeRule {
    fn name(&self) -> &str {
        "LotSizeRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        let order_qty = context.metadata.quantity.unwrap_or(0.0);
        if order_qty <= 0.0 {
            return RuleCheckResult::Pass;
        }

        let symbol = context.instrument_id.symbol.as_str();
        let is_buy = context.order.order_side() == OrderSide::Buy;
        self.check_lot_size(symbol, is_buy, order_qty, context)
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
        symbol: &str,
        qty: f64,
        side: OrderSide,
        account_id: Option<AccountId>,
    ) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from(symbol))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(side)
            .quantity(Quantity::new(qty, 0))
            .price(Price::new(10.0, 2))
            .build();

        RuleContext::new(
            order,
            InstrumentId::from(symbol),
            account_id,
            0,
        )
    }

    #[test]
    fn test_lot_size_rule_main_board_buy_pass() {
        let rule = LotSizeRule::new();
        let context = create_test_context("600000.SH", 100.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_lot_size_rule_main_board_buy_fail() {
        let rule = LotSizeRule::new();
        let context = create_test_context("600000.SH", 150.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("MAIN_BOARD_BUY_VIOLATION"));
    }

    #[test]
    fn test_lot_size_rule_star_market_buy_pass() {
        let rule = LotSizeRule::new();
        let context = create_test_context("688001.SH", 200.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_lot_size_rule_star_market_buy_fail() {
        let rule = LotSizeRule::new();
        let context = create_test_context("688001.SH", 199.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("STAR_MARKET_BUY_VIOLATION"));
    }

    #[test]
    fn test_lot_size_rule_chinext_buy_pass() {
        let rule = LotSizeRule::new();
        let context = create_test_context("300001.SZ", 100.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_lot_size_rule_chinext_buy_fail() {
        let rule = LotSizeRule::new();
        let context = create_test_context("300001.SZ", 99.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("CHINEXT_BUY_VIOLATION"));
    }

    #[test]
    fn test_lot_size_rule_sell_pass() {
        let rule = LotSizeRule::new();
        let context = create_test_context("600000.SH", 100.0, OrderSide::Sell, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_lot_size_rule_disabled() {
        let mut rule = LotSizeRule::new();
        rule.set_enabled(false);
        let context = create_test_context("600000.SH", 150.0, OrderSide::Buy, None);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }
}
