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

//! 为 `nautilus-core` 中的日期/时间转换工具所做的薄 FFI 封装。
//!
//! Rust 实现已经存在于 `crate::datetime` 之中；此模块仅仅是将这些转换
//! 暴露给 C（并进而通过 Cython 暴露给 Python），同时保持了行为和
//! 文档的一致性。每个导出的函数直接将其调用转发给 Rust 侧对应的实现，
//! 因而继承了同样的语义和安全保证。

use std::ffi::c_char;

use crate::{
    datetime::{unix_nanos_to_iso8601, unix_nanos_to_iso8601_millis},
    ffi::{abort_on_panic, string::str_to_cstr},
};

/// 将 UNIX 纳秒时间戳转换为 ISO 8601 (RFC 3339) 格式的 C 字符串指针。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn unix_nanos_to_iso8601_cstr(timestamp_ns: u64) -> *const c_char {
    abort_on_panic(|| str_to_cstr(&unix_nanos_to_iso8601(timestamp_ns.into())))
}

/// 将 UNIX 纳秒时间戳转换为带毫秒精度的 ISO 8601 (RFC 3339) 格式的 C 字符串指针。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn unix_nanos_to_iso8601_millis_cstr(timestamp_ns: u64) -> *const c_char {
    abort_on_panic(|| str_to_cstr(&unix_nanos_to_iso8601_millis(timestamp_ns.into())))
}

/// 将秒 (s) 转换为纳秒 (ns)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn secs_to_nanos(secs: f64) -> u64 {
    abort_on_panic(|| crate::datetime::secs_to_nanos_unchecked(secs))
}

/// 将秒 (s) 转换为毫秒 (ms)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn secs_to_millis(secs: f64) -> u64 {
    abort_on_panic(|| {
        crate::datetime::secs_to_millis(secs).expect("secs_to_millis: 无效或溢出的输入")
    })
}

/// 将毫秒 (ms) 转换为纳秒 (ns)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn millis_to_nanos(millis: f64) -> u64 {
    abort_on_panic(|| crate::datetime::millis_to_nanos_unchecked(millis))
}

/// 将微秒 (μs) 转换为纳秒 (ns)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn micros_to_nanos(micros: f64) -> u64 {
    abort_on_panic(|| crate::datetime::micros_to_nanos_unchecked(micros))
}

/// 将纳秒 (ns) 转换为秒 (s)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn nanos_to_secs(nanos: u64) -> f64 {
    abort_on_panic(|| crate::datetime::nanos_to_secs(nanos))
}

/// 将纳秒 (ns) 转换为毫秒 (ms)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn nanos_to_millis(nanos: u64) -> u64 {
    abort_on_panic(|| crate::datetime::nanos_to_millis(nanos))
}

/// 将纳秒 (ns) 转换为微秒 (μs)。
#[cfg(feature = "ffi")]
#[unsafe(no_mangle)]
pub extern "C" fn nanos_to_micros(nanos: u64) -> u64 {
    abort_on_panic(|| crate::datetime::nanos_to_micros(nanos))
}
