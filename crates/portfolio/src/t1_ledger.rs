use ahash::AHashMap;
use nautilus_model::{
    enums::OrderSide,
    identifiers::{AccountId, InstrumentId},
};

/// 单个标的的 T+1 持仓条目
#[derive(Debug, Clone)]
pub struct T1Entry {
    pub total_qty: f64,
    pub today_buy_qty: f64,
}

impl T1Entry {
    #[must_use]
    pub fn sellable(&self) -> f64 {
        (self.total_qty - self.today_buy_qty).max(0.0)
    }
}

/// T+1 持仓状态机
#[derive(Debug, Default)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::identifiers::{AccountId, InstrumentId, Venue};
    use ustr::ustr;

    #[test]
    fn test_t1_ledger() {
        let mut ledger = T1Ledger::new();
        let account_id = AccountId::new(ustr("ACCOUNT_1"));
        let instrument_id = InstrumentId::new(ustr("600000"), Venue::new(ustr("XSHG")));

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
