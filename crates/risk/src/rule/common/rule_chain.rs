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

use super::rule_trait::{Rule, RuleCheckResult, RuleContext};
use std::sync::Arc;

/// 规则链，按顺序应用多个规则。
///
/// 规则按添加的顺序进行检查。如果任何规则失败，
/// 链将停止并返回失败原因。如果所有规则都通过，
/// 链返回 `RuleCheckResult::Pass`。
#[derive(Debug, Default)]
pub struct RuleChain {
    rules: Vec<Arc<dyn Rule>>,
}

impl RuleChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// 向链添加规则。
    pub fn add_rule(&mut self, rule: Arc<dyn Rule>) {
        self.rules.push(rule);
    }

    /// 向链添加多个规则。
    pub fn add_rules(&mut self, rules: Vec<Arc<dyn Rule>>) {
        self.rules.extend(rules);
    }

    /// 根据给定上下文检查链中的所有规则。
    ///
    /// 规则按顺序检查。如果任何规则失败，链将停止
    /// 并返回失败。如果所有规则都通过，返回 `Pass`。
    pub fn check(&self, context: &RuleContext) -> RuleCheckResult {
        for rule in &self.rules {
            if !rule.is_enabled() {
                continue;
            }

            let result = rule.check(context);
            if !result.is_pass() {
                return result;
            }
        }
        RuleCheckResult::Pass
    }

    /// 返回链中的规则数量。
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// 如果链中没有规则则返回 true。
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// 返回链中规则的迭代器。
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Rule>> {
        self.rules.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestRule {
        name: String,
        enabled: std::sync::RwLock<bool>,
        should_pass: bool,
    }

    impl TestRule {
        fn new(name: &str, should_pass: bool) -> Self {
            Self {
                name: name.to_string(),
                enabled: std::sync::RwLock::new(true),
                should_pass,
            }
        }

        fn set_enabled(&self, enabled: bool) {
            *self.enabled.write().unwrap() = enabled;
        }
    }

    impl Rule for TestRule {
        fn name(&self) -> &str {
            &self.name
        }

        fn is_enabled(&self) -> bool {
            *self.enabled.read().unwrap()
        }

        fn check(&self, _context: &RuleContext) -> RuleCheckResult {
            if self.should_pass {
                RuleCheckResult::Pass
            } else {
                RuleCheckResult::Fail {
                    reason: format!("{} failed", self.name),
                }
            }
        }
    }

    #[test]
    fn test_rule_chain_all_pass() {
        let mut chain = RuleChain::new();
        chain.add_rule(Arc::new(TestRule::new("Rule1", true)));
        chain.add_rule(Arc::new(TestRule::new("Rule2", true)));
        chain.add_rule(Arc::new(TestRule::new("Rule3", true)));

        let order = nautilus_model::orders::OrderTestBuilder::new(nautilus_model::enums::OrderType::Limit)
            .trader_id(nautilus_model::identifiers::TraderId::from("TRADER-001"))
            .strategy_id(nautilus_model::identifiers::StrategyId::from("STRATEGY-001"))
            .instrument_id(nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"))
            .client_order_id(nautilus_model::identifiers::ClientOrderId::from("O-001"))
            .side(nautilus_model::enums::OrderSide::Buy)
            .quantity(nautilus_model::types::Quantity::new(100.0, 0))
            .price(nautilus_model::types::Price::new(10.0, 2))
            .build();

        let context = RuleContext::new(
            order,
            nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"),
            None,
            0,
        );

        let result = chain.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_rule_chain_one_fails() {
        let mut chain = RuleChain::new();
        chain.add_rule(Arc::new(TestRule::new("Rule1", true)));
        chain.add_rule(Arc::new(TestRule::new("Rule2", false)));
        chain.add_rule(Arc::new(TestRule::new("Rule3", true)));

        let order = nautilus_model::orders::OrderTestBuilder::new(nautilus_model::enums::OrderType::Limit)
            .trader_id(nautilus_model::identifiers::TraderId::from("TRADER-001"))
            .strategy_id(nautilus_model::identifiers::StrategyId::from("STRATEGY-001"))
            .instrument_id(nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"))
            .client_order_id(nautilus_model::identifiers::ClientOrderId::from("O-001"))
            .side(nautilus_model::enums::OrderSide::Buy)
            .quantity(nautilus_model::types::Quantity::new(100.0, 0))
            .price(nautilus_model::types::Price::new(10.0, 2))
            .build();

        let context = RuleContext::new(
            order,
            nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"),
            None,
            0,
        );

        let result = chain.check(&context);
        assert!(result.is_fail());
        assert_eq!(result, RuleCheckResult::Fail { reason: "Rule2 failed".to_string() });
    }

    #[test]
    fn test_rule_chain_disabled_rule() {
        let mut chain = RuleChain::new();
        let rule2 = Arc::new(TestRule::new("Rule2", false));
        rule2.set_enabled(false);
        chain.add_rule(Arc::new(TestRule::new("Rule1", true)));
        chain.add_rule(rule2);
        chain.add_rule(Arc::new(TestRule::new("Rule3", true)));

        let order = nautilus_model::orders::OrderTestBuilder::new(nautilus_model::enums::OrderType::Limit)
            .trader_id(nautilus_model::identifiers::TraderId::from("TRADER-001"))
            .strategy_id(nautilus_model::identifiers::StrategyId::from("STRATEGY-001"))
            .instrument_id(nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"))
            .client_order_id(nautilus_model::identifiers::ClientOrderId::from("O-001"))
            .side(nautilus_model::enums::OrderSide::Buy)
            .quantity(nautilus_model::types::Quantity::new(100.0, 0))
            .price(nautilus_model::types::Price::new(10.0, 2))
            .build();

        let context = RuleContext::new(
            order,
            nautilus_model::identifiers::InstrumentId::from("AAPL.XNAS"),
            None,
            0,
        );

        let result = chain.check(&context);
        assert!(result.is_pass());
    }

    #[test]
    fn test_rule_chain_empty() {
        let chain = RuleChain::new();
        assert!(chain.is_empty());
        assert_eq!(chain.len(), 0);
    }
}
