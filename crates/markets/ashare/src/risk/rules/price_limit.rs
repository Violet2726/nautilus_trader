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

//! A股涨跌停价格限制规则
//!
//! 提供A股市场的涨跌停价格验证功能。

use nautilus_model::enums::OrderType;
use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use std::sync::Arc;

use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};
use crate::risk::config::MissingMarketDataPolicy;
use crate::risk::provider::MarketDataProvider;

/// 计算涨跌停限制所需的市场数据
#[derive(Debug, Clone, Default)]
pub struct PriceLimitMarketData {
    /// 前收盘价
    pub prev_close: Option<Price>,
    /// 最小价格变动单位
    pub tick_size: Option<Price>,
    /// 股票名称（用于识别ST状态）
    pub stock_name: Option<String>,
}

/// 用于验证涨跌停价格限制的规则。
///
/// 此规则确保订单价格不超过涨跌停限制：
/// - 主板：±10%（ST股票：±5%）
/// - 创业板：±20%
/// - 科创板：±20%（上市前5日无限制）
/// - 北交所：±30%
pub struct PriceLimitRule {
    provider: Arc<dyn MarketDataProvider>,
    missing_market_data_policy: MissingMarketDataPolicy,
    enabled: bool,
}

impl std::fmt::Debug for PriceLimitRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriceLimitRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PriceLimitRule {
    pub fn new(
        provider: Arc<dyn MarketDataProvider>,
        missing_market_data_policy: MissingMarketDataPolicy,
    ) -> Self {
        Self {
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
            "ASHARE_PRICE_LIMIT_MISSING_DATA: code={code}, detail={detail}, policy={:?}",
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

impl Rule for PriceLimitRule {
    fn name(&self) -> &str {
        "PriceLimitRule"
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
                    "ASHARE_PRICE_LIMIT_MISSING_ORDER_PRICE",
                    "limit_order_has_no_price",
                );
            }
        };

        // 获取市场数据
        let market_data = self.provider.price_limit_market_data(context);

        let prev_close = match market_data.prev_close {
            Some(price) => price,
            None => {
                return self.handle_missing_data(
                    "ASHARE_PRICE_LIMIT_MISSING_PREV_CLOSE",
                    "provider.prev_close_is_none",
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
        let stock_name = market_data.stock_name.unwrap_or_default();

        // 计算涨跌停价格
        let limits = crate::market::price_limits::compute_price_limits_for_stock(
            context.instrument_id.symbol.as_str(),
            &stock_name,
            prev_close,
            tick_size,
        );

        // 检查是否违反涨跌停限制
        if let Some(msg) = crate::risk::checks::check_ashare_price_limit_violation(
            normalized_order_price,
            Some(tick_size),
            limits.limit_up,
            limits.limit_down,
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

    struct MockProvider {
        data: PriceLimitMarketData,
        precision: Option<u8>,
        tick: Option<Price>,
    }

    impl MarketDataProvider for MockProvider {
        fn price_limit_market_data(&self, _context: &RuleContext) -> PriceLimitMarketData {
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
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(price, 2))
            .build();
        RuleContext::new(order, instrument_id, None, 0)
    }

    #[test]
    fn test_price_limit_rule_pass_within_limit() {
        let provider = Arc::new(MockProvider {
            data: PriceLimitMarketData {
                prev_close: Some(Price::new(10.0, 2)),
                tick_size: Some(Price::new(0.01, 2)),
                stock_name: Some("正常股票".to_string()),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceLimitRule::new(provider, MissingMarketDataPolicy::FailClose);
        let ctx = make_context("600000.SH", 10.50);
        assert!(rule.check(&ctx).is_pass());
    }

    #[test]
    fn test_price_limit_rule_fail_above_limit_up() {
        let provider = Arc::new(MockProvider {
            data: PriceLimitMarketData {
                prev_close: Some(Price::new(10.0, 2)),
                tick_size: Some(Price::new(0.01, 2)),
                stock_name: Some("正常股票".to_string()),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceLimitRule::new(provider, MissingMarketDataPolicy::FailClose);
        let ctx = make_context("600000.SH", 11.01);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("PRICE_ABOVE_UP_LIMIT"));
    }

    #[test]
    fn test_price_limit_rule_fail_below_limit_down() {
        let provider = Arc::new(MockProvider {
            data: PriceLimitMarketData {
                prev_close: Some(Price::new(10.0, 2)),
                tick_size: Some(Price::new(0.01, 2)),
                stock_name: Some("正常股票".to_string()),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceLimitRule::new(provider, MissingMarketDataPolicy::FailClose);
        let ctx = make_context("600000.SH", 8.99);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("PRICE_BELOW_DOWN_LIMIT"));
    }

    #[test]
    fn test_price_limit_rule_missing_prev_close_fail_close() {
        let provider = Arc::new(MockProvider {
            data: PriceLimitMarketData {
                prev_close: None,
                tick_size: Some(Price::new(0.01, 2)),
                stock_name: Some("正常股票".to_string()),
            },
            precision: Some(2),
            tick: Some(Price::new(0.01, 2)),
        });
        let rule = PriceLimitRule::new(provider, MissingMarketDataPolicy::FailClose);
        let ctx = make_context("600000.SH", 10.00);
        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("ASHARE_PRICE_LIMIT_MISSING_PREV_CLOSE"));
    }
}
