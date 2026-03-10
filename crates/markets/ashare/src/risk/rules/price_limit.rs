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
        let market_data = self.provider.price_limit_market_data(context);

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
        if let Some(msg) = crate::risk::checks::check_ashare_price_limit_violation(
            order_price,
            market_data.tick_size,
            limits.limit_up,
            limits.limit_down,
        ) {
            return RuleCheckResult::Fail { reason: msg };
        }

        RuleCheckResult::Pass
    }
}
