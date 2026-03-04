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

use std::{
    any::Any,
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    fmt::Debug,
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
    rc::Rc,
    sync::Arc,
};

use ahash::{AHashMap, AHashSet};
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use nautilus_core::{Params, UUID4, UnixNanos, correctness::check_predicate_true};
#[cfg(feature = "defi")]
use nautilus_model::defi::{
    Block, Blockchain, Pool, PoolLiquidityUpdate, PoolSwap, data::PoolFeeCollect, data::PoolFlash,
};
use nautilus_model::{
    data::{
        Bar, BarType, DataType, FundingRateUpdate, IndexPriceUpdate, InstrumentStatus,
        MarkPriceUpdate, OrderBookDeltas, OrderBookDepth10, QuoteTick, TradeTick,
        close::InstrumentClose,
    },
    enums::BookType,
    events::order::{any::OrderEventAny, canceled::OrderCanceled, filled::OrderFilled},
    identifiers::{ActorId, ClientId, ComponentId, InstrumentId, TraderId, Venue},
    instruments::InstrumentAny,
    orderbook::OrderBook,
};
use ustr::Ustr;

#[cfg(feature = "indicators")]
use super::indicators::Indicators;
use super::{
    Actor,
    registry::{get_actor_unchecked, try_get_actor_unchecked},
};
#[cfg(feature = "defi")]
use crate::defi;
#[cfg(feature = "defi")]
#[allow(unused_imports)]
use crate::defi::data_actor as _; // Brings DeFi impl blocks into scope
use crate::{
    cache::Cache,
    clock::Clock,
    component::Component,
    enums::{ComponentState, ComponentTrigger},
    logging::{CMD, RECV, REQ, SEND},
    messages::{
        data::{
            BarsResponse, BookResponse, CustomDataResponse, DataCommand, FundingRatesResponse,
            InstrumentResponse, InstrumentsResponse, QuotesResponse, RequestBars,
            RequestBookSnapshot, RequestCommand, RequestCustomData, RequestFundingRates,
            RequestInstrument, RequestInstruments, RequestQuotes, RequestTrades, SubscribeBars,
            SubscribeBookDeltas, SubscribeBookSnapshots, SubscribeCommand, SubscribeCustomData,
            SubscribeFundingRates, SubscribeIndexPrices, SubscribeInstrument,
            SubscribeInstrumentClose, SubscribeInstrumentStatus, SubscribeInstruments,
            SubscribeMarkPrices, SubscribeQuotes, SubscribeTrades, TradesResponse, UnsubscribeBars,
            UnsubscribeBookDeltas, UnsubscribeBookSnapshots, UnsubscribeCommand,
            UnsubscribeCustomData, UnsubscribeFundingRates, UnsubscribeIndexPrices,
            UnsubscribeInstrument, UnsubscribeInstrumentClose, UnsubscribeInstrumentStatus,
            UnsubscribeInstruments, UnsubscribeMarkPrices, UnsubscribeQuotes, UnsubscribeTrades,
        },
        system::ShutdownSystem,
    },
    msgbus::{
        self, MStr, ShareableMessageHandler, Topic, TypedHandler, get_message_bus,
        switchboard::{
            MessagingSwitchboard, get_bars_topic, get_book_deltas_topic, get_book_snapshots_topic,
            get_custom_topic, get_funding_rate_topic, get_index_price_topic,
            get_instrument_close_topic, get_instrument_status_topic, get_instrument_topic,
            get_instruments_topic, get_mark_price_topic, get_order_cancels_topic,
            get_order_fills_topic, get_quotes_topic, get_trades_topic,
        },
    },
    signal::Signal,
    timer::{TimeEvent, TimeEventCallback},
};

/// 基于 [`DataActor`] 的组件的通用配置。
#[derive(Debug, Clone)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        module = "nautilus_trader.core.nautilus_pyo3.common",
        subclass,
        from_py_object
    )
)]
pub struct DataActorConfig {
    /// Actor 的自定义标识符。
    pub actor_id: Option<ActorId>,
    /// 是否记录事件日志。
    pub log_events: bool,
    /// 是否记录命令日志。
    pub log_commands: bool,
}

impl Default for DataActorConfig {
    fn default() -> Self {
        Self {
            actor_id: None,
            log_events: true,
            log_commands: true,
        }
    }
}

/// 用于从可导入路径创建 Actor 的配置。
#[derive(Debug, Clone)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.common", from_py_object)
)]
pub struct ImportableActorConfig {
    /// Actor 类的完全限定名称。
    pub actor_path: String,
    /// Actor 配置类的完全限定名称。
    pub config_path: String,
    /// 以字典形式表示的 Actor 配置。
    pub config: HashMap<String, serde_json::Value>,
}

type RequestCallback = Arc<dyn Fn(UUID4) + Send + Sync>;

pub trait DataActor:
    Component + Deref<Target = DataActorCore> + DerefMut<Target = DataActorCore>
{
    /// 当保存 Actor 状态时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果保存 Actor 状态失败，则返回错误。
    fn on_save(&self) -> anyhow::Result<IndexMap<String, Vec<u8>>> {
        Ok(IndexMap::new())
    }

    /// 当加载 Actor 状态时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果加载 Actor 状态失败，则返回错误。
    #[allow(unused_variables)]
    fn on_load(&mut self, state: IndexMap<String, Vec<u8>>) -> anyhow::Result<()> {
        Ok(())
    }

    /// 启动时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果启动 Actor 失败，则返回错误。
    fn on_start(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "调用了未被重写的 `on_start` 处理程序，\
            预计启动 Actor 时所需的任何操作（如订阅/请求数据）\
            应在此处发生"
        );
        Ok(())
    }

    /// 停止时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果停止 Actor 失败，则返回错误。
    fn on_stop(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "调用了未被重写的 `on_stop` 处理程序，\
            预计停止 Actor 时所需的任何操作（如取消订阅数据）\
            应在此处发生",
        );
        Ok(())
    }

    /// 恢复时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果恢复 Actor 失败，则返回错误。
    fn on_resume(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "调用了未被重写的 `on_resume` 处理程序，\
            预计在停止后恢复 Actor 时所需的任何操作\
            应在此处发生"
        );
        Ok(())
    }

    /// 重置时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果重置 Actor 失败，则返回错误。
    fn on_reset(&mut self) -> anyhow::Result<()> {
        log::warn!(
            "调用了未被重写的 `on_reset` 处理程序，\
            预计重置 Actor 时所需的任何操作（如重置指标和其他状态）\
            应在此处发生"
        );
        Ok(())
    }

    /// 销毁 (Dispose) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果销毁 Actor 失败，则返回错误。
    fn on_dispose(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 降级 (Degrade) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果降级 Actor 失败，则返回错误。
    fn on_degrade(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 故障 (Fault) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果 Actor 进入故障状态失败，则返回错误。
    fn on_fault(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收时间事件时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理时间事件失败，则返回错误。
    #[allow(unused_variables)]
    fn on_time_event(&mut self, event: &TimeEvent) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收自定义数据时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理数据失败，则返回错误。
    #[allow(unused_variables)]
    fn on_data(&mut self, data: &dyn Any) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收信号时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理信号失败，则返回错误。
    #[allow(unused_variables)]
    fn on_signal(&mut self, signal: &Signal) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收工具定义 (Instrument) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理工具定义失败，则返回错误。
    #[allow(unused_variables)]
    fn on_instrument(&mut self, instrument: &InstrumentAny) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收订单簿增量 (Order book deltas) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理订单簿增量失败，则返回错误。
    #[allow(unused_variables)]
    fn on_book_deltas(&mut self, deltas: &OrderBookDeltas) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收订单簿 (Order book) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理订单簿失败，则返回错误。
    #[allow(unused_variables)]
    fn on_book(&mut self, order_book: &OrderBook) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收报价 (Quote) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理报价失败，则返回错误。
    #[allow(unused_variables)]
    fn on_quote(&mut self, quote: &QuoteTick) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收逐笔成交 (Trade) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理逐笔成交失败，则返回错误。
    #[allow(unused_variables)]
    fn on_trade(&mut self, tick: &TradeTick) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收 K 线 (Bar) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理 K 线失败，则返回错误。
    #[allow(unused_variables)]
    fn on_bar(&mut self, bar: &Bar) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收标记价格更新 (Mark price update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理标记价格更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_mark_price(&mut self, mark_price: &MarkPriceUpdate) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收指数价格更新 (Index price update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理指数价格更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_index_price(&mut self, index_price: &IndexPriceUpdate) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收资金费率更新 (Funding rate update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理资金费率更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_funding_rate(&mut self, funding_rate: &FundingRateUpdate) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收工具状态更新 (Instrument status update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理工具状态更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_instrument_status(&mut self, data: &InstrumentStatus) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收工具收盘更新 (Instrument close update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理工具收盘更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_instrument_close(&mut self, update: &InstrumentClose) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收订单成交 (Order filled) 事件时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理订单成交事件失败，则返回错误。
    #[allow(unused_variables)]
    fn on_order_filled(&mut self, event: &OrderFilled) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收订单取消 (Order canceled) 事件时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理订单取消事件失败，则返回错误。
    #[allow(unused_variables)]
    fn on_order_canceled(&mut self, event: &OrderCanceled) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收区块 (Block) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理区块失败，则返回错误。
    #[allow(unused_variables)]
    fn on_block(&mut self, block: &Block) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收流动性池 (Pool) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理流动性池失败，则返回错误。
    #[allow(unused_variables)]
    fn on_pool(&mut self, pool: &Pool) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收流动性池交换 (Pool swap) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理流动性池交换失败，则返回错误。
    #[allow(unused_variables)]
    fn on_pool_swap(&mut self, swap: &PoolSwap) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收流动性池流动性更新 (Pool liquidity update) 时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理流动性池流动性更新失败，则返回错误。
    #[allow(unused_variables)]
    fn on_pool_liquidity_update(&mut self, update: &PoolLiquidityUpdate) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收流动性池费用收取事件时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理流动性池费用收取失败，则返回错误。
    #[allow(unused_variables)]
    fn on_pool_fee_collect(&mut self, collect: &PoolFeeCollect) -> anyhow::Result<()> {
        Ok(())
    }

    #[cfg(feature = "defi")]
    /// 接收流动性池闪电贷事件时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理流动性池闪电贷失败，则返回错误。
    #[allow(unused_variables)]
    fn on_pool_flash(&mut self, flash: &PoolFlash) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史数据时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史数据失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_data(&mut self, data: &dyn Any) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史报价时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史报价失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_quotes(&mut self, quotes: &[QuoteTick]) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史逐笔成交时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史逐笔成交失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_trades(&mut self, trades: &[TradeTick]) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史资金费率时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史资金费率失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_funding_rates(
        &mut self,
        funding_rates: &[FundingRateUpdate],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史 K 线时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史 K 线失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_bars(&mut self, bars: &[Bar]) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史标记价格时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史标记价格失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_mark_prices(&mut self, mark_prices: &[MarkPriceUpdate]) -> anyhow::Result<()> {
        Ok(())
    }

    /// 接收历史指数价格时要执行的操作。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果处理历史指数价格失败，则返回错误。
    #[allow(unused_variables)]
    fn on_historical_index_prices(
        &mut self,
        index_prices: &[IndexPriceUpdate],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// 处理接收到的时间事件。
    fn handle_time_event(&mut self, event: &TimeEvent) {
        log_received(&event);

        if let Err(e) = DataActor::on_time_event(self, event) {
            log_error(&e);
        }
    }

    /// 处理接收到的自定义数据点。
    fn handle_data(&mut self, data: &dyn Any) {
        log_received(&data);

        if self.not_running() {
            log_not_running(&data);
            return;
        }

        if let Err(e) = self.on_data(data) {
            log_error(&e);
        }
    }

    /// 处理接收到的信号。
    fn handle_signal(&mut self, signal: &Signal) {
        log_received(&signal);

        if self.not_running() {
            log_not_running(&signal);
            return;
        }

        if let Err(e) = self.on_signal(signal) {
            log_error(&e);
        }
    }

    /// 处理接收到的工具定义。
    fn handle_instrument(&mut self, instrument: &InstrumentAny) {
        log_received(&instrument);

        if self.not_running() {
            log_not_running(&instrument);
            return;
        }

        if let Err(e) = self.on_instrument(instrument) {
            log_error(&e);
        }
    }

    /// 处理接收到的订单簿增量。
    fn handle_book_deltas(&mut self, deltas: &OrderBookDeltas) {
        log_received(&deltas);

        if self.not_running() {
            log_not_running(&deltas);
            return;
        }

        if let Err(e) = self.on_book_deltas(deltas) {
            log_error(&e);
        }
    }

    /// 处理接收到的订单簿引用。
    fn handle_book(&mut self, book: &OrderBook) {
        log_received(&book);

        if self.not_running() {
            log_not_running(&book);
            return;
        }

        if let Err(e) = self.on_book(book) {
            log_error(&e);
        };
    }

    /// 处理接收到的报价。
    fn handle_quote(&mut self, quote: &QuoteTick) {
        log_received(&quote);

        if self.not_running() {
            log_not_running(&quote);
            return;
        }

        if let Err(e) = self.on_quote(quote) {
            log_error(&e);
        }
    }

    /// 处理接收到的逐笔成交。
    fn handle_trade(&mut self, trade: &TradeTick) {
        log_received(&trade);

        if self.not_running() {
            log_not_running(&trade);
            return;
        }

        if let Err(e) = self.on_trade(trade) {
            log_error(&e);
        }
    }

    /// 处理接收到的 K 线。
    fn handle_bar(&mut self, bar: &Bar) {
        log_received(&bar);

        if self.not_running() {
            log_not_running(&bar);
            return;
        }

        if let Err(e) = self.on_bar(bar) {
            log_error(&e);
        }
    }

    /// 处理接收到的标记价格更新。
    fn handle_mark_price(&mut self, mark_price: &MarkPriceUpdate) {
        log_received(&mark_price);

        if self.not_running() {
            log_not_running(&mark_price);
            return;
        }

        if let Err(e) = self.on_mark_price(mark_price) {
            log_error(&e);
        }
    }

    /// 处理接收到的指数价格更新。
    fn handle_index_price(&mut self, index_price: &IndexPriceUpdate) {
        log_received(&index_price);

        if self.not_running() {
            log_not_running(&index_price);
            return;
        }

        if let Err(e) = self.on_index_price(index_price) {
            log_error(&e);
        }
    }

    /// 处理接收到的资金费率更新。
    fn handle_funding_rate(&mut self, funding_rate: &FundingRateUpdate) {
        log_received(&funding_rate);

        if self.not_running() {
            log_not_running(&funding_rate);
            return;
        }

        if let Err(e) = self.on_funding_rate(funding_rate) {
            log_error(&e);
        }
    }

    /// 处理接收到的工具状态。
    fn handle_instrument_status(&mut self, status: &InstrumentStatus) {
        log_received(&status);

        if self.not_running() {
            log_not_running(&status);
            return;
        }

        if let Err(e) = self.on_instrument_status(status) {
            log_error(&e);
        }
    }

    /// 处理接收到的工具收盘。
    fn handle_instrument_close(&mut self, close: &InstrumentClose) {
        log_received(&close);

        if self.not_running() {
            log_not_running(&close);
            return;
        }

        if let Err(e) = self.on_instrument_close(close) {
            log_error(&e);
        }
    }

    /// 处理接收到的订单成交事件。
    fn handle_order_filled(&mut self, event: &OrderFilled) {
        log_received(&event);

        // 检查重复处理：如果事件的 strategy_id 与此 Actor 的 id 匹配，
        // 意味着策略通过自动订阅和手动 subscribe_order_fills 同时接收到了自己的成交事件，
        // 因此跳过手动处理程序。
        if event.strategy_id.inner() == self.actor_id().inner() {
            return;
        }

        if self.not_running() {
            log_not_running(&event);
            return;
        }

        if let Err(e) = self.on_order_filled(event) {
            log_error(&e);
        }
    }

    /// 处理接收到的订单取消事件。
    fn handle_order_canceled(&mut self, event: &OrderCanceled) {
        log_received(&event);

        // 检查重复处理：如果事件的 strategy_id 与此 Actor 的 id 匹配，
        // 意味着策略通过自动订阅和手动 subscribe_order_cancels 同时接收到了自己的取消事件，
        // 因此跳过手动处理程序。
        if event.strategy_id.inner() == self.actor_id().inner() {
            return;
        }

        if self.not_running() {
            log_not_running(&event);
            return;
        }

        if let Err(e) = self.on_order_canceled(event) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的区块。
    fn handle_block(&mut self, block: &Block) {
        log_received(&block);

        if self.not_running() {
            log_not_running(&block);
            return;
        }

        if let Err(e) = self.on_block(block) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的流动性池定义更新。
    fn handle_pool(&mut self, pool: &Pool) {
        log_received(&pool);

        if self.not_running() {
            log_not_running(&pool);
            return;
        }

        if let Err(e) = self.on_pool(pool) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的流动性池交换。
    fn handle_pool_swap(&mut self, swap: &PoolSwap) {
        log_received(&swap);

        if self.not_running() {
            log_not_running(&swap);
            return;
        }

        if let Err(e) = self.on_pool_swap(swap) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的流动性池流动性更新。
    fn handle_pool_liquidity_update(&mut self, update: &PoolLiquidityUpdate) {
        log_received(&update);

        if self.not_running() {
            log_not_running(&update);
            return;
        }

        if let Err(e) = self.on_pool_liquidity_update(update) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的流动性池费用收取。
    fn handle_pool_fee_collect(&mut self, collect: &PoolFeeCollect) {
        log_received(&collect);

        if self.not_running() {
            log_not_running(&collect);
            return;
        }

        if let Err(e) = self.on_pool_fee_collect(collect) {
            log_error(&e);
        }
    }

    #[cfg(feature = "defi")]
    /// 处理接收到的流动性池闪电贷事件。
    fn handle_pool_flash(&mut self, flash: &PoolFlash) {
        log_received(&flash);

        if self.not_running() {
            log_not_running(&flash);
            return;
        }

        if let Err(e) = self.on_pool_flash(flash) {
            log_error(&e);
        }
    }

    /// 处理接收到的历史数据。
    fn handle_historical_data(&mut self, data: &dyn Any) {
        log_received(&data);

        if let Err(e) = self.on_historical_data(data) {
            log_error(&e);
        }
    }

    /// 处理数据响应 (Data response)。
    fn handle_data_response(&mut self, resp: &CustomDataResponse) {
        log_received(&resp);

        if let Err(e) = self.on_historical_data(resp.data.as_ref()) {
            log_error(&e);
        }
    }

    /// 处理工具定义响应 (Instrument response)。
    fn handle_instrument_response(&mut self, resp: &InstrumentResponse) {
        log_received(&resp);

        if let Err(e) = self.on_instrument(&resp.data) {
            log_error(&e);
        }
    }

    /// 处理多工具定义响应 (Instruments response)。
    fn handle_instruments_response(&mut self, resp: &InstrumentsResponse) {
        log_received(&resp);

        for inst in &resp.data {
            if let Err(e) = self.on_instrument(inst) {
                log_error(&e);
            }
        }
    }

    /// 处理订单簿响应 (Book response)。
    fn handle_book_response(&mut self, resp: &BookResponse) {
        log_received(&resp);

        if let Err(e) = self.on_book(&resp.data) {
            log_error(&e);
        }
    }

    /// 处理报价响应 (Quotes response)。
    fn handle_quotes_response(&mut self, resp: &QuotesResponse) {
        log_received(&resp);

        if let Err(e) = self.on_historical_quotes(&resp.data) {
            log_error(&e);
        }
    }

    /// 处理逐笔成交响应 (Trades response)。
    fn handle_trades_response(&mut self, resp: &TradesResponse) {
        log_received(&resp);

        if let Err(e) = self.on_historical_trades(&resp.data) {
            log_error(&e);
        }
    }

    /// 处理资金费率响应 (Funding rates response)。
    fn handle_funding_rates_response(&mut self, resp: &FundingRatesResponse) {
        log_received(&resp);

        if let Err(e) = self.on_historical_funding_rates(&resp.data) {
            log_error(&e);
        }
    }

    /// 处理 K 线响应 (Bars response)。
    fn handle_bars_response(&mut self, resp: &BarsResponse) {
        log_received(&resp);

        if let Err(e) = self.on_historical_bars(&resp.data) {
            log_error(&e);
        }
    }

    /// 订阅流式 `data_type` 数据。
    fn subscribe_data(
        &mut self,
        data_type: DataType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_any(move |data: &dyn Any| {
            get_actor_unchecked::<Self>(&actor_id).handle_data(data);
        });

        DataActorCore::subscribe_data(self, handler, data_type, client_id, params);
    }

    /// 订阅针对 `instrument_id` 的流式 [`QuoteTick`]（行情报价）数据。
    fn subscribe_quotes(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_quotes_topic(instrument_id);

        let handler = TypedHandler::from(move |quote: &QuoteTick| {
            if let Some(mut actor) = try_get_actor_unchecked::<Self>(&actor_id) {
                actor.handle_quote(quote);
            } else {
                log::error!("未找到用于报价处理的 Actor {actor_id}");
            }
        });

        DataActorCore::subscribe_quotes(self, topic, handler, instrument_id, client_id, params);
    }

    /// 订阅针对 `venue`（场地）的流式 [`InstrumentAny`]（工具定义）数据。
    fn subscribe_instruments(
        &mut self,
        venue: Venue,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_instruments_topic(venue);

        let handler = ShareableMessageHandler::from_typed(move |instrument: &InstrumentAny| {
            if let Some(mut actor) = try_get_actor_unchecked::<Self>(&actor_id) {
                actor.handle_instrument(instrument);
            } else {
                log::error!("未找到用于多工具定义处理的 Actor {actor_id}");
            }
        });

        DataActorCore::subscribe_instruments(self, topic, handler, venue, client_id, params);
    }

    /// 订阅针对 `instrument_id` 的流式 [`InstrumentAny`]（工具定义）数据。
    fn subscribe_instrument(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_instrument_topic(instrument_id);

        let handler = ShareableMessageHandler::from_typed(move |instrument: &InstrumentAny| {
            if let Some(mut actor) = try_get_actor_unchecked::<Self>(&actor_id) {
                actor.handle_instrument(instrument);
            } else {
                log::error!("未找到用于工具定义处理的 Actor {actor_id}");
            }
        });

        DataActorCore::subscribe_instrument(self, topic, handler, instrument_id, client_id, params);
    }

    /// 订阅针对 `instrument_id` 的流式 [`OrderBookDeltas`]（订单簿增量）数据。
    fn subscribe_book_deltas(
        &mut self,
        instrument_id: InstrumentId,
        book_type: BookType,
        depth: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        managed: bool,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_book_deltas_topic(instrument_id);

        let handler = TypedHandler::from(move |deltas: &OrderBookDeltas| {
            get_actor_unchecked::<Self>(&actor_id).handle_book_deltas(deltas);
        });

        DataActorCore::subscribe_book_deltas(
            self,
            topic,
            handler,
            instrument_id,
            book_type,
            depth,
            client_id,
            managed,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的指定时间间隔的 [`OrderBook`]（订单簿）快照。
    fn subscribe_book_at_interval(
        &mut self,
        instrument_id: InstrumentId,
        book_type: BookType,
        depth: Option<NonZeroUsize>,
        interval_ms: NonZeroUsize,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_book_snapshots_topic(instrument_id, interval_ms);

        let handler = TypedHandler::from(move |book: &OrderBook| {
            get_actor_unchecked::<Self>(&actor_id).handle_book(book);
        });

        DataActorCore::subscribe_book_at_interval(
            self,
            topic,
            handler,
            instrument_id,
            book_type,
            depth,
            interval_ms,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的流式 [`TradeTick`]（逐笔成交）数据。
    fn subscribe_trades(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_trades_topic(instrument_id);

        let handler = TypedHandler::from(move |trade: &TradeTick| {
            get_actor_unchecked::<Self>(&actor_id).handle_trade(trade);
        });

        DataActorCore::subscribe_trades(self, topic, handler, instrument_id, client_id, params);
    }

    /// 订阅针对 `bar_type` 的流式 [`Bar`]（K 线）数据。
    fn subscribe_bars(
        &mut self,
        bar_type: BarType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_bars_topic(bar_type);

        let handler = TypedHandler::from(move |bar: &Bar| {
            get_actor_unchecked::<Self>(&actor_id).handle_bar(bar);
        });

        DataActorCore::subscribe_bars(self, topic, handler, bar_type, client_id, params);
    }

    /// 订阅针对 `instrument_id` 的流式 [`MarkPriceUpdate`]（标记价格更新）数据。
    fn subscribe_mark_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_mark_price_topic(instrument_id);

        let handler = TypedHandler::from(move |mark_price: &MarkPriceUpdate| {
            get_actor_unchecked::<Self>(&actor_id).handle_mark_price(mark_price);
        });

        DataActorCore::subscribe_mark_prices(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的流式 [`IndexPriceUpdate`]（指数价格更新）数据。
    fn subscribe_index_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_index_price_topic(instrument_id);

        let handler = TypedHandler::from(move |index_price: &IndexPriceUpdate| {
            get_actor_unchecked::<Self>(&actor_id).handle_index_price(index_price);
        });

        DataActorCore::subscribe_index_prices(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的流式 [`FundingRateUpdate`]（资金费率更新）数据。
    fn subscribe_funding_rates(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_funding_rate_topic(instrument_id);

        let handler = TypedHandler::from(move |funding_rate: &FundingRateUpdate| {
            get_actor_unchecked::<Self>(&actor_id).handle_funding_rate(funding_rate);
        });

        DataActorCore::subscribe_funding_rates(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的流式 [`InstrumentStatus`]（工具状态）数据。
    fn subscribe_instrument_status(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_instrument_status_topic(instrument_id);

        let handler = ShareableMessageHandler::from_typed(move |status: &InstrumentStatus| {
            get_actor_unchecked::<Self>(&actor_id).handle_instrument_status(status);
        });

        DataActorCore::subscribe_instrument_status(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的流式 [`InstrumentClose`]（工具收盘）数据。
    fn subscribe_instrument_close(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_instrument_close_topic(instrument_id);

        let handler = ShareableMessageHandler::from_typed(move |close: &InstrumentClose| {
            get_actor_unchecked::<Self>(&actor_id).handle_instrument_close(close);
        });

        DataActorCore::subscribe_instrument_close(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 订阅针对 `instrument_id` 的 [`OrderFilled`]（订单成交）事件。
    fn subscribe_order_fills(&mut self, instrument_id: InstrumentId)
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_order_fills_topic(instrument_id);

        let handler = TypedHandler::from(move |event: &OrderEventAny| {
            if let OrderEventAny::Filled(filled) = event {
                get_actor_unchecked::<Self>(&actor_id).handle_order_filled(filled);
            }
        });

        DataActorCore::subscribe_order_fills(self, topic, handler);
    }

    /// 订阅针对 `instrument_id` 的 [`OrderCanceled`]（订单取消）事件。
    fn subscribe_order_cancels(&mut self, instrument_id: InstrumentId)
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = get_order_cancels_topic(instrument_id);

        let handler = TypedHandler::from(move |event: &OrderEventAny| {
            if let OrderEventAny::Canceled(canceled) = event {
                get_actor_unchecked::<Self>(&actor_id).handle_order_canceled(canceled);
            }
        });

        DataActorCore::subscribe_order_cancels(self, topic, handler);
    }

    #[cfg(feature = "defi")]
    /// 订阅针对 `chain` 的流式 [`Block`]（区块）数据。
    fn subscribe_blocks(
        &mut self,
        chain: Blockchain,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_blocks_topic(chain);

        let handler = TypedHandler::from(move |block: &Block| {
            get_actor_unchecked::<Self>(&actor_id).handle_block(block);
        });

        DataActorCore::subscribe_blocks(self, topic, handler, chain, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 订阅针对位于 `instrument_id` 的 AMM 流动性池的流式 [`Pool`] 定义更新。
    fn subscribe_pool(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_pool_topic(instrument_id);

        let handler = TypedHandler::from(move |pool: &Pool| {
            get_actor_unchecked::<Self>(&actor_id).handle_pool(pool);
        });

        DataActorCore::subscribe_pool(self, topic, handler, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 订阅针对 `instrument_id` 的流式 [`PoolSwap`]（流动性池交换）数据。
    fn subscribe_pool_swaps(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_pool_swaps_topic(instrument_id);

        let handler = TypedHandler::from(move |swap: &PoolSwap| {
            get_actor_unchecked::<Self>(&actor_id).handle_pool_swap(swap);
        });

        DataActorCore::subscribe_pool_swaps(self, topic, handler, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 订阅针对 `instrument_id` 的流式 [`PoolLiquidityUpdate`]（流动性池流动性更新）数据。
    fn subscribe_pool_liquidity_updates(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_liquidity_topic(instrument_id);

        let handler = TypedHandler::from(move |update: &PoolLiquidityUpdate| {
            get_actor_unchecked::<Self>(&actor_id).handle_pool_liquidity_update(update);
        });

        DataActorCore::subscribe_pool_liquidity_updates(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    #[cfg(feature = "defi")]
    /// 订阅针对 `instrument_id` 的流式 [`PoolFeeCollect`]（流动性池费用收取）数据。
    fn subscribe_pool_fee_collects(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_collect_topic(instrument_id);

        let handler = TypedHandler::from(move |collect: &PoolFeeCollect| {
            get_actor_unchecked::<Self>(&actor_id).handle_pool_fee_collect(collect);
        });

        DataActorCore::subscribe_pool_fee_collects(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    #[cfg(feature = "defi")]
    /// 订阅针对给定 `instrument_id` 的流式 [`PoolFlash`]（流动性池闪电贷）事件。
    fn subscribe_pool_flash_events(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let topic = defi::switchboard::get_defi_flash_topic(instrument_id);

        let handler = TypedHandler::from(move |flash: &PoolFlash| {
            get_actor_unchecked::<Self>(&actor_id).handle_pool_flash(flash);
        });

        DataActorCore::subscribe_pool_flash_events(
            self,
            topic,
            handler,
            instrument_id,
            client_id,
            params,
        );
    }

    /// 取消订阅流式 `data_type` 数据。
    fn unsubscribe_data(
        &mut self,
        data_type: DataType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_data(self, data_type, client_id, params);
    }

    /// 取消订阅针对 `venue` 的流式 [`InstrumentAny`] 数据。
    fn unsubscribe_instruments(
        &mut self,
        venue: Venue,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_instruments(self, venue, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`InstrumentAny`] 数据。
    fn unsubscribe_instrument(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_instrument(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`OrderBookDeltas`] 数据。
    fn unsubscribe_book_deltas(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_book_deltas(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的指定时间间隔的 [`OrderBook`] 快照。
    fn unsubscribe_book_at_interval(
        &mut self,
        instrument_id: InstrumentId,
        interval_ms: NonZeroUsize,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_book_at_interval(
            self,
            instrument_id,
            interval_ms,
            client_id,
            params,
        );
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`QuoteTick`] 数据。
    fn unsubscribe_quotes(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_quotes(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`TradeTick`] 数据。
    fn unsubscribe_trades(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_trades(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `bar_type` 的流式 [`Bar`] 数据。
    fn unsubscribe_bars(
        &mut self,
        bar_type: BarType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_bars(self, bar_type, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`MarkPriceUpdate`] 数据。
    fn unsubscribe_mark_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_mark_prices(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`IndexPriceUpdate`] 数据。
    fn unsubscribe_index_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_index_prices(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`FundingRateUpdate`] 数据。
    fn unsubscribe_funding_rates(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_funding_rates(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`InstrumentStatus`] 数据。
    fn unsubscribe_instrument_status(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_instrument_status(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的流式 [`InstrumentClose`] 数据。
    fn unsubscribe_instrument_close(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_instrument_close(self, instrument_id, client_id, params);
    }

    /// 取消订阅针对 `instrument_id` 的 [`OrderFilled`] 事件。
    fn unsubscribe_order_fills(&mut self, instrument_id: InstrumentId)
    where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_order_fills(self, instrument_id);
    }

    /// 取消订阅针对 `instrument_id` 的 [`OrderCanceled`] 事件。
    fn unsubscribe_order_cancels(&mut self, instrument_id: InstrumentId)
    where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_order_cancels(self, instrument_id);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对 `chain` 的流式 [`Block`] 数据。
    fn unsubscribe_blocks(
        &mut self,
        chain: Blockchain,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_blocks(self, chain, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对位于 `instrument_id` 的 AMM 流动性池的流式 [`Pool`] 定义更新。
    fn unsubscribe_pool(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_pool(self, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对 `instrument_id` 的流式 [`PoolSwap`] 数据。
    fn unsubscribe_pool_swaps(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_pool_swaps(self, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对 `instrument_id` 的流式 [`PoolLiquidityUpdate`] 数据。
    fn unsubscribe_pool_liquidity_updates(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_pool_liquidity_updates(self, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对 `instrument_id` 的流式 [`PoolFeeCollect`] 数据。
    fn unsubscribe_pool_fee_collects(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_pool_fee_collects(self, instrument_id, client_id, params);
    }

    #[cfg(feature = "defi")]
    /// 取消订阅针对给定 `instrument_id` 的流式 [`PoolFlash`] 事件。
    fn unsubscribe_pool_flash_events(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) where
        Self: 'static + Debug + Sized,
    {
        DataActorCore::unsubscribe_pool_flash_events(self, instrument_id, client_id, params);
    }

    /// 请求给定 `data_type` 的历史自定义数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_data(
        &mut self,
        data_type: DataType,
        client_id: ClientId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &CustomDataResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_data_response(resp);
        });

        DataActorCore::request_data(
            self, data_type, client_id, start, end, limit, params, handler,
        )
    }

    /// 请求给定 `instrument_id` 的历史 [`InstrumentResponse`] 数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_instrument(
        &mut self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &InstrumentResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_instrument_response(resp);
        });

        DataActorCore::request_instrument(
            self,
            instrument_id,
            start,
            end,
            client_id,
            params,
            handler,
        )
    }

    /// 请求可选 `venue` 的历史 [`InstrumentsResponse`] 定义。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_instruments(
        &mut self,
        venue: Option<Venue>,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &InstrumentsResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_instruments_response(resp);
        });

        DataActorCore::request_instruments(self, venue, start, end, client_id, params, handler)
    }

    /// 请求给定 `instrument_id` 的 [`OrderBook`] 快照。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_book_snapshot(
        &mut self,
        instrument_id: InstrumentId,
        depth: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &BookResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_book_response(resp);
        });

        DataActorCore::request_book_snapshot(self, instrument_id, depth, client_id, params, handler)
    }

    /// 请求给定 `instrument_id` 的历史 [`QuoteTick`] 数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_quotes(
        &mut self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &QuotesResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_quotes_response(resp);
        });

        DataActorCore::request_quotes(
            self,
            instrument_id,
            start,
            end,
            limit,
            client_id,
            params,
            handler,
        )
    }

    /// 请求给定 `instrument_id` 的历史 [`TradeTick`] 数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_trades(
        &mut self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &TradesResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_trades_response(resp);
        });

        DataActorCore::request_trades(
            self,
            instrument_id,
            start,
            end,
            limit,
            client_id,
            params,
            handler,
        )
    }

    /// 请求给定 `instrument_id` 的历史 [`FundingRateUpdate`] 数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_funding_rates(
        &mut self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &FundingRatesResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_funding_rates_response(resp);
        });

        DataActorCore::request_funding_rates(
            self,
            instrument_id,
            start,
            end,
            limit,
            client_id,
            params,
            handler,
        )
    }

    /// 请求给定 `bar_type` 的历史 [`Bar`] 数据。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    fn request_bars(
        &mut self,
        bar_type: BarType,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) -> anyhow::Result<UUID4>
    where
        Self: 'static + Debug + Sized,
    {
        let actor_id = self.actor_id().inner();
        let handler = ShareableMessageHandler::from_typed(move |resp: &BarsResponse| {
            get_actor_unchecked::<Self>(&actor_id).handle_bars_response(resp);
        });

        DataActorCore::request_bars(
            self, bar_type, start, end, limit, client_id, params, handler,
        )
    }
}

// 通用实现：任何 DataActor 自动实现 Actor
impl<T> Actor for T
where
    T: DataActor + Debug + 'static,
{
    fn id(&self) -> Ustr {
        self.actor_id.inner()
    }

    #[allow(unused_variables)]
    fn handle(&mut self, msg: &dyn Any) {
        // 默认空实现 - 具体 Actor 可以根据需要重写
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// 通用实现：任何 DataActor 自动实现 Component
impl<T> Component for T
where
    T: DataActor + Debug + 'static,
{
    fn component_id(&self) -> ComponentId {
        ComponentId::new(self.actor_id.inner().as_str())
    }

    fn state(&self) -> ComponentState {
        self.state
    }

    fn transition_state(&mut self, trigger: ComponentTrigger) -> anyhow::Result<()> {
        self.state = self.state.transition(&trigger)?;
        log::info!("{}", self.state.variant_name());
        Ok(())
    }

    fn register(
        &mut self,
        trader_id: TraderId,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<()> {
        DataActorCore::register(self, trader_id, clock.clone(), cache)?;

        // 为此 Actor 注册默认的时间事件处理程序
        let actor_id = self.actor_id().inner();
        let callback = TimeEventCallback::from(move |event: TimeEvent| {
            if let Some(mut actor) = try_get_actor_unchecked::<Self>(&actor_id) {
                actor.handle_time_event(&event);
            } else {
                log::error!("未找到用于时间事件处理的 Actor {actor_id}");
            }
        });

        clock.borrow_mut().register_default_handler(callback);

        self.initialize()
    }

    fn on_start(&mut self) -> anyhow::Result<()> {
        DataActor::on_start(self)
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        DataActor::on_stop(self)
    }

    fn on_resume(&mut self) -> anyhow::Result<()> {
        DataActor::on_resume(self)
    }

    fn on_degrade(&mut self) -> anyhow::Result<()> {
        DataActor::on_degrade(self)
    }

    fn on_fault(&mut self) -> anyhow::Result<()> {
        DataActor::on_fault(self)
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        DataActor::on_reset(self)
    }

    fn on_dispose(&mut self) -> anyhow::Result<()> {
        DataActor::on_dispose(self)
    }
}

/// 所有 Actor 的核心功能。
#[derive(Clone)]
#[allow(
    dead_code,
    reason = "TODO: Under development (pending_requests, signal_classes)"
)]
pub struct DataActorCore {
    /// Actor 标识符。
    pub actor_id: ActorId,
    /// Actor 配置。
    pub config: DataActorConfig,
    trader_id: Option<TraderId>,
    clock: Option<Rc<RefCell<dyn Clock>>>, // 在注册时连接
    cache: Option<Rc<RefCell<Cache>>>,     // 在注册时连接
    state: ComponentState,
    topic_handlers: AHashMap<MStr<Topic>, ShareableMessageHandler>,
    deltas_handlers: AHashMap<MStr<Topic>, TypedHandler<OrderBookDeltas>>,
    depth10_handlers: AHashMap<MStr<Topic>, TypedHandler<OrderBookDepth10>>,
    book_handlers: AHashMap<MStr<Topic>, TypedHandler<OrderBook>>,
    quote_handlers: AHashMap<MStr<Topic>, TypedHandler<QuoteTick>>,
    trade_handlers: AHashMap<MStr<Topic>, TypedHandler<TradeTick>>,
    bar_handlers: AHashMap<MStr<Topic>, TypedHandler<Bar>>,
    mark_price_handlers: AHashMap<MStr<Topic>, TypedHandler<MarkPriceUpdate>>,
    index_price_handlers: AHashMap<MStr<Topic>, TypedHandler<IndexPriceUpdate>>,
    funding_rate_handlers: AHashMap<MStr<Topic>, TypedHandler<FundingRateUpdate>>,
    order_event_handlers: AHashMap<MStr<Topic>, TypedHandler<OrderEventAny>>,
    #[cfg(feature = "defi")]
    block_handlers: AHashMap<MStr<Topic>, TypedHandler<Block>>,
    #[cfg(feature = "defi")]
    pool_handlers: AHashMap<MStr<Topic>, TypedHandler<Pool>>,
    #[cfg(feature = "defi")]
    pool_swap_handlers: AHashMap<MStr<Topic>, TypedHandler<PoolSwap>>,
    #[cfg(feature = "defi")]
    pool_liquidity_handlers: AHashMap<MStr<Topic>, TypedHandler<PoolLiquidityUpdate>>,
    #[cfg(feature = "defi")]
    pool_collect_handlers: AHashMap<MStr<Topic>, TypedHandler<PoolFeeCollect>>,
    #[cfg(feature = "defi")]
    pool_flash_handlers: AHashMap<MStr<Topic>, TypedHandler<PoolFlash>>,
    warning_events: AHashSet<String>, // TODO: TBD
    pending_requests: AHashMap<UUID4, Option<RequestCallback>>,
    signal_classes: AHashMap<String, String>,
    #[cfg(feature = "indicators")]
    indicators: Indicators,
}

impl Debug for DataActorCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(DataActorCore))
            .field("actor_id", &self.actor_id)
            .field("config", &self.config)
            .field("state", &self.state)
            .field("trader_id", &self.trader_id)
            .finish()
    }
}

impl DataActorCore {
    /// 为 `topic`（主题）添加一个订阅处理程序。
    ///
    /// 如果 Actor 已经订阅了该主题，则记录一条警告。
    pub(crate) fn add_subscription_any(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
    ) {
        if self.topic_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅主题 '{topic}'",
                self.actor_id,
            );
            return;
        }

        self.topic_handlers.insert(topic, handler.clone());
        msgbus::subscribe_any(topic.into(), handler, None);
    }

    /// 如果存在，则移除 `topic` 的订阅处理程序。
    ///
    /// 如果 Actor 当前未订阅该主题，则记录一条警告。
    pub(crate) fn remove_subscription_any(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.topic_handlers.remove(&topic) {
            msgbus::unsubscribe_any(topic.into(), handler);
        } else {
            log::warn!(
                "Actor {} 尝试取消未订阅的主题 '{topic}'",
                self.actor_id,
            );
        }
    }

    pub(crate) fn add_quote_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<QuoteTick>,
    ) {
        if self.quote_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅报价 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.quote_handlers.insert(topic, handler.clone());
        msgbus::subscribe_quotes(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_quote_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.quote_handlers.remove(&topic) {
            msgbus::unsubscribe_quotes(topic.into(), &handler);
        }
    }

    pub(crate) fn add_trade_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<TradeTick>,
    ) {
        if self.trade_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅逐笔成交 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.trade_handlers.insert(topic, handler.clone());
        msgbus::subscribe_trades(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_trade_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.trade_handlers.remove(&topic) {
            msgbus::unsubscribe_trades(topic.into(), &handler);
        }
    }

    pub(crate) fn add_bar_subscription(&mut self, topic: MStr<Topic>, handler: TypedHandler<Bar>) {
        if self.bar_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅 K 线 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.bar_handlers.insert(topic, handler.clone());
        msgbus::subscribe_bars(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_bar_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.bar_handlers.remove(&topic) {
            msgbus::unsubscribe_bars(topic.into(), &handler);
        }
    }

    pub(crate) fn add_order_event_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderEventAny>,
    ) {
        if self.order_event_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅订单事件 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.order_event_handlers.insert(topic, handler.clone());
        msgbus::subscribe_order_events(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_order_event_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.order_event_handlers.remove(&topic) {
            msgbus::unsubscribe_order_events(topic.into(), &handler);
        }
    }

    pub(crate) fn add_deltas_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderBookDeltas>,
    ) {
        if self.deltas_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅增量 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.deltas_handlers.insert(topic, handler.clone());
        msgbus::subscribe_book_deltas(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_deltas_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.deltas_handlers.remove(&topic) {
            msgbus::unsubscribe_book_deltas(topic.into(), &handler);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn add_depth10_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderBookDepth10>,
    ) {
        if self.depth10_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅 depth10 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.depth10_handlers.insert(topic, handler.clone());
        msgbus::subscribe_book_depth10(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_depth10_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.depth10_handlers.remove(&topic) {
            msgbus::unsubscribe_book_depth10(topic.into(), &handler);
        }
    }

    pub(crate) fn add_instrument_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
    ) {
        if self.topic_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅工具定义 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.topic_handlers.insert(topic, handler.clone());
        msgbus::subscribe_any(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_instrument_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.topic_handlers.remove(&topic) {
            msgbus::unsubscribe_any(topic.into(), handler);
        }
    }

    pub(crate) fn add_instrument_close_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
    ) {
        if self.topic_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅工具收盘 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.topic_handlers.insert(topic, handler.clone());
        msgbus::subscribe_any(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_instrument_close_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.topic_handlers.remove(&topic) {
            msgbus::unsubscribe_any(topic.into(), handler);
        }
    }

    pub(crate) fn add_book_snapshot_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderBook>,
    ) {
        if self.book_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅订单簿快照 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.book_handlers.insert(topic, handler.clone());
        msgbus::subscribe_book_snapshots(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_book_snapshot_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.book_handlers.remove(&topic) {
            msgbus::unsubscribe_book_snapshots(topic.into(), &handler);
        }
    }

    pub(crate) fn add_mark_price_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<MarkPriceUpdate>,
    ) {
        if self.mark_price_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅标记价格 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.mark_price_handlers.insert(topic, handler.clone());
        msgbus::subscribe_mark_prices(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_mark_price_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.mark_price_handlers.remove(&topic) {
            msgbus::unsubscribe_mark_prices(topic.into(), &handler);
        }
    }

    pub(crate) fn add_index_price_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<IndexPriceUpdate>,
    ) {
        if self.index_price_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅指数价格 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.index_price_handlers.insert(topic, handler.clone());
        msgbus::subscribe_index_prices(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_index_price_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.index_price_handlers.remove(&topic) {
            msgbus::unsubscribe_index_prices(topic.into(), &handler);
        }
    }

    pub(crate) fn add_funding_rate_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<FundingRateUpdate>,
    ) {
        if self.funding_rate_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅资金费率 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.funding_rate_handlers.insert(topic, handler.clone());
        msgbus::subscribe_funding_rates(topic.into(), handler, None);
    }

    #[allow(dead_code)]
    pub(crate) fn remove_funding_rate_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.funding_rate_handlers.remove(&topic) {
            msgbus::unsubscribe_funding_rates(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_block_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<Block>,
    ) {
        if self.block_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅区块 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.block_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_blocks(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_block_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.block_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_blocks(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_pool_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<Pool>,
    ) {
        if self.pool_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅流动性池 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.pool_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_pools(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_pool_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.pool_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_pools(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_pool_swap_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<PoolSwap>,
    ) {
        if self.pool_swap_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅流动性池交换 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.pool_swap_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_swaps(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_pool_swap_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.pool_swap_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_swaps(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_pool_liquidity_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<PoolLiquidityUpdate>,
    ) {
        if self.pool_liquidity_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅流动性池流动性更新 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.pool_liquidity_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_liquidity(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_pool_liquidity_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.pool_liquidity_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_liquidity(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_pool_collect_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<PoolFeeCollect>,
    ) {
        if self.pool_collect_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅流动性池费用收取 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.pool_collect_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_collects(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_pool_collect_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.pool_collect_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_collects(topic.into(), &handler);
        }
    }

    #[cfg(feature = "defi")]
    pub(crate) fn add_pool_flash_subscription(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<PoolFlash>,
    ) {
        if self.pool_flash_handlers.contains_key(&topic) {
            log::warn!(
                "Actor {} 尝试重复订阅流动性池闪电贷 '{topic}'",
                self.actor_id
            );
            return;
        }
        self.pool_flash_handlers.insert(topic, handler.clone());
        msgbus::subscribe_defi_flash(topic.into(), handler, None);
    }

    #[cfg(feature = "defi")]
    #[allow(dead_code)]
    pub(crate) fn remove_pool_flash_subscription(&mut self, topic: MStr<Topic>) {
        if let Some(handler) = self.pool_flash_handlers.remove(&topic) {
            msgbus::unsubscribe_defi_flash(topic.into(), &handler);
        }
    }

    /// 创建一个新的 [`DataActorCore`] 实例。
    pub fn new(config: DataActorConfig) -> Self {
        let actor_id = config
            .actor_id
            .unwrap_or_else(|| Self::default_actor_id(&config));

        Self {
            actor_id,
            config,
            trader_id: None, // None until registered
            clock: None,     // None until registered
            cache: None,     // None until registered
            state: ComponentState::default(),
            topic_handlers: AHashMap::new(),
            deltas_handlers: AHashMap::new(),
            depth10_handlers: AHashMap::new(),
            book_handlers: AHashMap::new(),
            quote_handlers: AHashMap::new(),
            trade_handlers: AHashMap::new(),
            bar_handlers: AHashMap::new(),
            mark_price_handlers: AHashMap::new(),
            index_price_handlers: AHashMap::new(),
            funding_rate_handlers: AHashMap::new(),
            order_event_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            block_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            pool_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            pool_swap_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            pool_liquidity_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            pool_collect_handlers: AHashMap::new(),
            #[cfg(feature = "defi")]
            pool_flash_handlers: AHashMap::new(),
            warning_events: AHashSet::new(),
            pending_requests: AHashMap::new(),
            signal_classes: AHashMap::new(),
            #[cfg(feature = "indicators")]
            indicators: Indicators::default(),
        }
    }

    /// 将此实例的内存地址作为十六进制字符串返回。
    #[must_use]
    pub fn mem_address(&self) -> String {
        format!("{self:p}")
    }

    /// 返回 Actor 的状态。
    pub fn state(&self) -> ComponentState {
        self.state
    }

    /// 返回此 Actor 注册到的交易员 ID。
    pub fn trader_id(&self) -> Option<TraderId> {
        self.trader_id
    }

    /// 返回 Actor 的 ID。
    pub fn actor_id(&self) -> ActorId {
        self.actor_id
    }

    fn default_actor_id(config: &DataActorConfig) -> ActorId {
        let memory_address = std::ptr::from_ref(config) as usize;
        ActorId::from(format!("{}-{memory_address}", stringify!(DataActor)))
    }

    /// 从 Actor 的内部时钟获取 UNIX 纳秒时间戳。
    pub fn timestamp_ns(&self) -> UnixNanos {
        self.clock_ref().timestamp_ns()
    }

    /// 返回 Actor 的时钟（如果已注册）。
    ///
    /// # Panics
    ///
    /// 如果 Actor 尚未注册到交易员，则会 panic。
    pub fn clock(&mut self) -> RefMut<'_, dyn Clock> {
        self.clock
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "DataActor {} 在调用 `clock()` 之前必须先注册 - 交易员 ID: {:?}",
                    self.actor_id, self.trader_id
                )
            })
            .borrow_mut()
    }

    /// 返回引用计数时钟的一个克隆。
    ///
    /// # Panics
    ///
    /// 如果 Actor 尚未注册（时钟为 `None`），则会 panic。
    pub fn clock_rc(&self) -> Rc<RefCell<dyn Clock>> {
        self.clock
            .as_ref()
            .expect("DataActor 在访问时钟之前必须先注册")
            .clone()
    }

    fn clock_ref(&self) -> Ref<'_, dyn Clock> {
        self.clock
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "DataActor {} 在调用 `clock_ref()` 之前必须先注册 - 交易员 ID: {:?}",
                    self.actor_id, self.trader_id
                )
            })
            .borrow()
    }

    /// 返回缓存的只读引用。
    ///
    /// # Panics
    ///
    /// 如果 Actor 尚未注册（缓存为 `None`），则会 panic。
    pub fn cache(&self) -> Ref<'_, Cache> {
        self.cache
            .as_ref()
            .expect("DataActor 在访问缓存之前必须先注册")
            .borrow()
    }

    /// 返回引用计数缓存的一个克隆。
    ///
    /// # Panics
    ///
    /// 如果 Actor 尚未注册（缓存为 `None`），则会 panic。
    pub fn cache_rc(&self) -> Rc<RefCell<Cache>> {
        self.cache
            .as_ref()
            .expect("DataActor 在访问缓存之前必须先注册")
            .clone()
    }

    // -- REGISTRATION ----------------------------------------------------------------------------

    /// 向交易员注册数据 Actor。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果 Actor 已经注册到交易员，或者提供的依赖项无效，则返回错误。
    pub fn register(
        &mut self,
        trader_id: TraderId,
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
    ) -> anyhow::Result<()> {
        if let Some(existing_trader_id) = self.trader_id {
            anyhow::bail!(
                "DataActor {} 已经注册到交易员 {existing_trader_id}",
                self.actor_id
            );
        }

        // 通过尝试访问来验证时钟
        {
            let _timestamp = clock.borrow().timestamp_ns();
        }

        // 通过尝试访问来验证缓存
        {
            let _cache_borrow = cache.borrow();
        }

        self.trader_id = Some(trader_id);
        self.clock = Some(clock);
        self.cache = Some(cache);

        // 验证注册是否完成
        if !self.is_properly_registered() {
            anyhow::bail!(
                "DataActor {} 注册不完整 - 验证失败",
                self.actor_id
            );
        }

        log::debug!("已将 {} 注册到交易员 {trader_id}", self.actor_id);
        Ok(())
    }

    /// 为警告日志级别注册事件类型。
    pub fn register_warning_event(&mut self, event_type: &str) {
        self.warning_events.insert(event_type.to_string());
        log::debug!("已为警告日志注册事件类型 '{event_type}'");
    }

    /// 从警告日志级别注销事件类型。
    pub fn deregister_warning_event(&mut self, event_type: &str) {
        self.warning_events.remove(event_type);
        log::debug!("已从警告日志注销事件类型 '{event_type}'");
    }

    pub fn is_registered(&self) -> bool {
        self.trader_id.is_some()
    }

    pub(crate) fn check_registered(&self) {
        assert!(
            self.is_registered(),
            "Actor 尚未向交易员注册"
        );
    }

    /// 在不产生 panic 的情况下验证注册状态。
    fn is_properly_registered(&self) -> bool {
        self.trader_id.is_some() && self.clock.is_some() && self.cache.is_some()
    }

    pub(crate) fn send_data_cmd(&self, command: DataCommand) {
        if self.config.log_commands {
            log::info!("{CMD}{SEND} {command:?}");
        }

        let endpoint = MessagingSwitchboard::data_engine_queue_execute();
        msgbus::send_data_command(endpoint, command);
    }

    #[allow(dead_code)]
    fn send_data_req(&self, request: RequestCommand) {
        if self.config.log_commands {
            log::info!("{REQ}{SEND} {request:?}");
        }

        // 目前采用简化方法 - 不带动态处理程序的数据请求
        // TODO: 为响应处理程序实现适当的动态分发
        let endpoint = MessagingSwitchboard::data_engine_queue_execute();
        msgbus::send_any(endpoint, request.as_any());
    }

    /// 向系统发送带有可选原因的关机命令。
    ///
    /// # Panics
    ///
    /// 如果 Actor 未注册或没有交易员 ID，则会 panic。
    pub fn shutdown_system(&self, reason: Option<String>) {
        self.check_registered();

        // 安全性：在解包交易员 ID 之前检查是否已注册
        let command = ShutdownSystem::new(
            self.trader_id().unwrap(),
            self.actor_id.inner(),
            reason,
            UUID4::new(),
            self.timestamp_ns(),
        );

        let endpoint = "command.system.shutdown".into();
        msgbus::send_any(endpoint, command.as_any());
    }

    // -- SUBSCRIPTIONS ---------------------------------------------------------------------------

    /// 用于注册来自 trait 的数据订阅的辅助方法。
    ///
    /// # Panics
    ///
    /// 如果 Actor 未正确注册，则会 panic。
    pub fn subscribe_data(
        &mut self,
        handler: ShareableMessageHandler,
        data_type: DataType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        assert!(
            self.is_properly_registered(),
            "DataActor {} 未正确注册 - 交易员 ID: {:?}, 时钟: {}, 缓存: {}",
            self.actor_id,
            self.trader_id,
            self.clock.is_some(),
            self.cache.is_some()
        );

        let topic = get_custom_topic(&data_type);
        self.add_subscription_any(topic, handler);

        // 如果未指定客户端 ID，只需订阅该主题即可
        if client_id.is_none() {
            return;
        }

        let command = SubscribeCommand::Data(SubscribeCustomData {
            data_type,
            client_id,
            venue: None,
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的报价订阅的辅助方法。
    pub fn subscribe_quotes(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<QuoteTick>,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_quote_subscription(topic, handler);

        let command = SubscribeCommand::Quotes(SubscribeQuotes {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的多工具定义订阅的辅助方法。
    pub fn subscribe_instruments(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
        venue: Venue,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_instrument_subscription(topic, handler);

        let command = SubscribeCommand::Instruments(SubscribeInstruments {
            client_id,
            venue,
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的工具定义订阅的辅助方法。
    pub fn subscribe_instrument(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_instrument_subscription(topic, handler);

        let command = SubscribeCommand::Instrument(SubscribeInstrument {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的订单簿增量订阅的辅助方法。
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe_book_deltas(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderBookDeltas>,
        instrument_id: InstrumentId,
        book_type: BookType,
        depth: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        managed: bool,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_deltas_subscription(topic, handler);

        let command = SubscribeCommand::BookDeltas(SubscribeBookDeltas {
            instrument_id,
            book_type,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            depth,
            managed,
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的订单簿快照订阅的辅助方法。
    #[allow(clippy::too_many_arguments)]
    pub fn subscribe_book_at_interval(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderBook>,
        instrument_id: InstrumentId,
        book_type: BookType,
        depth: Option<NonZeroUsize>,
        interval_ms: NonZeroUsize,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_book_snapshot_subscription(topic, handler);

        let command = SubscribeCommand::BookSnapshots(SubscribeBookSnapshots {
            instrument_id,
            book_type,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            depth,
            interval_ms,
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的逐笔成交订阅的辅助方法。
    pub fn subscribe_trades(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<TradeTick>,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_trade_subscription(topic, handler);

        let command = SubscribeCommand::Trades(SubscribeTrades {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的 K 线订阅的辅助方法。
    pub fn subscribe_bars(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<Bar>,
        bar_type: BarType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_bar_subscription(topic, handler);

        let command = SubscribeCommand::Bars(SubscribeBars {
            bar_type,
            client_id,
            venue: Some(bar_type.instrument_id().venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的标记价格订阅的辅助方法。
    pub fn subscribe_mark_prices(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<MarkPriceUpdate>,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_mark_price_subscription(topic, handler);

        let command = SubscribeCommand::MarkPrices(SubscribeMarkPrices {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的指数价格订阅的辅助方法。
    pub fn subscribe_index_prices(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<IndexPriceUpdate>,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_index_price_subscription(topic, handler);

        let command = SubscribeCommand::IndexPrices(SubscribeIndexPrices {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的资金费率订阅的辅助方法。
    pub fn subscribe_funding_rates(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<FundingRateUpdate>,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_funding_rate_subscription(topic, handler);

        let command = SubscribeCommand::FundingRates(SubscribeFundingRates {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的工具状态订阅的辅助方法。
    pub fn subscribe_instrument_status(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_subscription_any(topic, handler);

        let command = SubscribeCommand::InstrumentStatus(SubscribeInstrumentStatus {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的工具收盘订阅的辅助方法。
    pub fn subscribe_instrument_close(
        &mut self,
        topic: MStr<Topic>,
        handler: ShareableMessageHandler,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        self.add_instrument_close_subscription(topic, handler);

        let command = SubscribeCommand::InstrumentClose(SubscribeInstrumentClose {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Subscribe(command));
    }

    /// 用于注册来自 trait 的订单成交订阅的辅助方法。
    pub fn subscribe_order_fills(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderEventAny>,
    ) {
        self.check_registered();
        self.add_order_event_subscription(topic, handler);
    }

    /// 用于注册来自 trait 的订单取消订阅的辅助方法。
    pub fn subscribe_order_cancels(
        &mut self,
        topic: MStr<Topic>,
        handler: TypedHandler<OrderEventAny>,
    ) {
        self.check_registered();
        self.add_order_event_subscription(topic, handler);
    }

    /// 用于取消订阅数据的辅助方法。
    pub fn unsubscribe_data(
        &mut self,
        data_type: DataType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_custom_topic(&data_type);
        self.remove_subscription_any(topic);

        if client_id.is_none() {
            return;
        }

        let command = UnsubscribeCommand::Data(UnsubscribeCustomData {
            data_type,
            client_id,
            venue: None,
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于从 `instruments`（工具定义）取消订阅的辅助方法。
    pub fn unsubscribe_instruments(
        &mut self,
        venue: Venue,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_instruments_topic(venue);
        self.remove_instrument_subscription(topic);

        let command = UnsubscribeCommand::Instruments(UnsubscribeInstruments {
            client_id,
            venue,
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于从 `instrument`（工具定义）取消订阅的辅助方法。
    pub fn unsubscribe_instrument(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_instrument_topic(instrument_id);
        self.remove_instrument_subscription(topic);

        let command = UnsubscribeCommand::Instrument(UnsubscribeInstrument {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于从 `book deltas`（订单簿增量）取消订阅的辅助方法。
    pub fn unsubscribe_book_deltas(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_book_deltas_topic(instrument_id);
        self.remove_deltas_subscription(topic);

        let command = UnsubscribeCommand::BookDeltas(UnsubscribeBookDeltas {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅指定时间间隔的订单簿快照的辅助方法。
    pub fn unsubscribe_book_at_interval(
        &mut self,
        instrument_id: InstrumentId,
        interval_ms: NonZeroUsize,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_book_snapshots_topic(instrument_id, interval_ms);
        self.remove_book_snapshot_subscription(topic);

        let command = UnsubscribeCommand::BookSnapshots(UnsubscribeBookSnapshots {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅报价的辅助方法。
    pub fn unsubscribe_quotes(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_quotes_topic(instrument_id);
        self.remove_quote_subscription(topic);

        let command = UnsubscribeCommand::Quotes(UnsubscribeQuotes {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅逐笔成交的辅助方法。
    pub fn unsubscribe_trades(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_trades_topic(instrument_id);
        self.remove_trade_subscription(topic);

        let command = UnsubscribeCommand::Trades(UnsubscribeTrades {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅 K 线的辅助方法。
    pub fn unsubscribe_bars(
        &mut self,
        bar_type: BarType,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_bars_topic(bar_type);
        self.remove_bar_subscription(topic);

        let command = UnsubscribeCommand::Bars(UnsubscribeBars {
            bar_type,
            client_id,
            venue: Some(bar_type.instrument_id().venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅标记价格的辅助方法。
    pub fn unsubscribe_mark_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_mark_price_topic(instrument_id);
        self.remove_mark_price_subscription(topic);

        let command = UnsubscribeCommand::MarkPrices(UnsubscribeMarkPrices {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅指数价格的辅助方法。
    pub fn unsubscribe_index_prices(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_index_price_topic(instrument_id);
        self.remove_index_price_subscription(topic);

        let command = UnsubscribeCommand::IndexPrices(UnsubscribeIndexPrices {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅资金费率的辅助方法。
    pub fn unsubscribe_funding_rates(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_funding_rate_topic(instrument_id);
        self.remove_funding_rate_subscription(topic);

        let command = UnsubscribeCommand::FundingRates(UnsubscribeFundingRates {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅工具状态的辅助方法。
    pub fn unsubscribe_instrument_status(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_instrument_status_topic(instrument_id);
        self.remove_subscription_any(topic);

        let command = UnsubscribeCommand::InstrumentStatus(UnsubscribeInstrumentStatus {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅工具收盘的辅助方法。
    pub fn unsubscribe_instrument_close(
        &mut self,
        instrument_id: InstrumentId,
        client_id: Option<ClientId>,
        params: Option<Params>,
    ) {
        self.check_registered();

        let topic = get_instrument_close_topic(instrument_id);
        self.remove_instrument_close_subscription(topic);

        let command = UnsubscribeCommand::InstrumentClose(UnsubscribeInstrumentClose {
            instrument_id,
            client_id,
            venue: Some(instrument_id.venue),
            command_id: UUID4::new(),
            ts_init: self.timestamp_ns(),
            correlation_id: None,
            params,
        });

        self.send_data_cmd(DataCommand::Unsubscribe(command));
    }

    /// 用于取消订阅订单成交的辅助方法。
    pub fn unsubscribe_order_fills(&mut self, instrument_id: InstrumentId) {
        self.check_registered();

        let topic = get_order_fills_topic(instrument_id);
        self.remove_order_event_subscription(topic);
    }

    /// 用于取消订阅订单取消的辅助方法。
    pub fn unsubscribe_order_cancels(&mut self, instrument_id: InstrumentId) {
        self.check_registered();

        let topic = get_order_cancels_topic(instrument_id);
        self.remove_order_event_subscription(topic);
    }

    /// 用于请求数据的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn request_data(
        &self,
        data_type: DataType,
        client_id: ClientId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Data(RequestCustomData {
            client_id,
            data_type,
            start,
            end,
            limit,
            request_id,
            ts_init: self.timestamp_ns(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求工具定义的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    pub fn request_instrument(
        &self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Instrument(RequestInstrument {
            instrument_id,
            start,
            end,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求多工具定义的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    pub fn request_instruments(
        &self,
        venue: Option<Venue>,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Instruments(RequestInstruments {
            venue,
            start,
            end,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求订单簿快照的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    pub fn request_book_snapshot(
        &self,
        instrument_id: InstrumentId,
        depth: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let request_id = UUID4::new();
        let command = RequestCommand::BookSnapshot(RequestBookSnapshot {
            instrument_id,
            depth,
            client_id,
            request_id,
            ts_init: self.timestamp_ns(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求报价的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn request_quotes(
        &self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Quotes(RequestQuotes {
            instrument_id,
            start,
            end,
            limit,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求逐笔成交的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn request_trades(
        &self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Trades(RequestTrades {
            instrument_id,
            start,
            end,
            limit,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求资金费率的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn request_funding_rates(
        &self,
        instrument_id: InstrumentId,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::FundingRates(RequestFundingRates {
            instrument_id,
            start,
            end,
            limit,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    /// 用于请求 K 线的辅助方法。
    ///
    /// # 错误 (Errors)
    ///
    /// 如果输入参数无效，则返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn request_bars(
        &self,
        bar_type: BarType,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        limit: Option<NonZeroUsize>,
        client_id: Option<ClientId>,
        params: Option<Params>,
        handler: ShareableMessageHandler,
    ) -> anyhow::Result<UUID4> {
        self.check_registered();

        let now = self.clock_ref().utc_now();
        check_timestamps(now, start, end)?;

        let request_id = UUID4::new();
        let command = RequestCommand::Bars(RequestBars {
            bar_type,
            start,
            end,
            limit,
            client_id,
            request_id,
            ts_init: now.into(),
            params,
        });

        get_message_bus()
            .borrow_mut()
            .register_response_handler(command.request_id(), handler)?;

        self.send_data_cmd(DataCommand::Request(command));

        Ok(request_id)
    }

    #[cfg(test)]
    pub fn quote_handler_count(&self) -> usize {
        self.quote_handlers.len()
    }

    #[cfg(test)]
    pub fn trade_handler_count(&self) -> usize {
        self.trade_handlers.len()
    }

    #[cfg(test)]
    pub fn bar_handler_count(&self) -> usize {
        self.bar_handlers.len()
    }

    #[cfg(test)]
    pub fn deltas_handler_count(&self) -> usize {
        self.deltas_handlers.len()
    }

    #[cfg(test)]
    pub fn has_quote_handler(&self, topic: &str) -> bool {
        self.quote_handlers
            .contains_key(&MStr::<Topic>::from(topic))
    }

    #[cfg(test)]
    pub fn has_trade_handler(&self, topic: &str) -> bool {
        self.trade_handlers
            .contains_key(&MStr::<Topic>::from(topic))
    }

    #[cfg(test)]
    pub fn has_bar_handler(&self, topic: &str) -> bool {
        self.bar_handlers.contains_key(&MStr::<Topic>::from(topic))
    }

    #[cfg(test)]
    pub fn has_deltas_handler(&self, topic: &str) -> bool {
        self.deltas_handlers
            .contains_key(&MStr::<Topic>::from(topic))
    }
}

fn check_timestamps(
    now: DateTime<Utc>,
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
) -> anyhow::Result<()> {
    if let Some(start) = start {
        check_predicate_true(start <= now, "开始时间大于当前时间")?;
    }
    if let Some(end) = end {
        check_predicate_true(end <= now, "结束时间大于当前时间")?;
    }

    if let (Some(start), Some(end)) = (start, end) {
        check_predicate_true(start < end, "开始时间大于或等于结束时间")?;
    }

    Ok(())
}

fn log_error(e: &anyhow::Error) {
    log::error!("{e}");
}

fn log_not_running<T>(msg: &T)
where
    T: Debug,
{
    log::trace!("在未运行时接收到消息 - 正在跳过 {msg:?}");
}

fn log_received<T>(msg: &T)
where
    T: Debug,
{
    log::debug!("{RECV} {msg:?}");
}
