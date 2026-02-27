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

//! 暴露给 Python 的日期/时间（Date/time）工具封装。

use pyo3::prelude::*;
use pyo3_stub_gen::derive::gen_stub_pyfunction;

use super::to_pyvalue_err;
use crate::{
    UnixNanos,
    datetime::{
        is_within_last_24_hours, last_weekday_nanos, micros_to_nanos, millis_to_nanos,
        nanos_to_micros, nanos_to_millis, nanos_to_secs, secs_to_millis, secs_to_nanos,
        unix_nanos_to_iso8601, unix_nanos_to_iso8601_millis,
    },
};

/// 返回由给定秒数转换而来的、保留整数部分的纳秒 (ns)。
///
/// 参数 (Parameters)
/// ----------
/// secs : float
///     待转换的秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "secs_to_nanos")]
pub fn py_secs_to_nanos(secs: f64) -> PyResult<u64> {
    secs_to_nanos(secs).map_err(to_pyvalue_err)
}

/// 返回由给定秒数转换而来的、保留整数部分的毫秒 (ms)。
///
/// 参数 (Parameters)
/// ----------
/// secs : float
///     待转换的秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "secs_to_millis")]
pub fn py_secs_to_millis(secs: f64) -> PyResult<u64> {
    secs_to_millis(secs).map_err(to_pyvalue_err)
}

/// 返回由给定毫秒 (ms) 转换而来的、保留整数部分的纳秒 (ns)。
///
/// 参数 (Parameters)
/// ----------
/// millis : float
///     待转换的毫秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "millis_to_nanos")]
pub fn py_millis_to_nanos(millis: f64) -> PyResult<u64> {
    millis_to_nanos(millis).map_err(to_pyvalue_err)
}

/// 返回由给定微秒 (μs) 转换而来的、保留整数部分的纳秒 (ns)。
///
/// 参数 (Parameters)
/// ----------
/// micros : float
///     待转换的微秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "micros_to_nanos")]
pub fn py_micros_to_nanos(micros: f64) -> PyResult<u64> {
    micros_to_nanos(micros).map_err(to_pyvalue_err)
}

/// 返回由给定纳秒 (ns) 转换而来的秒数。
///
/// 参数 (Parameters)
/// ----------
/// nanos : int
///     待转换的纳秒数。
///
/// 返回 (Returns)
/// -------
/// float
#[must_use]
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "nanos_to_secs")]
pub fn py_nanos_to_secs(nanos: u64) -> f64 {
    nanos_to_secs(nanos)
}

/// 返回由给定纳秒 (ns) 转换而来的、保留整数部分的毫秒 (ms)。
///
/// 参数 (Parameters)
/// ----------
/// nanos : int
///     待转换的纳秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[must_use]
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "nanos_to_millis")]
pub const fn py_nanos_to_millis(nanos: u64) -> u64 {
    nanos_to_millis(nanos)
}

/// 返回由给定纳秒 (ns) 转换而来的、保留整数部分的微秒 (μs)。
///
/// 参数 (Parameters)
/// ----------
/// nanos : int
///     待转换的纳秒数。
///
/// 返回 (Returns)
/// -------
/// int
#[must_use]
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "nanos_to_micros")]
pub const fn py_nanos_to_micros(nanos: u64) -> u64 {
    nanos_to_micros(nanos)
}

/// 以 ISO 8601 (RFC 3339) 格式字符串的形式返回 UNIX 纳秒。
///
/// 参数 (Parameters)
/// ----------
/// timestamp_ns : int
///     UNIX 时间戳（纳秒）。
/// nanos_precision : bool, 默认 True
///     若为 True，则使用纳秒级精度。若为 False，则使用毫秒级精度。
///
/// 返回 (Returns)
/// -------
/// str
///
/// 抛出 (Raises)
/// ------
/// ValueError
///     如果 `timestamp_ns` 无效。
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(
    name = "unix_nanos_to_iso8601",
    signature = (timestamp_ns, nanos_precision=Some(true))
)]
pub fn py_unix_nanos_to_iso8601(
    timestamp_ns: u64,
    nanos_precision: Option<bool>,
) -> PyResult<String> {
    if timestamp_ns > i64::MAX as u64 {
        return Err(to_pyvalue_err(
            "timestamp_ns 超出了转换范围",
        ));
    }

    let unix_nanos = UnixNanos::from(timestamp_ns);
    let formatted = if nanos_precision.unwrap_or(true) {
        unix_nanos_to_iso8601(unix_nanos)
    } else {
        unix_nanos_to_iso8601_millis(unix_nanos)
    };

    Ok(formatted)
}

/// 返回在上一个工作日 (周一至周五) 午夜 (UTC) 的 UNIX 纳秒。
///
/// 参数 (Parameters)
/// ----------
/// year : int
///     基准日期的年份。
/// month : int
///     基准日期的月份。
/// day : int
///     基准日期的天。
///
/// 返回 (Returns)
/// -------
/// int
///
/// 抛出 (Raises)
/// ------
/// `ValueError`
///     如果给定的日期无效。
///
/// # Errors
///
/// 如果提供的日期无效，则返回 `PyErr`。
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "last_weekday_nanos")]
pub fn py_last_weekday_nanos(year: i32, month: u32, day: u32) -> PyResult<u64> {
    Ok(last_weekday_nanos(year, month, day)
        .map_err(to_pyvalue_err)?
        .as_u64())
}

/// 返回给定的 UNIX 纳秒时间戳是否在过去 24 小时之内。
///
/// 参数 (Parameters)
/// ----------
/// timestamp_ns : int
///     UNIX 纳秒时间戳基准值。
///
/// 返回 (Returns)
/// -------
/// bool
///
/// 抛出 (Raises)
/// ------
/// ValueError
///     如果 `timestamp` 无效。
///
/// # Errors
///
/// 如果提供的时间戳无效，则返回 `PyErr`。
#[gen_stub_pyfunction(module = "nautilus_trader.core")]
#[pyfunction(name = "is_within_last_24_hours")]
pub fn py_is_within_last_24_hours(timestamp_ns: u64) -> PyResult<bool> {
    is_within_last_24_hours(UnixNanos::from(timestamp_ns)).map_err(to_pyvalue_err)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_py_unix_nanos_to_iso8601_errors_on_out_of_range_timestamp() {
        let result = py_unix_nanos_to_iso8601((i64::MAX as u64) + 1, Some(true));
        assert!(result.is_err());
    }

    #[rstest]
    fn test_py_unix_nanos_to_iso8601_formats_valid_timestamp() {
        let output = py_unix_nanos_to_iso8601(0, Some(false)).unwrap();
        assert_eq!(output, "1970-01-01T00:00:00.000Z");
    }
}
