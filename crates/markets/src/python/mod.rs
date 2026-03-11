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

//! `nautilus_markets` 的 Python 绑定入口（基于 `PyO3`）。

use pyo3::prelude::*;

/// 以 `nautilus_pyo3.markets` 模块名加载。
///
/// # 错误
///
/// 当任一类型或函数注册失败时返回 `PyErr`。
#[pymodule]
#[rustfmt::skip]
pub fn markets(_: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<nautilus_markets_ashare::market::price_limits::BoardType>()?;
    m.add_class::<nautilus_markets_ashare::market::price_limits::StockStatus>()?;
    m.add_class::<nautilus_markets_ashare::AShareSessionProvider>()?;
    m.add_class::<nautilus_markets_ashare::T1Ledger>()?;
    m.add_class::<nautilus_markets_ashare::TradingPhase>()?;

    // A 股导出项
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::market::price_limits::compute_ashare_price_limits,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::market::price_limits::compute_ashare_price_limits_by_board_status,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::compute_ashare_price_cage_violation,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::compute_ashare_price_cage_violation_by_phase,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::compute_ashare_lot_size_violation,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::compute_ashare_price_limit_violation,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::can_ashare_submit_order,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::can_ashare_cancel_order,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::is_ashare_trading_suspended,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::is_ashare_trading_resumed,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::calculate_ashare_sellable_quantity,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::identify_ashare_board_type,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::identify_ashare_stock_status,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::identify_ashare_board_type_from_str,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(
        nautilus_markets_ashare::risk::python::identify_ashare_stock_status_from_str,
        m
    )?)?;

    Ok(())
}

