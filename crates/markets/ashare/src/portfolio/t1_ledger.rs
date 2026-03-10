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

//! A股T+1持仓管理
//!
//! 提供A股市场的T+1交收账本功能，包括：
//! - T+1持仓条目管理
//! - 可卖出数量计算
//! - 成交回调处理
//! - 日切结算处理

use ahash::AHashMap;
use nautilus_model::{
    enums::OrderSide,
    identifiers::{AccountId, InstrumentId},
};

#[cfg(feature = "python")]
use pyo3::{exceptions::PyValueError, prelude::*};

/// 单个标的T+1持仓条目。
///
/// 跟踪标的的总数量和今日买入数量。
/// 可卖数量计算为 `total_qty - today_buy_qty`。
#[derive(Debug, Clone)]
pub struct T1Entry {
    pub total_qty: f64,
    pub today_buy_qty: f64,
}

impl T1Entry {
    #[must_use]
    pub fn sellable(&self) -> f64 {
        crate::risk::checks::calculate_ashare_sellable_quantity(self.total_qty, self.today_buy_qty)
    }
}

/// T+1持仓状态机
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "python", pyo3::pyclass(from_py_object, module = "nautilus_trader.core.nautilus_pyo3.markets"))]
pub struct T1Ledger {
    entries: AHashMap<(AccountId, InstrumentId), T1Entry>,
}

impl T1Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    /// 实盘启动时从券商加载初始持仓
    pub fn load_position(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        total_qty: f64,
        today_buy_qty: f64,
    ) {
        self.entries.insert(
            (account_id, instrument_id),
            T1Entry {
                total_qty,
                today_buy_qty,
            },
        );
    }

    /// 成交回调：更新持仓
    pub fn on_fill(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        side: OrderSide,
        fill_qty: f64,
    ) {
        let entry = self
            .entries
            .entry((account_id, instrument_id))
            .or_insert(T1Entry {
                total_qty: 0.0,
                today_buy_qty: 0.0,
            });

        match side {
            OrderSide::Buy => {
                entry.total_qty += fill_qty;
                entry.today_buy_qty += fill_qty;
            }
            OrderSide::Sell => {
                entry.total_qty = (entry.total_qty - fill_qty).max(0.0);
            }
            _ => {}
        }
    }

    /// 查询可卖数量
    #[must_use]
    pub fn sellable(&self, account_id: &AccountId, instrument_id: &InstrumentId) -> f64 {
        self.entries
            .get(&(*account_id, *instrument_id))
            .map_or(0.0, |e| e.sellable())
    }

    /// 日切/结算：释放今日买入量到可卖
    pub fn on_settlement(&mut self) {
        for entry in self.entries.values_mut() {
            entry.today_buy_qty = 0.0;
        }
    }

    /// 重置所有条目
    pub fn reset(&mut self) {
        self.entries.clear();
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl T1Ledger {
    #[new]
    pub fn py_new() -> Self {
        Self::new()
    }

    #[pyo3(name = "load_position")]
    pub fn py_load_position(
        &mut self,
        account_id: &str,
        instrument_id: &str,
        total_qty: f64,
        today_buy_qty: f64,
    ) -> PyResult<()> {
        let account_id = normalize_account_id(Some(account_id))?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        self.load_position(account_id, instrument_id, total_qty, today_buy_qty);
        Ok(())
    }

    #[pyo3(name = "on_fill")]
    pub fn py_on_fill(
        &mut self,
        account_id: &str,
        instrument_id: &str,
        is_buy: bool,
        fill_qty: f64,
    ) -> PyResult<()> {
        let account_id = normalize_account_id(Some(account_id))?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        let side = if is_buy { OrderSide::Buy } else { OrderSide::Sell };
        self.on_fill(account_id, instrument_id, side, fill_qty);
        Ok(())
    }

    #[pyo3(name = "sellable")]
    pub fn py_sellable(&self, account_id: &str, instrument_id: &str) -> PyResult<f64> {
        let account_id = normalize_account_id(Some(account_id))?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        Ok(self.sellable(&account_id, &instrument_id))
    }

    #[pyo3(name = "sellable_or_none")]
    pub fn py_sellable_or_none(
        &self,
        account_id: &str,
        instrument_id: &str,
    ) -> PyResult<Option<f64>> {
        let account_id = normalize_account_id(Some(account_id))?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        Ok(self
            .entries
            .get(&(account_id, instrument_id))
            .map(|e| e.sellable()))
    }

    #[pyo3(name = "load_position_opt")]
    pub fn py_load_position_opt(
        &mut self,
        account_id: Option<&str>,
        instrument_id: &str,
        total_qty: f64,
        today_buy_qty: f64,
    ) -> PyResult<()> {
        let account_id = normalize_account_id(account_id)?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        self.load_position(account_id, instrument_id, total_qty, today_buy_qty);
        Ok(())
    }

    #[pyo3(name = "on_fill_opt")]
    pub fn py_on_fill_opt(
        &mut self,
        account_id: Option<&str>,
        instrument_id: &str,
        is_buy: bool,
        fill_qty: f64,
    ) -> PyResult<()> {
        let account_id = normalize_account_id(account_id)?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        let side = if is_buy { OrderSide::Buy } else { OrderSide::Sell };
        self.on_fill(account_id, instrument_id, side, fill_qty);
        Ok(())
    }

    #[pyo3(name = "sellable_or_none_opt")]
    pub fn py_sellable_or_none_opt(
        &self,
        account_id: Option<&str>,
        instrument_id: &str,
    ) -> PyResult<Option<f64>> {
        let account_id = normalize_account_id(account_id)?;
        let instrument_id = parse_instrument_id(instrument_id)?;
        Ok(self
            .entries
            .get(&(account_id, instrument_id))
            .map(|e| e.sellable()))
    }

    #[pyo3(name = "on_settlement")]
    pub fn py_on_settlement(&mut self) {
        self.on_settlement();
    }

    #[pyo3(name = "reset")]
    pub fn py_reset(&mut self) {
        self.reset();
    }
}

#[cfg(feature = "python")]
fn normalize_account_id(value: Option<&str>) -> PyResult<AccountId> {
    let value = value.unwrap_or("__no_account__");
    let normalized = if value == "__no_account__" || value == "__NO_ACCOUNT__" {
        "NA-__NO_ACCOUNT__".to_string()
    } else if value.contains('-') {
        value.to_string()
    } else {
        format!("NA-{value}")
    };
    AccountId::new_checked(&normalized).map_err(|err| {
        PyValueError::new_err(format!(
            "Invalid account_id '{value}' (normalized='{normalized}'): {err}"
        ))
    })
}

#[cfg(feature = "python")]
fn parse_instrument_id(value: &str) -> PyResult<InstrumentId> {
    InstrumentId::from_as_ref(value)
        .map_err(|err| PyValueError::new_err(format!("Invalid instrument_id '{value}': {err}")))
}
#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::identifiers::{AccountId, InstrumentId};

    #[test]
    fn test_t1_ledger() {
        let mut ledger = T1Ledger::new();
        let account_id = AccountId::from("ACCOUNT-1");
        let instrument_id = InstrumentId::from("600000.XSHG");

        // 加载 1000 股
        ledger.load_position(account_id, instrument_id, 1000.0, 0.0);
        assert_eq!(ledger.sellable(&account_id, &instrument_id), 1000.0);

        // 今日买入 500 股
        ledger.on_fill(account_id, instrument_id, OrderSide::Buy, 500.0);
        // 可卖数量仍应为 1000，但总持仓为 1500
        assert_eq!(ledger.sellable(&account_id, &instrument_id), 1000.0);
        assert_eq!(
            ledger
                .entries
                .get(&(account_id, instrument_id))
                .unwrap()
                .total_qty,
            1500.0
        );

        // 卖出 400 股
        ledger.on_fill(account_id, instrument_id, OrderSide::Sell, 400.0);
        // 可卖数量变为 600，总持仓为 1100
        assert_eq!(ledger.sellable(&account_id, &instrument_id), 600.0);
        assert_eq!(
            ledger
                .entries
                .get(&(account_id, instrument_id))
                .unwrap()
                .total_qty,
            1100.0
        );

        // 日切结算
        ledger.on_settlement();
        // 可卖数量变为 1100，总持仓为 1100
        assert_eq!(ledger.sellable(&account_id, &instrument_id), 1100.0);
        assert_eq!(
            ledger
                .entries
                .get(&(account_id, instrument_id))
                .unwrap()
                .today_buy_qty,
            0.0
        );
    }
}
