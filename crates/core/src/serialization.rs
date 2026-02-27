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

//! 常用的序列化 trait 和函数。
//!
//! 此模块提供了自定义的 serde 反序列化器和序列化器，用于处理解析交易平台 API 响应时遇到的常见模式，特别是：
//!
//! - 应当被解释为 `None` 或零的空字符串。
//! - 从字符串到原始类型的类型转换。
//! - 以字符串形式表示的十进制 (Decimal) 数值。

use std::str::FromStr;

use bytes::Bytes;
use rust_decimal::Decimal;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error, Unexpected, Visitor},
    ser::SerializeSeq,
};
use ustr::Ustr;

struct BoolVisitor;

/// 零分配的十进制数 (Decimal) 访问器，用于最大化反序列化性能。
///
/// 直接访问 JSON 令牌，而不产生中间的 `serde_json::Value` 分配。
/// 处理所有 JSON 数值表示形式：字符串、整数、浮点数以及 null。
struct DecimalVisitor;

impl Visitor<'_> for DecimalVisitor {
    type Value = Decimal;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("以字符串、整数或浮点数形式表示的十进制数字")
    }

    // 快速路径：借用型字符串 (zero-copy)
    fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
        if v.is_empty() {
            return Ok(Decimal::ZERO);
        }
        // 检查是否为科学计数法
        if v.contains('e') || v.contains('E') {
            Decimal::from_scientific(v).map_err(E::custom)
        } else {
            Decimal::from_str(v).map_err(E::custom)
        }
    }

    // 所有型字符串（极少见情况，委托给 visit_str 处理）
    fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
        self.visit_str(&v)
    }

    // 直接处理整数 - 不需要进行字符串转换
    fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(Decimal::from(v))
    }

    fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(Decimal::from(v))
    }

    fn visit_i128<E: Error>(self, v: i128) -> Result<Self::Value, E> {
        Ok(Decimal::from(v))
    }

    fn visit_u128<E: Error>(self, v: u128) -> Result<Self::Value, E> {
        Ok(Decimal::from(v))
    }

    // 浮点数处理 - 直接转换
    fn visit_f64<E: Error>(self, v: f64) -> Result<Self::Value, E> {
        if v.is_nan() {
            return Err(E::invalid_value(Unexpected::Float(v), &self));
        }
        if v.is_infinite() {
            return Err(E::invalid_value(Unexpected::Float(v), &self));
        }
        Decimal::try_from(v).map_err(E::custom)
    }

    // Null → 零 (与现有行为保持一致)
    fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
        Ok(Decimal::ZERO)
    }

    fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
        Ok(Decimal::ZERO)
    }
}

/// 零分配的可选级十进制数 (Optional Decimal) 访问器，用于最大化反序列化性能。
///
/// 将 null 值处理为 `None`，将空字符串处理为 `None`。
/// 使用 `deserialize_any` 方法来统统处理所有 JSON 值类型。
struct OptionalDecimalVisitor;

impl Visitor<'_> for OptionalDecimalVisitor {
    type Value = Option<Decimal>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("null 或以字符串、整数、浮点数形式表示的十进制数字")
    }

    // 快速路径：借用型字符串 (zero-copy)
    // 空字符串 → None (与返回 ZERO 的 DecimalVisitor 不同)
    fn visit_str<E: Error>(self, v: &str) -> Result<Self::Value, E> {
        if v.is_empty() {
            return Ok(None);
        }
        DecimalVisitor.visit_str(v).map(Some)
    }

    fn visit_string<E: Error>(self, v: String) -> Result<Self::Value, E> {
        self.visit_str(&v)
    }

    fn visit_i64<E: Error>(self, v: i64) -> Result<Self::Value, E> {
        DecimalVisitor.visit_i64(v).map(Some)
    }

    fn visit_u64<E: Error>(self, v: u64) -> Result<Self::Value, E> {
        DecimalVisitor.visit_u64(v).map(Some)
    }

    fn visit_i128<E: Error>(self, v: i128) -> Result<Self::Value, E> {
        DecimalVisitor.visit_i128(v).map(Some)
    }

    fn visit_u128<E: Error>(self, v: u128) -> Result<Self::Value, E> {
        DecimalVisitor.visit_u128(v).map(Some)
    }

    fn visit_f64<E: Error>(self, v: f64) -> Result<Self::Value, E> {
        DecimalVisitor.visit_f64(v).map(Some)
    }

    // Null → None
    fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

/// 表示符合 JSON 规范且可序列化的类型。
pub trait Serializable: Serialize + for<'de> Deserialize<'de> {
    /// 从 JSON 编码的字节反序列化对象。
    ///
    /// # 错误
    ///
    /// 返回序列化错误。
    fn from_json_bytes(data: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(data)
    }

    /// 将对象序列化为 JSON 编码的字节。
    ///
    /// # 错误
    ///
    /// 返回序列化错误。
    fn to_json_bytes(&self) -> Result<Bytes, serde_json::Error> {
        serde_json::to_vec(self).map(Bytes::from)
    }
}

pub use self::msgpack::{FromMsgPack, MsgPackSerializable, ToMsgPack};

/// 为实现 [`Serializable`] 的类型提供 MsgPack 序列化支持。
///
/// 此模块包含用于 MsgPack 序列化和反序列化的 trait，与核心 [`Serializable`] trait 分离开，允许独立启用。
pub mod msgpack {
    use bytes::Bytes;
    use serde::{Deserialize, Serialize};

    use super::Serializable;

    /// 提供从 MsgPack 编码字节的反序列化能力。
    pub trait FromMsgPack: for<'de> Deserialize<'de> + Sized {
        /// 从 MsgPack 编码的字节反序列化对象。
        ///
        /// # 错误
        ///
        /// 返回序列化错误。
        fn from_msgpack_bytes(data: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
            rmp_serde::from_slice(data)
        }
    }

    /// 提供向 MsgPack 编码字节的序列化能力。
    pub trait ToMsgPack: Serialize {
        /// 将对象序列化为 MsgPack 编码的字节。
        ///
        /// # 错误
        ///
        /// 返回序列化错误。
        fn to_msgpack_bytes(&self) -> Result<Bytes, rmp_serde::encode::Error> {
            rmp_serde::to_vec_named(self).map(Bytes::from)
        }
    }

    /// 结合了 [`Serializable`], [`FromMsgPack`] 和 [`ToMsgPack`] 的标记性 (marker) trait。
    ///
    /// 为所有实现 [`Serializable`] 的类型自动实现此 trait。
    pub trait MsgPackSerializable: Serializable + FromMsgPack + ToMsgPack {}

    impl<T> FromMsgPack for T where T: Serializable {}

    impl<T> ToMsgPack for T where T: Serializable {}

    impl<T> MsgPackSerializable for T where T: Serializable {}
}

impl Visitor<'_> for BoolVisitor {
    type Value = u8;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("以 u8 类型表示的布尔值")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(u8::from(value))
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "用于解析，是有意为之，且已验证过值范围"
    )]
    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // 只有 0 或 1 被视为作为整数提供时的有效表示。
        // 我们有意拒绝此范围之外的值，以避免将较大的整数静默截断为实现定义的布尔语义。
        if value > 1 {
            Err(E::invalid_value(Unexpected::Unsigned(value), &self))
        } else {
            Ok(value as u8)
        }
    }
}

/// Serde 默认值函数，返回 `true`。
///
/// 在布尔字段上使用 `#[serde(default = "default_true")]`。
#[must_use]
pub const fn default_true() -> bool {
    true
}

/// Serde 默认值函数，返回 `false`。
///
/// 在布尔字段上使用 `#[serde(default = "default_false")]`。
#[must_use]
pub const fn default_false() -> bool {
    false
}

/// 将布尔值作为 `u8` 反序列化。
///
/// # 错误
///
/// 返回序列化错误。
pub fn from_bool_as_u8<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(BoolVisitor)
}

/// 从 JSON 字符串或数字中反序列化为一个 `Decimal`。
///
/// 高性能实现，使用自定义反序列化器，避免中间的 `serde_json::Value` 分配。
/// 处理所有的 JSON 数值表示形式：
///
/// - JSON 字符串: `"123.456"` → Decimal (对于借用字符串为零拷贝)
/// - JSON 整数: `123` → Decimal (直接转换，无字符串分配)
/// - JSON 浮点数: `123.456` → Decimal
/// - JSON null: → `Decimal::ZERO`
/// - 科学计数法: `"1.5e-8"` → Decimal
///
/// # 性能
///
/// 此实现针对高频交易场景进行了优化：
/// - 字符串值零分配（使用借用的 `&str`）
/// - 无需通过字符串中间体，直接进行整数转换
/// - 不产生中间的 `serde_json::Value` 堆分配
///
/// # 错误
///
/// 如果值无法解析为有效的十进制数，则返回错误。
pub fn deserialize_decimal<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(DecimalVisitor)
}

/// 从 JSON 字符串、数字或 null 反序列化为一个 `Option<Decimal>`。
///
/// 高性能实现，使用自定义反序列化器，避免中间的 `serde_json::Value` 分配。
/// 处理所有的 JSON 数值表示形式：
///
/// - JSON 字符串: `"123.456"` → Some(Decimal) (对于借用字符串为零拷贝)
/// - JSON 整数: `123` → Some(Decimal) (直接转换)
/// - JSON 浮点数: `123.456` → Some(Decimal)
/// - JSON null: → `None`
/// - 空字符串: `""` → `None`
/// - 科学计数法: `"1.5e-8"` → Some(Decimal)
///
/// # 性能
///
/// 此实现针对高频交易场景进行了优化：
/// - 字符串值零分配（使用借用的 `&str`）
/// - 无需通过字符串中间体，直接进行整数转换
/// - 不产生中间的 `serde_json::Value` 堆分配
///
/// # 错误
///
/// 如果值无法解析为有效的十进制数，则返回错误。
pub fn deserialize_optional_decimal<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    // 使用 deserialize_any 以统统处理所有的 JSON 值类型
    // (deserialize_option 会将非 null 路由到 visit_some，从而丢失对空字符串的处理)
    deserializer.deserialize_any(OptionalDecimalVisitor)
}

/// 将 `Decimal` 序列化为 JSON 数字（浮点数）。
///
/// 用于交易平台 API 期望获得 JSON 数字的出站请求。
///
/// # 错误
///
/// 如果序列化失败，则返回错误。
pub fn serialize_decimal<S: Serializer>(d: &Decimal, s: S) -> Result<S::Ok, S::Error> {
    rust_decimal::serde::float::serialize(d, s)
}

/// 将 `Option<Decimal>` 序列化为 JSON 数字或 null。
///
/// # 错误
///
/// 如果序列化失败，则返回错误。
pub fn serialize_optional_decimal<S: Serializer>(
    d: &Option<Decimal>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match d {
        Some(decimal) => rust_decimal::serde::float::serialize(decimal, s),
        None => s.serialize_none(),
    }
}

/// 从 JSON 字符串反序列化 `Decimal`。
///
/// 这是严格形式，要求值必须是字符串，拒绝数值型的 JSON 值，以避免精度丢失。
///
/// # 错误
///
/// 如果字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_decimal_from_str<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Decimal::from_str(&s).map_err(D::Error::custom)
}

/// 从可能为空的字符串字段中反序列化 `Decimal`。
///
/// 处理边界情况，使空字符串 "" 或 "0" 变为 `Decimal::ZERO`。
///
/// # 错误
///
/// 如果字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_decimal_or_zero<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    if s.is_empty() || s == "0" {
        Ok(Decimal::ZERO)
    } else {
        Decimal::from_str(&s).map_err(D::Error::custom)
    }
}

/// 从字符串字段中反序列化可选级 `Decimal`。
///
/// 如果字符串为空或为 "0"，则返回 `None`；否则解析为 `Decimal`。
/// 这是一个严格的仅限字符串的反序列化器；如需灵活处理字符串、数字以及 null，
/// 请使用 [`deserialize_optional_decimal`]。
///
/// # 错误
///
/// 如果字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_optional_decimal_str<'de, D>(
    deserializer: D,
) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    if s.is_empty() || s == "0" {
        Ok(None)
    } else {
        Decimal::from_str(&s).map(Some).map_err(D::Error::custom)
    }
}

/// 从仅限字符串的字段中反序列化可选级 `Decimal`。
///
/// 如果值为 null 或字符串为空，则返回 `None`；否则解析为 `Decimal`。
///
/// # 错误
///
/// 如果字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_optional_decimal_from_str<'de, D>(
    deserializer: D,
) -> Result<Option<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        Some(s) if !s.is_empty() => Decimal::from_str(&s).map(Some).map_err(D::Error::custom),
        _ => Ok(None),
    }
}

/// 从可选级字符串字段中反序列化 `Decimal`，默认值为零。
///
/// 处理边界情况：`None`、空字符串 "" 或 "0" 均会变为 `Decimal::ZERO`。
///
/// # 错误
///
/// 如果字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_optional_decimal_or_zero<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Deserialize::deserialize(deserializer)?;
    match opt {
        None => Ok(Decimal::ZERO),
        Some(s) if s.is_empty() || s == "0" => Ok(Decimal::ZERO),
        Some(s) => Decimal::from_str(&s).map_err(D::Error::custom),
    }
}

/// 从 JSON 字符串数组中反序列化 `Vec<Decimal>`。
///
/// # 错误
///
/// 如果任何字符串无法解析为有效的十进制数，则返回错误。
pub fn deserialize_vec_decimal_from_str<'de, D>(deserializer: D) -> Result<Vec<Decimal>, D::Error>
where
    D: Deserializer<'de>,
{
    let strings = Vec::<String>::deserialize(deserializer)?;
    strings
        .into_iter()
        .map(|s| Decimal::from_str(&s).map_err(D::Error::custom))
        .collect()
}

/// 将 `Decimal` 序列化为字符串（无损，不使用科学计数法）。
///
/// # 错误
///
/// 如果序列化失败，则返回错误。
pub fn serialize_decimal_as_str<S>(decimal: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&decimal.to_string())
}

/// 将可选级 `Decimal` 序列化为字符串。
///
/// # 错误
///
/// 如果序列化失败，则返回错误。
pub fn serialize_optional_decimal_as_str<S>(
    decimal: &Option<Decimal>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match decimal {
        Some(d) => serializer.serialize_str(&d.to_string()),
        None => serializer.serialize_none(),
    }
}

/// 将 `Vec<Decimal>` 序列化为字符串数组。
///
/// # 错误
///
/// 如果序列化失败，则返回错误。
pub fn serialize_vec_decimal_as_str<S>(
    decimals: &Vec<Decimal>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(decimals.len()))?;
    for decimal in decimals {
        seq.serialize_element(&decimal.to_string())?;
    }
    seq.end()
}

/// 将字符串解析为 `Decimal`，如果解析失败则返回错误。
///
/// # 错误
///
/// 如果字符串无法解析为十进制数，则返回错误。
pub fn parse_decimal(s: &str) -> anyhow::Result<Decimal> {
    Decimal::from_str(s).map_err(|e| anyhow::anyhow!("无法从 '{s}' 解析十进制数：{e}"))
}

/// 将可选级字符串解析为 `Decimal`，如果字符串为 `None` 或为空，则返回 `None`。
///
/// # 错误
///
/// 如果字符串无法解析为十进制数，则返回错误。
pub fn parse_optional_decimal(s: &Option<String>) -> anyhow::Result<Option<Decimal>> {
    match s {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => parse_decimal(s).map(Some),
    }
}

/// 将空字符串反序列化为 `None`。
///
/// 许多交易平台 API 将 null 字符串字段表示为空字符串 (`""`)。
/// 当此类型的数据映射到 `Option<String>` 时，默认行为会产生 `Some("")`，
/// 这与预期的“值缺失”语义不同。此工具确保空字符串在反序列化过程中被规格化为 `None`。
///
/// # 错误
///
/// 如果 JSON 值无法反序列化为字符串，则返回错误。
pub fn deserialize_empty_string_as_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    Ok(opt.filter(|s| !s.is_empty()))
}

/// 将空的 [`Ustr`] 反序列化为 `None`。
///
/// # 错误
///
/// 如果 JSON 值无法反序列化为字符串，则返回错误。
pub fn deserialize_empty_ustr_as_none<'de, D>(deserializer: D) -> Result<Option<Ustr>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<Ustr>::deserialize(deserializer)?;
    Ok(opt.filter(|s| !s.is_empty()))
}

/// 从字符串字段反序列化 `u8`。
///
/// 如果字符串为空，则返回 0。
///
/// # 错误
///
/// 如果字符串无法解析为 u8，则返回错误。
pub fn deserialize_string_to_u8<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    if s.is_empty() {
        return Ok(0);
    }
    s.parse::<u8>().map_err(D::Error::custom)
}

/// 从字符串字段反序列化 `u64`。
///
/// 如果字符串为空，则返回 0。
///
/// # 错误
///
/// 如果字符串无法解析为 u64，则返回错误。
pub fn deserialize_string_to_u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    if s.is_empty() {
        Ok(0)
    } else {
        s.parse::<u64>().map_err(D::Error::custom)
    }
}

/// 从字符串字段反序列化可选级 `u64`。
///
/// 如果值为 null 或字符串为空，则返回 `None`。
///
/// # 错误
///
/// 如果字符串无法解析为 u64，则返回错误。
pub fn deserialize_optional_string_to_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => s.parse().map(Some).map_err(D::Error::custom),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use rstest::*;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use serde::{Deserialize, Serialize};
    use ustr::Ustr;

    use super::{
        Serializable, deserialize_decimal, deserialize_decimal_from_str,
        deserialize_decimal_or_zero, deserialize_empty_string_as_none,
        deserialize_empty_ustr_as_none, deserialize_optional_decimal,
        deserialize_optional_decimal_or_zero, deserialize_optional_decimal_str,
        deserialize_optional_string_to_u64, deserialize_string_to_u8, deserialize_string_to_u64,
        deserialize_vec_decimal_from_str, from_bool_as_u8,
        msgpack::{FromMsgPack, ToMsgPack},
        parse_decimal, parse_optional_decimal, serialize_decimal, serialize_decimal_as_str,
        serialize_optional_decimal, serialize_optional_decimal_as_str,
        serialize_vec_decimal_as_str,
    };

    #[derive(Deserialize)]
    pub struct TestStruct {
        #[serde(deserialize_with = "from_bool_as_u8")]
        pub value: u8,
    }

    #[rstest]
    #[case(r#"{"value": true}"#, 1)]
    #[case(r#"{"value": false}"#, 0)]
    fn test_deserialize_bool_as_u8_with_boolean(#[case] json_str: &str, #[case] expected: u8) {
        let test_struct: TestStruct = serde_json::from_str(json_str).unwrap();
        assert_eq!(test_struct.value, expected);
    }

    #[rstest]
    #[case(r#"{"value": 1}"#, 1)]
    #[case(r#"{"value": 0}"#, 0)]
    fn test_deserialize_bool_as_u8_with_u64(#[case] json_str: &str, #[case] expected: u8) {
        let test_struct: TestStruct = serde_json::from_str(json_str).unwrap();
        assert_eq!(test_struct.value, expected);
    }

    #[rstest]
    fn test_deserialize_bool_as_u8_with_invalid_integer() {
        // 除 0/1 以外的任何整数均无效，应报错
        let json = r#"{"value": 2}"#;
        let result: Result<TestStruct, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct SerializableTestStruct {
        id: u32,
        name: String,
        value: f64,
    }

    impl Serializable for SerializableTestStruct {}

    #[rstest]
    fn test_serializable_json_roundtrip() {
        let original = SerializableTestStruct {
            id: 42,
            name: "test".to_string(),
            value: std::f64::consts::PI,
        };

        let json_bytes = original.to_json_bytes().unwrap();
        let deserialized = SerializableTestStruct::from_json_bytes(&json_bytes).unwrap();

        assert_eq!(original, deserialized);
    }

    #[rstest]
    fn test_serializable_msgpack_roundtrip() {
        let original = SerializableTestStruct {
            id: 123,
            name: "msgpack_test".to_string(),
            value: std::f64::consts::E,
        };

        let msgpack_bytes = original.to_msgpack_bytes().unwrap();
        let deserialized = SerializableTestStruct::from_msgpack_bytes(&msgpack_bytes).unwrap();

        assert_eq!(original, deserialized);
    }

    #[rstest]
    fn test_serializable_json_invalid_data() {
        let invalid_json = b"invalid json data";
        let result = SerializableTestStruct::from_json_bytes(invalid_json);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_serializable_msgpack_invalid_data() {
        let invalid_msgpack = b"invalid msgpack data";
        let result = SerializableTestStruct::from_msgpack_bytes(invalid_msgpack);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_serializable_json_empty_values() {
        let test_struct = SerializableTestStruct {
            id: 0,
            name: String::new(),
            value: 0.0,
        };

        let json_bytes = test_struct.to_json_bytes().unwrap();
        let deserialized = SerializableTestStruct::from_json_bytes(&json_bytes).unwrap();

        assert_eq!(test_struct, deserialized);
    }

    #[rstest]
    fn test_serializable_msgpack_empty_values() {
        let test_struct = SerializableTestStruct {
            id: 0,
            name: String::new(),
            value: 0.0,
        };

        let msgpack_bytes = test_struct.to_msgpack_bytes().unwrap();
        let deserialized = SerializableTestStruct::from_msgpack_bytes(&msgpack_bytes).unwrap();

        assert_eq!(test_struct, deserialized);
    }

    #[derive(Deserialize)]
    struct TestOptionalDecimalStr {
        #[serde(deserialize_with = "deserialize_optional_decimal_str")]
        value: Option<Decimal>,
    }

    #[derive(Deserialize)]
    struct TestDecimalOrZero {
        #[serde(deserialize_with = "deserialize_decimal_or_zero")]
        value: Decimal,
    }

    #[derive(Deserialize)]
    struct TestOptionalDecimalOrZero {
        #[serde(deserialize_with = "deserialize_optional_decimal_or_zero")]
        value: Decimal,
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestDecimalRoundtrip {
        #[serde(
            serialize_with = "serialize_decimal_as_str",
            deserialize_with = "deserialize_decimal_from_str"
        )]
        value: Decimal,
        #[serde(
            serialize_with = "serialize_optional_decimal_as_str",
            deserialize_with = "super::deserialize_optional_decimal_from_str"
        )]
        optional_value: Option<Decimal>,
    }

    #[rstest]
    #[case(r#"{"value":"123.45"}"#, Some(dec!(123.45)))]
    #[case(r#"{"value":"0"}"#, None)]
    #[case(r#"{"value":""}"#, None)]
    fn test_deserialize_optional_decimal_str(
        #[case] json: &str,
        #[case] expected: Option<Decimal>,
    ) {
        let result: TestOptionalDecimalStr = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case(r#"{"value":"123.45"}"#, dec!(123.45))]
    #[case(r#"{"value":"0"}"#, Decimal::ZERO)]
    #[case(r#"{"value":""}"#, Decimal::ZERO)]
    fn test_deserialize_decimal_or_zero(#[case] json: &str, #[case] expected: Decimal) {
        let result: TestDecimalOrZero = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case(r#"{"value":"123.45"}"#, dec!(123.45))]
    #[case(r#"{"value":"0"}"#, Decimal::ZERO)]
    #[case(r#"{"value":null}"#, Decimal::ZERO)]
    fn test_deserialize_optional_decimal_or_zero(#[case] json: &str, #[case] expected: Decimal) {
        let result: TestOptionalDecimalOrZero = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    fn test_decimal_serialization_roundtrip() {
        let original = TestDecimalRoundtrip {
            value: dec!(123.456789012345678),
            optional_value: Some(dec!(0.000000001)),
        };

        let json = serde_json::to_string(&original).unwrap();

        // 检查其是否序列化为字符串
        assert!(json.contains("\"123.456789012345678\""));
        assert!(json.contains("\"0.000000001\""));

        let deserialized: TestDecimalRoundtrip = serde_json::from_str(&json).unwrap();
        assert_eq!(original.value, deserialized.value);
        assert_eq!(original.optional_value, deserialized.optional_value);
    }

    #[rstest]
    fn test_decimal_optional_none_handling() {
        let test_struct = TestDecimalRoundtrip {
            value: dec!(42.0),
            optional_value: None,
        };

        let json = serde_json::to_string(&test_struct).unwrap();
        assert!(json.contains("null"));

        let parsed: TestDecimalRoundtrip = serde_json::from_str(&json).unwrap();
        assert_eq!(test_struct.value, parsed.value);
        assert_eq!(None, parsed.optional_value);
    }

    #[derive(Deserialize)]
    struct TestEmptyStringAsNone {
        #[serde(deserialize_with = "deserialize_empty_string_as_none")]
        value: Option<String>,
    }

    #[rstest]
    #[case(r#"{"value":"hello"}"#, Some("hello".to_string()))]
    #[case(r#"{"value":""}"#, None)]
    #[case(r#"{"value":null}"#, None)]
    fn test_deserialize_empty_string_as_none(#[case] json: &str, #[case] expected: Option<String>) {
        let result: TestEmptyStringAsNone = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[derive(Deserialize)]
    struct TestEmptyUstrAsNone {
        #[serde(deserialize_with = "deserialize_empty_ustr_as_none")]
        value: Option<Ustr>,
    }

    #[rstest]
    #[case(r#"{"value":"hello"}"#, Some(Ustr::from("hello")))]
    #[case(r#"{"value":""}"#, None)]
    #[case(r#"{"value":null}"#, None)]
    fn test_deserialize_empty_ustr_as_none(#[case] json: &str, #[case] expected: Option<Ustr>) {
        let result: TestEmptyUstrAsNone = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct TestVecDecimal {
        #[serde(
            serialize_with = "serialize_vec_decimal_as_str",
            deserialize_with = "deserialize_vec_decimal_from_str"
        )]
        values: Vec<Decimal>,
    }

    #[rstest]
    fn test_vec_decimal_roundtrip() {
        let original = TestVecDecimal {
            values: vec![dec!(1.5), dec!(2.25), dec!(100.001)],
        };

        let json = serde_json::to_string(&original).unwrap();
        assert!(json.contains("[\"1.5\",\"2.25\",\"100.001\"]"));

        let parsed: TestVecDecimal = serde_json::from_str(&json).unwrap();
        assert_eq!(original.values, parsed.values);
    }

    #[rstest]
    fn test_vec_decimal_empty() {
        let original = TestVecDecimal { values: vec![] };

        let json = serde_json::to_string(&original).unwrap();
        let parsed: TestVecDecimal = serde_json::from_str(&json).unwrap();
        assert_eq!(original.values, parsed.values);
    }

    #[derive(Deserialize)]
    struct TestStringToU8 {
        #[serde(deserialize_with = "deserialize_string_to_u8")]
        value: u8,
    }

    #[rstest]
    #[case(r#"{"value":"42"}"#, 42)]
    #[case(r#"{"value":"0"}"#, 0)]
    #[case(r#"{"value":""}"#, 0)]
    fn test_deserialize_string_to_u8(#[case] json: &str, #[case] expected: u8) {
        let result: TestStringToU8 = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[derive(Deserialize)]
    struct TestStringToU64 {
        #[serde(deserialize_with = "deserialize_string_to_u64")]
        value: u64,
    }

    #[rstest]
    #[case(r#"{"value":"12345678901234"}"#, 12345678901234)]
    #[case(r#"{"value":"0"}"#, 0)]
    #[case(r#"{"value":""}"#, 0)]
    fn test_deserialize_string_to_u64(#[case] json: &str, #[case] expected: u64) {
        let result: TestStringToU64 = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[derive(Deserialize)]
    struct TestOptionalStringToU64 {
        #[serde(deserialize_with = "deserialize_optional_string_to_u64")]
        value: Option<u64>,
    }

    #[rstest]
    #[case(r#"{"value":"12345678901234"}"#, Some(12345678901234))]
    #[case(r#"{"value":"0"}"#, Some(0))]
    #[case(r#"{"value":""}"#, None)]
    #[case(r#"{"value":null}"#, None)]
    fn test_deserialize_optional_string_to_u64(#[case] json: &str, #[case] expected: Option<u64>) {
        let result: TestOptionalStringToU64 = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case("123.45", dec!(123.45))]
    #[case("0", Decimal::ZERO)]
    #[case("0.0", Decimal::ZERO)]
    fn test_parse_decimal(#[case] input: &str, #[case] expected: Decimal) {
        let result = parse_decimal(input).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_parse_decimal_invalid() {
        assert!(parse_decimal("invalid").is_err());
        assert!(parse_decimal("").is_err());
    }

    #[rstest]
    #[case(&Some("123.45".to_string()), Some(dec!(123.45)))]
    #[case(&Some("0".to_string()), Some(Decimal::ZERO))]
    #[case(&Some(String::new()), None)]
    #[case(&None, None)]
    fn test_parse_optional_decimal(
        #[case] input: &Option<String>,
        #[case] expected: Option<Decimal>,
    ) {
        let result = parse_optional_decimal(input).unwrap();
        assert_eq!(result, expected);
    }

    // 灵活的十进制数反序列化测试（同时处理字符串和数字形式的 JSON 值）

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestFlexibleDecimal {
        #[serde(
            serialize_with = "serialize_decimal",
            deserialize_with = "deserialize_decimal"
        )]
        value: Decimal,
        #[serde(
            serialize_with = "serialize_optional_decimal",
            deserialize_with = "deserialize_optional_decimal"
        )]
        optional_value: Option<Decimal>,
    }

    #[rstest]
    #[case(r#"{"value": 123.456, "optional_value": 789.012}"#, dec!(123.456), Some(dec!(789.012)))]
    #[case(r#"{"value": "123.456", "optional_value": "789.012"}"#, dec!(123.456), Some(dec!(789.012)))]
    #[case(r#"{"value": 100, "optional_value": null}"#, dec!(100), None)]
    #[case(r#"{"value": null, "optional_value": null}"#, Decimal::ZERO, None)]
    fn test_deserialize_flexible_decimal(
        #[case] json: &str,
        #[case] expected_value: Decimal,
        #[case] expected_optional: Option<Decimal>,
    ) {
        let result: TestFlexibleDecimal = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected_value);
        assert_eq!(result.optional_value, expected_optional);
    }

    #[rstest]
    fn test_flexible_decimal_roundtrip() {
        let original = TestFlexibleDecimal {
            value: dec!(123.456),
            optional_value: Some(dec!(789.012)),
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: TestFlexibleDecimal = serde_json::from_str(&json).unwrap();

        assert_eq!(original.value, deserialized.value);
        assert_eq!(original.optional_value, deserialized.optional_value);
    }

    #[rstest]
    fn test_flexible_decimal_scientific_notation() {
        // 测试来自 serde_json 的科学计数法是否得到了正确处理。
        // serde_json 会将极小的数值（如 0.00000001）输出为 "1e-8"。
        // 注意：JSON 数值将被解析为 f64，因此有效数字被限制在大约 15 位。
        let json = r#"{"value": 0.00000001, "optional_value": 12345678.12345}"#;
        let parsed: TestFlexibleDecimal = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.value, dec!(0.00000001));
        assert_eq!(parsed.optional_value, Some(dec!(12345678.12345)));
    }

    #[rstest]
    fn test_flexible_decimal_empty_string_optional() {
        let json = r#"{"value": 100, "optional_value": ""}"#;
        let parsed: TestFlexibleDecimal = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.value, dec!(100));
        assert_eq!(parsed.optional_value, None);
    }

    // 针对 DecimalVisitor 边界情况的额外测试

    #[derive(Debug, Deserialize)]
    struct TestDecimalOnly {
        #[serde(deserialize_with = "deserialize_decimal")]
        value: Decimal,
    }

    #[rstest]
    #[case(r#"{"value": "1.5e-8"}"#, dec!(0.000000015))]
    #[case(r#"{"value": "1E10"}"#, dec!(10000000000))]
    #[case(r#"{"value": "-1.23e5"}"#, dec!(-123000))]
    fn test_deserialize_decimal_scientific_string(#[case] json: &str, #[case] expected: Decimal) {
        let result: TestDecimalOnly = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case(r#"{"value": 9223372036854775807}"#, dec!(9223372036854775807))] // i64::MAX
    #[case(r#"{"value": -9223372036854775808}"#, dec!(-9223372036854775808))] // i64::MIN
    #[case(r#"{"value": 0}"#, Decimal::ZERO)]
    fn test_deserialize_decimal_large_integers(#[case] json: &str, #[case] expected: Decimal) {
        let result: TestDecimalOnly = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case(r#"{"value": "-123.456789"}"#, dec!(-123.456789))]
    #[case(r#"{"value": -999.99}"#, dec!(-999.99))]
    fn test_deserialize_decimal_negative(#[case] json: &str, #[case] expected: Decimal) {
        let result: TestDecimalOnly = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }

    #[rstest]
    #[case(r#"{"value": "123456789.123456789012345678"}"#)] // 高精度字符串
    fn test_deserialize_decimal_high_precision(#[case] json: &str) {
        let result: TestDecimalOnly = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, dec!(123456789.123456789012345678));
    }

    #[derive(Debug, Deserialize)]
    struct TestOptionalDecimalOnly {
        #[serde(deserialize_with = "deserialize_optional_decimal")]
        value: Option<Decimal>,
    }

    #[rstest]
    #[case(r#"{"value": "1.5e-8"}"#, Some(dec!(0.000000015)))]
    #[case(r#"{"value": null}"#, None)]
    #[case(r#"{"value": ""}"#, None)]
    #[case(r#"{"value": 42}"#, Some(dec!(42)))]
    #[case(r#"{"value": -100.5}"#, Some(dec!(-100.5)))]
    fn test_deserialize_optional_decimal_various(
        #[case] json: &str,
        #[case] expected: Option<Decimal>,
    ) {
        let result: TestOptionalDecimalOnly = serde_json::from_str(json).unwrap();
        assert_eq!(result.value, expected);
    }
}
