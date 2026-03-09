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

use crate::risk::rules::{price_band::PriceBand, price_cage::MarketData as PriceCageMarketData, price_limit::PriceLimitMarketData};
use nautilus_model::{enums::MarketStatusAction, identifiers::InstrumentId, types::Price};
use nautilus_rules::common::RuleContext;

pub trait MarketDataProvider: Send + Sync {
    fn instrument_status_action(&self, _instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        None
    }

    fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
        None
    }

    fn price_band(&self, _instrument_id: &InstrumentId) -> PriceBand {
        PriceBand::default()
    }

    fn price_cage_market_data(&self, _context: &RuleContext) -> PriceCageMarketData {
        PriceCageMarketData::default()
    }

    fn price_limit_market_data(&self, _context: &RuleContext) -> PriceLimitMarketData {
        PriceLimitMarketData::default()
    }
}
