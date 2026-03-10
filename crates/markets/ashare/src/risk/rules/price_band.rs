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

use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use crate::risk::provider::MarketDataProvider;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default)]
pub struct PriceBand {
    pub min_price: Option<Price>,
    pub max_price: Option<Price>,
}

#[derive(Clone)]
pub struct PriceBandRule {
    provider: Arc<dyn MarketDataProvider>,
    enabled: bool,
}

impl std::fmt::Debug for PriceBandRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriceBandRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PriceBandRule {
    pub fn new(provider: Arc<dyn MarketDataProvider>) -> Self {
        Self {
            provider,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl Rule for PriceBandRule {
    fn name(&self) -> &str {
        "PriceBandRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        let order_price = match context.order.price() {
            Some(px) => px,
            None => return RuleCheckResult::Pass,
        };

        let band = self.provider.price_band(&context.instrument_id);

        if let Some(msg) = crate::risk::checks::check_ashare_price_limit_violation(
            order_price,
            None,
            band.max_price,
            band.min_price,
        ) {
            return RuleCheckResult::Fail { reason: msg };
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
    use nautilus_model::types::Quantity;

    #[test]
    fn test_price_band_rule_fail_above() {
        struct MockProvider {
            band: PriceBand,
        }

        impl MarketDataProvider for MockProvider {
            fn price_band(&self, _instrument_id: &InstrumentId) -> PriceBand {
                self.band
            }
        }

        let instrument_id = InstrumentId::from("600000.SH");
        let provider = Arc::new(MockProvider {
            band: PriceBand {
                min_price: Some(Price::new(9.0, 2)),
                max_price: Some(Price::new(10.0, 2)),
            },
        });
        let rule = PriceBandRule::new(provider);

        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(instrument_id)
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.01, 2))
            .build();

        let ctx = RuleContext::new(order, instrument_id, None, 0);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("PRICE_ABOVE_UP_LIMIT"));
    }
}
