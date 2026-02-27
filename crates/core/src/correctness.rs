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

//! 类似于“契约式设计”（design by contract）理念的正确性检查函数。
//!
//! 此模块提供对函数或方法条件的验证检查。
//!
//! 条件是一个谓词，为了确保符合设计规范的正确行为，它在某段代码执行前必须为真。
//!
//! 当条件检查失败时，将返回一个包含描述性消息的 [`anyhow::Result`]。

use std::fmt::{Debug, Display};

use rust_decimal::Decimal;

use crate::collections::{MapLike, SetLike};

/// 可用于 `expect` 或其他断言相关函数的消息前缀。
///
/// 此常量提供了一个标准消息，用于在谓词或条件不成立时指示失败。
/// 它通常与 `expect` 等函数配合使用，以提供一致的错误消息。
pub const FAILED: &str = "条件检查失败";

/// 检查 `predicate` 是否为真。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_predicate_true(predicate: bool, fail_msg: &str) -> anyhow::Result<()> {
    if !predicate {
        anyhow::bail!("{fail_msg}")
    }
    Ok(())
}

/// 检查 `predicate` 是否为假。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_predicate_false(predicate: bool, fail_msg: &str) -> anyhow::Result<()> {
    if predicate {
        anyhow::bail!("{fail_msg}")
    }
    Ok(())
}

/// 检查字符串 `s` 是否不为空。
///
/// 此函数执行基本检查，以确保字符串至少包含一个字符。
/// 与 `check_valid_string` 不同，它不验证 ASCII 字符或检查空白。
///
/// # 错误
///
/// 如果 `s` 为空，则返回错误。
#[inline(always)]
pub fn check_nonempty_string<T: AsRef<str>>(s: T, param: &str) -> anyhow::Result<()> {
    if s.as_ref().is_empty() {
        anyhow::bail!("'{param}' 是无效字符串，不能为空");
    }
    Ok(())
}

/// 检查字符串 `s` 是否具有语义含义且仅包含 ASCII 字符。
///
/// # 错误
///
/// 在以下情况下返回错误：
/// - `s` 是空字符串。
/// - `s` 仅由空白字符组成。
/// - `s` 包含一个或多个非 ASCII 字符。
#[inline(always)]
pub fn check_valid_string_ascii<T: AsRef<str>>(s: T, param: &str) -> anyhow::Result<()> {
    let s = s.as_ref();

    if s.is_empty() {
        anyhow::bail!("'{param}' 是无效字符串，不能为空");
    }

    // 确保仅遍历一次字符串
    let mut has_non_whitespace = false;
    for c in s.chars() {
        if !c.is_whitespace() {
            has_non_whitespace = true;
        }
        if !c.is_ascii() {
            anyhow::bail!("'{param}' 是无效字符串，包含非 ASCII 字符，输入为 '{s}'");
        }
    }

    if !has_non_whitespace {
        anyhow::bail!("'{param}' 是无效字符串，全为空白字符");
    }

    Ok(())
}

/// 检查字符串 `s` 是否具有语义含义并允许 UTF-8 字符。
///
/// 这是 [`check_valid_string_ascii`] 的宽松版本，允许非 ASCII 的 UTF-8 字符。
/// 用于可能包含 Unicode 字符的外部标识符（例如交易代码）。
///
/// # 错误
///
/// 在以下情况下返回错误：
/// - `s` 是空字符串。
/// - `s` 仅由空白字符组成。
#[inline(always)]
pub fn check_valid_string_utf8<T: AsRef<str>>(s: T, param: &str) -> anyhow::Result<()> {
    let s = s.as_ref();

    if s.is_empty() {
        anyhow::bail!("'{param}' 是无效字符串，不能为空");
    }

    let has_non_whitespace = s.chars().any(|c| !c.is_whitespace());

    if !has_non_whitespace {
        anyhow::bail!("'{param}' 是无效字符串，全为空白字符");
    }

    Ok(())
}

/// 检查字符串 `s`（如果是 Some）是否仅包含 ASCII 字符且具有语义含义。
///
/// # 错误
///
/// 在以下情况下返回错误：
/// - `s` 是空字符串。
/// - `s` 仅由空白字符组成。
/// - `s` 包含一个或多个非 ASCII 字符。
#[inline(always)]
pub fn check_valid_string_ascii_optional<T: AsRef<str>>(
    s: Option<T>,
    param: &str,
) -> anyhow::Result<()> {
    if let Some(s) = s {
        check_valid_string_ascii(s, param)?;
    }
    Ok(())
}

/// 检查字符串 `s` 是否包含模式 `pat`。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_string_contains<T: AsRef<str>>(s: T, pat: &str, param: &str) -> anyhow::Result<()> {
    let s = s.as_ref();
    if !s.contains(pat) {
        anyhow::bail!("'{param}' 是无效字符串，未包含 '{pat}'，输入为 '{s}'")
    }
    Ok(())
}

/// 检查值是否相等。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_equal<T: PartialEq + Debug + Display>(
    lhs: &T,
    rhs: &T,
    lhs_param: &str,
    rhs_param: &str,
) -> anyhow::Result<()> {
    if lhs != rhs {
        anyhow::bail!("'{lhs_param}' 的值 {lhs} 与 '{rhs_param}' 的值 {rhs} 不相等");
    }
    Ok(())
}

/// 检查 `u8` 值是否相等。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_equal_u8(lhs: u8, rhs: u8, lhs_param: &str, rhs_param: &str) -> anyhow::Result<()> {
    if lhs != rhs {
        anyhow::bail!("'{lhs_param}' 的 u8 值 {lhs} 与 '{rhs_param}' 的 u8 值 {rhs} 不相等")
    }
    Ok(())
}

/// 检查 `usize` 值是否相等。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_equal_usize(
    lhs: usize,
    rhs: usize,
    lhs_param: &str,
    rhs_param: &str,
) -> anyhow::Result<()> {
    if lhs != rhs {
        anyhow::bail!("'{lhs_param}' 的 usize 值 {lhs} 与 '{rhs_param}' 的 usize 值 {rhs} 不相等")
    }
    Ok(())
}

/// 检查 `u64` 值是否为正数 (> 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_positive_u64(value: u64, param: &str) -> anyhow::Result<()> {
    if value == 0 {
        anyhow::bail!("'{param}' 的 u64 值无效，不是正数，输入为 {value}")
    }
    Ok(())
}

/// 检查 `u128` 值是否为正数 (> 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_positive_u128(value: u128, param: &str) -> anyhow::Result<()> {
    if value == 0 {
        anyhow::bail!("'{param}' 的 u128 值无效，不是正数，输入为 {value}")
    }
    Ok(())
}

/// 检查 `i64` 值是否为正数 (> 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_positive_i64(value: i64, param: &str) -> anyhow::Result<()> {
    if value <= 0 {
        anyhow::bail!("'{param}' 的 i64 值无效，不是正数，输入为 {value}")
    }
    Ok(())
}

/// 检查 `i128` 值是否为正数 (> 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_positive_i128(value: i128, param: &str) -> anyhow::Result<()> {
    if value <= 0 {
        anyhow::bail!("'{param}' 的 i128 值无效，不是正数，输入为 {value}")
    }
    Ok(())
}

/// 检查 `f64` 值是否为非负数 (>= 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_non_negative_f64(value: f64, param: &str) -> anyhow::Result<()> {
    if value.is_nan() || value.is_infinite() {
        anyhow::bail!("'{param}' 的 f64 值无效，输入为 {value}")
    }
    if value < 0.0 {
        anyhow::bail!("'{param}' 的 f64 值无效，是负数，输入为 {value}")
    }
    Ok(())
}

/// 检查 `u8` 值是否在范围 [`l`, `r`] 内（闭区间）。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_in_range_inclusive_u8(value: u8, l: u8, r: u8, param: &str) -> anyhow::Result<()> {
    if value < l || value > r {
        anyhow::bail!("'{param}' 的 u8 值无效，不在范围 [{l}, {r}] 内，输入为 {value}")
    }
    Ok(())
}

/// 检查 `u64` 值是否在范围 [`l`, `r`] 内（闭区间）。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_in_range_inclusive_u64(value: u64, l: u64, r: u64, param: &str) -> anyhow::Result<()> {
    if value < l || value > r {
        anyhow::bail!("'{param}' 的 u64 值无效，不在范围 [{l}, {r}] 内，输入为 {value}")
    }
    Ok(())
}

/// 检查 `i64` 值是否在范围 [`l`, `r`] 内（闭区间）。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_in_range_inclusive_i64(value: i64, l: i64, r: i64, param: &str) -> anyhow::Result<()> {
    if value < l || value > r {
        anyhow::bail!("'{param}' 的 i64 值无效，不在范围 [{l}, {r}] 内，输入为 {value}")
    }
    Ok(())
}

/// 检查 `f64` 值是否在范围 [`l`, `r`] 内（闭区间）。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_in_range_inclusive_f64(value: f64, l: f64, r: f64, param: &str) -> anyhow::Result<()> {
    // 安全：硬编码 epsilon 是有意为之且在此处是合适的，因为：
    // - 1e-15 对于 IEEE 754 双精度是保守的（机器 epsilon 约 2.22e-16）
    // - 此函数用于验证，而非高精度计算
    // - epsilon 防止由于浮点表示导致的误报失败
    // - 使其可配置会使 API 复杂化而收益微乎其微
    const EPSILON: f64 = 1e-15;

    if value.is_nan() || value.is_infinite() {
        anyhow::bail!("'{param}' 的 f64 值无效，输入为 {value}")
    }
    if value < l - EPSILON || value > r + EPSILON {
        anyhow::bail!("'{param}' 的 f64 值无效，不在范围 [{l}, {r}] 内，输入为 {value}")
    }
    Ok(())
}

/// 检查 `usize` 值是否在范围 [`l`, `r`] 内（闭区间）。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_in_range_inclusive_usize(
    value: usize,
    l: usize,
    r: usize,
    param: &str,
) -> anyhow::Result<()> {
    if value < l || value > r {
        anyhow::bail!("'{param}' 的 usize 值无效，不在范围 [{l}, {r}] 内，输入为 {value}")
    }
    Ok(())
}

/// 检查切片是否为空。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_slice_empty<T>(slice: &[T], param: &str) -> anyhow::Result<()> {
    if !slice.is_empty() {
        anyhow::bail!(
            "'{param}' 切片 `&[{}]` 不为空",
            std::any::type_name::<T>()
        )
    }
    Ok(())
}

/// 检查切片是否**不**为空。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_slice_not_empty<T>(slice: &[T], param: &str) -> anyhow::Result<()> {
    if slice.is_empty() {
        anyhow::bail!(
            "'{param}' 切片 `&[{}]` 为空",
            std::any::type_name::<T>()
        )
    }
    Ok(())
}

/// 检查哈希映射是否为空。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_map_empty<M>(map: &M, param: &str) -> anyhow::Result<()>
where
    M: MapLike,
{
    if !map.is_empty() {
        anyhow::bail!(
            "'{param}' 映射 `&<{}, {}>` 不为空",
            std::any::type_name::<M::Key>(),
            std::any::type_name::<M::Value>(),
        );
    }
    Ok(())
}

/// 检查映射是否**不**为空。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_map_not_empty<M>(map: &M, param: &str) -> anyhow::Result<()>
where
    M: MapLike,
{
    if map.is_empty() {
        anyhow::bail!(
            "'{param}' 映射 `&<{}, {}>` 为空",
            std::any::type_name::<M::Key>(),
            std::any::type_name::<M::Value>(),
        );
    }
    Ok(())
}

/// 检查 `key` 是否**不在** `map` 中。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_key_not_in_map<M>(
    key: &M::Key,
    map: &M,
    key_name: &str,
    map_name: &str,
) -> anyhow::Result<()>
where
    M: MapLike,
{
    if map.contains_key(key) {
        anyhow::bail!(
            "键 '{key_name}' ({key}) 已存在于 '{map_name}' 映射 `&<{}, {}>` 中",
            std::any::type_name::<M::Key>(),
            std::any::type_name::<M::Value>(),
        );
    }
    Ok(())
}

/// 检查 `key` 是否在 `map` 中。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_key_in_map<M>(
    key: &M::Key,
    map: &M,
    key_name: &str,
    map_name: &str,
) -> anyhow::Result<()>
where
    M: MapLike,
{
    if !map.contains_key(key) {
        anyhow::bail!(
            "键 '{key_name}' ({key}) 不在 '{map_name}' 映射 `&<{}, {}>` 中",
            std::any::type_name::<M::Key>(),
            std::any::type_name::<M::Value>(),
        );
    }
    Ok(())
}

/// 检查 `member` 是否**不在** `set` 中。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_member_not_in_set<S>(
    member: &S::Item,
    set: &S,
    member_name: &str,
    set_name: &str,
) -> anyhow::Result<()>
where
    S: SetLike,
{
    if set.contains(member) {
        anyhow::bail!(
            "成员 '{member_name}' 已存在于 '{set_name}' 集合 `&<{}>` 中",
            std::any::type_name::<S::Item>(),
        );
    }
    Ok(())
}

/// 检查 `member` 是否在 `set` 中。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_member_in_set<S>(
    member: &S::Item,
    set: &S,
    member_name: &str,
    set_name: &str,
) -> anyhow::Result<()>
where
    S: SetLike,
{
    if !set.contains(member) {
        anyhow::bail!(
            "成员 '{member_name}' 不在 '{set_name}' 集合 `&<{}>` 中",
            std::any::type_name::<S::Item>(),
        );
    }
    Ok(())
}

/// 检查 `Decimal` 值是否为正数 (> 0)。
///
/// # 错误
///
/// 如果验证检查失败，则返回错误。
#[inline(always)]
pub fn check_positive_decimal(value: Decimal, param: &str) -> anyhow::Result<()> {
    if value <= Decimal::ZERO {
        anyhow::bail!("'{param}' 的 Decimal 值无效，不是正数，输入为 {value}")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        fmt::Display,
        str::FromStr,
    };

    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;

    #[rstest]
    #[case(false, false)]
    #[case(true, true)]
    fn test_check_predicate_true(#[case] predicate: bool, #[case] expected: bool) {
        let result = check_predicate_true(predicate, "谓词为假").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(false, true)]
    #[case(true, false)]
    fn test_check_predicate_false(#[case] predicate: bool, #[case] expected: bool) {
        let result = check_predicate_false(predicate, "谓词为真").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("a")]
    #[case(" ")] // <-- 允许空白
    #[case("  ")] // <-- 允许连续空白
    #[case("🦀")] // <-- 允许非 ASCII
    #[case(" a")]
    #[case("a ")]
    #[case("abc")]
    fn test_check_nonempty_string_with_valid_values(#[case] s: &str) {
        assert!(check_nonempty_string(s, "value").is_ok());
    }

    #[rstest]
    #[case("")] // 空字符串
    fn test_check_nonempty_string_with_invalid_values(#[case] s: &str) {
        assert!(check_nonempty_string(s, "value").is_err());
    }

    #[rstest]
    #[case(" a")]
    #[case("a ")]
    #[case("a a")]
    #[case(" a ")]
    #[case("abc")]
    fn test_check_valid_string_ascii_with_valid_value(#[case] s: &str) {
        assert!(check_valid_string_ascii(s, "value").is_ok());
    }

    #[rstest]
    #[case("")] // <-- 空字符串
    #[case(" ")] // <-- 仅包含空白
    #[case("  ")] // <-- 仅包含空白
    #[case("🦀")] // <-- 包含非 ASCII 字符
    fn test_check_valid_string_ascii_with_invalid_values(#[case] s: &str) {
        assert!(check_valid_string_ascii(s, "value").is_err());
    }

    #[rstest]
    #[case(" a")]
    #[case("a ")]
    #[case("abc")]
    #[case("ETHUSDT")]
    fn test_check_valid_string_utf8_with_valid_values(#[case] s: &str) {
        assert!(check_valid_string_utf8(s, "value").is_ok());
    }

    #[rstest]
    #[case("")] // <-- 空字符串
    #[case(" ")] // <-- 仅包含空白
    #[case("  ")] // <-- 仅包含空白
    fn test_check_valid_string_utf8_with_invalid_values(#[case] s: &str) {
        assert!(check_valid_string_utf8(s, "value").is_err());
    }

    #[rstest]
    #[case(None)]
    #[case(Some(" a"))]
    #[case(Some("a "))]
    #[case(Some("a a"))]
    #[case(Some(" a "))]
    #[case(Some("abc"))]
    fn test_check_valid_string_ascii_optional_with_valid_value(#[case] s: Option<&str>) {
        assert!(check_valid_string_ascii_optional(s, "value").is_ok());
    }

    #[rstest]
    #[case("a", "a")]
    fn test_check_string_contains_when_does_contain(#[case] s: &str, #[case] pat: &str) {
        assert!(check_string_contains(s, pat, "value").is_ok());
    }

    #[rstest]
    #[case("a", "b")]
    fn test_check_string_contains_when_does_not_contain(#[case] s: &str, #[case] pat: &str) {
        assert!(check_string_contains(s, pat, "value").is_err());
    }

    #[rstest]
    #[case(0u8, 0u8, "left", "right", true)]
    #[case(1u8, 1u8, "left", "right", true)]
    #[case(0u8, 1u8, "left", "right", false)]
    #[case(1u8, 0u8, "left", "right", false)]
    #[case(10i32, 10i32, "left", "right", true)]
    #[case(10i32, 20i32, "left", "right", false)]
    #[case("hello", "hello", "left", "right", true)]
    #[case("hello", "world", "left", "right", false)]
    fn test_check_equal<T: PartialEq + Debug + Display>(
        #[case] lhs: T,
        #[case] rhs: T,
        #[case] lhs_param: &str,
        #[case] rhs_param: &str,
        #[case] expected: bool,
    ) {
        let result = check_equal(&lhs, &rhs, lhs_param, rhs_param).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, 0, "left", "right", true)]
    #[case(1, 1, "left", "right", true)]
    #[case(0, 1, "left", "right", false)]
    #[case(1, 0, "left", "right", false)]
    fn test_check_equal_u8_when_equal(
        #[case] lhs: u8,
        #[case] rhs: u8,
        #[case] lhs_param: &str,
        #[case] rhs_param: &str,
        #[case] expected: bool,
    ) {
        let result = check_equal_u8(lhs, rhs, lhs_param, rhs_param).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(0, 0, "left", "right", true)]
    #[case(1, 1, "left", "right", true)]
    #[case(0, 1, "left", "right", false)]
    #[case(1, 0, "left", "right", false)]
    fn test_check_equal_usize_when_equal(
        #[case] lhs: usize,
        #[case] rhs: usize,
        #[case] lhs_param: &str,
        #[case] rhs_param: &str,
        #[case] expected: bool,
    ) {
        let result = check_equal_usize(lhs, rhs, lhs_param, rhs_param).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(1, "value")]
    fn test_check_positive_u64_when_positive(#[case] value: u64, #[case] param: &str) {
        assert!(check_positive_u64(value, param).is_ok());
    }

    #[rstest]
    #[case(0, "value")]
    fn test_check_positive_u64_when_not_positive(#[case] value: u64, #[case] param: &str) {
        assert!(check_positive_u64(value, param).is_err());
    }

    #[rstest]
    #[case(1, "value")]
    fn test_check_positive_i64_when_positive(#[case] value: i64, #[case] param: &str) {
        assert!(check_positive_i64(value, param).is_ok());
    }

    #[rstest]
    #[case(0, "value")]
    #[case(-1, "value")]
    fn test_check_positive_i64_when_not_positive(#[case] value: i64, #[case] param: &str) {
        assert!(check_positive_i64(value, param).is_err());
    }

    #[rstest]
    #[case(0.0, "value")]
    #[case(1.0, "value")]
    fn test_check_non_negative_f64_when_not_negative(#[case] value: f64, #[case] param: &str) {
        assert!(check_non_negative_f64(value, param).is_ok());
    }

    #[rstest]
    #[case(f64::NAN, "value")]
    #[case(f64::INFINITY, "value")]
    #[case(f64::NEG_INFINITY, "value")]
    #[case(-0.1, "value")]
    fn test_check_non_negative_f64_when_negative(#[case] value: f64, #[case] param: &str) {
        assert!(check_non_negative_f64(value, param).is_err());
    }

    #[rstest]
    #[case(0, 0, 0, "value")]
    #[case(0, 0, 1, "value")]
    #[case(1, 0, 1, "value")]
    fn test_check_in_range_inclusive_u8_when_in_range(
        #[case] value: u8,
        #[case] l: u8,
        #[case] r: u8,
        #[case] desc: &str,
    ) {
        assert!(check_in_range_inclusive_u8(value, l, r, desc).is_ok());
    }

    #[rstest]
    #[case(0, 1, 2, "value")]
    #[case(3, 1, 2, "value")]
    fn test_check_in_range_inclusive_u8_when_out_of_range(
        #[case] value: u8,
        #[case] l: u8,
        #[case] r: u8,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_u8(value, l, r, param).is_err());
    }

    #[rstest]
    #[case(0, 0, 0, "value")]
    #[case(0, 0, 1, "value")]
    #[case(1, 0, 1, "value")]
    fn test_check_in_range_inclusive_u64_when_in_range(
        #[case] value: u64,
        #[case] l: u64,
        #[case] r: u64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_u64(value, l, r, param).is_ok());
    }

    #[rstest]
    #[case(0, 1, 2, "value")]
    #[case(3, 1, 2, "value")]
    fn test_check_in_range_inclusive_u64_when_out_of_range(
        #[case] value: u64,
        #[case] l: u64,
        #[case] r: u64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_u64(value, l, r, param).is_err());
    }

    #[rstest]
    #[case(0, 0, 0, "value")]
    #[case(0, 0, 1, "value")]
    #[case(1, 0, 1, "value")]
    fn test_check_in_range_inclusive_i64_when_in_range(
        #[case] value: i64,
        #[case] l: i64,
        #[case] r: i64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_i64(value, l, r, param).is_ok());
    }

    #[rstest]
    #[case(0.0, 0.0, 0.0, "value")]
    #[case(0.0, 0.0, 1.0, "value")]
    #[case(1.0, 0.0, 1.0, "value")]
    fn test_check_in_range_inclusive_f64_when_in_range(
        #[case] value: f64,
        #[case] l: f64,
        #[case] r: f64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_f64(value, l, r, param).is_ok());
    }

    #[rstest]
    #[case(-1e16, 0.0, 0.0, "value")]
    #[case(1.0 + 1e16, 0.0, 1.0, "value")]
    fn test_check_in_range_inclusive_f64_when_out_of_range(
        #[case] value: f64,
        #[case] l: f64,
        #[case] r: f64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_f64(value, l, r, param).is_err());
    }

    #[rstest]
    #[case(0, 1, 2, "value")]
    #[case(3, 1, 2, "value")]
    fn test_check_in_range_inclusive_i64_when_out_of_range(
        #[case] value: i64,
        #[case] l: i64,
        #[case] r: i64,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_i64(value, l, r, param).is_err());
    }

    #[rstest]
    #[case(0, 0, 0, "value")]
    #[case(0, 0, 1, "value")]
    #[case(1, 0, 1, "value")]
    fn test_check_in_range_inclusive_usize_when_in_range(
        #[case] value: usize,
        #[case] l: usize,
        #[case] r: usize,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_usize(value, l, r, param).is_ok());
    }

    #[rstest]
    #[case(0, 1, 2, "value")]
    #[case(3, 1, 2, "value")]
    fn test_check_in_range_inclusive_usize_when_out_of_range(
        #[case] value: usize,
        #[case] l: usize,
        #[case] r: usize,
        #[case] param: &str,
    ) {
        assert!(check_in_range_inclusive_usize(value, l, r, param).is_err());
    }

    #[rstest]
    #[case(vec![], true)]
    #[case(vec![1_u8], false)]
    fn test_check_slice_empty(#[case] collection: Vec<u8>, #[case] expected: bool) {
        let result = check_slice_empty(collection.as_slice(), "param").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(vec![], false)]
    #[case(vec![1_u8], true)]
    fn test_check_slice_not_empty(#[case] collection: Vec<u8>, #[case] expected: bool) {
        let result = check_slice_not_empty(collection.as_slice(), "param").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(HashMap::new(), true)]
    #[case(HashMap::from([("A".to_string(), 1_u8)]), false)]
    fn test_check_map_empty(#[case] map: HashMap<String, u8>, #[case] expected: bool) {
        let result = check_map_empty(&map, "param").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(HashMap::new(), false)]
    #[case(HashMap::from([("A".to_string(), 1_u8)]), true)]
    fn test_check_map_not_empty(#[case] map: HashMap<String, u8>, #[case] expected: bool) {
        let result = check_map_not_empty(&map, "param").is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(&HashMap::<u32, u32>::new(), 5, "key", "map", true)] // 空映射
    #[case(&HashMap::from([(1, 10), (2, 20)]), 1, "key", "map", false)] // 键已存在
    #[case(&HashMap::from([(1, 10), (2, 20)]), 5, "key", "map", true)] // 键不存在
    fn test_check_key_not_in_map(
        #[case] map: &HashMap<u32, u32>,
        #[case] key: u32,
        #[case] key_name: &str,
        #[case] map_name: &str,
        #[case] expected: bool,
    ) {
        let result = check_key_not_in_map(&key, map, key_name, map_name).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(&HashMap::<u32, u32>::new(), 5, "key", "map", false)] // 空映射
    #[case(&HashMap::from([(1, 10), (2, 20)]), 1, "key", "map", true)] // 键已存在
    #[case(&HashMap::from([(1, 10), (2, 20)]), 5, "key", "map", false)] // 键不存在
    fn test_check_key_in_map(
        #[case] map: &HashMap<u32, u32>,
        #[case] key: u32,
        #[case] key_name: &str,
        #[case] map_name: &str,
        #[case] expected: bool,
    ) {
        let result = check_key_in_map(&key, map, key_name, map_name).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(&HashSet::<u32>::new(), 5, "member", "set", true)] // 空集合
    #[case(&HashSet::from([1, 2]), 1, "member", "set", false)] // 成员已存在
    #[case(&HashSet::from([1, 2]), 5, "member", "set", true)] // 成员不存在
    fn test_check_member_not_in_set(
        #[case] set: &HashSet<u32>,
        #[case] member: u32,
        #[case] member_name: &str,
        #[case] set_name: &str,
        #[case] expected: bool,
    ) {
        let result = check_member_not_in_set(&member, set, member_name, set_name).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case(&HashSet::<u32>::new(), 5, "member", "set", false)] // 空集合
    #[case(&HashSet::from([1, 2]), 1, "member", "set", true)] // 成员已存在
    #[case(&HashSet::from([1, 2]), 5, "member", "set", false)] // 成员不存在
    fn test_check_member_in_set(
        #[case] set: &HashSet<u32>,
        #[case] member: u32,
        #[case] member_name: &str,
        #[case] set_name: &str,
        #[case] expected: bool,
    ) {
        let result = check_member_in_set(&member, set, member_name, set_name).is_ok();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("1", true)] // 简单正整数
    #[case("0.0000000000000000000000000001", true)] // 最小正数 (1 × 10⁻²⁸)
    #[case("79228162514264337593543950335", true)] // 极大正数 (≈ Decimal::MAX)
    #[case("0", false)] // 零应失败
    #[case("-0.0000000000000000000000000001", false)] // 微小负数
    #[case("-1", false)] // 简单负整数
    fn test_check_positive_decimal(#[case] raw: &str, #[case] expected: bool) {
        let value = Decimal::from_str(raw).expect("有效的 decimal 字面量");
        let result = super::check_positive_decimal(value, "param").is_ok();
        assert_eq!(result, expected);
    }
}
