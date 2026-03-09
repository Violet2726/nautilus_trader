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

use nautilus_common::messages::execution::TradingCommand;
use nautilus_model::identifiers::{AccountId, InstrumentId};
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq)]
pub enum CommandRuleCheckResult {
    Pass,
    Fail { reason: String },
}

impl CommandRuleCheckResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

impl std::fmt::Display for CommandRuleCheckResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail { reason } => write!(f, "FAIL: {reason}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommandContext {
    pub command: TradingCommand,
    pub instrument_id: Option<InstrumentId>,
    pub account_id: Option<AccountId>,
    pub timestamp_ns: u64,
}

pub trait CommandRule: Debug + Send + Sync {
    fn name(&self) -> &str;

    fn is_enabled(&self) -> bool {
        true
    }

    fn check(&self, context: &CommandContext) -> CommandRuleCheckResult;

    fn reset(&mut self) {}
}
