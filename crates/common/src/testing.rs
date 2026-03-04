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

//! 常用的测试相关辅助函数。

#[cfg(feature = "live")]
use std::future::Future;
use std::{
    thread,
    time::{Duration, Instant},
};

use nautilus_core::UUID4;
use nautilus_model::{identifiers::TraderId, stubs::TestDefault};

use crate::logging::{
    init_logging,
    logger::{LogGuard, LoggerConfig},
    writer::FileWriterConfig,
};

/// # 错误
///
/// 如果初始化日志记录器失败，则返回错误。
pub fn init_logger_for_testing(stdout_level: Option<log::LevelFilter>) -> anyhow::Result<LogGuard> {
    let config = LoggerConfig {
        stdout_level: stdout_level.unwrap_or(log::LevelFilter::Trace),
        ..Default::default()
    };
    init_logging(
        TraderId::test_default(),
        UUID4::new(),
        config,
        FileWriterConfig::default(),
    )
}

/// 带有延迟地重复评估某个条件，直到该条件变为 true 或发生超时。
///
/// # Panics
///
/// 如果在未满足条件的情况下超过了超时持续时间，此函数将抛出 panic。
///
/// # 示例
///
/// ```
/// use std::time::Duration;
/// use std::thread;
/// use nautilus_common::testing::wait_until;
///
/// let start_time = std::time::Instant::now();
/// let timeout = Duration::from_secs(5);
///
/// wait_until(|| {
///     if start_time.elapsed().as_secs() > 2 {
///         true
///     } else {
///         false
///     }
/// }, timeout);
/// ```
///
/// 在上面的示例中，`wait_until` 函数将阻塞至少 2 秒，因为这是满足条件所需的时间。
/// 如果在 5 秒内未满足条件，它将引发 panic。
pub fn wait_until<F>(mut condition: F, timeout: Duration)
where
    F: FnMut() -> bool,
{
    let start_time = Instant::now();

    loop {
        if condition() {
            break;
        }

        assert!(
            start_time.elapsed() <= timeout,
            "等待条件超时"
        );

        thread::sleep(Duration::from_millis(100));
    }
}

/// # Panics
///
/// 如果在未满足条件的情况下超过了超时持续时间，此函数将抛出 panic。
#[cfg(feature = "live")]
pub async fn wait_until_async<F, Fut>(mut condition: F, timeout: Duration)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let start_time = Instant::now();

    loop {
        if condition().await {
            break;
        }

        assert!(
            start_time.elapsed() <= timeout,
            "等待条件超时"
        );

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
