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

use super::rule_trait::{CommandContext, CommandRule, CommandRuleCheckResult};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct CommandRuleChain {
    rules: Vec<Arc<dyn CommandRule>>,
}

impl CommandRuleChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&mut self, rule: Arc<dyn CommandRule>) {
        self.rules.push(rule);
    }

    pub fn add_rules(&mut self, rules: Vec<Arc<dyn CommandRule>>) {
        self.rules.extend(rules);
    }

    pub fn check(&self, context: &CommandContext) -> CommandRuleCheckResult {
        for rule in &self.rules {
            if !rule.is_enabled() {
                continue;
            }

            let result = rule.check(context);
            if !result.is_pass() {
                return result;
            }
        }

        CommandRuleCheckResult::Pass
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn CommandRule>> {
        self.rules.iter()
    }
}
