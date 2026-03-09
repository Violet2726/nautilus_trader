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

use nautilus_model::enums::MarketStatusAction;
use nautilus_model::identifiers::InstrumentId;
use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use std::sync::Arc;

pub type InstrumentStatusCallback = dyn Fn(&InstrumentId) -> Option<MarketStatusAction> + Send + Sync;

#[derive(Clone)]
pub struct InstrumentStatusRule {
    status_callback: Arc<InstrumentStatusCallback>,
    enabled: bool,
}

impl std::fmt::Debug for InstrumentStatusRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstrumentStatusRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl InstrumentStatusRule {
    pub fn new(status_callback: Arc<InstrumentStatusCallback>) -> Self {
        Self {
            status_callback,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn is_suspended(action: MarketStatusAction) -> bool {
        matches!(
            action,
            MarketStatusAction::Halt
                | MarketStatusAction::Suspend
                | MarketStatusAction::NotAvailableForTrading
        )
    }
}

impl Rule for InstrumentStatusRule {
    fn name(&self) -> &str {
        "InstrumentStatusRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        if let Some(action) = (self.status_callback)(&context.instrument_id) {
            if Self::is_suspended(action) {
                return RuleCheckResult::Fail {
                    reason: format!("INSTRUMENT_SUSPENDED: action={action:?}"),
                };
            }
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::enums::OrderType;
    use nautilus_model::identifiers::{ClientOrderId, InstrumentId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::{Price, Quantity};

    #[test]
    fn test_instrument_status_rule_pass_when_no_status() {
        let rule = InstrumentStatusRule::new(Arc::new(|_| None));
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.0, 2))
            .build();

        let ctx = RuleContext::new(order, InstrumentId::from("600000.SH"), None, 0);
        assert!(rule.check(&ctx).is_pass());
    }

    #[test]
    fn test_instrument_status_rule_fail_when_suspended() {
        let rule = InstrumentStatusRule::new(Arc::new(|_| Some(MarketStatusAction::Suspend)));
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.0, 2))
            .build();

        let ctx = RuleContext::new(order, InstrumentId::from("600000.SH"), None, 0);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("INSTRUMENT_SUSPENDED"));
    }
}
