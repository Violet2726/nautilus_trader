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

//! A股价格笼子规则
//!
//! 提供A股市场的价格笼子验证功能。

use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use nautilus_model::enums::{OrderSide, OrderType};
use nautilus_model::types::Price;
use nautilus_model::orders::Order;
use crate::market::session::SessionProvider;
use crate::risk::config::MissingMarketDataPolicy;
use crate::risk::provider::MarketDataProvider;
use std::sync::Arc;

/// 用于验证价格笼子限制的规则。
///
/// 此规则确保订单价格不超过当前价格的笼子限制。
#[allow(dead_code)]
pub struct PriceCageRule {
    session_provider: Arc<dyn SessionProvider>,
    price_cage_pct: f64,
    provider: Arc<dyn MarketDataProvider>,
    missing_market_data_policy: MissingMarketDataPolicy,
    enabled: bool,
}

impl std::fmt::Debug for PriceCageRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriceCageRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PriceCageRule {
    pub fn new(
        session_provider: Arc<dyn SessionProvider>,
        price_cage_pct: f64,
        provider: Arc<dyn MarketDataProvider>,
        missing_market_data_policy: MissingMarketDataPolicy,
    ) -> Self {
        Self {
            session_provider,
            price_cage_pct,
            provider,
            missing_market_data_policy,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn handle_missing_data(&self, code: &str, detail: &str) -> RuleCheckResult {
        log::warn!(
            "ASHARE_PRICE_CAGE_MISSING_DATA: code={code}, detail={detail}, policy={:?}",
            self.missing_market_data_policy
        );

        match self.missing_market_data_policy {
            MissingMarketDataPolicy::FailOpen => RuleCheckResult::Pass,
            MissingMarketDataPolicy::FailClose => RuleCheckResult::Fail {
                reason: format!("MISSING_MARKET_DATA: code={code}, detail={detail}"),
            },
        }
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

        // 只检查限价单
        if !matches!(context.order.order_type(), OrderType::Limit) {
            return RuleCheckResult::Pass;
        }

        let order_price = match context.order.price() {
            Some(price) => price,
            None => {
                return self.handle_missing_data(
                    "ASHARE_PRICE_CAGE_MISSING_ORDER_PRICE",
                    "limit_order_has_no_price",
                );
            }
        };

        // 获取市场数据
        let market_data = self.provider.price_cage_market_data(context);
        let current_price = match market_data.current_price {
            Some(price) => price,
            None => {
                return self.handle_missing_data(
                    "ASHARE_PRICE_CAGE_MISSING_CURRENT_PRICE",
                    "provider.current_price_is_none",
                );
            }
        };
        let precision = self
            .provider
            .price_precision(&context.instrument_id)
            .unwrap_or(order_price.precision);
        let tick_size = market_data
            .tick_size
            .or_else(|| self.provider.tick_size(&context.instrument_id))
            .unwrap_or_else(|| {
                let step = 10f64.powi(-(precision as i32));
                Price::new(step, precision)
            });
        let normalized_order_price = Price::new(order_price.as_f64(), precision);

        // 检查是否违反价格笼子限制
        if let Some(limit_price) = crate::risk::checks::check_price_cage_violation(
            context.order.order_side() == OrderSide::Buy,
            normalized_order_price,
            current_price,
            self.price_cage_pct,
            tick_size,
        ) {
            return RuleCheckResult::Fail {
                reason: format!(
                    "PRICE_CAGE_VIOLATION: price={:.2}, limit={:.2}",
                    normalized_order_price.as_f64(),
                    limit_price.as_f64()
                ),
            };
        }

        RuleCheckResult::Pass
    }
}

/// 获取市场数据的回调函数，用于价格笼子验证
#[derive(Debug, Clone, Default)]
pub struct MarketData {
    /// 当前价格
    pub current_price: Option<Price>,
    /// 最小价格变动单位
    pub tick_size: Option<Price>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::session::TradingPhase;
    use nautilus_core::UnixNanos;
    use nautilus_model::enums::OrderType;
    use nautilus_model::identifiers::{ClientOrderId, InstrumentId, StrategyId, TraderId, Venue};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::Quantity;

    struct MockSessionProvider;

    impl SessionProvider for MockSessionProvider {
        fn phase_at(&self, _venue: &Venue, _ts_ns: UnixNanos) -> TradingPhase {
            TradingPhase::ContinuousAm
        }
    }

    struct MockProvider {
        data: MarketData,
        precision: Option<u8>,
        tick: Option<Price>,
    }

    impl MarketDataProvider for MockProvider {
        fn price_cage_market_data(&self, _context: &RuleContext) -> MarketData {
            self.data.clone()
        }

        fn price_precision(&self, _instrument_id: &InstrumentId) -> Option<u8> {
            self.precision
        }

        fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
            self.tick
        }
    }

    fn make_context(symbol: &str, price: f64) -> RuleContext {
        let instrument_id = InstrumentId::from(symbol);
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(instrument_id)
            .client_order_id(ClientOrderId::from("O-PRICE-CAGE-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(price, 2))
            .build();

        RuleContext::new(order, instrument_id, None, 0)
    }

    #[test]
    fn test_price_cage_rule_fail_when_above_cage() {
        let session_provider = Arc::new(MockSessionProvider);
        let provider = Arc::new(MockProvider {
            data: MarketData {
                current_price: Some(Price::new(10.0, 2)),
                tick_size: Some(Price::new(0.01, 2)),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceCageRule::new(
            session_provider,
            0.02,
            provider,
            MissingMarketDataPolicy::FailClose,
        );
        let ctx = make_context("600000.SH", 10.31);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("PRICE_CAGE_VIOLATION"));
    }

    #[test]
    fn test_price_cage_rule_pass_when_market_data_missing_fail_open() {
        let session_provider = Arc::new(MockSessionProvider);
        let provider = Arc::new(MockProvider {
            data: MarketData {
                current_price: None,
                tick_size: Some(Price::new(0.01, 2)),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceCageRule::new(
            session_provider,
            0.02,
            provider,
            MissingMarketDataPolicy::FailOpen,
        );
        let ctx = make_context("600000.SH", 10.00);
        assert!(rule.check(&ctx).is_pass());
    }

    #[test]
    fn test_price_cage_rule_fail_when_market_data_missing_fail_close() {
        let session_provider = Arc::new(MockSessionProvider);
        let provider = Arc::new(MockProvider {
            data: MarketData {
                current_price: None,
                tick_size: Some(Price::new(0.01, 2)),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceCageRule::new(
            session_provider,
            0.02,
            provider,
            MissingMarketDataPolicy::FailClose,
        );
        let ctx = make_context("600000.SH", 10.00);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("ASHARE_PRICE_CAGE_MISSING_CURRENT_PRICE"));
    }
}
