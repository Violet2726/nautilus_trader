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

#[cfg(feature = "python")]
use nautilus_model::types::Price;
#[cfg(feature = "python")]
use crate::market::session::TradingPhase;
#[cfg(feature = "python")]
use crate::risk::checks;

#[cfg(feature = "python")]
use pyo3::pyfunction;

#[cfg(feature = "python")]
#[pyfunction]
pub fn compute_ashare_price_cage_violation(
    is_buy: bool,
    order_price: Price,
    current_price: Option<Price>,
    pct: f64,
    tick_size: Option<Price>,
) -> Option<Price> {
    let cp = current_price?;
    let ts = tick_size.unwrap_or_else(|| Price::new(0.01, 2));
    checks::check_price_cage_violation(is_buy, order_price, cp, pct, ts)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn compute_ashare_price_cage_violation_by_phase(
    phase: TradingPhase,
    is_buy: bool,
    order_price: Price,
    best_bid: Option<Price>,
    last_trade: Option<Price>,
    pct: f64,
    tick_size: Option<Price>,
) -> Option<Price> {
    if !matches!(phase, TradingPhase::ContinuousAm | TradingPhase::ContinuousPm) {
        return None;
    }
    let cp = best_bid.or(last_trade)?;
    let ts = tick_size.unwrap_or_else(|| Price::new(0.01, 2));
    checks::check_price_cage_violation(is_buy, order_price, cp, pct, ts)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn compute_ashare_lot_size_violation(
    symbol: &str,
    is_buy: bool,
    qty: f64,
    sellable: Option<f64>,
) -> Option<String> {
    checks::check_ashare_lot_size_violation(symbol, is_buy, qty, sellable)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn compute_ashare_price_limit_violation(
    order_price: Price,
    tick_size: Option<Price>,
    max_price: Option<Price>,
    min_price: Option<Price>,
) -> Option<String> {
    checks::check_ashare_price_limit_violation(order_price, tick_size, max_price, min_price)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn can_ashare_submit_order(phase: TradingPhase) -> bool {
    checks::can_ashare_submit_order(phase)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn can_ashare_cancel_order(phase: TradingPhase) -> bool {
    checks::can_ashare_cancel_order(phase)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn is_ashare_trading_suspended(action: nautilus_model::enums::MarketStatusAction) -> bool {
    checks::is_ashare_trading_suspended(action)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn is_ashare_trading_resumed(action: nautilus_model::enums::MarketStatusAction) -> bool {
    checks::is_ashare_trading_resumed(action)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn calculate_ashare_sellable_quantity(total: f64, today_buy: f64) -> f64 {
    checks::calculate_ashare_sellable_quantity(total, today_buy)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn identify_ashare_board_type(symbol: &str) -> crate::market::price_limits::BoardType {
    checks::identify_board_type(symbol)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn identify_ashare_stock_status(symbol: &str, name: &str) -> crate::market::price_limits::StockStatus {
    checks::identify_stock_status(symbol, name)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn identify_ashare_board_type_from_str(board: &str) -> crate::market::price_limits::BoardType {
    checks::identify_board_type_from_str(board)
}

#[cfg(feature = "python")]
#[pyfunction]
pub fn identify_ashare_stock_status_from_str(status: &str) -> crate::market::price_limits::StockStatus {
    checks::identify_stock_status_from_str(status)
}
