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

//! 交易时段管理基础设施。
//!
//! 本模块提供交易时段管理的核心 trait：
//!
//! - **SessionProvider**：用于查询特定时间的交易阶段
//!
//! 特定市场的实现在子模块中提供：
//!
//! - `ashare`：A 股市场交易时段提供者
//!
//! # 架构
//!
//! 交易时段系统采用分层架构：
//!
//! 1. **核心 Trait**（`common/session/mod.rs`）：
//!    - `SessionProvider`：用于时段查询的通用 trait
//!
//! 2. **特定市场实现**（`common/session/ashare.rs`）：
//!    - `TradingPhase`：A 股特定的阶段枚举
//!    - `AShareSessionProvider`：A 股特定的时段逻辑
//!
//! 3. **规则层**（`risk/rule/ashare/session_rule.rs`）：
//!    - `SessionRule`：使用 SessionProvider 验证订单时机

pub mod ashare;

// 为向后兼容重新导出 TradingPhase
pub use ashare::TradingPhase;

use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;

/// 用于查询交易时段阶段的 trait。
///
/// 此 trait 为确定给定场地和时间戳的当前交易阶段提供统一接口。
/// 不同市场可以实现自己的时段逻辑，同时保持一致的接口。
///
/// # 示例
///
/// ```no_run
/// use nautilus_common::session::{SessionProvider, AShareSessionProvider};
/// use nautilus_model::identifiers::Venue;
/// use nautilus_core::UnixNanos;
///
/// let provider = AShareSessionProvider::new(None);
/// let venue = Venue::from("XSHG");
/// let phase = provider.phase_at(&venue, UnixNanos::now());
///
/// if phase.can_accept_order() {
///     // 提交订单
/// }
/// ```
pub trait SessionProvider: Send + Sync {
    /// 查询给定场地和时间戳的交易阶段。
    ///
    /// # 参数
    ///
    /// * `venue` - 要查询的交易场地
    /// * `ts_ns` - 纳秒级时间戳
    ///
    /// # 返回值
    ///
    /// 指定场地和时间的当前交易阶段
    fn phase_at(&self, venue: &Venue, ts_ns: UnixNanos) -> TradingPhase;
}
