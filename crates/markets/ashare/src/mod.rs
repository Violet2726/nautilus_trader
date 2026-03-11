// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
// https://nautechsystems.io
//
// Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
// -------------------------------------------------------------------------------------------------

//! A 股市场子模块根入口。
//!
//! 按职责拆分为：
//! - `market`：交易时段与涨跌停等市场基础能力
//! - `risk`：规则链、校验函数与市场数据提供抽象
//! - `portfolio`：T+1 账本等持仓侧能力

pub mod market;
pub mod portfolio;
pub mod risk;
