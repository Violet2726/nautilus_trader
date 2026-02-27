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

//! 通用的日期和时间函数。
use std::convert::TryFrom;

use chrono::{DateTime, Datelike, NaiveDate, SecondsFormat, TimeDelta, Utc, Weekday};

use crate::UnixNanos;

/// 一秒包含的毫秒数。
pub const MILLISECONDS_IN_SECOND: u64 = 1_000;

/// 一秒包含的纳秒数。
pub const NANOSECONDS_IN_SECOND: u64 = 1_000_000_000;

/// 一毫秒包含的纳秒数。
pub const NANOSECONDS_IN_MILLISECOND: u64 = 1_000_000;

/// 一微秒包含的纳秒数。
pub const NANOSECONDS_IN_MICROSECOND: u64 = 1_000;

// 在不溢出 `u64` 的情况下，可转换为纳秒的最大有限秒数输入。
const MAX_SECS_FOR_NANOS: f64 = u64::MAX as f64 / NANOSECONDS_IN_SECOND as f64;
// 在不溢出 `u64` 的情况下，可转换为毫秒的最大有限秒数输入。
const MAX_SECS_FOR_MILLIS: f64 = u64::MAX as f64 / MILLISECONDS_IN_SECOND as f64;
// 在不溢出 `u64` 的情况下，可转换为纳秒的最大有限毫秒数输入。
const MAX_MILLIS_FOR_NANOS: f64 = u64::MAX as f64 / NANOSECONDS_IN_MILLISECOND as f64;
// 在不溢出 `u64` 的情况下，可转换为纳秒的最大有限微秒数输入。
const MAX_MICROS_FOR_NANOS: f64 = u64::MAX as f64 / NANOSECONDS_IN_MICROSECOND as f64;

// 编译时检查时间常量，防止意外修改
const _: () = {
    assert!(NANOSECONDS_IN_SECOND == 1_000_000_000);
    assert!(NANOSECONDS_IN_MILLISECOND == 1_000_000);
    assert!(NANOSECONDS_IN_MICROSECOND == 1_000);
    assert!(MILLISECONDS_IN_SECOND == 1_000);
    assert!(NANOSECONDS_IN_SECOND == MILLISECONDS_IN_SECOND * NANOSECONDS_IN_MILLISECOND);
    assert!(NANOSECONDS_IN_MILLISECOND == NANOSECONDS_IN_MICROSECOND * 1_000);
    assert!(NANOSECONDS_IN_SECOND / NANOSECONDS_IN_MILLISECOND == 1_000);
    assert!(NANOSECONDS_IN_SECOND / NANOSECONDS_IN_MICROSECOND == 1_000_000);
};

/// 工作日列表（周一至周五）。
pub const WEEKDAYS: [Weekday; 5] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
];

/// 将秒转换为纳秒 (ns)。
///
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "用于单位转换，截断操作是有意为之，在限制范围后可能会丢失精度"
)]
pub fn secs_to_nanos(secs: f64) -> anyhow::Result<u64> {
    anyhow::ensure!(secs.is_finite(), "秒数必须是有限值，输入为 {secs}");
    if secs <= 0.0 {
        return Ok(0);
    }
    anyhow::ensure!(
        secs <= MAX_SECS_FOR_NANOS,
        "秒数 {secs} 超过了最大可表示值 {MAX_SECS_FOR_NANOS}"
    );
    let nanos = secs * NANOSECONDS_IN_SECOND as f64;
    Ok(nanos.trunc() as u64)
}

/// 将秒转换为毫秒 (ms)。
///
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "用于单位转换，截断操作是有意为之，在限制范围后可能会丢失精度"
)]
pub fn secs_to_millis(secs: f64) -> anyhow::Result<u64> {
    anyhow::ensure!(secs.is_finite(), "秒数必须是有限值，输入为 {secs}");
    if secs <= 0.0 {
        return Ok(0);
    }
    anyhow::ensure!(
        secs <= MAX_SECS_FOR_MILLIS,
        "秒数 {secs} 超过了最大可表示值 {MAX_SECS_FOR_MILLIS}"
    );
    let millis = secs * MILLISECONDS_IN_SECOND as f64;
    Ok(millis.trunc() as u64)
}

/// 将秒转换为纳秒 (ns)，如果输入无效则触发 panic。
///
/// 这是 [`secs_to_nanos`] 的便捷包装函数，适用于调用者确信输入可信且在范围内的情况。
#[must_use]
pub fn secs_to_nanos_unchecked(secs: f64) -> u64 {
    secs_to_nanos(secs).expect("secs_to_nanos_unchecked: 输入无效或溢出")
}

/// 将毫秒 (ms) 转换为纳秒 (ns)。
///
/// 通过截断小数部分将 f64 转换为 u64 是用于单位转换的特意设计，
/// 在范围限制后可能会丢失精度并丢弃负值。
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "用于单位转换，截断操作是有意为之，在限制范围后可能会丢失精度"
)]
pub fn millis_to_nanos(millis: f64) -> anyhow::Result<u64> {
    anyhow::ensure!(
        millis.is_finite(),
        "毫秒数必须是有限值，输入为 {millis}"
    );
    if millis <= 0.0 {
        return Ok(0);
    }
    anyhow::ensure!(
        millis <= MAX_MILLIS_FOR_NANOS,
        "毫秒数 {millis} 超过了最大可表示值 {MAX_MILLIS_FOR_NANOS}"
    );
    let nanos = millis * NANOSECONDS_IN_MILLISECOND as f64;
    Ok(nanos.trunc() as u64)
}

/// 将毫秒 (ms) 转换为纳秒 (ns)，如果输入无效则触发 panic。
#[must_use]
pub fn millis_to_nanos_unchecked(millis: f64) -> u64 {
    millis_to_nanos(millis).expect("millis_to_nanos_unchecked: 输入无效或溢出")
}

/// 将微秒 (μs) 转换为纳秒 (ns)。
///
/// 通过截断小数部分将 f64 转换为 u64 是用于单位转换的特意设计，
/// 在范围限制后可能会丢失精度并丢弃负值。
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "用于单位转换，截断操作是有意为之，在限制范围后可能会丢失精度"
)]
pub fn micros_to_nanos(micros: f64) -> anyhow::Result<u64> {
    anyhow::ensure!(
        micros.is_finite(),
        "微秒数必须是有限值，输入为 {micros}"
    );
    if micros <= 0.0 {
        return Ok(0);
    }
    anyhow::ensure!(
        micros <= MAX_MICROS_FOR_NANOS,
        "微秒数 {micros} 超过了最大可表示值 {MAX_MICROS_FOR_NANOS}"
    );
    let nanos = micros * NANOSECONDS_IN_MICROSECOND as f64;
    Ok(nanos.trunc() as u64)
}

/// 将微秒 (μs) 转换为纳秒 (ns)，如果输入无效则触发 panic。
#[must_use]
pub fn micros_to_nanos_unchecked(micros: f64) -> u64 {
    micros_to_nanos(micros).expect("micros_to_nanos_unchecked: 输入无效或溢出")
}

/// 将纳秒 (ns) 转换为秒。
///
/// 对于较大的值，将 u64 转换为 f64 可能会丢失精度，
/// 但在计算秒数的小数部分时是可以接受的。
#[allow(
    clippy::cast_precision_loss,
    reason = "对于时间转换，精度丢失是可以接受的"
)]
#[must_use]
pub fn nanos_to_secs(nanos: u64) -> f64 {
    let seconds = nanos / NANOSECONDS_IN_SECOND;
    let rem_nanos = nanos % NANOSECONDS_IN_SECOND;
    (seconds as f64) + (rem_nanos as f64) / (NANOSECONDS_IN_SECOND as f64)
}

/// 将纳秒 (ns) 转换为毫秒 (ms)。
#[must_use]
pub const fn nanos_to_millis(nanos: u64) -> u64 {
    nanos / NANOSECONDS_IN_MILLISECOND
}

/// 将纳秒 (ns) 转换为微秒 (μs)。
#[must_use]
pub const fn nanos_to_micros(nanos: u64) -> u64 {
    nanos / NANOSECONDS_IN_MICROSECOND
}

/// 将 UNIX 纳秒时间戳转换为 ISO 8601 (RFC 3339) 格式的字符串。
#[inline]
#[must_use]
pub fn unix_nanos_to_iso8601(unix_nanos: UnixNanos) -> String {
    let datetime = unix_nanos.to_datetime_utc();
    datetime.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

/// 将 ISO 8601 (RFC 3339) 格式的字符串转换为 UNIX 纳秒时间戳。
///
/// 此函数接受多种 ISO 8601 格式，包括：
/// - 具有纳秒精度的完整 RFC 3339 格式: "2024-02-10T14:58:43.456789Z"
/// - 不带小数秒的 RFC 3339 格式: "2024-02-10T14:58:43Z"
/// - 简单的日期格式: "2024-02-10"（被解析为 UTC 午夜）
///
/// # 参数
///
/// - `date_string`: 要解析的 ISO 8601 格式日期字符串
///
/// # 返回
///
/// 如果字符串解析成功，则返回 `Ok(UnixNanos)`；如果格式无效或时间戳超出范围，则返回错误。
///
/// # 错误
///
/// 在以下情况下返回错误：
/// - 字符串格式不是有效的 ISO 8601 格式
/// - 时间戳超出 `UnixNanos` 的表示范围
/// - 日期/时间值无效
#[inline]
pub fn iso8601_to_unix_nanos(date_string: String) -> anyhow::Result<UnixNanos> {
    date_string
        .parse::<UnixNanos>()
        .map_err(|e| anyhow::anyhow!("无法解析 ISO 8601 字符串 '{date_string}': {e}"))
}

/// 将 UNIX 纳秒时间戳转换为具有毫秒精度的 ISO 8601 (RFC 3339) 格式字符串。
#[inline]
#[must_use]
pub fn unix_nanos_to_iso8601_millis(unix_nanos: UnixNanos) -> String {
    let datetime = unix_nanos.to_datetime_utc();
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// 将给定的 UNIX 纳秒向下舍入到最接近的微秒。
#[must_use]
pub const fn floor_to_nearest_microsecond(unix_nanos: u64) -> u64 {
    (unix_nanos / NANOSECONDS_IN_MICROSECOND) * NANOSECONDS_IN_MICROSECOND
}

/// 根据给定的 `year`、`month` 和 `day` 计算最后一个工作日（周一至周五）。
///
/// # 错误
///
/// 如果日期无效，则返回错误。
pub fn last_weekday_nanos(year: i32, month: u32, day: u32) -> anyhow::Result<UnixNanos> {
    let date =
        NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| anyhow::anyhow!("无效日期"))?;
    let current_weekday = date.weekday().number_from_monday();

    // 计算最接近工作日（周一至周五）的天数偏移量
    let offset = i64::from(match current_weekday {
        1..=5 => 0, // 周一至周五，无需调整
        6 => 1,     // 周六，调整为上周五
        _ => 2,     // 周日，调整为上周五
    });
    // 计算上一个最接近的工作日
    let last_closest = date - TimeDelta::days(offset);

    // 转换为 UNIX 纳秒
    let unix_timestamp_ns = last_closest
        .and_hms_nano_opt(0, 0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("`and_hms_nano_opt` 失败"))?;

    // 将时间戳纳秒安全地从 i64 转换为 u64
    let raw_ns = unix_timestamp_ns
        .and_utc()
        .timestamp_nanos_opt()
        .ok_or_else(|| anyhow::anyhow!("`timestamp_nanos_opt` 失败"))?;
    let ns_u64 =
        u64::try_from(raw_ns).map_err(|_| anyhow::anyhow!("负数时间戳: {raw_ns}"))?;
    Ok(UnixNanos::from(ns_u64))
}

/// 检查给定的 UNIX 纳秒时间戳是否在最近 24 小时内。
///
/// # 错误
///
/// 如果时间戳无效，则返回错误。
pub fn is_within_last_24_hours(timestamp_ns: UnixNanos) -> anyhow::Result<bool> {
    let timestamp_ns = timestamp_ns.as_u64();
    let seconds = timestamp_ns / NANOSECONDS_IN_SECOND;
    let nanoseconds = (timestamp_ns % NANOSECONDS_IN_SECOND) as u32;
    // 安全地将秒数转换为 i64
    let secs_i64 = i64::try_from(seconds)
        .map_err(|_| anyhow::anyhow!("时间戳秒数溢出: {seconds}"))?;
    let timestamp = DateTime::from_timestamp(secs_i64, nanoseconds)
        .ok_or_else(|| anyhow::anyhow!("无效的时间戳 {timestamp_ns}"))?;
    let now = Utc::now();

    // 未来时间戳不属于“最近 24 小时内”
    if timestamp > now {
        return Ok(false);
    }

    // 检查时间戳是否在最近 24 小时内（非负持续时间 <= 1 天）
    Ok(now.signed_duration_since(timestamp) <= TimeDelta::days(1))
}

/// 从 chrono `DateTime<Utc>` 中减去 `n` 个月。
///
/// # 错误
///
/// 如果结果日期无效或超出范围，则返回错误。
pub fn subtract_n_months(datetime: DateTime<Utc>, n: u32) -> anyhow::Result<DateTime<Utc>> {
    match datetime.checked_sub_months(chrono::Months::new(n)) {
        Some(result) => Ok(result),
        None => anyhow::bail!("无法从 {datetime} 中减去 {n} 个月"),
    }
}

/// 向 chrono `DateTime<Utc>` 中增加 `n` 个月。
///
/// # 错误
///
/// 如果结果日期无效或超出范围，则返回错误。
pub fn add_n_months(datetime: DateTime<Utc>, n: u32) -> anyhow::Result<DateTime<Utc>> {
    match datetime.checked_add_months(chrono::Months::new(n)) {
        Some(result) => Ok(result),
        None => anyhow::bail!("无法向 {datetime} 中增加 {n} 个月"),
    }
}

/// 从给定的 UNIX 纳秒时间戳中减去 `n` 个月。
///
/// # 错误
///
/// 如果结果时间戳超出范围或无效，则返回错误。
pub fn subtract_n_months_nanos(unix_nanos: UnixNanos, n: u32) -> anyhow::Result<UnixNanos> {
    let datetime = unix_nanos.to_datetime_utc();
    let result = subtract_n_months(datetime, n)?;
    let timestamp = match result.timestamp_nanos_opt() {
        Some(ts) => ts,
        None => anyhow::bail!("减去 {n} 个月后时间戳超出范围"),
    };

    if timestamp < 0 {
        anyhow::bail!("不允许负数时间戳");
    }

    Ok(UnixNanos::from(timestamp as u64))
}

/// 向给定的 UNIX 纳秒时间戳中增加 `n` 个月。
///
/// # 错误
///
/// 如果结果时间戳超出范围或无效，则返回错误。
pub fn add_n_months_nanos(unix_nanos: UnixNanos, n: u32) -> anyhow::Result<UnixNanos> {
    let datetime = unix_nanos.to_datetime_utc();
    let result = add_n_months(datetime, n)?;
    let timestamp = match result.timestamp_nanos_opt() {
        Some(ts) => ts,
        None => anyhow::bail!("增加 {n} 个月后时间戳超出范围"),
    };

    if timestamp < 0 {
        anyhow::bail!("不允许负数时间戳");
    }

    Ok(UnixNanos::from(timestamp as u64))
}

/// 向 chrono `DateTime<Utc>` 中增加 `n` 年。
///
/// # 错误
///
/// 如果结果日期无效或超出范围，则返回错误。
pub fn add_n_years(datetime: DateTime<Utc>, n: u32) -> anyhow::Result<DateTime<Utc>> {
    let months = n.checked_mul(12).ok_or_else(|| {
        anyhow::anyhow!("无法向 {datetime} 增加 {n} 年：月份数量溢出")
    })?;

    match datetime.checked_add_months(chrono::Months::new(months)) {
        Some(result) => Ok(result),
        None => anyhow::bail!("无法向 {datetime} 中增加 {n} 年"),
    }
}

/// 从 chrono `DateTime<Utc>` 中减去 `n` 年。
///
/// # 错误
///
/// 如果结果日期无效或超出范围，则返回错误。
pub fn subtract_n_years(datetime: DateTime<Utc>, n: u32) -> anyhow::Result<DateTime<Utc>> {
    let months = n.checked_mul(12).ok_or_else(|| {
        anyhow::anyhow!("无法从 {datetime} 减去 {n} 年：月份数量溢出")
    })?;

    match datetime.checked_sub_months(chrono::Months::new(months)) {
        Some(result) => Ok(result),
        None => anyhow::bail!("无法从 {datetime} 中减去 {n} 年"),
    }
}

/// 向给定的 UNIX 纳秒时间戳中增加 `n` 年。
///
/// # 错误
///
/// 如果结果时间戳超出范围或无效，则返回错误。
pub fn add_n_years_nanos(unix_nanos: UnixNanos, n: u32) -> anyhow::Result<UnixNanos> {
    let datetime = unix_nanos.to_datetime_utc();
    let result = add_n_years(datetime, n)?;
    let timestamp = match result.timestamp_nanos_opt() {
        Some(ts) => ts,
        None => anyhow::bail!("增加 {n} 年后时间戳超出范围"),
    };

    if timestamp < 0 {
        anyhow::bail!("不允许负数时间戳");
    }

    Ok(UnixNanos::from(timestamp as u64))
}

/// 从给定的 UNIX 纳秒时间戳中减去 `n` 年。
///
/// # 错误
///
/// 如果结果时间戳超出范围或无效，则返回错误。
pub fn subtract_n_years_nanos(unix_nanos: UnixNanos, n: u32) -> anyhow::Result<UnixNanos> {
    let datetime = unix_nanos.to_datetime_utc();
    let result = subtract_n_years(datetime, n)?;
    let timestamp = match result.timestamp_nanos_opt() {
        Some(ts) => ts,
        None => anyhow::bail!("减去 {n} 年后时间戳超出范围"),
    };

    if timestamp < 0 {
        anyhow::bail!("不允许负数时间戳");
    }

    Ok(UnixNanos::from(timestamp as u64))
}

/// 返回 `(year, month)` 对应的最后一个有效天数。
///
/// 如果 `month` 不在 1..=12 范围内，则返回 `None`。
#[must_use]
pub const fn last_day_of_month(year: i32, month: u32) -> Option<u32> {
    // 验证月份范围 1-12
    if month < 1 || month > 12 {
        return None;
    }

    // 二月闰年逻辑
    Some(match month {
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31, // 一月、三月、五月、七月、八月、十月、十二月
    })
}

/// 基础闰年检查
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// 将可选的 DateTime 转换为可选的 UnixNanos 时间戳。
pub fn datetime_to_unix_nanos(value: Option<DateTime<Utc>>) -> Option<UnixNanos> {
    value
        .and_then(|dt| dt.timestamp_nanos_opt())
        .and_then(|nanos| u64::try_from(nanos).ok())
        .map(UnixNanos::from)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "测试中允许精确的浮点数比较"
)]
mod tests {
    use chrono::{DateTime, TimeDelta, TimeZone, Timelike, Utc};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(0.0, 0)]
    #[case(1.0, 1_000_000_000)]
    #[case(1.1, 1_100_000_000)]
    #[case(42.0, 42_000_000_000)]
    #[case(0.000_123_5, 123_500)]
    #[case(0.000_000_01, 10)]
    #[case(0.000_000_001, 1)]
    #[case(9.999_999_999, 9_999_999_999)]
    fn test_secs_to_nanos(#[case] value: f64, #[case] expected: u64) {
        let result = secs_to_nanos(value).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0.0, 0)]
    #[case(1.0, 1_000)]
    #[case(1.1, 1_100)]
    #[case(42.0, 42_000)]
    #[case(0.012_34, 12)]
    #[case(0.001, 1)]
    fn test_secs_to_millis(#[case] value: f64, #[case] expected: u64) {
        let result = secs_to_millis(value).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_secs_to_nanos_unchecked_matches_checked() {
        assert_eq!(secs_to_nanos_unchecked(1.1), secs_to_nanos(1.1).unwrap());
    }

    #[rstest]
    fn test_secs_to_nanos_non_finite_errors() {
        let err = secs_to_nanos(f64::NAN).unwrap_err();
        assert!(err.to_string().contains("finite"));
    }

    #[rstest]
    fn test_secs_to_nanos_overflow_errors() {
        let err = secs_to_nanos(MAX_SECS_FOR_NANOS + 1.0).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[rstest]
    fn test_secs_to_millis_non_finite_errors() {
        let err = secs_to_millis(f64::INFINITY).unwrap_err();
        assert!(err.to_string().contains("finite"));
    }

    #[rstest]
    fn test_millis_to_nanos_overflow_errors() {
        let err = millis_to_nanos(MAX_MILLIS_FOR_NANOS + 1.0).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[rstest]
    fn test_millis_to_nanos_non_finite_errors() {
        let err = millis_to_nanos(f64::NEG_INFINITY).unwrap_err();
        assert!(err.to_string().contains("finite"));
    }

    #[rstest]
    fn test_micros_to_nanos_non_finite_errors() {
        let err = micros_to_nanos(f64::NAN).unwrap_err();
        assert!(err.to_string().contains("finite"));
    }

    #[rstest]
    fn test_micros_to_nanos_overflow_errors() {
        // 使用 * 2.0 是因为由于 f64 的精度限制，+ 1.0 不会改变 MAX_MICROS_FOR_NANOS
        let err = micros_to_nanos(MAX_MICROS_FOR_NANOS * 2.0).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[rstest]
    fn test_secs_to_nanos_negative_infinity_errors() {
        let result = secs_to_nanos(f64::NEG_INFINITY);
        assert!(result.is_err());
    }

    #[rstest]
    #[case(2024, 0)] // 月份超出下限
    #[case(2024, 13)] // 月份超出上限
    fn test_last_day_of_month_invalid_month(#[case] year: i32, #[case] month: u32) {
        assert!(last_day_of_month(year, month).is_none());
    }

    #[rstest]
    #[case(0.0, 0)]
    #[case(1.0, 1_000_000)]
    #[case(1.1, 1_100_000)]
    #[case(42.0, 42_000_000)]
    #[case(0.000_123_4, 123)]
    #[case(0.000_01, 10)]
    #[case(0.000_001, 1)]
    #[case(9.999_999, 9_999_999)]
    fn test_millis_to_nanos(#[case] value: f64, #[case] expected: u64) {
        let result = millis_to_nanos(value).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_millis_to_nanos_unchecked_matches_checked() {
        assert_eq!(
            millis_to_nanos_unchecked(1.1),
            millis_to_nanos(1.1).unwrap()
        );
    }

    #[rstest]
    #[case(0.0, 0)]
    #[case(1.0, 1_000)]
    #[case(1.1, 1_100)]
    #[case(42.0, 42_000)]
    #[case(0.1234, 123)]
    #[case(0.01, 10)]
    #[case(0.001, 1)]
    #[case(9.999, 9_999)]
    fn test_micros_to_nanos(#[case] value: f64, #[case] expected: u64) {
        let result = micros_to_nanos(value).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_micros_to_nanos_unchecked_matches_checked() {
        assert_eq!(
            micros_to_nanos_unchecked(1.1),
            micros_to_nanos(1.1).unwrap()
        );
    }

    #[rstest]
    #[case(0, 0.0)]
    #[case(1, 1e-09)]
    #[case(1_000_000_000, 1.0)]
    #[case(42_897_123_111, 42.897_123_111)]
    fn test_nanos_to_secs(#[case] value: u64, #[case] expected: f64) {
        let result = nanos_to_secs(value);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, 0)]
    #[case(1_000_000, 1)]
    #[case(1_000_000_000, 1000)]
    #[case(42_897_123_111, 42897)]
    fn test_nanos_to_millis(#[case] value: u64, #[case] expected: u64) {
        let result = nanos_to_millis(value);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, 0)]
    #[case(1_000, 1)]
    #[case(1_000_000_000, 1_000_000)]
    #[case(42_897_123, 42_897)]
    fn test_nanos_to_micros(#[case] value: u64, #[case] expected: u64) {
        let result = nanos_to_micros(value);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, "1970-01-01T00:00:00.000000000Z")] // Unix 纪元
    #[case(1, "1970-01-01T00:00:00.000000001Z")] // 1 纳秒
    #[case(1_000, "1970-01-01T00:00:00.000001000Z")] // 1 微秒
    #[case(1_000_000, "1970-01-01T00:00:00.001000000Z")] // 1 毫秒
    #[case(1_000_000_000, "1970-01-01T00:00:01.000000000Z")] // 1 秒
    #[case(1_702_857_600_000_000_000, "2023-12-18T00:00:00.000000000Z")] // 特定日期
    fn test_unix_nanos_to_iso8601(#[case] nanos: u64, #[case] expected: &str) {
        let result = unix_nanos_to_iso8601(UnixNanos::from(nanos));
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, "1970-01-01T00:00:00.000Z")] // Unix 纪元
    #[case(1_000_000, "1970-01-01T00:00:00.001Z")] // 1 毫秒
    #[case(1_000_000_000, "1970-01-01T00:00:01.000Z")] // 1 秒
    #[case(1_702_857_600_123_456_789, "2023-12-18T00:00:00.123Z")] // 毫秒精度
    fn test_unix_nanos_to_iso8601_millis(#[case] nanos: u64, #[case] expected: &str) {
        let result = unix_nanos_to_iso8601_millis(UnixNanos::from(nanos));
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(2023, 12, 15, 1_702_598_400_000_000_000)] // 周五
    #[case(2023, 12, 16, 1_702_598_400_000_000_000)] // 周六
    #[case(2023, 12, 17, 1_702_598_400_000_000_000)] // 周日
    #[case(2023, 12, 18, 1_702_857_600_000_000_000)] // 周一
    fn test_last_closest_weekday_nanos_with_valid_date(
        #[case] year: i32,
        #[case] month: u32,
        #[case] day: u32,
        #[case] expected: u64,
    ) {
        let result = last_weekday_nanos(year, month, day).unwrap().as_u64();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_last_closest_weekday_nanos_with_invalid_date() {
        let result = last_weekday_nanos(2023, 4, 31);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_last_closest_weekday_nanos_with_nonexistent_date() {
        let result = last_weekday_nanos(2023, 2, 30);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_last_closest_weekday_nanos_with_invalid_conversion() {
        let result = last_weekday_nanos(9999, 12, 31);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_is_within_last_24_hours_when_now() {
        let now_ns = Utc::now().timestamp_nanos_opt().unwrap();
        assert!(is_within_last_24_hours(UnixNanos::from(now_ns as u64)).unwrap());
    }

    #[rstest]
    fn test_is_within_last_24_hours_when_two_days_ago() {
        let past_ns = (Utc::now() - TimeDelta::try_days(2).unwrap())
            .timestamp_nanos_opt()
            .unwrap();
        assert!(!is_within_last_24_hours(UnixNanos::from(past_ns as u64)).unwrap());
    }

    #[rstest]
    fn test_is_within_last_24_hours_when_future() {
        // 未来时间戳应返回 false
        let future_ns = (Utc::now() + TimeDelta::try_hours(1).unwrap())
            .timestamp_nanos_opt()
            .unwrap();
        assert!(!is_within_last_24_hours(UnixNanos::from(future_ns as u64)).unwrap());

        // 未来一天也应返回 false
        let future_ns = (Utc::now() + TimeDelta::try_days(1).unwrap())
            .timestamp_nanos_opt()
            .unwrap();
        assert!(!is_within_last_24_hours(UnixNanos::from(future_ns as u64)).unwrap());
    }

    #[rstest]
    #[case(Utc.with_ymd_and_hms(2024, 3, 31, 12, 0, 0).unwrap(), 1, Utc.with_ymd_and_hms(2024, 2, 29, 12, 0, 0).unwrap())] // 闰年二月
    #[case(Utc.with_ymd_and_hms(2024, 3, 31, 12, 0, 0).unwrap(), 12, Utc.with_ymd_and_hms(2023, 3, 31, 12, 0, 0).unwrap())] // 一年前
    #[case(Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap(), 1, Utc.with_ymd_and_hms(2023, 12, 31, 12, 0, 0).unwrap())] // 回跨至上一年
    #[case(Utc.with_ymd_and_hms(2024, 3, 31, 12, 0, 0).unwrap(), 2, Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap())] // 回退多个月
    fn test_subtract_n_months(
        #[case] input: DateTime<Utc>,
        #[case] months: u32,
        #[case] expected: DateTime<Utc>,
    ) {
        let result = subtract_n_months(input, months).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(Utc.with_ymd_and_hms(2023, 2, 28, 12, 0, 0).unwrap(), 1, Utc.with_ymd_and_hms(2023, 3, 28, 12, 0, 0).unwrap())] // 简单月份加法
    #[case(Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap(), 1, Utc.with_ymd_and_hms(2024, 2, 29, 12, 0, 0).unwrap())] // 闰年二月
    #[case(Utc.with_ymd_and_hms(2023, 12, 31, 12, 0, 0).unwrap(), 1, Utc.with_ymd_and_hms(2024, 1, 31, 12, 0, 0).unwrap())] // 跨至下一年
    #[case(Utc.with_ymd_and_hms(2023, 1, 31, 12, 0, 0).unwrap(), 13, Utc.with_ymd_and_hms(2024, 2, 29, 12, 0, 0).unwrap())] // 跨年并增加多个月
    fn test_add_n_months(
        #[case] input: DateTime<Utc>,
        #[case] months: u32,
        #[case] expected: DateTime<Utc>,
    ) {
        let result = add_n_months(input, months).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_add_n_years_overflow() {
        let datetime = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let err = add_n_years(datetime, u32::MAX).unwrap_err();
        assert!(err.to_string().contains("月份数量溢出"));
    }

    #[rstest]
    fn test_subtract_n_years_overflow() {
        let datetime = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let err = subtract_n_years(datetime, u32::MAX).unwrap_err();
        assert!(err.to_string().contains("月份数量溢出"));
    }

    #[rstest]
    fn test_add_n_years_nanos_overflow() {
        let nanos = UnixNanos::from(0);
        let err = add_n_years_nanos(nanos, u32::MAX).unwrap_err();
        assert!(err.to_string().contains("月份数量溢出"));
    }

    #[rstest]
    #[case(2024, 2, 29)] // 闰年二月
    #[case(2023, 2, 28)] // 非闰年二月
    #[case(2024, 12, 31)] // 十二月
    #[case(2023, 11, 30)] // 十一月
    fn test_last_day_of_month(#[case] year: i32, #[case] month: u32, #[case] expected: u32) {
        let result = last_day_of_month(year, month).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(2024, true)] // 能被 4 整除的闰年
    #[case(1900, false)] // 不是闰年，能被 100 整除但不能被 400 整除
    #[case(2000, true)] // 闰年，能被 400 整除
    #[case(2023, false)] // 非闰年
    fn test_is_leap_year(#[case] year: i32, #[case] expected: bool) {
        let result = is_leap_year(year);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("1970-01-01T00:00:00.000000000Z", 0)] // Unix 纪元
    #[case("1970-01-01T00:00:00.000000001Z", 1)] // 1 纳秒
    #[case("1970-01-01T00:00:00.001000000Z", 1_000_000)] // 1 毫秒
    #[case("1970-01-01T00:00:01.000000000Z", 1_000_000_000)] // 1 秒
    #[case("2023-12-18T00:00:00.000000000Z", 1_702_857_600_000_000_000)] // 特定日期
    #[case("2024-02-10T14:58:43.456789Z", 1_707_577_123_456_789_000)] // 带有小数部分的 RFC3339
    #[case("2024-02-10T14:58:43Z", 1_707_577_123_000_000_000)] // 不带小数部分的 RFC3339
    #[case("2024-02-10", 1_707_523_200_000_000_000)] // 简单日期格式
    fn test_iso8601_to_unix_nanos(#[case] input: &str, #[case] expected: u64) {
        let result = iso8601_to_unix_nanos(input.to_string()).unwrap();
        assert_eq!(result.as_u64(), expected);
    }

    #[rstest]
    #[case("invalid-date")] // 无效格式
    #[case("2024-02-30")] // 无效日期
    #[case("2024-13-01")] // 无效月份
    #[case("not a timestamp")] // 随机字符串
    fn test_iso8601_to_unix_nanos_invalid(#[case] input: &str) {
        let result = iso8601_to_unix_nanos(input.to_string());
        assert!(result.is_err());
    }

    #[rstest]
    fn test_iso8601_roundtrip() {
        let original_nanos = UnixNanos::from(1_707_577_123_456_789_000);
        let iso8601_string = unix_nanos_to_iso8601(original_nanos);
        let parsed_nanos = iso8601_to_unix_nanos(iso8601_string).unwrap();
        assert_eq!(parsed_nanos, original_nanos);
    }

    #[rstest]
    fn test_add_n_years_nanos_normal_case() {
        // 测试从 2020-01-01 开始增加 1 年
        let start = UnixNanos::from(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap());
        let result = add_n_years_nanos(start, 1).unwrap();
        let expected = UnixNanos::from(Utc.with_ymd_and_hms(2021, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_add_n_years_nanos_prevents_negative_timestamp() {
        // 边界情况：确保捕获是否会产生负数时间戳
        // 这是一种防御性检查——在实践中，从有效的 UnixNanos 增加年份不应产生负数时间戳，但我们要验证该检查已就位。
        let start = UnixNanos::from(0); // 纪元
        // 向纪元增加年份绝不应产生负数，但检查确实存在。
        let result = add_n_years_nanos(start, 1);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_datetime_to_unix_nanos_at_epoch() {
        // Unix 纪元 (1970-01-01 00:00:00 UTC) 应返回 0 纳秒
        let epoch = Utc.timestamp_opt(0, 0).unwrap();
        let result = datetime_to_unix_nanos(Some(epoch));
        assert_eq!(result, Some(UnixNanos::from(0)));
    }

    #[rstest]
    fn test_datetime_to_unix_nanos_typical_datetime() {
        let dt = Utc
            .with_ymd_and_hms(2024, 1, 15, 13, 30, 45)
            .unwrap()
            .with_nanosecond(123_456_789)
            .unwrap();
        let result = datetime_to_unix_nanos(Some(dt));

        // 预期: 1705325445123456789 纳秒
        assert!(result.is_some());
        assert_eq!(result.unwrap().as_u64(), 1_705_325_445_123_456_789);
    }

    #[rstest]
    fn test_datetime_to_unix_nanos_before_epoch() {
        // 纪元前的日期时间 (1969-12-31 23:59:59 UTC) 应返回 None
        // 因为负数时间戳无法转换为 u64
        let before_epoch = Utc.with_ymd_and_hms(1969, 12, 31, 23, 59, 59).unwrap();
        let result = datetime_to_unix_nanos(Some(before_epoch));
        assert_eq!(result, None);
    }

    #[rstest]
    fn test_datetime_to_unix_nanos_one_second_after_epoch() {
        // 1970-01-01 00:00:01 UTC = 1_000_000_000 纳秒
        let dt = Utc.timestamp_opt(1, 0).unwrap();
        let result = datetime_to_unix_nanos(Some(dt));
        assert_eq!(result, Some(UnixNanos::from(1_000_000_000)));
    }

    #[rstest]
    fn test_datetime_to_unix_nanos_with_subsecond_precision() {
        // 使用微秒测试: 1970-01-01 00:00:00.000001 UTC
        let dt = Utc.timestamp_opt(0, 1_000).unwrap(); // 1 微秒 = 1000 纳秒
        let result = datetime_to_unix_nanos(Some(dt));
        assert_eq!(result, Some(UnixNanos::from(1_000)));
    }
}
