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

//! 实时和静态 `Clock`（时钟）实现。

use std::{any::Any, collections::BTreeMap, fmt::Debug, ops::Deref, time::Duration};

use ahash::AHashMap;
use chrono::{DateTime, Utc};
use nautilus_core::{
    AtomicTime, UnixNanos,
    correctness::{check_positive_u64, check_predicate_true, check_valid_string_utf8},
    formatting::Separable,
};
use ustr::Ustr;

use crate::timer::{
    TestTimer, TimeEvent, TimeEventCallback, TimeEventHandler, create_valid_interval,
};

/// 代表一种类型的时钟。
///
/// # 注意
///
/// 活跃的定时器是指尚未过期的定时器 (`timer.is_expired == False`)。
pub trait Clock: Debug + Any {
    /// 以时区感知型 `DateTime<UTC>` 格式返回当前日期和时间。
    fn utc_now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_nanos(self.timestamp_ns().as_i64())
    }

    /// 以纳秒 (ns) 为单位返回当前 UNIX 时间戳。
    fn timestamp_ns(&self) -> UnixNanos;

    /// 以微秒 (μs) 为单位返回当前 UNIX 时间戳。
    fn timestamp_us(&self) -> u64;

    /// 以毫秒 (ms) 为单位返回当前 UNIX 时间戳。
    fn timestamp_ms(&self) -> u64;

    /// 以秒为单位返回当前 UNIX 时间戳。
    fn timestamp(&self) -> f64;

    /// 返回时钟内活跃定时器的名称。
    fn timer_names(&self) -> Vec<&str>;

    /// 返回时钟内活跃定时器的数量。
    fn timer_count(&self) -> usize;

    /// 检查是否存在名为 `name` 的定时器。
    fn timer_exists(&self, name: &Ustr) -> bool;

    /// 为时钟注册默认事件处理器。如果定时器没有关联处理器，则使用此处理器。
    fn register_default_handler(&mut self, callback: TimeEventCallback);

    /// 获取 [`TimeEvent`] 的处理器。
    ///
    /// 注意：如果事件没有关联的处理器，则会抛出 panic。
    fn get_handler(&self, event: TimeEvent) -> TimeEventHandler;

    /// 设置定时器在指定时间发出警报。
    ///
    /// 标志语义请参阅 [`Clock::set_time_alert_ns`]。
    ///
    /// # 回调函数 (Callback)
    ///
    /// - `callback`: 如果为 Some，则由该回调处理时间事件。
    /// - `callback`: 如果为 None，则使用时钟的默认时间事件回调。
    ///
    /// # 错误
    ///
    /// 如果 `name` 无效、`alert_time` 在过去但不允许、或任何断言检查失败，则返回错误。
    #[allow(clippy::too_many_arguments)]
    fn set_time_alert(
        &mut self,
        name: &str,
        alert_time: DateTime<Utc>,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
    ) -> anyhow::Result<()> {
        self.set_time_alert_ns(name, alert_time.into(), callback, allow_past)
    }

    /// 设置定时器在指定时间发出警报。
    ///
    /// 任何以同一 `name` 注册的现有定时器在安排新警报之前都会被取消并发出警告。
    ///
    /// # 标志 (Flags)
    ///
    /// | `allow_past` | 行为                                                                                |
    /// |--------------|-------------------------------------------------------------------------------------|
    /// | `true`       | 如果警报时间在**过去**，警报立即触发；否则在警报时间触发。                             |
    /// | `false`      | 如果警报时间早于当前时间，则返回错误。                                                |
    ///
    /// # 回调函数 (Callback)
    ///
    /// - `callback`: 如果为 Some，则由该回调处理时间事件。
    /// - `callback`: 如果为 None，则使用时钟的默认时间事件回调。
    ///
    /// # 错误
    ///
    /// 如果 `name` 无效、`alert_time_ns` 早于当前时间且不允许、或任何断言检查失败，则返回错误。
    #[allow(clippy::too_many_arguments)]
    fn set_time_alert_ns(
        &mut self,
        name: &str,
        alert_time_ns: UnixNanos,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
    ) -> anyhow::Result<()>;

    /// 设置定时器在开始时间和停止时间之间每隔一定时间间隔触发一次时间事件。
    ///
    /// 任何以同一 `name` 注册的现有定时器在安排新定时器之前都会被取消并发出警告。
    ///
    /// 标志语义请参阅 [`Clock::set_timer_ns`]。
    ///
    /// # 回调函数 (Callback)
    ///
    /// - `callback`: 如果为 Some，则由该回调处理时间事件。
    /// - `callback`: 如果为 None，则使用时钟的默认时间事件回调。
    ///
    /// # 错误
    ///
    /// 如果 `name` 无效、`interval` 不为正、或任何断言检查失败，则返回错误。
    #[allow(clippy::too_many_arguments)]
    fn set_timer(
        &mut self,
        name: &str,
        interval: Duration,
        start_time: Option<DateTime<Utc>>,
        stop_time: Option<DateTime<Utc>>,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
        fire_immediately: Option<bool>,
    ) -> anyhow::Result<()> {
        self.set_timer_ns(
            name,
            interval.as_nanos() as u64,
            start_time.map(UnixNanos::from),
            stop_time.map(UnixNanos::from),
            callback,
            allow_past,
            fire_immediately,
        )
    }

    /// 设置定时器在开始时间和停止时间之间每隔一定时间间隔触发一次时间事件。
    ///
    /// 任何以同一 `name` 注册的现有定时器在安排新定时器之前都会被取消。
    ///
    /// # 开始时间 (Start Time)
    ///
    /// - `None` 或 `Some(0)`: 使用当前时间作为开始时间。
    /// - `Some(non_zero)`: 使用指定的时间戳作为开始时间。
    ///
    /// # 标志 (Flags)
    ///
    /// | `allow_past` | `fire_immediately` | 行为                                                                              |
    /// |--------------|--------------------|---------------------------------------------------------------------------------------|
    /// | `true`       | `true`             | 第一个事件在开始时间立即触发，即使开始时间已过。                                         |
    /// | `true`       | `false`            | 第一个事件在开始时间 + 间隔处触发，即使开始时间已过。                                     |
    /// | `false`      | `true`             | 如果开始时间已过（第一个事件将立即但已过期），则返回错误。                               |
    /// | `false`      | `false`            | 如果开始时间 + 间隔已过，则返回错误。                                                  |
    ///
    /// # 回调函数 (Callback)
    ///
    /// - `callback`: 如果为 Some，则由该回调处理时间事件。
    /// - `callback`: 如果为 None，则使用时钟的默认时间事件回调。
    ///
    /// # 错误
    ///
    /// 如果 `name` 无效、`interval_ns` 不为正、或任何断言检查失败，则返回错误。
    #[allow(clippy::too_many_arguments)]
    fn set_timer_ns(
        &mut self,
        name: &str,
        interval_ns: u64,
        start_time_ns: Option<UnixNanos>,
        stop_time_ns: Option<UnixNanos>,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
        fire_immediately: Option<bool>,
    ) -> anyhow::Result<()>;

    /// 返回触发名为 `name` 的定时器的时间间隔。
    ///
    /// 如果定时器不存在，则返回 `None`。
    fn next_time_ns(&self, name: &str) -> Option<UnixNanos>;

    /// 取消名为 `name` 的定时器。
    fn cancel_timer(&mut self, name: &str);

    /// 取消所有定时器。
    fn cancel_timers(&mut self);

    /// 通过清除内部状态重置时钟。
    fn reset(&mut self);
}

impl dyn Clock {
    /// 返回此时钟作为 `Any` 的引用，用于向下转型 (downcasting)。
    pub fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    /// 返回此时钟作为 `Any` 的可变引用，用于向下转型 (downcasting)。
    pub fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// 定时器事件回调的注册表。
///
/// 提供 `TestClock` 和 `LiveClock` 共用的回调注册和检索逻辑。
#[derive(Debug, Default)]
pub struct CallbackRegistry {
    default_callback: Option<TimeEventCallback>,
    callbacks: AHashMap<Ustr, TimeEventCallback>,
}

impl CallbackRegistry {
    /// 创建一个新的 [`CallbackRegistry`] 实例。
    #[must_use]
    pub fn new() -> Self {
        Self {
            default_callback: None,
            callbacks: AHashMap::new(),
        }
    }

    /// 注册一个默认的处理器回调。
    pub fn register_default_handler(&mut self, callback: TimeEventCallback) {
        self.default_callback = Some(callback);
    }

    /// 为特定的定时器名称注册回调。
    pub fn register_callback(&mut self, name: Ustr, callback: TimeEventCallback) {
        self.callbacks.insert(name, callback);
    }

    /// 返回针对给定名称是否存在任何回调（特定或默认）。
    #[must_use]
    pub fn has_any_callback(&self, name: &Ustr) -> bool {
        self.callbacks.contains_key(name) || self.default_callback.is_some()
    }

    /// 获取特定定时器名称的回调，回退到默认回调。
    #[must_use]
    pub fn get_callback(&self, name: &Ustr) -> Option<TimeEventCallback> {
        self.callbacks
            .get(name)
            .cloned()
            .or_else(|| self.default_callback.clone())
    }

    /// 获取时间事件的处理器。
    ///
    /// # Panics
    ///
    /// 如果该事件名称不存在任何回调，则会抛出 panic。
    #[must_use]
    pub fn get_handler(&self, event: TimeEvent) -> TimeEventHandler {
        let callback = self
            .get_callback(&event.name)
            .unwrap_or_else(|| panic!("Event '{}' should have associated handler", event.name));

        TimeEventHandler::new(event, callback)
    }

    /// 清除所有已注册的回调。
    pub fn clear(&mut self) {
        self.callbacks.clear();
    }
}

/// 验证并准备设置时间警报的参数。
///
/// 处理名称验证、默认值解包以及过去时间戳调整。
///
/// # 错误
///
/// 如果名称无效，或者警报时间在过去且不允许，则返回错误。
pub fn validate_and_prepare_time_alert(
    name: &str,
    mut alert_time_ns: UnixNanos,
    allow_past: Option<bool>,
    ts_now: UnixNanos,
) -> anyhow::Result<(Ustr, UnixNanos)> {
    check_valid_string_utf8(name, stringify!(name))?;

    let name = Ustr::from(name);
    let allow_past = allow_past.unwrap_or(true);

    if alert_time_ns < ts_now {
        if allow_past {
            alert_time_ns = ts_now;
            log::warn!(
                "定时器 '{name}' 警报时间 {} 是过去的时间，已调整为当前时间以便立即触发",
                alert_time_ns.to_rfc3339(),
            );
        } else {
            anyhow::bail!(
                "定时器 '{name}' 警报时间 {} 是过去的时间（当前时间是 {ts_now}）",
                alert_time_ns.to_rfc3339(),
            );
        }
    }

    Ok((name, alert_time_ns))
}

/// 验证并准备设置定时器的参数。
///
/// 处理名称和间隔验证、默认值解包、开始时间归一化以及停止时间验证。
///
/// # 错误
///
/// 如果名称无效、间隔不为正、或停止时间验证失败，则返回错误。
pub fn validate_and_prepare_timer(
    name: &str,
    interval_ns: u64,
    start_time_ns: Option<UnixNanos>,
    stop_time_ns: Option<UnixNanos>,
    allow_past: Option<bool>,
    fire_immediately: Option<bool>,
    ts_now: UnixNanos,
) -> anyhow::Result<(Ustr, UnixNanos, Option<UnixNanos>, bool, bool)> {
    check_valid_string_utf8(name, stringify!(name))?;
    check_positive_u64(interval_ns, stringify!(interval_ns))?;

    let name = Ustr::from(name);
    let allow_past = allow_past.unwrap_or(true);
    let fire_immediately = fire_immediately.unwrap_or(false);

    let mut start_time_ns = start_time_ns.unwrap_or_default();

    if start_time_ns == 0 {
        // 零开始时间表示没有显式指定开始时间；我们使用当前时间
        start_time_ns = ts_now;
    } else if !allow_past {
        let next_event_time = if fire_immediately {
            start_time_ns
        } else {
            start_time_ns + interval_ns
        };

        if next_event_time < ts_now {
            anyhow::bail!(
                "定时器 '{name}' 的下一个事件时间 {} 在过去（当前时间是 {ts_now}）",
                next_event_time.to_rfc3339(),
            );
        }
    }

    if let Some(stop_time) = stop_time_ns {
        if stop_time <= start_time_ns {
            anyhow::bail!(
                "定时器 '{name}' 停止时间 {} 必须在开始时间 {} 之后",
                stop_time.to_rfc3339(),
                start_time_ns.to_rfc3339(),
            );
        }
        if !allow_past && stop_time <= ts_now {
            anyhow::bail!(
                "定时器 '{name}' 停止时间 {} 是过去的时间（当前时间是 {ts_now}）",
                stop_time.to_rfc3339(),
            );
        }
    }

    Ok((
        name,
        start_time_ns,
        stop_time_ns,
        allow_past,
        fire_immediately,
    ))
}

/// 一个静态测试时钟。
///
/// 在内部存储当前时间戳，并且可以推进该时间。
///
/// # 线程安全 (Threading)
///
/// 此时钟是线程相关的 (thread-affine)；仅在创建它的线程中使用。
#[derive(Debug)]
pub struct TestClock {
    time: AtomicTime,
    // Use btree map to ensure stable ordering when scanning for timers in `advance_time`
    timers: BTreeMap<Ustr, TestTimer>,
    callbacks: CallbackRegistry,
}

impl TestClock {
    /// 创建一个新的 [`TestClock`] 实例。
    #[must_use]
    pub fn new() -> Self {
        Self {
            time: AtomicTime::new(false, UnixNanos::default()),
            timers: BTreeMap::new(),
            callbacks: CallbackRegistry::new(),
        }
    }

    /// 返回时钟内部定时器的引用。
    #[must_use]
    pub const fn get_timers(&self) -> &BTreeMap<Ustr, TestTimer> {
        &self.timers
    }

    /// 将内部时钟推进到指定的 `to_time_ns`，并可选地将时钟设置为该时间。
    ///
    /// 此时钟确保时间以非递减方式运行。如果 `set_time` 为 `true`，
    /// 内部时钟将更新为 `to_time_ns` 的值。否则，时钟将推进但不显式设置时间。
    ///
    /// 该方法处理活动定时器，将它们推进到 `to_time_ns`，并收集由于该操作触发的所有 [`TimeEvent`] 
    /// 对象。仅处理未过期的定时器。
    ///
    /// # 警告 (Warnings)
    ///
    /// 如果在推进期间分配了 >= 1,000,000 个时间事件，则记录一条警告。
    ///
    /// # Panics
    ///
    /// 如果 `to_time_ns` 小于当前内部时钟时间，则会 panic。
    pub fn advance_time(&mut self, to_time_ns: UnixNanos, set_time: bool) -> Vec<TimeEvent> {
        const WARN_TIME_EVENTS_THRESHOLD: usize = 1_000_000;

        let from_time_ns = self.time.get_time_ns();

        assert!(
            to_time_ns >= from_time_ns,
            "Invariant violated: time must be non-decreasing, `to_time_ns` {to_time_ns} < `from_time_ns` {from_time_ns}"
        );

        if set_time {
            self.time.set_time(to_time_ns);
        }

        // 迭代并推进定时器并收集事件，仅保留存活的定时器
        let mut events: Vec<TimeEvent> = Vec::new();
        self.timers.retain(|_, timer| {
            timer.advance(to_time_ns).for_each(|event| {
                events.push(event);
            });

            !timer.is_expired()
        });

        if events.len() >= WARN_TIME_EVENTS_THRESHOLD {
            log::warn!(
                "Allocated {} time events during clock advancement from {} to {}, \
                 consider stopping the timer between large time ranges with no data points",
                events.len().separate_with_commas(),
                from_time_ns,
                to_time_ns
            );
        }

        events.sort_by_key(|a| a.ts_event);
        events
    }

    /// 将 [`TimeEvent`] 对象与其对应的事件处理器进行匹配。
    ///
    /// 此函数接收一个 `TimeEvent` 对象的 `events` 向量，假设它们已经根据 
    /// `ts_event` 进行了排序，并将它们与内部回调注册表中的适当回调处理器匹配。
    /// 如果未找到特定事件的回调，则使用默认回调。
    ///
    /// # Panics
    ///
    /// 如果匹配处理器时仍未为时钟设置默认回调，则会 panic。
    #[must_use]
    pub fn match_handlers(&self, events: Vec<TimeEvent>) -> Vec<TimeEventHandler> {
        events
            .into_iter()
            .map(|event| self.callbacks.get_handler(event))
            .collect()
    }

    fn replace_existing_timer_if_needed(&mut self, name: &Ustr) {
        if self.timer_exists(name) {
            self.cancel_timer(name.as_str());
            log::warn!("Timer '{name}' replaced");
        }
    }
}

impl Default for TestClock {
    /// 创建一个新的默认 [`TestClock`] 实例。
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for TestClock {
    type Target = AtomicTime;

    fn deref(&self) -> &Self::Target {
        &self.time
    }
}

impl Clock for TestClock {
    fn timestamp_ns(&self) -> UnixNanos {
        self.time.get_time_ns()
    }

    fn timestamp_us(&self) -> u64 {
        self.time.get_time_us()
    }

    fn timestamp_ms(&self) -> u64 {
        self.time.get_time_ms()
    }

    fn timestamp(&self) -> f64 {
        self.time.get_time()
    }

    fn timer_names(&self) -> Vec<&str> {
        self.timers
            .iter()
            .filter(|(_, timer)| !timer.is_expired())
            .map(|(k, _)| k.as_str())
            .collect()
    }

    fn timer_count(&self) -> usize {
        self.timers
            .iter()
            .filter(|(_, timer)| !timer.is_expired())
            .count()
    }

    fn timer_exists(&self, name: &Ustr) -> bool {
        self.timers.contains_key(name)
    }

    fn register_default_handler(&mut self, callback: TimeEventCallback) {
        self.callbacks.register_default_handler(callback);
    }

    /// 返回给定 [`TimeEvent`] 的处理器。
    ///
    /// # Panics
    ///
    /// 如果未为该事件注册特定事件或默认的回调，则会 panic。
    fn get_handler(&self, event: TimeEvent) -> TimeEventHandler {
        self.callbacks.get_handler(event)
    }

    fn set_time_alert_ns(
        &mut self,
        name: &str,
        alert_time_ns: UnixNanos,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
    ) -> anyhow::Result<()> {
        let ts_now = self.get_time_ns();
        let (name, alert_time_ns) =
            validate_and_prepare_time_alert(name, alert_time_ns, allow_past, ts_now)?;

        self.replace_existing_timer_if_needed(&name);

        check_predicate_true(
            callback.is_some() | self.callbacks.has_any_callback(&name),
            "No callbacks provided",
        )?;

        if let Some(callback) = callback {
            self.callbacks.register_callback(name, callback);
        }

        // 在确保 alert_time_ns >= ts_now 之后，现在可以安全计算间隔了
        let interval_ns = create_valid_interval((alert_time_ns - ts_now).into());
        let fire_immediately = alert_time_ns == ts_now;

        let timer = TestTimer::new(
            name,
            interval_ns,
            ts_now,
            Some(alert_time_ns),
            fire_immediately,
        );
        self.timers.insert(name, timer);

        Ok(())
    }

    fn set_timer_ns(
        &mut self,
        name: &str,
        interval_ns: u64,
        start_time_ns: Option<UnixNanos>,
        stop_time_ns: Option<UnixNanos>,
        callback: Option<TimeEventCallback>,
        allow_past: Option<bool>,
        fire_immediately: Option<bool>,
    ) -> anyhow::Result<()> {
        let ts_now = self.get_time_ns();
        let (name, start_time_ns, stop_time_ns, _allow_past, fire_immediately) =
            validate_and_prepare_timer(
                name,
                interval_ns,
                start_time_ns,
                stop_time_ns,
                allow_past,
                fire_immediately,
                ts_now,
            )?;

        check_predicate_true(
            callback.is_some() | self.callbacks.has_any_callback(&name),
            "No callbacks provided",
        )?;

        self.replace_existing_timer_if_needed(&name);

        if let Some(callback) = callback {
            self.callbacks.register_callback(name, callback);
        }

        let interval_ns = create_valid_interval(interval_ns);

        let timer = TestTimer::new(
            name,
            interval_ns,
            start_time_ns,
            stop_time_ns,
            fire_immediately,
        );
        self.timers.insert(name, timer);

        Ok(())
    }

    fn next_time_ns(&self, name: &str) -> Option<UnixNanos> {
        self.timers
            .get(&Ustr::from(name))
            .map(|timer| timer.next_time_ns())
    }

    fn cancel_timer(&mut self, name: &str) {
        let timer = self.timers.remove(&Ustr::from(name));
        if let Some(mut timer) = timer {
            timer.cancel();
        }
    }

    fn cancel_timers(&mut self) {
        for timer in &mut self.timers.values_mut() {
            timer.cancel();
        }

        self.timers.clear();
    }

    fn reset(&mut self) {
        self.time = AtomicTime::new(false, UnixNanos::default());
        self.timers = BTreeMap::new();
        self.callbacks.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use nautilus_core::{MUTEX_POISONED, UnixNanos};
    use rstest::{fixture, rstest};
    use ustr::Ustr;

    use super::*;
    use crate::timer::{TimeEvent, TimeEventCallback};

    #[derive(Debug, Default)]
    struct TestCallback {
        /// 在定时器回调内部更新的共享标志；Mutex 保证闭包在测试中是 `Send` 的。
        called: Arc<Mutex<bool>>,
    }

    impl TestCallback {
        fn new(called: Arc<Mutex<bool>>) -> Self {
            Self { called }
        }
    }

    impl From<TestCallback> for TimeEventCallback {
        fn from(callback: TestCallback) -> Self {
            Self::from(move |_event: TimeEvent| {
                if let Ok(mut called) = callback.called.lock() {
                    *called = true;
                }
            })
        }
    }

    #[fixture]
    pub fn test_clock() -> TestClock {
        let mut clock = TestClock::new();
        clock.register_default_handler(TestCallback::default().into());
        clock
    }

    #[rstest]
    fn test_time_monotonicity(mut test_clock: TestClock) {
        let initial_time = test_clock.timestamp_ns();
        test_clock.advance_time(UnixNanos::from(*initial_time + 1000), true);
        assert!(test_clock.timestamp_ns() > initial_time);
    }

    #[rstest]
    fn test_timer_registration(mut test_clock: TestClock) {
        test_clock
            .set_time_alert_ns(
                "test_timer",
                (*test_clock.timestamp_ns() + 1000).into(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(test_clock.timer_count(), 1);
        assert_eq!(test_clock.timer_names(), vec!["test_timer"]);
    }

    #[rstest]
    fn test_timer_expiration(mut test_clock: TestClock) {
        let alert_time = (*test_clock.timestamp_ns() + 1000).into();
        test_clock
            .set_time_alert_ns("test_timer", alert_time, None, None)
            .unwrap();
        let events = test_clock.advance_time(alert_time, true);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name.as_str(), "test_timer");
    }

    #[rstest]
    fn test_timer_cancellation(mut test_clock: TestClock) {
        test_clock
            .set_time_alert_ns(
                "test_timer",
                (*test_clock.timestamp_ns() + 1000).into(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(test_clock.timer_count(), 1);
        test_clock.cancel_timer("test_timer");
        assert_eq!(test_clock.timer_count(), 0);
    }

    #[rstest]
    fn test_time_advancement(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        test_clock
            .set_timer_ns("test_timer", 1000, Some(start_time), None, None, None, None)
            .unwrap();
        let events = test_clock.advance_time(UnixNanos::from(*start_time + 2500), true);
        assert_eq!(events.len(), 2);
        assert_eq!(*events[0].ts_event, *start_time + 1000);
        assert_eq!(*events[1].ts_event, *start_time + 2000);
    }

    #[rstest]
    fn test_default_and_custom_callbacks() {
        let mut clock = TestClock::new();
        let default_called = Arc::new(Mutex::new(false));
        let custom_called = Arc::new(Mutex::new(false));

        let default_callback = TestCallback::new(Arc::clone(&default_called));
        let custom_callback = TestCallback::new(Arc::clone(&custom_called));

        clock.register_default_handler(TimeEventCallback::from(default_callback));
        clock
            .set_time_alert_ns(
                "default_timer",
                (*clock.timestamp_ns() + 1000).into(),
                None,
                None,
            )
            .unwrap();
        clock
            .set_time_alert_ns(
                "custom_timer",
                (*clock.timestamp_ns() + 1000).into(),
                Some(TimeEventCallback::from(custom_callback)),
                None,
            )
            .unwrap();

        let events = clock.advance_time(UnixNanos::from(*clock.timestamp_ns() + 1000), true);
        let handlers = clock.match_handlers(events);

        for handler in handlers {
            handler.callback.call(handler.event);
        }

        assert!(*default_called.lock().expect(MUTEX_POISONED));
        assert!(*custom_called.lock().expect(MUTEX_POISONED));
    }

    #[rstest]
    fn test_timer_with_rust_local_callback() {
        use std::{cell::RefCell, rc::Rc};

        let mut clock = TestClock::new();
        let call_count = Rc::new(RefCell::new(0_u32));
        let call_count_clone = Rc::clone(&call_count);

        // 使用 Rc 创建 RustLocal 回调（非 Send/Sync）
        let callback: Rc<dyn Fn(TimeEvent)> = Rc::new(move |_event: TimeEvent| {
            *call_count_clone.borrow_mut() += 1;
        });

        clock
            .set_time_alert_ns(
                "local_timer",
                (*clock.timestamp_ns() + 1000).into(),
                Some(TimeEventCallback::from(callback)),
                None,
            )
            .unwrap();

        let events = clock.advance_time(UnixNanos::from(*clock.timestamp_ns() + 1000), true);
        let handlers = clock.match_handlers(events);

        for handler in handlers {
            handler.callback.call(handler.event);
        }

        assert_eq!(*call_count.borrow(), 1);
    }

    #[rstest]
    fn test_multiple_timers(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        test_clock
            .set_timer_ns("timer1", 1000, Some(start_time), None, None, None, None)
            .unwrap();
        test_clock
            .set_timer_ns("timer2", 2000, Some(start_time), None, None, None, None)
            .unwrap();
        let events = test_clock.advance_time(UnixNanos::from(*start_time + 2000), true);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].name.as_str(), "timer1");
        assert_eq!(events[1].name.as_str(), "timer1");
        assert_eq!(events[2].name.as_str(), "timer2");
    }

    #[rstest]
    fn test_allow_past_parameter_true(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(2000));
        let current_time = test_clock.timestamp_ns();
        let past_time = UnixNanos::from(current_time.as_u64() - 1000);

        // 当 allow_past=true（默认值）时，应调整为当前时间并成功执行
        test_clock
            .set_time_alert_ns("past_timer", past_time, None, Some(true))
            .unwrap();

        // 验证是否已使用调整后的时间创建了定时器
        assert_eq!(test_clock.timer_count(), 1);
        assert_eq!(test_clock.timer_names(), vec!["past_timer"]);

        // 下一次时间应等于或晚于当前时间，而不应在过去
        let next_time = test_clock.next_time_ns("past_timer").unwrap();
        assert!(next_time >= current_time);
    }

    #[rstest]
    fn test_allow_past_parameter_false(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(2000));
        let current_time = test_clock.timestamp_ns();
        let past_time = current_time - 1000;

        // 当 allow_past=false 时，对于过去的时间应失败
        let result = test_clock.set_time_alert_ns("past_timer", past_time, None, Some(false));

        // 验证操作是否因适当的错误而失败
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("was in the past"));

        // 验证未创建任何定时器
        assert_eq!(test_clock.timer_count(), 0);
        assert!(test_clock.timer_names().is_empty());
    }

    #[rstest]
    fn test_invalid_stop_time_validation(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(2000));
        let current_time = test_clock.timestamp_ns();
        let start_time = current_time + 1000;
        let stop_time = current_time + 500; // 停止时间在开始时间之前

        // 由于 stop_time < start_time，应该失败
        let result = test_clock.set_timer_ns(
            "invalid_timer",
            100,
            Some(start_time),
            Some(stop_time),
            None,
            None,
            None,
        );

        // 验证操作是否失败并返回适当的错误
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("must be after start time"));

        // 验证没有创建定时器
        assert_eq!(test_clock.timer_count(), 0);
    }

    #[rstest]
    fn test_set_timer_ns_fire_immediately_true(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;

        test_clock
            .set_timer_ns(
                "fire_immediately_timer",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                Some(true),
            )
            .unwrap();

        // 推进时间以检查是否立即触发以及随后的间隔
        let events = test_clock.advance_time(start_time + 2500, true);

        // 应该在 start_time (0) 立即触发，然后是 start_time+1000，再然后是 start_time+2000
        assert_eq!(events.len(), 3);
        assert_eq!(*events[0].ts_event, *start_time); // 立即触发
        assert_eq!(*events[1].ts_event, *start_time + 1000); // 随后在间隔后触发
        assert_eq!(*events[2].ts_event, *start_time + 2000); // 再次在第二个间隔后触发
    }

    #[rstest]
    fn test_set_timer_ns_fire_immediately_false(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;

        test_clock
            .set_timer_ns(
                "normal_timer",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                Some(false),
            )
            .unwrap();

        // 推进时间以检查正常行为
        let events = test_clock.advance_time(start_time + 2500, true);

        // 应该在第一个间隔后触发，而不是立即触发
        assert_eq!(events.len(), 2);
        assert_eq!(*events[0].ts_event, *start_time + 1000); // 第一个间隔后触发
        assert_eq!(*events[1].ts_event, *start_time + 2000); // 随后在第二个间隔后触发
    }

    #[rstest]
    fn test_set_timer_ns_fire_immediately_default_is_false(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;

        // 不指定 fire_immediately (应该默认为 false)
        test_clock
            .set_timer_ns(
                "default_timer",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let events = test_clock.advance_time(start_time + 1500, true);

        // 行为应该与 fire_immediately=false 相同
        assert_eq!(events.len(), 1);
        assert_eq!(*events[0].ts_event, *start_time + 1000); // 在第一个间隔后触发
    }

    #[rstest]
    fn test_set_timer_ns_fire_immediately_with_zero_start_time(mut test_clock: TestClock) {
        test_clock.set_time(5000.into());
        let interval_ns = 1000;

        test_clock
            .set_timer_ns(
                "zero_start_timer",
                interval_ns,
                None,
                None,
                None,
                None,
                Some(true),
            )
            .unwrap();

        let events = test_clock.advance_time(UnixNanos::from(7000), true);

        // 当开始时间为零时，应该使用当前时间作为开始时间
        // 在当前时间 (5000) 立即触发，然后在 6000、7000 触发
        assert_eq!(events.len(), 3);
        assert_eq!(*events[0].ts_event, 5000); // 当前时间立即触发
        assert_eq!(*events[1].ts_event, 6000);
        assert_eq!(*events[2].ts_event, 7000);
    }

    #[rstest]
    fn test_multiple_timers_different_fire_immediately_settings(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;

        // One timer with fire_immediately=true
        test_clock
            .set_timer_ns(
                "immediate_timer",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                Some(true),
            )
            .unwrap();

        // One timer with fire_immediately=false
        test_clock
            .set_timer_ns(
                "normal_timer",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                Some(false),
            )
            .unwrap();

        let events = test_clock.advance_time(start_time + 1500, true);

        // 总共应该有 3 个事件：immediate_timer 在开始时间和 1000 触发，normal_timer 在 1000 触发
        assert_eq!(events.len(), 3);

        // 按时间戳对事件进行排序以检查顺序
        let mut event_times: Vec<u64> = events.iter().map(|e| e.ts_event.as_u64()).collect();
        event_times.sort_unstable();

        assert_eq!(event_times[0], start_time.as_u64()); // immediate_timer 立即触发
        assert_eq!(event_times[1], start_time.as_u64() + 1000); // 两个定时器都在 1000 触发
        assert_eq!(event_times[2], start_time.as_u64() + 1000); // 两个定时器都在 1000 触发
    }

    #[rstest]
    fn test_timer_name_collision_overwrites(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();

        // 设置第一个定时器
        test_clock
            .set_timer_ns(
                "collision_timer",
                1000,
                Some(start_time),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // 设置同名定时器应该覆盖现有定时器
        let result = test_clock.set_timer_ns(
            "collision_timer",
            2000,
            Some(start_time),
            None,
            None,
            None,
            None,
        );

        assert!(result.is_ok());
        // 应该仍然只有一个定时器（被覆盖）
        assert_eq!(test_clock.timer_count(), 1);

        // 定时器应该具有新的时间间隔
        let next_time = test_clock.next_time_ns("collision_timer").unwrap();
        // 间隔为 2000，从 start_time 开始，下一个时间应该是 start_time + 2000
        assert_eq!(next_time, start_time + 2000);
    }

    #[rstest]
    fn test_timer_zero_interval_error(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();

        // 尝试设置间隔为零的定时器应该失败
        let result =
            test_clock.set_timer_ns("zero_interval", 0, Some(start_time), None, None, None, None);

        assert!(result.is_err());
        assert_eq!(test_clock.timer_count(), 0);
    }

    #[rstest]
    fn test_timer_empty_name_error(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();

        // 尝试设置空名称的定时器应该失败
        let result = test_clock.set_timer_ns("", 1000, Some(start_time), None, None, None, None);

        assert!(result.is_err());
        assert_eq!(test_clock.timer_count(), 0);
    }

    #[rstest]
    fn test_timer_exists(mut test_clock: TestClock) {
        let name = Ustr::from("exists_timer");
        assert!(!test_clock.timer_exists(&name));

        test_clock
            .set_time_alert_ns(
                name.as_str(),
                (*test_clock.timestamp_ns() + 1_000).into(),
                None,
                None,
            )
            .unwrap();

        assert!(test_clock.timer_exists(&name));
    }

    #[rstest]
    fn test_timer_rejects_past_stop_time_when_not_allowed(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(10_000));
        let current = test_clock.timestamp_ns();

        let result = test_clock.set_timer_ns(
            "past_stop",
            10_000,
            Some(current - 500),
            Some(current - 100),
            None,
            Some(false),
            None,
        );

        let err = result.expect_err("expected stop time validation error");
        let err_msg = err.to_string();
        assert!(err_msg.contains("stop time"));
        assert!(err_msg.contains("in the past"));
    }

    #[rstest]
    fn test_timer_accepts_future_stop_time(mut test_clock: TestClock) {
        let current = test_clock.timestamp_ns();

        let result = test_clock.set_timer_ns(
            "future_stop",
            1_000,
            Some(current),
            Some(current + 10_000),
            None,
            Some(false),
            None,
        );

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_timer_fire_immediately_at_exact_stop_time(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;
        let stop_time = start_time + interval_ns; // Stop exactly at first interval

        test_clock
            .set_timer_ns(
                "exact_stop",
                interval_ns,
                Some(start_time),
                Some(stop_time),
                None,
                None,
                Some(true),
            )
            .unwrap();

        let events = test_clock.advance_time(stop_time, true);

        // 应该在开始时立即触发，然后在停止时间触发（等于第一个间隔）
        assert_eq!(events.len(), 2);
        assert_eq!(*events[0].ts_event, *start_time); // 立即触发
        assert_eq!(*events[1].ts_event, *stop_time); // 在停止时间触发
    }

    #[rstest]
    fn test_timer_advance_to_exact_next_time(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();
        let interval_ns = 1000;

        test_clock
            .set_timer_ns(
                "exact_advance",
                interval_ns,
                Some(start_time),
                None,
                None,
                None,
                Some(false),
            )
            .unwrap();

        // 推进到准确的下一个触发时间
        let next_time = test_clock.next_time_ns("exact_advance").unwrap();
        let events = test_clock.advance_time(next_time, true);

        assert_eq!(events.len(), 1);
        assert_eq!(*events[0].ts_event, *next_time);
    }

    #[rstest]
    fn test_allow_past_bar_aggregation_use_case(mut test_clock: TestClock) {
        // 模拟 K 线聚合场景：当前时间处于一个 K 线窗口中间
        test_clock.set_time(UnixNanos::from(100_500)); // 100.5 秒

        let bar_start_time = UnixNanos::from(100_000); // 100 秒 (0.5 秒前)
        let interval_ns = 1000; // 1 秒一根 K 线

        // 设置 allow_past=false 且 fire_immediately=false：
        // start_time 在过去（100 秒）但下一个事件（101 秒）在未来
        // 这对于 K 线聚合应该是 被允许的
        let result = test_clock.set_timer_ns(
            "bar_timer",
            interval_ns,
            Some(bar_start_time),
            None,
            None,
            Some(false), // allow_past = false
            Some(false), // fire_immediately = false
        );

        // 应该成功，因为下一个事件时间 (100_000 + 1000 = 101_000) > 当前时间 (100_500)
        assert!(result.is_ok());
        assert_eq!(test_clock.timer_count(), 1);

        // 下一个事件应该在 bar_start_time + interval = 101_000
        let next_time = test_clock.next_time_ns("bar_timer").unwrap();
        assert_eq!(*next_time, 101_000);
    }

    #[rstest]
    fn test_allow_past_false_rejects_when_next_event_in_past(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(102_000)); // 102 秒

        let past_start_time = UnixNanos::from(100_000); // 100 秒 (2 秒前)
        let interval_ns = 1000; // 1 秒间隔

        // 设置 allow_past=false 且 fire_immediately=false：
        // 下一个事件将是 100_000 + 1000 = 101_000，小于当前时间 (102_000)
        // 这应该被 拒绝
        let result = test_clock.set_timer_ns(
            "past_event_timer",
            interval_ns,
            Some(past_start_time),
            None,
            None,
            Some(false), // allow_past = false
            Some(false), // fire_immediately = false
        );

        // 应该失败，因为下一个事件时间 (101_000) < 当前时间 (102_000)
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("would be in the past")
        );
    }

    #[rstest]
    fn test_allow_past_false_with_fire_immediately_true(mut test_clock: TestClock) {
        test_clock.set_time(UnixNanos::from(100_500)); // 100.5 秒

        let past_start_time = UnixNanos::from(100_000); // 100 秒 (0.5 秒前)
        let interval_ns = 1000;

        // 设置 fire_immediately=true，下个事件 = start_time（在过去）
        // 当 allow_past=false 时，这应该被 拒绝
        let result = test_clock.set_timer_ns(
            "immediate_past_timer",
            interval_ns,
            Some(past_start_time),
            None,
            None,
            Some(false), // allow_past = false
            Some(true),  // fire_immediately = true
        );

        // 应该失败，因为下一个事件时间 (100_000) < 当前时间 (100_500)
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("would be in the past")
        );
    }

    #[rstest]
    fn test_cancel_timer_during_execution(mut test_clock: TestClock) {
        let start_time = test_clock.timestamp_ns();

        test_clock
            .set_timer_ns(
                "cancel_test",
                1000,
                Some(start_time),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);

        // 取消定时器
        test_clock.cancel_timer("cancel_test");

        assert_eq!(test_clock.timer_count(), 0);

        // 推进时间 - 不应从已取消的定时器处获取任何事件
        let events = test_clock.advance_time(start_time + 2000, true);
        assert_eq!(events.len(), 0);
    }

    #[rstest]
    fn test_cancel_all_timers(mut test_clock: TestClock) {
        // Create multiple timers
        test_clock
            .set_timer_ns("timer1", 1000, None, None, None, None, None)
            .unwrap();
        test_clock
            .set_timer_ns("timer2", 1500, None, None, None, None, None)
            .unwrap();
        test_clock
            .set_timer_ns("timer3", 2000, None, None, None, None, None)
            .unwrap();

        assert_eq!(test_clock.timer_count(), 3);

        // 取消所有定时器
        test_clock.cancel_timers();

        assert_eq!(test_clock.timer_count(), 0);

        // 推进时间 - 不应获取任何事件
        let events = test_clock.advance_time(UnixNanos::from(5000), true);
        assert_eq!(events.len(), 0);
    }

    #[rstest]
    fn test_clock_reset_clears_timers(mut test_clock: TestClock) {
        test_clock
            .set_timer_ns("reset_test", 1000, None, None, None, None, None)
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);

        // 重置时钟
        test_clock.reset();

        assert_eq!(test_clock.timer_count(), 0);
        assert_eq!(test_clock.timestamp_ns(), UnixNanos::default()); // 时间重置为零
    }

    #[rstest]
    fn test_set_time_alert_default_impl(mut test_clock: TestClock) {
        let current_time = test_clock.utc_now();
        let alert_time = current_time + chrono::Duration::seconds(1);

        // 测试委托给 set_time_alert_ns 的默认实现
        test_clock
            .set_time_alert("alert_test", alert_time, None, None)
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);
        assert_eq!(test_clock.timer_names(), vec!["alert_test"]);

        // 验证定时器是否设置为正确的时间
        let expected_ns = UnixNanos::from(alert_time);
        let next_time = test_clock.next_time_ns("alert_test").unwrap();

        // 由于转换，时间应该非常接近（在几纳秒之内）
        let diff = if next_time >= expected_ns {
            next_time.as_u64() - expected_ns.as_u64()
        } else {
            expected_ns.as_u64() - next_time.as_u64()
        };
        assert!(
            diff < 1000,
            "定时器应设置为预期时间的 1 微秒之内"
        );
    }

    #[rstest]
    fn test_set_timer_default_impl(mut test_clock: TestClock) {
        let current_time = test_clock.utc_now();
        let start_time = current_time + chrono::Duration::seconds(1);
        let interval = Duration::from_millis(500);

        // 测试委托给 set_timer_ns 的默认实现
        test_clock
            .set_timer(
                "timer_test",
                interval,
                Some(start_time),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);
        assert_eq!(test_clock.timer_names(), vec!["timer_test"]);

        // 推进时间并验证定时器是否在正确的时间间隔触发
        let start_ns = UnixNanos::from(start_time);
        let interval_ns = interval.as_nanos() as u64;

        let events = test_clock.advance_time(start_ns + interval_ns * 3, true);
        assert_eq!(events.len(), 3); // 应该触发 3 次

        // 验证时间
        assert_eq!(*events[0].ts_event, *start_ns + interval_ns);
        assert_eq!(*events[1].ts_event, *start_ns + interval_ns * 2);
        assert_eq!(*events[2].ts_event, *start_ns + interval_ns * 3);
    }

    #[rstest]
    fn test_set_timer_with_stop_time_default_impl(mut test_clock: TestClock) {
        let current_time = test_clock.utc_now();
        let start_time = current_time + chrono::Duration::seconds(1);
        let stop_time = current_time + chrono::Duration::seconds(3);
        let interval = Duration::from_secs(1);

        // 带停止时间的测试
        test_clock
            .set_timer(
                "timer_with_stop",
                interval,
                Some(start_time),
                Some(stop_time),
                None,
                None,
                None,
            )
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);

        // 推进到超出停止时间
        let stop_ns = UnixNanos::from(stop_time);
        let events = test_clock.advance_time(stop_ns + 1000, true);

        // 应该触发两次：在 start_time + 1s 和 start_time + 2s，
        // 但不在 start_time + 3s，因为那已经是在停止时间了
        assert_eq!(events.len(), 2);

        let start_ns = UnixNanos::from(start_time);
        let interval_ns = interval.as_nanos() as u64;
        assert_eq!(*events[0].ts_event, *start_ns + interval_ns);
        assert_eq!(*events[1].ts_event, *start_ns + interval_ns * 2);
    }

    #[rstest]
    fn test_set_timer_fire_immediately_default_impl(mut test_clock: TestClock) {
        let current_time = test_clock.utc_now();
        let start_time = current_time + chrono::Duration::seconds(1);
        let interval = Duration::from_millis(500);

        // 设置 fire_immediately=true 的测试
        test_clock
            .set_timer(
                "immediate_timer",
                interval,
                Some(start_time),
                None,
                None,
                None,
                Some(true),
            )
            .unwrap();

        let start_ns = UnixNanos::from(start_time);
        let interval_ns = interval.as_nanos() as u64;

        // 推进到开始时间 + 1 个间隔
        let events = test_clock.advance_time(start_ns + interval_ns, true);

        // 应该在开始时间立即触发，然后在开始时间 + 间隔再次触发
        assert_eq!(events.len(), 2);
        assert_eq!(*events[0].ts_event, *start_ns); // 立即触发
        assert_eq!(*events[1].ts_event, *start_ns + interval_ns); // 常规间隔触发
    }

    #[rstest]
    fn test_set_time_alert_when_alert_time_equals_current_time(mut test_clock: TestClock) {
        let current_time = test_clock.timestamp_ns();

        // 为准确的当前时间设置时间警报
        test_clock
            .set_time_alert_ns("alert_at_current_time", current_time, None, None)
            .unwrap();

        assert_eq!(test_clock.timer_count(), 1);

        // 推进 0 的时间（即到当前时间） - 应该立即触发
        let events = test_clock.advance_time(current_time, true);

        // 由于 alert_time_ns == ts_now，应该立即触发
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name.as_str(), "alert_at_current_time");
        assert_eq!(*events[0].ts_event, *current_time);
    }

    #[rstest]
    fn test_cancel_and_reschedule_same_name(mut test_clock: TestClock) {
        let start = test_clock.timestamp_ns();

        test_clock
            .set_time_alert_ns("timer", UnixNanos::from(*start + 1000), None, None)
            .unwrap();
        assert_eq!(test_clock.timer_count(), 1);

        test_clock.cancel_timer("timer");
        assert_eq!(test_clock.timer_count(), 0);

        test_clock
            .set_time_alert_ns("timer", UnixNanos::from(*start + 2000), None, None)
            .unwrap();
        assert_eq!(test_clock.timer_count(), 1);

        let events = test_clock.advance_time(UnixNanos::from(*start + 1500), true);
        assert!(events.is_empty());

        let events = test_clock.advance_time(UnixNanos::from(*start + 2000), true);
        assert_eq!(events.len(), 1);
        assert_eq!(*events[0].ts_event, *start + 2000);
    }

    #[rstest]
    fn test_multiple_timers_same_timestamp_all_fire(mut test_clock: TestClock) {
        let fire_time = UnixNanos::from(*test_clock.timestamp_ns() + 1000);

        for i in 0..5 {
            test_clock
                .set_time_alert_ns(&format!("timer_{i}"), fire_time, None, None)
                .unwrap();
        }
        assert_eq!(test_clock.timer_count(), 5);

        let events = test_clock.advance_time(fire_time, true);
        assert_eq!(events.len(), 5);
        for event in &events {
            assert_eq!(*event.ts_event, *fire_time);
        }
    }

    #[rstest]
    fn test_events_ordered_by_timestamp_after_advance() {
        let mut clock = TestClock::new();
        clock.register_default_handler(TestCallback::default().into());
        let start = clock.timestamp_ns();

        clock
            .set_time_alert_ns("third", UnixNanos::from(*start + 300), None, None)
            .unwrap();
        clock
            .set_time_alert_ns("first", UnixNanos::from(*start + 100), None, None)
            .unwrap();
        clock
            .set_time_alert_ns("second", UnixNanos::from(*start + 200), None, None)
            .unwrap();

        let events = clock.advance_time(UnixNanos::from(*start + 400), true);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].name.as_str(), "first");
        assert_eq!(events[1].name.as_str(), "second");
        assert_eq!(events[2].name.as_str(), "third");
    }

    #[rstest]
    fn test_large_interval_does_not_overflow(mut test_clock: TestClock) {
        let start = test_clock.timestamp_ns();
        let large_interval: u64 = 1_000_000_000 * 60 * 60 * 24 * 365; // ~1 year in ns

        test_clock
            .set_timer_ns(
                "large_interval",
                large_interval,
                Some(start),
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let events = test_clock.advance_time(UnixNanos::from(*start + large_interval), true);
        assert_eq!(events.len(), 1);
        assert_eq!(*events[0].ts_event, *start + large_interval);
    }

    #[rstest]
    fn test_near_zero_interval_fires_correctly(mut test_clock: TestClock) {
        let start = test_clock.timestamp_ns();

        test_clock
            .set_timer_ns("tiny", 1, Some(start), None, None, None, None)
            .unwrap();

        let events = test_clock.advance_time(UnixNanos::from(*start + 10), true);
        assert_eq!(events.len(), 10);

        for i in 1..events.len() {
            assert!(events[i].ts_event >= events[i - 1].ts_event);
        }
    }

    #[rstest]
    fn test_repeated_advance_to_same_time_no_double_fire(mut test_clock: TestClock) {
        let fire_time = UnixNanos::from(*test_clock.timestamp_ns() + 1000);

        test_clock
            .set_time_alert_ns("once", fire_time, None, None)
            .unwrap();

        let events1 = test_clock.advance_time(fire_time, true);
        assert_eq!(events1.len(), 1);

        let events2 = test_clock.advance_time(fire_time, true);
        assert!(events2.is_empty());
    }

    #[rstest]
    fn test_advance_with_no_timers(mut test_clock: TestClock) {
        let start = test_clock.timestamp_ns();

        let events = test_clock.advance_time(UnixNanos::from(*start + 1000), true);
        assert!(events.is_empty());
        assert_eq!(*test_clock.timestamp_ns(), *start + 1000);
    }
}
