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

//! 跨平台环境变量工具。
//!
//! 此模块提供用于安全访问环境变量的函数，并带有适当的错误处理。

/// 返回给定 `key` 对应的环境变量的值。
///
/// # 错误
///
/// 如果未设置该环境变量，则返回错误。
pub fn get_env_var(key: &str) -> anyhow::Result<String> {
    match std::env::var(key) {
        Ok(var) => Ok(var),
        Err(_) => anyhow::bail!("必须设置环境变量 '{key}'"),
    }
}

/// 如果 `value` 为 `Some`，则返回提供的 `value`；否则回退到读取给定 `key` 对应的环境变量。
///
/// 仅在 `value` 为 `None` 时尝试读取环境变量，从而避免不必要的环境变量查询和错误。
///
/// # 错误
///
/// 如果 `value` 为 `None` 且未设置对应的环境变量，则返回错误。
pub fn get_or_env_var(value: Option<String>, key: &str) -> anyhow::Result<String> {
    match value {
        Some(v) => Ok(v),
        None => get_env_var(key),
    }
}

/// 如果 `value` 为 `Some`，则返回提供的 `value`；否则回退到读取给定 `key` 对应的环境变量。
///
/// 与 [`get_or_env_var`] 不同，当未设置环境变量时，此函数返回 `None` 而不是错误。
/// 适用于可接受缺失值的情形（例如，仅支持公开数据的 API 客户端）。
#[must_use]
pub fn get_or_env_var_opt(value: Option<String>, key: &str) -> Option<String> {
    value.or_else(|| std::env::var(key).ok())
}

/// 从提供的值或环境变量中解析 key/secret 对。
///
/// 当两者均可用时，返回 `Some((key, secret))`；否则返回 `None`。
#[must_use]
pub fn resolve_env_var_pair(
    key: Option<String>,
    secret: Option<String>,
    key_var: &str,
    secret_var: &str,
) -> Option<(String, String)> {
    let key = get_or_env_var_opt(key, key_var)?;
    let secret = get_or_env_var_opt(secret, secret_var)?;
    Some((key, secret))
}

#[cfg(test)]
mod tests {
    use rstest::*;

    use super::*;

    #[rstest]
    fn test_get_env_var_success() {
        // 使用一个通用的环境变量进行测试
        if let Ok(path) = std::env::var("PATH") {
            let result = get_env_var("PATH");
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), path);
        }
    }

    #[rstest]
    fn test_get_env_var_not_set() {
        // 使用一个极不可能存在的环境变量名称
        let result = get_env_var("NONEXISTENT_ENV_VAR_THAT_SHOULD_NOT_EXIST_12345");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(
            "必须设置环境变量 'NONEXISTENT_ENV_VAR_THAT_SHOULD_NOT_EXIST_12345'"
        ));
    }

    #[rstest]
    fn test_get_env_var_error_message_format() {
        let var_name = "DEFINITELY_NONEXISTENT_VAR_123456789";
        let result = get_env_var(var_name);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains(var_name));
        assert!(error_msg.contains("必须设置"));
    }

    #[rstest]
    fn test_get_or_env_var_with_some_value() {
        let provided_value = Some("provided_value".to_string());
        let result = get_or_env_var(provided_value, "PATH");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "provided_value");
    }

    #[rstest]
    fn test_get_or_env_var_with_none_and_env_var_set() {
        // 使用一个通用的环境变量进行测试
        if let Ok(path) = std::env::var("PATH") {
            let result = get_or_env_var(None, "PATH");
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), path);
        }
    }

    #[rstest]
    fn test_get_or_env_var_with_none_and_env_var_not_set() {
        let result = get_or_env_var(None, "NONEXISTENT_ENV_VAR_THAT_SHOULD_NOT_EXIST_67890");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains(
            "必须设置环境变量 'NONEXISTENT_ENV_VAR_THAT_SHOULD_NOT_EXIST_67890'"
        ));
    }

    #[rstest]
    fn test_get_or_env_var_empty_string_value() {
        // 空字符串仍是应当被返回的有效值
        let provided_value = Some(String::new());
        let result = get_or_env_var(provided_value, "PATH");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[rstest]
    fn test_get_or_env_var_priority() {
        // 当提供的 value 和环境变量均可用时，value 具有更高优先级
        // 使用 PATH 是因为它在大多数环境中都可用
        if std::env::var("PATH").is_ok() {
            let provided = Some("custom_value_takes_priority".to_string());
            let result = get_or_env_var(provided, "PATH");
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "custom_value_takes_priority");
        }
    }

    #[rstest]
    fn test_get_or_env_var_opt_with_some_value() {
        let provided_value = Some("provided_value".to_string());
        let result = get_or_env_var_opt(provided_value, "PATH");
        assert_eq!(result, Some("provided_value".to_string()));
    }

    #[rstest]
    fn test_get_or_env_var_opt_with_none_and_env_var_set() {
        if let Ok(path) = std::env::var("PATH") {
            let result = get_or_env_var_opt(None, "PATH");
            assert_eq!(result, Some(path));
        }
    }

    #[rstest]
    fn test_get_or_env_var_opt_with_none_and_env_var_not_set() {
        let result = get_or_env_var_opt(None, "NONEXISTENT_ENV_VAR_OPT_12345");
        assert_eq!(result, None);
    }

    #[rstest]
    fn test_get_or_env_var_opt_priority() {
        // 当提供的 value 和环境变量均可用时，value 具有更高优先级
        if std::env::var("PATH").is_ok() {
            let provided = Some("custom_value".to_string());
            let result = get_or_env_var_opt(provided, "PATH");
            assert_eq!(result, Some("custom_value".to_string()));
        }
    }

    #[rstest]
    fn test_resolve_env_var_pair_both_provided() {
        let result = resolve_env_var_pair(
            Some("my_key".to_string()),
            Some("my_secret".to_string()),
            "NONEXISTENT_KEY_VAR",
            "NONEXISTENT_SECRET_VAR",
        );
        assert_eq!(
            result,
            Some(("my_key".to_string(), "my_secret".to_string()))
        );
    }

    #[rstest]
    fn test_resolve_env_var_pair_key_missing_returns_none() {
        let result = resolve_env_var_pair(
            None,
            Some("my_secret".to_string()),
            "NONEXISTENT_PAIR_KEY_12345",
            "NONEXISTENT_PAIR_SECRET_12345",
        );
        assert_eq!(result, None);
    }

    #[rstest]
    fn test_resolve_env_var_pair_secret_missing_returns_none() {
        let result = resolve_env_var_pair(
            Some("my_key".to_string()),
            None,
            "NONEXISTENT_PAIR_KEY_12345",
            "NONEXISTENT_PAIR_SECRET_12345",
        );
        assert_eq!(result, None);
    }

    #[rstest]
    fn test_resolve_env_var_pair_both_missing_returns_none() {
        let result = resolve_env_var_pair(
            None,
            None,
            "NONEXISTENT_PAIR_KEY_12345",
            "NONEXISTENT_PAIR_SECRET_12345",
        );
        assert_eq!(result, None);
    }
}
