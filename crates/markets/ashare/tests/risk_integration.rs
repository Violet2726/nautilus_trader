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

use std::{cell::RefCell, rc::Rc, sync::Arc};

use nautilus_common::{
    cache::Cache,
    clock::TestClock,
    messages::execution::{CancelOrder, SubmitOrder, TradingCommand},
    msgbus::{
        self, MessagingSwitchboard,
        stubs::{TypedIntoMessageSavingHandler, get_typed_into_message_saving_handler},
    },
};
use nautilus_core::UUID4;
use nautilus_model::{
    accounts::{AccountAny, stubs::cash_account},
    events::{OrderEventAny, OrderEventType, account::stubs::cash_account_state_million_usd},
    identifiers::{AccountId, ClientId, ClientOrderId, InstrumentId, StrategyId, TraderId},
    instruments::stubs::audusd_sim,
    instruments::Instrument,
    orders::{OrderAny, OrderTestBuilder},
    orders::Order,
    enums::{MarketStatusAction, OrderSide, OrderType},
    types::{Price, Quantity},
};
use nautilus_portfolio::Portfolio;
use nautilus_risk::engine::{RiskEngine, config::RiskEngineConfig};
use nautilus_rules::CommandRuleChain;

use nautilus_markets_ashare::{
    AShareRuleConfig, AShareSessionProvider, CancelSessionRule, MarketDataProvider, T1Ledger,
    create_ashare_rule_chain, create_ashare_rule_chain_with_provider,
};

fn register_process_handler() -> TypedIntoMessageSavingHandler<OrderEventAny> {
    let (handler, saving_handler) = get_typed_into_message_saving_handler::<OrderEventAny>(Some(
        "ExecEngine.process".into(),
    ));
    msgbus::register_order_event_endpoint(MessagingSwitchboard::exec_engine_process(), handler);
    saving_handler
}

struct TestProvider {
    status_action: Option<MarketStatusAction>,
    tick: Option<Price>,
    band: nautilus_markets_ashare::risk::rules::price_band::PriceBand,
}

impl MarketDataProvider for TestProvider {
    fn instrument_status_action(&self, _instrument_id: &InstrumentId) -> Option<MarketStatusAction> {
        self.status_action
    }

    fn tick_size(&self, _instrument_id: &InstrumentId) -> Option<Price> {
        self.tick
    }

    fn price_band(&self, _instrument_id: &InstrumentId) -> nautilus_markets_ashare::risk::rules::price_band::PriceBand {
        self.band
    }
}

fn build_engine(cache: Cache, clock: Rc<RefCell<TestClock>>) -> RiskEngine {
    let cache = Rc::new(RefCell::new(cache));
    let portfolio = Portfolio::new(cache.clone(), clock.clone(), None);
    RiskEngine::new(RiskEngineConfig::default(), portfolio, clock, cache)
}

fn register_execute_handler() -> TypedIntoMessageSavingHandler<TradingCommand> {
    let (handler, saving_handler) = get_typed_into_message_saving_handler::<TradingCommand>(Some(
        "ExecEngine.queue_execute".into(),
    ));
    msgbus::register_trading_command_endpoint(
        MessagingSwitchboard::exec_engine_queue_execute(),
        handler,
    );
    saving_handler
}

#[test]
fn test_instrument_suspended_rule_denies_submit_order() {
    let process_handler = register_process_handler();
    let execute_handler = register_execute_handler();

    let clock = Rc::new(RefCell::new(TestClock::new()));
    let mut cache = Cache::default();

    let instrument = nautilus_model::instruments::InstrumentAny::CurrencyPair(audusd_sim());
    cache.add_instrument(instrument.clone()).unwrap();
    cache
        .add_account(AccountAny::Cash(cash_account(
            cash_account_state_million_usd("1000000 USD", "0 USD", "1000000 USD"),
        )))
        .unwrap();

    let mut engine = build_engine(cache, clock);

    let rules_cfg = AShareRuleConfig::new();
    let instrument_id = instrument.id();
    let status_cb = Arc::new(move |iid: &InstrumentId| {
        if *iid == instrument_id {
            Some(MarketStatusAction::Suspend)
        } else {
            None
        }
    });

    let chain = create_ashare_rule_chain(&rules_cfg, Some(status_cb), None, None, None, None);
    engine.set_pre_trade_rules(Some(chain));

    let order: OrderAny = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .account_id(AccountId::from("SIM-001"))
        .price(Price::from("1"))
        .quantity(Quantity::from("100"))
        .build();

    engine
        .cache()
        .borrow_mut()
        .add_order(order.clone(), None, Some(ClientId::from("SIM")), false)
        .unwrap();

    let submit = SubmitOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("STRATEGY-001"),
        instrument.id(),
        order.client_order_id(),
        order.init_event().clone(),
        None,
        None,
        None,
        UUID4::new(),
        engine.clock().borrow().timestamp_ns(),
    );

    engine.execute(TradingCommand::SubmitOrder(submit));

    let process_msgs = process_handler.get_messages();
    assert_eq!(process_msgs.len(), 1);
    assert_eq!(process_msgs[0].event_type(), OrderEventType::Denied);
    assert!(process_msgs[0].message().unwrap_or_default().contains("INSTRUMENT_SUSPENDED"));

    let exec_msgs = execute_handler.get_messages();
    assert_eq!(exec_msgs.len(), 0);
}

fn add_default_account(cache: &mut Cache) {
    cache
        .add_account(AccountAny::Cash(cash_account(
            cash_account_state_million_usd("1000000 USD", "0 USD", "1000000 USD"),
        )))
        .unwrap();
}

#[test]
fn test_cancel_denied_in_locked_phase() {
    let process_handler = register_process_handler();
    let execute_handler = register_execute_handler();

    let ts_locked = nautilus_core::UnixNanos::from((1 * 3600 + 21 * 60) as u64 * 1_000_000_000);
    let clock = Rc::new(RefCell::new(TestClock::new()));
    clock.borrow_mut().set_time(ts_locked);

    let instrument = nautilus_model::instruments::InstrumentAny::CurrencyPair(audusd_sim());
    let mut cache = Cache::default();
    cache.add_instrument(instrument.clone()).unwrap();
    let mut engine = build_engine(cache, clock);

    let session_provider = Arc::new(AShareSessionProvider::default());
    let mut chain = CommandRuleChain::new();
    chain.add_rule(Arc::new(CancelSessionRule::new(session_provider)));
    engine.set_pre_trade_command_rules(Some(chain));

    let cancel = CancelOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("STRATEGY-001"),
        instrument.id(),
        ClientOrderId::from("O-TEST-LOCKED"),
        None,
        UUID4::new(),
        ts_locked,
        None,
    );
    engine.execute(TradingCommand::CancelOrder(cancel));

    let process_msgs = process_handler.get_messages();
    assert_eq!(process_msgs.len(), 1);
    assert_eq!(process_msgs[0].event_type(), OrderEventType::CancelRejected);
    assert!(process_msgs[0].message().unwrap_or_default().contains("CANCEL_DENIED"));

    let exec_msgs = execute_handler.get_messages();
    assert_eq!(exec_msgs.len(), 0);
}

#[test]
fn test_cancel_accepted_in_continuous_am() {
    let process_handler = register_process_handler();
    let execute_handler = register_execute_handler();

    let ts_am = nautilus_core::UnixNanos::from((2 * 3600) as u64 * 1_000_000_000);
    let clock = Rc::new(RefCell::new(TestClock::new()));
    clock.borrow_mut().set_time(ts_am);

    let instrument = nautilus_model::instruments::InstrumentAny::CurrencyPair(audusd_sim());
    let mut cache = Cache::default();
    cache.add_instrument(instrument.clone()).unwrap();
    let mut engine = build_engine(cache, clock);

    let session_provider = Arc::new(AShareSessionProvider::default());
    let mut chain = CommandRuleChain::new();
    chain.add_rule(Arc::new(CancelSessionRule::new(session_provider)));
    engine.set_pre_trade_command_rules(Some(chain));

    let cancel = CancelOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("STRATEGY-001"),
        instrument.id(),
        ClientOrderId::from("O-TEST-AM"),
        None,
        UUID4::new(),
        ts_am,
        None,
    );
    engine.execute(TradingCommand::CancelOrder(cancel));

    let process_msgs = process_handler.get_messages();
    assert_eq!(process_msgs.len(), 0);
    let exec_msgs = execute_handler.get_messages();
    assert_eq!(exec_msgs.len(), 1);
}

#[test]
fn test_tick_and_band_rules_deny_submit_order() {
    let process_handler = register_process_handler();
    let execute_handler = register_execute_handler();

    let clock = Rc::new(RefCell::new(TestClock::new()));
    let instrument = nautilus_model::instruments::InstrumentAny::CurrencyPair(audusd_sim());
    let mut cache = Cache::default();
    cache.add_instrument(instrument.clone()).unwrap();
    add_default_account(&mut cache);
    let mut engine = build_engine(cache, clock);

    let provider = Arc::new(TestProvider {
        status_action: None,
        tick: Some(Price::new(0.01, 2)),
        band: nautilus_markets_ashare::risk::rules::price_band::PriceBand {
            min_price: Some(Price::new(1.00, 2)),
            max_price: Some(Price::new(1.10, 2)),
        },
    });
    let rules_cfg = AShareRuleConfig::new();
    let chain = create_ashare_rule_chain_with_provider(&rules_cfg, provider);
    engine.set_pre_trade_rules(Some(chain));

    let order: OrderAny = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Buy)
        .price(Price::new(1.105, 3))
        .quantity(Quantity::from("100"))
        .build();
    engine
        .cache()
        .borrow_mut()
        .add_order(order.clone(), None, Some(ClientId::from("SIM")), false)
        .unwrap();
    let submit = SubmitOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("STRATEGY-001"),
        instrument.id(),
        order.client_order_id(),
        order.init_event().clone(),
        None,
        None,
        None,
        UUID4::new(),
        engine.clock().borrow().timestamp_ns(),
    );
    engine.execute(TradingCommand::SubmitOrder(submit));

    let process_msgs = process_handler.get_messages();
    assert_eq!(process_msgs.len(), 1);
    assert_eq!(process_msgs[0].event_type(), OrderEventType::Denied);
    let exec_msgs = execute_handler.get_messages();
    assert_eq!(exec_msgs.len(), 0);
}

#[test]
fn test_t1_rule_denies_when_sell_exceeds_sellable() {
    let process_handler = register_process_handler();
    let execute_handler = register_execute_handler();

    let clock = Rc::new(RefCell::new(TestClock::new()));
    let instrument = nautilus_model::instruments::InstrumentAny::CurrencyPair(audusd_sim());
    let mut cache = Cache::default();
    cache.add_instrument(instrument.clone()).unwrap();
    add_default_account(&mut cache);
    let mut engine = build_engine(cache, clock);

    let account_id = AccountId::from("SIM-001");
    let mut ledger = T1Ledger::new();
    ledger.load_position(account_id, instrument.id(), 1000.0, 500.0);
    let shared_ledger = Arc::new(std::sync::RwLock::new(ledger));

    let rules_cfg = AShareRuleConfig::new().with_t1(true, Some(shared_ledger));
    let chain = create_ashare_rule_chain(&rules_cfg, None, None, None, None, None);
    engine.set_pre_trade_rules(Some(chain));

    let order: OrderAny = OrderTestBuilder::new(OrderType::Limit)
        .instrument_id(instrument.id())
        .side(OrderSide::Sell)
        .price(Price::new(1.0, 2))
        .quantity(Quantity::new(600.0, 0))
        .build();
    engine
        .cache()
        .borrow_mut()
        .add_order(order.clone(), Some(account_id), Some(ClientId::from("SIM")), false)
        .unwrap();
    let submit = SubmitOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("STRATEGY-001"),
        instrument.id(),
        order.client_order_id(),
        order.init_event().clone(),
        None,
        None,
        None,
        UUID4::new(),
        engine.clock().borrow().timestamp_ns(),
    );
    engine.execute(TradingCommand::SubmitOrder(submit));

    let process_msgs = process_handler.get_messages();
    assert_eq!(process_msgs.len(), 1);
    assert_eq!(process_msgs[0].event_type(), OrderEventType::Denied);
    let exec_msgs = execute_handler.get_messages();
    assert_eq!(exec_msgs.len(), 0);

    let _ = execute_handler;
}
