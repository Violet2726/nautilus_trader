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

//! 用于 `Clock` 实现的实时和测试定时器。

use std::{
    cmp::Ordering,
    fmt::{Debug, Display},
    num::NonZeroU64,
    rc::Rc,
    sync::Arc,
};

use nautilus_core::{
    UUID4, UnixNanos,
    correctness::{FAILED, check_valid_string_utf8},
};
#[cfg(feature = "python")]
use pyo3::{Py, PyAny, Python};
use ustr::Ustr;

/// 创建一个合法的纳秒间隔，保证为正数。
///
/// 将 0 强制转换为 1 以确保返回一个有效的 `NonZeroU64`。
#[must_use]
#[allow(clippy::missing_panics_doc)] // Value is coerced to >= 1
pub fn create_valid_interval(interval_ns: u64) -> NonZeroU64 {
    NonZeroU64::new(std::cmp::max(interval_ns, 1)).expect("`interval_ns` 必须为正数")
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.common", from_py_object)
)]
/// 表示在事件时间戳处发生的一个时间事件 (Time Event)。
///
/// 一个 `TimeEvent` 携带元数据，如事件名称、唯一的事件 ID，
/// 以及指明事件计划发生时间和初始化时间的时间戳。
pub struct TimeEvent {
    /// 事件名称，标识事件的性质或目的。
    pub name: Ustr,
    /// 事件的唯一标识符。
    pub event_id: UUID4,
    /// 事件发生时的 UNIX 时间戳（纳秒）。
    pub ts_event: UnixNanos,
    /// 实例创建时的 UNIX 时间戳（纳秒）。
    pub ts_init: UnixNanos,
}

impl TimeEvent {
    /// 创建一个新的 [`TimeEvent`] 实例。
    ///
    /// # 安全性 (Safety)
    ///
    /// 假设 `name` 是一个有效的字符串。
    #[must_use]
    pub const fn new(name: Ustr, event_id: UUID4, ts_event: UnixNanos, ts_init: UnixNanos) -> Self {
        Self {
            name,
            event_id,
            ts_event,
            ts_init,
        }
    }
}

impl Display for TimeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}(name={}, event_id={}, ts_event={}, ts_init={})",
            stringify!(TimeEvent),
            self.name,
            self.event_id,
            self.ts_event,
            self.ts_init
        )
    }
}

/// [`TimeEvent`] 的包装器，实现了基于时间戳的排序，以便堆调度 (heap scheduling)。
///
/// 这个新类型允许时间事件在优先级队列（最大堆）中按其时间戳进行排序，
/// 同时保持 [`TimeEvent`] 本身具有标准的基于字段的等价性。
/// 事件按倒序排列（越早的时间戳具有越高的优先级）。
#[repr(transparent)] // 保证与相同内存布局的零成本抽象 (zero-cost abstraction)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledTimeEvent(pub TimeEvent);

impl ScheduledTimeEvent {
    /// 创建一个新的已安排时间事件。
    #[must_use]
    pub const fn new(event: TimeEvent) -> Self {
        Self(event)
    }

    /// 提取内部的时间事件。
    #[must_use]
    pub fn into_inner(self) -> TimeEvent {
        self.0
    }
}

impl PartialOrd for ScheduledTimeEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledTimeEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        // 最大堆的逆序：越早的时间戳优先级越高
        other.0.ts_event.cmp(&self.0.ts_event)
    }
}

/// 时间事件的回调类型。
///
/// # 变体 (Variants)
///
/// - `Python`: 用于 Python 回调（需要 `python` feature）。
/// - `Rust`: 使用 `Arc` 的线程安全回调。当闭包是 `Send + Sync` 时使用。
/// - `RustLocal`: 使用 `Rc` 的单线程回调。在捕获 `Rc<RefCell<...>>` 时使用。
///
/// # 在 `Rust` 和 `RustLocal` 之间做出选择
///
/// 在以下情况下使用 `Rust`（线程安全）：
/// - 回调不会捕获 `Rc<RefCell<...>>` 或其他非 `Send` 类型。
/// - 闭包是 `Send + Sync`（大多数简单的闭包都符合要求）。
///
/// 在以下情况下使用 `RustLocal`：
/// - 回调捕获 `Rc<RefCell<...>>` 用于共享可变状态。
/// - 线程安全性限制了 `Arc` 的使用。
///
/// 这两种变体都适用于 `TestClock` 和 `LiveClock`。`RustLocal` 变体与 `LiveClock` 配合使用是安全的，
/// 因为回调通过通道发送并在发起线程的时间事件循环中执行 - 它们实际上从未跨越线程边界。
///
/// # 自动转换
///
/// - 符合 `Fn + Send + Sync + 'static` 的闭包会自动转换为 `Rust`。
/// - `Rc<dyn Fn(TimeEvent)>` 转换为 `RustLocal`。
/// - `Arc<dyn Fn(TimeEvent) + Send + Sync>` 转换为 `Rust`。
pub enum TimeEventCallback {
    /// 适用于通过 PyO3 从 Python 中使用的 Python Callable。
    #[cfg(feature = "python")]
    Python(Py<PyAny>),
    /// 使用 `Arc` 的线程安全 Rust 回调 (`Send + Sync`)。
    Rust(Arc<dyn Fn(TimeEvent) + Send + Sync>),
    /// 使用 `Rc` 的本地 Rust 回调（非 `Send`/`Sync`）。
    RustLocal(Rc<dyn Fn(TimeEvent)>),
}

impl Clone for TimeEventCallback {
    fn clone(&self) -> Self {
        match self {
            #[cfg(feature = "python")]
            Self::Python(obj) => Self::Python(nautilus_core::python::clone_py_object(obj)),
            Self::Rust(cb) => Self::Rust(cb.clone()),
            Self::RustLocal(cb) => Self::RustLocal(cb.clone()),
        }
    }
}

impl Debug for TimeEventCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "python")]
            Self::Python(_) => f.write_str("Python callback"),
            Self::Rust(_) => f.write_str("Rust callback (thread-safe)"),
            Self::RustLocal(_) => f.write_str("Rust callback (local)"),
        }
    }
}

impl TimeEventCallback {
    /// 如果这是线程安全的 Rust 回调，则返回 `true`。
    #[must_use]
    pub const fn is_rust(&self) -> bool {
        matches!(self, Self::Rust(_))
    }

    /// 如果这是本地（非线程安全）的 Rust 回调，则返回 `true`。
    ///
    /// 本地回调在内部使用 `Rc`。它们同时适用于 `TestClock` 和 `LiveClock`，
    /// 因为回调在发起线程上执行。
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::RustLocal(_))
    }

    /// 为给定的 `TimeEvent` 调用回调。
    ///
    /// 对于 Python 回调，异常会被记录为错误，而不是直接抛出 panic。
    pub fn call(&self, event: TimeEvent) {
        match self {
            #[cfg(feature = "python")]
            Self::Python(callback) => {
                Python::attach(|py| {
                    if let Err(e) = callback.call1(py, (event,)) {
                        log::error!("Python 时间事件回调引发了异常: {e}");
                    }
                });
            }
            Self::Rust(callback) => callback(event),
            Self::RustLocal(callback) => callback(event),
        }
    }
}

impl<F> From<F> for TimeEventCallback
where
    F: Fn(TimeEvent) + Send + Sync + 'static,
{
    fn from(value: F) -> Self {
        Self::Rust(Arc::new(value))
    }
}

impl From<Arc<dyn Fn(TimeEvent) + Send + Sync>> for TimeEventCallback {
    fn from(value: Arc<dyn Fn(TimeEvent) + Send + Sync>) -> Self {
        Self::Rust(value)
    }
}

impl From<Rc<dyn Fn(TimeEvent)>> for TimeEventCallback {
    fn from(value: Rc<dyn Fn(TimeEvent)>) -> Self {
        Self::RustLocal(value)
    }
}

#[cfg(feature = "python")]
impl From<Py<PyAny>> for TimeEventCallback {
    fn from(value: Py<PyAny>) -> Self {
        Self::Python(value)
    }
}

// 安全性 (SAFETY): TimeEventCallback 是 Send + Sync，基于以下不变性：
//
// - Python 变体: Py<PyAny> 本质上是 Send + Sync（在需要时获取 GIL）。
//
// - Rust 变体: Arc<dyn Fn + Send + Sync> 本质上是 Send + Sync。
//
// - RustLocal 变体: 使用了 Rc<dyn Fn>，它不是 Send/Sync。这是安全的，因为：
//   1. RustLocal 回调在同一线程上创建和执行
//   2. 它们虽然通过通道发送，但执行发生在发起线程的时间事件循环中（见 LiveClock/TestClock 使用模式）
//   3. Rc 绝不会跨线程边界被克隆
//
//   不变性 (INVARIANT): RustLocal 回调必须仅从创建它们的线程中调用。
//   违反此不变性会导致未定义行为（Rc 引用计数上的数据竞争）。
//   如果需要跨线程执行，请使用 Rust 变体（使用 Arc）。
#[allow(unsafe_code)]
unsafe impl Send for TimeEventCallback {}
#[allow(unsafe_code)]
unsafe impl Sync for TimeEventCallback {}

#[repr(C)]
#[derive(Clone, Debug)]
/// 表示一个时间事件及其关联的处理器。
///
/// `TimeEventHandler` 将一个 `TimeEvent` 与一个回调函数关联起来，该函数在达到事件时间戳时触发。
pub struct TimeEventHandler {
    /// 时间事件。
    pub event: TimeEvent,
    /// 事件的回调处理器。
    pub callback: TimeEventCallback,
}

impl TimeEventHandler {
    /// 创建一个新的 [`TimeEventHandler`] 实例。
    #[must_use]
    pub const fn new(event: TimeEvent, callback: TimeEventCallback) -> Self {
        Self { event, callback }
    }

    /// 通过为其关联的事件调用回调来执行处理器。
    pub fn run(self) {
        let Self { event, callback } = self;
        callback.call(event);
    }
}

impl PartialOrd for TimeEventHandler {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TimeEventHandler {
    fn eq(&self, other: &Self) -> bool {
        self.event.ts_event == other.event.ts_event
    }
}

impl Eq for TimeEventHandler {}

impl Ord for TimeEventHandler {
    fn cmp(&self, other: &Self) -> Ordering {
        self.event.ts_event.cmp(&other.event.ts_event)
    }
}

/// 用于 `TestClock` 的测试定时器。
///
/// `TestTimer` 在受控环境中模拟时间推进，允许在测试场景中精确控制事件生成。
///
/// # 线程安全性 (Threading)
///
/// 该定时器会修改其内部状态，因此只能从其所属线程中使用。
#[derive(Clone, Debug)]
pub struct TestTimer {
    /// 定时器的名称。
    pub name: Ustr,
    /// 定时器事件之间的时间间隔（以纳秒为单位）。
    pub interval_ns: NonZeroU64,
    /// 定时器的开始时间（采用 UNIX 纳秒格式）。
    pub start_time_ns: UnixNanos,
    /// 可选的定时器停止时间（采用 UNIX 纳秒格式）。
    pub stop_time_ns: Option<UnixNanos>,
    /// 定时器是否应在开始时间立即触发。
    pub fire_immediately: bool,
    next_time_ns: UnixNanos,
    is_expired: bool,
}

impl TestTimer {
    /// 创建一个新的 [`TestTimer`] 实例。
    ///
    /// # Panics
    ///
    /// 如果 `name` 不是有效的字符串，则会抛出 panic。
    #[must_use]
    pub fn new(
        name: Ustr,
        interval_ns: NonZeroU64,
        start_time_ns: UnixNanos,
        stop_time_ns: Option<UnixNanos>,
        fire_immediately: bool,
    ) -> Self {
        check_valid_string_utf8(name, stringify!(name)).expect(FAILED);

        let next_time_ns = if fire_immediately {
            start_time_ns
        } else {
            start_time_ns + interval_ns.get()
        };

        Self {
            name,
            interval_ns,
            start_time_ns,
            stop_time_ns,
            fire_immediately,
            next_time_ns,
            is_expired: false,
        }
    }

    /// 返回定时器下次触发的 UNIX 纳秒时间。
    #[must_use]
    pub const fn next_time_ns(&self) -> UnixNanos {
        self.next_time_ns
    }

    /// 返回定时器是否已过期。
    #[must_use]
    pub const fn is_expired(&self) -> bool {
        self.is_expired
    }

    #[must_use]
    pub const fn pop_event(&self, event_id: UUID4, ts_init: UnixNanos) -> TimeEvent {
        TimeEvent {
            name: self.name,
            event_id,
            ts_event: self.next_time_ns,
            ts_init,
        }
    }

    /// 将测试定时器向前推进到给定时间，生成一系列事件。
    /// 每当下一个事件时间 <= 给定的 `to_time_ns` 时，就会添加一个 [`TimeEvent`]。
    ///
    /// 这允许在单个步骤中测试多个时间间隔。
    pub fn advance(&mut self, to_time_ns: UnixNanos) -> impl Iterator<Item = TimeEvent> + '_ {
        // Calculate how many events should fire up to and including to_time_ns
        let advances = if self.next_time_ns <= to_time_ns {
            ((to_time_ns.as_u64() - self.next_time_ns.as_u64()) / self.interval_ns.get())
                .saturating_add(1)
        } else {
            0
        };
        self.take(advances as usize).map(|(event, _)| event)
    }

    /// 取消定时器（定时器将不再生成事件）。
    ///
    /// 用于在计划停用时间之前停止定时器。
    pub const fn cancel(&mut self) {
        self.is_expired = true;
    }
}

impl Iterator for TestTimer {
    type Item = (TimeEvent, UnixNanos);

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_expired {
            None
        } else {
            // Check if current event would exceed stop time before creating the event
            if let Some(stop_time_ns) = self.stop_time_ns
                && self.next_time_ns > stop_time_ns
            {
                self.is_expired = true;
                return None;
            }

            let item = (
                TimeEvent {
                    name: self.name,
                    event_id: UUID4::new(),
                    ts_event: self.next_time_ns,
                    ts_init: self.next_time_ns,
                },
                self.next_time_ns,
            );

            // Check if we should expire after this event (for repeating timers at stop boundary)
            if let Some(stop_time_ns) = self.stop_time_ns
                && self.next_time_ns == stop_time_ns
            {
                self.is_expired = true;
            }

            self.next_time_ns += self.interval_ns;

            Some(item)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use nautilus_core::UnixNanos;
    use rstest::*;
    use ustr::Ustr;

    use super::{TestTimer, TimeEvent};

    #[rstest]
    fn test_test_timer_pop_event() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(1).unwrap(),
            UnixNanos::from(1),
            None,
            false,
        );

        assert!(timer.next().is_some());
        assert!(timer.next().is_some());
        timer.is_expired = true;
        assert!(timer.next().is_none());
    }

    #[rstest]
    fn test_test_timer_advance_within_next_time_ns() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(5).unwrap(),
            UnixNanos::default(),
            None,
            false,
        );
        let _: Vec<TimeEvent> = timer.advance(UnixNanos::from(1)).collect();
        let _: Vec<TimeEvent> = timer.advance(UnixNanos::from(2)).collect();
        let _: Vec<TimeEvent> = timer.advance(UnixNanos::from(3)).collect();
        assert_eq!(timer.advance(UnixNanos::from(4)).count(), 0);
        assert_eq!(timer.next_time_ns, 5);
        assert!(!timer.is_expired);
    }

    #[rstest]
    fn test_test_timer_advance_up_to_next_time_ns() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(1).unwrap(),
            UnixNanos::default(),
            None,
            false,
        );
        assert_eq!(timer.advance(UnixNanos::from(1)).count(), 1);
        assert!(!timer.is_expired);
    }

    #[rstest]
    fn test_test_timer_advance_up_to_next_time_ns_with_stop_time() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(1).unwrap(),
            UnixNanos::default(),
            Some(UnixNanos::from(2)),
            false,
        );
        assert_eq!(timer.advance(UnixNanos::from(2)).count(), 2);
        assert!(timer.is_expired);
    }

    #[rstest]
    fn test_test_timer_advance_beyond_next_time_ns() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(1).unwrap(),
            UnixNanos::default(),
            Some(UnixNanos::from(5)),
            false,
        );
        assert_eq!(timer.advance(UnixNanos::from(5)).count(), 5);
        assert!(timer.is_expired);
    }

    #[rstest]
    fn test_test_timer_advance_beyond_stop_time() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(1).unwrap(),
            UnixNanos::default(),
            Some(UnixNanos::from(5)),
            false,
        );
        assert_eq!(timer.advance(UnixNanos::from(10)).count(), 5);
        assert!(timer.is_expired);
    }

    #[rstest]
    fn test_test_timer_advance_exact_boundary() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(5).unwrap(),
            UnixNanos::from(0),
            None,
            false,
        );
        assert_eq!(
            timer.advance(UnixNanos::from(5)).count(),
            1,
            "Expected one event at the 5 ns boundary"
        );
        assert_eq!(
            timer.advance(UnixNanos::from(10)).count(),
            1,
            "Expected one event at the 10 ns boundary"
        );
    }

    #[rstest]
    fn test_test_timer_fire_immediately_true() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(5).unwrap(),
            UnixNanos::from(10),
            None,
            true, // fire_immediately = true
        );

        // With fire_immediately=true, next_time_ns should be start_time_ns
        assert_eq!(timer.next_time_ns(), UnixNanos::from(10));

        // Advance to start time should produce an event
        let events: Vec<TimeEvent> = timer.advance(UnixNanos::from(10)).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ts_event, UnixNanos::from(10));

        // Next event should be at start_time + interval
        assert_eq!(timer.next_time_ns(), UnixNanos::from(15));
    }

    #[rstest]
    fn test_test_timer_fire_immediately_false() {
        let mut timer = TestTimer::new(
            Ustr::from("TEST_TIMER"),
            NonZeroU64::new(5).unwrap(),
            UnixNanos::from(10),
            None,
            false, // fire_immediately = false
        );

        // With fire_immediately=false, next_time_ns should be start_time_ns + interval
        assert_eq!(timer.next_time_ns(), UnixNanos::from(15));

        // Advance to start time should produce no events
        assert_eq!(timer.advance(UnixNanos::from(10)).count(), 0);

        // Advance to first interval should produce an event
        let events: Vec<TimeEvent> = timer.advance(UnixNanos::from(15)).collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].ts_event, UnixNanos::from(15));
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Property-based testing
    ////////////////////////////////////////////////////////////////////////////////

    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum TimerOperation {
        AdvanceTime(u64),
        Cancel,
    }

    fn timer_operation_strategy() -> impl Strategy<Value = TimerOperation> {
        prop_oneof![
            8 => prop::num::u64::ANY.prop_map(|v| TimerOperation::AdvanceTime(v % 1000 + 1)),
            2 => Just(TimerOperation::Cancel),
        ]
    }

    fn timer_config_strategy() -> impl Strategy<Value = (u64, u64, Option<u64>, bool)> {
        (
            1u64..=100u64,                    // interval_ns (1-100)
            0u64..=50u64,                     // start_time_ns (0-50)
            prop::option::of(51u64..=200u64), // stop_time_ns (51-200 or None)
            prop::bool::ANY,                  // fire_immediately
        )
    }

    fn timer_test_strategy()
    -> impl Strategy<Value = (Vec<TimerOperation>, (u64, u64, Option<u64>, bool))> {
        (
            prop::collection::vec(timer_operation_strategy(), 5..=50),
            timer_config_strategy(),
        )
    }

    #[allow(clippy::needless_collect)] // Collect needed for indexing and .is_empty()
    fn test_timer_with_operations(
        operations: Vec<TimerOperation>,
        (interval_ns, start_time_ns, stop_time_ns, fire_immediately): (u64, u64, Option<u64>, bool),
    ) {
        let mut timer = TestTimer::new(
            Ustr::from("PROP_TEST_TIMER"),
            NonZeroU64::new(interval_ns).unwrap(),
            UnixNanos::from(start_time_ns),
            stop_time_ns.map(UnixNanos::from),
            fire_immediately,
        );

        let mut current_time = start_time_ns;

        for operation in operations {
            if timer.is_expired() {
                break;
            }

            match operation {
                TimerOperation::AdvanceTime(delta) => {
                    let to_time = current_time + delta;
                    let events: Vec<TimeEvent> = timer.advance(UnixNanos::from(to_time)).collect();
                    current_time = to_time;

                    // Verify event ordering and timing
                    for (i, event) in events.iter().enumerate() {
                        // Event timestamps should be in order
                        if i > 0 {
                            assert!(
                                event.ts_event >= events[i - 1].ts_event,
                                "Events should be in chronological order"
                            );
                        }

                        // Event timestamp should be within reasonable bounds
                        assert!(
                            event.ts_event.as_u64() >= start_time_ns,
                            "Event timestamp should not be before start time"
                        );

                        assert!(
                            event.ts_event.as_u64() <= to_time,
                            "Event timestamp should not be after advance time"
                        );

                        // If there's a stop time, event should not exceed it
                        if let Some(stop_time_ns) = stop_time_ns {
                            assert!(
                                event.ts_event.as_u64() <= stop_time_ns,
                                "Event timestamp should not exceed stop time"
                            );
                        }
                    }
                }
                TimerOperation::Cancel => {
                    timer.cancel();
                    assert!(timer.is_expired(), "Timer should be expired after cancel");
                }
            }

            // Timer invariants
            if !timer.is_expired() {
                // Next time should be properly spaced
                let expected_interval_multiple = if fire_immediately {
                    timer.next_time_ns().as_u64() >= start_time_ns
                } else {
                    timer.next_time_ns().as_u64() >= start_time_ns + interval_ns
                };
                assert!(
                    expected_interval_multiple,
                    "Next time should respect interval spacing"
                );

                // If timer has stop time, check if it should be considered logically expired
                // Note: Timer only becomes actually expired when advance() or next() is called
                if let Some(stop_time_ns) = stop_time_ns
                    && timer.next_time_ns().as_u64() > stop_time_ns
                {
                    // The timer should expire on the next advance/iteration
                    let mut test_timer = timer.clone();
                    let events: Vec<TimeEvent> = test_timer
                        .advance(UnixNanos::from(stop_time_ns + 1))
                        .collect();
                    assert!(
                        events.is_empty() || test_timer.is_expired(),
                        "Timer should not generate events beyond stop time"
                    );
                }
            }
        }

        // Final consistency check: if timer is not expired and we haven't hit stop time,
        // advancing far enough should eventually expire it
        if !timer.is_expired()
            && let Some(stop_time_ns) = stop_time_ns
        {
            let events: Vec<TimeEvent> = timer
                .advance(UnixNanos::from(stop_time_ns + 1000))
                .collect();
            assert!(
                timer.is_expired() || events.is_empty(),
                "Timer should eventually expire or stop generating events"
            );
        }
    }

    proptest! {
        #[rstest]
        fn prop_timer_advance_operations((operations, config) in timer_test_strategy()) {
            test_timer_with_operations(operations, config);
        }

        #[rstest]
        fn prop_timer_interval_consistency(
            interval_ns in 1u64..=100u64,
            start_time_ns in 0u64..=50u64,
            fire_immediately in prop::bool::ANY,
            advance_count in 1usize..=20usize,
        ) {
            let mut timer = TestTimer::new(
                Ustr::from("CONSISTENCY_TEST"),
                NonZeroU64::new(interval_ns).unwrap(),
                UnixNanos::from(start_time_ns),
                None, // No stop time for this test
                fire_immediately,
            );

            let mut previous_event_time = if fire_immediately { start_time_ns } else { start_time_ns + interval_ns };

            for _ in 0..advance_count {
                let events: Vec<TimeEvent> = timer.advance(UnixNanos::from(previous_event_time)).collect();

                if !events.is_empty() {
                    // Should get exactly one event at the expected time
                    prop_assert_eq!(events.len(), 1);
                    prop_assert_eq!(events[0].ts_event.as_u64(), previous_event_time);
                }

                previous_event_time += interval_ns;
            }
        }
    }
}
