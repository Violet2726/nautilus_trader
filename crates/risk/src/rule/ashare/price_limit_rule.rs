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

use super::super::common::{Rule, RuleCheckResult, RuleContext};
use nautilus_common::session::price_limits::{
    compute_price_limits_for_stock, PriceLimits,
};
use nautilus_model::enums::{OrderSide, OrderType};
use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use std::sync::Arc;

/// 获取市场数据的回调函数，用于计算涨跌停价格
pub type MarketDataCallback = dyn Fn(&RuleContext) -> PriceLimitMarketData + Send + Sync;

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
    market_data_callback: Arc<MarketDataCallback>,
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
    pub fn new(market_data_callback: Arc<MarketDataCallback>) -> Self {
        Self {
            market_data_callback,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 检查订单价格是否违反涨跌停限制
    fn check_price_limit_violation(
        &self,
        order_price: Price,
        limits: &PriceLimits,
        side: OrderSide,
    ) -> bool {
        match side {
            OrderSide::Buy => {
                // 买入价不能超过涨停价
                if let Some(limit_up) = limits.limit_up {
                    order_price > limit_up
                } else {
                    false // 无涨停限制
                }
            }
            OrderSide::Sell => {
                // 卖出价不能低于跌停价
                if let Some(limit_down) = limits.limit_down {
                    order_price < limit_down
                } else {
                    false // 无跌停限制
                }
            }
            _ => false,
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

        let order_price = match context.metadata.price {
            Some(price) => Price::new(price, 2), // 默认精度为2
            None => return RuleCheckResult::Pass,
        };

        // 获取市场数据
        let market_data = (self.market_data_callback)(context);

        let prev_close = match market_data.prev_close {
            Some(price) => price,
            None => return RuleCheckResult::Pass, // 没有前收盘价则跳过检查
        };

        let tick_size = market_data.tick_size.unwrap_or_else(|| Price::new(0.01, 2));
        let stock_name = market_data.stock_name.unwrap_or_default();

        // 计算涨跌停价格
        let limits = compute_price_limits_for_stock(
            context.instrument_id.symbol.as_str(),
            &stock_name,
            prev_close,
            tick_size,
        );

        // 检查是否违反涨跌停限制
        if self.check_price_limit_violation(order_price, &limits, context.order.order_side()) {
            let (limit_type, limit_price) = match context.order.order_side() {
                OrderSide::Buy => ("涨停", limits.limit_up.unwrap_or_default()),
                OrderSide::Sell => ("跌停", limits.limit_down.unwrap_or_default()),
                _ => ("限制", Price::new(0.0, 2)),
            };

            return RuleCheckResult::Fail {
                reason: format!(
                    "PRICE_LIMIT_VIOLATION: price={:.2}, {}={:.2}",
                    order_price.as_f64(),
                    limit_type,
                    limit_price.as_f64()
                ),
            };
        }

        RuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::enums::{OrderSide, OrderType};
    use nautilus_model::identifiers::{ClientOrderId, InstrumentId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::{Price, Quantity};

    fn create_test_context(
        symbol: &str,
        price: f64,
        side: OrderSide,
        _prev_close: f64,
        _stock_name: &str,
    ) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from(format!("{}.SH", symbol)))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(side)
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(price, 2))
            .build();

        let mut context = RuleContext::new(
            order,
            InstrumentId::from(format!("{}.SH", symbol)),
            None,
            0,
        );
        context.metadata.price = Some(price);
        context
    }

    fn create_market_data_callback(prev_close: f64, stock_name: &str) -> Arc<MarketDataCallback> {
        let stock_name = stock_name.to_string();
        Arc::new(move |_ctx: &RuleContext| PriceLimitMarketData {
            prev_close: Some(Price::new(prev_close, 2)),
            tick_size: Some(Price::new(0.01, 2)),
            stock_name: Some(stock_name.clone()),
        })
    }

    #[test]
    fn test_price_limit_rule_pass_main_board() {
        // 主板股票，前收盘10.00，买入价10.50应该通过（涨停11.00）
        let callback = create_market_data_callback(10.00, "平安银行");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 10.50, OrderSide::Buy, 10.00, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_fail_main_board_buy() {
        // 主板股票，前收盘10.00，买入价11.10应该失败（涨停11.00）
        let callback = create_market_data_callback(10.00, "平安银行");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 11.10, OrderSide::Buy, 10.00, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("PRICE_LIMIT_VIOLATION"));
        assert!(result.to_string().contains("涨停"));
    }

    #[test]
    fn test_price_limit_rule_fail_main_board_sell() {
        // 主板股票，前收盘10.00，卖出价8.90应该失败（跌停9.00）
        let callback = create_market_data_callback(10.00, "平安银行");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 8.90, OrderSide::Sell, 10.00, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("PRICE_LIMIT_VIOLATION"));
        assert!(result.to_string().contains("跌停"));
    }

    #[test]
    fn test_price_limit_rule_st_stock() {
        // ST股票，前收盘10.00，涨停10.50
        let callback = create_market_data_callback(10.00, "ST平安");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 10.60, OrderSide::Buy, 10.00, "ST平安");

        let result = rule.check(&context);
        assert!(result.is_fail());
        assert!(result.to_string().contains("涨停"));
    }

    #[test]
    fn test_price_limit_rule_chinext() {
        // 创业板，前收盘10.00，涨停12.00
        let callback = create_market_data_callback(10.00, "特锐德");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("300001", 11.50, OrderSide::Buy, 10.00, "特锐德");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_star_market() {
        // 科创板，前收盘10.00，涨停12.00
        let callback = create_market_data_callback(10.00, "中芯国际");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("688001", 11.50, OrderSide::Buy, 10.00, "中芯国际");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_bse() {
        // 北交所，前收盘10.00，涨停13.00
        let callback = create_market_data_callback(10.00, "贝特瑞");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("835185", 12.50, OrderSide::Buy, 10.00, "贝特瑞");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_no_market_data() {
        // 没有市场数据时应该通过
        let callback = Arc::new(|_ctx: &RuleContext| PriceLimitMarketData::default());
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 100.00, OrderSide::Buy, 10.00, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_disabled() {
        let callback = create_market_data_callback(10.00, "平安银行");
        let mut rule = PriceLimitRule::new(callback);
        rule.set_enabled(false);
        let context = create_test_context("600000", 100.00, OrderSide::Buy, 10.00, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_non_limit_order() {
        // 市价单应该通过
        let callback = create_market_data_callback(10.00, "平安银行");
        let rule = PriceLimitRule::new(callback);
        
        let order = OrderTestBuilder::new(OrderType::Market)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from("600000.SH"))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(OrderSide::Buy)
            .quantity(Quantity::new(100.0, 0))
            .build();

        let context = RuleContext::new(
            order,
            InstrumentId::from("600000.SH"),
            None,
            0,
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_price_limit_rule_price_alignment() {
        // 测试价格对齐
        let callback = create_market_data_callback(10.13, "平安银行");
        let rule = PriceLimitRule::new(callback);
        let context = create_test_context("600000", 11.15, OrderSide::Buy, 10.13, "平安银行");

        let result = rule.check(&context);
        assert!(result.is_pass()); // 11.15应该等于涨停价
    }
}
