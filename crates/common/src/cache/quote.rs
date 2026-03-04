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

//! 通用报价缓存，用于维护每个交易工具的最后已知报价。
//!
//! 该缓存通常由 WebSocket 适配器使用，以处理交易所可能发送不完整买入或卖出信息的增量（部分）报价更新。
//! 通过缓存最后一次完整的报价，适配器可以将部分更新与缓存值合并，以重建完整的 `QuoteTick`。

use ahash::AHashMap;
use nautilus_core::UnixNanos;
use nautilus_model::{
    data::quote::QuoteTick,
    identifiers::InstrumentId,
    types::{Price, Quantity},
};

/// 存储每个交易工具最后已知报价的缓存。
///
/// 这对于处理来自交易所 WebSocket 推送的部分报价更新特别有用，因为这些更新可能只包含市场的一侧（买价或卖价）。
/// 该缓存为每个交易工具维持最近的完整报价，允许适配器在处理部分更新时填补缺失的信息。
///
/// # 线程安全
///
/// 此缓存不是线程安全的。如果跨线程共享，请将其包装在适当的同步原语中，
/// 例如 `Arc<RwLock<QuoteCache>>` 或 `Arc<Mutex<QuoteCache>>`。
#[derive(Debug, Clone)]
pub struct QuoteCache {
    quotes: AHashMap<InstrumentId, QuoteTick>,
}

impl QuoteCache {
    /// 创建一个新的空 [`QuoteCache`]。
    #[must_use]
    pub fn new() -> Self {
        Self {
            quotes: AHashMap::new(),
        }
    }

    /// 返回给定工具的缓存报价（如果可用）。
    #[must_use]
    pub fn get(&self, instrument_id: &InstrumentId) -> Option<&QuoteTick> {
        self.quotes.get(instrument_id)
    }

    /// 为给定工具在缓存中插入或更新报价。
    ///
    /// 如果之前已存在缓存报价，则将其返回。
    pub fn insert(&mut self, instrument_id: InstrumentId, quote: QuoteTick) -> Option<QuoteTick> {
        self.quotes.insert(instrument_id, quote)
    }

    /// 移除给定工具的缓存报价。
    ///
    /// 如果之前已存在缓存报价，则将其返回。
    pub fn remove(&mut self, instrument_id: &InstrumentId) -> Option<QuoteTick> {
        self.quotes.remove(instrument_id)
    }

    /// 如果缓存中包含给定工具的报价，则返回 `true`。
    #[must_use]
    pub fn contains(&self, instrument_id: &InstrumentId) -> bool {
        self.quotes.contains_key(instrument_id)
    }

    /// 返回缓存报价的数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.quotes.len()
    }

    /// 如果缓存为空，则返回 `true`。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.quotes.is_empty()
    }

    /// 清除所有缓存报价。
    ///
    /// 通常在重新连接后调用，以确保不使用断开连接前的陈旧报价。
    pub fn clear(&mut self) {
        self.quotes.clear();
    }

    /// 处理部分报价更新，必要时与缓存值合并。
    ///
    /// 此方法处理可能缺失某些字段的部分报价更新。
    /// 如果任何字段为 `None`，它将使用缓存报价中对应的字段。
    /// 如果没有缓存报价且缺失任何必需字段，则返回错误。
    ///
    /// # Errors
    ///
    /// 在以下情况下返回错误：
    /// - 任何必需字段为 `None` 且没有缓存报价。
    /// - 接收到的第一个报价不完整（没有可合并的缓存值）。
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        instrument_id: InstrumentId,
        bid_price: Option<Price>,
        ask_price: Option<Price>,
        bid_size: Option<Quantity>,
        ask_size: Option<Quantity>,
        ts_event: UnixNanos,
        ts_init: UnixNanos,
    ) -> anyhow::Result<QuoteTick> {
        let cached = self.quotes.get(&instrument_id);

        // 解析每个字段：使用提供的值或回流到缓存值
        let bid_price = match (bid_price, cached) {
            (Some(p), _) => p,
            (None, Some(q)) => q.bid_price,
            (None, None) => {
                anyhow::bail!(
                    "无法处理 {instrument_id} 的部分报价：缺失 bid_price 且没有缓存值"
                )
            }
        };

        let ask_price = match (ask_price, cached) {
            (Some(p), _) => p,
            (None, Some(q)) => q.ask_price,
            (None, None) => {
                anyhow::bail!(
                    "无法处理 {instrument_id} 的部分报价：缺失 ask_price 且没有缓存值"
                )
            }
        };

        let bid_size = match (bid_size, cached) {
            (Some(s), _) => s,
            (None, Some(q)) => q.bid_size,
            (None, None) => {
                anyhow::bail!(
                    "无法处理 {instrument_id} 的部分报价：缺失 bid_size 且没有缓存值"
                )
            }
        };

        let ask_size = match (ask_size, cached) {
            (Some(s), _) => s,
            (None, Some(q)) => q.ask_size,
            (None, None) => {
                anyhow::bail!(
                    "无法处理 {instrument_id} 的部分报价：缺失 ask_size 且没有缓存值"
                )
            }
        };

        let quote = QuoteTick::new(
            instrument_id,
            bid_price,
            ask_price,
            bid_size,
            ask_size,
            ts_event,
            ts_init,
        );

        self.quotes.insert(instrument_id, quote);

        Ok(quote)
    }
}

impl Default for QuoteCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use nautilus_core::UnixNanos;
    use nautilus_model::types::{Price, Quantity};
    use rstest::rstest;

    use super::*;

    fn make_quote(instrument_id: InstrumentId, _bid: f64, _ask: f64) -> QuoteTick {
        QuoteTick::new(
            instrument_id,
            Price::from("100.0"),
            Price::from("101.0"),
            Quantity::from("10.0"),
            Quantity::from("20.0"),
            UnixNanos::default(),
            UnixNanos::default(),
        )
    }

    #[rstest]
    fn test_new_cache_is_empty() {
        let cache = QuoteCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[rstest]
    fn test_insert_and_get() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");
        let quote = make_quote(instrument_id, 100.0, 101.0);

        assert_eq!(cache.insert(instrument_id, quote), None);
        assert_eq!(cache.len(), 1);
        assert!(cache.contains(&instrument_id));
        assert_eq!(cache.get(&instrument_id), Some(&quote));
    }

    #[rstest]
    fn test_insert_returns_previous_value() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");
        let quote1 = make_quote(instrument_id, 100.0, 101.0);
        let quote2 = make_quote(instrument_id, 102.0, 103.0);

        cache.insert(instrument_id, quote1);
        let previous = cache.insert(instrument_id, quote2);

        assert_eq!(previous, Some(quote1));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&instrument_id), Some(&quote2));
    }

    #[rstest]
    fn test_remove() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");
        let quote = make_quote(instrument_id, 100.0, 101.0);

        cache.insert(instrument_id, quote);
        assert_eq!(cache.remove(&instrument_id), Some(quote));
        assert!(cache.is_empty());
        assert!(!cache.contains(&instrument_id));
        assert_eq!(cache.get(&instrument_id), None);
    }

    #[rstest]
    fn test_remove_nonexistent() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        assert_eq!(cache.remove(&instrument_id), None);
    }

    #[rstest]
    fn test_clear() {
        let mut cache = QuoteCache::new();
        let id1 = InstrumentId::from("BTCUSDT.BINANCE");
        let id2 = InstrumentId::from("ETHUSDT.BINANCE");

        cache.insert(id1, make_quote(id1, 100.0, 101.0));
        cache.insert(id2, make_quote(id2, 200.0, 201.0));

        assert_eq!(cache.len(), 2);

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert!(!cache.contains(&id1));
        assert!(!cache.contains(&id2));
    }

    #[rstest]
    fn test_multiple_instruments() {
        let mut cache = QuoteCache::new();
        let id1 = InstrumentId::from("BTCUSDT.BINANCE");
        let id2 = InstrumentId::from("ETHUSDT.BINANCE");
        let id3 = InstrumentId::from("XRPUSDT.BINANCE");

        let quote1 = make_quote(id1, 100.0, 101.0);
        let quote2 = make_quote(id2, 200.0, 201.0);
        let quote3 = make_quote(id3, 0.5, 0.51);

        cache.insert(id1, quote1);
        cache.insert(id2, quote2);
        cache.insert(id3, quote3);

        assert_eq!(cache.len(), 3);
        assert_eq!(cache.get(&id1), Some(&quote1));
        assert_eq!(cache.get(&id2), Some(&quote2));
        assert_eq!(cache.get(&id3), Some(&quote3));
    }

    #[rstest]
    fn test_default() {
        let cache = QuoteCache::default();
        assert!(cache.is_empty());
    }

    #[rstest]
    fn test_clone() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");
        let quote = make_quote(instrument_id, 100.0, 101.0);

        cache.insert(instrument_id, quote);

        let cloned = cache.clone();
        assert_eq!(cloned.len(), 1);
        assert_eq!(cloned.get(&instrument_id), Some(&quote));
    }

    #[rstest]
    fn test_process_complete_quote() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        let result = cache.process(
            instrument_id,
            Some(Price::from("100.5")),
            Some(Price::from("101.0")),
            Some(Quantity::from("10.0")),
            Some(Quantity::from("20.0")),
            UnixNanos::default(),
            UnixNanos::default(),
        );

        assert!(result.is_ok());
        let quote = result.unwrap();
        assert_eq!(quote.instrument_id, instrument_id);
        assert_eq!(quote.bid_price, Price::from("100.5"));
        assert_eq!(quote.ask_price, Price::from("101.0"));
        assert_eq!(quote.bid_size, Quantity::from("10.0"));
        assert_eq!(quote.ask_size, Quantity::from("20.0"));

        // 应该已被缓存
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&instrument_id), Some(&quote));
    }

    #[rstest]
    fn test_process_partial_quote_without_cache() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        // 第一次更新时缺失 bid_price 应该失败
        let result = cache.process(
            instrument_id,
            None,
            Some(Price::from("101.0")),
            Some(Quantity::from("10.0")),
            Some(Quantity::from("20.0")),
            UnixNanos::default(),
            UnixNanos::default(),
        );

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing bid_price")
        );
    }

    #[rstest]
    fn test_process_partial_quote_with_cache() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        // 首先，处理一个完整的报价
        let first_quote = cache
            .process(
                instrument_id,
                Some(Price::from("100.0")),
                Some(Price::from("101.0")),
                Some(Quantity::from("10.0")),
                Some(Quantity::from("20.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        // 现在处理仅包含买方（bid side）的部分更新
        let result = cache.process(
            instrument_id,
            Some(Price::from("100.5")),
            None, // 使用缓存的 ask_price
            Some(Quantity::from("15.0")),
            None, // 使用缓存的 ask_size
            UnixNanos::default(),
            UnixNanos::default(),
        );

        assert!(result.is_ok());
        let quote = result.unwrap();

        // 买方（bid side）应该被更新
        assert_eq!(quote.bid_price, Price::from("100.5"));
        assert_eq!(quote.bid_size, Quantity::from("15.0"));

        // 卖方（ask side）应该来自缓存
        assert_eq!(quote.ask_price, first_quote.ask_price);
        assert_eq!(quote.ask_size, first_quote.ask_size);

        // 缓存应该用新报价更新
        assert_eq!(cache.get(&instrument_id), Some(&quote));
    }

    #[rstest]
    fn test_process_updates_cache() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        // 第一次报价
        cache
            .process(
                instrument_id,
                Some(Price::from("100.0")),
                Some(Price::from("101.0")),
                Some(Quantity::from("10.0")),
                Some(Quantity::from("20.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        // 第二个完整的报价应该替换缓存值
        let quote2 = cache
            .process(
                instrument_id,
                Some(Price::from("102.0")),
                Some(Price::from("103.0")),
                Some(Quantity::from("30.0")),
                Some(Quantity::from("40.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        assert_eq!(cache.get(&instrument_id), Some(&quote2));
        assert_eq!(quote2.bid_price, Price::from("102.0"));
    }

    #[rstest]
    fn test_process_multiple_instruments() {
        let mut cache = QuoteCache::new();
        let id1 = InstrumentId::from("BTCUSDT.BINANCE");
        let id2 = InstrumentId::from("ETHUSDT.BINANCE");

        let quote1 = cache
            .process(
                id1,
                Some(Price::from("100.0")),
                Some(Price::from("101.0")),
                Some(Quantity::from("10.0")),
                Some(Quantity::from("20.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        let quote2 = cache
            .process(
                id2,
                Some(Price::from("200.0")),
                Some(Price::from("201.0")),
                Some(Quantity::from("30.0")),
                Some(Quantity::from("40.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&id1), Some(&quote1));
        assert_eq!(cache.get(&id2), Some(&quote2));
    }

    #[rstest]
    fn test_process_clear_removes_cached_values() {
        let mut cache = QuoteCache::new();
        let instrument_id = InstrumentId::from("BTCUSDT.BINANCE");

        // 添加一个报价
        cache
            .process(
                instrument_id,
                Some(Price::from("100.0")),
                Some(Price::from("101.0")),
                Some(Quantity::from("10.0")),
                Some(Quantity::from("20.0")),
                UnixNanos::default(),
                UnixNanos::default(),
            )
            .unwrap();

        assert_eq!(cache.len(), 1);

        // 清除缓存
        cache.clear();

        // 部分更新现在应该失败（没有缓存值）
        let result = cache.process(
            instrument_id,
            Some(Price::from("100.5")),
            None,
            Some(Quantity::from("15.0")),
            None,
            UnixNanos::default(),
            UnixNanos::default(),
        );

        assert!(result.is_err());
    }
}
