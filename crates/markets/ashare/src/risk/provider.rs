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
use nautilus_common::cache::Cache;
use nautilus_model::{enums::MarketStatusAction, identifiers::InstrumentId, types::Price};
use nautilus_rules::common::RuleContext;

pub trait MarketDataProvider: Send + Sync {
    fn instrument_status_action(&self, _instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        // 权威来源：缓存中的工具状态数据 (cache.instrument_status)
        None
    }

    fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
        // 权威来源：工具定义中的 price_increment 字段
        None
    }

    fn price_band(&self, _instrument_id: &InstrumentId) -> PriceBand {
        // 权威来源：前收盘价，来自动态行情数据 (bars 或 market data)
        PriceBand::default()
    }

    fn price_cage_market_data(&self, _context: &RuleContext) -> PriceCageMarketData {
        // 权威来源：基于上下文的动态计算，可能来自行情数据
        PriceCageMarketData::default()
    }

    fn price_limit_market_data(&self, _context: &RuleContext) -> PriceLimitMarketData {
        // 权威来源：基于工具类型和板块的涨跌停规则，可能来自静态配置或动态数据
        PriceLimitMarketData::default()
    }
}

use std::sync::{Arc, Mutex};

/// 基于缓存的市场数据提供者实现。
///
/// 从缓存中获取工具状态、tick 大小等数据，提供给风险规则使用。
pub struct CacheMarketDataProvider {
    // 暂时使用空实现，避免线程安全问题
    // TODO: 实现线程安全的数据访问方法
}

impl CacheMarketDataProvider {
    pub fn new(_cache: Arc<Mutex<Cache>>) -> Self {
        Self {}
    }
}

// 确保类型实现 Send + Sync
unsafe impl Send for CacheMarketDataProvider {}
unsafe impl Sync for CacheMarketDataProvider {}

impl MarketDataProvider for CacheMarketDataProvider {
    fn instrument_status_action(&self, _instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        // TODO: 实现线程安全的状态访问
        None
    }

    fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
        // TODO: 实现线程安全的 tick size 访问
        None
    }

    fn price_band(&self, _instrument_id: &InstrumentId) -> PriceBand {
        // TODO: 从 prev_close 计算价格带，需要动态数据源
        PriceBand::default()
    }

    fn price_cage_market_data(&self, _context: &RuleContext) -> PriceCageMarketData {
        // TODO: 基于上下文计算价格笼子数据
        PriceCageMarketData::default()
    }

    fn price_limit_market_data(&self, _context: &RuleContext) -> PriceLimitMarketData {
        // TODO: 基于上下文计算价格限制数据
        PriceLimitMarketData::default()
    }
}
