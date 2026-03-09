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

//! A股风险规则引擎
//!
//! 提供A股市场的风险管理规则链构建功能，包括：
//! - 交易时段规则
//! - 价格笼子规则
//! - 涨跌停限制规则
//! - 手数规则
//! - T+1规则
//! - 限流规则

use nautilus_common::throttler::RateLimit;
use nautilus_risk::rule::common::{Rule, RuleChain, RuleContext};
use super::rules::*;
use super::config::AShareRuleConfig;
use crate::portfolio::T1Ledger;
use std::sync::Arc;

/// 根据给定配置创建包含所有A股规则的策略链。
///
/// # 参数
///
/// * `config` - A股规则配置，指定要启用的规则。
/// * `market_data_callback` - 可选的回调函数，用于获取价格笼子验证所需的市场数据。
/// * `price_limit_callback` - 可选的回调函数，用于获取涨跌停验证所需的市场数据。
///
/// # 返回值
///
/// 包含所有已启用的A股规则的 `RuleChain`，顺序如下：
/// 1. 交易时段规则（如果启用）
/// 2. 价格笼子规则（如果启用且提供了市场数据回调）
/// 3. 涨跌停限制规则（如果启用且提供了市场数据回调）
/// 4. 手数规则（如果启用）
/// 5. T+1 规则（如果启用）
pub fn create_ashare_rule_chain(
    config: &AShareRuleConfig,
    market_data_callback: Option<Arc<dyn Fn(&RuleContext) -> price_cage::MarketData + Send + Sync>>,
    price_limit_callback: Option<Arc<dyn Fn(&RuleContext) -> price_limit::PriceLimitMarketData + Send + Sync>>,
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
        chain.add_rule(Arc::new(lot_size::LotSizeRule::new()));
    }

    if config.t1_enabled {
        if let Some(t1_ledger) = &config.t1_ledger {
            let rw_ledger = Arc::new(std::sync::RwLock::new((**t1_ledger).clone()));
            chain.add_rule(Arc::new(T1Rule::new(rw_ledger)));
        }
    }

    chain
}

/// 根据给定配置和共享账本创建包含所有A股规则的策略链。
///
/// # 参数
///
/// * `config` - A股规则配置，指定要启用的规则。
/// * `t1_ledger` - 可选的共享T+1账本，用于跟踪可卖出持仓。
/// * `market_data_callback` - 可选的回调函数，用于获取价格笼子验证所需的市场数据。
/// * `price_limit_callback` - 可选的回调函数，用于获取涨跌停验证所需的市场数据。
///
/// # 返回值
///
/// 包含所有已启用的A股规则的 `RuleChain`，顺序如下：
/// 1. 交易时段规则（如果启用）
/// 2. 价格笼子规则（如果启用且提供了市场数据回调）
/// 3. 涨跌停限制规则（如果启用且提供了市场数据回调）
/// 4. 手数规则（如果启用）
/// 5. T+1 规则（如果启用且提供了账本）
pub fn create_ashare_rule_chain_with_ledger(
    config: &AShareRuleConfig,
    t1_ledger: Option<Arc<std::sync::RwLock<T1Ledger>>>,
    market_data_callback: Option<Arc<dyn Fn(&RuleContext) -> price_cage::MarketData + Send + Sync>>,
    price_limit_callback: Option<Arc<dyn Fn(&RuleContext) -> price_limit::PriceLimitMarketData + Send + Sync>>,
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
        chain.add_rule(Arc::new(lot_size::LotSizeRule::new()));
    }

    if config.t1_enabled {
        if let Some(ledger) = &t1_ledger {
            chain.add_rule(Arc::new(T1Rule::new(ledger.clone())));
        }
    }

    chain
}

/// 为A股交易创建限流规则。
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
    max_order_submit_per_account: Option<RateLimit>,
    max_order_submit_per_symbol: Option<RateLimit>,
) -> Option<Arc<dyn Rule>> {
    if max_order_submit_per_account.is_none() && max_order_submit_per_symbol.is_none() {
        return None;
    }

    Some(Arc::new(ThrottlerRule::new(
        max_order_submit_per_account,
        max_order_submit_per_symbol,
    )))
}
