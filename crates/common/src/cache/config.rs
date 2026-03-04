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

use serde::{Deserialize, Serialize};

use crate::{enums::SerializationEncoding, msgbus::database::DatabaseConfig};

/// `Cache` 实例配置。
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.common", from_py_object)
)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// 缓存后端数据库配置。
    pub database: Option<DatabaseConfig>,
    /// 数据库操作的编码方式，控制所使用的序列化器类型。
    pub encoding: SerializationEncoding,
    /// 是否应将时间戳持久化为 ISO 8601 字符串。
    pub timestamps_as_iso8601: bool,
    /// 流水线/批量事务之间的缓冲间隔 (毫秒)。
    pub buffer_interval_ms: Option<usize>,
    /// 批量读取操作 (例如 MGET) 的批大小。
    /// 如果已设置，批量读取将按此大小切分为块。
    pub bulk_read_batch_size: Option<usize>,
    /// 是否在键中使用 'trader-' 前缀。
    pub use_trader_prefix: bool,
    /// 是否在键中使用交易员实例 ID。
    pub use_instance_id: bool,
    /// 是否在启动时刷新数据库。
    pub flush_on_start: bool,
    /// 在重置时是否应从缓存内存中丢弃工具数据。
    pub drop_instruments_on_reset: bool,
    /// 内部 Tick 队列的最大长度。
    pub tick_capacity: usize,
    /// 内部 Bar 队列的最大长度。
    pub bar_capacity: usize,
    /// 是否应将市场数据持久化到磁盘。
    pub save_market_data: bool,
}

impl Default for CacheConfig {
    /// 创建一个新的默认 [`CacheConfig`] 实例。
    fn default() -> Self {
        Self {
            database: None,
            encoding: SerializationEncoding::MsgPack,
            timestamps_as_iso8601: false,
            buffer_interval_ms: None,
            bulk_read_batch_size: None,
            use_trader_prefix: true,
            use_instance_id: false,
            flush_on_start: false,
            drop_instruments_on_reset: true,
            tick_capacity: 10_000,
            bar_capacity: 10_000,
            save_market_data: false,
        }
    }
}

impl CacheConfig {
    /// 创建一个新的 [`CacheConfig`] 实例。
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        database: Option<DatabaseConfig>,
        encoding: SerializationEncoding,
        timestamps_as_iso8601: bool,
        buffer_interval_ms: Option<usize>,
        bulk_read_batch_size: Option<usize>,
        use_trader_prefix: bool,
        use_instance_id: bool,
        flush_on_start: bool,
        drop_instruments_on_reset: bool,
        tick_capacity: usize,
        bar_capacity: usize,
        save_market_data: bool,
    ) -> Self {
        Self {
            database,
            encoding,
            timestamps_as_iso8601,
            buffer_interval_ms,
            bulk_read_batch_size,
            use_trader_prefix,
            use_instance_id,
            flush_on_start,
            drop_instruments_on_reset,
            tick_capacity,
            bar_capacity,
            save_market_data,
        }
    }
}
