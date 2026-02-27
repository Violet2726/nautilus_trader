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

//! 为 [`UUID4`] 封装类型提供的 FFI 辅助函数。
//!
//! 此处导出的函数使得 C/Python 代码能够创建、比较以及哈希 UUID 值，
//! 而无需去了解 NautilusTrader 所采用的内部表示形式。

use std::{
    collections::hash_map::DefaultHasher,
    ffi::{CStr, c_char},
    hash::{Hash, Hasher},
};

use crate::{UUID4, ffi::abort_on_panic};

/// 生成一个新的随机（版本 4）UUID 并在返回值中返回。
#[unsafe(no_mangle)]
pub extern "C" fn uuid4_new() -> UUID4 {
    abort_on_panic(UUID4::new)
}

/// 从 C 字符串指针返回一个 [`UUID4`]。
///
/// # 安全性 (Safety)
///
/// 假定 `ptr` 是一个有效的 C 字符串指针。
///
/// # Panics
///
/// 如果 `ptr` 无法被转换为有效的 C 字符串，则触发 panic。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn uuid4_from_cstr(ptr: *const c_char) -> UUID4 {
    abort_on_panic(|| {
        assert!(!ptr.is_null(), "`ptr` 为 NULL");
        // 安全性：根据函数合约，调用方确保 ptr 是有效的
        let cstr = unsafe { CStr::from_ptr(ptr) };
        let value = cstr.to_str().expect("未能将 C 字符串转换为 UTF-8 编码");
        UUID4::from(value)
    })
}

/// 返回一个借用的、*以 null 结尾的* 表示 `uuid` 的 UTF-8 C 字符串。
///
/// 该指针的有效性与输入的 `UUID4` 引用所具有的生命周期相同 —— 调用方**不得**尝试释放它。
#[unsafe(no_mangle)]
pub extern "C" fn uuid4_to_cstr(uuid: &UUID4) -> *const c_char {
    abort_on_panic(|| uuid.to_cstr().as_ptr())
}

/// 比较两个 UUID 值。相等时返回 `1`，否则返回 `0`。
#[unsafe(no_mangle)]
pub extern "C" fn uuid4_eq(lhs: &UUID4, rhs: &UUID4) -> u8 {
    abort_on_panic(|| u8::from(lhs == rhs))
}

/// 使用 Rust 的默认哈希器计算 `uuid` 稳定的 [`u64`] 哈希值。
#[unsafe(no_mangle)]
pub extern "C" fn uuid4_hash(uuid: &UUID4) -> u64 {
    abort_on_panic(|| {
        let mut h = DefaultHasher::new();
        uuid.hash(&mut h);
        h.finish()
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use rstest::*;
    use uuid::{self, Uuid};

    use super::*;

    #[rstest]
    fn test_new() {
        let uuid = uuid4_new();
        let uuid_string = uuid.to_string();
        let uuid_parsed = Uuid::parse_str(&uuid_string).expect("Uuid::parse_str 失败");
        assert_eq!(uuid_parsed.get_version().unwrap(), uuid::Version::Random);
    }

    #[rstest]
    fn test_from_cstr() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid_cstring = CString::new(uuid_string).expect("CString::new 失败");
        let uuid_ptr = uuid_cstring.as_ptr();
        let uuid = unsafe { uuid4_from_cstr(uuid_ptr) };
        assert_eq!(uuid_string, uuid.to_string());
    }

    #[rstest]
    fn test_to_cstr() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);
        let uuid_ptr = uuid4_to_cstr(&uuid);
        let uuid_cstr = unsafe { CStr::from_ptr(uuid_ptr) };
        let uuid_result_string = uuid_cstr.to_str().expect("CStr::to_str 失败").to_string();
        assert_eq!(uuid_string, uuid_result_string);
    }

    #[rstest]
    fn test_eq() {
        let uuid1 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c757");
        let uuid2 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c757");
        let uuid3 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c758");
        assert_eq!(uuid4_eq(&uuid1, &uuid2), 1);
        assert_eq!(uuid4_eq(&uuid1, &uuid3), 0);
    }

    #[rstest]
    fn test_hash() {
        let uuid1 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c757");
        let uuid2 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c757");
        let uuid3 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c758");
        assert_eq!(uuid4_hash(&uuid1), uuid4_hash(&uuid2));
        assert_ne!(uuid4_hash(&uuid1), uuid4_hash(&uuid3));
    }
}
