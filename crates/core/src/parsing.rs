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

//! 核心解析函数。

/// 将长度限制在 `u8::MAX` 以内，并可选择是否记录调试日志。
#[inline]
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "用于解析，是有意为之，且已验证过值范围"
)]
fn clamp_precision_with_log(len: usize, context: &str, input: &str) -> u8 {
    if len > u8::MAX as usize {
        log::debug!(
            "{} 精度对于输入 {} 已从 {} 限制为 {}",
            context,
            input,
            len,
            u8::MAX
        );
    }
    len.min(u8::MAX as usize) as u8
}

/// 解析科学计数指数，并将其限制在 `u8::MAX` 以内。
///
/// 当 `strict` 为 false 时，如果指数无效或为空，则返回 `None`；
/// 否则，如果输入格式错误，则触发 panic。
#[inline]
#[must_use]
fn parse_scientific_exponent(exponent_str: &str, strict: bool) -> Option<u8> {
    if let Ok(exp) = exponent_str.parse::<u64>() {
        Some(exp.min(u64::from(u8::MAX)) as u8)
    } else {
        assert!(
            !(exponent_str.is_empty() && strict),
            "科学计数法格式无效：'e-' 之后缺少指数"
        );

        // 空字符串是无效的（它并非导致溢出的大数值）
        if exponent_str.is_empty() {
            return None;
        }

        // 如果全部由数字组成但超过了 u64 范围，则限制为 u8::MAX
        if exponent_str.chars().all(|c| c.is_ascii_digit()) {
            Some(u8::MAX)
        } else if strict {
            panic!("无效的科学计数指数 '{exponent_str}'：必须是有效的数字")
        } else {
            None
        }
    }
}

/// 返回从给定字符串推断出的十进制精度。
///
/// 对于具有极大负指数的科学计数法（例如 "1e-300", "1e-4294967296"），
/// 精度会被限制为 `u8::MAX` (255)，因为这代表了此系统中可表示的最大精度。
/// 这样可以在不触发 panic 的情况下处理任意大的指数。
///
/// # Panics
///
/// 如果输入字符串格式错误（例如 "1e-" 后面缺失指数值，或使用了 "1e-abc" 这种非数字指数），
/// 则会触发 panic。
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "用于解析，是有意为之，且已验证过值范围"
)]
pub fn precision_from_str(s: &str) -> u8 {
    let s = s.trim().to_ascii_lowercase();

    // 检查是否为科学计数法
    if s.contains("e-") {
        let exponent_str = s
            .split("e-")
            .nth(1)
            .expect("科学计数法格式无效：'e-' 之后缺少指数");

        return parse_scientific_exponent(exponent_str, true)
            .expect("在严格模式下 parse_scientific_exponent 应当返回 Some");
    }

    // 检查十进制精度
    if let Some((_, decimal_part)) = s.split_once('.') {
        clamp_precision_with_log(decimal_part.len(), "十进制", &s)
    } else {
        0
    }
}

/// 返回从给定字符串推断出的最小增量 (minimum increment) 精度，忽略尾随零。
///
/// 对于具有极大负指数的科学计数法（例如 "1e-300"），精度将被限制为 `u8::MAX` (255)，
/// 与 `precision_from_str` 的行为保持一致。
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "用于解析，是有意为之，且已验证过值范围"
)]
pub fn min_increment_precision_from_str(s: &str) -> u8 {
    let s = s.trim().to_ascii_lowercase();

    // 检查是否为科学计数法
    if let Some(pos) = s.find('e')
        && s[pos + 1..].starts_with('-')
    {
        let exponent_str = &s[pos + 2..];
        // 使用宽松解析（无效则返回 0，不触发 panic）
        return parse_scientific_exponent(exponent_str, false).unwrap_or(0);
    }

    // 检查十进制精度
    if let Some(dot_pos) = s.find('.') {
        let decimal_part = &s[dot_pos + 1..];
        if decimal_part.chars().any(|c| c != '0') {
            let trimmed_len = decimal_part.trim_end_matches('0').len();
            return clamp_precision_with_log(trimmed_len, "最小增量", &s);
        }
        clamp_precision_with_log(decimal_part.len(), "十进制", &s)
    } else {
        0
    }
}

/// 从给定的字节序列中返回 `usize`。
///
/// # 错误
///
/// 如果字节数不足以表示一个 `usize`，则返回错误。
pub fn bytes_to_usize(bytes: &[u8]) -> anyhow::Result<usize> {
    // 检查字节宽度
    if bytes.len() >= std::mem::size_of::<usize>() {
        let mut buffer = [0u8; std::mem::size_of::<usize>()];
        buffer.copy_from_slice(&bytes[..std::mem::size_of::<usize>()]);

        Ok(usize::from_le_bytes(buffer))
    } else {
        anyhow::bail!("字节不足，无法表示一个 `usize` 值");
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("", 0)]
    #[case("0", 0)]
    #[case("1.0", 1)]
    #[case("1.00", 2)]
    #[case("1.23456789", 8)]
    #[case("123456.789101112", 9)]
    #[case("0.000000001", 9)]
    #[case("1e-1", 1)]
    #[case("1e-2", 2)]
    #[case("1e-3", 3)]
    #[case("1e8", 0)]
    #[case("-1.23", 2)]
    #[case("-1e-2", 2)]
    #[case("1E-2", 2)]
    #[case("  1.23", 2)]
    #[case("1.23  ", 2)]
    fn test_precision_from_str(#[case] s: &str, #[case] expected: u8) {
        let result = precision_from_str(s);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("", 0)]
    #[case("0", 0)]
    #[case("1.0", 1)]
    #[case("1.00", 2)]
    #[case("1.23456789", 8)]
    #[case("123456.789101112", 9)]
    #[case("0.000000001", 9)]
    #[case("1e-1", 1)]
    #[case("1e-2", 2)]
    #[case("1e-3", 3)]
    #[case("1e8", 0)]
    #[case("-1.23", 2)]
    #[case("-1e-2", 2)]
    #[case("1E-2", 2)]
    #[case("  1.23", 2)]
    #[case("1.23  ", 2)]
    #[case("1.010", 2)]
    #[case("1.00100", 3)]
    #[case("0.0001000", 4)]
    #[case("1.000000000", 9)]
    fn test_min_increment_precision_from_str(#[case] s: &str, #[case] expected: u8) {
        let result = min_increment_precision_from_str(s);
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_bytes_to_usize_empty() {
        let payload: Vec<u8> = vec![];
        let result = bytes_to_usize(&payload);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "字节不足，无法表示一个 `usize` 值"
        );
    }

    #[rstest]
    fn test_bytes_to_usize_invalid() {
        let payload: Vec<u8> = vec![0x01, 0x02, 0x03];
        let result = bytes_to_usize(&payload);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "字节不足，无法表示一个 `usize` 值"
        );
    }

    #[rstest]
    fn test_bytes_to_usize_valid() {
        let payload: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let result = bytes_to_usize(&payload).unwrap();
        assert_eq!(result, 0x0807_0605_0403_0201);
        assert_eq!(result, 578_437_695_752_307_201);
    }

    #[rstest]
    fn test_precision_from_str_large_exponent_clamped() {
        // u8::MAX 为 255, 999 应当被限制为 255
        let result = precision_from_str("1e-999");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_precision_from_str_very_large_exponent_clamped() {
        // 非常大的指数也应当被限制为 u8::MAX
        let result = precision_from_str("1e-300");
        assert_eq!(result, 255);

        let result = precision_from_str("1e-1000000");
        assert_eq!(result, 255);
    }

    #[rstest]
    #[should_panic(expected = "Invalid scientific notation exponent")]
    fn test_precision_from_str_invalid_exponent_not_numeric() {
        let _ = precision_from_str("1e-abc");
    }

    #[rstest]
    #[should_panic(expected = "missing exponent after 'e-'")]
    fn test_precision_from_str_malformed_scientific_notation() {
        // 带有空指数的 "1e-" 应当触发 panic (对格式错误输入采用快速失败策略)
        let _ = precision_from_str("1e-");
    }

    #[rstest]
    fn test_precision_from_str_edge_case_max_u8() {
        // u8::MAX = 255，应正常工作
        let result = precision_from_str("1e-255");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_precision_from_str_just_above_max_u8() {
        // 256 应当被限制为 255
        let result = precision_from_str("1e-256");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_precision_from_str_u32_overflow() {
        // 大于 u32::MAX (4294967296) 的指数应当被限制为 255
        let result = precision_from_str("1e-4294967296");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_precision_from_str_u64_overflow() {
        // 大于 u64::MAX 的指数应当被限制为 255
        let result = precision_from_str("1e-99999999999999999999");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_min_increment_precision_from_str_large_exponent() {
        // 大指数应当被限制为 u8::MAX (255)，而不是返回 0
        let result = min_increment_precision_from_str("1e-300");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_min_increment_precision_from_str_very_large_exponent() {
        // 非常大的指数也应当被限制为 255
        let result = min_increment_precision_from_str("1e-99999999999999999999");
        assert_eq!(result, 255);
    }

    #[rstest]
    fn test_min_increment_precision_from_str_consistency() {
        // 应当对于大指数与 precision_from_str 保持一致
        let input = "1e-1000";
        let precision = precision_from_str(input);
        let min_precision = min_increment_precision_from_str(input);
        assert_eq!(precision, min_precision);
        assert_eq!(precision, 255);
    }

    #[rstest]
    fn test_min_increment_precision_from_str_empty_exponent() {
        // 空指数应当返回 0，而不是 u8::MAX
        let result = min_increment_precision_from_str("1e-");
        assert_eq!(result, 0);
    }
}
