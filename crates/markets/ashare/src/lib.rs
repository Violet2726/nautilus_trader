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

//! NautilusTrader A股市场支持模块
//!
//! 本模块提供A股市场的特定功能，包括：
//! - 交易时段管理
//! - 涨跌停价格计算
//! - 风险管理规则
//! - T+1持仓管理
//!
//! # 示例
//!
//! ```rust
//! use nautilus_markets_ashare::{AShareSessionProvider, AShareRuleConfig};
//! use std::sync::Arc;
//!
//! // 创建A股交易时段提供者
//! let session_provider = Arc::new(AShareSessionProvider::new(None));
//!
//! // 创建A股规则配置
//! let config = AShareRuleConfig::new()
//!     .with_session(true, Some(session_provider))
//!     .with_price_limit(true);
//! ```

pub mod market;
pub mod risk;
pub mod portfolio;

// 重新导出常用类型
pub use market::{AShareSessionProvider, TradingPhase, PriceLimits};
pub use market::compute_ashare_price_limits;
pub use market::compute_ashare_price_limits_by_board_status;
pub use risk::{AShareRuleConfig, MarketDataProvider, create_ashare_rule_chain, create_ashare_rule_chain_with_provider, create_throttler_rule};
pub use risk::{CancelSessionRule, create_ashare_command_rule_chain, create_ashare_command_rule_chain_with_provider};
pub use portfolio::T1Ledger;
