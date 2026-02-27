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

//! Nautilus 实时系统节点的配置类型。

use std::{collections::HashMap, time::Duration};

use nautilus_common::{
    cache::CacheConfig, enums::Environment, logging::logger::LoggerConfig,
    msgbus::database::MessageBusConfig,
};
use nautilus_core::UUID4;
use nautilus_data::engine::config::DataEngineConfig;
use nautilus_execution::engine::config::ExecutionEngineConfig;
use nautilus_model::identifiers::TraderId;
use nautilus_portfolio::config::PortfolioConfig;
use nautilus_risk::engine::config::RiskEngineConfig;
use nautilus_system::config::{NautilusKernelConfig, StreamingConfig};
use serde::{Deserialize, Serialize};

/// 实盘数据引擎的配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveDataEngineConfig {
    /// 引擎内部队列缓冲区的队列大小。
    pub qsize: u32,
}

impl Default for LiveDataEngineConfig {
    fn default() -> Self {
        Self { qsize: 100_000 }
    }
}

impl From<LiveDataEngineConfig> for DataEngineConfig {
    fn from(_config: LiveDataEngineConfig) -> Self {
        Self::default()
    }
}

/// 实盘风险引擎的配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveRiskEngineConfig {
    /// 引擎内部队列缓冲区的队列大小。
    pub qsize: u32,
}

impl Default for LiveRiskEngineConfig {
    fn default() -> Self {
        Self { qsize: 100_000 }
    }
}

impl From<LiveRiskEngineConfig> for RiskEngineConfig {
    fn from(_config: LiveRiskEngineConfig) -> Self {
        Self::default()
    }
}

/// 实盘执行引擎的配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveExecEngineConfig {
    /// 启动时是否激活对账 (reconciliation)。
    pub reconciliation: bool,
    /// 启动时开始对账前的延迟（秒）。
    pub reconciliation_startup_delay_secs: f64,
    /// 对账状态的最大回顾分钟数。
    pub reconciliation_lookback_mins: Option<u32>,
    /// 要对账的具体标的 ID（如果为 None，则对账所有标的）。
    pub reconciliation_instrument_ids: Option<Vec<String>>,
    /// 是否应过滤/丢弃具有 EXTERNAL 策略 ID 的未申领订单事件。
    pub filter_unclaimed_external_orders: bool,
    /// 是否从对账中过滤持仓状态报告。
    pub filter_position_reports: bool,
    /// 要从对账中过滤的客户订单 ID。
    pub filtered_client_order_ids: Option<Vec<String>>,
    /// 是否在对账期间生成 MARKET 订单事件以对齐差异。
    pub generate_missing_orders: bool,
    /// 检查在途订单 (in-flight orders) 是否超过其阈值的间隔（毫秒）。
    pub inflight_check_interval_ms: u32,
    /// 超过此阈值（毫秒）后，将向交易所检查在途订单的状态。
    pub inflight_check_threshold_ms: u32,
    /// 验证在途订单状态的重试次数。
    pub inflight_check_retries: u32,
    /// 检查交易平台开仓订单的间隔（秒）。
    pub open_check_interval_secs: Option<f64>,
    /// 开仓订单检查的回顾分钟数。
    pub open_check_lookback_mins: Option<u32>,
    /// 订单更新后处理差异前的最小流逝时间（毫秒）。
    pub open_check_threshold_ms: u32,
    /// 遗漏开仓订单的重试次数。
    pub open_check_missing_retries: u32,
    /// `check_open_orders` 请求是否仅从交易所请求当前开仓的订单。
    pub open_check_open_only: bool,
    /// 每个一致性检查周期中单笔订单查询的最大数量。
    pub max_single_order_queries_per_cycle: u32,
    /// 连续单笔订单查询之间的延迟（毫秒）。
    pub single_order_query_delay_ms: u32,
    /// 检查交易所持仓的间隔（秒）。
    pub position_check_interval_secs: Option<f64>,
    /// 持仓一致性检查的回顾分钟数。
    pub position_check_lookback_mins: u32,
    /// 持仓更新后处理差异前的最小流逝时间（毫秒）。
    pub position_check_threshold_ms: u32,
    /// 从内存缓存中清除已关闭订单的间隔（分钟）。
    pub purge_closed_orders_interval_mins: Option<u32>,
    /// 已关闭订单被清除前的时间缓冲（分钟）。
    pub purge_closed_orders_buffer_mins: Option<u32>,
    /// 从内存缓存中清除已关闭持仓的间隔（分钟）。
    pub purge_closed_positions_interval_mins: Option<u32>,
    /// 已关闭持仓被清除前的时间缓冲（分钟）。
    pub purge_closed_positions_buffer_mins: Option<u32>,
    /// 从内存缓存中清除账户事件的间隔（分钟）。
    pub purge_account_events_interval_mins: Option<u32>,
    /// 账户事件被清除前的回顾时间缓冲（分钟）。
    pub purge_account_events_lookback_mins: Option<u32>,
    /// 清除操作是否也应从后端数据库中删除。
    pub purge_from_database: bool,
    /// 根据公开订单簿审计自有订单簿的间隔（秒）。
    pub own_books_audit_interval_secs: Option<f64>,
    /// 当队列处理遇到意外错误时，引擎是否应优雅停机。
    pub graceful_shutdown_on_error: bool,
    /// 引擎内部队列缓冲区的队列大小。
    pub qsize: u32,
}

impl Default for LiveExecEngineConfig {
    fn default() -> Self {
        Self {
            reconciliation: true,
            reconciliation_startup_delay_secs: 10.0,
            reconciliation_lookback_mins: None,
            reconciliation_instrument_ids: None,
            filter_unclaimed_external_orders: false,
            filter_position_reports: false,
            filtered_client_order_ids: None,
            generate_missing_orders: true,
            inflight_check_interval_ms: 2_000,
            inflight_check_threshold_ms: 5_000,
            inflight_check_retries: 5,
            open_check_interval_secs: None,
            open_check_lookback_mins: Some(60),
            open_check_threshold_ms: 5_000,
            open_check_missing_retries: 5,
            open_check_open_only: true,
            max_single_order_queries_per_cycle: 5,
            single_order_query_delay_ms: 100,
            position_check_interval_secs: None,
            position_check_lookback_mins: 60,
            position_check_threshold_ms: 60_000,
            purge_closed_orders_interval_mins: None,
            purge_closed_orders_buffer_mins: None,
            purge_closed_positions_interval_mins: None,
            purge_closed_positions_buffer_mins: None,
            purge_account_events_interval_mins: None,
            purge_account_events_lookback_mins: None,
            purge_from_database: false,
            own_books_audit_interval_secs: None,
            graceful_shutdown_on_error: false,
            qsize: 100_000,
        }
    }
}

impl From<LiveExecEngineConfig> for ExecutionEngineConfig {
    fn from(_config: LiveExecEngineConfig) -> Self {
        Self::default()
    }
}

/// 实盘客户端消息路由的配置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// 客户端是否应注册为默认路由客户端。
    pub default: bool,
    /// 要注册路由的交易平台 (Venues)。
    pub venues: Option<Vec<String>>,
}

/// 标的提供者 (Instrument Provider) 的配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentProviderConfig {
    /// 是否在启动时加载所有标的。
    pub load_all: bool,
    /// 是否仅加载标的 ID。
    pub load_ids: bool,
    /// 加载特定标的的过滤器。
    pub filters: HashMap<String, String>,
}

impl Default for InstrumentProviderConfig {
    fn default() -> Self {
        Self {
            load_all: false,
            load_ids: true,
            filters: HashMap::new(),
        }
    }
}

/// 实盘数据客户端的配置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LiveDataClientConfig {
    /// 当新 K 线开启时，`DataClient` 是否会发出 K 线更新信息。
    pub handle_revised_bars: bool,
    /// 客户端的标的提供者配置。
    pub instrument_provider: InstrumentProviderConfig,
    /// 客户端的消息路由配置。
    pub routing: RoutingConfig,
}

/// 实盘执行客户端的配置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LiveExecClientConfig {
    /// 客户端的标的提供者配置。
    pub instrument_provider: InstrumentProviderConfig,
    /// 客户端的消息路由配置。
    pub routing: RoutingConfig,
}

/// Nautilus 实时系统节点的配置。
#[derive(Debug, Clone)]
pub struct LiveNodeConfig {
    /// 交易环境。
    pub environment: Environment,
    /// 节点的交易员 ID。
    pub trader_id: TraderId,
    /// 启动时是否应从数据库加载交易策略状态。
    pub load_state: bool,
    /// 停止时是否应将交易策略状态保存到数据库。
    pub save_state: bool,
    /// 内核的日志配置。
    pub logging: LoggerConfig,
    /// 内核的唯一实例标识符。
    pub instance_id: Option<UUID4>,
    /// 所有客户端连接并初始化的超时时间。
    pub timeout_connection: Duration,
    /// 执行状态对账的超时时间。
    pub timeout_reconciliation: Duration,
    /// 投资组合初始化保证金和浮动盈亏的超时时间。
    pub timeout_portfolio: Duration,
    /// 所有引擎客户端断开连接的超时时间。
    pub timeout_disconnection: Duration,
    /// 节点停止后，在最终关闭前等待残留事件的延迟时间。
    pub delay_post_stop: Duration,
    /// 关闭期间等待挂起任务取消的超时时间。
    pub timeout_shutdown: Duration,
    /// 缓存配置。
    pub cache: Option<CacheConfig>,
    /// 消息总线配置。
    pub msgbus: Option<MessageBusConfig>,
    /// 投资组合配置。
    pub portfolio: Option<PortfolioConfig>,
    /// 向 feather 文件流式传输的配置。
    pub streaming: Option<StreamingConfig>,
    /// 实盘数据引擎配置。
    pub data_engine: LiveDataEngineConfig,
    /// 实盘风险引擎配置。
    pub risk_engine: LiveRiskEngineConfig,
    /// 实盘执行引擎配置。
    pub exec_engine: LiveExecEngineConfig,
    /// 数据客户端配置。
    pub data_clients: HashMap<String, LiveDataClientConfig>,
    /// 执行客户端配置。
    pub exec_clients: HashMap<String, LiveExecClientConfig>,
}

impl Default for LiveNodeConfig {
    fn default() -> Self {
        Self {
            environment: Environment::Live,
            trader_id: TraderId::from("TRADER-001"),
            load_state: false,
            save_state: false,
            logging: LoggerConfig::default(),
            instance_id: None,
            timeout_connection: Duration::from_secs(60),
            timeout_reconciliation: Duration::from_secs(30),
            timeout_portfolio: Duration::from_secs(10),
            timeout_disconnection: Duration::from_secs(10),
            delay_post_stop: Duration::from_secs(10),
            timeout_shutdown: Duration::from_secs(5),
            cache: None,
            msgbus: None,
            portfolio: None,
            streaming: None,
            data_engine: LiveDataEngineConfig::default(),
            risk_engine: LiveRiskEngineConfig::default(),
            exec_engine: LiveExecEngineConfig::default(),
            data_clients: HashMap::new(),
            exec_clients: HashMap::new(),
        }
    }
}

impl NautilusKernelConfig for LiveNodeConfig {
    fn environment(&self) -> Environment {
        self.environment
    }

    fn trader_id(&self) -> TraderId {
        self.trader_id
    }

    fn load_state(&self) -> bool {
        self.load_state
    }

    fn save_state(&self) -> bool {
        self.save_state
    }

    fn logging(&self) -> LoggerConfig {
        self.logging.clone()
    }

    fn instance_id(&self) -> Option<UUID4> {
        self.instance_id
    }

    fn timeout_connection(&self) -> Duration {
        self.timeout_connection
    }

    fn timeout_reconciliation(&self) -> Duration {
        self.timeout_reconciliation
    }

    fn timeout_portfolio(&self) -> Duration {
        self.timeout_portfolio
    }

    fn timeout_disconnection(&self) -> Duration {
        self.timeout_disconnection
    }

    fn delay_post_stop(&self) -> Duration {
        self.delay_post_stop
    }

    fn timeout_shutdown(&self) -> Duration {
        self.timeout_shutdown
    }

    fn cache(&self) -> Option<CacheConfig> {
        self.cache.clone()
    }

    fn msgbus(&self) -> Option<MessageBusConfig> {
        self.msgbus.clone()
    }

    fn data_engine(&self) -> Option<DataEngineConfig> {
        Some(self.data_engine.clone().into())
    }

    fn risk_engine(&self) -> Option<RiskEngineConfig> {
        Some(self.risk_engine.clone().into())
    }

    fn exec_engine(&self) -> Option<ExecutionEngineConfig> {
        Some(self.exec_engine.clone().into())
    }

    fn portfolio(&self) -> Option<PortfolioConfig> {
        self.portfolio.clone()
    }

    fn streaming(&self) -> Option<StreamingConfig> {
        self.streaming.clone()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn test_trading_node_config_default() {
        let config = LiveNodeConfig::default();

        assert_eq!(config.environment, Environment::Live);
        assert_eq!(config.trader_id, TraderId::from("TRADER-001"));
        assert_eq!(config.data_engine.qsize, 100_000);
        assert_eq!(config.risk_engine.qsize, 100_000);
        assert_eq!(config.exec_engine.qsize, 100_000);
        assert!(config.exec_engine.reconciliation);
        assert!(!config.exec_engine.filter_unclaimed_external_orders);
        assert!(config.data_clients.is_empty());
        assert!(config.exec_clients.is_empty());
    }

    #[rstest]
    fn test_trading_node_config_as_kernel_config() {
        let config = LiveNodeConfig::default();

        assert_eq!(config.environment(), Environment::Live);
        assert_eq!(config.trader_id(), TraderId::from("TRADER-001"));
        assert!(config.data_engine().is_some());
        assert!(config.risk_engine().is_some());
        assert!(config.exec_engine().is_some());
        assert!(!config.load_state());
        assert!(!config.save_state());
    }

    #[rstest]
    fn test_live_exec_engine_config_defaults() {
        let config = LiveExecEngineConfig::default();

        assert!(config.reconciliation);
        assert_eq!(config.reconciliation_startup_delay_secs, 10.0);
        assert_eq!(config.reconciliation_lookback_mins, None);
        assert_eq!(config.reconciliation_instrument_ids, None);
        assert_eq!(config.filtered_client_order_ids, None);
        assert!(!config.filter_unclaimed_external_orders);
        assert!(!config.filter_position_reports);
        assert!(config.generate_missing_orders);
        assert_eq!(config.inflight_check_interval_ms, 2_000);
        assert_eq!(config.inflight_check_threshold_ms, 5_000);
        assert_eq!(config.inflight_check_retries, 5);
        assert_eq!(config.open_check_threshold_ms, 5_000);
        assert_eq!(config.open_check_lookback_mins, Some(60));
        assert_eq!(config.open_check_missing_retries, 5);
        assert!(config.open_check_open_only);
        assert!(!config.purge_from_database);
        assert!(!config.graceful_shutdown_on_error);
        assert_eq!(config.qsize, 100_000);
        assert_eq!(config.reconciliation_startup_delay_secs, 10.0);
    }

    #[rstest]
    fn test_routing_config_default() {
        let config = RoutingConfig::default();

        assert!(!config.default);
        assert_eq!(config.venues, None);
    }

    #[rstest]
    fn test_live_data_client_config_default() {
        let config = LiveDataClientConfig::default();

        assert!(!config.handle_revised_bars);
        assert!(!config.instrument_provider.load_all);
        assert!(config.instrument_provider.load_ids);
        assert!(!config.routing.default);
    }
}
