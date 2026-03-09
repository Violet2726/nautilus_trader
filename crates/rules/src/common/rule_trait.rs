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

/// 规则检查的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RuleCheckResult {
    /// 规则检查通过。
    Pass,
    /// 规则检查失败，附带原因。
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

/// 验证期间提供给规则的上下文。
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// 正在验证的订单。
    pub order: OrderAny,
    /// 标的 ID。
    pub instrument_id: InstrumentId,
    /// 账户 ID（如果可用）。
    pub account_id: Option<AccountId>,
    /// 当前时间戳（纳秒）。
    pub timestamp_ns: u64,
    /// 用于规则特定验证的附加元数据。
    pub metadata: RuleMetadata,
}

/// 用于规则特定验证的附加元数据。
#[derive(Debug, Clone, Default)]
pub struct RuleMetadata {
    /// 交易场所标识符。
    pub venue: Option<String>,
    /// 标的标识符。
    pub symbol: Option<String>,
    /// 订单方向。
    pub side: Option<String>,
    /// 订单数量。
    pub quantity: Option<f64>,
    /// 订单价格。
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

/// 用于验证交易操作的风险规则 trait。
///
/// 规则按顺序应用，在订单提交前进行验证。
/// 每个规则可以检查订单和市场条件的特定方面。
pub trait Rule: Debug + Send + Sync {
    /// 返回此规则的名称。
    fn name(&self) -> &str;

    /// 检查此规则是否已启用。
    fn is_enabled(&self) -> bool {
        true
    }

    /// 针对此规则验证订单。
    fn check(&self, context: &RuleContext) -> RuleCheckResult;

    /// 订单被接受时的可选回调。
    fn on_order_accepted(&mut self, _context: &RuleContext) {}

    /// 订单被拒绝时的可选回调。
    fn on_order_rejected(&mut self, _context: &RuleContext, _reason: &str) {}

    /// 将规则重置为初始状态。
    fn reset(&mut self) {}
}

/// 用于向下转换规则的辅助 trait。
pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any + Debug + Send + Sync> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
