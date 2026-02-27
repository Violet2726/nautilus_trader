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

//! `UUID4` 通用唯一识别码 (UUID) 版本 4 (RFC 4122)。

use std::{
    ffi::CStr,
    fmt::{Debug, Display},
    hash::Hash,
    io::{Cursor, Write},
    str::FromStr,
};

use rand::Rng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// `UUID4` 字符串值的最大 ASCII 字符长度（包含 null 终止符）。
pub(crate) const UUID4_LEN: usize = 37;

/// 表示根据 RFC 4122 规范基于 128 位标签生成的
/// 版本 4 通用唯一识别码 (UUID)。
#[repr(C)]
#[derive(Copy, Clone, Hash, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.core", from_py_object)
)]
pub struct UUID4 {
    /// 作为固定长度 C 字符串字节数组存储的 UUID v4 值（包含 null 终止符）。
    pub(crate) value: [u8; 37], // cbindgen 在数组中使用常量时存在问题
}

impl UUID4 {
    /// 创建一个新 [`UUID4`] 实例。
    ///
    /// UUID 值作为固定长度的 C 字符串字节数组进行存储。
    #[must_use]
    pub fn new() -> Self {
        let mut rng = rand::rng();
        let mut bytes = [0u8; 16];
        rng.fill_bytes(&mut bytes);

        bytes[6] = (bytes[6] & 0x0F) | 0x40; // 将版本设为 4
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // 将变体设为 RFC 4122

        let mut value = [0u8; UUID4_LEN];
        let mut cursor = Cursor::new(&mut value[..36]);

        write!(
            cursor,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            u16::from_be_bytes([bytes[4], bytes[5]]),
            u16::from_be_bytes([bytes[6], bytes[7]]),
            u16::from_be_bytes([bytes[8], bytes[9]]),
            u64::from_be_bytes([
                bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], 0, 0
            ]) >> 16
        )
        .expect("将 UUID 字符串写入缓冲区时出错");

        value[36] = 0; // 添加 null 终止符

        Self { value }
    }

    /// 将 [`UUID4`] 转换为 C 字符串引用。
    ///
    /// # Panics
    ///
    /// 如果内部字节数组不是有效的 C 字符串（即不以 null 终止符结尾），则触发 panic。
    #[must_use]
    pub fn to_cstr(&self) -> &CStr {
        // 安全性：我们始终存储有效的 C 字符串
        CStr::from_bytes_with_nul(&self.value)
            .expect("UUID 的字节表示形式应当是有效的 C 字符串")
    }

    /// 将 UUID 作为字符串切片返回。
    #[must_use]
    pub fn as_str(&self) -> &str {
        // 安全性：我们始终存储有效的 ASCII UUID 字符串
        self.to_cstr().to_str().expect("UUID 应当是有效的 UTF-8 编码")
    }

    /// 返回原始 UUID 字节（16 字节）。
    ///
    /// 此方法针对序列化进行了优化，在此场景下可以直接获取 UUID 字节
    /// 而无需进行字符串转换的开销。
    #[must_use]
    pub fn as_bytes(&self) -> [u8; 16] {
        // 解析字符串表示形式以提取原始字节
        // 此操作在读取时仅执行一次，以避免重复解析
        let uuid_str = self.to_cstr().to_str().expect("有效的 UTF-8 编码");
        let uuid = Uuid::parse_str(uuid_str).expect("有效的 UUID4");
        *uuid.as_bytes()
    }

    fn validate_v4(uuid: &Uuid) {
        // 验证这是否为 v4 UUID
        assert_eq!(
            uuid.get_version(),
            Some(uuid::Version::Random),
            "UUID 不是版本 4"
        );

        // 验证 RFC4122 变体
        assert_eq!(
            uuid.get_variant(),
            uuid::Variant::RFC4122,
            "UUID 不是 RFC 4122 变体"
        );
    }

    fn try_validate_v4(uuid: &Uuid) -> Result<(), String> {
        if uuid.get_version() != Some(uuid::Version::Random) {
            return Err("UUID 不是版本 4".to_string());
        }
        if uuid.get_variant() != uuid::Variant::RFC4122 {
            return Err("UUID 不是 RFC 4122 变体".to_string());
        }
        Ok(())
    }

    fn from_validated_uuid(uuid: &Uuid) -> Self {
        let mut value = [0; UUID4_LEN];
        let uuid_str = uuid.to_string();
        value[..uuid_str.len()].copy_from_slice(uuid_str.as_bytes());
        value[uuid_str.len()] = 0; // 添加 null 终止符
        Self { value }
    }
}

impl FromStr for UUID4 {
    type Err = String;

    /// 尝试从字符串表示形式创建 [`UUID4`]。
    ///
    /// 字符串应当是标准格式的有效 UUID (例如 "2d89666b-1a1e-4a75-b193-4eb3b454c757")。
    ///
    /// # Errors
    ///
    /// 如果 `value` 不是符合 RFC 4122 的有效版本 4 UUID，则返回错误。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::try_parse(value).map_err(|e| e.to_string())?;
        Self::try_validate_v4(&uuid)?;
        Ok(Self::from_validated_uuid(&uuid))
    }
}

impl From<&str> for UUID4 {
    fn from(value: &str) -> Self {
        Self::from_str(value).expect("无效的 UUID4 字符串")
    }
}

impl From<String> for UUID4 {
    fn from(value: String) -> Self {
        Self::from_str(&value).expect("无效的 UUID4 字符串")
    }
}

impl From<uuid::Uuid> for UUID4 {
    /// 从 [`uuid::Uuid`] 创建 [`UUID4`]。
    ///
    /// # Panics
    ///
    /// 如果 `value` 不是符合 RFC 4122 的有效版本 4 UUID，则触发 panic。
    fn from(value: uuid::Uuid) -> Self {
        Self::validate_v4(&value);
        Self::from_validated_uuid(&value)
    }
}

impl From<UUID4> for uuid::Uuid {
    /// 从 [`UUID4`] 创建 [`uuid::Uuid`]。
    fn from(value: UUID4) -> Self {
        Self::from_bytes(value.as_bytes())
    }
}

impl Default for UUID4 {
    /// 创建一个新的默认 [`UUID4`] 实例。
    ///
    /// 默认的 UUID4 即是一个新生成的版本 4 UUID。
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for UUID4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", stringify!(UUID4), self)
    }
}

impl Display for UUID4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_cstr().to_string_lossy())
    }
}

impl Serialize for UUID4 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UUID4 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let uuid4_str: &str = Deserialize::deserialize(deserializer)?;
        uuid4_str.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    use rstest::*;
    use uuid;

    use super::*;

    #[rstest]
    fn test_new() {
        let uuid = UUID4::new();
        let uuid_string = uuid.to_string();
        let uuid_parsed = Uuid::parse_str(&uuid_string).unwrap();
        assert_eq!(uuid_parsed.get_version().unwrap(), uuid::Version::Random);
        assert_eq!(uuid_parsed.to_string().len(), 36);

        // 版本 4 要求位：0b0100xxxx
        assert_eq!(&uuid_string[14..15], "4");
        // RFC4122 变体要求位：0b10xxxxxx
        let variant_char = &uuid_string[19..20];
        assert!(matches!(variant_char, "8" | "9" | "a" | "b" | "A" | "B"));
    }

    #[rstest]
    fn test_uuid_format() {
        let uuid = UUID4::new();
        let bytes = uuid.value;

        // 检查是否以 null 终止
        assert_eq!(bytes[36], 0);

        // 验证连字符位置
        assert_eq!(bytes[8] as char, '-');
        assert_eq!(bytes[13] as char, '-');
        assert_eq!(bytes[18] as char, '-');
        assert_eq!(bytes[23] as char, '-');

        let s = uuid.to_string();
        assert_eq!(s.chars().nth(14).unwrap(), '4');
    }

    #[rstest]
    #[should_panic(expected = "UUID is not version 4")]
    fn test_from_str_with_non_version_4_uuid_panics() {
        let uuid_string = "6ba7b810-9dad-11d1-80b4-00c04fd430c8"; // v1 UUID
        let _ = UUID4::from(uuid_string);
    }

    #[rstest]
    fn test_case_insensitive_parsing() {
        let upper = "2D89666B-1A1E-4A75-B193-4EB3B454C757";
        let lower = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid_upper = UUID4::from(upper);
        let uuid_lower = UUID4::from(lower);

        assert_eq!(uuid_upper, uuid_lower);
        assert_eq!(uuid_upper.to_string(), lower);
    }

    #[rstest]
    #[case("6ba7b810-9dad-11d1-80b4-00c04fd430c8")] // v1 (基于时间)
    #[case("000001f5-8fa9-21d1-9df3-00e098032b8c")] // v2 (DCE 安全)
    #[case("3d813cbb-47fb-32ba-91df-831e1593ac29")] // v3 (MD5 哈希)
    #[case("fb4f37c1-4ba3-5173-9812-2b90e76a06f7")] // v5 (SHA-1 哈希)
    #[should_panic(expected = "UUID is not version 4")]
    fn test_invalid_version(#[case] uuid_string: &str) {
        let _ = UUID4::from(uuid_string);
    }

    #[rstest]
    #[should_panic(expected = "UUID is not RFC 4122 variant")]
    fn test_non_rfc4122_variant() {
        // 虽然是有效的 v4，但是变体错误
        let uuid = "550e8400-e29b-41d4-0000-446655440000";
        let _ = UUID4::from(uuid);
    }

    #[rstest]
    #[case("")] // 空字符串
    #[case("not-a-uuid-at-all")] // 无效格式
    #[case("6ba7b810-9dad-11d1-80b4")] // 过短
    #[case("6ba7b810-9dad-11d1-80b4-00c04fd430c8-extra")] // 过长
    #[case("6ba7b810-9dad-11d1-80b4=00c04fd430c8")] // 分隔符错误
    #[case("6ba7b81019dad111d180b400c04fd430c8")] // 无分隔符
    #[case("6ba7b810-9dad-11d1-80b4-00c04fd430")] // 被截断
    #[case("6ba7b810-9dad-11d1-80b4-00c04fd430cg")] // 无效的十六进制字符
    fn test_invalid_uuid_cases(#[case] invalid_uuid: &str) {
        assert!(UUID4::from_str(invalid_uuid).is_err());
    }

    #[rstest]
    fn test_default() {
        let uuid: UUID4 = UUID4::default();
        let uuid_string = uuid.to_string();
        let uuid_parsed = Uuid::parse_str(&uuid_string).unwrap();
        assert_eq!(uuid_parsed.get_version().unwrap(), uuid::Version::Random);
    }

    #[rstest]
    fn test_from_str() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);
        let result_string = uuid.to_string();
        let result_parsed = Uuid::parse_str(&result_string).unwrap();
        let expected_parsed = Uuid::parse_str(uuid_string).unwrap();
        assert_eq!(result_parsed, expected_parsed);
    }

    #[rstest]
    fn test_from_uuid() {
        let original = uuid::Uuid::new_v4();
        let uuid4 = UUID4::from(original);
        assert_eq!(uuid4.to_string(), original.to_string());
    }

    #[rstest]
    fn test_equality() {
        let uuid1 = UUID4::from("2d89666b-1a1e-4a75-b193-4eb3b454c757");
        let uuid2 = UUID4::from("46922ecb-4324-4e40-a56c-841e0d774cef");
        assert_eq!(uuid1, uuid1);
        assert_ne!(uuid1, uuid2);
    }

    #[rstest]
    fn test_debug() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);
        assert_eq!(format!("{uuid:?}"), format!("UUID4({uuid_string})"));
    }

    #[rstest]
    fn test_display() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);
        assert_eq!(format!("{uuid}"), uuid_string);
    }

    #[rstest]
    fn test_to_cstr() {
        let uuid = UUID4::new();
        let cstr = uuid.to_cstr();

        assert_eq!(cstr.to_str().unwrap(), uuid.to_string());
        assert_eq!(cstr.to_bytes_with_nul()[36], 0);
    }

    #[rstest]
    fn test_as_str() {
        let uuid = UUID4::new();
        let s = uuid.as_str();

        assert_eq!(s, uuid.to_string());
        assert_eq!(s.len(), 36);
    }

    #[rstest]
    fn test_hash_consistency() {
        let uuid = UUID4::new();

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();

        uuid.hash(&mut hasher1);
        uuid.hash(&mut hasher2);

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[rstest]
    fn test_serialize_json() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);

        let serialized = serde_json::to_string(&uuid).unwrap();
        let expected_json = format!("\"{uuid_string}\"");
        assert_eq!(serialized, expected_json);
    }

    #[rstest]
    fn test_deserialize_json() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let serialized = format!("\"{uuid_string}\"");

        let deserialized: UUID4 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.to_string(), uuid_string);
    }

    #[rstest]
    fn test_serialize_deserialize_round_trip() {
        let uuid = UUID4::new();

        let serialized = serde_json::to_string(&uuid).unwrap();
        let deserialized: UUID4 = serde_json::from_str(&serialized).unwrap();

        assert_eq!(uuid, deserialized);
    }

    #[rstest]
    fn test_as_bytes() {
        let uuid_string = "2d89666b-1a1e-4a75-b193-4eb3b454c757";
        let uuid = UUID4::from(uuid_string);

        let bytes = uuid.as_bytes();
        assert_eq!(bytes.len(), 16);

        // 使用字节重新构造 UUID 并验证匹配
        let reconstructed = Uuid::from_bytes(bytes);
        assert_eq!(reconstructed.to_string(), uuid_string);

        // 验证版本 4
        assert_eq!(reconstructed.get_version().unwrap(), uuid::Version::Random);
    }

    #[rstest]
    fn test_as_bytes_round_trip() {
        let uuid1 = UUID4::new();
        let bytes = uuid1.as_bytes();
        let uuid2 = UUID4::from(Uuid::from_bytes(bytes));

        assert_eq!(uuid1, uuid2);
    }

    #[rstest]
    #[case("\"not-a-uuid\"")] // 无效格式
    #[case("\"6ba7b810-9dad-11d1-80b4-00c04fd430c8\"")] // v1 UUID (版本错误)
    #[case("\"\"")] // 空字符串
    fn test_deserialize_invalid_uuid_returns_error(#[case] json: &str) {
        let result: Result<UUID4, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
