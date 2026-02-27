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

//! 通用消息类型。
//!
//! [`Params`] 类型使用 `IndexMap<String, Value>`，以确一致的顺序并支持 JSON 值。

// 从集中的 params 模块重新导出 Params
pub use crate::params::Params;
use crate::{UUID4, UnixNanos};

/// 代表系统中不同类型的消息。
#[derive(Debug, Clone)]
pub enum Message {
    /// 带有标识符和初始化时间戳的命令 (Command) 消息。
    Command {
        /// 此命令的唯一标识符。
        id: UUID4,
        /// 初始化时间戳。
        ts_init: UnixNanos,
    },
    /// 带有标识符和初始化时间戳的文档 (Document) 消息。
    Document {
        /// 此文档的唯一标识符。
        id: UUID4,
        /// 初始化时间戳。
        ts_init: UnixNanos,
    },
    /// 带有标识符和时间戳的事件 (Event) 消息。
    Event {
        /// 此事件的唯一标识符。
        id: UUID4,
        /// 初始化时间戳。
        ts_init: UnixNanos,
        /// 事件发生时间戳。
        ts_event: UnixNanos,
    },
    /// 带有标识符和初始化时间戳的请求 (Request) 消息。
    Request {
        /// 此请求的唯一标识符。
        id: UUID4,
        /// 初始化时间戳。
        ts_init: UnixNanos,
    },
    /// 带有标识符、时间戳和关联信息的响应 (Response) 消息。
    Response {
        /// 此响应的唯一标识符。
        id: UUID4,
        /// 初始化时间戳。
        ts_init: UnixNanos,
        /// 将此响应与请求链接起来的关联标识符 (correlation identifier)。
        correlation_id: UUID4,
    },
}
