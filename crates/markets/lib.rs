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

//! NautilusTrader 市场特定模块
//!
//! 提供各个市场的特定规则和功能。

pub mod ashare;
// pub mod us;     // 未来添加
// pub mod hk;     // 未来添加

// 重新导出常用类型
pub use ashare::{
    AShareSessionProvider, PriceLimits, T1Ledger, 
    AShareRuleConfig, create_ashare_rule_chain
};
