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

use crate::risk::rules::{
    price_band::PriceBand, price_cage::MarketData as PriceCageMarketData,
    price_limit::PriceLimitMarketData,
};
use nautilus_common::cache::Cache;
use nautilus_model::{
    enums::{MarketStatusAction, OrderSide},
    identifiers::InstrumentId,
    instruments::Instrument,
    orders::Order,
    types::Price,
};
use nautilus_rules::common::RuleContext;
use std::sync::{Arc, Mutex};

pub trait MarketDataProvider: Send + Sync {
    fn instrument_status_action(&self, _instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        // 来源：缓存中的工具状态数据 (cache.instrument_status)
        None
    }

    fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
        // 来源：工具定义中的 price_increment 字段
        None
    }

    fn price_precision(&self, _instrument_id: &InstrumentId) -> Option<u8> {
        // 来源：工具定义中的 price_precision 字段
        None
    }

    fn price_band(&self, _instrument_id: &InstrumentId) -> PriceBand {
        // 来源：前收盘价，来自动态行情数据 (bars 或 market data)
        PriceBand::default()
    }

    fn price_cage_market_data(&self, _context: &RuleContext) -> PriceCageMarketData {
        // 来源：基于上下文的动态计算，可能来自行情数据
        PriceCageMarketData::default()
    }

    fn price_limit_market_data(&self, _context: &RuleContext) -> PriceLimitMarketData {
        // 来源：基于工具类型和板块的涨跌停规则，可能来自静态配置或动态数据
        PriceLimitMarketData::default()
    }
}

/// 基于缓存的市场数据提供者实现。
///
/// 从缓存中获取工具状态、tick 大小等数据，提供给风险规则使用。
pub struct CacheMarketDataProvider {
    cache: Arc<Mutex<Cache>>,
}

impl CacheMarketDataProvider {
    pub fn new(cache: Arc<Mutex<Cache>>) -> Self {
        Self { cache }
    }

    fn with_cache<T>(&self, f: impl FnOnce(&Cache) -> T) -> T {
        match self.cache.lock() {
            Ok(guard) => f(&guard),
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                f(&guard)
            }
        }
    }

    fn tick_size_from_cache(cache: &Cache, instrument_id: &InstrumentId) -> Option<Price> {
        cache
            .instrument(instrument_id)
            .map(|instrument| instrument.price_increment())
    }

    fn price_precision_from_cache(cache: &Cache, instrument_id: &InstrumentId) -> Option<u8> {
        cache
            .instrument(instrument_id)
            .map(|instrument| instrument.price_precision())
    }
}

// Cache 内含非 Send 的数据库适配器 trait object；当前 provider 仅通过互斥锁访问内存缓存数据，
// 不跨线程移动底层 database adapter。这里延续现有设计，手动声明并发边界。
unsafe impl Send for CacheMarketDataProvider {}
unsafe impl Sync for CacheMarketDataProvider {}

impl MarketDataProvider for CacheMarketDataProvider {
    fn instrument_status_action(&self, instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        self.with_cache(|cache| cache.instrument_status(instrument_id).map(|s| s.action))
    }

    fn tick_size(&self, instrument_id: &InstrumentId) -> Option<Price> {
        self.with_cache(|cache| Self::tick_size_from_cache(cache, instrument_id))
    }

    fn price_precision(&self, instrument_id: &InstrumentId) -> Option<u8> {
        self.with_cache(|cache| Self::price_precision_from_cache(cache, instrument_id))
    }

    fn price_band(&self, instrument_id: &InstrumentId) -> PriceBand {
        self.with_cache(|cache| {
            cache
                .instrument(instrument_id)
                .map(|instrument| PriceBand {
                    min_price: instrument.min_price(),
                    max_price: instrument.max_price(),
                })
                .unwrap_or_default()
        })
    }

    fn price_cage_market_data(&self, context: &RuleContext) -> PriceCageMarketData {
        self.with_cache(|cache| {
            let quote = cache.quote(&context.instrument_id);
            let trade = cache.trade(&context.instrument_id);
            let side = context.order.order_side();

            let current_price = match side {
                OrderSide::Buy => quote
                    .and_then(|q| {
                        if q.ask_price.raw > 0 {
                            Some(q.ask_price)
                        } else if q.bid_price.raw > 0 {
                            Some(q.bid_price)
                        } else {
                            None
                        }
                    })
                    .or_else(|| trade.map(|t| t.price)),
                OrderSide::Sell => quote
                    .and_then(|q| {
                        if q.bid_price.raw > 0 {
                            Some(q.bid_price)
                        } else if q.ask_price.raw > 0 {
                            Some(q.ask_price)
                        } else {
                            None
                        }
                    })
                    .or_else(|| trade.map(|t| t.price)),
                _ => quote
                    .and_then(|q| {
                        if q.bid_price.raw > 0 {
                            Some(q.bid_price)
                        } else if q.ask_price.raw > 0 {
                            Some(q.ask_price)
                        } else {
                            None
                        }
                    })
                    .or_else(|| trade.map(|t| t.price)),
            };

            PriceCageMarketData {
                current_price,
                tick_size: Self::tick_size_from_cache(cache, &context.instrument_id),
            }
        })
    }

    fn price_limit_market_data(&self, context: &RuleContext) -> PriceLimitMarketData {
        self.with_cache(|cache| {
            let prev_close = {
                use nautilus_model::enums::{AggregationSource, PriceType};

                let mut close_from_bars = None;
                for source in [AggregationSource::External, AggregationSource::Internal] {
                    let bar_types =
                        cache.bar_types(Some(&context.instrument_id), Some(&PriceType::Last), source);
                    for bar_type in bar_types {
                        if let Some(bar) = cache.bar(bar_type) {
                            close_from_bars = Some(bar.close);
                            break;
                        }
                    }
                    if close_from_bars.is_some() {
                        break;
                    }
                }

                close_from_bars
                    .or_else(|| cache.trade(&context.instrument_id).map(|t| t.price))
                    .or_else(|| {
                        cache.quote(&context.instrument_id).map(|q| {
                            let mid = (q.bid_price.as_f64() + q.ask_price.as_f64()) / 2.0;
                            Price::new(mid, q.bid_price.precision)
                        })
                    })
            };

            let stock_name = cache
                .instrument(&context.instrument_id)
                .map(|instrument| instrument.raw_symbol().as_str().to_string());

            PriceLimitMarketData {
                prev_close,
                tick_size: Self::tick_size_from_cache(cache, &context.instrument_id),
                stock_name,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_core::UnixNanos;
    use nautilus_model::{
        data::{InstrumentStatus, QuoteTick, TradeTick},
        enums::{AggressorSide, MarketStatusAction, OrderType},
        identifiers::{ClientOrderId, InstrumentId, StrategyId, TradeId, TraderId},
        instruments::{Equity, InstrumentAny},
        orders::OrderTestBuilder,
        types::{Currency, Quantity},
    };
    use nautilus_rules::common::RuleContext;
    use ustr::Ustr;

    fn make_equity(id: InstrumentId) -> InstrumentAny {
        Equity::new(
            id,
            nautilus_model::identifiers::Symbol::from("600000"),
            Some(Ustr::from("TEST600000")),
            Currency::from("CNY"),
            2,
            Price::from("0.01"),
            None,
            None,
            None,
            Some(Price::from("20.00")),
            Some(Price::from("5.00")),
            None,
            None,
            None,
            None,
            None,
            UnixNanos::default(),
            UnixNanos::default(),
        )
        .into()
    }

    fn make_rule_context(instrument_id: InstrumentId, side: OrderSide, price: f64) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRAT-001"))
            .instrument_id(instrument_id)
            .client_order_id(ClientOrderId::from("O-1"))
            .side(side)
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(price, 2))
            .build();

        RuleContext::new(order, instrument_id, None, 0)
    }

    #[test]
    fn test_cache_provider_reads_instrument_status_action() {
        let instrument_id = InstrumentId::from("600000.SH");
        let mut cache = Cache::default();
        cache
            .add_instrument_status(InstrumentStatus::new(
                instrument_id,
                MarketStatusAction::Suspend,
                UnixNanos::default(),
                UnixNanos::default(),
                None,
                None,
                None,
                None,
                None,
            ))
            .unwrap();

        let provider = CacheMarketDataProvider::new(Arc::new(Mutex::new(cache)));
        assert_eq!(
            provider.instrument_status_action(&instrument_id),
            Some(MarketStatusAction::Suspend)
        );
    }

    #[test]
    fn test_cache_provider_reads_tick_size_from_instrument() {
        let instrument_id = InstrumentId::from("600000.SH");
        let mut cache = Cache::default();
        cache.add_instrument(make_equity(instrument_id)).unwrap();

        let provider = CacheMarketDataProvider::new(Arc::new(Mutex::new(cache)));
        assert_eq!(provider.tick_size(&instrument_id), Some(Price::from("0.01")));
    }

    #[test]
    fn test_cache_provider_price_cage_prefers_side_quote_price() {
        let instrument_id = InstrumentId::from("600000.SH");
        let mut cache = Cache::default();
        cache.add_instrument(make_equity(instrument_id)).unwrap();
        cache
            .add_quote(QuoteTick::new(
                instrument_id,
                Price::new(10.00, 2),
                Price::new(10.10, 2),
                Quantity::new(100.0, 0),
                Quantity::new(100.0, 0),
                UnixNanos::default(),
                UnixNanos::default(),
            ))
            .unwrap();

        let provider = CacheMarketDataProvider::new(Arc::new(Mutex::new(cache)));

        let buy_ctx = make_rule_context(instrument_id, OrderSide::Buy, 10.05);
        let sell_ctx = make_rule_context(instrument_id, OrderSide::Sell, 10.05);

        let buy_data = provider.price_cage_market_data(&buy_ctx);
        let sell_data = provider.price_cage_market_data(&sell_ctx);

        assert_eq!(buy_data.current_price, Some(Price::new(10.10, 2)));
        assert_eq!(sell_data.current_price, Some(Price::new(10.00, 2)));
        assert_eq!(buy_data.tick_size, Some(Price::from("0.01")));
    }

    #[test]
    fn test_cache_provider_price_limit_uses_trade_as_prev_close() {
        let instrument_id = InstrumentId::from("600000.SH");
        let mut cache = Cache::default();
        cache.add_instrument(make_equity(instrument_id)).unwrap();
        cache
            .add_trade(TradeTick::new(
                instrument_id,
                Price::new(10.25, 2),
                Quantity::new(100.0, 0),
                AggressorSide::NoAggressor,
                TradeId::new("T-1"),
                UnixNanos::default(),
                UnixNanos::default(),
            ))
            .unwrap();

        let provider = CacheMarketDataProvider::new(Arc::new(Mutex::new(cache)));
        let ctx = make_rule_context(instrument_id, OrderSide::Buy, 10.00);
        let data = provider.price_limit_market_data(&ctx);

        assert_eq!(data.prev_close, Some(Price::new(10.25, 2)));
        assert_eq!(data.tick_size, Some(Price::from("0.01")));
        assert_eq!(data.stock_name.as_deref(), Some("600000"));
    }
}
