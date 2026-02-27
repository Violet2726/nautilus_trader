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

//! 用于在 FFI 边界之间安全传输 UTF-8 字符串的工具函数。
//!
//! Rust 与 C/C++/Python 之间的互操作性通常需要指向*以 null 结尾的*字符串的原始指针。
//! 此模块提供的便捷辅助函数可以：
//!
//! * 将原始 `*const c_char` 指针转换为 Rust [`String`]、[`&str`]、字节切片或
//!   `ustr::Ustr` 值。
//! * 在 Rust 侧需要将字符串所有权移交给外部代码时，执行相反的转换。
//!
//! 由于这些函数接受原始指针，且依赖调用方维持基本的不变性（指针有效性、生命周期、UTF-8 正确性），
//! 因此其中大部分都被标记为 `unsafe`。每个函数都详细记录了特定的安全性要求。

use std::{
    ffi::{CStr, CString, c_char},
    str,
};

#[cfg(feature = "python")]
use pyo3::{Bound, Python, ffi};
use ustr::Ustr;

use crate::ffi::abort_on_panic;

#[cfg(feature = "python")]
/// 从一个有效的 Python 对象指针返回一个拥有的字符串。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是从一个有效的 Python UTF-8 `str` 中借用的。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[must_use]
pub unsafe fn pystr_to_string(ptr: *mut ffi::PyObject) -> String {
    assert!(!ptr.is_null(), "`ptr` 为 NULL");
    // 安全性：调用方确保 ptr 是从一个有效的 Python UTF-8 str 中借用的
    Python::attach(|py| unsafe { Bound::from_borrowed_ptr(py, ptr).to_string() })
}

/// 将 C 字符串指针转换为一个拥有的 `Ustr`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[must_use]
pub unsafe fn cstr_to_ustr(ptr: *const c_char) -> Ustr {
    assert!(!ptr.is_null(), "`ptr` 为 NULL");
    // 安全性：根据函数合约，调用方确保 ptr 是有效的
    let cstr = unsafe { CStr::from_ptr(ptr) };
    Ustr::from(cstr.to_str().expect("CStr::from_ptr 失败"))
}

/// 将 C 字符串指针转换为一个借用的字节切片。
///
/// # 安全性 (Safety)
///
/// - 假定 `ptr` 是一个指向以 null 结尾的、有效的 UTF-8 C 字符串的指针。
/// - 返回的切片借用了底层的分配空间；调用方必须确保 C 缓冲区的生命周期
///   长于对该切片的每一次使用。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[must_use]
pub unsafe fn cstr_to_bytes<'a>(ptr: *const c_char) -> &'a [u8] {
    assert!(!ptr.is_null(), "`ptr` 为 NULL");
    // 安全性：根据函数合约，调用方确保 ptr 是有效的
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_bytes()
}

/// 将 C 字符串指针转换为一个拥有的 `Option<Ustr>`。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针或为 NULL。
///
/// # Panics
///
/// 如果 `ptr` 不为空但不是有效的 UTF-8 C 字符串，则触发 panic。
#[must_use]
pub unsafe fn optional_cstr_to_ustr(ptr: *const c_char) -> Option<Ustr> {
    if ptr.is_null() {
        None
    } else {
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        Some(unsafe { cstr_to_ustr(ptr) })
    }
}

/// 将 C 字符串指针转换为一个借用的字符串切片。
///
/// # 安全性 (Safety)
///
/// - 假定 `ptr` 是一个指向以 null 结尾的、有效的 UTF-8 C 字符串的指针。
/// - 返回的 `&str` 借用了底层的分配空间；调用方必须确保 C 缓冲区的生命周期
///   长于对该字符串切片的每一次使用。
///
/// # Panics
///
/// 如果 `ptr` 为空或包含无效的 UTF-8 编码，则触发 panic。
#[must_use]
pub unsafe fn cstr_as_str<'a>(ptr: *const c_char) -> &'a str {
    assert!(!ptr.is_null(), "`ptr` 为 NULL");
    // 安全性：根据函数合约，调用方确保 ptr 是有效的
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().expect("C 字符串包含无效的 UTF-8 编码")
}

/// 将可选的 C 字符串指针转换为 `Option<&str>`。
///
/// # 安全性 (Safety)
///
/// - 假定 `ptr` 是一个指向以 null 结尾的、有效的 UTF-8 C 字符串指针或为 NULL。
/// - 任何借用的字符串其生命周期都不能长于底层的分配空间。
///
/// # Panics
///
/// 如果 `ptr` 不为空但包含无效的 UTF-8 编码，则触发 panic。
#[must_use]
pub unsafe fn optional_cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        None
    } else {
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        Some(unsafe { cstr_as_str(ptr) })
    }
}

/// 基于 [`&str`] 为新分配的内存创建一个 C 字符串指针。
///
/// # Panics
///
/// 如果输入字符串内部包含 null 字节，则触发 panic。
#[must_use]
pub fn str_to_cstr(s: &str) -> *const c_char {
    CString::new(s).expect("CString::new 失败").into_raw()
}

/// 释放指针指向的 C 字符串内存。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 为空，则触发 panic。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cstr_drop(ptr: *const c_char) {
    abort_on_panic(|| {
        assert!(!ptr.is_null(), "`ptr` 为 NULL");
        // 安全性：调用方确保 ptr 是由 str_to_cstr 分配的
        let cstring = unsafe { CString::from_raw(ptr.cast_mut()) };
        drop(cstring);
    });
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "python")]
    use pyo3::types::PyString;
    use rstest::*;

    use super::*;

    #[cfg(feature = "python")]
    #[cfg_attr(miri, ignore)]
    #[rstest]
    fn test_pystr_to_string() {
        Python::initialize();
        // 创建一个有效的 Python 对象指针
        let ptr = Python::attach(|py| PyString::new(py, "test string1").as_ptr());
        let result = unsafe { pystr_to_string(ptr) };
        assert_eq!(result, "test string1");
    }

    #[cfg(feature = "python")]
    #[rstest]
    #[should_panic(expected = "`ptr` 为 NULL")]
    fn test_pystr_to_string_with_null_ptr() {
        // 创建一个空的 Python 对象指针
        let ptr: *mut ffi::PyObject = std::ptr::null_mut();
        unsafe {
            let _ = pystr_to_string(ptr);
        };
    }

    #[rstest]
    fn test_cstr_as_str() {
        // 创建一个有效的 C 字符串指针
        let c_string = CString::new("test string2").expect("CString::new 失败");
        let ptr = c_string.as_ptr();
        let result = unsafe { cstr_as_str(ptr) };
        assert_eq!(result, "test string2");
    }

    #[rstest]
    fn test_cstr_to_bytes() {
        // 创建一个有效的 C 字符串
        let sample_c_string = CString::new("Hello, world!").expect("CString::new 失败");
        let cstr_ptr = sample_c_string.as_ptr();
        let result = unsafe { cstr_to_bytes(cstr_ptr) };
        assert_eq!(result, b"Hello, world!");
        assert_eq!(result.len(), 13);
    }

    #[rstest]
    #[should_panic(expected = "`ptr` 为 NULL")]
    fn test_cstr_to_bytes_with_null_ptr() {
        // 创建一个空的 C 字符串指针
        let ptr: *const c_char = std::ptr::null();
        unsafe {
            let _ = cstr_to_bytes(ptr);
        };
    }

    #[rstest]
    fn test_optional_cstr_to_str_with_null_ptr() {
        // 使用空指针调用 optional_cstr_to_str
        let ptr = std::ptr::null();
        let result = unsafe { optional_cstr_to_str(ptr) };
        assert!(result.is_none());
    }

    #[rstest]
    fn test_optional_cstr_to_str_with_valid_ptr() {
        // 创建一个有效的 C 字符串
        let input_str = "hello world";
        let c_str = CString::new(input_str).expect("CString::new 失败");
        let result = unsafe { optional_cstr_to_str(c_str.as_ptr()) };
        assert!(result.is_some());
        assert_eq!(result.unwrap(), input_str);
    }

    #[rstest]
    fn test_string_to_cstr() {
        let s = "test string";
        let c_str_ptr = str_to_cstr(s);
        let c_str = unsafe { CStr::from_ptr(c_str_ptr) };
        let result = c_str.to_str().expect("CStr::from_ptr 失败");
        assert_eq!(result, s);
    }

    #[rstest]
    fn test_cstr_drop() {
        let c_string = CString::new("test string3").expect("CString::new 失败");
        let ptr = c_string.into_raw(); // <-- 注意：指针“必须”通过这种方式获取
        unsafe { cstr_drop(ptr) };
    }
}
