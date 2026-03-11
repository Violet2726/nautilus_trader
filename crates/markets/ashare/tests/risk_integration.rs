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

//! A股风控规则集成测试
//!
//! 本文件覆盖以下场景（全部为规则链层面的单元测试，不依赖 RiskEngine）：
//!
//! 1. **交易时段规则**（SessionRule）—— 各阶段报单/拒绝
//! 2. **撤单时段规则**（CancelSessionRule）—— 各阶段撤单/拒绝
//! 3. **停牌状态规则**（InstrumentStatusRule）—— Halt/Suspend/正常
//! 4. **价格对齐规则**（PriceTickRule）—— 价格是否对齐最小变动
//! 5. **价格笼子/涨跌停规则**（PriceCageRule/PriceLimitRule）—— 越界/缺失数据
//! 6. **手数规则**（LotSizeRule）—— 主板/科创/创业板
//! 7. **T+1规则**（T1Rule）—— 可卖不足/充足/日切
//! 8. **限流规则**（ThrottlerRule）—— 账户/标的级别限流
//! 9. **规则链组装**（create_ashare_rule_chain）—— 端到端规则链
//! 10. **命令规则链**（create_ashare_command_rule_chain）—— CancelSessionRule 链

use std::sync::{Arc, RwLock};

use nautilus_common::throttler::RateLimit;
use nautilus_common::cache::Cache;
use nautilus_core::UnixNanos;
use nautilus_model::{
    data::{InstrumentStatus, QuoteTick, TradeTick},
    enums::{MarketStatusAction, OrderSide, OrderType},
    identifiers::{AccountId, ClientOrderId, InstrumentId, StrategyId, TradeId, TraderId, Venue},
    instruments::{Equity, InstrumentAny},
    orders::OrderTestBuilder,
    types::{Currency, Price, Quantity},
};
use nautilus_rules::{
    command::{CommandContext, CommandRule},
    common::{Rule, RuleContext},
};

use nautilus_markets_ashare::{
    AShareRuleConfig, AShareSessionProvider, CancelSessionRule, MarketDataProvider, T1Ledger,
    TradingPhase,
    create_ashare_rule_chain, create_ashare_rule_chain_with_provider,
    risk::rules::{
        instrument_status::InstrumentStatusRule,
        lot_size::LotSizeRule,
        price_cage,
        price_limit,
        price_tick::PriceTickRule,
        session::SessionRule,
        t1::T1Rule,
        throttler::ThrottlerRule,
    },
};
use nautilus_markets_ashare::risk::{CacheMarketDataProvider, MissingMarketDataPolicy};
use nautilus_markets_ashare::market::session::SessionProvider;
use nautilus_common::messages::execution::{CancelOrder, TradingCommand};

// ============================================================
// 测试辅助设施
// ============================================================

/// 固定的A股主板标的 ID
const SH_MAIN: &str = "600000.SH";
/// 固定的科创板标的 ID
const SH_STAR: &str = "688001.SH";
/// 固定的创业板标的 ID
const SZ_CHINEXT: &str = "300001.SZ";

/// 快速构建 `RuleContext`
fn make_ctx(
    symbol: &str,
    side: OrderSide,
    qty: f64,
    price: f64,
    account_id: Option<AccountId>,
    timestamp_ns: u64,
) -> RuleContext {
    let instrument_id = InstrumentId::from(symbol);
    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(TraderId::from("TRADER-001"))
        .strategy_id(StrategyId::from("S-001"))
        .instrument_id(instrument_id)
        .client_order_id(ClientOrderId::from("O-20240101-001"))
        .side(side)
        .quantity(Quantity::new(qty, 0))
        .price(Price::new(price, 2))
        .build();

    RuleContext::new(order, instrument_id, account_id, timestamp_ns)
}

/// 快速构建一个市价单 `RuleContext`（不含 price）
fn make_market_ctx(
    symbol: &str,
    side: OrderSide,
    qty: f64,
    account_id: Option<AccountId>,
    timestamp_ns: u64,
) -> RuleContext {
    let instrument_id = InstrumentId::from(symbol);
    let order = OrderTestBuilder::new(OrderType::Market)
        .trader_id(TraderId::from("TRADER-001"))
        .strategy_id(StrategyId::from("S-001"))
        .instrument_id(instrument_id)
        .client_order_id(ClientOrderId::from("O-20240101-002"))
        .side(side)
        .quantity(Quantity::new(qty, 0))
        .build();

    RuleContext::new(order, instrument_id, account_id, timestamp_ns)
}

fn make_equity_for_test(
    instrument_id: InstrumentId,
    raw_symbol: &str,
    price_precision: u8,
    price_increment: Price,
) -> InstrumentAny {
    Equity::new(
        instrument_id,
        nautilus_model::identifiers::Symbol::from(raw_symbol),
        Some(ustr::Ustr::from("TESTISIN")),
        Currency::from("CNY"),
        price_precision,
        price_increment,
        None,
        None,
        None,
        Some(Price::new(9999.0, price_precision)),
        Some(Price::new(0.001, price_precision)),
        None,
        None,
        None,
        None,
        None,
        UnixNanos::default(),
        UnixNanos::default(),
    )
    .into()
}

/// 模拟固定阶段的时段提供者
struct MockSessionProvider {
    phase: TradingPhase,
}

impl SessionProvider for MockSessionProvider {
    fn phase_at(&self, _venue: &Venue, _ts_ns: UnixNanos) -> TradingPhase {
        self.phase
    }
}

/// 模拟市场数据提供者，支持配置各字段
struct MockMarketDataProvider {
    status_action: Option<MarketStatusAction>,
    tick: Option<Price>,
    cage_data: price_cage::MarketData,
    limit_data: price_limit::PriceLimitMarketData,
}

impl Default for MockMarketDataProvider {
    fn default() -> Self {
        Self {
            status_action: None,
            tick: None,
            cage_data: price_cage::MarketData::default(),
            limit_data: price_limit::PriceLimitMarketData::default(),
        }
    }
}

impl MarketDataProvider for MockMarketDataProvider {
    fn instrument_status_action(&self, _id: &InstrumentId) -> Option<MarketStatusAction> {
        self.status_action
    }

    fn tick_size(&self, _id: &InstrumentId) -> Option<Price> {
        self.tick
    }

    fn price_cage_market_data(&self, _context: &nautilus_rules::common::RuleContext) -> price_cage::MarketData {
        self.cage_data.clone()
    }

    fn price_limit_market_data(&self, _context: &nautilus_rules::common::RuleContext) -> price_limit::PriceLimitMarketData {
        self.limit_data.clone()
    }
}

/// 将北京时间 (hour, minute) 转换为纳秒时间戳（假设周一工作日）
fn beijing_ts(hour: u32, minute: u32) -> u64 {
    // 北京时间 = UTC + 8
    let hour_utc = if hour >= 8 { hour - 8 } else { hour + 24 - 8 };
    let secs = hour_utc * 3600 + minute * 60;
    secs as u64 * 1_000_000_000
}

// ============================================================
// 1. 交易时段规则 —— SessionRule
// ============================================================

#[test]
fn test_session_rule_pass_continuous_am() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_session_rule_pass_continuous_pm() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousPm,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Sell, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_session_rule_pass_pre_auction_open() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionOpen,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_session_rule_pass_pre_auction_locked() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionLocked,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_session_rule_pass_closing_auction() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ClosingAuction,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_session_rule_fail_closed() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::Closed,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("OUT_OF_SESSION"));
}

#[test]
fn test_session_rule_fail_midday_break() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::MiddayBreak,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Sell, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("OUT_OF_SESSION"));
}

#[test]
fn test_session_rule_fail_pre_auction_silent() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionSilent,
    });
    let rule = SessionRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("OUT_OF_SESSION"));
}

#[test]
fn test_session_rule_disabled_always_pass() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::Closed,
    });
    let mut rule = SessionRule::new(provider);
    rule.set_enabled(false);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

// ============================================================
// 2. 撤单时段规则 —— CancelSessionRule
// ============================================================

fn make_cancel_cmd_ctx(instrument_id: InstrumentId, timestamp_ns: u64) -> CommandContext {
    let cancel = CancelOrder::new(
        TraderId::from("TRADER-001"),
        None,
        StrategyId::from("S-001"),
        instrument_id,
        ClientOrderId::from("O-CANCEL-001"),
        None,
        nautilus_core::UUID4::new(),
        timestamp_ns.into(),
        None,
    );
    CommandContext {
        command: TradingCommand::CancelOrder(cancel),
        instrument_id: Some(instrument_id),
        account_id: None,
        timestamp_ns,
    }
}

#[test]
fn test_cancel_session_rule_pass_continuous_am() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_cancel_session_rule_pass_continuous_pm() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousPm,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_cancel_session_rule_pass_pre_auction_open() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionOpen,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_cancel_session_rule_deny_pre_auction_locked() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionLocked,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("CANCEL_DENIED"));
}

#[test]
fn test_cancel_session_rule_deny_closing_auction() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::ClosingAuction,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("CANCEL_DENIED"));
}

#[test]
fn test_cancel_session_rule_deny_closed() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::Closed,
    });
    let rule = CancelSessionRule::new(provider);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("CANCEL_DENIED"));
}

#[test]
fn test_cancel_session_rule_disabled_always_pass() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionLocked,
    });
    let mut rule = CancelSessionRule::new(provider);
    rule.set_enabled(false);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(rule.check(&ctx).is_pass());
}

// ============================================================
// 3. 停牌状态规则 —— InstrumentStatusRule
// ============================================================

#[test]
fn test_instrument_status_rule_pass_when_no_status() {
    let provider = Arc::new(MockMarketDataProvider::default());
    let rule = InstrumentStatusRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_instrument_status_rule_pass_when_trading() {
    let provider = Arc::new(MockMarketDataProvider {
        status_action: Some(MarketStatusAction::Trading),
        ..Default::default()
    });
    let rule = InstrumentStatusRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_instrument_status_rule_fail_when_halt() {
    let provider = Arc::new(MockMarketDataProvider {
        status_action: Some(MarketStatusAction::Halt),
        ..Default::default()
    });
    let rule = InstrumentStatusRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("INSTRUMENT_SUSPENDED"));
}

#[test]
fn test_instrument_status_rule_fail_when_suspend() {
    let provider = Arc::new(MockMarketDataProvider {
        status_action: Some(MarketStatusAction::Suspend),
        ..Default::default()
    });
    let rule = InstrumentStatusRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("INSTRUMENT_SUSPENDED"));
}

#[test]
fn test_instrument_status_rule_fail_not_available() {
    let provider = Arc::new(MockMarketDataProvider {
        status_action: Some(MarketStatusAction::NotAvailableForTrading),
        ..Default::default()
    });
    let rule = InstrumentStatusRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("INSTRUMENT_SUSPENDED"));
}

// ============================================================
// 4. 价格对齐规则 —— PriceTickRule
// ============================================================

#[test]
fn test_price_tick_rule_pass_on_tick() {
    let provider = Arc::new(MockMarketDataProvider {
        tick: Some(Price::new(0.01, 2)),
        ..Default::default()
    });
    let rule = PriceTickRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.00, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_price_tick_rule_fail_off_tick() {
    let provider = Arc::new(MockMarketDataProvider {
        tick: Some(Price::new(0.01, 2)),
        ..Default::default()
    });
    let rule = PriceTickRule::new(provider);
    // 构造一个精度为 3 的价格（不在 0.01 tick 上）
    let instrument_id = InstrumentId::from(SH_MAIN);
    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(TraderId::from("TRADER-001"))
        .strategy_id(StrategyId::from("S-001"))
        .instrument_id(instrument_id)
        .client_order_id(ClientOrderId::from("O-TICK-001"))
        .side(OrderSide::Buy)
        .quantity(Quantity::new(100.0, 0))
        .price(Price::new(10.005, 3)) // 不在 0.01 tick 上
        .build();
    let ctx = RuleContext::new(order, instrument_id, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_NOT_ON_TICK"));
}

#[test]
fn test_price_tick_rule_pass_no_tick_configured() {
    let provider = Arc::new(MockMarketDataProvider::default());
    let rule = PriceTickRule::new(provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.005, None, 0);
    // tick_size 未配置 → 跳过检查 → Pass
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_price_tick_rule_pass_market_order() {
    let provider = Arc::new(MockMarketDataProvider {
        tick: Some(Price::new(0.01, 2)),
        ..Default::default()
    });
    let rule = PriceTickRule::new(provider);
    let ctx = make_market_ctx(SH_MAIN, OrderSide::Buy, 100.0, None, 0);
    // 市价单无价格 → Pass
    assert!(rule.check(&ctx).is_pass());
}

// ============================================================
// 5. 手数规则 —— LotSizeRule
// ============================================================

// --- 主板 (600xxx) ---

#[test]
fn test_lot_size_main_board_buy_100_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_main_board_buy_200_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 200.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_main_board_buy_150_fail() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 150.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("MAIN_BOARD_BUY_VIOLATION"));
}

#[test]
fn test_lot_size_main_board_buy_50_fail() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 50.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("MAIN_BOARD_BUY_VIOLATION"));
}

// --- 科创板 (688xxx) ---

#[test]
fn test_lot_size_star_market_buy_200_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_STAR, OrderSide::Buy, 200.0, 30.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_star_market_buy_201_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_STAR, OrderSide::Buy, 201.0, 30.0, None, 0);
    // 科创板1股递增 → 201 ok
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_star_market_buy_199_fail() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_STAR, OrderSide::Buy, 199.0, 30.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("STAR_MARKET_BUY_VIOLATION"));
}

// --- 创业板 (300xxx) ---

#[test]
fn test_lot_size_chinext_buy_100_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SZ_CHINEXT, OrderSide::Buy, 100.0, 20.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_chinext_buy_101_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SZ_CHINEXT, OrderSide::Buy, 101.0, 20.0, None, 0);
    // 创业板1股递增 → 101 ok
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_chinext_buy_99_fail() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SZ_CHINEXT, OrderSide::Buy, 99.0, 20.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("CHINEXT_BUY_VIOLATION"));
}

#[test]
fn test_lot_size_sell_pass() {
    let rule = LotSizeRule::new();
    let ctx = make_ctx(SH_MAIN, OrderSide::Sell, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_lot_size_disabled_always_pass() {
    let mut rule = LotSizeRule::new();
    rule.set_enabled(false);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 150.0, 10.0, None, 0);
    // 即使 qty=150 不合规，禁用后也通过
    assert!(rule.check(&ctx).is_pass());
}

// ============================================================
// 7. T+1 规则 —— T1Rule
// ============================================================

fn make_t1_rule(total_qty: f64, today_buy_qty: f64) -> T1Rule {
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        total_qty,
        today_buy_qty,
    );
    T1Rule::new(Arc::new(RwLock::new(ledger)))
}

#[test]
fn test_t1_rule_buy_always_pass() {
    let rule = make_t1_rule(0.0, 0.0);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Buy,
        100.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_rule_sell_within_sellable_pass() {
    // total=1000, today_buy=0 → sellable=1000
    let rule = make_t1_rule(1000.0, 0.0);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        500.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_rule_sell_exact_sellable_pass() {
    // total=1000, today_buy=0 → sellable=1000
    let rule = make_t1_rule(1000.0, 0.0);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        1000.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_rule_sell_exceeds_sellable_fail() {
    // total=1000, today_buy=500 → sellable=500
    let rule = make_t1_rule(1000.0, 500.0);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        600.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("EXCEEDS_SELLABLE"));
}

#[test]
fn test_t1_rule_sell_no_position_fail() {
    // 没有持仓 → sellable=0
    let ledger = T1Ledger::new();
    let rule = T1Rule::new(Arc::new(RwLock::new(ledger)));
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        100.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("EXCEEDS_SELLABLE"));
}

#[test]
fn test_t1_rule_sell_after_settlement_pass() {
    // 验证日切后 today_buy_qty 清零，释放出可卖
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        1000.0,
        1000.0,
    );
    // 日切前 → sellable=0
    assert_eq!(
        ledger.sellable(&AccountId::from("ACC-001"), &InstrumentId::from(SH_MAIN)),
        0.0
    );

    // 日切
    ledger.on_settlement();

    // 日切后 → sellable=1000
    assert_eq!(
        ledger.sellable(&AccountId::from("ACC-001"), &InstrumentId::from(SH_MAIN)),
        1000.0
    );

    let rule = T1Rule::new(Arc::new(RwLock::new(ledger)));
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        800.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_rule_disabled_skip_check() {
    let mut rule = make_t1_rule(100.0, 100.0); // sellable=0
    rule.set_enabled(false);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        100.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    // 禁用后即使可卖不足也通过
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_rule_sell_without_account_id_pass() {
    // 没有 account_id → 跳过检查
    let rule = make_t1_rule(0.0, 0.0);
    let ctx = make_ctx(SH_MAIN, OrderSide::Sell, 100.0, 10.0, None, 0);
    assert!(rule.check(&ctx).is_pass());
}

#[test]
fn test_t1_ledger_on_fill_buy_increases_today_buy() {
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        1000.0,
        0.0,
    );

    // 买入 500 → total=1500, today_buy=500, sellable=1000
    ledger.on_fill(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        OrderSide::Buy,
        500.0,
    );
    assert_eq!(
        ledger.sellable(&AccountId::from("ACC-001"), &InstrumentId::from(SH_MAIN)),
        1000.0,
    );
}

#[test]
fn test_t1_ledger_on_fill_sell_decreases_total() {
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        1000.0,
        0.0,
    );

    // 卖出 400 → total=600, today_buy=0, sellable=600
    ledger.on_fill(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        OrderSide::Sell,
        400.0,
    );
    assert_eq!(
        ledger.sellable(&AccountId::from("ACC-001"), &InstrumentId::from(SH_MAIN)),
        600.0,
    );
}

// ============================================================
// 8. 限流规则 —— ThrottlerRule
// ============================================================

#[test]
fn test_throttler_rule_no_limit_always_pass() {
    let rule = ThrottlerRule::new(None, None);
    for _ in 0..100 {
        let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
        assert!(rule.check(&ctx).is_pass());
    }
}

#[test]
fn test_throttler_rule_account_limit() {
    let limit = RateLimit {
        interval_ns: 1_000_000_000,
        limit: 3,
    };
    let rule = ThrottlerRule::new(Some(limit), None);
    let acct = Some(AccountId::from("ACC-001"));

    // 前3次通过
    for _ in 0..3 {
        let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
        assert!(rule.check(&ctx).is_pass());
    }

    // 第4次失败
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("THROTTLED"));
}

#[test]
fn test_throttler_rule_symbol_limit() {
    let limit = RateLimit {
        interval_ns: 1_000_000_000,
        limit: 2,
    };
    let rule = ThrottlerRule::new(None, Some(limit));

    // 同标的前2次通过
    for _ in 0..2 {
        let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
        assert!(rule.check(&ctx).is_pass());
    }

    // 同标的第3次失败
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = rule.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("THROTTLED"));

    // 不同标的仍通过
    let ctx2 = make_ctx(SH_STAR, OrderSide::Buy, 200.0, 30.0, None, 0);
    assert!(rule.check(&ctx2).is_pass());
}

#[test]
fn test_throttler_rule_reset_clears_state() {
    let limit = RateLimit {
        interval_ns: 1_000_000_000,
        limit: 1,
    };
    let mut rule = ThrottlerRule::new(Some(limit), None);
    let acct = Some(AccountId::from("ACC-001"));

    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
    assert!(rule.check(&ctx).is_pass());

    // 已满
    let ctx2 = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
    assert!(rule.check(&ctx2).is_fail());

    // 重置后再次通过
    rule.reset();
    let ctx3 = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
    assert!(rule.check(&ctx3).is_pass());
}

#[test]
fn test_throttler_rule_disabled_via_chain_always_pass() {
    // ThrottlerRule.check() 不检查 is_enabled()，is_enabled 由 RuleChain 负责。
    // 所以这里通过 RuleChain 测试禁用行为。
    use nautilus_rules::common::RuleChain;

    let limit = RateLimit {
        interval_ns: 1_000_000_000,
        limit: 1,
    };
    let mut rule = ThrottlerRule::new(Some(limit), None);
    rule.set_enabled(false);

    let mut chain = RuleChain::new();
    chain.add_rule(Arc::new(rule));

    let acct = Some(AccountId::from("ACC-001"));
    for _ in 0..10 {
        let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, acct, 0);
        assert!(chain.check(&ctx).is_pass());
    }
}

// ============================================================
// 9. 规则链组装 —— create_ashare_rule_chain
// ============================================================

#[test]
fn test_rule_chain_empty_config_always_pass() {
    let cfg = AShareRuleConfig::new();
    let chain = create_ashare_rule_chain(&cfg, None);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(chain.check(&ctx).is_pass());
}

#[test]
fn test_rule_chain_session_enabled_denies_closed() {
    let provider = Arc::new(MockSessionProvider {
        phase: TradingPhase::Closed,
    });
    let cfg = AShareRuleConfig::new().with_session(true, Some(provider as Arc<dyn SessionProvider>));
    let chain = create_ashare_rule_chain(&cfg, None);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("OUT_OF_SESSION"));
}

#[test]
fn test_rule_chain_with_provider_denies_suspended() {
    let session = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let data_provider = Arc::new(MockMarketDataProvider {
        status_action: Some(MarketStatusAction::Suspend),
        ..Default::default()
    });
    let cfg = AShareRuleConfig::new().with_session(true, Some(session as Arc<dyn SessionProvider>));
    let chain = create_ashare_rule_chain(&cfg, Some(data_provider));
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("INSTRUMENT_SUSPENDED"));
}

#[test]
fn test_rule_chain_with_provider_denies_price_above_limit() {
    let data_provider = Arc::new(MockMarketDataProvider {
        status_action: None,
        tick: Some(Price::new(0.01, 2)),
        limit_data: price_limit::PriceLimitMarketData {
            prev_close: Some(Price::new(10.0, 2)),
            tick_size: Some(Price::new(0.01, 2)),
            stock_name: Some("正常股票".to_string()),
        },
        ..Default::default()
    });
    let cfg = AShareRuleConfig::new().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(data_provider));
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 12.0, None, 0);
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_ABOVE_UP_LIMIT"));
}

#[test]
fn test_rule_chain_t1_enabled_denies_exceeds_sellable() {
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        1000.0,
        800.0,
    );
    // sellable = 200

    let cfg = AShareRuleConfig::new().with_t1(true, Some(Arc::new(RwLock::new(ledger))));
    let chain = create_ashare_rule_chain(&cfg, None);
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        300.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("EXCEEDS_SELLABLE"));
}

#[test]
fn test_rule_chain_lot_size_enabled_denies_non_multiple() {
    let cfg = AShareRuleConfig::new().with_lot_size(true);
    let chain = create_ashare_rule_chain(&cfg, None);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 150.0, 10.0, None, 0);
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("MAIN_BOARD_BUY_VIOLATION"));
}

#[test]
fn test_rule_chain_all_rules_pass() {
    let session = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let data_provider = Arc::new(MockMarketDataProvider {
        status_action: None,
        tick: Some(Price::new(0.01, 2)),
        limit_data: price_limit::PriceLimitMarketData {
            prev_close: Some(Price::new(10.0, 2)),
            tick_size: Some(Price::new(0.01, 2)),
            stock_name: Some("正常股票".to_string()),
        },
        ..Default::default()
    });
    let mut ledger = T1Ledger::new();
    ledger.load_position(
        AccountId::from("ACC-001"),
        InstrumentId::from(SH_MAIN),
        10000.0,
        0.0,
    );

    let cfg = AShareRuleConfig::new()
        .with_session(true, Some(session as Arc<dyn SessionProvider>))
        .with_lot_size(true)
        .with_t1(true, Some(Arc::new(RwLock::new(ledger))));

    let chain = create_ashare_rule_chain(&cfg, Some(data_provider));
    let ctx = make_ctx(
        SH_MAIN,
        OrderSide::Sell,
        100.0,
        10.0,
        Some(AccountId::from("ACC-001")),
        0,
    );
    assert!(chain.check(&ctx).is_pass());
}

#[test]
fn test_rule_chain_with_provider_shortcut() {
    // 测试 create_ashare_rule_chain_with_provider 快捷函数
    let data_provider = Arc::new(MockMarketDataProvider::default());
    let cfg = AShareRuleConfig::new();
    let chain = create_ashare_rule_chain_with_provider(&cfg, data_provider);
    let ctx = make_ctx(SH_MAIN, OrderSide::Buy, 100.0, 10.0, None, 0);
    assert!(chain.check(&ctx).is_pass());
}

// ============================================================
// 10. 命令规则链 —— create_ashare_command_rule_chain
// ============================================================

#[test]
fn test_command_rule_chain_denies_cancel_in_locked_phase() {
    use nautilus_markets_ashare::create_ashare_command_rule_chain;

    let session = Arc::new(MockSessionProvider {
        phase: TradingPhase::PreAuctionLocked,
    });
    let cfg = AShareRuleConfig::new().with_session(true, Some(session as Arc<dyn SessionProvider>));
    let chain = create_ashare_command_rule_chain(&cfg, None);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    let res = chain.check(&ctx);
    assert!(res.is_fail());
    assert!(res.to_string().contains("CANCEL_DENIED"));
}

#[test]
fn test_command_rule_chain_allows_cancel_in_continuous() {
    use nautilus_markets_ashare::create_ashare_command_rule_chain;

    let session = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let cfg = AShareRuleConfig::new().with_session(true, Some(session as Arc<dyn SessionProvider>));
    let chain = create_ashare_command_rule_chain(&cfg, None);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(chain.check(&ctx).is_pass());
}

#[test]
fn test_command_rule_chain_empty_config_always_pass() {
    use nautilus_markets_ashare::create_ashare_command_rule_chain;

    let cfg = AShareRuleConfig::new();
    let chain = create_ashare_command_rule_chain(&cfg, None);
    let ctx = make_cancel_cmd_ctx(InstrumentId::from(SH_MAIN), 0);
    assert!(chain.check(&ctx).is_pass());
}

// ============================================================
// 11. AShareSessionProvider 真实时间戳测试
// ============================================================

#[test]
fn test_ashare_session_provider_phases() {
    let provider = AShareSessionProvider::default();
    let venue = Venue::new(ustr::ustr("XSHG"));

    // 北京时间 09:14 → Closed
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(9, 14))),
        TradingPhase::Closed,
    );
    // 北京时间 09:15 → PreAuctionOpen
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(9, 15))),
        TradingPhase::PreAuctionOpen,
    );
    // 北京时间 09:20 → PreAuctionLocked
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(9, 20))),
        TradingPhase::PreAuctionLocked,
    );
    // 北京时间 09:25 → PreAuctionSilent
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(9, 25))),
        TradingPhase::PreAuctionSilent,
    );
    // 北京时间 09:30 → ContinuousAm
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(9, 30))),
        TradingPhase::ContinuousAm,
    );
    // 北京时间 11:30 → MiddayBreak
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(11, 30))),
        TradingPhase::MiddayBreak,
    );
    // 北京时间 13:00 → ContinuousPm
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(13, 0))),
        TradingPhase::ContinuousPm,
    );
    // 北京时间 14:57 → ClosingAuction
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(14, 57))),
        TradingPhase::ClosingAuction,
    );
    // 北京时间 15:00 → Closed
    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(beijing_ts(15, 0))),
        TradingPhase::Closed,
    );
}

#[test]
fn test_ashare_session_provider_with_holiday() {
    // 添加 2024-05-01（劳动节假期）
    let provider = AShareSessionProvider::new(Some(vec![20240501]));
    let venue = Venue::new(ustr::ustr("XSHG"));

    // 2024-05-01 是周三，如果加入假日表后应返回 Closed
    // 构造 2024-05-01 09:30 北京时间的时间戳
    let ts_may_01 = chrono::NaiveDate::from_ymd_opt(2024, 5, 1)
        .unwrap()
        .and_hms_opt(1, 30, 0) // UTC 01:30 = 北京 09:30
        .unwrap()
        .and_utc()
        .timestamp() as u64
        * 1_000_000_000;

    assert_eq!(
        provider.phase_at(&venue, UnixNanos::from(ts_may_01)),
        TradingPhase::Closed,
    );
}

// ============================================================
// 12. TradingPhase 方法验证
// ============================================================

#[test]
fn test_trading_phase_can_accept_order() {
    assert!(TradingPhase::PreAuctionOpen.can_accept_order());
    assert!(TradingPhase::PreAuctionLocked.can_accept_order());
    assert!(TradingPhase::ContinuousAm.can_accept_order());
    assert!(TradingPhase::ContinuousPm.can_accept_order());
    assert!(TradingPhase::ClosingAuction.can_accept_order());

    assert!(!TradingPhase::PreAuctionSilent.can_accept_order());
    assert!(!TradingPhase::MiddayBreak.can_accept_order());
    assert!(!TradingPhase::Closed.can_accept_order());
}

#[test]
fn test_trading_phase_can_cancel_order() {
    assert!(TradingPhase::PreAuctionOpen.can_cancel_order());
    assert!(TradingPhase::ContinuousAm.can_cancel_order());
    assert!(TradingPhase::ContinuousPm.can_cancel_order());

    assert!(!TradingPhase::PreAuctionLocked.can_cancel_order());
    assert!(!TradingPhase::PreAuctionSilent.can_cancel_order());
    assert!(!TradingPhase::MiddayBreak.can_cancel_order());
    assert!(!TradingPhase::ClosingAuction.can_cancel_order());
    assert!(!TradingPhase::Closed.can_cancel_order());
}

#[test]
fn test_trading_phase_is_continuous() {
    assert!(TradingPhase::ContinuousAm.is_continuous());
    assert!(TradingPhase::ContinuousPm.is_continuous());

    assert!(!TradingPhase::PreAuctionOpen.is_continuous());
    assert!(!TradingPhase::ClosingAuction.is_continuous());
    assert!(!TradingPhase::Closed.is_continuous());
}

#[test]
fn test_trading_phase_is_auction() {
    assert!(TradingPhase::PreAuctionOpen.is_auction());
    assert!(TradingPhase::PreAuctionLocked.is_auction());
    assert!(TradingPhase::ClosingAuction.is_auction());

    assert!(!TradingPhase::ContinuousAm.is_auction());
    assert!(!TradingPhase::Closed.is_auction());
}

// ============================================================
// 13. 规则链配置 builder 验证
// ============================================================

#[test]
fn test_ashare_rule_config_defaults() {
    let cfg = AShareRuleConfig::new();
    assert!(!cfg.session_enabled);
    assert!(!cfg.price_cage_enabled);
    assert!(!cfg.price_limit_enabled);
    assert_eq!(cfg.missing_market_data_policy, MissingMarketDataPolicy::FailOpen);
    assert!(!cfg.t1_enabled);
    assert!(!cfg.lot_size_enabled);
    assert!(cfg.session_provider.is_none());
    assert!(cfg.t1_ledger.is_none());
}

#[test]
fn test_ashare_rule_config_production_uses_fail_close() {
    let cfg = AShareRuleConfig::production();
    assert_eq!(cfg.missing_market_data_policy, MissingMarketDataPolicy::FailClose);
}

#[test]
fn test_ashare_rule_config_builder_chain() {
    let session = Arc::new(AShareSessionProvider::default());
    let ledger = Arc::new(RwLock::new(T1Ledger::new()));

    let cfg = AShareRuleConfig::new()
        .with_session(true, Some(session as Arc<dyn SessionProvider>))
        .with_price_cage(true, 0.05)
        .with_price_limit(true)
        .with_t1(true, Some(ledger))
        .with_lot_size(true)
        .with_throttling(
            Some(RateLimit::new(10, 1_000_000_000)),
            Some(RateLimit::new(5, 1_000_000_000)),
        );

    assert!(cfg.session_enabled);
    assert!(cfg.price_cage_enabled);
    assert_eq!(cfg.price_cage_pct, 0.05);
    assert!(cfg.price_limit_enabled);
    assert!(cfg.t1_enabled);
    assert!(cfg.lot_size_enabled);
    assert!(cfg.session_provider.is_some());
    assert!(cfg.t1_ledger.is_some());
    assert!(cfg.max_order_submit_per_account.is_some());
    assert!(cfg.max_order_submit_per_symbol.is_some());
}

// ============================================================
// 14. 真实 Cache provider 回放端到端测试
// ============================================================

#[test]
fn test_real_provider_price_limit_missing_data_fail_close() {
    let instrument_id = InstrumentId::from("600111.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600111",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::production().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600111.SH", OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_fail());
    assert!(res.to_string().contains("MISSING_MARKET_DATA"));
    assert!(res.to_string().contains("ASHARE_PRICE_LIMIT_MISSING_PREV_CLOSE"));
}

#[test]
fn test_real_provider_price_limit_missing_data_fail_open() {
    let instrument_id = InstrumentId::from("600112.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600112",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::new()
        .with_price_limit(true)
        .with_missing_market_data_policy(MissingMarketDataPolicy::FailOpen);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600112.SH", OrderSide::Buy, 100.0, 10.0, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_pass());
}

#[test]
fn test_real_provider_price_cage_violation_with_replay_quote() {
    let instrument_id = InstrumentId::from("600113.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600113",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();
    cache
        .add_quote(QuoteTick::new(
            instrument_id,
            Price::new(10.00, 2),
            Price::new(10.10, 2),
            Quantity::new(100.0, 0),
            Quantity::new(100.0, 0),
            UnixNanos::default(),
            UnixNanos::default(),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let session = Arc::new(MockSessionProvider {
        phase: TradingPhase::ContinuousAm,
    });
    let cfg = AShareRuleConfig::production()
        .with_session(false, Some(session as Arc<dyn SessionProvider>))
        .with_price_cage(true, 0.02);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600113.SH", OrderSide::Buy, 100.0, 10.31, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_CAGE_VIOLATION"));
}

#[test]
fn test_real_provider_price_limit_respects_instrument_precision() {
    let instrument_id = InstrumentId::from("600114.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600114",
            3,
            Price::new(0.001, 3),
        ))
        .unwrap();
    cache
        .add_trade(TradeTick::new(
            instrument_id,
            Price::new(10.123, 3),
            Quantity::new(100.0, 0),
            nautilus_model::enums::AggressorSide::NoAggressor,
            TradeId::new("T-600114"),
            UnixNanos::default(),
            UnixNanos::default(),
        ))
        .unwrap();
    cache
        .add_instrument_status(InstrumentStatus::new(
            instrument_id,
            MarketStatusAction::Trading,
            UnixNanos::default(),
            UnixNanos::default(),
            None,
            None,
            Some(true),
            Some(true),
            None,
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::production().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));

    let order = OrderTestBuilder::new(OrderType::Limit)
        .trader_id(TraderId::from("TRADER-001"))
        .strategy_id(StrategyId::from("S-001"))
        .instrument_id(instrument_id)
        .client_order_id(ClientOrderId::from("O-PREC-001"))
        .side(OrderSide::Buy)
        .quantity(Quantity::new(100.0, 0))
        .price(Price::new(11.136, 3))
        .build();
    let ctx = RuleContext::new(order, instrument_id, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_pass());
}

#[test]
fn test_real_provider_price_limit_fail_above_limit_up() {
    let instrument_id = InstrumentId::from("600115.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600115",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();
    cache
        .add_trade(TradeTick::new(
            instrument_id,
            Price::new(10.00, 2),
            Quantity::new(100.0, 0),
            nautilus_model::enums::AggressorSide::NoAggressor,
            TradeId::new("T-600115"),
            UnixNanos::default(),
            UnixNanos::default(),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::production().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600115.SH", OrderSide::Buy, 100.0, 11.01, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_ABOVE_UP_LIMIT"));
}

#[test]
fn test_real_provider_price_limit_fail_below_limit_down() {
    let instrument_id = InstrumentId::from("600116.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "600116",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();
    cache
        .add_trade(TradeTick::new(
            instrument_id,
            Price::new(10.00, 2),
            Quantity::new(100.0, 0),
            nautilus_model::enums::AggressorSide::NoAggressor,
            TradeId::new("T-600116"),
            UnixNanos::default(),
            UnixNanos::default(),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::production().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600116.SH", OrderSide::Sell, 100.0, 8.99, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_BELOW_DOWN_LIMIT"));
}

#[test]
fn test_real_provider_price_limit_st_stock_uses_5pct() {
    let instrument_id = InstrumentId::from("600117.SH");
    let mut cache = Cache::default();
    cache
        .add_instrument(make_equity_for_test(
            instrument_id,
            "ST600117",
            2,
            Price::new(0.01, 2),
        ))
        .unwrap();
    cache
        .add_trade(TradeTick::new(
            instrument_id,
            Price::new(10.00, 2),
            Quantity::new(100.0, 0),
            nautilus_model::enums::AggressorSide::NoAggressor,
            TradeId::new("T-600117"),
            UnixNanos::default(),
            UnixNanos::default(),
        ))
        .unwrap();

    let provider = Arc::new(CacheMarketDataProvider::new(Arc::new(std::sync::Mutex::new(cache))));
    let cfg = AShareRuleConfig::production().with_price_limit(true);
    let chain = create_ashare_rule_chain(&cfg, Some(provider));
    let ctx = make_ctx("600117.SH", OrderSide::Buy, 100.0, 10.60, None, 0);
    let res = chain.check(&ctx);

    assert!(res.is_fail());
    assert!(res.to_string().contains("PRICE_ABOVE_UP_LIMIT"));
}
