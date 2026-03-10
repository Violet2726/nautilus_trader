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

use nautilus_model::types::Price;
use crate::market::session::TradingPhase;

/// A股价格笼子相关计算结果
#[derive(Debug, Clone, Copy)]
pub struct PriceCageBounds {
    pub limit_up: Price,
    pub limit_down: Price,
}

/// 计算 A 股价格笼子上下界。
///
/// 对于主板/创业板/科创板，进入连续竞价后通常有 ±2% 的价格笼子限制。
pub fn calculate_price_cage_bounds(
    current_price: Price,
    pct: f64,
    tick_size: Price,
) -> PriceCageBounds {
    let limit_up = current_price.as_f64() * (1.0 + pct);
    let limit_down = current_price.as_f64() * (1.0 - pct);

    PriceCageBounds {
        limit_up: Price::new(limit_up, tick_size.precision),
        limit_down: Price::new(limit_down, tick_size.precision),
    }
}

/// 检查是否违反 A 股价格笼子限制。
///
/// 返回 None 表示没有违反，返回 Some(Price) 表示违反，并给出具体的限制界限。
pub fn check_price_cage_violation(
    is_buy: bool,
    order_price: Price,
    current_price: Price,
    pct: f64,
    tick_size: Price,
) -> Option<Price> {
    let bounds = calculate_price_cage_bounds(current_price, pct, tick_size);
    if is_buy && order_price > bounds.limit_up {
        return Some(bounds.limit_up);
    }
    if !is_buy && order_price < bounds.limit_down {
        return Some(bounds.limit_down);
    }
    None
}

/// 检查是否违反 A 股手数规则。
///
/// 逻辑：
/// - 主板：买入 100 股起，100 的整数倍；卖出可卖余额不足 100 股时必须一次性卖出。
/// - 科创板：买入 200 股起，1 股递增；不足 200 股时必须一次性卖出。
/// - 创业板：买入 100 股起，1 股递增；不足 100 股时必须一次性卖出。
pub fn check_ashare_lot_size_violation(
    symbol: &str,
    is_buy: bool,
    qty: f64,
    sellable: Option<f64>,
) -> Option<String> {
    let board = crate::market::price_limits::identify_board_type(symbol);
    let min_qty = match board {
        crate::market::price_limits::BoardType::Star => 200.0,
        _ => 100.0,
    };

    if is_buy {
        if qty < min_qty {
            let reason = match board {
                crate::market::price_limits::BoardType::Star => "STAR_MARKET_BUY_VIOLATION",
                crate::market::price_limits::BoardType::ChiNext => "CHINEXT_BUY_VIOLATION",
                _ => "MAIN_BOARD_BUY_VIOLATION",
            };
            return Some(format!(
                "{}: qty={:.2} less than min_qty={:.2}",
                reason, qty, min_qty
            ));
        }
        // 只有主板要求 100 的整数倍
        if matches!(board, crate::market::price_limits::BoardType::Main) && (qty % 100.0).abs() > f64::EPSILON {
            return Some(format!(
                "MAIN_BOARD_BUY_VIOLATION: qty={:.2} not multiple of 100",
                qty
            ));
        }
    } else {
        // 卖出检查
        let is_odd_lot = if matches!(
            board,
            crate::market::price_limits::BoardType::Star | crate::market::price_limits::BoardType::ChiNext
        ) {
            qty < min_qty
        } else {
            (qty % 100.0).abs() > f64::EPSILON
        };

        if is_odd_lot {
            if let Some(s) = sellable {
                if (qty - s).abs() > 0.0001 {
                    return Some(format!(
                        "ODD_LOT_VIOLATION: qty={:.2} must be full sellable={:.2}",
                        qty, s
                    ));
                }
            }
        }
    }
    None
}

/// 检查 A 股价格限制（涨跌停及 Tick 对齐）。
pub fn check_ashare_price_limit_violation(
    order_price: Price,
    tick_size: Option<Price>,
    max_price: Option<Price>,
    min_price: Option<Price>,
) -> Option<String> {
    if let Some(tick) = tick_size {
        if tick.raw > 0 && (order_price.raw % tick.raw) != 0 {
            return Some(format!(
                "PRICE_NOT_ON_TICK: price={} not multiple of tick={}",
                order_price, tick
            ));
        }
    }
    if let Some(max_p) = max_price {
        if order_price > max_p {
            return Some(format!(
                "PRICE_ABOVE_UP_LIMIT: price={} > limit_up={}",
                order_price, max_p
            ));
        }
    }
    if let Some(min_p) = min_price {
        if order_price < min_p {
            return Some(format!(
                "PRICE_BELOW_DOWN_LIMIT: price={} < limit_down={}",
                order_price, min_p
            ));
        }
    }
    None
}

/// 检查 A 股当前阶段是否允许下单。
pub fn can_ashare_submit_order(phase: TradingPhase) -> bool {
    phase.can_accept_order()
}

/// 检查 A 股当前阶段是否允许撤单。
pub fn can_ashare_cancel_order(phase: TradingPhase) -> bool {
    phase.can_cancel_order()
}

/// 检查 A 股标的是否处于停牌或不可交易状态。
pub fn is_ashare_trading_suspended(action: nautilus_model::enums::MarketStatusAction) -> bool {
    use nautilus_model::enums::MarketStatusAction;
    matches!(
        action,
        MarketStatusAction::Halt
            | MarketStatusAction::Suspend
            | MarketStatusAction::NotAvailableForTrading
    )
}

/// 检查 A 股标的是否复牌。
pub fn is_ashare_trading_resumed(action: nautilus_model::enums::MarketStatusAction) -> bool {
    use nautilus_model::enums::MarketStatusAction;
    matches!(action, MarketStatusAction::Resumed)
}

/// 计算 A 股 T+1 制度下的可卖数量。
pub fn calculate_ashare_sellable_quantity(total: f64, today_buy: f64) -> f64 {
    (total - today_buy).max(0.0)
}

/// 检查 A 股 T+1 卖出违规。
pub fn check_ashare_t1_violation(order_qty: f64, sellable: f64) -> Option<String> {
    if order_qty > sellable {
        Some(format!(
            "EXCEEDS_SELLABLE: qty={:.0}, sellable={:.0}",
            order_qty, sellable
        ))
    } else {
        None
    }
}

// 重新导出板块和状态识别逻辑，使其在风险模块也易于访问
pub use crate::market::price_limits::{identify_board_type, identify_stock_status, BoardType, StockStatus};

/// 从字符串识别板块类型。
pub fn identify_board_type_from_str(board: &str) -> BoardType {
    match board.to_uppercase().as_str() {
        "MAIN" => BoardType::Main,
        "GEM" | "CHINEXT" => BoardType::ChiNext,
        "STAR" => BoardType::Star,
        "BSE" | "BJSE" => BoardType::BSE,
        _ => BoardType::Main,
    }
}

/// 从字符串识别股票状态。
pub fn identify_stock_status_from_str(status: &str) -> StockStatus {
    match status.to_uppercase().as_str() {
        "NORMAL" => StockStatus::Normal,
        "ST" => StockStatus::ST,
        "STARST" | "*ST" => StockStatus::StarST,
        "IPO5" => StockStatus::IPO5,
        _ => StockStatus::Normal,
    }
}
