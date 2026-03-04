use nautilus_model::{
    enums::OrderSide,
    types::{Price, price::PriceRaw},
};

/// 价格笼子校验结果
#[derive(Debug)]
pub enum PriceCageResult {
    Pass,
    Violation {
        price: Price,
        cage_bound: Price,
        side: OrderSide,
    },
}

/// 计算价格笼子边界
///
/// - `side`: 订单方向
/// - `best_bid` / `best_ask`: 当前买一/卖一
/// - `last_trade`: 最新成交价
/// - `prev_close`: 前收盘价
/// - `tick`: 最小变动单位
/// - `pct`: 笼子百分比（如 0.02 = 2%）
pub fn compute_price_cage(
    side: OrderSide,
    best_bid: Option<Price>,
    best_ask: Option<Price>,
    last_trade: Option<Price>,
    prev_close: Price,
    tick: Price,
    pct: f64,
) -> Option<Price> {
    match side {
        OrderSide::Buy => {
            let benchmark = best_ask.or(best_bid).or(last_trade).unwrap_or(prev_close);

            let cage_raw = ((benchmark.as_f64() * (1.0 + pct)) * 1e9) as PriceRaw;
            let tick_raw = tick.raw;
            if tick_raw == 0 {
                return Some(benchmark);
            }
            let cage_aligned = (cage_raw / tick_raw) * tick_raw; // 向下取整
            Some(Price::from_raw(cage_aligned, benchmark.precision))
        }
        OrderSide::Sell => {
            let benchmark = best_bid.or(best_ask).or(last_trade).unwrap_or(prev_close);

            let cage_raw = ((benchmark.as_f64() * (1.0 - pct)) * 1e9) as PriceRaw;
            let tick_raw = tick.raw;
            if tick_raw == 0 {
                return Some(benchmark);
            }
            let cage_aligned = ((cage_raw + tick_raw - 1) / tick_raw) * tick_raw; // 向上取整
            Some(Price::from_raw(cage_aligned, benchmark.precision))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::types::Price;

    #[test]
    fn test_compute_price_cage() {
        let best_bid = Some(Price::new(10.0, 2));
        let best_ask = Some(Price::new(10.1, 2));
        let last_trade = Some(Price::new(10.05, 2));
        let prev_close = Price::new(9.9, 2);
        let tick = Price::new(0.01, 2);

        // 买入方向：基准价 = 卖一价 (10.1)
        // 价格笼子上限 = 10.1 * 1.02 = 10.302 -> 10.30
        let buy_bound = compute_price_cage(
            OrderSide::Buy,
            best_bid,
            best_ask,
            last_trade,
            prev_close,
            tick,
            0.02,
        );
        assert_eq!(buy_bound.unwrap().as_f64(), 10.30);

        // 卖出方向：基准价 = 买一价 (10.0)
        // 价格笼子下限 = 10.0 * 0.98 = 9.8 -> 9.80
        let sell_bound = compute_price_cage(
            OrderSide::Sell,
            best_bid,
            best_ask,
            last_trade,
            prev_close,
            tick,
            0.02,
        );
        assert_eq!(sell_bound.unwrap().as_f64(), 9.80);

        // 无买一或卖一，买入方向使用最新成交价
        let buy_bound_last = compute_price_cage(
            OrderSide::Buy,
            None,
            None,
            last_trade,
            prev_close,
            tick,
            0.02,
        );
        // 10.05 * 1.02 = 10.251 -> 10.25
        assert_eq!(buy_bound_last.unwrap().as_f64(), 10.25);
    }
}
