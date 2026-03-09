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

use super::super::common::{Rule, RuleCheckResult, RuleContext};
use ahash::AHashMap;
use nautilus_common::throttler::RateLimit;
use nautilus_model::identifiers::{AccountId, InstrumentId};
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Rule for throttling order submissions.
///
/// This rule enforces rate limits on order submissions at both account and symbol levels.
#[derive(Debug)]
pub struct ThrottlerRule {
    account_limits: RwLock<AHashMap<AccountId, (u64, Instant)>>,
    symbol_limits: RwLock<AHashMap<InstrumentId, (u64, Instant)>>,
    max_order_submit_per_account: Option<(u64, Duration)>,
    max_order_submit_per_symbol: Option<(u64, Duration)>,
    enabled: bool,
}

impl ThrottlerRule {
    pub fn new(
        max_order_submit_per_account: Option<RateLimit>,
        max_order_submit_per_symbol: Option<RateLimit>,
    ) -> Self {
        let account_limits = max_order_submit_per_account.map(|rl| (rl.limit as u64, Duration::from_nanos(rl.interval_ns)));
        let symbol_limits = max_order_submit_per_symbol.map(|rl| (rl.limit as u64, Duration::from_nanos(rl.interval_ns)));

        Self {
            account_limits: RwLock::new(AHashMap::new()),
            symbol_limits: RwLock::new(AHashMap::new()),
            max_order_submit_per_account: account_limits,
            max_order_submit_per_symbol: symbol_limits,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn check_account_limit(&self, account_id: &AccountId) -> RuleCheckResult {
        if let Some((limit, duration)) = &self.max_order_submit_per_account {
            let mut limits = self.account_limits.write().unwrap();
            let (count, last_time) = limits.entry(*account_id).or_insert((0, Instant::now()));

            let now = Instant::now();
            if now.duration_since(*last_time) >= *duration {
                *count = 0;
                *last_time = now;
            }

            if *count >= *limit {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "THROTTLED: Account {} exceeded rate limit",
                        account_id
                    ),
                };
            }

            *count += 1;
        }
        RuleCheckResult::Pass
    }

    fn check_symbol_limit(&self, instrument_id: &InstrumentId) -> RuleCheckResult {
        if let Some((limit, duration)) = &self.max_order_submit_per_symbol {
            let mut limits = self.symbol_limits.write().unwrap();
            let (count, last_time) = limits.entry(*instrument_id).or_insert((0, Instant::now()));

            let now = Instant::now();
            if now.duration_since(*last_time) >= *duration {
                *count = 0;
                *last_time = now;
            }

            if *count >= *limit {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "THROTTLED: Symbol {} exceeded rate limit",
                        instrument_id
                    ),
                };
            }

            *count += 1;
        }
        RuleCheckResult::Pass
    }
}

impl Default for ThrottlerRule {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl Rule for ThrottlerRule {
    fn name(&self) -> &str {
        "ThrottlerRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if self.max_order_submit_per_account.is_none() && self.max_order_submit_per_symbol.is_none() {
            return RuleCheckResult::Pass;
        }

        if let Some(account_id) = &context.account_id {
            let result = self.check_account_limit(account_id);
            if !result.is_pass() {
                return result;
            }
        }

        let result = self.check_symbol_limit(&context.instrument_id);
        if !result.is_pass() {
            return result;
        }

        RuleCheckResult::Pass
    }

    fn reset(&mut self) {
        self.account_limits.write().unwrap().clear();
        self.symbol_limits.write().unwrap().clear();
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
        account_id: Option<AccountId>,
    ) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from(symbol))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(OrderSide::Buy)
            .quantity(Quantity::new(100.0, 0))
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
    fn test_throttler_rule_pass() {
        let rate_limit = RateLimit::new(10, 1_000_000_000);
        let rule = ThrottlerRule::new(Some(rate_limit), None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_no_limits() {
        let rule = ThrottlerRule::new(None, None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_disabled() {
        let rate_limit = RateLimit::new(1, 1_000_000_000);
        let mut rule = ThrottlerRule::new(Some(rate_limit), None);
        rule.set_enabled(false);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_reset() {
        let rate_limit = RateLimit::new(10, 1_000_000_000);
        let mut rule = ThrottlerRule::new(Some(rate_limit), None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        rule.check(&context);
        assert_eq!(rule.account_limits.read().unwrap().len(), 1);

        rule.reset();
        assert_eq!(rule.account_limits.read().unwrap().len(), 0);
    }
}
