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

use super::super::common::{Rule, RuleCheckResult, RuleContext};
use ahash::AHashMap;
use nautilus_common::throttler::RateLimit;
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::{AccountId, InstrumentId};
use std::collections::VecDeque;
use std::sync::RwLock;

/// 用于限制订单提交的规则。
///
/// 此规则在账户和标的级别强制执行订单提交速率限制。
/// 使用滑动时间窗口算法，记录每个请求的时间戳，
/// 确保在任意时间窗口内不超过限制次数。
#[derive(Debug)]
pub struct ThrottlerRule {
    /// 账户级别的请求时间戳记录
    account_timestamps: RwLock<AHashMap<AccountId, VecDeque<UnixNanos>>>,
    /// 标的级别的请求时间戳记录
    symbol_timestamps: RwLock<AHashMap<InstrumentId, VecDeque<UnixNanos>>>,
    /// 账户级别的速率限制配置
    max_order_submit_per_account: Option<RateLimit>,
    /// 标的级别的速率限制配置
    max_order_submit_per_symbol: Option<RateLimit>,
    enabled: bool,
}

impl ThrottlerRule {
    pub fn new(
        max_order_submit_per_account: Option<RateLimit>,
        max_order_submit_per_symbol: Option<RateLimit>,
    ) -> Self {
        Self {
            account_timestamps: RwLock::new(AHashMap::new()),
            symbol_timestamps: RwLock::new(AHashMap::new()),
            max_order_submit_per_account,
            max_order_submit_per_symbol,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 检查账户级别的速率限制（滑动时间窗口实现）
    fn check_account_limit(&self, account_id: &AccountId, now: UnixNanos) -> RuleCheckResult {
        if let Some(rate_limit) = &self.max_order_submit_per_account {
            let mut timestamps = self.account_timestamps.write().unwrap();
            let queue = timestamps.entry(*account_id).or_insert_with(VecDeque::new);

            // 移除窗口外的时间戳
            let window_start = now.as_u64().saturating_sub(rate_limit.interval_ns);
            while let Some(&ts) = queue.front() {
                if ts.as_u64() < window_start {
                    queue.pop_front();
                } else {
                    break;
                }
            }

            // 检查是否超过限制
            if queue.len() >= rate_limit.limit {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "THROTTLED: 账户 {} 超出速率限制（{}/{}）",
                        account_id,
                        queue.len(),
                        rate_limit.limit
                    ),
                };
            }

            // 记录当前请求时间戳
            queue.push_back(now);
        }
        RuleCheckResult::Pass
    }

    /// 检查标的级别的速率限制（滑动时间窗口实现）
    fn check_symbol_limit(&self, instrument_id: &InstrumentId, now: UnixNanos) -> RuleCheckResult {
        if let Some(rate_limit) = &self.max_order_submit_per_symbol {
            let mut timestamps = self.symbol_timestamps.write().unwrap();
            let queue = timestamps.entry(*instrument_id).or_insert_with(VecDeque::new);

            // 移除窗口外的时间戳
            let window_start = now.as_u64().saturating_sub(rate_limit.interval_ns);
            while let Some(&ts) = queue.front() {
                if ts.as_u64() < window_start {
                    queue.pop_front();
                } else {
                    break;
                }
            }

            // 检查是否超过限制
            if queue.len() >= rate_limit.limit {
                return RuleCheckResult::Fail {
                    reason: format!(
                        "THROTTLED: 标的 {} 超出速率限制（{}/{}）",
                        instrument_id,
                        queue.len(),
                        rate_limit.limit
                    ),
                };
            }

            // 记录当前请求时间戳
            queue.push_back(now);
        }
        RuleCheckResult::Pass
    }
}

impl Default for ThrottlerRule {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl Rule for ThrottlerRule {
    fn name(&self) -> &str {
        "ThrottlerRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &RuleContext) -> RuleCheckResult {
        if self.max_order_submit_per_account.is_none() && self.max_order_submit_per_symbol.is_none() {
            return RuleCheckResult::Pass;
        }

        let now = UnixNanos::from(context.timestamp_ns);

        if let Some(account_id) = &context.account_id {
            let result = self.check_account_limit(account_id, now);
            if !result.is_pass() {
                return result;
            }
        }

        let result = self.check_symbol_limit(&context.instrument_id, now);
        if !result.is_pass() {
            return result;
        }

        RuleCheckResult::Pass
    }

    fn reset(&mut self) {
        self.account_timestamps.write().unwrap().clear();
        self.symbol_timestamps.write().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_model::enums::{OrderSide, OrderType};
    use nautilus_model::identifiers::{AccountId, ClientOrderId, InstrumentId, StrategyId, TraderId};
    use nautilus_model::orders::OrderTestBuilder;
    use nautilus_model::types::{Price, Quantity};

    fn create_test_context(
        symbol: &str,
        account_id: Option<AccountId>,
    ) -> RuleContext {
        let order = OrderTestBuilder::new(OrderType::Limit)
            .trader_id(TraderId::from("TRADER-001"))
            .strategy_id(StrategyId::from("STRATEGY-001"))
            .instrument_id(InstrumentId::from(symbol))
            .client_order_id(ClientOrderId::from("O-20240101-001"))
            .side(OrderSide::Buy)
            .quantity(Quantity::new(100.0, 0))
            .price(Price::new(10.0, 2))
            .build();

        RuleContext::new(
            order,
            InstrumentId::from(symbol),
            account_id,
            0,
        )
    }

    #[test]
    fn test_throttler_rule_pass() {
        let rate_limit = RateLimit::new(10, 1_000_000_000);
        let rule = ThrottlerRule::new(Some(rate_limit), None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_no_limits() {
        let rule = ThrottlerRule::new(None, None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_disabled() {
        let rate_limit = RateLimit::new(1, 1_000_000_000);
        let mut rule = ThrottlerRule::new(Some(rate_limit), None);
        rule.set_enabled(false);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        let result = rule.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_throttler_rule_reset() {
        let rate_limit = RateLimit::new(10, 1_000_000_000);
        let mut rule = ThrottlerRule::new(Some(rate_limit), None);
        let context = create_test_context(
            "600000.SH",
            Some(AccountId::from("ACC-001")),
        );

        rule.check(&context);
        assert_eq!(rule.account_timestamps.read().unwrap().len(), 1);

        rule.reset();
        assert_eq!(rule.account_timestamps.read().unwrap().len(), 0);
    }

    #[test]
    fn test_throttler_rule_sliding_window() {
        // 测试滑动时间窗口：限制为每 1 秒最多 2 次请求
        let rate_limit = RateLimit::new(2, 1_000_000_000); // 2 次/秒
        let rule = ThrottlerRule::new(Some(rate_limit), None);
        
        let account_id = AccountId::from("ACC-001");
        let instrument_id = InstrumentId::from("600000.SH");
        
        // 第 1 次请求（时间 0）- 应该通过
        let context1 = RuleContext::new(
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(TraderId::from("TRADER-001"))
                .strategy_id(StrategyId::from("STRATEGY-001"))
                .instrument_id(instrument_id)
                .client_order_id(ClientOrderId::from("O-001"))
                .side(OrderSide::Buy)
                .quantity(Quantity::new(100.0, 0))
                .price(Price::new(10.0, 2))
                .build(),
            instrument_id,
            Some(account_id),
            0, // 时间 0
        );
        assert!(rule.check(&context1).is_pass());
        
        // 第 2 次请求（时间 0.5 秒）- 应该通过
        let context2 = RuleContext::new(
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(TraderId::from("TRADER-001"))
                .strategy_id(StrategyId::from("STRATEGY-001"))
                .instrument_id(instrument_id)
                .client_order_id(ClientOrderId::from("O-002"))
                .side(OrderSide::Buy)
                .quantity(Quantity::new(100.0, 0))
                .price(Price::new(10.0, 2))
                .build(),
            instrument_id,
            Some(account_id),
            500_000_000, // 时间 0.5 秒
        );
        assert!(rule.check(&context2).is_pass());
        
        // 第 3 次请求（时间 0.8 秒）- 应该失败（窗口内已有 2 次）
        let context3 = RuleContext::new(
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(TraderId::from("TRADER-001"))
                .strategy_id(StrategyId::from("STRATEGY-001"))
                .instrument_id(instrument_id)
                .client_order_id(ClientOrderId::from("O-003"))
                .side(OrderSide::Buy)
                .quantity(Quantity::new(100.0, 0))
                .price(Price::new(10.0, 2))
                .build(),
            instrument_id,
            Some(account_id),
            800_000_000, // 时间 0.8 秒
        );
        let result3 = rule.check(&context3);
        assert!(result3.is_fail());
        assert!(result3.to_string().contains("超出速率限制"));
        
        // 第 4 次请求（时间 1.1 秒）- 应该通过（第 1 次请求已滑出窗口）
        let context4 = RuleContext::new(
            OrderTestBuilder::new(OrderType::Limit)
                .trader_id(TraderId::from("TRADER-001"))
                .strategy_id(StrategyId::from("STRATEGY-001"))
                .instrument_id(instrument_id)
                .client_order_id(ClientOrderId::from("O-004"))
                .side(OrderSide::Buy)
                .quantity(Quantity::new(100.0, 0))
                .price(Price::new(10.0, 2))
                .build(),
            instrument_id,
            Some(account_id),
            1_100_000_000, // 时间 1.1 秒
        );
        assert!(rule.check(&context4).is_pass());
    }
}
