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

use nautilus_model::types::{Price, price::PriceRaw};

/// A股板块类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common",
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum BoardType {
    /// 主板（上海60xx，深圳00xx）
    Main,
    /// 科创板（688xxx）
    Star,
    /// 创业板（30xxxx）
    ChiNext,
    /// 北交所（8xxxx，43xxxx）
    BSE,
}

/// A股股票状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen,
        eq,
        eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common",
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum StockStatus {
    /// 正常股票
    Normal,
    /// ST股票
    ST,
    /// *ST股票（存在退市风险）
    StarST,
    /// 科创板上市前5日
    IPO5,
}

/// 涨跌停限制配置
#[derive(Debug, Clone)]
pub struct PriceLimitConfig {
    /// 涨幅限制（例如0.10表示10%）
    pub limit_up_pct: f64,
    /// 跌幅限制（例如0.10表示10%）
    pub limit_down_pct: f64,
}

impl PriceLimitConfig {
    pub fn new(limit_up_pct: f64, limit_down_pct: f64) -> Self {
        Self {
            limit_up_pct,
            limit_down_pct,
        }
    }
}

/// 涨跌停价格计算结果
#[derive(Debug, Clone)]
pub struct PriceLimits {
    /// 涨停价（None表示无限制）
    pub limit_up: Option<Price>,
    /// 跌停价（None表示无限制）
    pub limit_down: Option<Price>,
}

impl PriceLimits {
    pub fn new(limit_up: Option<Price>, limit_down: Option<Price>) -> Self {
        Self { limit_up, limit_down }
    }
}

/// 根据股票代码识别板块类型
pub fn identify_board_type(symbol: &str) -> BoardType {
    if symbol.starts_with("688") {
        BoardType::Star
    } else if symbol.starts_with("30") {
        BoardType::ChiNext
    } else if symbol.starts_with("8") || symbol.starts_with("43") {
        BoardType::BSE
    } else if symbol.starts_with("60") || symbol.starts_with("00") {
        BoardType::Main
    } else {
        // 默认按主板处理
        BoardType::Main
    }
}

/// 根据股票名称识别状态
pub fn identify_stock_status(_symbol: &str, name: &str) -> StockStatus {
    // 检查ST状态
    if name.contains("*ST") {
        StockStatus::StarST
    } else if name.contains("ST") {
        StockStatus::ST
    } else {
        StockStatus::Normal
    }
}

/// 获取涨跌停限制配置
pub fn get_price_limit_config(board: BoardType, status: StockStatus) -> Option<PriceLimitConfig> {
    match (board, status) {
        // 主板正常股票：±10%
        (BoardType::Main, StockStatus::Normal) => {
            Some(PriceLimitConfig::new(0.10, 0.10))
        }
        // 主板ST股票：±5%
        (BoardType::Main, StockStatus::ST) => {
            Some(PriceLimitConfig::new(0.05, 0.05))
        }
        // 主板*ST股票：±5%
        (BoardType::Main, StockStatus::StarST) => {
            Some(PriceLimitConfig::new(0.05, 0.05))
        }
        // 创业板：±20%
        (BoardType::ChiNext, StockStatus::Normal) => {
            Some(PriceLimitConfig::new(0.20, 0.20))
        }
        // 科创板正常股票：±20%
        (BoardType::Star, StockStatus::Normal) => {
            Some(PriceLimitConfig::new(0.20, 0.20))
        }
        // 科创板上市前5日：无涨跌幅限制
        (BoardType::Star, StockStatus::IPO5) => {
            None
        }
        // 北交所：±30%
        (BoardType::BSE, StockStatus::Normal) => {
            Some(PriceLimitConfig::new(0.30, 0.30))
        }
        // 其他情况默认主板限制
        _ => Some(PriceLimitConfig::new(0.10, 0.10)),
    }
}

/// 计算涨跌停价格
///
/// # 参数
///
/// * `prev_close` - 前收盘价
/// * `tick_size` - 最小价格变动单位
/// * `config` - 涨跌停配置
///
/// # 返回值
///
/// 返回涨跌停价格，如果配置为None则表示无涨跌幅限制
pub fn compute_price_limits(
    prev_close: Price,
    tick_size: Price,
    config: Option<PriceLimitConfig>,
) -> PriceLimits {
    if let Some(limit_config) = config {
        let prev_close_raw = prev_close.raw;
        let tick_raw = tick_size.raw;
        
        if tick_raw == 0 {
            // 如果tick为0，返回原价
            return PriceLimits::new(Some(prev_close), Some(prev_close));
        }

        // 计算涨停价：向上取整到tick
        let limit_up_raw = ((prev_close_raw as f64 * (1.0 + limit_config.limit_up_pct) / tick_raw as f64).ceil() as i64) * tick_raw;
        let limit_up = Price::from_raw(limit_up_raw as PriceRaw, prev_close.precision);

        // 计算跌停价：向下取整到tick
        let limit_down_raw = ((prev_close_raw as f64 * (1.0 - limit_config.limit_down_pct) / tick_raw as f64).floor() as i64) * tick_raw;
        let limit_down = Price::from_raw(limit_down_raw as PriceRaw, prev_close.precision);

        PriceLimits::new(Some(limit_up), Some(limit_down))
    } else {
        // 无涨跌幅限制
        PriceLimits::new(None, None)
    }
}

/// 便捷函数：根据股票信息计算涨跌停价格
pub fn compute_price_limits_for_stock(
    symbol: &str,
    name: &str,
    prev_close: Price,
    tick_size: Price,
) -> PriceLimits {
    let board = identify_board_type(symbol);
    let status = identify_stock_status(symbol, name);
    let config = get_price_limit_config(board, status);
    compute_price_limits(prev_close, tick_size, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_price(value: f64) -> Price {
        Price::new(value, 2)
    }

    #[test]
    fn test_identify_board_type() {
        assert_eq!(identify_board_type("600000"), BoardType::Main);
        assert_eq!(identify_board_type("000001"), BoardType::Main);
        assert_eq!(identify_board_type("688001"), BoardType::Star);
        assert_eq!(identify_board_type("300001"), BoardType::ChiNext);
        assert_eq!(identify_board_type("430001"), BoardType::BSE);
        assert_eq!(identify_board_type("800001"), BoardType::BSE);
    }

    #[test]
    fn test_identify_stock_status() {
        assert_eq!(identify_stock_status("600000", "平安银行"), StockStatus::Normal);
        assert_eq!(identify_stock_status("600000", "ST平安"), StockStatus::ST);
        assert_eq!(identify_stock_status("600000", "*ST平安"), StockStatus::StarST);
    }

    #[test]
    fn test_get_price_limit_config() {
        // 主板正常股票
        let config = get_price_limit_config(BoardType::Main, StockStatus::Normal).unwrap();
        assert_eq!(config.limit_up_pct, 0.10);
        assert_eq!(config.limit_down_pct, 0.10);

        // 主板ST股票
        let config = get_price_limit_config(BoardType::Main, StockStatus::ST).unwrap();
        assert_eq!(config.limit_up_pct, 0.05);
        assert_eq!(config.limit_down_pct, 0.05);

        // 创业板
        let config = get_price_limit_config(BoardType::ChiNext, StockStatus::Normal).unwrap();
        assert_eq!(config.limit_up_pct, 0.20);
        assert_eq!(config.limit_down_pct, 0.20);

        // 科创板上市前5日
        let config = get_price_limit_config(BoardType::Star, StockStatus::IPO5);
        assert!(config.is_none());

        // 北交所
        let config = get_price_limit_config(BoardType::BSE, StockStatus::Normal).unwrap();
        assert_eq!(config.limit_up_pct, 0.30);
        assert_eq!(config.limit_down_pct, 0.30);
    }

    #[test]
    fn test_compute_price_limits() {
        let prev_close = create_price(10.00);
        let tick_size = create_price(0.01);
        let config = Some(PriceLimitConfig::new(0.10, 0.10));

        let limits = compute_price_limits(prev_close, tick_size, config);
        
        // 涨停价：10.00 * 1.10 = 11.00
        assert_eq!(limits.limit_up.unwrap().as_f64(), 11.00);
        // 跌停价：10.00 * 0.90 = 9.00
        assert_eq!(limits.limit_down.unwrap().as_f64(), 9.00);
    }

    #[test]
    fn test_compute_price_limits_no_limits() {
        let prev_close = create_price(10.00);
        let tick_size = create_price(0.01);
        let config = None;

        let limits = compute_price_limits(prev_close, tick_size, config);
        
        assert!(limits.limit_up.is_none());
        assert!(limits.limit_down.is_none());
    }

    #[test]
    fn test_compute_price_limits_for_stock() {
        let prev_close = create_price(10.00);
        let tick_size = create_price(0.01);

        // 主板正常股票
        let limits = compute_price_limits_for_stock("600000", "平安银行", prev_close, tick_size);
        assert_eq!(limits.limit_up.unwrap().as_f64(), 11.00);
        assert_eq!(limits.limit_down.unwrap().as_f64(), 9.00);

        // 主板ST股票
        let limits = compute_price_limits_for_stock("600000", "ST平安", prev_close, tick_size);
        assert_eq!(limits.limit_up.unwrap().as_f64(), 10.50);
        assert_eq!(limits.limit_down.unwrap().as_f64(), 9.50);

        // 创业板
        let limits = compute_price_limits_for_stock("300001", "特锐德", prev_close, tick_size);
        assert_eq!(limits.limit_up.unwrap().as_f64(), 12.00);
        assert_eq!(limits.limit_down.unwrap().as_f64(), 8.00);
    }

    #[test]
    fn test_price_alignment() {
        // 测试价格对齐到tick
        let prev_close = create_price(10.13);
        let tick_size = create_price(0.01);
        let config = Some(PriceLimitConfig::new(0.10, 0.10));

        let limits = compute_price_limits(prev_close, tick_size, config);
        
        // 涨停价：10.13 * 1.10 = 11.143 -> 向上取整到11.15
        assert_eq!(limits.limit_up.unwrap().as_f64(), 11.15);
        // 跌停价：10.13 * 0.90 = 9.117 -> 向下取整到9.11
        assert_eq!(limits.limit_down.unwrap().as_f64(), 9.11);
    }
}
