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
use nautilus_rules::command::CommandRuleChain;
use nautilus_rules::common::{Rule, RuleChain};
use super::rules::*;
use super::config::AShareRuleConfig;
use super::provider::MarketDataProvider;
use std::sync::Arc;

pub fn create_ashare_rule_chain_with_provider(
    config: &AShareRuleConfig,
    provider: Arc<dyn MarketDataProvider>,
) -> RuleChain {
    create_ashare_rule_chain(config, Some(provider))
}

/// 根据给定配置创建包含所有A股规则的策略链。
///
/// # 参数
///
/// * `config` - A股规则配置，指定要启用的规则。
/// * `provider` - 可选的市场数据提供者，用于获取各种市场数据。
///
/// # 返回值
///
/// 包含所有已启用的A股规则的 `RuleChain`，顺序如下：
/// 1. 交易时段规则（如果启用）
/// 2. 停牌状态规则（如果提供了 provider）
/// 3. 价格对齐规则（如果提供了 provider）
/// 4. 价格限制规则（如果提供了 provider）
/// 5. 价格笼子规则（如果启用且提供了 provider）
/// 6. 涨跌停限制规则（如果启用且提供了 provider）
/// 7. 手数规则（如果启用）
/// 8. T+1 规则（如果启用）
/// 9. 限流规则（如果配置了）
pub fn create_ashare_rule_chain(
    config: &AShareRuleConfig,
    provider: Option<Arc<dyn MarketDataProvider>>,
) -> RuleChain {
    let mut chain = RuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(SessionRule::new(session_provider.clone())));
        }
    }

    if let Some(ref provider) = provider {
        chain.add_rule(Arc::new(InstrumentStatusRule::new(provider.clone())));
        chain.add_rule(Arc::new(PriceTickRule::new(provider.clone())));
        chain.add_rule(Arc::new(PriceBandRule::new(provider.clone())));

        if config.price_cage_enabled {
            if let Some(session_provider) = &config.session_provider {
                chain.add_rule(Arc::new(PriceCageRule::new(
                    session_provider.clone(),
                    config.price_cage_pct,
                    provider.clone(),
                    config.missing_market_data_policy,
                )));
            }
        }

        if config.price_limit_enabled {
            chain.add_rule(Arc::new(PriceLimitRule::new(
                provider.clone(),
                config.missing_market_data_policy,
            )));
        }
    }

    if config.lot_size_enabled {
        chain.add_rule(Arc::new(lot_size::LotSizeRule::new()));
    }

    if config.t1_enabled {
        if let Some(t1_ledger) = &config.t1_ledger {
            chain.add_rule(Arc::new(T1Rule::new(t1_ledger.clone())));
        }
    }

    if let Some(throttler) = create_throttler_rule(
        config.max_order_submit_per_account.clone(),
        config.max_order_submit_per_symbol.clone(),
    ) {
        chain.add_rule(throttler);
    }

    chain
}

pub fn create_ashare_command_rule_chain_with_provider(
    config: &AShareRuleConfig,
    provider: Arc<dyn MarketDataProvider>,
) -> CommandRuleChain {
    create_ashare_command_rule_chain(config, Some(provider))
}

/// 根据给定配置创建包含所有A股命令规则的策略链。
///
/// # 参数
///
/// * `config` - A股规则配置，指定要启用的规则。
/// * `provider` - 可选的市场数据提供者，用于获取各种市场数据。
///
/// # 返回值
///
/// 包含所有已启用的A股命令规则的 `CommandRuleChain`，顺序如下：
/// 1. 撤单会话规则（如果启用）
pub fn create_ashare_command_rule_chain(
    config: &AShareRuleConfig,
    _provider: Option<Arc<dyn MarketDataProvider>>,
) -> CommandRuleChain {
    let mut chain = CommandRuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(CancelSessionRule::new(session_provider.clone())));
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
