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

use nautilus_model::identifiers::InstrumentId;
use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use std::sync::Arc;

pub type TickSizeCallback = dyn Fn(&InstrumentId) -> Option<Price> + Send + Sync;

#[derive(Clone)]
pub struct PriceTickRule {
    tick_size_callback: Arc<TickSizeCallback>,
    enabled: bool,
}

impl std::fmt::Debug for PriceTickRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriceTickRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PriceTickRule {
    pub fn new(tick_size_callback: Arc<TickSizeCallback>) -> Self {
        Self {
            tick_size_callback,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl Rule for PriceTickRule {
    fn name(&self) -> &str {
        "PriceTickRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        let order_price: Price = match context.order.price() {
            Some(px) => px,
            None => return RuleCheckResult::Pass,
        };

        let tick = match (self.tick_size_callback)(&context.instrument_id) {
            Some(tick) => tick,
            None => return RuleCheckResult::Pass,
        };

        if !order_price.is_on_tick(tick) {
            return RuleCheckResult::Fail {
                reason: format!("PRICE_NOT_ON_TICK: price={order_price}, tick={tick}"),
            };
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::enums::OrderType;
    use nautilus_model::identifiers::{ClientOrderId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::Quantity;

    #[test]
    fn test_price_tick_rule_fail() {
        let rule = PriceTickRule::new(Arc::new(|_| Some(Price::new(0.01, 2))));
        let instrument_id = InstrumentId::from("600000.SH");
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(instrument_id)
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.005, 3))
            .build();

        let ctx = RuleContext::new(order, instrument_id, None, 0);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("PRICE_NOT_ON_TICK"));
    }
}
