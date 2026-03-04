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

//! 提供 `Cache` 的数据库后端支持。

// 开发中
#![allow(dead_code)]
#![allow(unused_variables)]

use ahash::AHashMap;
use bytes::Bytes;
use nautilus_core::UnixNanos;
use nautilus_model::{
    accounts::AccountAny,
    data::{Bar, DataType, FundingRateUpdate, GreeksData, QuoteTick, TradeTick, YieldCurveData},
    events::{OrderEventAny, OrderSnapshot, position::snapshot::PositionSnapshot},
    identifiers::{
        AccountId, ClientId, ClientOrderId, ComponentId, InstrumentId, PositionId, StrategyId,
        VenueOrderId,
    },
    instruments::{InstrumentAny, SyntheticInstrument},
    orderbook::OrderBook,
    orders::OrderAny,
    position::Position,
    types::Currency,
};
use ustr::Ustr;

use crate::{custom::CustomData, signal::Signal};

#[derive(Debug, Default)]
pub struct CacheMap {
    pub currencies: AHashMap<Ustr, Currency>,
    pub instruments: AHashMap<InstrumentId, InstrumentAny>,
    pub synthetics: AHashMap<InstrumentId, SyntheticInstrument>,
    pub accounts: AHashMap<AccountId, AccountAny>,
    pub orders: AHashMap<ClientOrderId, OrderAny>,
    pub positions: AHashMap<PositionId, Position>,
    pub greeks: AHashMap<InstrumentId, GreeksData>,
    pub yield_curves: AHashMap<String, YieldCurveData>,
}

#[async_trait::async_trait]
pub trait CacheDatabaseAdapter {
    /// 关闭缓存数据库连接。
    ///
    /// # 错误
    ///
    /// 如果数据库未能正常关闭，则返回错误。
    fn close(&mut self) -> anyhow::Result<()>;

    /// 将任何挂起的更改刷新到数据库。
    ///
    /// # 错误
    ///
    /// 如果刷新更改失败，则返回错误。
    fn flush(&mut self) -> anyhow::Result<()>;

    /// 将所有缓存数据加载到内存中。
    ///
    /// # 错误
    ///
    /// 如果从数据库加载数据失败，则返回错误。
    async fn load_all(&self) -> anyhow::Result<CacheMap>;

    /// 从数据库加载原始键值数据。
    ///
    /// # 错误
    ///
    /// 如果加载操作失败，则返回错误。
    fn load(&self) -> anyhow::Result<AHashMap<String, Bytes>>;

    /// 从缓存中加载所有货币。
    ///
    /// # 错误
    ///
    /// 如果加载货币失败，则返回错误。
    async fn load_currencies(&self) -> anyhow::Result<AHashMap<Ustr, Currency>>;

    /// 从缓存中加载所有金融工具。
    ///
    /// # 错误
    ///
    /// 如果加载金融工具失败，则返回错误。
    async fn load_instruments(&self) -> anyhow::Result<AHashMap<InstrumentId, InstrumentAny>>;

    /// 从缓存中加载所有合成金融工具。
    ///
    /// # 错误
    ///
    /// 如果加载合成金融工具失败，则返回错误。
    async fn load_synthetics(&self) -> anyhow::Result<AHashMap<InstrumentId, SyntheticInstrument>>;

    /// 从缓存中加载所有账户。
    ///
    /// # 错误
    ///
    /// 如果加载账户失败，则返回错误。
    async fn load_accounts(&self) -> anyhow::Result<AHashMap<AccountId, AccountAny>>;

    /// 从缓存中加载所有订单。
    ///
    /// # 错误
    ///
    /// 如果加载订单失败，则返回错误。
    async fn load_orders(&self) -> anyhow::Result<AHashMap<ClientOrderId, OrderAny>>;

    /// 从缓存中加载所有持仓。
    ///
    /// # 错误
    ///
    /// 如果加载持仓失败，则返回错误。
    async fn load_positions(&self) -> anyhow::Result<AHashMap<PositionId, Position>>;

    /// 从缓存中加载所有 [`GreeksData`]。
    ///
    /// # 错误
    ///
    /// 如果加载希腊字母数据失败，则返回错误。
    async fn load_greeks(&self) -> anyhow::Result<AHashMap<InstrumentId, GreeksData>> {
        Ok(AHashMap::new())
    }

    /// 从缓存中加载所有 [`YieldCurveData`]。
    ///
    /// # 错误
    ///
    /// 如果加载收益率曲线数据失败，则返回错误。
    async fn load_yield_curves(&self) -> anyhow::Result<AHashMap<String, YieldCurveData>> {
        Ok(AHashMap::new())
    }

    /// 加载订单 ID 到持仓 ID 的映射。
    ///
    /// # 错误
    ///
    /// 如果加载订单-持仓索引映射失败，则返回错误。
    fn load_index_order_position(&self) -> anyhow::Result<AHashMap<ClientOrderId, Position>>;

    /// 加载订单 ID 到客户端 ID 的映射。
    ///
    /// # 错误
    ///
    /// 如果加载订单-客户端索引映射失败，则返回错误。
    fn load_index_order_client(&self) -> anyhow::Result<AHashMap<ClientOrderId, ClientId>>;

    /// 根据代码加载单个货币。
    ///
    /// # 错误
    ///
    /// 如果加载单个货币失败，则返回错误。
    async fn load_currency(&self, code: &Ustr) -> anyhow::Result<Option<Currency>>;

    /// 根据 ID 加载单个金融工具。
    ///
    /// # 错误
    ///
    /// 如果加载单个金融工具失败，则返回错误。
    async fn load_instrument(
        &self,
        instrument_id: &InstrumentId,
    ) -> anyhow::Result<Option<InstrumentAny>>;

    /// 根据 ID 加载单个合成金融工具。
    ///
    /// # 错误
    ///
    /// 如果加载单个合成金融工具失败，则返回错误。
    async fn load_synthetic(
        &self,
        instrument_id: &InstrumentId,
    ) -> anyhow::Result<Option<SyntheticInstrument>>;

    /// 根据 ID 加载单个账户。
    ///
    /// # 错误
    ///
    /// 如果加载单个账户失败，则返回错误。
    async fn load_account(&self, account_id: &AccountId) -> anyhow::Result<Option<AccountAny>>;

    /// 根据客户订单 ID 加载单个订单。
    ///
    /// # 错误
    ///
    /// 如果加载单个订单失败，则返回错误。
    async fn load_order(&self, client_order_id: &ClientOrderId)
    -> anyhow::Result<Option<OrderAny>>;

    /// 根据持仓 ID 加载单个持仓。
    ///
    /// # 错误
    ///
    /// 如果加载单个持仓失败，则返回错误。
    async fn load_position(&self, position_id: &PositionId) -> anyhow::Result<Option<Position>>;

    /// 根据组件 ID 加载 Actor 状态。
    ///
    /// # 错误
    ///
    /// 如果加载 Actor 状态失败，则返回错误。
    fn load_actor(&self, component_id: &ComponentId) -> anyhow::Result<AHashMap<String, Bytes>>;

    /// 根据策略 ID 加载策略状态。
    ///
    /// # 错误
    ///
    /// 如果加载策略状态失败，则返回错误。
    fn load_strategy(&self, strategy_id: &StrategyId) -> anyhow::Result<AHashMap<String, Bytes>>;

    /// 根据名称加载信号。
    ///
    /// # 错误
    ///
    /// 如果加载信号失败，则返回错误。
    fn load_signals(&self, name: &str) -> anyhow::Result<Vec<Signal>>;

    /// 根据数据类型加载自定义数据。
    ///
    /// # 错误
    ///
    /// 如果加载自定义数据失败，则返回错误。
    fn load_custom_data(&self, data_type: &DataType) -> anyhow::Result<Vec<CustomData>>;

    /// 根据客户订单 ID 加载订单快照。
    ///
    /// # 错误
    ///
    /// 如果加载订单快照失败，则返回错误。
    fn load_order_snapshot(
        &self,
        client_order_id: &ClientOrderId,
    ) -> anyhow::Result<Option<OrderSnapshot>>;

    /// 根据持仓 ID 加载持仓快照。
    ///
    /// # 错误
    ///
    /// 如果加载持仓快照失败，则返回错误。
    fn load_position_snapshot(
        &self,
        position_id: &PositionId,
    ) -> anyhow::Result<Option<PositionSnapshot>>;

    /// 根据金融工具 ID 加载报价 Tick。
    ///
    /// # 错误
    ///
    /// 如果加载报价失败，则返回错误。
    fn load_quotes(&self, instrument_id: &InstrumentId) -> anyhow::Result<Vec<QuoteTick>>;

    /// 根据金融工具 ID 加载成交 Tick。
    ///
    /// # 错误
    ///
    /// 如果加载成交失败，则返回错误。
    fn load_trades(&self, instrument_id: &InstrumentId) -> anyhow::Result<Vec<TradeTick>>;

    /// 根据金融工具 ID 加载资金费率更新。
    ///
    /// # 错误
    ///
    /// 如果加载资金费率失败，则返回错误。
    fn load_funding_rates(
        &self,
        instrument_id: &InstrumentId,
    ) -> anyhow::Result<Vec<FundingRateUpdate>>;

    /// 根据金融工具 ID 加载 Bar 数据。
    ///
    /// # 错误
    ///
    /// 如果加载 Bar 数据失败，则返回错误。
    fn load_bars(&self, instrument_id: &InstrumentId) -> anyhow::Result<Vec<Bar>>;

    /// 向缓存添加一个通用的键值对。
    ///
    /// # 错误
    ///
    /// 如果添加通用的键/值失败，则返回错误。
    fn add(&self, key: String, value: Bytes) -> anyhow::Result<()>;

    /// 向缓存添加一种货币。
    ///
    /// # 错误
    ///
    /// 如果添加货币失败，则返回错误。
    fn add_currency(&self, currency: &Currency) -> anyhow::Result<()>;

    /// 向缓存添加一个金融工具。
    ///
    /// # 错误
    ///
    /// 如果添加金融工具失败，则返回错误。
    fn add_instrument(&self, instrument: &InstrumentAny) -> anyhow::Result<()>;

    /// 向缓存添加一个合成金融工具。
    ///
    /// # 错误
    ///
    /// 如果添加合成金融工具失败，则返回错误。
    fn add_synthetic(&self, synthetic: &SyntheticInstrument) -> anyhow::Result<()>;

    /// 向缓存添加一个账户。
    ///
    /// # 错误
    ///
    /// 如果添加账户失败，则返回错误。
    fn add_account(&self, account: &AccountAny) -> anyhow::Result<()>;

    /// 向缓存添加一个订单。
    ///
    /// # 错误
    ///
    /// 如果添加订单失败，则返回错误。
    fn add_order(&self, order: &OrderAny, client_id: Option<ClientId>) -> anyhow::Result<()>;

    /// 向缓存添加一个订单快照。
    ///
    /// # 错误
    ///
    /// 如果添加订单快照失败，则返回错误。
    fn add_order_snapshot(&self, snapshot: &OrderSnapshot) -> anyhow::Result<()>;

    /// 向缓存添加一个持仓。
    ///
    /// # 错误
    ///
    /// 如果添加持仓失败，则返回错误。
    fn add_position(&self, position: &Position) -> anyhow::Result<()>;

    /// 向缓存添加一个持仓快照。
    ///
    /// # 错误
    ///
    /// 如果添加持仓快照失败，则返回错误。
    fn add_position_snapshot(&self, snapshot: &PositionSnapshot) -> anyhow::Result<()>;

    /// 向缓存添加一个订单簿。
    ///
    /// # 错误
    ///
    /// 如果添加订单簿失败，则返回错误。
    fn add_order_book(&self, order_book: &OrderBook) -> anyhow::Result<()>;

    /// 向缓存添加一个信号。
    ///
    /// # 错误
    ///
    /// 如果添加信号失败，则返回错误。
    fn add_signal(&self, signal: &Signal) -> anyhow::Result<()>;

    /// 向缓存添加自定义数据。
    ///
    /// # 错误
    ///
    /// 如果添加自定义数据失败，则返回错误。
    fn add_custom_data(&self, data: &CustomData) -> anyhow::Result<()>;

    /// 向缓存添加一个报价 Tick。
    ///
    /// # 错误
    ///
    /// 如果添加报价 Tick 失败，则返回错误。
    fn add_quote(&self, quote: &QuoteTick) -> anyhow::Result<()>;

    /// 向缓存添加一个成交 Tick。
    ///
    /// # 错误
    ///
    /// 如果添加成交 Tick 失败，则返回错误。
    fn add_trade(&self, trade: &TradeTick) -> anyhow::Result<()>;

    /// 向缓存添加资金费率更新。
    ///
    /// # 错误
    ///
    /// 如果添加资金费率更新失败，则返回错误。
    fn add_funding_rate(&self, funding_rate: &FundingRateUpdate) -> anyhow::Result<()>;

    /// 向缓存添加 Bar 数据。
    ///
    /// # 错误
    ///
    /// 如果添加 Bar 数据失败，则返回错误。
    fn add_bar(&self, bar: &Bar) -> anyhow::Result<()>;

    /// 向缓存添加希腊字母数据。
    ///
    /// # 错误
    ///
    /// 如果添加希腊字母数据失败，则返回错误。
    fn add_greeks(&self, greeks: &GreeksData) -> anyhow::Result<()> {
        Ok(())
    }

    /// 向缓存添加收益率曲线数据。
    ///
    /// # 错误
    ///
    /// 如果添加收益率曲线数据失败，则返回错误。
    fn add_yield_curve(&self, yield_curve: &YieldCurveData) -> anyhow::Result<()> {
        Ok(())
    }

    /// 从缓存中删除 Actor 状态。
    ///
    /// # 错误
    ///
    /// 如果删除 Actor 状态失败，则返回错误。
    fn delete_actor(&self, component_id: &ComponentId) -> anyhow::Result<()>;

    /// 从缓存中删除策略状态。
    ///
    /// # 错误
    ///
    /// 如果删除策略状态失败，则返回错误。
    fn delete_strategy(&self, component_id: &StrategyId) -> anyhow::Result<()>;

    /// 从缓存中删除订单。
    ///
    /// # 错误
    ///
    /// 如果删除订单失败，则返回错误。
    fn delete_order(&self, client_order_id: &ClientOrderId) -> anyhow::Result<()>;

    /// 从缓存中删除持仓。
    ///
    /// # 错误
    ///
    /// 如果删除持仓失败，则返回错误。
    fn delete_position(&self, position_id: &PositionId) -> anyhow::Result<()>;

    /// 从缓存中删除账户事件。
    ///
    /// # 错误
    ///
    /// 如果删除账户事件失败，则返回错误。
    fn delete_account_event(&self, account_id: &AccountId, event_id: &str) -> anyhow::Result<()>;

    /// 索引交易所订单 ID 及其对应的客户订单 ID。
    ///
    /// # 错误
    ///
    /// 如果索引交易所订单 ID 失败，则返回错误。
    fn index_venue_order_id(
        &self,
        client_order_id: ClientOrderId,
        venue_order_id: VenueOrderId,
    ) -> anyhow::Result<()>;

    /// 索引订单-持仓映射。
    ///
    /// # 错误
    ///
    /// 如果索引订单-持仓映射失败，则返回错误。
    fn index_order_position(
        &self,
        client_order_id: ClientOrderId,
        position_id: PositionId,
    ) -> anyhow::Result<()>;

    /// 在缓存中更新 Actor 状态。
    ///
    /// # 错误
    ///
    /// 如果更新 Actor 状态失败，则返回错误。
    fn update_actor(&self) -> anyhow::Result<()>;

    /// 在缓存中更新策略状态。
    ///
    /// # 错误
    ///
    /// 如果更新策略状态失败，则返回错误。
    fn update_strategy(&self) -> anyhow::Result<()>;

    /// 在缓存中更新账户。
    ///
    /// # 错误
    ///
    /// 如果更新账户失败，则返回错误。
    fn update_account(&self, account: &AccountAny) -> anyhow::Result<()>;

    /// 使用订单事件在缓存中更新订单。
    ///
    /// # 错误
    ///
    /// 如果更新订单失败，则返回错误。
    fn update_order(&self, order_event: &OrderEventAny) -> anyhow::Result<()>;

    /// 在缓存中更新持仓。
    ///
    /// # 错误
    ///
    /// 如果更新持仓失败，则返回错误。
    fn update_position(&self, position: &Position) -> anyhow::Result<()>;

    /// 创建订单状态快照。
    ///
    /// # 错误
    ///
    /// 如果订单状态快照失败，则返回错误。
    fn snapshot_order_state(&self, order: &OrderAny) -> anyhow::Result<()>;

    /// 创建持仓状态快照。
    ///
    /// # 错误
    ///
    /// 如果持仓状态快照失败，则返回错误。
    fn snapshot_position_state(&self, position: &Position) -> anyhow::Result<()>;

    /// 记录心跳时间戳。
    ///
    /// # 错误
    ///
    /// 如果记录心跳失败，则返回错误。
    fn heartbeat(&self, timestamp: UnixNanos) -> anyhow::Result<()>;
}
