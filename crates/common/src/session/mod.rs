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

pub mod ashare;
pub mod price_limits;
pub use ashare::TradingPhase;
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;

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
pub trait SessionProvider: Send + Sync {
    fn phase_at(&self, venue: &Venue, ts_ns: UnixNanos) -> TradingPhase;
}
