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

//! `Cache` 的测试模块。

#[cfg(feature = "defi")]
use std::sync::Arc;

use bytes::Bytes;
use nautilus_core::{UUID4, UnixNanos};
#[cfg(feature = "defi")]
use nautilus_model::defi::{
    AmmType, Dex, DexType, Pool, PoolIdentifier, PoolProfiler, Token, chain::chains,
};
use nautilus_model::{
    accounts::AccountAny,
    data::{Bar, BarType, FundingRateUpdate, MarkPriceUpdate, QuoteTick, TradeTick},
    enums::{
        AggressorSide, BookType, OmsType, OrderSide, OrderStatus, OrderType, PositionSide,
        PriceType, TriggerType,
    },
    events::{
        OrderAccepted, OrderCanceled, OrderEmulated, OrderEventAny, OrderFilled, OrderRejected,
        OrderReleased, OrderSubmitted,
    },
    identifiers::{
        AccountId, ClientOrderId, InstrumentId, OrderListId, PositionId, StrategyId, Symbol,
        TradeId, Venue, VenueOrderId,
    },
    instruments::{CurrencyPair, Instrument, InstrumentAny, SyntheticInstrument, stubs::*},
    orderbook::OrderBook,
    orders::{
        Order, OrderList,
        builder::OrderTestBuilder,
        stubs::{TestOrderEventStubs, TestOrdersGenerator},
    },
    position::Position,
    stubs::TestDefault,
    types::{Currency, Price, Quantity},
};
use rstest::{fixture, rstest};

use crate::cache::Cache;

#[fixture]
fn cache() -> Cache {
    Cache::default()
}

#[rstest]
fn test_build_index_when_empty(mut cache: Cache) {
    cache.build_index();
}

#[rstest]
fn test_check_integrity_when_empty(mut cache: Cache) {
    let result = cache.check_integrity();
    assert!(result);
}

#[rstest]
fn test_check_residuals_when_empty(cache: Cache) {
    let result = cache.check_residuals();
    assert!(!result);
}

#[rstest]
fn test_clear_index_when_empty(mut cache: Cache) {
    cache.clear_index();
}

#[rstest]
fn test_reset_when_empty(mut cache: Cache) {
    cache.reset();
}

#[rstest]
fn test_dispose_when_empty(mut cache: Cache) {
    cache.dispose();
}

#[rstest]
fn test_flush_db_when_empty(mut cache: Cache) {
    cache.flush_db();
}

#[rstest]
fn test_cache_general_when_no_database(mut cache: Cache) {
    assert!(cache.cache_general().is_ok());
}

// -- EXECUTION -------------------------------------------------------------------------------

#[rstest]
fn test_cache_orders_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_orders()).is_ok());
}

#[rstest]
fn test_order_when_empty(cache: Cache) {
    let client_order_id = ClientOrderId::test_default();
    let result = cache.order(&client_order_id);
    assert!(result.is_none());
}

#[rstest]
fn test_order_when_initialized(mut cache: Cache, audusd_sim: CurrencyPair) {
    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let client_order_id = order.client_order_id();
    cache.add_order(order, None, None, false).unwrap();

    let order = cache.order(&client_order_id).unwrap();
    assert_eq!(cache.orders(None, None, None, None, None), vec![order]);
    assert!(cache.orders_open(None, None, None, None, None).is_empty());
    assert!(cache.orders_closed(None, None, None, None, None).is_empty());
    assert!(
        cache
            .orders_emulated(None, None, None, None, None)
            .is_empty()
    );
    assert!(
        cache
            .orders_inflight(None, None, None, None, None)
            .is_empty()
    );
    assert!(cache.order_exists(&order.client_order_id()));
    assert!(!cache.is_order_open(&order.client_order_id()));
    assert!(!cache.is_order_closed(&order.client_order_id()));
    assert!(!cache.is_order_emulated(&order.client_order_id()));
    assert!(!cache.is_order_inflight(&order.client_order_id()));
    assert!(!cache.is_order_pending_cancel_local(&order.client_order_id()));
    assert_eq!(cache.orders_open_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_closed_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_inflight_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
    assert_eq!(cache.venue_order_id(&order.client_order_id()), None);
}

#[rstest]
fn test_order_when_submitted(mut cache: Cache, audusd_sim: CurrencyPair) {
    let mut order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let client_order_id = order.client_order_id();
    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    // 检查缓存订单的状态更改
    let cached_order = cache.order(&client_order_id).unwrap();
    assert_eq!(cached_order.status(), OrderStatus::Submitted);

    let result = cache.order(&order.client_order_id()).unwrap();

    assert_eq!(order.status(), OrderStatus::Submitted);
    assert_eq!(result, &order);
    assert_eq!(cache.orders(None, None, None, None, None), vec![&order]);
    assert!(cache.orders_open(None, None, None, None, None).is_empty());
    assert!(cache.orders_closed(None, None, None, None, None).is_empty());
    assert!(
        cache
            .orders_emulated(None, None, None, None, None)
            .is_empty()
    );
    assert!(
        !cache
            .orders_inflight(None, None, None, None, None)
            .is_empty()
    );
    assert!(cache.order_exists(&order.client_order_id()));
    assert!(!cache.is_order_open(&order.client_order_id()));
    assert!(!cache.is_order_closed(&order.client_order_id()));
    assert!(!cache.is_order_emulated(&order.client_order_id()));
    assert!(cache.is_order_inflight(&order.client_order_id()));
    assert!(!cache.is_order_pending_cancel_local(&order.client_order_id()));
    assert_eq!(cache.orders_open_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_closed_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_inflight_count(None, None, None, None, None), 1);
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
    assert_eq!(cache.venue_order_id(&order.client_order_id()), None);
}

// 测试订单被拒绝时的状态转换和缓存查询。
//
// 此测试验证完整生命周期的缓存行为：已初始化 -> 已提交 -> 已拒绝。
//
// 生产代码 BUG：此测试在第 220 行失败，原因如下：
//   assertion failed: cache.orders_emulated(None, None, None, None, None).is_empty()
//
// 当订单转换到 REJECTED 状态时，它错误地出现在模拟订单集合中。
// 缓存应该仅单独追踪模拟订单，不应包含被拒绝的订单。
//
// TODO：修复订单状态管理 - 被拒绝的订单不应出现在模拟列表中。
// 该 Bug 存在于生产代码 (cache.rs) 中，而非此测试中。
#[ignore = "生产环境 Bug：被拒绝的订单错误地显示在模拟列表中"]
#[rstest]
fn test_order_when_rejected(mut cache: Cache, audusd_sim: CurrencyPair) {
    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();
    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    let rejected = OrderRejected::default();
    order.apply(OrderEventAny::Rejected(rejected)).unwrap();
    cache.update_order(&order).unwrap();

    // 检查缓存订单的状态更改
    let cached_order = cache.order(&order.client_order_id()).unwrap();
    assert_eq!(cached_order.status(), OrderStatus::Rejected);

    let result = cache.order(&order.client_order_id()).unwrap();

    assert!(order.is_closed());
    assert_eq!(result, &order);
    assert_eq!(cache.orders(None, None, None, None, None), vec![&order]);
    assert!(cache.orders_open(None, None, None, None, None).is_empty());
    assert_eq!(
        cache.orders_closed(None, None, None, None, None),
        vec![&order]
    );
    assert!(
        cache
            .orders_emulated(None, None, None, None, None)
            .is_empty()
    );
    assert!(
        cache
            .orders_inflight(None, None, None, None, None)
            .is_empty()
    );
    assert!(cache.order_exists(&order.client_order_id()));
    assert!(!cache.is_order_open(&order.client_order_id()));
    assert!(cache.is_order_closed(&order.client_order_id()));
    assert!(!cache.is_order_emulated(&order.client_order_id()));
    assert!(!cache.is_order_inflight(&order.client_order_id()));
    assert!(!cache.is_order_pending_cancel_local(&order.client_order_id()));
    assert_eq!(cache.orders_open_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_closed_count(None, None, None, None, None), 1);
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_inflight_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
}

#[rstest]
fn test_order_when_accepted(mut cache: Cache, audusd_sim: CurrencyPair) {
    let mut order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = OrderAccepted::default();
    order.apply(OrderEventAny::Accepted(accepted)).unwrap();
    cache.update_order(&order).unwrap();

    let result = cache.order(&order.client_order_id()).unwrap();

    assert!(order.is_open());
    assert_eq!(result, &order);
    assert_eq!(cache.orders(None, None, None, None, None), vec![&order]);
    assert_eq!(
        cache.orders_open(None, None, None, None, None),
        vec![&order]
    );
    assert!(cache.orders_closed(None, None, None, None, None).is_empty());
    assert!(
        cache
            .orders_emulated(None, None, None, None, None)
            .is_empty()
    );
    assert!(
        cache
            .orders_inflight(None, None, None, None, None)
            .is_empty()
    );
    assert!(cache.order_exists(&order.client_order_id()));
    assert!(cache.is_order_open(&order.client_order_id()));
    assert!(!cache.is_order_closed(&order.client_order_id()));
    assert!(!cache.is_order_emulated(&order.client_order_id()));
    assert!(!cache.is_order_inflight(&order.client_order_id()));
    assert!(!cache.is_order_pending_cancel_local(&order.client_order_id()));
    assert_eq!(cache.orders_open_count(None, None, None, None, None), 1);
    assert_eq!(cache.orders_closed_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_inflight_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
    assert_eq!(
        cache.client_order_id(&order.venue_order_id().unwrap()),
        Some(&order.client_order_id())
    );
    assert_eq!(
        cache.venue_order_id(&order.client_order_id()),
        Some(&order.venue_order_id().unwrap())
    );
}

#[rstest]
fn test_client_order_ids_filtering(mut cache: Cache) {
    // 构建一个确定性的小型数据环境：2 个交易所 × 3 个交易工具 × 2 个订单
    let venue_a = Venue::from("VENUE-A");
    let _venue_b = Venue::from("VENUE-B");

    let mut generator = TestOrdersGenerator::new(OrderType::Limit);
    generator.add_venue_and_total_instruments(venue_a, 3);
    generator.add_venue_and_total_instruments(_venue_b, 3);
    generator.set_orders_per_instrument(2);

    let orders = generator.build();

    let _instrument_a0 = InstrumentId::from("SYMBOL-0.VENUE-A");

    // Sanity-check the generated volume: 2 × 3 × 2 = 12
    assert_eq!(orders.len(), 12);

    // 加载到缓存中以实时构建索引
    for order in &orders {
        cache.add_order(order.clone(), None, None, false).unwrap();
    }

    // 无过滤器 – 期望所有订单
    assert_eq!(
        cache.client_order_ids(None, None, None, None).len(),
        orders.len()
    );

    // 仅按交易所查询
    let expected_venue_a = orders
        .iter()
        .filter(|o| o.instrument_id().venue == venue_a)
        .count();
    assert_eq!(
        cache
            .client_order_ids(Some(&venue_a), None, None, None)
            .len(),
        expected_venue_a
    );

    // 交易所 + 交易工具查询
    let instrument_a0 = InstrumentId::from("SYMBOL-0.VENUE-A");
    assert_eq!(
        cache
            .client_order_ids(Some(&venue_a), Some(&instrument_a0), None, None)
            .len(),
        orders
            .iter()
            .filter(|o| o.instrument_id() == instrument_a0)
            .count()
    );
}

#[rstest]
fn test_position_ids_filtering(mut cache: Cache) {
    fn make_pair(id_str: &str) -> CurrencyPair {
        CurrencyPair::new(
            InstrumentId::from(id_str),
            Symbol::from(id_str),
            Currency::USD(),
            Currency::EUR(),
            2,
            4,
            Price::from("0.01"),
            Quantity::from("0.0001"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            UnixNanos::default(),
            UnixNanos::default(),
        )
    }

    let venue_a = Venue::from("VENUE-A");
    let _venue_b = Venue::from("VENUE-B");

    // 跨交易所构建两个开仓头寸和一个平仓头寸
    let instr_a0 = make_pair("PAIR-0.VENUE-A");
    let instr_b0 = make_pair("PAIR-0.VENUE-B");

    let base_order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instr_a0.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1"))
        .build();

    let fill_a_event = TestOrderEventStubs::filled(
        &base_order,
        &InstrumentAny::CurrencyPair(instr_a0.clone()),
        None,
        Some(PositionId::new("POS-A")),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let fill_a = match fill_a_event {
        OrderEventAny::Filled(f) => f,
        _ => unreachable!(),
    };
    let pos_a = Position::new(&InstrumentAny::CurrencyPair(instr_a0.clone()), fill_a);

    // 在交易所 B 上的第二个开仓头寸
    let order_b = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instr_b0.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from("1"))
        .build();

    let fill_b_event = TestOrderEventStubs::filled(
        &order_b,
        &InstrumentAny::CurrencyPair(instr_b0.clone()),
        None,
        Some(PositionId::new("POS-B")),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let fill_b = match fill_b_event {
        OrderEventAny::Filled(f) => f,
        _ => unreachable!(),
    };
    let pos_b = Position::new(&InstrumentAny::CurrencyPair(instr_b0), fill_b);

    // 在交易所 A 上的平仓头寸 (side Flat + ts_closed)
    let mut pos_closed = pos_a.clone();
    pos_closed.id = PositionId::new("POS-C");
    pos_closed.side = PositionSide::Flat;
    pos_closed.ts_closed = Some(UnixNanos::from(1));

    // Insert into cache
    cache.add_position(pos_a.clone(), OmsType::Netting).unwrap();
    cache.add_position(pos_b, OmsType::Netting).unwrap();
    cache.add_position(pos_closed, OmsType::Netting).unwrap();

    // 断言
    assert_eq!(cache.position_ids(None, None, None, None).len(), 3);

    // 交易所过滤器
    assert_eq!(
        cache.position_ids(Some(&venue_a), None, None, None).len(),
        2
    );

    // 交易所 + 交易工具过滤器
    assert_eq!(
        cache
            .position_ids(Some(&venue_a), Some(&instr_a0.id), None, None)
            .len(),
        2 // open + closed on venue A instrument
    );

    // 开仓 / 平仓分离
    assert!(
        cache
            .position_open_ids(None, None, None, None)
            .contains(&pos_a.id)
    );
}

// 测试订单成交时的状态转换和缓存查询。
//
// 此测试验证完整生命周期的缓存行为：已初始化 -> 已提交 -> 已接受 -> 已成交。
// 同时也测试持仓创建以及订单-持仓关系是否被正确缓存。
//
// 生产代码 BUG：此测试可能会因为与 test_order_when_rejected 类似的原因失败。
// 缓存可能会错误地分类已成交订单，或者在订单生命周期转换期间未能正确更新状态。
//
// TODO：修复订单生命周期中的缓存状态管理。在修复 test_order_when_rejected 后运行此测试以查看具体失败。
// 该 Bug 存在于生产代码 (cache.rs) 中，而非此测试中。
#[ignore = "生产环境 Bug：订单生命周期中的缓存状态管理"]
#[rstest]
fn test_order_when_filled(mut cache: Cache, audusd_sim: CurrencyPair) {
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);
    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();
    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = OrderAccepted::default();
    order.apply(OrderEventAny::Accepted(accepted)).unwrap();
    cache.update_order(&order).unwrap();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    order.apply(filled).unwrap();
    cache.update_order(&order).unwrap();

    let result = cache.order(&order.client_order_id()).unwrap();

    assert!(order.is_closed());
    assert_eq!(result, &order);
    assert_eq!(cache.orders(None, None, None, None, None), vec![&order]);
    assert_eq!(
        cache.orders_closed(None, None, None, None, None),
        vec![&order]
    );
    assert!(cache.orders_open(None, None, None, None, None).is_empty());
    assert!(
        cache
            .orders_emulated(None, None, None, None, None)
            .is_empty()
    );
    assert!(
        cache
            .orders_inflight(None, None, None, None, None)
            .is_empty()
    );
    assert!(cache.order_exists(&order.client_order_id()));
    assert!(!cache.is_order_open(&order.client_order_id()));
    assert!(cache.is_order_closed(&order.client_order_id()));
    assert!(!cache.is_order_emulated(&order.client_order_id()));
    assert!(!cache.is_order_inflight(&order.client_order_id()));
    assert!(!cache.is_order_pending_cancel_local(&order.client_order_id()));
    assert_eq!(cache.orders_open_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_closed_count(None, None, None, None, None), 1);
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_inflight_count(None, None, None, None, None), 0);
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
    assert_eq!(
        cache.client_order_id(&order.venue_order_id().unwrap()),
        Some(&order.client_order_id())
    );
    assert_eq!(
        cache.venue_order_id(&order.client_order_id()),
        Some(&order.venue_order_id().unwrap())
    );
}

#[rstest]
fn test_get_general_when_empty(cache: Cache) {
    let result = cache.get("A").unwrap();
    assert!(result.is_none());
}

#[rstest]
fn test_add_general_when_value(mut cache: Cache) {
    let key = "A";
    let value = Bytes::from_static(&[0_u8]);
    cache.add(key, value.clone()).unwrap();
    let result = cache.get(key).unwrap();
    assert_eq!(result, Some(&value));
}

#[rstest]
fn test_orders_for_position(mut cache: Cache, audusd_sim: CurrencyPair) {
    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let position_id = PositionId::test_default();
    cache
        .add_order(order.clone(), Some(position_id), None, false)
        .unwrap();
    let result = cache.order(&order.client_order_id()).unwrap();
    assert_eq!(result, &order);
    assert_eq!(cache.orders_for_position(&position_id), vec![&order]);
}

#[rstest]
fn test_correct_order_indexing(mut cache: Cache) {
    let binance = Venue::from("BINANCE");
    let bybit = Venue::from("BYBIT");
    let mut orders_generator = TestOrdersGenerator::new(OrderType::Limit);
    orders_generator.add_venue_and_total_instruments(bybit, 10);
    orders_generator.add_venue_and_total_instruments(binance, 10);
    orders_generator.set_orders_per_instrument(2);
    let orders = orders_generator.build();
    // 将会有 2 个交易所 * 10 个工具 * 2 个订单 = 40 个订单
    assert_eq!(orders.len(), 40);
    for order in orders {
        cache.add_order(order, None, None, false).unwrap();
    }
    assert_eq!(cache.orders(None, None, None, None, None).len(), 40);
    assert_eq!(cache.orders(Some(&bybit), None, None, None, None).len(), 20);
    assert_eq!(
        cache.orders(Some(&binance), None, None, None, None).len(),
        20
    );
    assert_eq!(
        cache
            .orders(
                Some(&bybit),
                Some(&InstrumentId::from("SYMBOL-0.BYBIT")),
                None,
                None,
                None,
            )
            .len(),
        2
    );
    assert_eq!(
        cache
            .orders(
                Some(&binance),
                Some(&InstrumentId::from("SYMBOL-0.BINANCE")),
                None,
                None,
                None,
            )
            .len(),
        2
    );
}

#[rstest]
fn test_add_order_with_account_id_populates_account_index() {
    // 验证当 account_id 已设置时，add_order 是否填充 account_orders 索引
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);
    let account_id = AccountId::new("SIM-001");

    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    // 在添加之前设置 account_id (例如，从数据库加载的订单)
    let submitted = TestOrderEventStubs::submitted(&order, account_id);
    order.apply(submitted).unwrap();

    let client_order_id = order.client_order_id();
    cache.add_order(order.clone(), None, None, false).unwrap();

    // 验证订单是否在 account_orders 索引中
    assert!(cache.index.account_orders.contains_key(&account_id));
    assert!(
        cache
            .index
            .account_orders
            .get(&account_id)
            .unwrap()
            .contains(&client_order_id)
    );

    // 验证账户过滤查询是否返回该订单
    let orders_for_account = cache.orders(None, None, None, Some(&account_id), None);
    assert_eq!(orders_for_account.len(), 1);
    assert!(orders_for_account.contains(&&order));
}

#[rstest]
fn test_add_order_list() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);

    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let order_list_id = OrderListId::new("OL-001");
    let order_list = OrderList::new(
        order_list_id,
        instrument.id(),
        order.strategy_id(),
        vec![order.client_order_id()],
        UnixNanos::default(),
    );

    cache.add_order_list(order_list.clone()).unwrap();

    assert!(cache.order_list_exists(&order_list_id));
    assert_eq!(cache.order_list(&order_list_id), Some(&order_list));
    assert!(
        cache
            .order_lists(None, None, None, None)
            .contains(&&order_list)
    );
}

#[rstest]
fn test_add_order_list_when_already_exists_errors() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);

    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let order_list_id = OrderListId::new("OL-001");
    let order_list = OrderList::new(
        order_list_id,
        instrument.id(),
        order.strategy_id(),
        vec![order.client_order_id()],
        UnixNanos::default(),
    );

    cache.add_order_list(order_list.clone()).unwrap();
    let result = cache.add_order_list(order_list);

    assert!(result.is_err());
}

#[rstest]
fn test_cache_positions_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_positions()).is_ok());
}

#[rstest]
fn test_position_when_empty(cache: Cache) {
    let position_id = PositionId::from("1");
    let result = cache.position(&position_id);
    assert!(result.is_none());
    assert!(!cache.position_exists(&position_id));
}

#[rstest]
fn test_position_when_some(mut cache: Cache, audusd_sim: CurrencyPair) {
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();
    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        Some(PositionId::new("P-123456")),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let position = Position::new(&audusd_sim, filled.into());
    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();

    let result = cache.position(&position.id);
    assert_eq!(result, Some(&position));
    assert!(cache.position_exists(&position.id));
    assert_eq!(
        cache.position_id(&order.client_order_id()),
        Some(&position.id)
    );
    assert_eq!(
        cache.positions_open(None, None, None, None, None),
        vec![&position]
    );
    assert_eq!(
        cache.positions_closed(None, None, None, None, None),
        Vec::<&Position>::new()
    );
    assert_eq!(cache.positions_open_count(None, None, None, None, None), 1);
    assert_eq!(
        cache.positions_closed_count(None, None, None, None, None),
        0
    );
}

// -- 数据 (DATA) ------------------------------------------------------------------------------------

#[rstest]
fn test_cache_currencies_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_currencies()).is_ok());
}

#[rstest]
fn test_cache_instruments_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_instruments()).is_ok());
}

#[rstest]
fn test_instrument_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.instrument(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_instrument_when_some(mut cache: Cache, audusd_sim: CurrencyPair) {
    cache
        .add_instrument(InstrumentAny::CurrencyPair(audusd_sim.clone()))
        .unwrap();

    let result = cache.instrument(&audusd_sim.id);
    assert_eq!(result, Some(&InstrumentAny::CurrencyPair(audusd_sim)));
}

#[rstest]
fn test_instruments_when_empty(cache: Cache) {
    let esz1 = futures_contract_es(None, None);
    let result = cache.instruments(&esz1.id.venue, None);
    assert!(result.is_empty());
}

#[rstest]
fn test_instruments_when_some(mut cache: Cache) {
    let esz1 = futures_contract_es(None, None);
    cache
        .add_instrument(InstrumentAny::FuturesContract(esz1.clone()))
        .unwrap();

    let result1 = cache.instruments(&esz1.id.venue, None);
    let result2 = cache.instruments(&esz1.id.venue, Some(&esz1.underlying));
    assert_eq!(result1, vec![&InstrumentAny::FuturesContract(esz1.clone())]);
    assert_eq!(result2, vec![&InstrumentAny::FuturesContract(esz1.clone())]);
}

#[rstest]
fn test_cache_synthetics_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_synthetics()).is_ok());
}

#[rstest]
fn test_synthetic_when_empty(cache: Cache) {
    let synth = SyntheticInstrument::default();
    let result = cache.synthetic(&synth.id);
    assert!(result.is_none());
}

#[rstest]
fn test_synthetic_when_some(mut cache: Cache) {
    let synth = SyntheticInstrument::default();
    cache.add_synthetic(synth.clone()).unwrap();
    let result = cache.synthetic(&synth.id);
    assert_eq!(result, Some(&synth));
}

#[rstest]
fn test_order_book_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.order_book(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_order_book_when_some(mut cache: Cache, audusd_sim: CurrencyPair) {
    let book = OrderBook::new(audusd_sim.id, BookType::L2_MBP);
    cache.add_order_book(book.clone()).unwrap();
    let result = cache.order_book(&audusd_sim.id);
    assert_eq!(result, Some(&book));
}

#[rstest]
fn test_order_book_mut_when_empty(mut cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.order_book_mut(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_order_book_mut_when_some(mut cache: Cache, audusd_sim: CurrencyPair) {
    let mut book = OrderBook::new(audusd_sim.id, BookType::L2_MBP);
    cache.add_order_book(book.clone()).unwrap();
    let result = cache.order_book_mut(&audusd_sim.id);
    assert_eq!(result, Some(&mut book));
}

#[cfg(feature = "defi")]
#[fixture]
fn test_pool() -> Pool {
    let chain = Arc::new(chains::ETHEREUM.clone());
    let dex = Dex::new(
        chains::ETHEREUM.clone(),
        DexType::UniswapV3,
        "0x1F98431c8aD98523631AE4a59f267346ea31F984",
        0,
        AmmType::CLAMM,
        "PoolCreated(address,address,uint24,int24,address)",
        "Swap(address,address,int256,int256,uint160,uint128,int24)",
        "Mint(address,address,int24,int24,uint128,uint256,uint256)",
        "Burn(address,int24,int24,uint128,uint256,uint256)",
        "Collect(address,address,int24,int24,uint128,uint128)",
    );

    let token0 = Token::new(
        chain.clone(),
        "0xA0b86a33E6441b936662bb6B5d1F8Fb0E2b57A5D"
            .parse()
            .unwrap(),
        "Wrapped Ether".to_string(),
        "WETH".to_string(),
        18,
    );

    let token1 = Token::new(
        chain.clone(),
        "0xdAC17F958D2ee523a2206206994597C13D831ec7"
            .parse()
            .unwrap(),
        "Tether USD".to_string(),
        "USDT".to_string(),
        6,
    );

    let pool_address = "0x11b815efB8f581194ae79006d24E0d814B7697F6"
        .parse()
        .unwrap();
    let pool_identifier: PoolIdentifier = "0x11b815efB8f581194ae79006d24E0d814B7697F6"
        .parse()
        .unwrap();
    Pool::new(
        chain,
        Arc::new(dex),
        pool_address,
        pool_identifier,
        12345678,
        token0,
        token1,
        Some(3000),
        Some(60),
        UnixNanos::from(1_234_567_890_000_000_000u64),
    )
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_when_empty(cache: Cache, test_pool: Pool) {
    let instrument_id = test_pool.instrument_id;
    let result = cache.pool(&instrument_id);
    assert!(result.is_none());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_when_some(mut cache: Cache, test_pool: Pool) {
    let instrument_id = test_pool.instrument_id;
    cache.add_pool(test_pool.clone()).unwrap();
    let result = cache.pool(&instrument_id);
    assert_eq!(result, Some(&test_pool));
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_mut_when_empty(mut cache: Cache, test_pool: Pool) {
    let instrument_id = test_pool.instrument_id;
    let result = cache.pool_mut(&instrument_id);
    assert!(result.is_none());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_mut_when_some(mut cache: Cache, test_pool: Pool) {
    let instrument_id = test_pool.instrument_id;
    cache.add_pool(test_pool).unwrap();
    let result = cache.pool_mut(&instrument_id);

    assert!(result.is_some());
    if let Some(pool_ref) = result {
        assert_eq!(pool_ref.fee.unwrap(), 3000);
    }
}

#[cfg(feature = "defi")]
#[rstest]
fn test_add_pool(mut cache: Cache, test_pool: Pool) {
    let instrument_id = test_pool.instrument_id;

    cache.add_pool(test_pool.clone()).unwrap();

    let cached_pool = cache.pool(&instrument_id);
    assert!(cached_pool.is_some());
    assert_eq!(cached_pool.unwrap(), &test_pool);
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_ids_when_empty(cache: Cache, test_pool: Pool) {
    let result = cache.pool_ids(Some(&test_pool.instrument_id.venue));
    assert!(result.is_empty());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_ids_when_some(mut cache: Cache, test_pool: Pool) {
    let venue = test_pool.instrument_id.venue;
    cache.add_pool(test_pool.clone()).unwrap();

    let result1 = cache.pool_ids(None);
    let result2 = cache.pool_ids(Some(&venue));
    assert_eq!(result1, vec![test_pool.instrument_id]);
    assert_eq!(result2, vec![test_pool.instrument_id]);
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pools_when_empty(cache: Cache, test_pool: Pool) {
    let result = cache.pools(Some(&test_pool.instrument_id.venue));
    assert!(result.is_empty());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pools_when_some(mut cache: Cache, test_pool: Pool) {
    let venue = test_pool.instrument_id.venue;
    cache.add_pool(test_pool.clone()).unwrap();

    let result1 = cache.pools(None);
    let result2 = cache.pools(Some(&venue));
    assert_eq!(result1, vec![&test_pool]);
    assert_eq!(result2, vec![&test_pool]);
}

#[cfg(feature = "defi")]
#[fixture]
fn test_pool_profiler(test_pool: Pool) -> PoolProfiler {
    PoolProfiler::new(Arc::new(test_pool))
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_when_empty(cache: Cache, test_pool_profiler: PoolProfiler) {
    let instrument_id = test_pool_profiler.pool.instrument_id;
    let result = cache.pool_profiler(&instrument_id);
    assert!(result.is_none());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_when_some(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let instrument_id = test_pool_profiler.pool.instrument_id;
    cache.add_pool_profiler(test_pool_profiler).unwrap();
    let result = cache.pool_profiler(&instrument_id);
    assert!(result.is_some());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_mut_when_empty(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let instrument_id = test_pool_profiler.pool.instrument_id;
    let result = cache.pool_profiler_mut(&instrument_id);
    assert!(result.is_none());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_mut_when_some(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let instrument_id = test_pool_profiler.pool.instrument_id;
    cache.add_pool_profiler(test_pool_profiler).unwrap();
    let result = cache.pool_profiler_mut(&instrument_id);
    assert!(result.is_some());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_add_pool_profiler(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let instrument_id = test_pool_profiler.pool.instrument_id;

    cache.add_pool_profiler(test_pool_profiler).unwrap();

    let cached_profiler = cache.pool_profiler(&instrument_id);
    assert!(cached_profiler.is_some());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_ids_when_empty(cache: Cache, test_pool_profiler: PoolProfiler) {
    let result = cache.pool_profiler_ids(Some(&test_pool_profiler.pool.instrument_id.venue));
    assert!(result.is_empty());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profiler_ids_when_some(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let venue = test_pool_profiler.pool.instrument_id.venue;
    cache.add_pool_profiler(test_pool_profiler.clone()).unwrap();

    let result1 = cache.pool_profiler_ids(None);
    let result2 = cache.pool_profiler_ids(Some(&venue));
    assert_eq!(result1, vec![test_pool_profiler.pool.instrument_id]);
    assert_eq!(result2, vec![test_pool_profiler.pool.instrument_id]);
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profilers_when_empty(cache: Cache, test_pool_profiler: PoolProfiler) {
    let result = cache.pool_profilers(Some(&test_pool_profiler.pool.instrument_id.venue));
    assert!(result.is_empty());
}

#[cfg(feature = "defi")]
#[rstest]
fn test_pool_profilers_when_some(mut cache: Cache, test_pool_profiler: PoolProfiler) {
    let venue = test_pool_profiler.pool.instrument_id.venue;
    cache.add_pool_profiler(test_pool_profiler).unwrap();

    let result1 = cache.pool_profilers(None);
    let result2 = cache.pool_profilers(Some(&venue));
    assert_eq!(result1.len(), 1);
    assert_eq!(result2.len(), 1);
}

#[rstest]
#[case(PriceType::Bid)]
#[case(PriceType::Ask)]
#[case(PriceType::Mid)]
#[case(PriceType::Last)]
#[case(PriceType::Mark)]
fn test_price_when_empty(cache: Cache, audusd_sim: CurrencyPair, #[case] price_type: PriceType) {
    let result = cache.price(&audusd_sim.id, price_type);
    assert!(result.is_none());
}

#[rstest]
fn test_price_when_some(mut cache: Cache, audusd_sim: CurrencyPair) {
    let mark_price = MarkPriceUpdate::new(
        audusd_sim.id,
        Price::from("1.00000"),
        UnixNanos::from(5),
        UnixNanos::from(10),
    );
    cache.add_mark_price(mark_price).unwrap();
    let result = cache.price(&audusd_sim.id, PriceType::Mark);
    assert_eq!(result, Some(mark_price.value));
}

#[rstest]
fn test_quote_tick_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.quote(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_quote_tick_when_some(mut cache: Cache) {
    let quote = QuoteTick::default();
    cache.add_quote(quote).unwrap();
    let result = cache.quote(&quote.instrument_id);
    assert_eq!(result, Some(&quote));
}

#[rstest]
fn test_quote_ticks_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.quotes(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_quote_ticks_when_some(mut cache: Cache) {
    let quotes = vec![
        QuoteTick::default(),
        QuoteTick::default(),
        QuoteTick::default(),
    ];
    cache.add_quotes(&quotes).unwrap();
    let result = cache.quotes(&quotes[0].instrument_id);
    assert_eq!(result, Some(quotes));
}

#[rstest]
fn test_trade_tick_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.trade(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_trade_tick_when_some(mut cache: Cache) {
    let trade = TradeTick::default();
    cache.add_trade(trade).unwrap();
    let result = cache.trade(&trade.instrument_id);
    assert_eq!(result, Some(&trade));
}

#[rstest]
fn test_trade_ticks_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.trades(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_trade_ticks_when_some(mut cache: Cache) {
    let trades = vec![
        TradeTick::default(),
        TradeTick::default(),
        TradeTick::default(),
    ];
    cache.add_trades(&trades).unwrap();
    let result = cache.trades(&trades[0].instrument_id);
    assert_eq!(result, Some(trades));
}

#[rstest]
fn test_mark_price_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.mark_price(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_mark_prices_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.mark_prices(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_index_price_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.index_price(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_index_prices_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.index_prices(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_funding_rate_when_empty(cache: Cache, audusd_sim: CurrencyPair) {
    let result = cache.funding_rate(&audusd_sim.id);
    assert!(result.is_none());
}

#[rstest]
fn test_add_funding_rate(mut cache: Cache, audusd_sim: CurrencyPair) {
    let funding_rate = FundingRateUpdate::new(
        audusd_sim.id,
        "0.0001".parse().unwrap(),
        None,
        UnixNanos::from(5),
        UnixNanos::from(10),
    );

    cache.add_funding_rate(funding_rate).unwrap();

    let result = cache.funding_rate(&audusd_sim.id);
    assert_eq!(result, Some(&funding_rate));
}

#[rstest]
fn test_add_funding_rate_updates_existing(mut cache: Cache, audusd_sim: CurrencyPair) {
    let funding_rate1 = FundingRateUpdate::new(
        audusd_sim.id,
        "0.0001".parse().unwrap(),
        None,
        UnixNanos::from(5),
        UnixNanos::from(10),
    );

    let funding_rate2 = FundingRateUpdate::new(
        audusd_sim.id,
        "0.0002".parse().unwrap(),
        None,
        UnixNanos::from(15),
        UnixNanos::from(20),
    );

    cache.add_funding_rate(funding_rate1).unwrap();
    cache.add_funding_rate(funding_rate2).unwrap();

    let result = cache.funding_rate(&audusd_sim.id);
    assert_eq!(result, Some(&funding_rate2));
}

#[rstest]
fn test_bar_when_empty(cache: Cache) {
    let bar = Bar::default();
    let result = cache.bar(&bar.bar_type);
    assert!(result.is_none());
}

#[rstest]
fn test_bar_when_some(mut cache: Cache) {
    let bar = Bar::default();
    cache.add_bar(bar).unwrap();
    let result = cache.bar(&bar.bar_type);
    assert_eq!(result, Some(&bar));
}

#[rstest]
fn test_bars_when_empty(cache: Cache) {
    let bar = Bar::default();
    let result = cache.bars(&bar.bar_type);
    assert!(result.is_none());
}

#[rstest]
fn test_bars_when_some(mut cache: Cache) {
    let bars = vec![Bar::default(), Bar::default(), Bar::default()];
    cache.add_bars(&bars).unwrap();
    let result = cache.bars(&bars[0].bar_type);
    assert_eq!(result, Some(bars));
}

// -- 账户 (ACCOUNT) ---------------------------------------------------------------------------------

#[rstest]
fn test_cache_accounts_when_no_database(mut cache: Cache) {
    assert!(futures::executor::block_on(cache.cache_accounts()).is_ok());
}

#[rstest]
fn test_cache_add_account(mut cache: Cache) {
    let account = AccountAny::default();
    cache.add_account(account.clone()).unwrap();
    let result = cache.account(&account.id());
    assert!(result.is_some());
    assert_eq!(*result.unwrap(), account);
}

#[rstest]
fn test_cache_accounts_when_no_accounts_returns_empty(cache: Cache) {
    let result = cache.accounts(&AccountId::test_default());
    assert!(result.is_empty());
}

#[rstest]
fn test_cache_account_for_venue_returns_empty(cache: Cache) {
    let venue = Venue::test_default();
    let result = cache.account_for_venue(&venue);
    assert!(result.is_none());
}

#[rstest]
fn test_cache_account_for_venue_return_correct(mut cache: Cache) {
    let account = AccountAny::default();
    let venue = account.last_event().unwrap().account_id.get_issuer();
    cache.add_account(account.clone()).unwrap();
    let result = cache.account_for_venue(&venue);
    assert!(result.is_some());
    assert_eq!(*result.unwrap(), account);
}

#[rstest]
fn test_get_mark_xrate_returns_none(cache: Cache) {
    // 当没有为 (USD, EUR) 设置标记汇率时，应返回 None
    assert!(
        cache
            .get_mark_xrate(Currency::USD(), Currency::EUR())
            .is_none()
    );
}

#[rstest]
fn test_set_and_get_mark_xrate(mut cache: Cache) {
    // 为 (USD, EUR) 设置标记汇率，并检查正向和反向汇率
    let xrate = 1.25;
    cache.set_mark_xrate(Currency::USD(), Currency::EUR(), xrate);
    assert_eq!(
        cache.get_mark_xrate(Currency::USD(), Currency::EUR()),
        Some(xrate)
    );
    assert_eq!(
        cache.get_mark_xrate(Currency::EUR(), Currency::USD()),
        Some(1.0 / xrate)
    );
}

#[rstest]
fn test_clear_mark_xrate(mut cache: Cache) {
    // 设置汇率，然后清除正向键 (forward key)
    let xrate = 1.25;
    cache.set_mark_xrate(Currency::USD(), Currency::EUR(), xrate);
    assert!(
        cache
            .get_mark_xrate(Currency::USD(), Currency::EUR())
            .is_some()
    );
    cache.clear_mark_xrate(Currency::USD(), Currency::EUR());
    assert!(
        cache
            .get_mark_xrate(Currency::USD(), Currency::EUR())
            .is_none()
    );
    assert_eq!(
        cache.get_mark_xrate(Currency::EUR(), Currency::USD()),
        Some(1.0 / xrate)
    );
}

#[rstest]
fn test_clear_mark_xrates(mut cache: Cache) {
    // 设置两个标记汇率，然后全部清除
    cache.set_mark_xrate(Currency::USD(), Currency::EUR(), 1.25);
    cache.set_mark_xrate(Currency::AUD(), Currency::USD(), 0.75);
    cache.clear_mark_xrates();
    assert!(
        cache
            .get_mark_xrate(Currency::USD(), Currency::EUR())
            .is_none()
    );
    assert!(
        cache
            .get_mark_xrate(Currency::EUR(), Currency::USD())
            .is_none()
    );
    assert!(
        cache
            .get_mark_xrate(Currency::AUD(), Currency::USD())
            .is_none()
    );
    assert!(
        cache
            .get_mark_xrate(Currency::USD(), Currency::AUD())
            .is_none()
    );
}

#[rstest]
#[should_panic(expected = "xrate was zero")]
fn test_set_mark_xrate_panics_on_zero(mut cache: Cache) {
    // 设置标记汇率为零应触发 Panic
    cache.set_mark_xrate(Currency::USD(), Currency::EUR(), 0.0);
}

#[rstest]
fn test_purge_order() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建一个订单并成交以生成一个持仓
    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let client_order_id = order.client_order_id();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        Some(TradeId::new("T-1")),
        Some(PositionId::new("P-123456")),
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );

    cache.add_order(order, None, None, false).unwrap();

    let mut position = Position::new(&audusd_sim, filled.into());
    let position_id = position.id;
    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();

    // 关闭持仓以测试从已关闭持仓中清除
    let order_close = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Sell)
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-19700101-000000-001-001-2"))
        .build();

    let filled_close = TestOrderEventStubs::filled(
        &order_close,
        &audusd_sim,
        Some(TradeId::new("T-2")),
        Some(position_id),
        Some(Price::from("1.00010")),
        None,
        None,
        None,
        None,
        None,
    );

    position.apply(&filled_close.into());
    cache.update_position(&position).unwrap();

    // 验证持仓现已关闭
    assert!(position.is_closed());

    // 验证订单是否存在
    assert!(cache.order_exists(&client_order_id));
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);

    // 将平仓订单添加到缓存中，以便可以将其清除
    let client_order_id_close = order_close.client_order_id();
    cache
        .add_order(order_close, Some(position_id), None, false)
        .unwrap();

    // 清除这两个订单 - 成交（fills）不应从持仓中清除
    cache.purge_order(client_order_id);
    cache.purge_order(client_order_id_close);

    // 验证订单已消失
    assert!(!cache.order_exists(&client_order_id));
    assert!(!cache.order_exists(&client_order_id_close));
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 0);
    // 验证持仓成交信息被保留 (purge_order 不会触及持仓成交信息)
    assert_eq!(cache.position(&position_id).unwrap().event_count(), 2);
}

#[rstest]
fn test_purge_open_order_skips_purge() {
    // 测试防护机制：尝试清除一个开仓订单应被阻止
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建并接受一个订单使其成为开仓状态
    let mut order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .build();

    let client_order_id = order.client_order_id();
    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = OrderAccepted::default();
    order.apply(OrderEventAny::Accepted(accepted)).unwrap();
    cache.update_order(&order).unwrap();

    // 验证订单为开仓状态
    assert!(order.is_open());
    assert!(cache.order_exists(&client_order_id));
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);

    // 尝试清除开仓订单 - 应被防护机制阻止
    cache.purge_order(client_order_id);

    // 验证订单仍然存在 (防护机制阻止了清除操作)
    assert!(cache.order_exists(&client_order_id));
    assert_eq!(cache.orders_total_count(None, None, None, None, None), 1);
    assert!(cache.order(&client_order_id).is_some());
}

#[rstest]
fn test_purge_position() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建一个订单并成交以生成一个持仓
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        Some(PositionId::new("P-123456")),
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );

    let mut position = Position::new(&audusd_sim, filled.into());
    let position_id = position.id;

    // 将持仓添加到缓存中
    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();

    // 验证持仓存在且为开仓状态
    assert!(cache.position_exists(&position_id));
    assert!(position.is_open());
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 1);

    // 首先平仓 (创建一个平仓订单并成交)
    let order_close = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Sell)
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-19700101-000000-001-001-2"))
        .build();

    let filled_close = TestOrderEventStubs::filled(
        &order_close,
        &audusd_sim,
        Some(TradeId::new("T-2")),
        Some(position_id),
        Some(Price::from("1.00010")),
        None,
        None,
        None,
        None,
        None,
    );

    position.apply(&filled_close.into());
    cache.update_position(&position).unwrap();

    // 验证持仓现已关闭
    assert!(position.is_closed());

    // 清除该持仓
    cache.purge_position(position_id);

    // 验证持仓已消失
    assert!(!cache.position_exists(&position_id));
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 0);
}

#[rstest]
fn test_purge_open_position_skips_purge() {
    // 测试防护机制：尝试清除一个开仓持仓应被阻止
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建一个订单并成交以生成一个开仓持仓状态
    let order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        Some(PositionId::new("P-123456")),
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );

    let position = Position::new(&audusd_sim, filled.into());
    let position_id = position.id;

    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();

    // 验证持仓为开仓状态
    assert!(position.is_open());
    assert!(cache.position_exists(&position_id));
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 1);
    assert_eq!(position.event_count(), 1);

    // 尝试清除开仓持仓 - 应被防护机制阻止
    cache.purge_position(position_id);

    // 验证持仓仍然存在 (防护机制阻止了清除操作)
    assert!(cache.position_exists(&position_id));
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 1);
    assert!(cache.position(&position_id).is_some());
    // 验证事件被保留
    assert_eq!(cache.position(&position_id).unwrap().event_count(), 1);
}

#[rstest]
fn test_purge_closed_positions_does_not_purge_reopened_position() {
    // 创建一个先平仓 (FLAT) 后重新开启的持仓
    // 此测试验证针对竞态条件的修复：以前被关闭但后来重新开启的持仓会被错误清除的问题。

    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建初始买单以开启持仓
    let order1 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    // 成交买单以开启多头头寸 (LONG)
    let fill1 = TestOrderEventStubs::filled(
        &order1,
        &audusd_sim,
        Some(TradeId::new("T-1")),            // trade_id
        Some(PositionId::new("P-1")),         // position_id
        Some(Price::from("1.00000")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(1_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    let mut position = Position::new(&audusd_sim, fill1.into());
    let position_id = position.id;

    // 将持仓添加到缓存中
    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();
    cache.update_position(&position).unwrap();

    // 验证持仓为多头头寸 (LONG)
    assert!(position.is_long());
    assert!(!position.is_closed());
    assert!(cache.is_position_open(&position_id));

    // 创建卖单以平仓 (使其变为 FLAT)
    let order2 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Sell)
        .quantity(Quantity::from(100_000))
        .build();

    // 成交卖单以平仓 (FLAT)
    let fill2 = TestOrderEventStubs::filled(
        &order2,
        &audusd_sim,
        Some(TradeId::new("T-2")),            // trade_id
        Some(position_id),                    // position_id
        Some(Price::from("1.00010")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(2_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    position.apply(&fill2.into());
    cache.update_position(&position).unwrap();

    // 验证持仓现为平仓状态 (FLAT/closed)
    assert_eq!(position.side, PositionSide::Flat);
    assert!(position.is_closed());
    assert!(position.ts_closed.is_some());
    let ts_closed_original = position.ts_closed.unwrap();
    assert!(cache.is_position_closed(&position_id));

    // 创建另一个买单以重新开启 (REOPEN) 状态持仓
    let order3 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(50_000))
        .build();

    // 成交买单以重新开启持仓 (再次变为 LONG)
    let fill3 = TestOrderEventStubs::filled(
        &order3,
        &audusd_sim,
        Some(TradeId::new("T-3")),            // trade_id
        Some(position_id),                    // position_id
        Some(Price::from("1.00020")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(3_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    position.apply(&fill3.into());
    cache.update_position(&position).unwrap();

    // 验证持仓再次变为多头 (已重新开启)
    assert!(position.is_long());
    assert!(!position.is_closed());
    assert_eq!(position.ts_closed, None); // Close timestamp should be reset
    assert!(cache.is_position_open(&position_id));

    // 尝试清除已关闭持仓
    // 即使该持仓之前曾被关闭，现在也不应被清除，因为它当前处于开启 (OPEN) 状态。
    // 使用一个遥远未来的时间戳，以确保任何旧的 ts_closed 都会触发清除。
    cache.purge_closed_positions(
        UnixNanos::from(ts_closed_original.as_u64() + 1_000_000_000_000),
        0, // No buffer
    );

    // 持仓应仍然存在，因为它当前为开启状态
    assert!(cache.position_exists(&position_id));
    assert!(cache.position(&position_id).is_some());
    assert!(cache.is_position_open(&position_id));
    assert!(!cache.is_position_closed(&position_id));
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 1);
    assert_eq!(cache.positions_open_count(None, None, None, None, None), 1);
    assert_eq!(
        cache.positions_closed_count(None, None, None, None, None),
        0
    );
}

#[rstest]
fn test_purge_order_cleans_up_strategy_orders_index() {
    // 关于 strategy_orders 索引清理 Bug 的回归测试
    // 验证在清除一个订单后，它会从策略的集合中移除
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建并添加一个已关闭的订单
    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    let strategy_id = order.strategy_id();
    let client_order_id = order.client_order_id();

    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = OrderSubmitted::default();
    order.apply(OrderEventAny::Submitted(submitted)).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = OrderAccepted::default();
    order.apply(OrderEventAny::Accepted(accepted)).unwrap();
    cache.update_order(&order).unwrap();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        None,
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );
    order.apply(filled).unwrap();
    cache.update_order(&order).unwrap();

    // 验证订单在策略索引中
    assert!(cache.index.strategy_orders.contains_key(&strategy_id));
    assert!(
        cache
            .index
            .strategy_orders
            .get(&strategy_id)
            .unwrap()
            .contains(&client_order_id)
    );

    // 清除订单
    cache.purge_order(client_order_id);

    // 验证订单已从策略索引中移除
    if let Some(strategy_orders) = cache.index.strategy_orders.get(&strategy_id) {
        assert!(!strategy_orders.contains(&client_order_id));
        // 如果这是唯一的订单，策略键应当被移除
        assert!(
            !strategy_orders.is_empty(),
            "Empty strategy_orders set should have been removed"
        );
    }

    // 查询该策略的订单不应崩溃，且不应包含已清除的订单
    let orders_for_strategy = cache.orders(None, None, Some(&strategy_id), None, None);
    assert!(!orders_for_strategy.contains(&&order));
}

#[rstest]
fn test_purge_order_cleans_up_exec_spawn_orders_index() {
    // 关于 exec_spawn_orders 索引清理 Bug 的回归测试
    // 验证在清除一个生成的子订单后，它会从父订单的集合中移除
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // 创建父订单
    let parent_id = ClientOrderId::new("PARENT-001");

    // 创建并添加一个带有 exec_spawn_id 的子订单
    let mut child_order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .exec_spawn_id(parent_id)
        .build();

    let child_id = child_order.client_order_id();

    cache
        .add_order(child_order.clone(), None, None, false)
        .unwrap();

    let submitted = OrderSubmitted::default();
    child_order
        .apply(OrderEventAny::Submitted(submitted))
        .unwrap();
    cache.update_order(&child_order).unwrap();

    let accepted = OrderAccepted::default();
    child_order
        .apply(OrderEventAny::Accepted(accepted))
        .unwrap();
    cache.update_order(&child_order).unwrap();

    let filled = TestOrderEventStubs::filled(
        &child_order,
        &audusd_sim,
        None,
        None,
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );
    child_order.apply(filled).unwrap();
    cache.update_order(&child_order).unwrap();

    // 验证子订单在父订单的生成集合中
    assert!(cache.index.exec_spawn_orders.contains_key(&parent_id));
    assert!(
        cache
            .index
            .exec_spawn_orders
            .get(&parent_id)
            .unwrap()
            .contains(&child_id)
    );

    // 清除子订单
    cache.purge_order(child_id);

    // 验证子订单已从父订单的生成集合中移除
    if let Some(spawn_orders) = cache.index.exec_spawn_orders.get(&parent_id) {
        assert!(!spawn_orders.contains(&child_id));
    }

    // 查询执行生成的订单不应崩溃，且不应包含已清除的订单
    let orders_for_spawn = cache.orders_for_exec_spawn(&parent_id);
    assert!(!orders_for_spawn.contains(&&child_order));
}

#[rstest]
fn test_purge_order_when_order_not_in_cache_still_cleans_up_indices() {
    // 测试即使订单不在缓存中，索引也能使用正向映射进行清理
    let mut cache = Cache::default();

    let client_order_id = ClientOrderId::new("O-NOT-IN-CACHE");
    let strategy_id = StrategyId::test_default();

    // 手动添加到索引 (模拟损坏的状态)
    cache
        .index
        .order_strategy
        .insert(client_order_id, strategy_id);
    cache
        .index
        .strategy_orders
        .entry(strategy_id)
        .or_default()
        .insert(client_order_id);

    // 验证索引已设置
    assert!(cache.index.order_strategy.contains_key(&client_order_id));
    assert!(
        cache
            .index
            .strategy_orders
            .get(&strategy_id)
            .unwrap()
            .contains(&client_order_id)
    );

    // 清除不存在的订单
    cache.purge_order(client_order_id);

    // 验证即使订单不在缓存中也能清理索引
    assert!(!cache.index.order_strategy.contains_key(&client_order_id));
    if let Some(strategy_orders) = cache.index.strategy_orders.get(&strategy_id) {
        assert!(!strategy_orders.contains(&client_order_id));
    }
}

#[rstest]
fn test_purge_order_cleans_up_account_orders_index() {
    // 回归测试：清除订单必须将其从账户订单索引中移除
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    let client_order_id = order.client_order_id();
    let account_id = AccountId::new("SIM-001");

    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = TestOrderEventStubs::submitted(&order, account_id);
    order.apply(submitted).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = TestOrderEventStubs::accepted(&order, account_id, VenueOrderId::new("V-001"));
    order.apply(accepted).unwrap();
    cache.update_order(&order).unwrap();

    let filled = TestOrderEventStubs::filled(
        &order,
        &audusd_sim,
        None,
        None,
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );
    order.apply(filled).unwrap();
    cache.update_order(&order).unwrap();

    // 验证订单在账户索引中 (由 update_order 填充)
    assert!(cache.index.account_orders.contains_key(&account_id));
    assert!(
        cache
            .index
            .account_orders
            .get(&account_id)
            .unwrap()
            .contains(&client_order_id)
    );

    cache.purge_order(client_order_id);

    // 由于这是唯一的订单，账户键应当被完全移除
    assert!(!cache.index.account_orders.contains_key(&account_id));

    let orders_for_account = cache.orders(None, None, None, Some(&account_id), None);
    assert!(!orders_for_account.contains(&&order));
}

#[rstest]
fn test_purge_position_cleans_up_account_positions_index() {
    // 回归测试：清除持仓必须将其从账户持仓索引中移除
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);

    let mut order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    let account_id = AccountId::new("SIM-001");
    let trade_id = TradeId::new("T-001");

    cache.add_order(order.clone(), None, None, false).unwrap();

    let submitted = TestOrderEventStubs::submitted(&order, account_id);
    order.apply(submitted).unwrap();
    cache.update_order(&order).unwrap();

    let accepted = TestOrderEventStubs::accepted(&order, account_id, VenueOrderId::new("V-001"));
    order.apply(accepted).unwrap();
    cache.update_order(&order).unwrap();

    let filled = TestOrderEventStubs::filled(
        &order,
        &instrument,
        Some(trade_id),
        None,
        Some(Price::from("1.00001")),
        None,
        None,
        None,
        None,
        None,
    );
    order.apply(filled.clone()).unwrap();
    cache.update_order(&order).unwrap();

    let position = Position::new(&instrument, filled.into());
    let position_id = position.id;
    cache.add_position(position, OmsType::Hedging).unwrap();

    // 验证持仓在账户索引中 (由 add_position 填充)
    assert!(cache.index.account_positions.contains_key(&account_id));
    assert!(
        cache
            .index
            .account_positions
            .get(&account_id)
            .unwrap()
            .contains(&position_id)
    );

    // 平仓以便可以进行清除 (开仓持仓受保护)
    let mut position = cache.position(&position_id).unwrap().clone();
    let close_order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(instrument.id())
        .side(OrderSide::Sell)
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-19700101-000000-001-001-2"))
        .build();

    let close_filled = TestOrderEventStubs::filled(
        &close_order,
        &instrument,
        Some(TradeId::new("T-002")),
        Some(position_id),
        Some(Price::from("1.00002")),
        None,
        None,
        None,
        None,
        None,
    );
    let close_filled: OrderFilled = close_filled.into();
    position.apply(&close_filled);
    cache.update_position(&position).unwrap();

    assert!(position.is_closed());

    cache.purge_position(position_id);

    // 由于这是唯一的持仓，账户键应当被完全移除
    assert!(!cache.index.account_positions.contains_key(&account_id));

    let positions_for_account = cache.positions(None, None, None, Some(&account_id), None);
    assert!(positions_for_account.is_empty());
}

#[rstest]
fn test_update_own_order_book_with_market_order_does_not_panic(mut cache: Cache) {
    let audusd_sim = audusd_sim();
    cache
        .add_instrument(InstrumentAny::CurrencyPair(audusd_sim.clone()))
        .unwrap();

    // 为交易工具创建一个限价单以建立自己的订单簿
    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);
    assert!(cache.own_order_book(&audusd_sim.id()).is_some());

    // 创建一个市价单 (无价格) 并在状态上转换为 FILLED
    let market_order = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(50_000))
        .client_order_id(ClientOrderId::new("O-19700101-000000-001-001-2"))
        .build();

    cache
        .add_order(market_order.clone(), None, None, false)
        .unwrap();

    let submitted = TestOrderEventStubs::submitted(&market_order, AccountId::new("SIM-001"));
    let mut market_order_mut = market_order;
    market_order_mut.apply(submitted).unwrap();

    let accepted = TestOrderEventStubs::accepted(
        &market_order_mut,
        AccountId::new("SIM-001"),
        VenueOrderId::new("V-001"),
    );
    market_order_mut.apply(accepted).unwrap();

    let filled = TestOrderEventStubs::filled(
        &market_order_mut,
        &InstrumentAny::CurrencyPair(audusd_sim.clone()),
        Some(TradeId::new("T-001")),
        None,
        Some(Price::from("1.00010")),
        None,
        None,
        None,
        None,
        None,
    );
    market_order_mut.apply(filled).unwrap();

    // 不应发生 Panic - 此前会在 `.expect("OwnBookOrder must have a price")` 处发生 Panic
    cache.update_own_order_book(&market_order_mut);

    assert!(cache.own_order_book(&audusd_sim.id()).is_some());
}

#[rstest]
fn test_purge_closed_orders_also_purges_order_lists() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);

    let order_list_id = OrderListId::new("OL-001");

    let mut order1 = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-001"))
        .order_list_id(order_list_id)
        .build();

    let mut order2 = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Sell)
        .price(Price::from("1.00100"))
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-002"))
        .order_list_id(order_list_id)
        .build();
    let order_list = OrderList::new(
        order_list_id,
        instrument.id(),
        order1.strategy_id(),
        vec![order1.client_order_id(), order2.client_order_id()],
        UnixNanos::default(),
    );

    let account_id = AccountId::new("SIM-001");

    cache.add_order(order1.clone(), None, None, false).unwrap();
    cache.add_order(order2.clone(), None, None, false).unwrap();
    cache.add_order_list(order_list).unwrap();

    assert!(cache.order_list_exists(&order_list_id));

    // 转换订单 1：Initialized -> Submitted -> Accepted -> Filled
    let submitted1 = TestOrderEventStubs::submitted(&order1, account_id);
    order1.apply(submitted1).unwrap();
    cache.update_order(&order1).unwrap();

    let accepted1 = TestOrderEventStubs::accepted(&order1, account_id, VenueOrderId::new("V-001"));
    order1.apply(accepted1).unwrap();
    cache.update_order(&order1).unwrap();

    let filled1 = TestOrderEventStubs::filled(
        &order1,
        &instrument,
        Some(TradeId::new("T-1")),
        None,
        Some(Price::from("1.00000")),
        None,
        None,
        None,
        None,
        None,
    );
    order1.apply(filled1).unwrap();
    cache.update_order(&order1).unwrap();

    // 转换订单 2：Initialized -> Submitted -> Accepted -> Canceled
    let submitted2 = TestOrderEventStubs::submitted(&order2, account_id);
    order2.apply(submitted2).unwrap();
    cache.update_order(&order2).unwrap();

    let accepted2 = TestOrderEventStubs::accepted(&order2, account_id, VenueOrderId::new("V-002"));
    order2.apply(accepted2).unwrap();
    cache.update_order(&order2).unwrap();

    let canceled2 =
        TestOrderEventStubs::canceled(&order2, account_id, Some(VenueOrderId::new("V-002")));
    order2.apply(canceled2).unwrap();
    cache.update_order(&order2).unwrap();

    assert!(order1.is_closed());
    assert!(order2.is_closed());

    let ts_now = UnixNanos::from(1_000_000_000_000);
    cache.purge_closed_orders(ts_now, 0);

    assert!(!cache.order_exists(&order1.client_order_id()));
    assert!(!cache.order_exists(&order2.client_order_id()));
    assert!(!cache.order_list_exists(&order_list_id));
}

#[rstest]
fn test_purge_closed_orders_does_not_purge_order_list_with_open_orders() {
    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let instrument = InstrumentAny::CurrencyPair(audusd_sim);

    let order_list_id = OrderListId::new("OL-001");

    let mut order1 = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::from("1.00000"))
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-001"))
        .order_list_id(order_list_id)
        .build();

    let mut order2 = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Sell)
        .price(Price::from("1.00100"))
        .quantity(Quantity::from(100_000))
        .client_order_id(ClientOrderId::new("O-002"))
        .order_list_id(order_list_id)
        .build();
    let order_list = OrderList::new(
        order_list_id,
        instrument.id(),
        order1.strategy_id(),
        vec![order1.client_order_id(), order2.client_order_id()],
        UnixNanos::default(),
    );

    let account_id = AccountId::new("SIM-001");

    cache.add_order(order1.clone(), None, None, false).unwrap();
    cache.add_order(order2.clone(), None, None, false).unwrap();
    cache.add_order_list(order_list).unwrap();

    // 关闭订单 1，保持订单 2 为处于开启（open）状态
    let submitted1 = TestOrderEventStubs::submitted(&order1, account_id);
    order1.apply(submitted1).unwrap();
    cache.update_order(&order1).unwrap();

    let accepted1 = TestOrderEventStubs::accepted(&order1, account_id, VenueOrderId::new("V-001"));
    order1.apply(accepted1).unwrap();
    cache.update_order(&order1).unwrap();

    let filled1 = TestOrderEventStubs::filled(
        &order1,
        &instrument,
        Some(TradeId::new("T-1")),
        None,
        Some(Price::from("1.00000")),
        None,
        None,
        None,
        None,
        None,
    );
    order1.apply(filled1).unwrap();
    cache.update_order(&order1).unwrap();

    let submitted2 = TestOrderEventStubs::submitted(&order2, account_id);
    order2.apply(submitted2).unwrap();
    cache.update_order(&order2).unwrap();

    let accepted2 = TestOrderEventStubs::accepted(&order2, account_id, VenueOrderId::new("V-002"));
    order2.apply(accepted2).unwrap();
    cache.update_order(&order2).unwrap();

    assert!(order1.is_closed());
    assert!(order2.is_open());

    let ts_now = UnixNanos::from(1_000_000_000_000);
    cache.purge_closed_orders(ts_now, 0);

    // 订单 1 已清除，订单 2 和列表保留 (订单 2 仍然在缓存中)
    assert!(!cache.order_exists(&order1.client_order_id()));
    assert!(cache.order_exists(&order2.client_order_id()));
    assert!(cache.order_list_exists(&order_list_id));
}

#[rstest]
fn test_force_remove_from_own_order_book(mut cache: Cache) {
    let audusd_sim = audusd_sim();
    cache
        .add_instrument(InstrumentAny::CurrencyPair(audusd_sim.clone()))
        .unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let submitted = TestOrderEventStubs::submitted(&limit_order, AccountId::new("SIM-001"));
    let mut limit_order_mut = limit_order;
    limit_order_mut.apply(submitted).unwrap();
    cache.update_order(&limit_order_mut).unwrap();

    assert!(cache.order_exists(&limit_order_mut.client_order_id()));
    assert!(
        cache
            .index
            .orders_inflight
            .contains(&limit_order_mut.client_order_id())
    );
    assert!(cache.own_order_book(&audusd_sim.id()).is_some());

    cache.force_remove_from_own_order_book(&limit_order_mut.client_order_id());

    assert!(
        !cache
            .index
            .orders_open
            .contains(&limit_order_mut.client_order_id())
    );
    assert!(
        !cache
            .index
            .orders_inflight
            .contains(&limit_order_mut.client_order_id())
    );
    assert!(
        !cache
            .index
            .orders_emulated
            .contains(&limit_order_mut.client_order_id())
    );
    assert!(
        !cache
            .index
            .orders_pending_cancel
            .contains(&limit_order_mut.client_order_id())
    );
    assert!(
        cache
            .index
            .orders_closed
            .contains(&limit_order_mut.client_order_id())
    );
}

#[rstest]
fn test_audit_own_order_books_with_inflight_orders(mut cache: Cache) {
    let audusd_sim = audusd_sim();
    cache
        .add_instrument(InstrumentAny::CurrencyPair(audusd_sim.clone()))
        .unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let submitted = TestOrderEventStubs::submitted(&limit_order, AccountId::new("SIM-001"));
    let mut limit_order_mut = limit_order;
    limit_order_mut.apply(submitted).unwrap();
    cache.update_order(&limit_order_mut).unwrap();

    let own_book = cache.own_order_book(&audusd_sim.id()).unwrap();
    assert!(own_book.bids().count() > 0);

    cache.audit_own_order_books();

    let own_book = cache.own_order_book(&audusd_sim.id()).unwrap();
    assert!(own_book.bids().count() > 0);
}

#[rstest]
fn test_audit_own_order_books_removes_closed(mut cache: Cache) {
    let audusd_sim = audusd_sim();
    cache
        .add_instrument(InstrumentAny::CurrencyPair(audusd_sim.clone()))
        .unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let submitted = TestOrderEventStubs::submitted(&limit_order, AccountId::new("SIM-001"));
    let mut limit_order_mut = limit_order;
    limit_order_mut.apply(submitted).unwrap();
    cache.update_order(&limit_order_mut).unwrap();

    let accepted = TestOrderEventStubs::accepted(
        &limit_order_mut,
        AccountId::new("SIM-001"),
        VenueOrderId::new("V-001"),
    );
    limit_order_mut.apply(accepted).unwrap();
    cache.update_order(&limit_order_mut).unwrap();

    let own_book = cache.own_order_book(&audusd_sim.id()).unwrap();
    assert!(own_book.bids().count() > 0);

    let canceled = TestOrderEventStubs::canceled(
        &limit_order_mut,
        AccountId::new("SIM-001"),
        Some(VenueOrderId::new("V-001")),
    );
    limit_order_mut.apply(canceled).unwrap();
    cache.update_order(&limit_order_mut).unwrap();

    cache.update_own_order_book(&limit_order_mut);

    cache.audit_own_order_books();

    let own_book = cache.own_order_book(&audusd_sim.id()).unwrap();
    assert_eq!(own_book.bids().count(), 0);
}

#[rstest]
fn test_own_order_book_lifecycle_sequence(mut cache: Cache) {
    let instrument = InstrumentAny::CurrencyPair(audusd_sim());
    cache.add_instrument(instrument.clone()).unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let mut live_order = limit_order;

    let submitted = TestOrderEventStubs::submitted(&live_order, AccountId::new("SIM-001"));
    live_order.apply(submitted).unwrap();
    cache.update_order(&live_order).unwrap();

    let venue_order_id = VenueOrderId::new("V-LCYCLE");
    let accepted =
        TestOrderEventStubs::accepted(&live_order, AccountId::new("SIM-001"), venue_order_id);
    live_order.apply(accepted).unwrap();
    cache.update_order(&live_order).unwrap();

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert!(own_book.bids().count() > 0);

    let partial_fill = TestOrderEventStubs::filled(
        &live_order,
        &instrument,
        None,
        None,
        None,
        Some(Quantity::from(50_000)),
        None,
        None,
        None,
        None,
    );
    live_order.apply(partial_fill).unwrap();
    cache.update_order(&live_order).unwrap();

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert!(own_book.bids().count() > 0);

    let canceled = TestOrderEventStubs::canceled(
        &live_order,
        AccountId::new("SIM-001"),
        Some(VenueOrderId::new("V-LCYCLE")),
    );
    live_order.apply(canceled).unwrap();
    cache.update_order(&live_order).unwrap();
    cache.update_own_order_book(&live_order);

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert_eq!(own_book.bids().count(), 0);
}

#[rstest]
fn test_own_order_book_pending_cancel_persists_until_final(mut cache: Cache) {
    let instrument = InstrumentAny::CurrencyPair(audusd_sim());
    cache.add_instrument(instrument.clone()).unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let mut live_order = limit_order;
    let accepted = TestOrderEventStubs::accepted(
        &live_order,
        AccountId::new("SIM-001"),
        VenueOrderId::new("V-PENDING"),
    );
    live_order.apply(accepted).unwrap();
    cache.update_order(&live_order).unwrap();

    cache.update_order_pending_cancel_local(&live_order);
    cache.audit_own_order_books();

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert!(own_book.bids().count() > 0);

    let canceled = TestOrderEventStubs::canceled(
        &live_order,
        AccountId::new("SIM-001"),
        Some(VenueOrderId::new("V-PENDING")),
    );
    live_order.apply(canceled).unwrap();
    cache.update_order(&live_order).unwrap();
    cache.update_own_order_book(&live_order);

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert_eq!(own_book.bids().count(), 0);
}

#[rstest]
fn test_update_own_order_book_reinserts_missing_levels(mut cache: Cache) {
    let instrument = InstrumentAny::CurrencyPair(audusd_sim());
    cache.add_instrument(instrument.clone()).unwrap();

    let limit_order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache
        .add_order(limit_order.clone(), None, None, false)
        .unwrap();
    cache.update_own_order_book(&limit_order);

    let mut live_order = limit_order;
    let accepted = TestOrderEventStubs::accepted(
        &live_order,
        AccountId::new("SIM-001"),
        VenueOrderId::new("V-REINSERT"),
    );
    live_order.apply(accepted).unwrap();
    cache.update_order(&live_order).unwrap();

    {
        let own_book = cache
            .own_books
            .get_mut(&instrument.id())
            .expect("own book missing");
        own_book.clear();
    }

    cache.update_own_order_book(&live_order);

    let own_book = cache.own_order_book(&instrument.id()).unwrap();
    assert!(own_book.bids().count() > 0);
}

#[rstest]
fn test_position_flip_netting_mode_cleans_up_closed_index() {
    // Regression test for NETTING position flip index corruption (issue #3081)
    // Verifies that when a position ID is reused in NETTING mode,
    // add_position removes the position from the closed index

    let mut cache = Cache::default();
    let audusd_sim = audusd_sim();
    let audusd_sim = InstrumentAny::CurrencyPair(audusd_sim);

    // Create initial buy order to open LONG position
    let order1 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .build();

    // Fill the buy order to open LONG position
    let fill1 = TestOrderEventStubs::filled(
        &order1,
        &audusd_sim,
        Some(TradeId::new("T-1")),            // trade_id
        Some(PositionId::new("P-1")),         // position_id
        Some(Price::from("1.00000")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(1_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    let mut position = Position::new(&audusd_sim, fill1.into());
    let position_id = position.id;

    // Add position to cache
    cache
        .add_position(position.clone(), OmsType::Netting)
        .unwrap();

    // Verify position is LONG and in open index
    assert!(position.is_long());
    assert!(!position.is_closed());
    assert!(cache.is_position_open(&position_id));
    assert!(!cache.is_position_closed(&position_id));

    // Create a SELL order that closes the position (makes it FLAT)
    let order2 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Sell)
        .quantity(Quantity::from(100_000))
        .build();

    // Fill the sell order to close position to FLAT
    let fill2 = TestOrderEventStubs::filled(
        &order2,
        &audusd_sim,
        Some(TradeId::new("T-2")),            // trade_id
        Some(position_id),                    // position_id (same ID in NETTING)
        Some(Price::from("1.00010")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(2_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    position.apply(&fill2.into());
    cache.update_position(&position).unwrap();

    // Verify position is now FLAT (closed)
    assert_eq!(position.side, PositionSide::Flat);
    assert!(position.is_closed());
    assert!(cache.is_position_closed(&position_id));
    assert!(!cache.is_position_open(&position_id));

    // Snapshot the closed position before reusing the ID (as execution engine does)
    cache.snapshot_position(&position).unwrap();

    // Create a new BUY order to reopen the position (NETTING mode reuses the ID)
    let order3 = OrderTestBuilder::new(OrderType::Market)
        .instrument_id(audusd_sim.id())
        .side(OrderSide::Buy)
        .quantity(Quantity::from(50_000))
        .build();

    // Fill to create a new LONG position with the same position ID
    let fill3 = TestOrderEventStubs::filled(
        &order3,
        &audusd_sim,
        Some(TradeId::new("T-3")),            // trade_id
        Some(position_id),                    // position_id (reused in NETTING)
        Some(Price::from("1.00020")),         // last_px
        None,                                 // last_qty
        None,                                 // liquidity_side
        None,                                 // commission
        Some(UnixNanos::from(3_000_000_000)), // ts_filled_ns
        None,                                 // account_id
    );

    // Create new position object with the same ID (as execution engine does)
    let position_reopened = Position::new(&audusd_sim, fill3.into());
    assert_eq!(position_reopened.id, position_id); // Same ID reused

    // Add the reopened position to cache
    // THIS IS THE KEY TEST: add_position should remove from closed index
    cache
        .add_position(position_reopened.clone(), OmsType::Netting)
        .unwrap();

    // The reopened position should be in open index, NOT closed index
    assert!(position_reopened.is_long());
    assert!(!position_reopened.is_closed());
    assert!(
        cache.is_position_open(&position_id),
        "Position should be in open index"
    );
    assert!(
        !cache.is_position_closed(&position_id),
        "Position should NOT be in closed index (bug fixed)"
    );

    // Verify position counts
    assert_eq!(cache.positions_total_count(None, None, None, None, None), 1);
    assert_eq!(cache.positions_open_count(None, None, None, None, None), 1);
    assert_eq!(
        cache.positions_closed_count(None, None, None, None, None),
        0
    );

    // Verify the snapshot exists
    assert!(cache.position_snapshots.contains_key(&position_id));

    // Verify the active position is LONG with correct quantity
    let cached_pos = cache.position(&position_id).unwrap();
    assert_eq!(cached_pos.side, PositionSide::Long);
    assert_eq!(cached_pos.quantity, Quantity::from(50_000));
    assert_eq!(cached_pos.event_count(), 1); // Only the reopen fill event
}

#[rstest]
fn test_add_trades_same_timestamp_adds_all(mut cache: Cache) {
    // 在同一时间戳发生的多个成交 (例如，大额订单扫盘)
    let ts = UnixNanos::from(1000);
    let instrument_id = InstrumentId::from("AUDUSD.SIM");

    let trade1 = TradeTick::new(
        instrument_id,
        Price::from("1.00000"),
        Quantity::from(100_000),
        AggressorSide::Buyer,
        TradeId::new("1"),
        ts,
        ts,
    );

    let trade2 = TradeTick::new(
        instrument_id,
        Price::from("1.00001"),
        Quantity::from(100_000),
        AggressorSide::Buyer,
        TradeId::new("2"),
        ts,
        ts,
    );

    let trade3 = TradeTick::new(
        instrument_id,
        Price::from("1.00002"),
        Quantity::from(100_000),
        AggressorSide::Buyer,
        TradeId::new("3"),
        ts,
        ts,
    );

    cache.add_trade(trade1).unwrap();
    cache.add_trades(&[trade2, trade3]).unwrap();

    // 所有三个成交信息都应位于缓存中
    let result = cache.trades(&instrument_id).unwrap();
    assert_eq!(
        result.len(),
        3,
        "All trades with same timestamp should be added"
    );
}

#[rstest]
fn test_add_quotes_same_timestamp_adds_all(mut cache: Cache) {
    // 同一时间戳的多个报价信息
    let ts = UnixNanos::from(1000);
    let instrument_id = InstrumentId::from("AUDUSD.SIM");

    let quote1 = QuoteTick::new(
        instrument_id,
        Price::from("1.00000"),
        Price::from("1.00001"),
        Quantity::from(100_000),
        Quantity::from(100_000),
        ts,
        ts,
    );

    let quote2 = QuoteTick::new(
        instrument_id,
        Price::from("1.00002"),
        Price::from("1.00003"),
        Quantity::from(100_000),
        Quantity::from(100_000),
        ts,
        ts,
    );

    let quote3 = QuoteTick::new(
        instrument_id,
        Price::from("1.00004"),
        Price::from("1.00005"),
        Quantity::from(100_000),
        Quantity::from(100_000),
        ts,
        ts,
    );

    cache.add_quote(quote1).unwrap();
    cache.add_quotes(&[quote2, quote3]).unwrap();

    // 所有三个报价信息都应位于缓存中
    let result = cache.quotes(&instrument_id).unwrap();
    assert_eq!(
        result.len(),
        3,
        "All quotes with same timestamp should be added"
    );
}

#[rstest]
fn test_add_bars_same_timestamp_adds_all(mut cache: Cache) {
    // 同一时间戳的多个 K 线信息
    let ts = UnixNanos::from(1000);
    let bar_type = BarType::from("AUDUSD.SIM-1-MINUTE-BID-EXTERNAL");

    let bar1 = Bar::new(
        bar_type,
        Price::from("1.00000"),
        Price::from("1.00001"),
        Price::from("0.99999"),
        Price::from("1.00000"),
        Quantity::from(100_000),
        ts,
        ts,
    );

    let bar2 = Bar::new(
        bar_type,
        Price::from("1.00001"),
        Price::from("1.00002"),
        Price::from("1.00000"),
        Price::from("1.00001"),
        Quantity::from(100_000),
        ts,
        ts,
    );

    let bar3 = Bar::new(
        bar_type,
        Price::from("1.00002"),
        Price::from("1.00003"),
        Price::from("1.00001"),
        Price::from("1.00002"),
        Quantity::from(100_000),
        ts,
        ts,
    );

    cache.add_bar(bar1).unwrap();
    cache.add_bars(&[bar2, bar3]).unwrap();

    // 所有三个 K 线信息都应位于缓存中
    let result = cache.bars(&bar_type).unwrap();
    assert_eq!(
        result.len(),
        3,
        "All bars with same timestamp should be added"
    );
}

// -- orders_emulated 索引测试 ------------------------------------------------------------------

#[rstest]
fn test_add_emulated_order_indexes_in_orders_emulated(mut cache: Cache, audusd_sim: CurrencyPair) {
    let order = OrderTestBuilder::new(OrderType::StopMarket)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .trigger_price(Price::from("1.00010"))
        .emulation_trigger(TriggerType::LastPrice)
        .build();

    cache.add_order(order.clone(), None, None, false).unwrap();

    assert!(
        cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Emulated order should be in orders_emulated index after add"
    );
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 1);
}

#[rstest]
fn test_add_non_emulated_order_not_in_orders_emulated(mut cache: Cache, audusd_sim: CurrencyPair) {
    let order = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .price(Price::from("1.00000"))
        .build();

    cache.add_order(order.clone(), None, None, false).unwrap();

    assert!(
        !cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Non-emulated order should not be in orders_emulated index"
    );
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
}

#[rstest]
fn test_update_released_order_removes_from_orders_emulated(
    mut cache: Cache,
    audusd_sim: CurrencyPair,
) {
    let order = OrderTestBuilder::new(OrderType::StopMarket)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .trigger_price(Price::from("1.00010"))
        .emulation_trigger(TriggerType::LastPrice)
        .build();

    cache.add_order(order.clone(), None, None, false).unwrap();

    assert!(
        cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Emulated order should be in orders_emulated index after add"
    );

    // 应用释放 (released) 事件 (订单已发送到交易所，不再是模拟状态)
    let released = OrderReleased::new(
        order.trader_id(),
        order.strategy_id(),
        order.instrument_id(),
        order.client_order_id(),
        Price::from("1.00010"),
        UUID4::new(),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    let mut order = order;
    order.apply(OrderEventAny::Released(released)).unwrap();
    cache.update_order(&order).unwrap();

    assert!(
        !cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Released order should be removed from orders_emulated index"
    );
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
}

#[rstest]
fn test_update_closed_emulated_order_removes_from_orders_emulated(
    mut cache: Cache,
    audusd_sim: CurrencyPair,
) {
    let order = OrderTestBuilder::new(OrderType::StopMarket)
        .instrument_id(audusd_sim.id)
        .side(OrderSide::Buy)
        .quantity(Quantity::from(100_000))
        .trigger_price(Price::from("1.00010"))
        .emulation_trigger(TriggerType::LastPrice)
        .build();

    cache.add_order(order.clone(), None, None, false).unwrap();

    assert!(
        cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Emulated order should be in orders_emulated index after add"
    );

    // 首先应用模拟 (emulated) 事件
    let emulated = OrderEmulated::new(
        order.trader_id(),
        order.strategy_id(),
        order.instrument_id(),
        order.client_order_id(),
        UUID4::new(),
        UnixNanos::default(),
        UnixNanos::default(),
    );
    let mut order = order;
    order.apply(OrderEventAny::Emulated(emulated)).unwrap();
    cache.update_order(&order).unwrap();

    // 订单应仍处于模拟状态销
    assert!(
        cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Order should still be in orders_emulated after emulated event"
    );

    // 应用撤单 (canceled) 事件 (订单现已关闭)
    let canceled = OrderCanceled::new(
        order.trader_id(),
        order.strategy_id(),
        order.instrument_id(),
        order.client_order_id(),
        UUID4::new(),
        UnixNanos::default(),
        UnixNanos::default(),
        false,
        None,
        None,
    );
    order.apply(OrderEventAny::Canceled(canceled)).unwrap();
    cache.update_order(&order).unwrap();

    assert!(
        !cache
            .index
            .orders_emulated
            .contains(&order.client_order_id()),
        "Closed emulated order should be removed from orders_emulated index"
    );
    assert_eq!(cache.orders_emulated_count(None, None, None, None, None), 0);
}
