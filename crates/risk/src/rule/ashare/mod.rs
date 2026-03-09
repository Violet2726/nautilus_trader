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

//! A 股（中国股市）特定的交易规则。
//!
//! 本模块提供中国 A 股市场订单验证规则，包括：
//! - 交易时段限制
//! - 价格笼子限制
//! - 涨跌停价格限制
//! - T+1 交收规则
//! - 手数要求
//! - 限流控制

pub mod config;
pub mod lot_size_rule;
pub mod price_cage_rule;
pub mod price_limit_rule;
pub mod session_rule;
pub mod t1_rule;
pub mod throttler_rule;

use super::common::{Rule, RuleChain, RuleContext};
use config::AShareRuleConfig;
use lot_size_rule::LotSizeRule;
use nautilus_portfolio::T1Ledger;
use price_cage_rule::{MarketData, PriceCageRule};
use price_limit_rule::{PriceLimitMarketData, PriceLimitRule};
use session_rule::SessionRule;
use std::sync::Arc;
use t1_rule::T1Rule;
use throttler_rule::ThrottlerRule;

/// 根据给定配置创建包含所有 A 股规则的策略链。
///
/// # 参数
///
/// * `config` - A 股规则配置，指定要启用的规则。
/// * `market_data_callback` - 可选的回调函数，用于获取价格笼子验证所需的市场数据。
/// * `price_limit_callback` - 可选的回调函数，用于获取涨跌停验证所需的市场数据。
///
/// # 返回值
///
/// 包含所有已启用的 A 股规则的 `RuleChain`，顺序如下：
/// 1. 交易时段规则（如果启用）
/// 2. 价格笼子规则（如果启用且提供了市场数据回调）
/// 3. 涨跌停限制规则（如果启用且提供了市场数据回调）
/// 4. 手数规则（如果启用）
/// 5. T+1 规则（如果启用）
pub fn create_ashare_rule_chain(
    config: &AShareRuleConfig,
    market_data_callback: Option<Arc<dyn Fn(&RuleContext) -> MarketData + Send + Sync>>,
    price_limit_callback: Option<Arc<dyn Fn(&RuleContext) -> PriceLimitMarketData + Send + Sync>>,
) -> RuleChain {
    let mut chain = RuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(SessionRule::new(session_provider.clone())));
        }
    }

    if config.price_cage_enabled {
        if let (Some(session_provider), Some(callback)) = (&config.session_provider, &market_data_callback) {
            chain.add_rule(Arc::new(PriceCageRule::new(
                session_provider.clone(),
                config.price_cage_pct,
                callback.clone(),
            )));
        }
    }

    if config.price_limit_enabled {
        if let Some(callback) = &price_limit_callback {
            chain.add_rule(Arc::new(PriceLimitRule::new(callback.clone())));
        }
    }

    if config.lot_size_enabled {
        chain.add_rule(Arc::new(LotSizeRule::new()));
    }

    if config.t1_enabled {
        if let Some(t1_ledger) = &config.t1_ledger {
            let rw_ledger = Arc::new(std::sync::RwLock::new((**t1_ledger).clone()));
            chain.add_rule(Arc::new(T1Rule::new(rw_ledger)));
        }
    }

    chain
}

/// 根据给定配置和共享账本创建包含所有 A 股规则的策略链。
///
/// # 参数
///
/// * `config` - A 股规则配置，指定要启用的规则。
/// * `t1_ledger` - 可选的共享 T+1 账本，用于跟踪可卖出持仓。
/// * `market_data_callback` - 可选的回调函数，用于获取价格笼子验证所需的市场数据。
/// * `price_limit_callback` - 可选的回调函数，用于获取涨跌停验证所需的市场数据。
///
/// # 返回值
///
/// 包含所有已启用的 A 股规则的 `RuleChain`，顺序如下：
/// 1. 交易时段规则（如果启用）
/// 2. 价格笼子规则（如果启用且提供了市场数据回调）
/// 3. 涨跌停限制规则（如果启用且提供了市场数据回调）
/// 4. 手数规则（如果启用）
/// 5. T+1 规则（如果启用且提供了账本）
pub fn create_ashare_rule_chain_with_ledger(
    config: &AShareRuleConfig,
    t1_ledger: Option<Arc<std::sync::RwLock<T1Ledger>>>,
    market_data_callback: Option<Arc<dyn Fn(&RuleContext) -> MarketData + Send + Sync>>,
    price_limit_callback: Option<Arc<dyn Fn(&RuleContext) -> PriceLimitMarketData + Send + Sync>>,
) -> RuleChain {
    let mut chain = RuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(SessionRule::new(session_provider.clone())));
        }
    }

    if config.price_cage_enabled {
        if let (Some(session_provider), Some(callback)) = (&config.session_provider, &market_data_callback) {
            chain.add_rule(Arc::new(PriceCageRule::new(
                session_provider.clone(),
                config.price_cage_pct,
                callback.clone(),
            )));
        }
    }

    if config.price_limit_enabled {
        if let Some(callback) = &price_limit_callback {
            chain.add_rule(Arc::new(PriceLimitRule::new(callback.clone())));
        }
    }

    if config.lot_size_enabled {
        chain.add_rule(Arc::new(LotSizeRule::new()));
    }

    if config.t1_enabled {
        if let Some(ledger) = &t1_ledger {
            chain.add_rule(Arc::new(T1Rule::new(ledger.clone())));
        }
    }

    chain
}

/// 为 A 股交易创建限流规则。
///
/// # 参数
///
/// * `max_order_submit_per_account` - 可选的每个账户订单提交速率限制。
/// * `max_order_submit_per_symbol` - 可选的每个标的订单提交速率限制。
///
/// # 返回值
///
/// 如果提供了至少一个速率限制，则返回 `Some(Arc<dyn Rule>)`，
/// 否则返回 `None`。
///
pub fn create_throttler_rule(
    max_order_submit_per_account: Option<nautilus_common::throttler::RateLimit>,
    max_order_submit_per_symbol: Option<nautilus_common::throttler::RateLimit>,
) -> Option<Arc<dyn Rule>> {
    if max_order_submit_per_account.is_none() && max_order_submit_per_symbol.is_none() {
        return None;
    }

    Some(Arc::new(ThrottlerRule::new(
        max_order_submit_per_account,
        max_order_submit_per_symbol,
    )))
}
