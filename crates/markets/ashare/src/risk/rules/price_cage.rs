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
    ) -> Self {
        Self {
            session_provider,
            price_cage_pct,
            provider,
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

        // 只检查限价单
        if !matches!(context.order.order_type(), OrderType::Limit) {
            return RuleCheckResult::Pass;
        }

        let order_price = match context.metadata.price {
            Some(price) => Price::new(price, 2),
            None => return RuleCheckResult::Pass,
        };

        // 获取市场数据
        let market_data = self.provider.price_cage_market_data(context);
        let current_price = match market_data.current_price {
            Some(price) => price,
            None => return RuleCheckResult::Pass,
        };
        let tick_size = market_data.tick_size.unwrap_or_else(|| Price::new(0.01, 2));

        // 检查是否违反价格笼子限制
        if let Some(limit_price) = crate::risk::checks::check_price_cage_violation(
            context.order.order_side() == OrderSide::Buy,
            order_price,
            current_price,
            self.price_cage_pct,
            tick_size,
        ) {
            return RuleCheckResult::Fail {
                reason: format!(
                    "PRICE_CAGE_VIOLATION: price={:.2}, limit={:.2}",
                    order_price.as_f64(),
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
