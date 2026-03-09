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

use crate::market::session::SessionProvider;
use nautilus_common::messages::execution::TradingCommand;
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;
use nautilus_rules::command::{CommandContext, CommandRule, CommandRuleCheckResult};
use std::sync::Arc;

#[derive(Clone)]
pub struct CancelSessionRule {
    session_provider: Arc<dyn SessionProvider>,
    enabled: bool,
}

impl std::fmt::Debug for CancelSessionRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CancelSessionRule")
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl CancelSessionRule {
    pub fn new(session_provider: Arc<dyn SessionProvider>) -> Self {
        Self {
            session_provider,
            enabled: true,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

impl CommandRule for CancelSessionRule {
    fn name(&self) -> &str {
        "CancelSessionRule"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn check(&self, context: &CommandContext) -> CommandRuleCheckResult {
        if !self.is_enabled() {
            return CommandRuleCheckResult::Pass;
        }

        match &context.command {
            TradingCommand::CancelOrder(_) | TradingCommand::CancelAllOrders(_) => {}
            _ => return CommandRuleCheckResult::Pass,
        }

        let instrument_id = match context.instrument_id {
            Some(id) => id,
            None => return CommandRuleCheckResult::Pass,
        };

        let venue = Venue::from(instrument_id.venue.as_str());
        let ts = UnixNanos::from(context.timestamp_ns);
        let phase = self.session_provider.phase_at(&venue, ts);

        if !phase.can_cancel_order() {
            return CommandRuleCheckResult::Fail {
                reason: format!("CANCEL_DENIED: phase={phase:?}"),
            };
        }

        CommandRuleCheckResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::session::TradingPhase;
    use nautilus_common::messages::execution::CancelOrder;
    use nautilus_model::identifiers::{ClientOrderId, StrategyId, TraderId};
    use nautilus_model::identifiers::{InstrumentId};

    struct MockSessionProvider {
        phase: TradingPhase,
    }

    impl SessionProvider for MockSessionProvider {
        fn phase_at(&self, _venue: &Venue, _ts_ns: UnixNanos) -> TradingPhase {
            self.phase
        }
    }

    #[test]
    fn test_cancel_session_rule_denies_when_locked() {
        let provider = Arc::new(MockSessionProvider {
            phase: TradingPhase::PreAuctionLocked,
        });
        let rule = CancelSessionRule::new(provider);

        let cancel = CancelOrder::new(
            TraderId::from("TRADER-001"),
            None,
            StrategyId::from("STRATEGY-001"),
            InstrumentId::from("600000.SH"),
            ClientOrderId::from("O-1"),
            None,
            nautilus_core::UUID4::new(),
            0.into(),
            None,
        );

        let ctx = CommandContext {
            command: TradingCommand::CancelOrder(cancel),
            instrument_id: Some(InstrumentId::from("600000.SH")),
            account_id: None,
            timestamp_ns: 0,
        };

        let res = rule.check(&ctx);
        assert!(res.is_fail());
        assert!(res.to_string().contains("CANCEL_DENIED"));
    }
}
