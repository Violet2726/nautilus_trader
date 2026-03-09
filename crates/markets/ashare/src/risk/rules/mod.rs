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

//! A股风险规则
//!
//! 包含所有A股特定的风险管理规则。

pub mod lot_size;
pub mod price_cage;
pub mod price_limit;
pub mod session;
pub mod t1;
pub mod throttler;
pub mod instrument_status;
pub mod price_tick;
pub mod price_band;
pub mod cancel_session;

// 重新导出
pub use lot_size::*;
pub use price_cage::*;
pub use price_limit::*;
pub use session::*;
pub use t1::*;
pub use throttler::*;
pub use instrument_status::*;
pub use price_tick::*;
pub use price_band::*;
pub use cancel_session::*;
