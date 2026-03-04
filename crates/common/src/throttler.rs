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

//! 消息节流 (Throttling) 和速率限制 (Rate limiting) 功能。
//!
//! 此模块提供节流功能，以控制消息处理速率并防止系统过载。
//! 节流器可以根据配置的速率限制和时间间隔对消息进行缓冲、丢弃或延迟。

use std::{
    any::Any,
    cell::{RefCell, UnsafeCell},
    collections::VecDeque,
    fmt::Debug,
    marker::PhantomData,
    rc::Rc,
};

use nautilus_core::{UnixNanos, correctness::FAILED};
use ustr::Ustr;

use crate::{
    actor::{
        Actor,
        registry::{get_actor_unchecked, register_actor},
    },
    clock::Clock,
    msgbus::{self, Endpoint, Handler, MStr, ShareableMessageHandler},
    timer::{TimeEvent, TimeEventCallback},
};

/// 表示每个时间间隔内的节流限制。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimit {
    pub limit: usize,
    pub interval_ns: u64,
}

impl RateLimit {
    /// 创建一个新的 [`RateLimit`] 实例。
    #[must_use]
    pub const fn new(limit: usize, interval_ns: u64) -> Self {
        Self { limit, interval_ns }
    }
}

/// 节流器通过丢弃或缓冲消息来限制消息速率。
///
/// 节流器接收类型为 T 的消息，并使用类型为 F 的回调来丢弃或处理消息。
pub struct Throttler<T, F> {
    /// 已接收的消息数量。
    pub recv_count: usize,
    /// 已发送的消息数量。
    pub sent_count: usize,
    /// 节流器当前是否正在限制消息速率。
    pub is_limiting: bool,
    /// 在该时间间隔内可以发送的最大消息数量。
    pub limit: usize,
    /// 待发送的消息缓冲区。
    pub buffer: VecDeque<T>,
    /// 已发送消息的时间戳。
    pub timestamps: VecDeque<UnixNanos>,
    /// 用于跟踪时间的时钟。
    pub clock: Rc<RefCell<dyn Clock>>,
    /// 节流器的 Actor ID。
    pub actor_id: Ustr,
    /// 消息之间的时间间隔（以纳秒为单位）。
    interval: u64,
    /// 定时器的名称。
    timer_name: Ustr,
    /// 发送消息的回调。
    output_send: F,
    /// 丢弃消息的回调。
    output_drop: Option<F>,
}

impl<T, F> Actor for Throttler<T, F>
where
    T: 'static + Debug,
    F: Fn(T) + 'static,
{
    fn id(&self) -> Ustr {
        self.actor_id
    }

    fn handle(&mut self, _msg: &dyn Any) {}

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl<T, F> Debug for Throttler<T, F>
where
    T: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(InnerThrottler))
            .field("recv_count", &self.recv_count)
            .field("sent_count", &self.sent_count)
            .field("is_limiting", &self.is_limiting)
            .field("limit", &self.limit)
            .field("buffer", &self.buffer)
            .field("timestamps", &self.timestamps)
            .field("interval", &self.interval)
            .field("timer_name", &self.timer_name)
            .finish()
    }
}

impl<T, F> Throttler<T, F>
where
    T: Debug,
{
    #[inline]
    pub fn new(
        limit: usize,
        interval: u64,
        clock: Rc<RefCell<dyn Clock>>,
        timer_name: String,
        output_send: F,
        output_drop: Option<F>,
        actor_id: Ustr,
    ) -> Self {
        Self {
            recv_count: 0,
            sent_count: 0,
            is_limiting: false,
            limit,
            buffer: VecDeque::new(),
            timestamps: VecDeque::with_capacity(limit),
            clock,
            interval,
            timer_name: Ustr::from(&timer_name),
            output_send,
            output_drop,
            actor_id,
        }
    }

    /// 在下一个时间间隔触发时设置带有回调的定时器。
    ///
    /// 通常用于注册回调：
    /// - 处理缓冲的消息
    /// - 停止缓冲
    ///
    /// # Panics
    ///
    /// 如果在内部时钟上设置时间警报失败，则会抛出 panic。
    #[inline]
    pub fn set_timer(&mut self, callback: Option<TimeEventCallback>) {
        let delta = self.delta_next();
        let mut clock = self.clock.borrow_mut();
        if clock.timer_exists(&self.timer_name) {
            clock.cancel_timer(&self.timer_name);
        }
        let alert_ts = clock.timestamp_ns() + delta;

        clock
            .set_time_alert_ns(&self.timer_name, alert_ts, callback, None)
            .expect(FAILED);
    }

    /// 可以发送下一条消息的时间增量。
    #[inline]
    pub fn delta_next(&mut self) -> u64 {
        match self.timestamps.get(self.limit - 1) {
            Some(ts) => {
                let diff = self.clock.borrow().timestamp_ns().as_u64() - ts.as_u64();
                self.interval.saturating_sub(diff)
            }
            None => 0,
        }
    }

    /// 重置节流器，清除内部状态。
    #[inline]
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.recv_count = 0;
        self.sent_count = 0;
        self.is_limiting = false;
        self.timestamps.clear();
    }

    /// 当前时间间隔内已消耗的速率限制比例值。
    #[inline]
    pub fn used(&self) -> f64 {
        if self.timestamps.is_empty() {
            return 0.0;
        }

        let now = self.clock.borrow().timestamp_ns().as_i64();
        let interval_start = now - self.interval as i64;

        let messages_in_current_interval = self
            .timestamps
            .iter()
            .take_while(|&&ts| ts.as_i64() > interval_start)
            .count();

        (messages_in_current_interval as f64) / (self.limit as f64)
    }

    /// 队列中缓冲的消息数量。
    #[inline]
    pub fn qsize(&self) -> usize {
        self.buffer.len()
    }
}

impl<T, F> Throttler<T, F>
where
    T: 'static + Debug,
    F: Fn(T) + 'static,
{
    pub fn to_actor(self) -> Rc<UnsafeCell<Self>> {
        // Register process endpoint
        let process_handler = ThrottlerProcess::<T, F>::new(self.actor_id);
        msgbus::register_any(
            process_handler.id().as_str().into(),
            ShareableMessageHandler::from(Rc::new(process_handler) as Rc<dyn Handler<dyn Any>>),
        );

        // Register actor state and return the wrapped reference
        register_actor(self)
    }

    #[inline]
    pub fn send_msg(&mut self, msg: T) {
        let now = self.clock.borrow().timestamp_ns();

        if self.timestamps.len() >= self.limit {
            self.timestamps.pop_back();
        }
        self.timestamps.push_front(now);

        self.sent_count += 1;
        (self.output_send)(msg);
    }

    #[inline]
    pub fn limit_msg(&mut self, msg: T) {
        let callback = if self.output_drop.is_none() {
            self.buffer.push_front(msg);
            log::debug!("Buffering {}", self.buffer.len());
            Some(ThrottlerProcess::<T, F>::new(self.actor_id).get_timer_callback())
        } else {
            log::debug!("Dropping");
            if let Some(drop) = &self.output_drop {
                drop(msg);
            }
            Some(throttler_resume::<T, F>(self.actor_id))
        };
        if !self.is_limiting {
            log::debug!("Limiting");
            self.set_timer(callback);
            self.is_limiting = true;
        }
    }

    #[inline]
    pub fn send(&mut self, msg: T)
    where
        T: 'static,
        F: Fn(T) + 'static,
    {
        self.recv_count += 1;

        if self.is_limiting || self.delta_next() > 0 {
            self.limit_msg(msg);
        } else {
            self.send_msg(msg);
        }
    }
}

/// 为节流器处理缓冲的消息
///
/// 当达到限制时，安排一个定时器事件以再次调用自身。
/// 该处理器在消息总线上被注册为一个单独的端点，名称为 `{actor_id}_process`。
struct ThrottlerProcess<T, F> {
    actor_id: Ustr,
    endpoint: MStr<Endpoint>,
    phantom_t: PhantomData<T>,
    phantom_f: PhantomData<F>,
}

impl<T, F> ThrottlerProcess<T, F>
where
    T: Debug,
{
    pub fn new(actor_id: Ustr) -> Self {
        let endpoint = MStr::endpoint(format!("{actor_id}_process")).expect(FAILED);
        Self {
            actor_id,
            endpoint,
            phantom_t: PhantomData,
            phantom_f: PhantomData,
        }
    }

    pub fn get_timer_callback(&self) -> TimeEventCallback {
        let endpoint = self.endpoint;
        TimeEventCallback::from(move |event: TimeEvent| {
            msgbus::send_any(endpoint, &(event));
        })
    }
}

impl<T, F> Handler<dyn Any> for ThrottlerProcess<T, F>
where
    T: 'static + Debug,
    F: Fn(T) + 'static,
{
    fn id(&self) -> Ustr {
        *self.endpoint
    }

    fn handle(&self, _message: &dyn Any) {
        let mut throttler = get_actor_unchecked::<Throttler<T, F>>(&self.actor_id);
        while let Some(msg) = throttler.buffer.pop_back() {
            throttler.send_msg(msg);

            // 如果达到间隔限制且还有更多缓冲消息待处理，
            // 则设置定时器以处理更多缓冲消息。
            if !throttler.buffer.is_empty() && throttler.delta_next() > 0 {
                throttler.is_limiting = true;

                let endpoint = self.endpoint;

                // 发送消息到节流器处理端点以恢复发送动作
                throttler.set_timer(Some(TimeEventCallback::from(move |event: TimeEvent| {
                    msgbus::send_any(endpoint, &(event));
                })));
                return;
            }
        }

        throttler.is_limiting = false;
    }
}

/// 设置节流器恢复发送消息。
pub fn throttler_resume<T, F>(actor_id: Ustr) -> TimeEventCallback
where
    T: 'static + Debug,
    F: Fn(T) + 'static,
{
    TimeEventCallback::from(move |_event: TimeEvent| {
        let mut throttler = get_actor_unchecked::<Throttler<T, F>>(&actor_id);
        throttler.is_limiting = false;
    })
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{RefCell, UnsafeCell},
        rc::Rc,
    };

    use nautilus_core::UUID4;
    use rstest::{fixture, rstest};
    use ustr::Ustr;

    use super::{RateLimit, Throttler, ThrottlerProcess};
    use crate::{clock::TestClock, msgbus::Handler};
    type SharedThrottler = Rc<UnsafeCell<Throttler<u64, Box<dyn Fn(u64)>>>>;

    /// Test throttler with default values for testing
    ///
    /// - Rate limit is 5 messages in 10 intervals.
    /// - Message handling is decided by specific fixture
    #[derive(Clone)]
    struct TestThrottler {
        throttler: SharedThrottler,
        clock: Rc<RefCell<TestClock>>,
        interval: u64,
    }

    #[allow(unsafe_code)]
    impl TestThrottler {
        #[allow(clippy::mut_from_ref)]
        pub fn get_throttler(&self) -> &mut Throttler<u64, Box<dyn Fn(u64)>> {
            unsafe { &mut *self.throttler.get() }
        }
    }

    #[fixture]
    pub fn test_throttler_buffered() -> TestThrottler {
        let output_send: Box<dyn Fn(u64)> = Box::new(|msg: u64| {
            log::debug!("Sent: {msg}");
        });
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let inner_clock = Rc::clone(&clock);
        let rate_limit = RateLimit::new(5, 10);
        let interval = rate_limit.interval_ns;
        let actor_id = Ustr::from(UUID4::new().as_str());

        TestThrottler {
            throttler: Throttler::new(
                rate_limit.limit,
                rate_limit.interval_ns,
                clock,
                "buffer_timer".to_string(),
                output_send,
                None,
                actor_id,
            )
            .to_actor(),
            clock: inner_clock,
            interval,
        }
    }

    #[fixture]
    pub fn test_throttler_unbuffered() -> TestThrottler {
        let output_send: Box<dyn Fn(u64)> = Box::new(|msg: u64| {
            log::debug!("Sent: {msg}");
        });
        let output_drop: Box<dyn Fn(u64)> = Box::new(|msg: u64| {
            log::debug!("Dropped: {msg}");
        });
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let inner_clock = Rc::clone(&clock);
        let rate_limit = RateLimit::new(5, 10);
        let interval = rate_limit.interval_ns;
        let actor_id = Ustr::from(UUID4::new().as_str());

        TestThrottler {
            throttler: Throttler::new(
                rate_limit.limit,
                rate_limit.interval_ns,
                clock,
                "dropper_timer".to_string(),
                output_send,
                Some(output_drop),
                actor_id,
            )
            .to_actor(),
            clock: inner_clock,
            interval,
        }
    }

    #[rstest]
    fn test_buffering_send_to_limit_becomes_throttled(test_throttler_buffered: TestThrottler) {
        let throttler = test_throttler_buffered.get_throttler();
        for _ in 0..6 {
            throttler.send(42);
        }
        assert_eq!(throttler.qsize(), 1);

        assert!(throttler.is_limiting);
        assert_eq!(throttler.recv_count, 6);
        assert_eq!(throttler.sent_count, 5);
        assert_eq!(throttler.clock.borrow().timer_names(), vec!["buffer_timer"]);
    }

    #[rstest]
    fn test_buffering_used_when_sent_to_limit_returns_one(test_throttler_buffered: TestThrottler) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..5 {
            throttler.send(42);
        }

        assert_eq!(throttler.used(), 1.0);
        assert_eq!(throttler.recv_count, 5);
        assert_eq!(throttler.sent_count, 5);
    }

    #[rstest]
    fn test_buffering_used_when_half_interval_from_limit_returns_one(
        test_throttler_buffered: TestThrottler,
    ) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..5 {
            throttler.send(42);
        }

        let half_interval = test_throttler_buffered.interval / 2;
        // Advance the clock by half the interval
        {
            let mut clock = test_throttler_buffered.clock.borrow_mut();
            clock.advance_time(half_interval.into(), true);
        }

        assert_eq!(throttler.used(), 1.0);
        assert_eq!(throttler.recv_count, 5);
        assert_eq!(throttler.sent_count, 5);
    }

    #[rstest]
    fn test_buffering_used_before_limit_when_halfway_returns_half(
        test_throttler_buffered: TestThrottler,
    ) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..3 {
            throttler.send(42);
        }

        assert_eq!(throttler.used(), 0.6);
        assert_eq!(throttler.recv_count, 3);
        assert_eq!(throttler.sent_count, 3);
    }

    #[rstest]
    fn test_buffering_refresh_when_at_limit_sends_remaining_items(
        test_throttler_buffered: TestThrottler,
    ) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..6 {
            throttler.send(42);
        }

        // Advance time and process events
        {
            let mut clock = test_throttler_buffered.clock.borrow_mut();
            let time_events = clock.advance_time(test_throttler_buffered.interval.into(), true);
            for each_event in clock.match_handlers(time_events) {
                drop(clock); // Release the mutable borrow

                each_event.callback.call(each_event.event);

                // Re-borrow the clock for the next iteration
                clock = test_throttler_buffered.clock.borrow_mut();
            }
        }

        // Assert final state
        assert_eq!(throttler.used(), 0.2);
        assert_eq!(throttler.recv_count, 6);
        assert_eq!(throttler.sent_count, 6);
        assert_eq!(throttler.qsize(), 0);
    }

    #[rstest]
    fn test_buffering_send_message_after_buffering_message(test_throttler_buffered: TestThrottler) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..6 {
            throttler.send(43);
        }

        // Advance time and process events
        {
            let mut clock = test_throttler_buffered.clock.borrow_mut();
            let time_events = clock.advance_time(test_throttler_buffered.interval.into(), true);
            for each_event in clock.match_handlers(time_events) {
                drop(clock); // Release the mutable borrow

                each_event.callback.call(each_event.event);

                // Re-borrow the clock for the next iteration
                clock = test_throttler_buffered.clock.borrow_mut();
            }
        }

        for _ in 0..6 {
            throttler.send(42);
        }

        // Assert final state
        assert_eq!(throttler.used(), 1.0);
        assert_eq!(throttler.recv_count, 12);
        assert_eq!(throttler.sent_count, 10);
        assert_eq!(throttler.qsize(), 2);
    }

    #[rstest]
    fn test_buffering_send_message_after_halfway_after_buffering_message(
        test_throttler_buffered: TestThrottler,
    ) {
        let throttler = test_throttler_buffered.get_throttler();

        for _ in 0..6 {
            throttler.send(42);
        }

        // Advance time and process events
        {
            let mut clock = test_throttler_buffered.clock.borrow_mut();
            let time_events = clock.advance_time(test_throttler_buffered.interval.into(), true);
            for each_event in clock.match_handlers(time_events) {
                drop(clock); // Release the mutable borrow

                each_event.callback.call(each_event.event);

                // Re-borrow the clock for the next iteration
                clock = test_throttler_buffered.clock.borrow_mut();
            }
        }

        for _ in 0..3 {
            throttler.send(42);
        }

        // Assert final state
        assert_eq!(throttler.used(), 0.8);
        assert_eq!(throttler.recv_count, 9);
        assert_eq!(throttler.sent_count, 9);
        assert_eq!(throttler.qsize(), 0);
    }

    #[rstest]
    fn test_dropping_send_sends_message_to_handler(test_throttler_unbuffered: TestThrottler) {
        let throttler = test_throttler_unbuffered.get_throttler();
        throttler.send(42);

        assert!(!throttler.is_limiting);
        assert_eq!(throttler.recv_count, 1);
        assert_eq!(throttler.sent_count, 1);
    }

    #[rstest]
    fn test_dropping_send_to_limit_drops_message(test_throttler_unbuffered: TestThrottler) {
        let throttler = test_throttler_unbuffered.get_throttler();
        for _ in 0..6 {
            throttler.send(42);
        }
        assert_eq!(throttler.qsize(), 0);

        assert!(throttler.is_limiting);
        assert_eq!(throttler.used(), 1.0);
        assert_eq!(throttler.clock.borrow().timer_count(), 1);
        assert_eq!(
            throttler.clock.borrow().timer_names(),
            vec!["dropper_timer"]
        );
        assert_eq!(throttler.recv_count, 6);
        assert_eq!(throttler.sent_count, 5);
    }

    #[rstest]
    fn test_dropping_advance_time_when_at_limit_dropped_message(
        test_throttler_unbuffered: TestThrottler,
    ) {
        let throttler = test_throttler_unbuffered.get_throttler();
        for _ in 0..6 {
            throttler.send(42);
        }

        // Advance time and process events
        {
            let mut clock = test_throttler_unbuffered.clock.borrow_mut();
            let time_events = clock.advance_time(test_throttler_unbuffered.interval.into(), true);
            for each_event in clock.match_handlers(time_events) {
                drop(clock); // Release the mutable borrow

                each_event.callback.call(each_event.event);

                // Re-borrow the clock for the next iteration
                clock = test_throttler_unbuffered.clock.borrow_mut();
            }
        }

        assert_eq!(throttler.clock.borrow().timer_count(), 0);
        assert!(!throttler.is_limiting);
        assert_eq!(throttler.used(), 0.0);
        assert_eq!(throttler.recv_count, 6);
        assert_eq!(throttler.sent_count, 5);
    }

    #[rstest]
    fn test_dropping_send_message_after_dropping_message(test_throttler_unbuffered: TestThrottler) {
        let throttler = test_throttler_unbuffered.get_throttler();
        for _ in 0..6 {
            throttler.send(42);
        }

        // Advance time and process events
        {
            let mut clock = test_throttler_unbuffered.clock.borrow_mut();
            let time_events = clock.advance_time(test_throttler_unbuffered.interval.into(), true);
            for each_event in clock.match_handlers(time_events) {
                drop(clock); // Release the mutable borrow

                each_event.callback.call(each_event.event);

                // Re-borrow the clock for the next iteration
                clock = test_throttler_unbuffered.clock.borrow_mut();
            }
        }

        throttler.send(42);

        assert_eq!(throttler.used(), 0.2);
        assert_eq!(throttler.clock.borrow().timer_count(), 0);
        assert!(!throttler.is_limiting);
        assert_eq!(throttler.recv_count, 7);
        assert_eq!(throttler.sent_count, 6);
    }

    ////////////////////////////////////////////////////////////////////////////////
    // Property-based testing
    ////////////////////////////////////////////////////////////////////////////////

    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum ThrottlerInput {
        SendMessage(u64),
        AdvanceClock(u8),
    }

    // Custom strategy for ThrottlerInput
    fn throttler_input_strategy() -> impl Strategy<Value = ThrottlerInput> {
        prop_oneof![
            2 => prop::bool::ANY.prop_map(|_| ThrottlerInput::SendMessage(42)),
            8 => prop::num::u8::ANY.prop_map(|v| ThrottlerInput::AdvanceClock(v % 5 + 5)),
        ]
    }

    // Custom strategy for ThrottlerTest
    fn throttler_test_strategy() -> impl Strategy<Value = Vec<ThrottlerInput>> {
        prop::collection::vec(throttler_input_strategy(), 10..=150)
    }

    fn test_throttler_with_inputs(inputs: Vec<ThrottlerInput>, test_throttler: TestThrottler) {
        let test_clock = test_throttler.clock.clone();
        let interval = test_throttler.interval;
        let throttler = test_throttler.get_throttler();
        let mut sent_count = 0;

        for input in inputs {
            match input {
                ThrottlerInput::SendMessage(msg) => {
                    throttler.send(msg);
                    sent_count += 1;
                }
                ThrottlerInput::AdvanceClock(duration) => {
                    let mut clock_ref = test_clock.borrow_mut();
                    let current_time = clock_ref.get_time_ns();
                    let time_events =
                        clock_ref.advance_time(current_time + u64::from(duration), true);
                    for each_event in clock_ref.match_handlers(time_events) {
                        drop(clock_ref);
                        each_event.callback.call(each_event.event);
                        clock_ref = test_clock.borrow_mut();
                    }
                }
            }

            // Check the throttler rate limits on the appropriate conditions
            // * At least one message is buffered
            // * Timestamp queue is filled upto limit
            // * Least recent timestamp in queue exceeds interval
            let buffered_messages = throttler.qsize() > 0;
            let now = throttler.clock.borrow().timestamp_ns().as_u64();
            let limit_filled_within_interval = throttler
                .timestamps
                .get(throttler.limit - 1)
                .is_some_and(|&ts| (now - ts.as_u64()) < interval);
            let expected_limiting = buffered_messages && limit_filled_within_interval;
            assert_eq!(throttler.is_limiting, expected_limiting);

            // Message conservation
            assert_eq!(sent_count, throttler.sent_count + throttler.qsize());
        }

        // Advance clock by a large amount to process all messages
        let time_events = test_clock
            .borrow_mut()
            .advance_time((interval * 100).into(), true);
        let mut clock_ref = test_clock.borrow_mut();
        for each_event in clock_ref.match_handlers(time_events) {
            drop(clock_ref);
            each_event.callback.call(each_event.event);
            clock_ref = test_clock.borrow_mut();
        }
        assert_eq!(throttler.qsize(), 0);
    }

    #[rstest]
    fn prop_test() {
        // Create a fresh throttler for each iteration to ensure clean state,
        // even when tests panic (which would skip the reset code)
        proptest!(|(inputs in throttler_test_strategy())| {
            let test_throttler = test_throttler_buffered();
            test_throttler_with_inputs(inputs, test_throttler);
        });
    }

    #[rstest]
    fn test_throttler_process_id_returns_ustr() {
        // This test verifies that ThrottlerProcess::id() correctly returns Ustr
        // by dereferencing MStr<Endpoint> (tests *self.endpoint -> Ustr conversion)
        let actor_id = Ustr::from("test_throttler");
        let process = ThrottlerProcess::<String, fn(String)>::new(actor_id);

        // Call id() which does *self.endpoint
        let handler_id: Ustr = process.id();

        // Verify it's a valid Ustr with expected format
        assert!(handler_id.as_str().contains("test_throttler_process"));
        assert!(!handler_id.is_empty());

        // Verify type - this wouldn't compile if id() didn't return Ustr
        let _type_check: Ustr = handler_id;
    }
}
