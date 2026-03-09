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

    fn is_star_market(symbol: &str) -> bool {
        symbol.starts_with("688")
    }

    fn is_chinext(symbol: &str) -> bool {
        symbol.starts_with("30")
    }

    fn is_main_board(symbol: &str) -> bool {
        symbol.starts_with("60") || symbol.starts_with("00") || symbol.starts_with("00")
    }

    fn check_buy_lot_size(&self, symbol: &str, qty_raw: i64) -> RuleCheckResult {
        let qty_f64 = qty_raw as f64 / 1_000_000_000.0;
        if Self::is_star_market(symbol) {
            if qty_f64 < 200.0 {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "STAR_MARKET_BUY_VIOLATION: buy_qty={} less than min_qty=200",
                        qty_f64
                    ),
                };
            }
        } else if Self::is_chinext(symbol) {
            if qty_f64 < 100.0 {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "CHINEXT_BUY_VIOLATION: buy_qty={} less than min_qty=100",
                        qty_f64
                    ),
                };
            }
        } else if Self::is_main_board(symbol) {
            if qty_f64 < 100.0 || (qty_raw % 100_000_000_000) != 0 {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "MAIN_BOARD_BUY_VIOLATION: buy_qty={} not multiple of 100",
                        qty_f64
                    ),
                };
            }
        } else {
            if qty_f64 < 100.0 || (qty_raw % 100_000_000_000) != 0 {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "LOT_SIZE_VIOLATION: buy_qty={} not multiple of 100",
                        qty_f64
                    ),
                };
            }
        }
        RuleCheckResult::Pass
    }

    fn check_sell_lot_size(
        &self,
        symbol: &str,
        qty_raw: i64,
        order_qty: f64,
        context: &RuleContext,
    ) -> RuleCheckResult {
        let is_star_market = Self::is_star_market(symbol);
        let is_chinext = Self::is_chinext(symbol);

        let min_sell = if is_star_market { 200 } else { 100 };

        let is_odd_lot = if is_star_market || is_chinext {
            qty_raw < min_sell
        } else {
            (qty_raw % 100_000_000_000) != 0
        };

        if is_odd_lot {
            if let Some(ledger) = &self.t1_ledger {
                if let Some(account_id) = &context.account_id {
                    let sellable = ledger.sellable(account_id, &context.instrument_id);
                    if (order_qty - sellable).abs() > f64::EPSILON {
                        return RuleCheckResult::Fail {
                            reason: format!(
                                "ODD_LOT_VIOLATION: sell_qty={} must be full sellable={}",
                                order_qty, sellable
                            ),
                        };
                    }
                }
            }
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

        let qty_raw = context.order.quantity().raw;
        let symbol = context.instrument_id.symbol.as_str();

        match context.order.order_side() {
            OrderSide::Buy => self.check_buy_lot_size(symbol, qty_raw as i64),
            OrderSide::Sell => self.check_sell_lot_size(symbol, qty_raw as i64, order_qty, context),
            _ => RuleCheckResult::Pass,
        }
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
