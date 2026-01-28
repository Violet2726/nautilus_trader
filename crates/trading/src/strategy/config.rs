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

use nautilus_core::serialization::{default_false, default_true};
use nautilus_model::{
    enums::OmsType,
    identifiers::{InstrumentId, StrategyId},
};
use serde::{Deserialize, Serialize};

/// 所有交易策略配置的基础模型。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.trading")
)]
pub struct StrategyConfig {
    /// 策略的唯一 ID。如果不为 None，将成为策略 ID。
    pub strategy_id: Option<StrategyId>,
    /// 策略的唯一订单 ID 标签。必须在特定交易员 ID 下的所有
    /// 运行策略中是唯一的。
    pub order_id_tag: Option<String>,
    /// 是否应使用 UUID4 作为客户端订单 ID 值。
    #[serde(default = "default_false")]
    pub use_uuid_client_order_ids: bool,
    /// 生成的客户端订单 ID 值中是否应使用连字符。
    #[serde(default = "default_true")]
    pub use_hyphens_in_client_order_ids: bool,
    /// 策略的订单管理系统类型。这将决定
    /// `ExecutionEngine` 如何处理持仓 ID。
    pub oms_type: Option<OmsType>,
    /// 外部订单认领的交易工具 ID。
    /// 匹配交易工具 ID 的外部订单将与该策略关联（由该策略认领）。
    pub external_order_claims: Option<Vec<InstrumentId>>,
    /// 是否应由策略自动管理 OTO、OCO 和 OUO **挂起的**条件订单。
    /// 任何在本地处于活动状态的模拟订单将改由 `OrderEmulator` 管理。
    #[serde(default = "default_false")]
    pub manage_contingent_orders: bool,
    /// 是否应由策略管理所有订单的 GTD（Good Till Date）有效时间过期。
    /// 如果为 True，将确保未结订单在启动时重新激活其 GTD 定时器。
    #[serde(default = "default_false")]
    pub manage_gtd_expiry: bool,
    /// 策略是否应记录事件日志。
    /// 如果为 False，则仅记录警告及以上级别的事件。
    #[serde(default = "default_true")]
    pub log_events: bool,
    /// 策略是否应记录命令日志。
    #[serde(default = "default_true")]
    pub log_commands: bool,
    /// 是否应将 `due_post_only` 为 True 的订单拒绝事件记录为警告。
    #[serde(default = "default_true")]
    pub log_rejected_due_post_only_as_warning: bool,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            strategy_id: None,
            order_id_tag: None,
            use_uuid_client_order_ids: false,
            use_hyphens_in_client_order_ids: true,
            oms_type: None,
            external_order_claims: None,
            manage_contingent_orders: false,
            manage_gtd_expiry: false,
            log_events: true,
            log_commands: true,
            log_rejected_due_post_only_as_warning: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_strategy_config_default() {
        let config = StrategyConfig::default();

        assert!(config.strategy_id.is_none());
        assert!(config.order_id_tag.is_none());
        assert!(!config.use_uuid_client_order_ids);
        assert!(config.use_hyphens_in_client_order_ids);
        assert!(config.oms_type.is_none());
        assert!(config.external_order_claims.is_none());
        assert!(!config.manage_contingent_orders);
        assert!(!config.manage_gtd_expiry);
        assert!(config.log_events);
        assert!(config.log_commands);
        assert!(config.log_rejected_due_post_only_as_warning);
    }

    #[rstest]
    fn test_strategy_config_with_strategy_id() {
        let strategy_id = StrategyId::from("TEST-001");
        let config = StrategyConfig {
            strategy_id: Some(strategy_id),
            ..Default::default()
        };

        assert_eq!(config.strategy_id, Some(strategy_id));
    }

    #[rstest]
    fn test_strategy_config_serialization() {
        let config = StrategyConfig {
            strategy_id: Some(StrategyId::from("TEST-001")),
            order_id_tag: Some("TAG1".to_string()),
            use_uuid_client_order_ids: true,
            ..Default::default()
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: StrategyConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.strategy_id, deserialized.strategy_id);
        assert_eq!(config.order_id_tag, deserialized.order_id_tag);
        assert_eq!(
            config.use_uuid_client_order_ids,
            deserialized.use_uuid_client_order_ids
        );
    }
}
