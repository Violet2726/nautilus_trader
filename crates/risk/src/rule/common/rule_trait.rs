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

use nautilus_model::{
    identifiers::{AccountId, InstrumentId},
    orders::{Order, OrderAny},
};
use std::any::Any;
use std::fmt::Debug;

/// Result of a rule check.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleCheckResult {
    /// The rule check passed.
    Pass,
    /// The rule check failed with a reason.
    Fail { reason: String },
}

impl RuleCheckResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

impl std::fmt::Display for RuleCheckResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pass => write!(f, "PASS"),
            Self::Fail { reason } => write!(f, "FAIL: {}", reason),
        }
    }
}

/// Context provided to rules during validation.
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// The order being validated.
    pub order: OrderAny,
    /// The instrument ID.
    pub instrument_id: InstrumentId,
    /// The account ID (if available).
    pub account_id: Option<AccountId>,
    /// Current timestamp in nanoseconds.
    pub timestamp_ns: u64,
    /// Additional metadata for rule-specific use.
    pub metadata: RuleMetadata,
}

/// Additional metadata for rule-specific validation.
#[derive(Debug, Clone, Default)]
pub struct RuleMetadata {
    /// Venue identifier.
    pub venue: Option<String>,
    /// Symbol identifier.
    pub symbol: Option<String>,
    /// Order side.
    pub side: Option<String>,
    /// Order quantity.
    pub quantity: Option<f64>,
    /// Order price.
    pub price: Option<f64>,
}

impl RuleContext {
    pub fn new(
        order: OrderAny,
        instrument_id: InstrumentId,
        account_id: Option<AccountId>,
        timestamp_ns: u64,
    ) -> Self {
        let venue = Some(instrument_id.venue.as_str().to_string());
        let symbol = Some(instrument_id.symbol.as_str().to_string());
        let side = Some(format!("{:?}", order.order_side()));
        let quantity = Some(order.quantity().as_f64());
        let price = order.price().map(|p| p.as_f64());

        Self {
            order,
            instrument_id,
            account_id,
            timestamp_ns,
            metadata: RuleMetadata {
                venue,
                symbol,
                side,
                quantity,
                price,
            },
        }
    }
}

/// Trait for risk rules that validate trading operations.
///
/// Rules are applied in sequence to validate orders before submission.
/// Each rule can check specific aspects of the order and market conditions.
pub trait Rule: Debug + Send + Sync {
    /// Returns the name of this rule.
    fn name(&self) -> &str;

    /// Checks if the rule is enabled.
    fn is_enabled(&self) -> bool {
        true
    }

    /// Validates the order against this rule.
    ///
    /// # Arguments
    ///
    /// * `context` - The validation context containing order and market information.
    ///
    /// # Returns
    ///
    /// Returns `RuleCheckResult::Pass` if the rule check succeeds,
    /// or `RuleCheckResult::Fail` with a reason if it fails.
    fn check(&self, context: &RuleContext) -> RuleCheckResult;

    /// Optional callback when an order is accepted.
    ///
    /// This can be used to update internal state after an order passes all checks.
    fn on_order_accepted(&mut self, _context: &RuleContext) {}

    /// Optional callback when an order is rejected.
    ///
    /// This can be used to update internal state after an order fails validation.
    fn on_order_rejected(&mut self, _context: &RuleContext, _reason: &str) {}

    /// Resets the rule to its initial state.
    fn reset(&mut self) {}
}

/// Helper trait for downcasting rules.
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Debug + Send + Sync> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
