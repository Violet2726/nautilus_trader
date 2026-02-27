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

//! 核心常量。

use std::env;

/// NautilusTrader 字符串常量。
pub static NAUTILUS_TRADER: &str = "NautilusTrader";

/// 编译时从顶级 `pyproject.toml` 读取的 NautilusTrader 版本字符串。
pub static NAUTILUS_VERSION: &str = env!("NAUTILUS_VERSION");

/// NautilusTrader 通用 User-Agent 字符串，包含编译时的当前版本。
pub static NAUTILUS_USER_AGENT: &str = env!("NAUTILUS_USER_AGENT");

/// 主日志子系统之外的日志消息前缀。
pub static NAUTILUS_PREFIX: &str = "[NAUTILUS]";
