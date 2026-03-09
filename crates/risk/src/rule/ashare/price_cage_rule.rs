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
use crate::price_cage;
use nautilus_common::session::SessionProvider;
use nautilus_core::UnixNanos;
use nautilus_model::enums::OrderSide;
use nautilus_model::identifiers::Venue;
use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use std::sync::Arc;

/// Callback function to get market data for price cage calculation.
pub type MarketDataCallback = dyn Fn(&RuleContext) -> MarketData + Send + Sync;

/// Market data needed for price cage calculation.
#[derive(Debug, Clone, Default)]
pub struct MarketData {
    pub best_bid: Option<Price>,
    pub best_ask: Option<Price>,
    pub last_trade: Option<Price>,
    pub prev_close: Price,
    pub price_increment: Price,
}

/// Rule for validating price cage limits.
///
/// This rule ensures that order prices stay within price cage bounds during continuous trading.
pub struct PriceCageRule {
    session_provider: Arc<dyn SessionProvider>,
    market_data_callback: Arc<MarketDataCallback>,
    price_cage_pct: f64,
    enabled: bool,
}

impl std::fmt::Debug for PriceCageRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriceCageRule")
            .field("price_cage_pct", &self.price_cage_pct)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PriceCageRule {
    pub fn new(
        session_provider: Arc<dyn SessionProvider>,
        price_cage_pct: f64,
        market_data_callback: Arc<MarketDataCallback>,
    ) -> Self {
        Self {
            session_provider,
            market_data_callback,
            price_cage_pct,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl Rule for PriceCageRule {
    fn name(&self) -> &str {
        "PriceCageRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if !self.is_enabled() {
            return RuleCheckResult::Pass;
        }

        let order_price = match context.metadata.price {
            Some(price) => price,
            None => return RuleCheckResult::Pass,
        };

        let venue = Venue::from(context.instrument_id.venue.as_str());
        let timestamp_ns = UnixNanos::from(context.timestamp_ns);
        let phase = self
            .session_provider
            .phase_at(&venue, timestamp_ns);

        if !phase.is_continuous() {
            return RuleCheckResult::Pass;
        }

        let market_data = (self.market_data_callback)(context);

        let cage_bound = price_cage::compute_price_cage(
            context.order.order_side(),
            market_data.best_bid,
            market_data.best_ask,
            market_data.last_trade,
            market_data.prev_close,
            market_data.price_increment,
            self.price_cage_pct,
        );

        if let Some(bound) = cage_bound {
            let order_price = Price::new(order_price, 2);
            let violated = match context.order.order_side() {
                OrderSide::Buy => order_price > bound,
                OrderSide::Sell => order_price < bound,
                _ => false,
            };

            if violated {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "PRICE_CAGE_VIOLATION: price={:.2}, cage={:.2}",
                        order_price.as_f64(),
                        bound.as_f64()
                    ),
                };
            }
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_common::session::TradingPhase;
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

    fn create_test_context(price: f64, side: OrderSide) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(side)
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(price, 2))
            .build();

        RuleContext::new(
            order,
            InstrumentId::from("600000.SH"),
            None,
            0,
        )
    }

    #[test]
    fn test_price_cage_rule_pass() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::ContinuousAm,
        });
        let callback = Arc::new(|_ctx: &RuleContext| MarketData {
            best_bid: Some(Price::new(10.0, 2)),
            best_ask: Some(Price::new(10.1, 2)),
            last_trade: Some(Price::new(10.05, 2)),
            prev_close: Price::new(10.0, 2),
            price_increment: Price::new(0.01, 2),
        });
        let rule = PriceCageRule::new(provider, 0.02, callback);
        let context = create_test_context(10.0, OrderSide::Buy);

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_cage_rule_not_continuous() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::Closed,
        });
        let callback = Arc::new(|_ctx: &RuleContext| MarketData::default());
        let rule = PriceCageRule::new(provider, 0.02, callback);
        let context = create_test_context(10.0, OrderSide::Buy);

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_cage_rule_disabled() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::ContinuousAm,
        });
        let callback = Arc::new(|_ctx: &RuleContext| MarketData::default());
        let mut rule = PriceCageRule::new(provider, 0.02, callback);
        rule.set_enabled(false);
        let context = create_test_context(10.0, OrderSide::Buy);

        let result = rule.check(&context);
        assert!(result.is_pass());
    }
}
