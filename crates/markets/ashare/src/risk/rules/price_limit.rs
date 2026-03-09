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

use nautilus_model::enums::{OrderSide, OrderType};
use nautilus_model::orders::Order;
use nautilus_model::types::Price;
use std::sync::Arc;

use nautilus_rules::common::{Rule, RuleCheckResult, RuleContext};

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
    #[allow(dead_code)]
    fn check_price_limit_violation(
        &self,
        order_price: Price,
        limits: &crate::market::price_limits::PriceLimits,
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
        let limits = crate::market::price_limits::compute_price_limits_for_stock(
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
