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

use nautilus_common::throttler::RateLimit;
use nautilus_common::session::SessionProvider;
use nautilus_portfolio::t1_ledger::T1Ledger;
use std::sync::Arc;

/// Configuration for A-share trading rules.
#[derive(Clone)]
pub struct AShareRuleConfig {
    /// Enable session-based trading phase checks.
    pub session_enabled: bool,
    /// Session provider for trading phase information.
    pub session_provider: Option<Arc<dyn SessionProvider>>,

    /// Enable price cage validation.
    pub price_cage_enabled: bool,
    /// Price cage percentage (e.g., 0.02 for 2%).
    pub price_cage_pct: f64,

    /// Enable T+1 settlement rule.
    pub t1_enabled: bool,
    /// T+1 ledger for tracking sellable positions.
    pub t1_ledger: Option<Arc<T1Ledger>>,

    /// Enable lot size validation.
    pub lot_size_enabled: bool,

    /// Maximum order submissions per account.
    pub max_order_submit_per_account: Option<RateLimit>,
    /// Maximum order submissions per symbol.
    pub max_order_submit_per_symbol: Option<RateLimit>,
}

impl std::fmt::Debug for AShareRuleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AShareRuleConfig")
            .field("session_enabled", &self.session_enabled)
            .field("price_cage_enabled", &self.price_cage_enabled)
            .field("price_cage_pct", &self.price_cage_pct)
            .field("t1_enabled", &self.t1_enabled)
            .field("lot_size_enabled", &self.lot_size_enabled)
            .field("max_order_submit_per_account", &self.max_order_submit_per_account)
            .field("max_order_submit_per_symbol", &self.max_order_submit_per_symbol)
            .finish()
    }
}

impl Default for AShareRuleConfig {
    fn default() -> Self {
        Self {
            session_enabled: false,
            session_provider: None,
            price_cage_enabled: false,
            price_cage_pct: 0.02,
            t1_enabled: false,
            t1_ledger: None,
            lot_size_enabled: false,
            max_order_submit_per_account: None,
            max_order_submit_per_symbol: None,
        }
    }
}

impl AShareRuleConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_session(mut self, enabled: bool, provider: Option<Arc<dyn SessionProvider>>) -> Self {
        self.session_enabled = enabled;
        self.session_provider = provider;
        self
    }

    pub fn with_price_cage(mut self, enabled: bool, pct: f64) -> Self {
        self.price_cage_enabled = enabled;
        self.price_cage_pct = pct;
        self
    }

    pub fn with_t1(mut self, enabled: bool, ledger: Option<Arc<T1Ledger>>) -> Self {
        self.t1_enabled = enabled;
        self.t1_ledger = ledger;
        self
    }

    pub fn with_lot_size(mut self, enabled: bool) -> Self {
        self.lot_size_enabled = enabled;
        self
    }

    pub fn with_throttling(
        mut self,
        per_account: Option<RateLimit>,
        per_symbol: Option<RateLimit>,
    ) -> Self {
        self.max_order_submit_per_account = per_account;
        self.max_order_submit_per_symbol = per_symbol;
        self
    }
}
