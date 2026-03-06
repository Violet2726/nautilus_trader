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

//! Provides a configuration for `RiskEngine` instances.

use ahash::AHashMap;
use nautilus_common::throttler::RateLimit;
use nautilus_core::datetime::NANOSECONDS_IN_SECOND;
use nautilus_model::identifiers::InstrumentId;
use rust_decimal::Decimal;

use std::sync::Arc;

/// Configuration for `RiskEngineConfig` instances.
#[derive(Clone)]
pub struct RiskEngineConfig {
    pub bypass: bool,
    pub max_order_submit: RateLimit,
    pub max_order_modify: RateLimit,
    pub max_notional_per_order: AHashMap<InstrumentId, Decimal>,
    pub debug: bool,
    // ---- A 股扩展 ----
    pub session_provider: Option<Arc<dyn nautilus_common::session::SessionProvider>>,
    pub price_cage_enabled: bool,
    pub price_cage_pct: f64,
    pub t1_enabled: bool,
    pub max_order_submit_per_account: Option<RateLimit>,
    pub max_order_submit_per_symbol: Option<RateLimit>,
    pub max_trade_command: Option<RateLimit>,
}

impl std::fmt::Debug for RiskEngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RiskEngineConfig")
            .field("bypass", &self.bypass)
            .field("max_order_submit", &self.max_order_submit)
            .field("max_order_modify", &self.max_order_modify)
            .field("max_notional_per_order", &self.max_notional_per_order)
            .field("debug", &self.debug)
            .field("session_provider", &self.session_provider.is_some())
            .field("price_cage_enabled", &self.price_cage_enabled)
            .field("price_cage_pct", &self.price_cage_pct)
            .field("t1_enabled", &self.t1_enabled)
            .field("max_order_submit_per_account", &self.max_order_submit_per_account)
            .field("max_order_submit_per_symbol", &self.max_order_submit_per_symbol)
            .field("max_trade_command", &self.max_trade_command)
            .finish()
    }
}

impl Default for RiskEngineConfig {
    /// Creates a new [`RiskEngineConfig`] instance.
    fn default() -> Self {
        Self {
            bypass: false,
            max_order_submit: RateLimit::new(100, NANOSECONDS_IN_SECOND),
            max_order_modify: RateLimit::new(100, NANOSECONDS_IN_SECOND),
            max_notional_per_order: AHashMap::new(),
            debug: false,
            session_provider: None,
            price_cage_enabled: false,
            price_cage_pct: 0.02,
            t1_enabled: false,
            max_order_submit_per_account: None,
            max_order_submit_per_symbol: None,
            max_trade_command: None,
        }
    }
}
