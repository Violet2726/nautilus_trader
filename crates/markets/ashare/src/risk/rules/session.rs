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

//! A股交易时段规则
//!
//! 提供A股市场的交易时段验证功能。

use crate::market::session::SessionProvider;
use nautilus_risk::rule::common::{Rule, RuleCheckResult, RuleContext};
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;
use std::sync::Arc;

/// 用于验证交易时段阶段的规则。
///
/// 此规则确保仅在允许的交易时段内提交订单，
/// 基于时段提供者提供的阶段信息。
pub struct SessionRule {
    session_provider: Arc<dyn SessionProvider>,
    enabled: bool,
}

impl std::fmt::Debug for SessionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl SessionRule {
    pub fn new(session_provider: Arc<dyn SessionProvider>) -> Self {
        Self {
            session_provider,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl Rule for SessionRule {
    fn name(&self) -> &str {
        "SessionRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        let venue = Venue::from(context.instrument_id.venue.as_str());
        let timestamp_ns = UnixNanos::from(context.timestamp_ns);
        let phase = self.session_provider.phase_at(&venue, timestamp_ns);

        if !phase.can_accept_order() {
            return RuleCheckResult::Fail {
                reason: format!("OUT_OF_SESSION: phase={phase:?}"),
            };
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::session::TradingPhase;
    use nautilus_common::{Rule, RuleCheckResult, RuleContext};
    use nautilus_core::UnixNanos;
    use nautilus_model::enums::{OrderSide, OrderType};
    use nautilus_model::identifiers::{ClientOrderId, InstrumentId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::{Price, Quantity};

    struct MockSessionProvider {
        phase: TradingPhase,
    }

    impl SessionProvider for MockSessionProvider {
        fn phase_at(&self, _venue: &Venue, _timestamp_ns: UnixNanos) -> TradingPhase {
            self.phase
        }
    }

    fn create_test_context(side: OrderSide) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(side)
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.0, 2))
            .build();

        RuleContext::new(
            order,
            InstrumentId::from("600000.SH"),
            None,
            0,
        )
    }

    #[test]
    fn test_session_rule_pass() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::ContinuousAm,
        });
        let rule = SessionRule::new(provider);
        let context = create_test_context(OrderSide::Buy);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_session_rule_fail_out_of_session() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::Closed,
        });
        let rule = SessionRule::new(provider);
        let context = create_test_context(OrderSide::Buy);
        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("OUT_OF_SESSION"));
    }

    #[test]
    fn test_session_rule_disabled() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::Closed,
        });
        let mut rule = SessionRule::new(provider);
        rule.set_enabled(false);
        let context = create_test_context(OrderSide::Buy);
        let result = rule.check(&context);
        assert!(result.is_pass());
    }
}
