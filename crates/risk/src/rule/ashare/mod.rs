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

//! A-share (Chinese stock market) specific trading rules.
//!
//! This module provides rules for validating orders in the Chinese A-share market,
//! including session restrictions, price cage limits, T+1 settlement rules,
//! lot size requirements, and throttling controls.

pub mod config;
pub mod lot_size_rule;
pub mod price_cage_rule;
pub mod session_rule;
pub mod t1_rule;
pub mod throttler_rule;

use super::common::{Rule, RuleChain};
use config::AShareRuleConfig;
use lot_size_rule::LotSizeRule;
use nautilus_portfolio::T1Ledger;
use session_rule::SessionRule;
use std::sync::Arc;
use t1_rule::T1Rule;
use throttler_rule::ThrottlerRule;

/// Creates a rule chain with all A-share rules based on the given configuration.
pub fn create_ashare_rule_chain(
    config: &AShareRuleConfig,
) -> RuleChain {
    let mut chain = RuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(SessionRule::new(session_provider.clone())));
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

/// Creates a rule chain with all A-share rules based on the given configuration and shared ledger.
pub fn create_ashare_rule_chain_with_ledger(
    config: &AShareRuleConfig,
    t1_ledger: Option<Arc<std::sync::RwLock<T1Ledger>>>,
) -> RuleChain {
    let mut chain = RuleChain::new();

    if config.session_enabled {
        if let Some(session_provider) = &config.session_provider {
            chain.add_rule(Arc::new(SessionRule::new(session_provider.clone())));
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

/// Creates a throttler rule for A-share trading.
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
