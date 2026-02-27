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

//! 一种用于高效存储标识符的栈分配 ASCII 字符串类型。
//!
//! 此模块提供了 [`StackStr`]，这是一种为短标识符字符串优化的固定容量字符串类型。适用于以下场景：
//!
//! - 已知字符串较短（≤36 个字符）。
//! - 相比堆分配，更倾向于栈分配。
//! - 需要 `Copy` 语义。
//! - 需要 C FFI 兼容性。
//!
//! # 关于 ASCII 的要求
//!
//! `StackStr` 仅接受 ASCII 字符串。这保证了 1 个字符等于 1 个字节，
//! 确保缓冲区始终能容纳刚好其容量大小的字符数。这与标识符通常本质上是 ASCII 的惯例相吻合。
//!
//! | 属性              | ASCII    | UTF-8               |
//! |-------------------|----------|---------------------|
//! | 每字符字节数      | 始终为 1 | 1-4                 |
//! | 36 字节可容纳     | 36 字符  | 9-36 字符           |
//! | 任意字节处切片   | 安全     | 可能切断码点 (codepoint) |
//! | `len()` == 字符数 | 是       | 否                  |

// 处理 C FFI 指针以及未经检查的 UTF-8/CStr 转换所需
#![allow(unsafe_code)]

use std::{
    borrow::Borrow,
    cmp::Ordering,
    ffi::{CStr, c_char},
    fmt::{Debug, Display},
    hash::{Hash, Hasher},
    ops::Deref,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::correctness::FAILED;

/// [`StackStr`] 的最大字符容量。
pub const STACKSTR_CAPACITY: usize = 36;

/// 包含 null 终止符在内的固定缓冲区大小（容量 + 1）。
const STACKSTR_BUFFER_SIZE: usize = STACKSTR_CAPACITY + 1;

/// 一种栈分配的 ASCII 字符串，最大容量为 36 个字符。
///
/// 针对短标识符字符串进行了以下优化：
/// - 栈分配（无堆分配）。
/// - `Copy` 语义。
/// - O(1) 时间复杂度的长度访问。
/// - C FFI 兼容性（以 null 结尾）。
///
/// 要求 ASCII 是为了保证 1 个字符等于 1 个字节，确保缓冲区始终能够容纳刚好其容量大小的字符数。
/// 这与标识符通常本质上是 ASCII 的惯例相吻合。
///
/// # 内存布局
///
/// `value` 字段被放置在最前面，因此结构体指针等于字符串指针，
/// 使得 C FFI 更加自然：`(char*)&stack_str` 可以直接工作。
#[derive(Clone, Copy)]
#[repr(C)]
pub struct StackStr {
    /// 带有 null 终止符的 ASCII 数据，用于 C FFI。
    value: [u8; 37], // STACKSTR_CAPACITY + 1
    /// 字符串的字节长度 (0-36)。
    len: u8,
}

impl StackStr {
    /// 最大字符长度。
    pub const MAX_LEN: usize = STACKSTR_CAPACITY;

    /// 从字符串切片创建一个新的 [`StackStr`]。
    ///
    /// # Panics
    ///
    /// 在以下情况下触发 panic：
    /// - `s` 为空或仅包含空白字符。
    /// - `s` 包含非 ASCII 字符或内部含有 NUL 字节。
    /// - `s` 超过了 36 个字符。
    #[must_use]
    pub fn new(s: &str) -> Self {
        Self::new_checked(s).expect(FAILED)
    }

    /// 创建一个带验证的新 [`StackStr`]，失败时返回错误。
    ///
    /// # Errors
    ///
    /// 在以下情况下返回错误：
    /// - `s` 为空或仅包含空白字符。
    /// - `s` 包含非 ASCII 字符或内部含有 NUL 字节。
    /// - `s` 超过了 36 个字符。
    pub fn new_checked(s: &str) -> anyhow::Result<Self> {
        if s.is_empty() {
            anyhow::bail!("字符串为空");
        }

        if s.len() > STACKSTR_CAPACITY {
            anyhow::bail!(
                "字符串超过了最大长度限制 {} 字符，实际为 {}",
                STACKSTR_CAPACITY,
                s.len()
            );
        }

        if !s.is_ascii() {
            anyhow::bail!("字符串包含非 ASCII 字符");
        }

        let bytes = s.as_bytes();
        if bytes.contains(&0) {
            anyhow::bail!("字符串内部包含 NUL 字节");
        }

        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            anyhow::bail!("字符串仅包含空白字符");
        }

        let mut value = [0u8; STACKSTR_BUFFER_SIZE];
        value[..s.len()].copy_from_slice(bytes);
        // Null 终止符已设置（数组初始化为 0）

        Ok(Self {
            value,
            len: s.len() as u8,
        })
    }

    /// 从字节切片创建一个 [`StackStr`]。
    ///
    /// # Errors
    ///
    /// 在以下情况下返回错误：
    /// - `bytes` 为空或仅包含空白字符。
    /// - `bytes` 包含非 ASCII 字符或内部含有 NUL 字节。
    /// - `bytes` 超过了 36 字节（不包括末尾的 null 终止符）。
    pub fn from_bytes(bytes: &[u8]) -> anyhow::Result<Self> {
        // 如果存在末尾的 null 终止符，则将其剥离
        let bytes = if bytes.last() == Some(&0) {
            &bytes[..bytes.len() - 1]
        } else {
            bytes
        };

        let s = std::str::from_utf8(bytes).map_err(|e| anyhow::anyhow!("无效的 UTF-8 编码：{e}"))?;

        Self::new_checked(s)
    }

    /// 从 C 字符串指针创建一个 [`StackStr`]。
    ///
    /// 对于来自 C 代码的不可信输入，请使用 [`from_c_ptr_checked`](Self::from_c_ptr_checked)
    /// 以避免跨越 FFI 边界时触发 panic。
    ///
    /// # Safety
    ///
    /// - `ptr` 必须是一个指向以 null 结尾的有效 C 字符串指针。
    /// - 字符串必须仅包含有效 ASCII（且不得含有内部 NUL 字节）。
    /// - 字符串不得超过 36 个字符。
    ///
    /// 违反这些要求将导致 panic。如果此函数是从 C 代码调用的，此类 panic 属于未定义行为。
    #[must_use]
    pub unsafe fn from_c_ptr(ptr: *const c_char) -> Self {
        // 安全性：调用者需保证 ptr 有效且以 null 结尾
        let cstr = unsafe { CStr::from_ptr(ptr) };
        let s = cstr.to_str().expect("C 字符串中包含无效的 UTF-8 编码");
        Self::new(s)
    }

    /// 从带验证的 C 字符串指针创建一个 [`StackStr`]。
    ///
    /// 如果字符串无效，则返回 `None`。此函数可以安全地从 C 代码调用，
    /// 因为它永远不会因无效输入而触发 panic。
    ///
    /// # Safety
    ///
    /// - `ptr` 必须是一个指向以 null 结尾的有效 C 字符串指针。
    #[must_use]
    pub unsafe fn from_c_ptr_checked(ptr: *const c_char) -> Option<Self> {
        // 安全性：调用者需保证 ptr 有效且以 null 结尾
        let cstr = unsafe { CStr::from_ptr(ptr) };
        let s = cstr.to_str().ok()?;
        Self::new_checked(s).ok()
    }

    /// 将字符串作为 `&str` 返回。
    ///
    /// 这是一个 O(1) 操作。
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        debug_assert!(
            self.len as usize <= STACKSTR_CAPACITY,
            "StackStr 长度 {} 超过了容量 {}",
            self.len,
            STACKSTR_CAPACITY
        );
        // 安全性：我们通过在构造时的 check_valid_string_ascii 保证了仅存储有效的 ASCII。
        // ASCII 始终是有效的 UTF-8。
        unsafe { std::str::from_utf8_unchecked(&self.value[..self.len as usize]) }
    }

    /// 返回以字节为单位的长度（对于 ASCII，等同于字符数）。
    ///
    /// 这是一个 O(1) 操作。
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// 如果字符串为空，则返回 `true`。
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 返回一个指向以 null 结尾的 C 字符串指针。
    #[inline]
    #[must_use]
    pub const fn as_ptr(&self) -> *const c_char {
        self.value.as_ptr().cast::<c_char>()
    }

    /// 将该值作为 C 字符串切片返回。
    #[inline]
    #[must_use]
    pub fn as_cstr(&self) -> &CStr {
        debug_assert!(
            self.len as usize <= STACKSTR_CAPACITY,
            "StackStr 长度 {} 超过了容量 {}",
            self.len,
            STACKSTR_CAPACITY
        );
        debug_assert!(
            self.value[self.len as usize] == 0,
            "StackStr 在位置 {} 处缺少 null 终止符",
            self.len
        );
        // 安全性：我们保证字符串是以 null 结尾的（缓冲区初始化为 0，
        // 且我们最多只写入 len 个字节，从而保持 null 终止符完好），
        // 且没有内部 NUL 字节（在构造时会被拒绝）。
        unsafe { CStr::from_bytes_with_nul_unchecked(&self.value[..=self.len as usize]) }
    }
}

impl PartialEq for StackStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.value[..self.len as usize] == other.value[..other.len as usize]
    }
}

impl Eq for StackStr {}

impl Hash for StackStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // 仅对实际内容进行哈希，不包括填充部分
        self.value[..self.len as usize].hash(state);
    }
}

impl Ord for StackStr {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for StackStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for StackStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Debug for StackStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl Serialize for StackStr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StackStr {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        Self::new_checked(s).map_err(serde::de::Error::custom)
    }
}

impl From<&str> for StackStr {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl AsRef<str> for StackStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for StackStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Default for StackStr {
    /// 创建一个长度为 0 的空 [`StackStr`]。
    ///
    /// 注意：虽然 [`StackStr::new`] 拒绝空字符串，但 `default()` 会创建一个空的占位符。
    /// 使用 [`is_empty`](StackStr::is_empty) 来检查这种状态。
    fn default() -> Self {
        Self {
            value: [0u8; STACKSTR_BUFFER_SIZE],
            len: 0,
        }
    }
}

impl Deref for StackStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl PartialEq<&str> for StackStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for StackStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl TryFrom<&[u8]> for StackStr {
    type Error = anyhow::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Self::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::{DefaultHasher, Hasher};

    use ahash::AHashMap;
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_new_valid() {
        let s = StackStr::new("hello");
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
    }

    #[rstest]
    fn test_max_length() {
        let input = "x".repeat(36);
        let s = StackStr::new(&input);
        assert_eq!(s.len(), 36);
        assert_eq!(s.as_str(), input);
    }

    #[rstest]
    #[should_panic]
    fn test_exceeds_max_length() {
        let input = "x".repeat(37);
        let _ = StackStr::new(&input);
    }

    #[rstest]
    #[should_panic]
    fn test_empty_string() {
        let _ = StackStr::new("");
    }

    #[rstest]
    #[should_panic]
    fn test_whitespace_only() {
        let _ = StackStr::new("   ");
    }

    #[rstest]
    #[should_panic]
    fn test_non_ascii() {
        let _ = StackStr::new("hello\u{1F600}"); // 表情符号
    }

    #[rstest]
    #[should_panic]
    fn test_interior_nul_byte() {
        let _ = StackStr::new("abc\0def");
    }

    #[rstest]
    fn test_interior_nul_byte_checked() {
        let result = StackStr::new_checked("abc\0def");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NUL"));
    }

    #[rstest]
    fn test_from_c_ptr_checked_valid() {
        let cstring = std::ffi::CString::new("hello").unwrap();
        let s = unsafe { StackStr::from_c_ptr_checked(cstring.as_ptr()) };
        assert!(s.is_some());
        assert_eq!(s.unwrap().as_str(), "hello");
    }

    #[rstest]
    fn test_from_c_ptr_checked_too_long() {
        let long = "x".repeat(37);
        let cstring = std::ffi::CString::new(long).unwrap();
        let s = unsafe { StackStr::from_c_ptr_checked(cstring.as_ptr()) };
        assert!(s.is_none());
    }

    #[rstest]
    fn test_equality() {
        let a = StackStr::new("test");
        let b = StackStr::new("test");
        let c = StackStr::new("other");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[rstest]
    fn test_hash_consistency() {
        use std::hash::DefaultHasher;

        let a = StackStr::new("test");
        let b = StackStr::new("test");

        let hash_a = {
            let mut h = DefaultHasher::new();
            a.hash(&mut h);
            h.finish()
        };
        let hash_b = {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        };

        assert_eq!(hash_a, hash_b);
    }

    #[rstest]
    fn test_hashmap_usage() {
        let mut map = AHashMap::new();
        map.insert(StackStr::new("key1"), 1);
        map.insert(StackStr::new("key2"), 2);

        assert_eq!(map.get(&StackStr::new("key1")), Some(&1));
        assert_eq!(map.get(&StackStr::new("key2")), Some(&2));
        assert_eq!(map.get(&StackStr::new("key3")), None);
    }

    #[rstest]
    fn test_ordering() {
        let a = StackStr::new("aaa");
        let b = StackStr::new("bbb");
        assert!(a < b);
        assert!(b > a);
    }

    #[rstest]
    fn test_c_compatibility() {
        let s = StackStr::new("test");
        let cstr = s.as_cstr();
        assert_eq!(cstr.to_str().unwrap(), "test");
    }

    #[rstest]
    fn test_as_ptr() {
        let s = StackStr::new("test");
        let ptr = s.as_ptr();
        assert!(!ptr.is_null());

        let cstr = unsafe { CStr::from_ptr(ptr) };
        assert_eq!(cstr.to_str().unwrap(), "test");
    }

    #[rstest]
    fn test_from_bytes() {
        let s = StackStr::from_bytes(b"hello").unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[rstest]
    fn test_from_bytes_with_null() {
        let s = StackStr::from_bytes(b"hello\0").unwrap();
        assert_eq!(s.as_str(), "hello");
    }

    #[rstest]
    fn test_serde_roundtrip() {
        let original = StackStr::new("test123");
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"test123\"");

        let deserialized: StackStr = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[rstest]
    fn test_display() {
        let s = StackStr::new("hello");
        assert_eq!(format!("{s}"), "hello");
    }

    #[rstest]
    fn test_debug() {
        let s = StackStr::new("hello");
        assert_eq!(format!("{s:?}"), "\"hello\"");
    }

    #[rstest]
    fn test_from_str() {
        let s: StackStr = "hello".into();
        assert_eq!(s.as_str(), "hello");
    }

    #[rstest]
    fn test_as_ref() {
        let s = StackStr::new("hello");
        let r: &str = s.as_ref();
        assert_eq!(r, "hello");
    }

    #[rstest]
    fn test_borrow() {
        let s = StackStr::new("hello");
        let b: &str = s.borrow();
        assert_eq!(b, "hello");
    }

    #[rstest]
    fn test_default() {
        let s = StackStr::default();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[rstest]
    fn test_copy_semantics() {
        let a = StackStr::new("test");
        let b = a; // 复制，而非移动
        assert_eq!(a, b); // 两者均依然有效
    }

    #[rstest]
    #[case("BINANCE")]
    #[case("ETH-PERP")]
    #[case("O-20231215-001")]
    #[case("123456789012345678901234567890123456")] // 36 字符（最大）
    fn test_valid_identifiers(#[case] s: &str) {
        let stack_str = StackStr::new(s);
        assert_eq!(stack_str.as_str(), s);
    }

    #[rstest]
    fn test_single_char() {
        let s = StackStr::new("x");
        assert_eq!(s.len(), 1);
        assert_eq!(s.as_str(), "x");
    }

    #[rstest]
    fn test_length_35() {
        let input = "x".repeat(35);
        let s = StackStr::new(&input);
        assert_eq!(s.len(), 35);
    }

    #[rstest]
    fn test_length_36_exact() {
        let input = "x".repeat(36);
        let s = StackStr::new(&input);
        assert_eq!(s.len(), 36);
        assert_eq!(s.as_str(), input);
    }

    #[rstest]
    fn test_length_37_rejected() {
        let input = "x".repeat(37);
        let result = StackStr::new_checked(&input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds"));
    }

    #[rstest]
    fn test_struct_size() {
        assert_eq!(std::mem::size_of::<StackStr>(), 38);
    }

    #[rstest]
    fn test_value_field_at_offset_zero() {
        let s = StackStr::new("hello");
        let struct_ptr = std::ptr::from_ref(&s).cast::<u8>();
        let first_byte = unsafe { *struct_ptr };
        assert_eq!(first_byte, b'h');
    }

    #[rstest]
    fn test_null_terminator_present() {
        let s = StackStr::new("test");
        let ptr = s.as_ptr();
        // 读取位置 4 的字节（在 "test" 之后）
        let null_byte = unsafe { *ptr.offset(4) };
        assert_eq!(null_byte, 0);
    }

    #[rstest]
    fn test_from_bytes_empty() {
        let result = StackStr::from_bytes(b"");
        assert!(result.is_err());
    }

    #[rstest]
    fn test_from_bytes_interior_nul() {
        let result = StackStr::from_bytes(b"abc\0def");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NUL"));
    }

    #[rstest]
    fn test_from_bytes_non_ascii() {
        let result = StackStr::from_bytes(&[0x80, 0x81]); // 非 ASCII 字节
        assert!(result.is_err());
    }

    #[rstest]
    fn test_from_bytes_too_long() {
        let bytes = [b'x'; 55];
        let result = StackStr::from_bytes(&bytes);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_from_bytes_whitespace_only() {
        let result = StackStr::from_bytes(b"   ");
        assert!(result.is_err());
    }

    #[rstest]
    fn test_hash_differs_for_different_content() {
        let a = StackStr::new("abc");
        let b = StackStr::new("xyz");

        let hash_a = {
            let mut h = DefaultHasher::new();
            a.hash(&mut h);
            h.finish()
        };
        let hash_b = {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        };

        assert_ne!(hash_a, hash_b);
    }

    #[rstest]
    fn test_hash_ignores_padding() {
        let a = StackStr::new("test");
        let b = StackStr::new("test");

        let hash_a = {
            let mut h = DefaultHasher::new();
            a.hash(&mut h);
            h.finish()
        };
        let hash_b = {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        };

        assert_eq!(hash_a, hash_b);
    }

    #[rstest]
    fn test_serde_deserialize_too_long() {
        let long = format!("\"{}\"", "x".repeat(55));
        let result: Result<StackStr, _> = serde_json::from_str(&long);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_serde_deserialize_empty() {
        let result: Result<StackStr, _> = serde_json::from_str("\"\"");
        assert!(result.is_err());
    }

    #[rstest]
    fn test_serde_deserialize_non_ascii() {
        let result: Result<StackStr, _> = serde_json::from_str("\"hello\u{1F600}\"");
        assert!(result.is_err());
    }

    #[rstest]
    #[case("!@#$%^&*()")]
    #[case("hello-world_123")]
    #[case("a.b.c.d")]
    #[case("key=value")]
    #[case("path/to/file")]
    #[case("[bracket]")]
    #[case("{curly}")]
    fn test_special_ascii_chars(#[case] s: &str) {
        let stack_str = StackStr::new(s);
        assert_eq!(stack_str.as_str(), s);
    }

    #[rstest]
    fn test_ascii_control_chars_tab() {
        // 制表符是空白字符，但也是有效的 ASCII
        let result = StackStr::new_checked("a\tb");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_str(), "a\tb");
    }

    #[rstest]
    fn test_ordering_same_prefix_different_length() {
        let short = StackStr::new("abc");
        let long = StackStr::new("abcd");
        assert!(short < long);
    }

    #[rstest]
    fn test_ordering_case_sensitive() {
        let upper = StackStr::new("ABC");
        let lower = StackStr::new("abc");
        // ASCII: 'A' (65) < 'a' (97)
        assert!(upper < lower);
    }

    #[rstest]
    fn test_partial_cmp_returns_some() {
        let a = StackStr::new("test");
        let b = StackStr::new("test");
        assert_eq!(a.partial_cmp(&b), Some(std::cmp::Ordering::Equal));
    }

    #[rstest]
    fn test_new_checked_error_empty() {
        let err = StackStr::new_checked("").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[rstest]
    fn test_new_checked_error_whitespace() {
        let err = StackStr::new_checked("   ").unwrap_err();
        assert!(err.to_string().contains("whitespace"));
    }

    #[rstest]
    fn test_new_checked_error_too_long() {
        let err = StackStr::new_checked(&"x".repeat(55)).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[rstest]
    fn test_new_checked_error_non_ascii() {
        let err = StackStr::new_checked("hello\u{1F600}").unwrap_err();
        assert!(err.to_string().contains("non-ASCII"));
    }

    #[rstest]
    fn test_new_checked_error_interior_nul() {
        let err = StackStr::new_checked("abc\0def").unwrap_err();
        assert!(err.to_string().contains("NUL"));
    }

    #[rstest]
    fn test_clone_equals_original() {
        let a = StackStr::new("test");
        #[allow(clippy::clone_on_copy)]
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[rstest]
    fn test_deref() {
        let s = StackStr::new("hello");
        assert!(s.starts_with("hell"));
        assert_eq!(s.len(), 5);
    }

    #[rstest]
    fn test_partial_eq_str_literal() {
        let s = StackStr::new("hello");
        assert!(s == "hello");
        assert!(s != "world");
    }

    #[rstest]
    fn test_try_from_bytes() {
        let s: StackStr = b"hello".as_slice().try_into().unwrap();
        assert_eq!(s.as_str(), "hello");
    }
}
