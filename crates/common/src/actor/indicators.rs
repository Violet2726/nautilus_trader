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

// 开发中
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

use std::{collections::HashMap, sync::Arc};

use nautilus_indicators::indicator::Indicator;
use nautilus_model::{data::BarType, identifiers::InstrumentId};

/// 包含所有与指标 (Indicator) 相关的引用。
#[derive(Clone, Default)]
pub(crate) struct Indicators {
    pub indicators: Vec<Arc<dyn Indicator>>,
    pub indicators_for_quotes: HashMap<InstrumentId, Vec<Arc<dyn Indicator>>>,
    pub indicators_for_trades: HashMap<InstrumentId, Vec<Arc<dyn Indicator>>>,
    pub indicators_for_bars: HashMap<BarType, Vec<Arc<dyn Indicator>>>,
}

impl Indicators {
    /// 检查所有注册的指标是否都已初始化。
    pub fn is_initialized(&self) -> bool {
        if self.indicators.is_empty() {
            return false;
        }

        self.indicators
            .iter()
            .all(|indicator| indicator.initialized())
    }

    /// 注册一个指标，用以接收给定工具 ID 的报价 Ticks (Quote ticks)。
    pub fn register_indicator_for_quotes(
        &mut self,
        instrument_id: InstrumentId,
        indicator: Arc<dyn Indicator>,
    ) {
        // 如果尚未存在，则添加到总体指标列表中
        if !self.indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            self.indicators.push(indicator.clone());
        }

        // 添加到工具特定的报价指标中
        let indicators = self.indicators_for_quotes.entry(instrument_id).or_default();

        if indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            // TODO: 记录错误 - 已经注册过
        } else {
            indicators.push(indicator);
            // TODO: 记录注册信息
        }
    }

    /// 注册一个指标，用以接收给定工具 ID 的逐笔成交 Ticks (Trade ticks)。
    pub fn register_indicator_for_trades(
        &mut self,
        instrument_id: InstrumentId,
        indicator: Arc<dyn Indicator>,
    ) {
        // 如果尚未存在，则添加到总体指标列表中
        if !self.indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            self.indicators.push(indicator.clone());
        }

        // 添加到工具特定的逐笔成交指标中
        let indicators = self.indicators_for_trades.entry(instrument_id).or_default();

        if indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            // TODO: 记录错误 - 已经注册过
        } else {
            indicators.push(indicator);
            // TODO: 记录注册信息
        }
    }

    /// 注册一个指标，用以接收给定 K 线类型 (Bar type) 的 K 线数据。
    pub fn register_indicator_for_bars(
        &mut self,
        bar_type: BarType,
        indicator: Arc<dyn Indicator>,
    ) {
        // 如果尚未存在，则添加到总体指标列表中
        if !self.indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            self.indicators.push(indicator.clone());
        }

        // 获取标准 K 线类型 (Standard bar type)
        let standard_bar_type = bar_type.standard();

        // 添加到 K 线类型特定的指标中
        let indicators = self
            .indicators_for_bars
            .entry(standard_bar_type)
            .or_default();

        if indicators.iter().any(|i| Arc::ptr_eq(i, &indicator)) {
            // TODO: 记录错误 - 已经注册过
        } else {
            indicators.push(indicator);
            // TODO: 记录注册信息
        }
    }
}
