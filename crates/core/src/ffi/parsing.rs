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

//! 辅助函数，用于将常见的 C 类型（主要是以 UTF-8 编码的 `char *` 指针）转换为
//! NautilusTrader 整个系统中所使用的 Rust 数据结构。
//!
//! 转换过程具有以下特征：
//!
//! * JSON 被用作复杂结构的交换格式。
//! * 出于性能考虑，在可能的情况下，`ustr::Ustr` 优于 `String`。
//!
//! 所有函数都带有 `#[must_use]` 标记，并且除非另有说明，否则均**假定**输入指针
//! 非空且指向一个有效的、*以 null 结尾的* UTF-8 字符串。

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
};

use serde_json::Value;
use ustr::Ustr;

use crate::{
    ffi::{abort_on_panic, string::cstr_as_str},
    parsing::{min_increment_precision_from_str, precision_from_str},
};

/// 将 C 字节指针转换为一个拥有的 `Vec<String>`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 为空，包含无效的 UTF-8/JSON，或者该 JSON 值
/// 不是一个字符串数组，则触发 panic。
#[must_use]
pub unsafe fn bytes_to_string_vec(ptr: *const c_char) -> Vec<String> {
    assert!(!ptr.is_null(), "`ptr` 为 NULL");

    // 安全性：根据函数合约，调用方确保 ptr 是有效的
    let c_str = unsafe { CStr::from_ptr(ptr) };
    let bytes = c_str.to_bytes();

    let json_string = std::str::from_utf8(bytes).expect("C 字符串包含无效的 UTF-8");
    let value: serde_json::Value =
        serde_json::from_str(json_string).expect("C 字符串包含无效的 JSON");

    let arr = value
        .as_array()
        .expect("C 字符串 JSON 必须是字符串数组");

    arr.iter()
        .map(|value| {
            value
                .as_str()
                .expect("C 字符串 JSON 数组必须仅包含字符串")
                .to_owned()
        })
        .collect()
}

/// 将 `String` 切片转换为 C 字符串指针（以 JSON 编码）。
///
/// # Panics
///
/// 如果 JSON 序列化失败，或者生成的字符串内部包含 null 字节，则触发 panic。
#[must_use]
pub fn string_vec_to_bytes(strings: &[String]) -> *const c_char {
    let json_string = serde_json::to_string(strings).expect("未能将字符串序列化为 JSON");
    let c_string = CString::new(json_string).expect("JSON 字符串内部包含 null 字节");

    c_string.into_raw()
}

/// 将 C 字节指针转换为一个拥有的 `Option<HashMap<String, Value>>`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 不为空，但包含无效的 UTF-8 或 JSON，则触发 panic。
#[must_use]
pub unsafe fn optional_bytes_to_json(ptr: *const c_char) -> Option<HashMap<String, Value>> {
    if ptr.is_null() {
        None
    } else {
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let c_str = unsafe { CStr::from_ptr(ptr) };
        let bytes = c_str.to_bytes();

        let json_string = std::str::from_utf8(bytes).expect("C 字符串包含无效的 UTF-8");
        let result = serde_json::from_str(json_string).expect("C 字符串包含无效的 JSON");

        Some(result)
    }
}

/// 将 C 字节指针转换为一个拥有的 `Option<HashMap<Ustr, Ustr>>`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 不为空，但包含无效的 UTF-8 或 JSON，则触发 panic。
#[must_use]
pub unsafe fn optional_bytes_to_str_map(ptr: *const c_char) -> Option<HashMap<Ustr, Ustr>> {
    if ptr.is_null() {
        None
    } else {
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let c_str = unsafe { CStr::from_ptr(ptr) };
        let bytes = c_str.to_bytes();

        let json_string = std::str::from_utf8(bytes).expect("C 字符串包含无效的 UTF-8");
        let result = serde_json::from_str(json_string).expect("C 字符串包含无效的 JSON");

        Some(result)
    }
}

/// 将 C 字节指针转换为一个拥有的 `Option<Vec<String>>`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 不为空，但包含无效的 UTF-8 或 JSON，则触发 panic。
#[must_use]
pub unsafe fn optional_bytes_to_str_vec(ptr: *const c_char) -> Option<Vec<String>> {
    if ptr.is_null() {
        None
    } else {
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let c_str = unsafe { CStr::from_ptr(ptr) };
        let bytes = c_str.to_bytes();

        let json_string = std::str::from_utf8(bytes).expect("C 字符串包含无效的 UTF-8");
        let result = serde_json::from_str(json_string).expect("C 字符串包含无效的 JSON");

        Some(result)
    }
}

/// 返回从给定 C 字符串推断出的十进制精度。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn precision_from_cstr(ptr: *const c_char) -> u8 {
    abort_on_panic(|| {
        assert!(!ptr.is_null(), "`ptr` 为 NULL");
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let s = unsafe { cstr_as_str(ptr) };
        precision_from_str(s)
    })
}

/// 返回从给定 C 字符串推断出的最小价格增量的十进制精度。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn min_increment_precision_from_cstr(ptr: *const c_char) -> u8 {
    abort_on_panic(|| {
        assert!(!ptr.is_null(), "`ptr` 为 NULL");
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let s = unsafe { cstr_as_str(ptr) };
        min_increment_precision_from_str(s)
    })
}

/// 从给定的 `u8` 返回其对应的 `bool` 值。
#[must_use]
pub const fn u8_as_bool(value: u8) -> bool {
    value != 0
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_optional_bytes_to_json_null() {
        let ptr = std::ptr::null();
        let result = unsafe { optional_bytes_to_json(ptr) };
        assert_eq!(result, None);
    }

    #[rstest]
    fn test_optional_bytes_to_json_empty() {
        let json_str = CString::new("{}").unwrap();
        let ptr = json_str.as_ptr().cast::<c_char>();
        let result = unsafe { optional_bytes_to_json(ptr) };
        assert_eq!(result, Some(HashMap::new()));
    }

    #[rstest]
    fn test_string_vec_to_bytes_valid() {
        let strings = vec!["value1", "value2", "value3"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>();

        let ptr = string_vec_to_bytes(&strings);

        let result = unsafe { bytes_to_string_vec(ptr) };
        assert_eq!(result, strings);
    }

    #[rstest]
    fn test_string_vec_to_bytes_empty() {
        let strings = Vec::new();
        let ptr = string_vec_to_bytes(&strings);

        let result = unsafe { bytes_to_string_vec(ptr) };
        assert_eq!(result, strings);
    }

    #[rstest]
    fn test_bytes_to_string_vec_valid() {
        let json_str = CString::new(r#"["value1", "value2", "value3"]"#).unwrap();
        let ptr = json_str.as_ptr().cast::<c_char>();
        let result = unsafe { bytes_to_string_vec(ptr) };

        let expected_vec = vec!["value1", "value2", "value3"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<String>>();

        assert_eq!(result, expected_vec);
    }

    #[rstest]
    #[should_panic(expected = "array must contain only strings")]
    fn test_bytes_to_string_vec_invalid() {
        let json_str = CString::new(r#"["value1", 42, "value3"]"#).unwrap();
        let ptr = json_str.as_ptr().cast::<c_char>();
        let _ = unsafe { bytes_to_string_vec(ptr) };
    }

    #[rstest]
    fn test_optional_bytes_to_json_valid() {
        let json_str = CString::new(r#"{"key1": "value1", "key2": 2}"#).unwrap();
        let ptr = json_str.as_ptr().cast::<c_char>();
        let result = unsafe { optional_bytes_to_json(ptr) };
        let mut expected_map = HashMap::new();
        expected_map.insert("key1".to_owned(), Value::String("value1".to_owned()));
        expected_map.insert(
            "key2".to_owned(),
            Value::Number(serde_json::Number::from(2)),
        );
        assert_eq!(result, Some(expected_map));
    }

    #[rstest]
    #[should_panic(expected = "C string contains invalid JSON")]
    fn test_optional_bytes_to_json_invalid() {
        let json_str = CString::new(r#"{"key1": "value1", "key2": }"#).unwrap();
        let ptr = json_str.as_ptr().cast::<c_char>();
        let _result = unsafe { optional_bytes_to_json(ptr) };
    }

    #[rstest]
    #[case("1e8", 0)]
    #[case("123", 0)]
    #[case("123.45", 2)]
    #[case("123.456789", 6)]
    #[case("1.23456789e-2", 2)]
    #[case("1.23456789e-12", 12)]
    fn test_precision_from_cstr(#[case] input: &str, #[case] expected: u8) {
        let c_str = CString::new(input).unwrap();
        assert_eq!(unsafe { precision_from_cstr(c_str.as_ptr()) }, expected);
    }
}
