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

//! 货币之间的汇率计算。
//!
//! 汇率是一种资产相对于另一种资产的价值。

use ahash::{AHashMap, AHashSet};
use nautilus_model::enums::PriceType;
use ustr::Ustr;

/// 使用提供的买入 (bid) 和卖出 (ask) 报价计算两种货币之间的汇率。
///
/// 此函数根据报价构建直接转换率图，并使用深度优先搜索 (DFS) 来累积有效转换路径上的转换率。
/// 虽然完整的 Floyd-Warshall 算法可以计算所有货币对之间的转换率，但此处的 DFS 方法
/// 为单个转换查询提供了一个快速解决方案。
///
/// # 错误
///
/// 如果发生以下情况，则返回错误：
/// - `price_type` 等于 `Last` 或 `Mark`（无法从报价中计算）。
/// - `quotes_bid` 或 `quotes_ask` 为空。
/// - `quotes_bid` 和 `quotes_ask` 长度不相等。
/// - 缺失某个货币对的买入或卖出端。
pub fn get_exchange_rate(
    from_currency: Ustr,
    to_currency: Ustr,
    price_type: PriceType,
    quotes_bid: AHashMap<String, f64>,
    quotes_ask: AHashMap<String, f64>,
) -> anyhow::Result<Option<f64>> {
    if from_currency == to_currency {
        // 当源货币和目标货币相同时，
        // 不需要转换；返回汇率为 1.0。
        return Ok(Some(1.0));
    }

    if quotes_bid.is_empty() || quotes_ask.is_empty() {
        anyhow::bail!("Quote maps must not be empty");
    }
    if quotes_bid.len() != quotes_ask.len() {
        anyhow::bail!("Quote maps must have equal lengths");
    }

    // 根据请求的价格类型构建有效报价
    let effective_quotes: AHashMap<String, f64> = match price_type {
        PriceType::Bid => quotes_bid,
        PriceType::Ask => quotes_ask,
        PriceType::Mid => {
            let mut mid_quotes = AHashMap::new();
            for (pair, bid) in &quotes_bid {
                let ask = quotes_ask
                    .get(pair)
                    .ok_or_else(|| anyhow::anyhow!("Missing ask quote for pair {pair}"))?;
                mid_quotes.insert(pair.clone(), (bid + ask) / 2.0);
            }
            mid_quotes
        }
        _ => anyhow::bail!("Invalid `price_type`, was '{price_type}'"),
    };

    // 构建图：每种货币映射到其邻居及相应的转换率
    let mut graph: AHashMap<Ustr, Vec<(Ustr, f64)>> = AHashMap::new();
    for (pair, rate) in effective_quotes {
        let parts: Vec<&str> = pair.split('/').collect();
        if parts.len() != 2 {
            log::warn!("Skipping invalid pair string: {pair}");
            continue;
        }
        let base = Ustr::from(parts[0]);
        let quote = Ustr::from(parts[1]);

        graph.entry(base).or_default().push((quote, rate));
        graph.entry(quote).or_default().push((base, 1.0 / rate));
    }

    // DFS：搜索从 `from_currency` 到 `to_currency` 的转换路径
    let mut stack: Vec<(Ustr, f64)> = vec![(from_currency, 1.0)];
    let mut visited: AHashSet<Ustr> = AHashSet::new();
    visited.insert(from_currency);

    while let Some((current, current_rate)) = stack.pop() {
        if current == to_currency {
            return Ok(Some(current_rate));
        }
        if let Some(neighbors) = graph.get(&current) {
            for (neighbor, rate) in neighbors {
                if visited.insert(*neighbor) {
                    stack.push((*neighbor, current_rate * rate));
                }
            }
        }
    }

    // 未找到转换路径
    Ok(None)
}

#[cfg(test)]
mod tests {
    use ahash::AHashMap;
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;

    fn setup_test_quotes() -> (AHashMap<String, f64>, AHashMap<String, f64>) {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();

        // 直接交易对
        quotes_bid.insert("EUR/USD".to_string(), 1.1000);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);

        quotes_bid.insert("GBP/USD".to_string(), 1.3000);
        quotes_ask.insert("GBP/USD".to_string(), 1.3002);

        quotes_bid.insert("USD/JPY".to_string(), 110.00);
        quotes_ask.insert("USD/JPY".to_string(), 110.02);

        quotes_bid.insert("AUD/USD".to_string(), 0.7500);
        quotes_ask.insert("AUD/USD".to_string(), 0.7502);

        (quotes_bid, quotes_ask)
    }

    #[rstest]
    fn test_invalid_pair_string() {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();
        // 无效的交易对字符串（缺少 '/'）
        quotes_bid.insert("EURUSD".to_string(), 1.1000);
        quotes_ask.insert("EURUSD".to_string(), 1.1002);
        // 有效的交易对字符串
        quotes_bid.insert("EUR/USD".to_string(), 1.1000);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);

        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();

        let expected = f64::midpoint(1.1000, 1.1002);
        assert!((rate.unwrap() - expected).abs() < 0.0001);
    }

    #[rstest]
    fn test_same_currency() {
        let (quotes_bid, quotes_ask) = setup_test_quotes();
        let rate = get_exchange_rate(
            Ustr::from("USD"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();
        assert_eq!(rate, Some(1.0));
    }

    #[rstest(
        price_type,
        expected,
        case(PriceType::Bid, 1.1000),
        case(PriceType::Ask, 1.1002),
        case(PriceType::Mid, f64::midpoint(1.1000, 1.1002))
    )]
    fn test_direct_pair(price_type: PriceType, expected: f64) {
        let (quotes_bid, quotes_ask) = setup_test_quotes();

        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            price_type,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();

        let rate = rate.unwrap_or_else(|| panic!("预期 {price_type} 有一个转换率"));
        assert!((rate - expected).abs() < 0.0001);
    }

    #[rstest]
    fn test_inverse_pair() {
        let (quotes_bid, quotes_ask) = setup_test_quotes();

        let rate_eur_usd = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid.clone(),
            quotes_ask.clone(),
        )
        .unwrap();
        let rate_usd_eur = get_exchange_rate(
            Ustr::from("USD"),
            Ustr::from("EUR"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();
        if let (Some(eur_usd), Some(usd_eur)) = (rate_eur_usd, rate_usd_eur) {
            assert!(eur_usd.mul_add(usd_eur, -1.0).abs() < 0.0001);
        } else {
            panic!("逆向转换预期应有有效的转换率");
        }
    }

    #[rstest]
    fn test_cross_pair_through_usd() {
        let (quotes_bid, quotes_ask) = setup_test_quotes();
        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("JPY"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();
        // 预期汇率: (EUR/USD mid) * (USD/JPY mid)
        let mid_eur_usd = f64::midpoint(1.1000, 1.1002);
        let mid_usd_jpy = f64::midpoint(110.00, 110.02);
        let expected = mid_eur_usd * mid_usd_jpy;
        if let Some(val) = rate {
            assert!((val - expected).abs() < 0.1);
        } else {
            panic!("预期应有通过 USD 转换的汇率，但得到的是 None");
        }
    }

    #[rstest]
    fn test_no_conversion_path() {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();

        // 仅提供了一个交易对
        quotes_bid.insert("EUR/USD".to_string(), 1.1000);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);

        // 尝试从 EUR 转换到 JPY 应该得到 None
        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("JPY"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();
        assert_eq!(rate, None);
    }

    #[rstest]
    fn test_empty_quotes() {
        let quotes_bid: AHashMap<String, f64> = AHashMap::new();
        let quotes_ask: AHashMap<String, f64> = AHashMap::new();
        let result = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        );
        assert!(result.is_err());
    }

    #[rstest]
    fn test_unequal_quotes_length() {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();

        quotes_bid.insert("EUR/USD".to_string(), 1.1000);
        quotes_bid.insert("GBP/USD".to_string(), 1.3000);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);
        // Missing GBP/USD in ask quotes.

        let result = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        );
        assert!(result.is_err());
    }

    #[rstest]
    fn test_invalid_price_type() {
        let (quotes_bid, quotes_ask) = setup_test_quotes();
        // 使用无效的价格类型变体（假设不支持 PriceType::Last）
        let result = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Last,
            quotes_bid,
            quotes_ask,
        );
        assert!(result.is_err());
    }

    #[rstest]
    fn test_cycle_handling() {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();
        // Create a cycle by including both EUR/USD and USD/EUR quotes
        quotes_bid.insert("EUR/USD".to_string(), 1.1);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);
        quotes_bid.insert("USD/EUR".to_string(), 0.909);
        quotes_ask.insert("USD/EUR".to_string(), 0.9091);

        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();

        // Expect the direct EUR/USD mid rate
        let expected = f64::midpoint(1.1, 1.1002);
        assert!((rate.unwrap() - expected).abs() < 0.0001);
    }

    #[rstest]
    fn test_multiple_paths() {
        let mut quotes_bid = AHashMap::new();
        let mut quotes_ask = AHashMap::new();
        // Direct conversion
        quotes_bid.insert("EUR/USD".to_string(), 1.1000);
        quotes_ask.insert("EUR/USD".to_string(), 1.1002);
        // Indirect path via GBP: EUR/GBP and GBP/USD
        quotes_bid.insert("EUR/GBP".to_string(), 0.8461);
        quotes_ask.insert("EUR/GBP".to_string(), 0.8463);
        quotes_bid.insert("GBP/USD".to_string(), 1.3000);
        quotes_ask.insert("GBP/USD".to_string(), 1.3002);

        let rate = get_exchange_rate(
            Ustr::from("EUR"),
            Ustr::from("USD"),
            PriceType::Mid,
            quotes_bid,
            quotes_ask,
        )
        .unwrap();

        // Both paths should be consistent:
        let direct: f64 = f64::midpoint(1.1000_f64, 1.1002_f64);
        let indirect: f64 =
            f64::midpoint(0.8461_f64, 0.8463_f64) * f64::midpoint(1.3000_f64, 1.3002_f64);
        assert!((direct - indirect).abs() < 0.0001_f64);
        assert!((rate.unwrap() - direct).abs() < 0.0001_f64);
    }
}
