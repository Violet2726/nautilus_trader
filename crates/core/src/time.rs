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

//! 核心 `AtomicTime`，用于实时时钟和静态时钟。
//!
//! 此模块提供了一种原子时间抽象，支持实时和静态时钟。它确保了线程平衡操作以及具有纳秒级精度的单调时间检索。
//!
//! # 模式
//!
//! - **实时模式 (Real-time mode):** 时钟会持续与系统挂钟时间（通过 [`SystemTime::now()`]）同步。
//!   为了确保跨多个线程的严格单调递增，内部更新使用了原子比较并交换 (compare-and-exchange) 循环 (`time_since_epoch`)。
//!   虽然这能保证每个新生成的时间戳都至少比上一个大一纳秒，但如果许多线程大量调用它，可能会引入较高的资源竞争。
//!
//! - **静态模式 (Static mode):** 时钟通过 [`AtomicTime::set_time`] 或 [`AtomicTime::increment_time`] 进行手动控制，
//!   这在模拟或回测中非常有用。你可以在运行时使用 [`AtomicTime::make_realtime`] 或 [`AtomicTime::make_static`] 切换模式。
//!   在**静态模式**下，我们使用了 acquire/release 语义，以便一个线程的更新可以被另一个线程观察到；
//!   但是，我们不对手动更新强制执行严格的全局顺序。如果你在**静态模式**下需要强大的多线程排序，必须自行协调更高级别的同步。

use std::{
    ops::Deref,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    UnixNanos,
    datetime::{NANOSECONDS_IN_MICROSECOND, NANOSECONDS_IN_MILLISECOND, NANOSECONDS_IN_SECOND},
};

/// 供系统全局使用的**实时模式**全局原子时间。
///
/// 此时钟运行在**实时模式**下，与系统时钟保持同步。它跨线程提供全局唯一的、严格递增的时间戳。
pub static ATOMIC_CLOCK_REALTIME: OnceLock<AtomicTime> = OnceLock::new();

/// 供系统全局使用的**静态模式**全局原子时间。
///
/// 此时钟运行在**静态模式**下，时间值可以手动设置或递增。适用于回测或模拟时间控制。
pub static ATOMIC_CLOCK_STATIC: OnceLock<AtomicTime> = OnceLock::new();

/// 返回全局**实时模式**原子时钟的静态引用。
///
/// 该时钟在底层使用 [`AtomicTime::time_since_epoch`]，确保跨线程生成严格递增的时间戳。
pub fn get_atomic_clock_realtime() -> &'static AtomicTime {
    ATOMIC_CLOCK_REALTIME.get_or_init(AtomicTime::default)
}

/// 返回全局**静态模式**原子时钟的静态引用。
///
/// 改时钟允许通过 [`AtomicTime::set_time`] 或 [`AtomicTime::increment_time`] 进行手动时间控制，
/// 且不会自动与系统时间同步。
pub fn get_atomic_clock_static() -> &'static AtomicTime {
    ATOMIC_CLOCK_STATIC.get_or_init(|| AtomicTime::new(false, UnixNanos::default()))
}

/// 基于 [`SystemTime::now()`] 返回自 UNIX 纪元以来的时长 (Duration)。
///
/// # Panics
///
/// 如果系统时间被设置为早于 UNIX 纪元，则触发 panic。
#[inline(always)]
#[must_use]
pub fn duration_since_unix_epoch() -> Duration {
    // 安全性：此处使用 expect() 是可以接受的，因为：
    // - SystemTime 失败代表了灾难性的系统时钟问题
    // - 这将影响整个应用程序的运行能力
    // - 替代性的错误处理会使所有依赖时间的路径变得复杂
    // - 此类失败在实际中极罕见，通常意味着硬件或操作系统出现了严重问题
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("调用 `SystemTime` 时出错")
}

/// 基于 [`SystemTime::now()`] 返回以纳秒为单位的当前 UNIX 时间。
///
/// # Panics
///
/// 如果以纳秒为单位的时长超过了 `u64::MAX`，则触发 panic。
#[inline(always)]
#[must_use]
pub fn nanos_since_unix_epoch() -> u64 {
    let ns = duration_since_unix_epoch().as_nanos();
    assert!(
        ns <= u128::from(u64::MAX),
        "系统时间溢出：数值超过了 u64::MAX 纳秒"
    );
    ns as u64
}

/// 表示一个原子计时结构体。
///
/// [`AtomicTime`] 根据其模式可以作为实时时钟或静态时钟运行。
/// 它使用 [`AtomicU64`] 仅通过不可变引用即可原子地更新其值。
///
/// `realtime` 标志指明了时钟当前处于哪种模式。
/// 为了保证并发性，此结构体使用了具有适当内存顺序的原子操作：
/// - 在**静态模式**下读取/写入时使用 **Acquire/Release**。
/// - 在**实时模式**下使用 **比较并交换 (compare-and-exchange, AcqRel)** 以保证单调递增。
#[repr(C)]
#[derive(Debug)]
pub struct AtomicTime {
    /// 指明时钟是运行在**实时模式**(`true`) 还是**静态模式**(`false`)。
    pub realtime: AtomicBool,
    /// 最近记录的时间（以 UNIX 纳秒为单位）。在**实时模式**下通过比较并交换进行原子更新，
    /// 或者在**静态模式**下通过简单的存储/获取进行更新。
    pub timestamp_ns: AtomicU64,
}

impl Deref for AtomicTime {
    type Target = AtomicU64;

    fn deref(&self) -> &Self::Target {
        &self.timestamp_ns
    }
}

impl Default for AtomicTime {
    /// 创建一个新的默认 [`AtomicTime`] 实例，处于**实时模式**，从当前系统时间开始。
    fn default() -> Self {
        Self::new(true, UnixNanos::default())
    }
}

impl AtomicTime {
    /// 创建一个新的 [`AtomicTime`] 实例。
    ///
    /// - 如果 `realtime` 为 `true`，提供的 `time` 仅作为初始占位符，
    ///   且会很快被对 [`AtomicTime::time_since_epoch`] 的调用所覆盖。
    /// - 如果 `realtime` 为 `false`，此时钟从**静态模式**开始运行，并以给定的 `time`
    ///   作为其当前值。
    #[must_use]
    pub fn new(realtime: bool, time: UnixNanos) -> Self {
        Self {
            realtime: AtomicBool::new(realtime),
            timestamp_ns: AtomicU64::new(time.into()),
        }
    }

    /// 根据时钟模式返回当前时间（纳秒）。
    ///
    /// - 在**实时模式**下，调用 [`AtomicTime::time_since_epoch`]，通过底层原子的 `AcqRel` 语义，
    ///   确保跨线程生成严格递增的时间戳。
    /// - 在**静态模式**下，使用 [`Ordering::Acquire`] 读取存储的时间。其他线程通过
    ///   [`AtomicTime::set_time`] 或 [`AtomicTime::increment_time`] (Release/AcqRel) 所做的更新在此处可见。
    #[must_use]
    pub fn get_time_ns(&self) -> UnixNanos {
        if self.realtime.load(Ordering::Acquire) {
            self.time_since_epoch()
        } else {
            UnixNanos::from(self.timestamp_ns.load(Ordering::Acquire))
        }
    }

    /// 以微秒为单位返回当前时间。
    #[must_use]
    pub fn get_time_us(&self) -> u64 {
        self.get_time_ns().as_u64() / NANOSECONDS_IN_MICROSECOND
    }

    /// 以毫秒为单位返回当前时间。
    #[must_use]
    pub fn get_time_ms(&self) -> u64 {
        self.get_time_ns().as_u64() / NANOSECONDS_IN_MILLISECOND
    }

    /// 以秒为单位返回当前时间。
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        reason = "时间转换时精度的损失是可以接受的"
    )]
    pub fn get_time(&self) -> f64 {
        self.get_time_ns().as_f64() / (NANOSECONDS_IN_SECOND as f64)
    }

    /// 手动为时钟设置新时间（仅在**静态模式**下有效）。
    ///
    /// 此处使用了带有 [`Ordering::Release`] 的原子存储，因此任何通过 [`Ordering::Acquire`]
    /// 进行读取的线程都会看到更新后的时间。这并不强制所有线程之间的总序 (total ordering)，
    /// 但足以确保：一旦某个线程看到了此更新，它也会看到写入线程中在此次调用之前所做的所有写入操作。
    ///
    /// 通常用于单线程场景，或**静态模式**下的协同并发，因为跨线程之间没有全局顺序。
    ///
    /// # Panics
    ///
    /// 如果在实时模式下调用，则触发 panic。
    ///
    /// # 线程安全性
    ///
    /// 模式检查与随后的存储操作并非原子完成。如果另一个线程在检查和存储之间调用了
    /// `make_realtime()`，则可能会违反不变性。这是有意为之的：模式切换属于设置阶段的操作，
    /// 不应与计时操作并发发生。调用方必须在恢复计时操作前确保模式切换已完成。
    pub fn set_time(&self, time: UnixNanos) {
        assert!(
            !self.realtime.load(Ordering::SeqCst),
            "当处于实时模式时无法设置时间"
        );

        self.store(time.into(), Ordering::Release);

        debug_assert!(
            !self.realtime.load(Ordering::SeqCst),
            "违反了不变性：在 set_time 期间模式被切换到了实时模式"
        );
    }

    /// 将当前的（静态模式）时间递增 `delta` 纳秒，并返回更新后的值。
    ///
    /// 内部使用了带有 [`Ordering::AcqRel`] 的 [`AtomicU64::fetch_update`]，以确保该递增
    /// 操作是原子的，且对使用 `Acquire` 加载的读取者可见。
    ///
    /// # Errors
    ///
    /// 如果递增会导致 `u64::MAX` 溢出，或在实时模式下调用，则返回错误。
    ///
    /// # 线程安全性
    ///
    /// 模式检查与随后的更新操作并非原子完成。如果另一个线程在检查和更新之间调用了
    /// `make_realtime()`，则可能会违反不变性。这是有意为之的：模式切换属于设置阶段的操作，
    /// 不应与计时操作并发发生。调用方必须在恢复计时操作前确保模式切换已完成。
    pub fn increment_time(&self, delta: u64) -> anyhow::Result<UnixNanos> {
        anyhow::ensure!(
            !self.realtime.load(Ordering::SeqCst),
            "当处于实时模式时无法递增时间"
        );

        let previous =
            match self
                .timestamp_ns
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_add(delta)
                }) {
                Ok(prev) => prev,
                Err(_) => anyhow::bail!("无法递增超过 u64::MAX 的时间"),
            };

        debug_assert!(
            !self.realtime.load(Ordering::SeqCst),
            "违反了不变性：在 increment_time 期间模式被切换到了实时模式"
        );

        Ok(UnixNanos::from(previous + delta))
    }

    /// 检索并更新当前的“实时”时钟，返回基于系统时间的、严格递增的时间戳。
    ///
    /// 内部逻辑：
    /// - 从 [`SystemTime::now()`] 获取 `now`。
    /// - 进行原子的“比较并交换”（使用 [`Ordering::AcqRel`]）以确保存储的时间戳
    ///   永远不会小于上一个时间戳。
    ///
    /// 这确保了：
    /// 1. **单调递增**: 返回的时间戳严格大于前一个时间戳（至少多 1 纳秒）。
    /// 2. **不会回跳**: 如果 OS 时间向后跳动，我们将忽略此跳动以维持单调性。
    /// 3. **可见性**: 在多线程环境下，一旦此次“比较并交换”完成，其他线程即可看到更新后的值。
    ///
    /// # Panics
    ///
    /// 如果内部计数器达到了 `u64::MAX`（这意味着该进程已经运行了超出其表示能力的范围（约 584 年），
    /// 或者时钟被手动破坏了），则触发 panic。
    pub fn time_since_epoch(&self) -> UnixNanos {
        // 此方法保证了严格的一致性，但在高度竞争下，可能会因为 `compare_exchange` 循环中的重试产生性能开销。
        let now = nanos_since_unix_epoch();
        loop {
            // 使用 Acquire 以观察最近存储的值
            let last = self.load(Ordering::Acquire);
            // 确保不会绕回超过 u64::MAX —— 将其视为致命错误
            let incremented = last
                .checked_add(1)
                .expect("AtomicTime 溢出：达到了 u64::MAX");
            let next = now.max(incremented);
            // 成功时的 AcqRel 保证了新值被发布，
            // 失败时的 Acquire 则在 CAS 竞争失败时重新加载。
            //
            // 请注意，在高度竞争的情况下（许多线程在紧凑的循环中调用此函数），
            // CAS 循环可能会增加延迟。
            //
            // 然而在实际中，循环会很快终止，因为：
            // - 系统时间会在两次迭代之间自然增长
            // - 每次迭代会将时间递增至少 1ns，从而防止了 ABA 问题
            // - 在正常的使用模式下，极少出现需要重试的真实竞争
            //
            // 并发压力测试（4 线程 × 10 万次迭代）验证了这种方法的有效性。
            if self
                .compare_exchange(last, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return UnixNanos::from(next);
            }
        }
    }

    /// 将时钟切换到**实时模式** (`realtime = true`)。
    ///
    /// 对于模式存储使用 [`Ordering::SeqCst`]，这确保了当其他线程同样对 `realtime`
    /// 进行 `SeqCst` 加载/存储时，模式切换具有全局顺序。
    /// 通常而言，切换模式的操作极少发生，因此此处 `SeqCst` 对性能的影响是可以接受的。
    pub fn make_realtime(&self) {
        self.realtime.store(true, Ordering::SeqCst);
    }

    /// 将时钟切换到**静态模式** (`realtime = false`)。
    ///
    /// 对于模式存储使用 [`Ordering::SeqCst`]，这确保了当其他线程同样对 `realtime`
    /// 进行 `SeqCst` 加载/存储时，模式切换具有全局顺序。
    pub fn make_static(&self) {
        self.realtime.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rstest::*;

    use super::*;

    #[rstest]
    fn test_global_clocks_initialization() {
        let realtime_clock = get_atomic_clock_realtime();
        assert!(realtime_clock.get_time_ns().as_u64() > 0);

        let static_clock = get_atomic_clock_static();
        static_clock.set_time(UnixNanos::from(500_000_000)); // 500 毫秒
        assert_eq!(static_clock.get_time_ns().as_u64(), 500_000_000);
    }

    #[rstest]
    fn test_mode_switching() {
        let time = AtomicTime::new(true, UnixNanos::default());

        // 验证实时模式
        let realtime_ns = time.get_time_ns();
        assert!(realtime_ns.as_u64() > 0);

        // 切换到静态模式
        time.make_static();
        time.set_time(UnixNanos::from(1_000_000_000)); // 1 秒
        let static_ns = time.get_time_ns();
        assert_eq!(static_ns.as_u64(), 1_000_000_000);

        // 切换回实时模式
        time.make_realtime();
        let new_realtime_ns = time.get_time_ns();
        assert!(new_realtime_ns.as_u64() > static_ns.as_u64());
    }

    #[rstest]
    #[should_panic(expected = "Cannot set time while clock is in realtime mode")]
    fn test_set_time_panics_in_realtime_mode() {
        let clock = AtomicTime::new(true, UnixNanos::default());
        clock.set_time(UnixNanos::from(123));
    }

    #[rstest]
    fn test_increment_time_returns_error_in_realtime_mode() {
        let clock = AtomicTime::new(true, UnixNanos::default());
        let result = clock.increment_time(1);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot increment time while clock is in realtime mode")
        );
    }

    #[rstest]
    #[should_panic(expected = "AtomicTime overflow")]
    fn test_time_since_epoch_overflow_panics() {
        use std::sync::atomic::{AtomicBool, AtomicU64};

        // 手动构造一个计数器已处于 u64::MAX 的时钟
        let clock = AtomicTime {
            realtime: AtomicBool::new(true),
            timestamp_ns: AtomicU64::new(u64::MAX),
        };

        // 此调用将尝试加 1 并且必须触发 panic
        let _ = clock.time_since_epoch();
    }

    #[rstest]
    fn test_mode_switching_concurrent() {
        let clock = Arc::new(AtomicTime::new(true, UnixNanos::default()));
        let num_threads = 4;
        let iterations = 10000;
        let mut handles = Vec::with_capacity(num_threads);

        for _ in 0..num_threads {
            let clock_clone = Arc::clone(&clock);
            let handle = std::thread::spawn(move || {
                for i in 0..iterations {
                    if i % 2 == 0 {
                        clock_clone.make_static();
                    } else {
                        clock_clone.make_realtime();
                    }
                    // 检索时间；我们在此并不断言具体数值，
                    // 但至少是在并发情况下对模式切换逻辑进行练习。
                    let _ = clock_clone.get_time_ns();
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[rstest]
    fn test_static_time_is_stable() {
        // 创建一个带有初始值的静态模式时钟
        let clock = AtomicTime::new(false, UnixNanos::from(42));
        let time1 = clock.get_time_ns();

        // 稍微休眠，让系统时间有机会发生变化（如果时钟是在实时运行的话）
        std::thread::sleep(std::time::Duration::from_millis(10));
        let time2 = clock.get_time_ns();

        // 在静态模式下，其数值应当保持不变
        assert_eq!(time1, time2);
    }

    #[rstest]
    fn test_increment_time() {
        // 从静态模式开始
        let time = AtomicTime::new(false, UnixNanos::from(0));

        let updated_time = time.increment_time(500).unwrap();
        assert_eq!(updated_time.as_u64(), 500);

        let updated_time = time.increment_time(1_000).unwrap();
        assert_eq!(updated_time.as_u64(), 1_500);
    }

    #[rstest]
    fn test_increment_time_overflow_errors() {
        let time = AtomicTime::new(false, UnixNanos::from(u64::MAX - 5));

        let err = time.increment_time(10).unwrap_err();
        assert_eq!(err.to_string(), "无法递增超过 u64::MAX 的时间");
    }

    #[rstest]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "针对 Python 互操作的有意的类型转换"
    )]
    fn test_nanos_since_unix_epoch_vs_system_time() {
        let unix_nanos = nanos_since_unix_epoch();
        let system_ns = duration_since_unix_epoch().as_nanos() as u64;
        assert!((unix_nanos as i64 - system_ns as i64).abs() < NANOSECONDS_IN_SECOND as i64);
    }

    #[rstest]
    fn test_time_since_epoch_monotonicity() {
        let clock = get_atomic_clock_realtime();
        let mut previous = clock.time_since_epoch();
        for _ in 0..1_000_000 {
            let current = clock.time_since_epoch();
            assert!(current > previous);
            previous = current;
        }
    }

    #[rstest]
    fn test_time_since_epoch_strictly_increasing_concurrent() {
        let time = Arc::new(AtomicTime::new(true, UnixNanos::default()));
        let num_threads = 4;
        let iterations = 100_000;
        let mut handles = Vec::with_capacity(num_threads);

        for thread_id in 0..num_threads {
            let time_clone = Arc::clone(&time);

            let handle = std::thread::spawn(move || {
                let mut previous = time_clone.time_since_epoch().as_u64();

                for i in 0..iterations {
                    let current = time_clone.time_since_epoch().as_u64();
                    assert!(
                        current > previous,
                        "线程 {thread_id}: 第 {i} 次迭代: 时间未增加：previous={previous}, current={current}",
                    );
                    previous = current;
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[rstest]
    fn test_duration_since_unix_epoch() {
        let time = AtomicTime::new(true, UnixNanos::default());
        let duration = Duration::from_nanos(time.get_time_ns().into());
        let now = SystemTime::now();

        // 检查 duration 是否接近于当前时刻与 UNIX_EPOCH 之间的实际差值
        let delta = now
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .checked_sub(duration);
        assert!(delta.unwrap_or_default() < Duration::from_millis(100));

        // 检查 duration 是否大于某个特定值（假设测试在此时间点之后运行）
        assert!(duration > Duration::from_secs(1_650_000_000));
    }

    #[rstest]
    fn test_unix_timestamp_is_monotonic_increasing() {
        let time = AtomicTime::new(true, UnixNanos::default());
        let result1 = time.get_time();
        let result2 = time.get_time();
        let result3 = time.get_time();
        let result4 = time.get_time();
        let result5 = time.get_time();

        assert!(result2 >= result1);
        assert!(result3 >= result2);
        assert!(result4 >= result3);
        assert!(result5 >= result4);
        assert!(result1 > 1_650_000_000.0);
    }

    #[rstest]
    fn test_unix_timestamp_ms_is_monotonic_increasing() {
        let time = AtomicTime::new(true, UnixNanos::default());
        let result1 = time.get_time_ms();
        let result2 = time.get_time_ms();
        let result3 = time.get_time_ms();
        let result4 = time.get_time_ms();
        let result5 = time.get_time_ms();

        assert!(result2 >= result1);
        assert!(result3 >= result2);
        assert!(result4 >= result3);
        assert!(result5 >= result4);
        assert!(result1 >= 1_650_000_000_000);
    }

    #[rstest]
    fn test_unix_timestamp_us_is_monotonic_increasing() {
        let time = AtomicTime::new(true, UnixNanos::default());
        let result1 = time.get_time_us();
        let result2 = time.get_time_us();
        let result3 = time.get_time_us();
        let result4 = time.get_time_us();
        let result5 = time.get_time_us();

        assert!(result2 >= result1);
        assert!(result3 >= result2);
        assert!(result4 >= result3);
        assert!(result5 >= result4);
        assert!(result1 > 1_650_000_000_000_000);
    }

    #[rstest]
    fn test_unix_timestamp_ns_is_monotonic_increasing() {
        let time = AtomicTime::new(true, UnixNanos::default());
        let result1 = time.get_time_ns();
        let result2 = time.get_time_ns();
        let result3 = time.get_time_ns();
        let result4 = time.get_time_ns();
        let result5 = time.get_time_ns();

        assert!(result2 >= result1);
        assert!(result3 >= result2);
        assert!(result4 >= result3);
        assert!(result5 >= result4);
        assert!(result1.as_u64() > 1_650_000_000_000_000_000);
    }

    #[rstest]
    fn test_acquire_release_contract_static_mode() {
        // 此测试明确证明了 Acquire/Release 内存顺序合约：
        // - 写入线程使用 set_time() 进行 Release 存储（参见 AtomicTime::set_time）
        // - 读取线程使用 get_time_ns() 进行 Acquire 加载（参见 AtomicTime::get_time_ns）
        // - Release-Acquire 配对确保了 Release 之前的所有写入在 Acquire 之后均可见

        let clock = Arc::new(AtomicTime::new(false, UnixNanos::from(0)));
        let aux_data = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        // 写入线程：更新辅助数据，随后通过 set_time 进行 release
        let writer_clock = Arc::clone(&clock);
        let writer_aux = Arc::clone(&aux_data);
        let writer_done = Arc::clone(&done);

        let writer = std::thread::spawn(move || {
            for i in 1..=1_000u64 {
                writer_aux.store(i, Ordering::Relaxed);

                // 通过 set_time 进行的 Release 存储创建了 release 屏障 —— 所有之前的写入操作（包括 aux_data）
                // 必须对任何通过 Acquire 加载观察到此次时间值的线程可见
                writer_clock.set_time(UnixNanos::from(i * 1000));

                // 出让执行权以促使交替运行
                std::thread::yield_now();
            }
            writer_done.store(true, Ordering::Release);
        });

        // 读取线程：通过 get_time_ns 进行 acquire，随后检查辅助数据
        let reader_clock = Arc::clone(&clock);
        let reader_aux = Arc::clone(&aux_data);
        let reader_done = Arc::clone(&done);

        let reader = std::thread::spawn(move || {
            let mut last_time = 0u64;
            let mut max_aux_seen = 0u64;

            // 轮询直至写入线程结束，不限迭代次数
            while !reader_done.load(Ordering::Acquire) {
                let current_time = reader_clock.get_time_ns().as_u64();

                if current_time > last_time {
                    // get_time_ns 中的 Acquire 与 set_time 中的 Release 同步，
                    // 使得 aux_data 可见
                    let aux_value = reader_aux.load(Ordering::Relaxed);

                    // 不变性：aux_value 绝不应后退（证明了 Release-Acquire 的同步是有效的）
                    if aux_value > 0 {
                        assert!(
                            aux_value >= max_aux_seen,
                            "违反了 Acquire/Release 合约：aux 值从 {max_aux_seen} 后退到了 {aux_value}"
                        );
                        max_aux_seen = aux_value;
                    }

                    last_time = current_time;
                }

                std::thread::yield_now();
            }

            // 在写入器完成后检查最终状态，以确保我们观察到了更新
            let final_time = reader_clock.get_time_ns().as_u64();
            if final_time > last_time {
                let final_aux = reader_aux.load(Ordering::Relaxed);
                if final_aux > 0 {
                    assert!(
                        final_aux >= max_aux_seen,
                        "违反了 Acquire/Release 合约：最终 aux {final_aux} < max {max_aux_seen}"
                    );
                    max_aux_seen = final_aux;
                }
            }

            max_aux_seen
        });

        writer.join().unwrap();
        let max_observed = reader.join().unwrap();

        // 确保读取器确实观察到了更新（而非无效地满足条件）
        assert!(max_observed > 0, "读取器必须观察到写入器所做的更新");
    }

    #[rstest]
    fn test_acquire_release_contract_increment_time() {
        // 对于 increment_time 的类似测试，其使用了带有 AcqRel 的 fetch_update (参见 AtomicTime::increment_time)

        let clock = Arc::new(AtomicTime::new(false, UnixNanos::from(0)));
        let aux_data = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let writer_clock = Arc::clone(&clock);
        let writer_aux = Arc::clone(&aux_data);
        let writer_done = Arc::clone(&done);

        let writer = std::thread::spawn(move || {
            for i in 1..=1_000u64 {
                writer_aux.store(i, Ordering::Relaxed);
                let _ = writer_clock.increment_time(1000).unwrap();
                std::thread::yield_now();
            }
            writer_done.store(true, Ordering::Release);
        });

        let reader_clock = Arc::clone(&clock);
        let reader_aux = Arc::clone(&aux_data);
        let reader_done = Arc::clone(&done);

        let reader = std::thread::spawn(move || {
            let mut last_time = 0u64;
            let mut max_aux = 0u64;

            // 轮询直至写入线程结束，不限迭代次数
            while !reader_done.load(Ordering::Acquire) {
                let current_time = reader_clock.get_time_ns().as_u64();

                if current_time > last_time {
                    let aux_value = reader_aux.load(Ordering::Relaxed);

                    // 不变性：aux_value 绝不应回退（证明了 AcqRel 同步是有效的）
                    if aux_value > 0 {
                        assert!(
                            aux_value >= max_aux,
                            "违反了 AcqRel 合约：aux 值从 {max_aux} 回退到了 {aux_value}"
                        );
                        max_aux = aux_value;
                    }

                    last_time = current_time;
                }

                std::thread::yield_now();
            }

            // 在写入器完成后检查最终状态，以确保我们观察到了更新
            let final_time = reader_clock.get_time_ns().as_u64();
            if final_time > last_time {
                let final_aux = reader_aux.load(Ordering::Relaxed);
                if final_aux > 0 {
                    assert!(
                        final_aux >= max_aux,
                        "违反了 AcqRel 合约：最终 aux {final_aux} < max {max_aux}"
                    );
                    max_aux = final_aux;
                }
            }

            max_aux
        });

        writer.join().unwrap();
        let max_observed = reader.join().unwrap();

        // 确保读取器确实观察到了更新（而非无效地满足条件）
        assert!(max_observed > 0, "读取器必须观察到写入器所做的更新");
    }
}
