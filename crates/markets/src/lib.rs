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

//! NautilusTrader 市场适配总模块。
//!
//! 该 crate 作为统一入口，按市场划分并聚合各子模块的规则、会话、风控与持仓能力。

#[cfg(feature = "python")]
pub mod python;

pub mod ashare;

// 重新导出常用市场类型
pub use ashare::*;

// 预留：未来扩展其他市场模块
// #[cfg(feature = "us")]
// pub mod us;
// #[cfg(feature = "hk")]
// pub mod hk;
