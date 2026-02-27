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

//! 一个 `UnixNanos` 类型，用于处理自 UNIX 纪元以来的纳秒级时间戳。
//!
//! 此模块提供了自 UNIX 纪元（1970 年 1 月 1 日，00:00:00 UTC）以来的纳秒时间戳的强类型表示。
//! `UnixNanos` 类型提供了转换工具、算术运算和比较方法。
//!
//! # 特性
//!
//! - 带有所需运算符实现的零成本抽象。
//! - 与 `DateTime<Utc>` 之间的相互转换。
//! - RFC 3339 字符串格式化。
//! - 持续时间计算。
//! - 灵活的解析和序列化。
//!
//! # 解析和序列化
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]
//!
//! `UnixNanos` 可以从多种格式创建，并序列化为以下格式：
//!
//! * 整数值被解释为自 UNIX 纪元以来的纳秒数。
//! * 浮点值被解释为自 UNIX 纪元以来的秒数（使用截断而非舍入将其转换为纳秒，
//!   以保持与 [`secs_to_nanos`](crate::datetime::secs_to_nanos) 的一致性）。
//! * 字符串值可以是：
//!   - 数字字符串（被解释为纳秒）。
//!   - 浮点数字符串（被解释为秒，并转换为纳秒）。
//!   - RFC 3339 格式的时间戳（带有时区的 ISO 8601）。
//!   - YYYY-MM-DD 格式的简单日期字符串（被解释为该日期的 UTC 午夜）。
//!
//! # 限制
//!
//! * 负数时间戳无效，将导致错误。
//! * 算术运算在发生溢出/下溢时将触发 panic，而不是采用回绕处理。
//! * 当时间戳在大约 2262 年之后（此时纳秒数将超过 `i64::MAX`）时，
//!   `as_i64()` 方法和 `DateTime<Utc>` 转换将触发 panic。

use std::{
    cmp::Ordering,
    fmt::Display,
    ops::{Add, AddAssign, Deref, Sub, SubAssign},
    str::FromStr,
    time::SystemTime,
};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, Visitor},
};

/// 代表纳秒级的持续时间。
pub type DurationNanos = u64;

/// 代表自 UNIX 纪元以来纳秒级的时间戳。
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct UnixNanos(u64);

impl UnixNanos {
    /// 创建一个新的 [`UnixNanos`] 实例。
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// 创建一个具有最大有效值的 [`UnixNanos`] 实例。
    #[must_use]
    pub const fn max() -> Self {
        Self(u64::MAX)
    }

    /// 如果此实例的值为零，则返回 `true`。
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// 以 `u64` 类型返回底层值。
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        self.0
    }

    /// 以 `i64` 类型返回底层值。
    ///
    /// # Panics
    ///
    /// 如果值超过 `i64::MAX`（大约在 2262 年），则触发 panic。
    #[must_use]
    pub const fn as_i64(&self) -> i64 {
        assert!(
            self.0 <= i64::MAX as u64,
            "UnixNanos 超过了 i64::MAX"
        );
        self.0 as i64
    }

    /// 以 `f64` 类型返回底层值。
    #[must_use]
    pub const fn as_f64(&self) -> f64 {
        self.0 as f64
    }

    /// 将底层值转换为日期时间 (UTC)。
    ///
    /// # Panics
    ///
    /// 如果值超过 `i64::MAX`（大约在 2262 年），则触发 panic。
    #[must_use]
    pub fn to_datetime_utc(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_nanos(self.as_i64())
    }

    /// 将底层值转换为 ISO 8601 (RFC 3339) 格式字符串。
    #[must_use]
    pub fn to_rfc3339(&self) -> String {
        self.to_datetime_utc().to_rfc3339()
    }

    /// 计算自另一个 [`UnixNanos`] 实例以来的纳秒级持续时间。
    ///
    /// 如果 `self` 晚于 `other`，返回 `Some(duration)`；否则如果 `other` 大于 `self` 则返回 `None`
    /// （表示 `DurationNanos` 不可能表示负的持续时间）。
    #[must_use]
    pub const fn duration_since(&self, other: &Self) -> Option<DurationNanos> {
        self.0.checked_sub(other.0)
    }

    fn parse_string(s: &str) -> Result<Self, String> {
        const MAX_NS_F64: f64 = u64::MAX as f64;

        // 尝试解析为整数（纳秒）
        if let Ok(int_value) = s.parse::<u64>() {
            return Ok(Self(int_value));
        }

        // 如果字符串完全由数字组成但无法装入 u64，则将其视为溢出错误，
        // 而不是尝试将其解释为浮点数形式的秒。这避免了调用者提供了纳秒，
        // 但却意外地得到了一个超出范围的浮点数解释。
        if s.chars().all(|c| c.is_ascii_digit()) {
            return Err("Unix 时间戳超出范围".into());
        }

        // 尝试解析为浮点数（秒）
        if let Ok(float_value) = s.parse::<f64>() {
            if !float_value.is_finite() {
                return Err("Unix 时间戳必须是有限值".into());
            }

            if float_value < 0.0 {
                return Err("Unix 时间戳不能为负".into());
            }

            // 将秒转换为纳秒并检查是否溢出
            // 我们在 `f64` 中进行乘法运算，然后在进行舍入/类型转换之前校验结果是否符合 `u64`。
            let nanos_f64 = float_value * 1_000_000_000.0;

            if nanos_f64 > MAX_NS_F64 {
                return Err("Unix 时间戳超出范围".into());
            }

            let nanos = nanos_f64.trunc() as u64;
            return Ok(Self(nanos));
        }

        // 尝试解析为 RFC 3339 时间戳
        if let Ok(datetime) = DateTime::parse_from_rfc3339(s) {
            let nanos = datetime
                .timestamp_nanos_opt()
                .ok_or_else(|| "时间戳超出范围".to_string())?;

            if nanos < 0 {
                return Err("Unix 时间戳不能为负".into());
            }

            // 安全性：已检查 nanos >= 0，因此强制转换为 u64 是安全的
            return Ok(Self(nanos as u64));
        }

        // 尝试解析为简单日期字符串 (YYYY-MM-DD 格式)
        if let Ok(datetime) = NaiveDate::parse_from_str(s, "%Y-%m-%d")
            // 安全性：此处使用 unwrap() 是安全的，因为 对于有效日期，and_hms_opt(0, 0, 0) 总会成功（午夜通常是有效时间）
            .map(|date| date.and_hms_opt(0, 0, 0).unwrap())
            .map(|naive_dt| DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc))
        {
            let nanos = datetime
                .timestamp_nanos_opt()
                .ok_or_else(|| "时间戳超出范围".to_string())?;
            if nanos < 0 {
                return Err("Unix 时间戳不能为负".into());
            }
            return Ok(Self(nanos as u64));
        }

        Err(format!("无效格式: {s}"))
    }

    /// 返回 `Some(self + rhs)`，如果加法导致溢出则返回 `None`。
    #[must_use]
    pub fn checked_add<T: Into<u64>>(self, rhs: T) -> Option<Self> {
        self.0.checked_add(rhs.into()).map(Self)
    }

    /// 返回 `Some(self - rhs)`，如果减法导致下溢则返回 `None`。
    #[must_use]
    pub fn checked_sub<T: Into<u64>>(self, rhs: T) -> Option<Self> {
        self.0.checked_sub(rhs.into()).map(Self)
    }

    /// 饱和加法 – 如果发生溢出，值将被限制在 `u64::MAX`。
    #[must_use]
    pub fn saturating_add_ns<T: Into<u64>>(self, rhs: T) -> Self {
        Self(self.0.saturating_add(rhs.into()))
    }

    /// 饱和减法 – 如果发生下溢，值将被限制在 `0`。
    #[must_use]
    pub fn saturating_sub_ns<T: Into<u64>>(self, rhs: T) -> Self {
        Self(self.0.saturating_sub(rhs.into()))
    }
}

impl Deref for UnixNanos {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<u64> for UnixNanos {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<u64> for UnixNanos {
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        self.0.partial_cmp(other)
    }
}

impl PartialEq<Option<u64>> for UnixNanos {
    fn eq(&self, other: &Option<u64>) -> bool {
        match other {
            Some(value) => self.0 == *value,
            None => false,
        }
    }
}

impl PartialOrd<Option<u64>> for UnixNanos {
    fn partial_cmp(&self, other: &Option<u64>) -> Option<Ordering> {
        match other {
            Some(value) => self.0.partial_cmp(value),
            None => Some(Ordering::Greater),
        }
    }
}

impl PartialEq<UnixNanos> for u64 {
    fn eq(&self, other: &UnixNanos) -> bool {
        *self == other.0
    }
}

impl PartialOrd<UnixNanos> for u64 {
    fn partial_cmp(&self, other: &UnixNanos) -> Option<Ordering> {
        self.partial_cmp(&other.0)
    }
}

impl From<u64> for UnixNanos {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<UnixNanos> for u64 {
    fn from(value: UnixNanos) -> Self {
        value.0
    }
}

/// 将字符串切片转换为 [`UnixNanos`]。
///
/// # Panics
///
/// 如果字符串无法解析为有效的 [`UnixNanos`]，此实现将触发 panic。
/// 这是有意设计的快速失败行为：无效的时间戳表明出现了严重逻辑错误，应当停止执行，而不是静默地传播错误数据。
///
/// 若需无 panic 的错误处理，请使用返回 [`Result`] 的 [`str::parse::<UnixNanos>()`]。
impl From<&str> for UnixNanos {
    fn from(value: &str) -> Self {
        value
            .parse()
            .unwrap_or_else(|e| panic!("无法将字符串 '{value}' 解析为 UnixNanos: {e}。请使用 str::parse() 进行无 panic 的错误处理。"))
    }
}

/// 将 [`String`] 转换为 [`UnixNanos`]。
///
/// # Panics
///
/// 如果字符串无法解析为有效的 [`UnixNanos`]，此实现将触发 panic。
/// 这是有意设计的快速失败行为：无效的时间戳表明出现了严重逻辑错误，应当停止执行，而不是静默地传播错误数据。
///
/// 若需无 panic 的错误处理，请使用返回 [`Result`] 的 [`str::parse::<UnixNanos>()`]。
impl From<String> for UnixNanos {
    fn from(value: String) -> Self {
        value
            .parse()
            .unwrap_or_else(|e| panic!("无法将字符串 '{value}' 解析为 UnixNanos: {e}。请使用 str::parse() 进行无 panic 的错误处理。"))
    }
}

impl From<DateTime<Utc>> for UnixNanos {
    fn from(value: DateTime<Utc>) -> Self {
        let nanos = value
            .timestamp_nanos_opt()
            .expect("DateTime 时间戳由于超出 UnixNanos 范围而无法转换");

        assert!(nanos >= 0, "DateTime 时间戳不能为负: {nanos}");

        Self::from(nanos as u64)
    }
}

impl From<SystemTime> for UnixNanos {
    fn from(value: SystemTime) -> Self {
        let duration = value
            .duration_since(std::time::UNIX_EPOCH)
            .expect("SystemTime 晚于 UNIX 纪元 (EPOCH)");

        let nanos = duration.as_nanos();
        assert!(
            nanos <= u64::MAX as u128,
            "SystemTime 纳秒数溢出了 u64"
        );

        Self::from(nanos as u64)
    }
}

impl FromStr for UnixNanos {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_string(s).map_err(std::convert::Into::into)
    }
}

/// 将两个 [`UnixNanos`] 值相加。
///
/// # Panics
///
/// 发生溢出时触发 panic。这是有意设计的快速失败行为：时间戳算术中的溢出表明计算中存在逻辑错误，会导致数据损坏。
/// 如果需要显式的溢出处理，请使用 [`UnixNanos::checked_add()`] 或 [`UnixNanos::saturating_add_ns()`]。
impl Add for UnixNanos {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(
            self.0
                .checked_add(rhs.0)
                .expect("UnixNanos 加法溢出 - 无效的时间戳计算"),
        )
    }
}

/// 从一个 [`UnixNanos`] 中减去另一个。
///
/// # Panics
///
/// 发生下溢时触发 panic。这是有意设计的快速失败行为：时间戳算术中的下溢表明计算中存在逻辑错误，会导致数据损坏。
/// 如果需要显式的下溢处理，请使用 [`UnixNanos::checked_sub()`] 或 [`UnixNanos::saturating_sub_ns()`]。
impl Sub for UnixNanos {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(
            self.0
                .checked_sub(rhs.0)
                .expect("UnixNanos 减法下溢 - 无效的时间戳计算"),
        )
    }
}

/// 向 [`UnixNanos`] 增加一个以 `u64` 表示的纳秒值。
///
/// # Panics
///
/// 发生溢出时触发 panic。这是针对时间戳算术有意设计的快速失败行为。
/// 若需显式的溢出处理，请使用 [`UnixNanos::checked_add()`]。
impl Add<u64> for UnixNanos {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Self(
            self.0
                .checked_add(rhs)
                .expect("UnixNanos 加法溢出"),
        )
    }
}

/// 从 [`UnixNanos`] 中减去一个以 `u64` 表示的纳秒值。
///
/// # Panics
///
/// 发生下溢时触发 panic。这是针对时间戳算术有意设计的快速失败行为。
/// 若需显式的下溢处理，请使用 [`UnixNanos::checked_sub()`]。
impl Sub<u64> for UnixNanos {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self::Output {
        Self(
            self.0
                .checked_sub(rhs)
                .expect("UnixNanos 减法下溢"),
        )
    }
}

/// 通过加法赋值向 [`UnixNanos`] 增加一个值。
///
/// # Panics
///
/// 发生溢出时触发 panic。这是针对时间戳算术有意设计的快速失败行为。
impl<T: Into<u64>> AddAssign<T> for UnixNanos {
    fn add_assign(&mut self, other: T) {
        let other_u64 = other.into();
        self.0 = self
            .0
            .checked_add(other_u64)
            .expect("UnixNanos 加法赋值溢出 (add_assign)");
    }
}

/// 通过减法赋值从 [`UnixNanos`] 中减去一个值。
///
/// # Panics
///
/// 发生下溢时触发 panic。这是针对时间戳算术有意设计的快速失败行为。
impl<T: Into<u64>> SubAssign<T> for UnixNanos {
    fn sub_assign(&mut self, other: T) {
        let other_u64 = other.into();
        self.0 = self
            .0
            .checked_sub(other_u64)
            .expect("UnixNanos 减法赋值下溢 (sub_assign)");
    }
}

impl Display for UnixNanos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<UnixNanos> for DateTime<Utc> {
    fn from(value: UnixNanos) -> Self {
        value.to_datetime_utc()
    }
}

impl<'de> Deserialize<'de> for UnixNanos {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UnixNanosVisitor;

        impl Visitor<'_> for UnixNanosVisitor {
            type Value = UnixNanos;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("一个整数、一个字符串型的整数或一个 RFC 3339 时间戳")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(UnixNanos(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::custom("Unix 时间戳不能为负"));
                }
                Ok(UnixNanos(value as u64))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                const MAX_NS_F64: f64 = u64::MAX as f64;

                if !value.is_finite() {
                    return Err(E::custom(format!(
                        "Unix 时间戳必须是有限值，实际为 {value}"
                    )));
                }
                if value < 0.0 {
                    return Err(E::custom("Unix 时间戳不能为负"));
                }

                // 将秒转换为纳秒并进行溢出检查
                let nanos_f64 = value * 1_000_000_000.0;
                if nanos_f64 > MAX_NS_F64 {
                    return Err(E::custom(format!(
                        "Unix 时间戳 {value} 秒超出范围"
                    )));
                }
                let nanos = nanos_f64.trunc() as u64;
                Ok(UnixNanos(nanos))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                UnixNanos::parse_string(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_any(UnixNanosVisitor)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_new() {
        let nanos = UnixNanos::new(123);
        assert_eq!(nanos.as_u64(), 123);
        assert_eq!(nanos.as_i64(), 123);
    }

    #[rstest]
    fn test_max() {
        let nanos = UnixNanos::max();
        assert_eq!(nanos.as_u64(), u64::MAX);
    }

    #[rstest]
    fn test_is_zero() {
        assert!(UnixNanos::default().is_zero());
        assert!(!UnixNanos::max().is_zero());
    }

    #[rstest]
    fn test_from_u64() {
        let nanos = UnixNanos::from(123);
        assert_eq!(nanos.as_u64(), 123);
        assert_eq!(nanos.as_i64(), 123);
    }

    #[rstest]
    fn test_default() {
        let nanos = UnixNanos::default();
        assert_eq!(nanos.as_u64(), 0);
        assert_eq!(nanos.as_i64(), 0);
    }

    #[rstest]
    fn test_into_from() {
        let nanos: UnixNanos = 456.into();
        let value: u64 = nanos.into();
        assert_eq!(value, 456);
    }

    #[rstest]
    #[case(0, "1970-01-01T00:00:00+00:00")]
    #[case(1_000_000_000, "1970-01-01T00:00:01+00:00")]
    #[case(1_000_000_000_000_000_000, "2001-09-09T01:46:40+00:00")]
    #[case(1_500_000_000_000_000_000, "2017-07-14T02:40:00+00:00")]
    #[case(1_707_577_123_456_789_000, "2024-02-10T14:58:43.456789+00:00")]
    fn test_to_datetime_utc(#[case] nanos: u64, #[case] expected: &str) {
        let nanos = UnixNanos::from(nanos);
        let datetime = nanos.to_datetime_utc();
        assert_eq!(datetime.to_rfc3339(), expected);
    }

    #[rstest]
    #[case(0, "1970-01-01T00:00:00+00:00")]
    #[case(1_000_000_000, "1970-01-01T00:00:01+00:00")]
    #[case(1_000_000_000_000_000_000, "2001-09-09T01:46:40+00:00")]
    #[case(1_500_000_000_000_000_000, "2017-07-14T02:40:00+00:00")]
    #[case(1_707_577_123_456_789_000, "2024-02-10T14:58:43.456789+00:00")]
    fn test_to_rfc3339(#[case] nanos: u64, #[case] expected: &str) {
        let nanos = UnixNanos::from(nanos);
        assert_eq!(nanos.to_rfc3339(), expected);
    }

    #[rstest]
    fn test_from_str() {
        let nanos: UnixNanos = "123".parse().unwrap();
        assert_eq!(nanos.as_u64(), 123);
    }

    #[rstest]
    fn test_from_str_invalid() {
        let result = "abc".parse::<UnixNanos>();
        assert!(result.is_err());
    }

    #[rstest]
    fn test_from_str_pre_epoch_date() {
        let err = "1969-12-31".parse::<UnixNanos>().unwrap_err();
        assert_eq!(err.to_string(), "Unix 时间戳不能为负");
    }

    #[rstest]
    fn test_from_str_pre_epoch_rfc3339() {
        let err = "1969-12-31T23:59:59Z".parse::<UnixNanos>().unwrap_err();
        assert_eq!(err.to_string(), "Unix 时间戳不能为负");
    }

    #[rstest]
    fn test_try_from_datetime_valid() {
        use chrono::TimeZone;
        let datetime = Utc.timestamp_opt(1_000_000_000, 0).unwrap(); // 纪元以来的 10 亿秒
        let nanos = UnixNanos::from(datetime);
        assert_eq!(nanos.as_u64(), 1_000_000_000_000_000_000);
    }

    #[rstest]
    fn test_from_system_time() {
        let system_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let nanos = UnixNanos::from(system_time);
        assert_eq!(nanos.as_u64(), 1_000_000_000_000_000_000);
    }

    #[rstest]
    #[should_panic(expected = "SystemTime before UNIX EPOCH")]
    fn test_from_system_time_before_epoch() {
        let system_time = std::time::UNIX_EPOCH - std::time::Duration::from_secs(1);
        let _ = UnixNanos::from(system_time);
    }

    #[rstest]
    fn test_eq() {
        let nanos = UnixNanos::from(100);
        assert_eq!(nanos, 100);
        assert_eq!(nanos, Some(100));
        assert_ne!(nanos, 200);
        assert_ne!(nanos, Some(200));
        assert_ne!(nanos, None);
    }

    #[rstest]
    fn test_partial_cmp() {
        let nanos = UnixNanos::from(100);
        assert_eq!(nanos.partial_cmp(&100), Some(Ordering::Equal));
        assert_eq!(nanos.partial_cmp(&200), Some(Ordering::Less));
        assert_eq!(nanos.partial_cmp(&50), Some(Ordering::Greater));
        assert_eq!(nanos.partial_cmp(&None), Some(Ordering::Greater));
    }

    #[rstest]
    fn test_edge_case_max_value() {
        let nanos = UnixNanos::from(u64::MAX);
        assert_eq!(format!("{nanos}"), format!("{}", u64::MAX));
    }

    #[rstest]
    fn test_display() {
        let nanos = UnixNanos::from(123);
        assert_eq!(format!("{nanos}"), "123");
    }

    #[rstest]
    fn test_addition() {
        let nanos1 = UnixNanos::from(100);
        let nanos2 = UnixNanos::from(200);
        let result = nanos1 + nanos2;
        assert_eq!(result.as_u64(), 300);
    }

    #[rstest]
    fn test_add_assign() {
        let mut nanos = UnixNanos::from(100);
        nanos += 50_u64;
        assert_eq!(nanos.as_u64(), 150);
    }

    #[rstest]
    fn test_subtraction() {
        let nanos1 = UnixNanos::from(200);
        let nanos2 = UnixNanos::from(100);
        let result = nanos1 - nanos2;
        assert_eq!(result.as_u64(), 100);
    }

    #[rstest]
    fn test_sub_assign() {
        let mut nanos = UnixNanos::from(200);
        nanos -= 50_u64;
        assert_eq!(nanos.as_u64(), 150);
    }

    #[rstest]
    #[should_panic(expected = "UnixNanos overflow")]
    fn test_overflow_add() {
        let nanos = UnixNanos::from(u64::MAX);
        let _ = nanos + UnixNanos::from(1); // 溢出应触发 panic
    }

    #[rstest]
    #[should_panic(expected = "UnixNanos overflow")]
    fn test_overflow_add_u64() {
        let nanos = UnixNanos::from(u64::MAX);
        let _ = nanos + 1_u64; // 溢出应触发 panic
    }

    #[rstest]
    #[should_panic(expected = "UnixNanos underflow")]
    fn test_overflow_sub() {
        let _ = UnixNanos::default() - UnixNanos::from(1); // 下溢应触发 panic
    }

    #[rstest]
    #[should_panic(expected = "UnixNanos underflow")]
    fn test_overflow_sub_u64() {
        let _ = UnixNanos::default() - 1_u64; // 下溢应触发 panic
    }

    #[rstest]
    #[case(100, 50, Some(50))]
    #[case(1_000_000_000, 500_000_000, Some(500_000_000))]
    #[case(u64::MAX, u64::MAX - 1, Some(1))]
    #[case(50, 50, Some(0))]
    #[case(50, 100, None)]
    #[case(0, 1, None)]
    fn test_duration_since(
        #[case] time1: u64,
        #[case] time2: u64,
        #[case] expected: Option<DurationNanos>,
    ) {
        let nanos1 = UnixNanos::from(time1);
        let nanos2 = UnixNanos::from(time2);
        assert_eq!(nanos1.duration_since(&nanos2), expected);
    }

    #[rstest]
    fn test_duration_since_same_moment() {
        let moment = UnixNanos::from(1_707_577_123_456_789_000);
        assert_eq!(moment.duration_since(&moment), Some(0));
    }

    #[rstest]
    fn test_duration_since_chronological() {
        // 创建一个参考时间（2024 年 2 月 10 日）
        let earlier = Utc.with_ymd_and_hms(2024, 2, 10, 12, 0, 0).unwrap();

        // 创建一个比参考时间晚 1 小时 30 分钟 45 秒的时间（带有纳秒）
        let later = earlier
            + Duration::hours(1)
            + Duration::minutes(30)
            + Duration::seconds(45)
            + Duration::nanoseconds(500_000_000);

        let earlier_nanos = UnixNanos::from(earlier);
        let later_nanos = UnixNanos::from(later);

        // 计算预期的纳秒级持续时间
        let expected_duration = 60 * 60 * 1_000_000_000 + // 1 小时
        30 * 60 * 1_000_000_000 + // 30 分钟
        45 * 1_000_000_000 + // 45 秒
        500_000_000; // 5 亿纳秒

        assert_eq!(
            later_nanos.duration_since(&earlier_nanos),
            Some(expected_duration)
        );
        assert_eq!(earlier_nanos.duration_since(&later_nanos), None);
    }

    #[rstest]
    fn test_duration_since_with_edge_cases() {
        // 使用最大值进行测试
        let max = UnixNanos::from(u64::MAX);
        let smaller = UnixNanos::from(u64::MAX - 1000);

        assert_eq!(max.duration_since(&smaller), Some(1000));
        assert_eq!(smaller.duration_since(&max), None);

        // 使用最小值进行测试
        let min = UnixNanos::default(); // 零时间戳
        let larger = UnixNanos::from(1000);

        assert_eq!(min.duration_since(&min), Some(0));
        assert_eq!(larger.duration_since(&min), Some(1000));
        assert_eq!(min.duration_since(&larger), None);
    }

    #[rstest]
    fn test_serde_json() {
        let nanos = UnixNanos::from(123);
        let json = serde_json::to_string(&nanos).unwrap();
        let deserialized: UnixNanos = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, nanos);
    }

    #[rstest]
    fn test_serde_edge_cases() {
        let nanos = UnixNanos::from(u64::MAX);
        let json = serde_json::to_string(&nanos).unwrap();
        let deserialized: UnixNanos = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, nanos);
    }

    #[rstest]
    #[case("123", 123)] // 整数型字符串
    #[case("1234.567", 1_234_567_000_000)] // 浮点型字符串（秒转为纳秒）
    #[case("2024-02-10", 1_707_523_200_000_000_000)] // 简单日期（UTC 午夜）
    #[case("2024-02-10T14:58:43Z", 1_707_577_123_000_000_000)] // 不带小数部分的 RFC3339
    #[case("2024-02-10T14:58:43.456789Z", 1_707_577_123_456_789_000)] // 带有小数部分的 RFC3339
    fn test_from_str_formats(#[case] input: &str, #[case] expected: u64) {
        let parsed: UnixNanos = input.parse().unwrap();
        assert_eq!(parsed.as_u64(), expected);
    }

    #[rstest]
    #[case("abc")] // 随机字符串
    #[case("not a timestamp")] // 非时间戳字符串
    #[case("2024-02-10 14:58:43")] // 使用空格分隔的格式（非 RFC3339）
    fn test_from_str_invalid_formats(#[case] input: &str) {
        let result = input.parse::<UnixNanos>();
        assert!(result.is_err());
    }

    #[rstest]
    fn test_from_str_integer_overflow() {
        // 数字数量多于 u64::MAX (20 位数字)，肯定会溢出
        let input = "184467440737095516160";
        let result = input.parse::<UnixNanos>();
        assert!(result.is_err());
    }

    #[rstest]
    fn test_checked_add_overflow_returns_none() {
        let max = UnixNanos::from(u64::MAX);
        assert_eq!(max.checked_add(1_u64), None);
    }

    #[rstest]
    fn test_checked_sub_underflow_returns_none() {
        let zero = UnixNanos::default();
        assert_eq!(zero.checked_sub(1_u64), None);
    }

    #[rstest]
    fn test_saturating_add_overflow() {
        let max = UnixNanos::from(u64::MAX);
        let result = max.saturating_add_ns(1_u64);
        assert_eq!(result, UnixNanos::from(u64::MAX));
    }

    #[rstest]
    fn test_saturating_sub_underflow() {
        let zero = UnixNanos::default();
        let result = zero.saturating_sub_ns(1_u64);
        assert_eq!(result, UnixNanos::default());
    }

    #[rstest]
    fn test_from_str_float_overflow() {
        // 使用科学计数法，从而走浮点数解析路径。
        let input = "2e10"; // 200 亿秒 ~ 634 年 (> u64::MAX 纳秒)
        let result = input.parse::<UnixNanos>();
        assert!(result.is_err());
    }

    #[rstest]
    fn test_deserialize_u64() {
        let json = "123456789";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 123_456_789);
    }

    #[rstest]
    fn test_deserialize_string_with_int() {
        let json = "\"123456789\"";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 123_456_789);
    }

    #[rstest]
    fn test_deserialize_float() {
        let json = "1234.567";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 1_234_567_000_000);
    }

    #[rstest]
    fn test_deserialize_string_with_float() {
        let json = "\"1234.567\"";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 1_234_567_000_000);
    }

    #[rstest]
    fn test_deserialize_float_uses_truncation() {
        // 为了与 secs_to_nanos() 等保持一致，使用截断而非舍入
        let json = "0.9999999999";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 999_999_999); // 截断后的值，而不是舍入为 10 亿
    }

    #[rstest]
    #[case("\"2024-02-10T14:58:43.456789Z\"", 1_707_577_123_456_789_000)]
    #[case("\"2024-02-10T14:58:43Z\"", 1_707_577_123_000_000_000)]
    fn test_deserialize_timestamp_strings(#[case] input: &str, #[case] expected: u64) {
        let deserialized: UnixNanos = serde_json::from_str(input).unwrap();
        assert_eq!(deserialized.as_u64(), expected);
    }

    #[rstest]
    fn test_deserialize_negative_int_fails() {
        let json = "-123456789";
        let result: Result<UnixNanos, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_deserialize_negative_float_fails() {
        let json = "-1234.567";
        let result: Result<UnixNanos, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_deserialize_nan_fails() {
        // JSON 不直接支持 NaN，因此测试内部反序列化器
        use serde::de::{
            IntoDeserializer,
            value::{Error as ValueError, F64Deserializer},
        };
        let deserializer: F64Deserializer<ValueError> = f64::NAN.into_deserializer();
        let result: Result<UnixNanos, _> = UnixNanos::deserialize(deserializer);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("必须是有限值"));
    }

    #[rstest]
    fn test_deserialize_infinity_fails() {
        use serde::de::{
            IntoDeserializer,
            value::{Error as ValueError, F64Deserializer},
        };
        let deserializer: F64Deserializer<ValueError> = f64::INFINITY.into_deserializer();
        let result: Result<UnixNanos, _> = UnixNanos::deserialize(deserializer);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("必须是有限值"));
    }

    #[rstest]
    fn test_deserialize_negative_infinity_fails() {
        use serde::de::{
            IntoDeserializer,
            value::{Error as ValueError, F64Deserializer},
        };
        let deserializer: F64Deserializer<ValueError> = f64::NEG_INFINITY.into_deserializer();
        let result: Result<UnixNanos, _> = UnixNanos::deserialize(deserializer);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("必须是有限值"));
    }

    #[rstest]
    fn test_deserialize_overflow_float_fails() {
        // 测试将浮点数转换为纳秒时会溢出 u64 的情形
        // u64::MAX 约为 18.4e18，因此 u64::MAX / 1e9 = 约为 18.4e9 秒
        let result: Result<UnixNanos, _> = serde_json::from_str("1e20");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("超出范围"));
    }

    #[rstest]
    fn test_deserialize_invalid_string_fails() {
        let json = "\"并非时间戳\"";
        let result: Result<UnixNanos, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_deserialize_edge_cases() {
        // 测试零
        let json = "0";
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), 0);

        // 测试大数值
        let json = "18446744073709551615"; // u64::MAX
        let deserialized: UnixNanos = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.as_u64(), u64::MAX);
    }

    #[rstest]
    #[should_panic(expected = "UnixNanos value exceeds i64::MAX")]
    fn test_as_i64_overflow_panics() {
        let nanos = UnixNanos::from(u64::MAX);
        let _ = nanos.as_i64(); // 应触发 panic
    }

    ////////////////////////////////////////////////////////////////////////////////
    // 基于属性的测试 (Property-based testing)
    ////////////////////////////////////////////////////////////////////////////////

    use proptest::prelude::*;

    fn unix_nanos_strategy() -> impl Strategy<Value = UnixNanos> {
        prop_oneof![
            // 小数值
            0u64..1_000_000u64,
            // 中等数值（微秒范围）
            1_000_000u64..1_000_000_000_000u64,
            // 大数值（1970 以来纳秒数，但对于算术运算安全）
            1_000_000_000_000u64..=i64::MAX as u64,
            // 边界情况
            Just(0u64),
            Just(1u64),
            Just(1_000_000_000u64),             // 1 秒的纳秒数
            Just(1_000_000_000_000u64),         // 约为 2001 年的时间戳
            Just(1_700_000_000_000_000_000u64), // 约为 2023 年的时间戳
            Just((i64::MAX / 2) as u64),        // 翻倍也安全
        ]
        .prop_map(UnixNanos::from)
    }

    fn unix_nanos_pair_strategy() -> impl Strategy<Value = (UnixNanos, UnixNanos)> {
        (unix_nanos_strategy(), unix_nanos_strategy())
    }

    proptest! {
        #[rstest]
        fn prop_unix_nanos_construction_roundtrip(value in 0u64..=i64::MAX as u64) {
            let nanos = UnixNanos::from(value);
            prop_assert_eq!(nanos.as_u64(), value);
            prop_assert_eq!(nanos.as_f64(), value as f64);

            // 仅对 i64 范围内的值测试 i64 转换
            if i64::try_from(value).is_ok() {
                prop_assert_eq!(nanos.as_i64(), value as i64);
            }
        }

        #[rstest]
        fn prop_unix_nanos_addition_commutative(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // 当不发生溢出时，加法应满足交换律
            if let (Some(sum1), Some(sum2)) = (
                nanos1.checked_add(nanos2.as_u64()),
                nanos2.checked_add(nanos1.as_u64())
            ) {
                prop_assert_eq!(sum1, sum2, "加法应满足交换律");
            }
        }

        #[rstest]
        fn prop_unix_nanos_addition_associative(
            nanos1 in unix_nanos_strategy(),
            nanos2 in unix_nanos_strategy(),
            nanos3 in unix_nanos_strategy(),
        ) {
            // 当不发生溢出时，加法应满足结合律
            if let (Some(sum1), Some(sum2)) = (
                nanos1.as_u64().checked_add(nanos2.as_u64()),
                nanos2.as_u64().checked_add(nanos3.as_u64())
            )
                && let (Some(left), Some(right)) = (
                    sum1.checked_add(nanos3.as_u64()),
                    nanos1.as_u64().checked_add(sum2)
                ) {
                    let left_result = UnixNanos::from(left);
                    let right_result = UnixNanos::from(right);
                    prop_assert_eq!(left_result, right_result, "加法应满足结合律");
                }
        }

        #[rstest]
        fn prop_unix_nanos_subtraction_inverse(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // 当不发生下溢时，减法应当是加法的逆运算
            if let Some(sum) = nanos1.checked_add(nanos2.as_u64()) {
                let diff = sum - nanos2;
                prop_assert_eq!(diff, nanos1, "减法应为加法的逆运算");
            }
        }

        #[rstest]
        fn prop_unix_nanos_zero_identity(nanos in unix_nanos_strategy()) {
            // 零应该是加法的单位元
            let zero = UnixNanos::default();
            prop_assert_eq!(nanos + zero, nanos, "零应当是加法的单位元");
            prop_assert_eq!(zero + nanos, nanos, "零应当是加法的单位元（交换律）");
            prop_assert!(zero.is_zero(), "零应当被识别为零");
        }

        #[rstest]
        fn prop_unix_nanos_ordering_consistency(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // 排序操作应当是一致的
            let eq = nanos1 == nanos2;
            let lt = nanos1 < nanos2;
            let gt = nanos1 > nanos2;
            let le = nanos1 <= nanos2;
            let ge = nanos1 >= nanos2;

            // eq, lt, gt 中应当有且仅有一个为 true
            let exclusive_count = [eq, lt, gt].iter().filter(|&&x| x).count();
            prop_assert_eq!(exclusive_count, 1, "==, <, > 中应有且仅有一个为 true");

            // 一致性检查
            prop_assert_eq!(le, eq || lt, "<= 应等于 == || <");
            prop_assert_eq!(ge, eq || gt, ">= 应等于 == || >");
            prop_assert_eq!(lt, nanos2 > nanos1, "< 应当与 > 对称");
            prop_assert_eq!(le, nanos2 >= nanos1, "<= 应当与 >= 对称");
        }

        #[rstest]
        fn prop_unix_nanos_string_roundtrip(nanos in unix_nanos_strategy()) {
            // 字符串序列化应当能正确往返转换
            let string_repr = nanos.to_string();
            let parsed = UnixNanos::from_str(&string_repr);
            prop_assert!(parsed.is_ok(), "有效的 UnixNanos 字符串解析应当成功");
            if let Ok(parsed_nanos) = parsed {
                prop_assert_eq!(parsed_nanos, nanos, "字符串往返转换应完全一致");
            }
        }

        #[rstest]
        fn prop_unix_nanos_datetime_conversion(nanos in unix_nanos_strategy()) {
            // 日期时间转换应当是一致的（仅测试 i64 范围内的值）
            if i64::try_from(nanos.as_u64()).is_ok() {
                let datetime = nanos.to_datetime_utc();
                let converted_back = UnixNanos::from(datetime);
                prop_assert_eq!(converted_back, nanos, "DateTime 转换应当能往返转换");

                // 有效日期的 RFC3339 字符串也应当能正确往返转换
                let rfc3339 = nanos.to_rfc3339();
                if let Ok(parsed_from_rfc3339) = UnixNanos::from_str(&rfc3339) {
                    prop_assert_eq!(parsed_from_rfc3339, nanos, "RFC3339 字符串应当能往返转换");
                }
            }
        }

        #[rstest]
        fn prop_unix_nanos_duration_since(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // duration_since 应当与比较和算术运算保持一致
            let duration = nanos1.duration_since(&nanos2);

            if nanos1 >= nanos2 {
                // 如果 nanos1 >= nanos2，持续时间应当为 Some 且等于差值
                prop_assert!(duration.is_some(), "当 第一个值 >= 第二个值 时，持续时间应为 Some");
                if let Some(dur) = duration {
                    prop_assert_eq!(dur, nanos1.as_u64() - nanos2.as_u64(),
                        "持续时间应等于两者的差值");
                    prop_assert_eq!(nanos2 + dur, nanos1.as_u64(),
                        "第二个值 + 持续时间应等于第一个值");
                }
            } else {
                // 如果 nanos1 < nanos2，持续时间应为 None
                prop_assert!(duration.is_none(), "当 第一个值 < 第二个值 时，持续时间应为 None");
            }
        }

        #[rstest]
        fn prop_unix_nanos_checked_arithmetic(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // 校验过的算术运算（Checked arithmetic）应当在不发生溢出/下溢时与常规算术保持一致
            let checked_add = nanos1.checked_add(nanos2.as_u64());
            let checked_sub = nanos1.checked_sub(nanos2.as_u64());

            // 如果 checked_add 成功，常规加法应当产生相同结果
            if let Some(sum) = checked_add
                && nanos1.as_u64().checked_add(nanos2.as_u64()).is_some() {
                    prop_assert_eq!(sum, nanos1 + nanos2, "当不溢出时，Checked 加法应与常规加法一致");
                }

            // 如果 checked_sub 成功，常规减法应当产生相同结果
            if let Some(diff) = checked_sub
                && nanos1.as_u64() >= nanos2.as_u64() {
                    prop_assert_eq!(diff, nanos1 - nanos2, "当不下溢时，Checked 减法应与常规减法一致");
                }
        }

        #[rstest]
        fn prop_unix_nanos_saturating_arithmetic(
            (nanos1, nanos2) in unix_nanos_pair_strategy()
        ) {
            // 饱和算术运算绝不应触发 panic 且应当产生合理结果
            let sat_add = nanos1.saturating_add_ns(nanos2.as_u64());
            let sat_sub = nanos1.saturating_sub_ns(nanos2.as_u64());

            // 饱和加法结果应 >= 两个操作数
            prop_assert!(sat_add >= nanos1, "饱和加法结果应 >= 第一个操作数");
            prop_assert!(sat_add.as_u64() >= nanos2.as_u64(), "饱和加法结果应 >= 第二个操作数");

            // 饱和减法结果应 <= 第一个操作数
            prop_assert!(sat_sub <= nanos1, "饱和减法结果应 <= 第一个操作数");

            // 如果不会发生溢出/下溢，饱和运算应当与 checked 运算一致
            if let Some(checked_sum) = nanos1.checked_add(nanos2.as_u64()) {
                prop_assert_eq!(sat_add, checked_sum, "当不溢出时，饱和加法应与 checked 加法一致");
            } else {
                prop_assert_eq!(sat_add, UnixNanos::from(u64::MAX), "发生溢出时，饱和加法应为 MAX");
            }

            if let Some(checked_diff) = nanos1.checked_sub(nanos2.as_u64()) {
                prop_assert_eq!(sat_sub, checked_diff, "当不下溢时，饱和减法应与 checked 减法一致");
            } else {
                prop_assert_eq!(sat_sub, UnixNanos::default(), "发生下溢时，饱和减法应为零");
            }
        }
    }
}
