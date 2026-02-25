// -------------------------------------------------------------------------------------------------
//  版权所有 (C) 2015-2026 Nautech Systems Pty Ltd。保留所有权利。
//  https://nautechsystems.io
//
//  基于 GNU Lesser General Public License 3.0 版本（“许可证”）获得许可；
//  除非符合许可证，否则您不得使用此文件。
//  您可以在 https://www.gnu.org/licenses/lgpl-3.0.en.html 获取许可证副本。
//
//  除非适用法律要求或书面同意，
//  否则根据许可证分发的软件是基于“按原样”基础分发的，
//  不附带任何明示或暗示的保证或条件。
//  请参阅许可证以了解管理许可证下的权限和限制的具体语言。
// -------------------------------------------------------------------------------------------------

//! 执行算法的配置。

use nautilus_core::serialization::default_true;
use nautilus_model::identifiers::ExecAlgorithmId;
use serde::{Deserialize, Serialize};

/// 执行算法的配置。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.trading", from_py_object)
)]
pub struct ExecutionAlgorithmConfig {
    /// 执行算法的唯一 ID。
    pub exec_algorithm_id: Option<ExecAlgorithmId>,
    /// 算法是否应记录事件日志。
    #[serde(default = "default_true")]
    pub log_events: bool,
    /// 算法是否应记录命令日志。
    #[serde(default = "default_true")]
    pub log_commands: bool,
}

impl Default for ExecutionAlgorithmConfig {
    fn default() -> Self {
        Self {
            exec_algorithm_id: None,
            log_events: true,
            log_commands: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_config_default() {
        let config = ExecutionAlgorithmConfig::default();

        assert!(config.exec_algorithm_id.is_none());
        assert!(config.log_events);
        assert!(config.log_commands);
    }

    #[rstest]
    fn test_config_with_id() {
        let exec_algorithm_id = ExecAlgorithmId::new("TWAP");
        let config = ExecutionAlgorithmConfig {
            exec_algorithm_id: Some(exec_algorithm_id),
            ..Default::default()
        };

        assert_eq!(config.exec_algorithm_id, Some(exec_algorithm_id));
    }

    #[rstest]
    fn test_config_serialization() {
        let config = ExecutionAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new("TWAP")),
            log_events: false,
            log_commands: true,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ExecutionAlgorithmConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.exec_algorithm_id, deserialized.exec_algorithm_id);
        assert_eq!(config.log_events, deserialized.log_events);
        assert_eq!(config.log_commands, deserialized.log_commands);
    }
}
