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

//! 字符串处理功能。

/// 在 `Debug` 实现中用于脱敏秘密字段的占位符。
pub const REDACTED: &str = "<redacted>";

/// 通过仅显示前 4 位和后 4 位字符来脱敏 API 密钥。
///
/// 对于长度等于或小于 8 个字符的密钥，仅返回星号。
///
/// # 示例
///
/// ```
/// use nautilus_core::string::mask_api_key;
///
/// assert_eq!(mask_api_key("abcdefghijklmnop"), "abcd...mnop");
/// assert_eq!(mask_api_key("short"), "*****");
/// ```
#[must_use]
pub fn mask_api_key(key: &str) -> String {
    // 使用 Unicode 标量以避免多字节字符导致的 panic。
    let chars: Vec<char> = key.chars().collect();
    let len = chars.len();

    if len <= 8 {
        return "*".repeat(len);
    }

    let first: String = chars[..4].iter().collect();
    let last: String = chars[len - 4..].iter().collect();

    format!("{first}...{last}")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("", "")]
    #[case("a", "*")]
    #[case("abc", "***")]
    #[case("abcdefgh", "********")]
    #[case("abcdefghi", "abcd...fghi")]
    #[case("abcdefghijklmnop", "abcd...mnop")]
    #[case("VeryLongAPIKey123456789", "Very...6789")]
    fn test_mask_api_key(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(mask_api_key(input), expected);
    }
}
