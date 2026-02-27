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

//! 用于构建 [`LiveNode`] 实例的生成器 (Builder)。

use std::{collections::HashMap, time::Duration};

use nautilus_common::{enums::Environment, logging::logger::LoggerConfig};
use nautilus_core::UUID4;
use nautilus_data::client::DataClientAdapter;
use nautilus_model::identifiers::TraderId;
use nautilus_system::{
    factories::{ClientConfig, DataClientFactory, ExecutionClientFactory},
    kernel::NautilusKernel,
};

use crate::{
    config::LiveNodeConfig,
    manager::{ExecutionManager, ExecutionManagerConfig},
    node::LiveNode,
    runner::AsyncRunner,
};

/// 用于通过流畅 (fluent) API 构建 [`LiveNode`] 的生成器。
///
/// 提供专门针对实盘节点的配置选项，包括客户端工厂注册和超时设置。
#[derive(Debug)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.live", unsendable)
)]
pub struct LiveNodeBuilder {
    name: String,
    config: LiveNodeConfig,
    data_client_factories: HashMap<String, Box<dyn DataClientFactory>>,
    exec_client_factories: HashMap<String, Box<dyn ExecutionClientFactory>>,
    data_client_configs: HashMap<String, Box<dyn ClientConfig>>,
    exec_client_configs: HashMap<String, Box<dyn ClientConfig>>,
}

impl LiveNodeBuilder {
    /// 使用必需参数创建一个新的 [`LiveNodeBuilder`]。
    ///
    /// # 错误
    ///
    /// 如果 `environment` 无效（为 BACKTEST），则返回错误。
    pub fn new(trader_id: TraderId, environment: Environment) -> anyhow::Result<Self> {
        match environment {
            Environment::Sandbox | Environment::Live => {}
            Environment::Backtest => {
                anyhow::bail!("LiveNode 不能用于回测 (Backtest) 环境");
            }
        }

        let config = LiveNodeConfig {
            environment,
            trader_id,
            ..Default::default()
        };

        Ok(Self {
            name: "LiveNode".to_string(),
            config,
            data_client_factories: HashMap::new(),
            exec_client_factories: HashMap::new(),
            data_client_configs: HashMap::new(),
            exec_client_configs: HashMap::new(),
        })
    }

    /// 返回节点的名称。
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 设置节点的名称。
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// 设置节点的实例 ID。
    #[must_use]
    pub const fn with_instance_id(mut self, instance_id: UUID4) -> Self {
        self.config.instance_id = Some(instance_id);
        self
    }

    /// 配置是否在启动时加载状态。
    #[must_use]
    pub const fn with_load_state(mut self, load_state: bool) -> Self {
        self.config.load_state = load_state;
        self
    }

    /// 配置是否在停机时保存状态。
    #[must_use]
    pub const fn with_save_state(mut self, save_state: bool) -> Self {
        self.config.save_state = save_state;
        self
    }

    /// 以秒为单位设置连接超时。
    #[must_use]
    pub const fn with_timeout_connection(mut self, timeout_secs: u64) -> Self {
        self.config.timeout_connection = Duration::from_secs(timeout_secs);
        self
    }

    /// 以秒为单位设置对账 (reconciliation) 超时。
    #[must_use]
    pub const fn with_timeout_reconciliation(mut self, timeout_secs: u64) -> Self {
        self.config.timeout_reconciliation = Duration::from_secs(timeout_secs);
        self
    }

    /// 配置是否运行启动对账。
    #[must_use]
    pub fn with_reconciliation(mut self, reconciliation: bool) -> Self {
        self.config.exec_engine.reconciliation = reconciliation;
        self
    }

    /// 以分钟为单位设置对账回顾时间 (lookback)。
    #[must_use]
    pub fn with_reconciliation_lookback_mins(mut self, mins: u32) -> Self {
        self.config.exec_engine.reconciliation_lookback_mins = Some(mins);
        self
    }

    /// 以秒为单位设置投资组合初始化超时。
    #[must_use]
    pub const fn with_timeout_portfolio(mut self, timeout_secs: u64) -> Self {
        self.config.timeout_portfolio = Duration::from_secs(timeout_secs);
        self
    }

    /// 以秒为单位设置断开连接超时。
    #[must_use]
    pub const fn with_timeout_disconnection_secs(mut self, timeout_secs: u64) -> Self {
        self.config.timeout_disconnection = Duration::from_secs(timeout_secs);
        self
    }

    /// 以秒为单位设置停止后的延迟。
    #[must_use]
    pub const fn with_delay_post_stop_secs(mut self, delay_secs: u64) -> Self {
        self.config.delay_post_stop = Duration::from_secs(delay_secs);
        self
    }

    /// 以秒为单位设置停机超时。
    #[must_use]
    pub const fn with_delay_shutdown_secs(mut self, delay_secs: u64) -> Self {
        self.config.timeout_shutdown = Duration::from_secs(delay_secs);
        self
    }

    /// 设置日志配置。
    #[must_use]
    pub fn with_logging(mut self, logging: LoggerConfig) -> Self {
        self.config.logging = logging;
        self
    }

    /// 添加带配置的数据客户端工厂。
    ///
    /// # 错误
    ///
    /// 如果已注册同名的客户端，则返回错误。
    pub fn add_data_client(
        mut self,
        name: Option<String>,
        factory: Box<dyn DataClientFactory>,
        config: Box<dyn ClientConfig>,
    ) -> anyhow::Result<Self> {
        let name = name.unwrap_or_else(|| factory.name().to_string());

        if self.data_client_factories.contains_key(&name) {
            anyhow::bail!("数据客户端 '{name}' 已注册");
        }

        self.data_client_factories.insert(name.clone(), factory);
        self.data_client_configs.insert(name, config);
        Ok(self)
    }

    /// 添加带配置的执行客户端工厂。
    ///
    /// # 错误
    ///
    /// 如果已注册同名的客户端，则返回错误。
    pub fn add_exec_client(
        mut self,
        name: Option<String>,
        factory: Box<dyn ExecutionClientFactory>,
        config: Box<dyn ClientConfig>,
    ) -> anyhow::Result<Self> {
        let name = name.unwrap_or_else(|| factory.name().to_string());

        if self.exec_client_factories.contains_key(&name) {
            anyhow::bail!("执行客户端 '{name}' 已注册");
        }

        self.exec_client_factories.insert(name.clone(), factory);
        self.exec_client_configs.insert(name, config);
        Ok(self)
    }

    /// 根据配置的设置构建 [`LiveNode`]。
    ///
    /// 此操作将执行：
    /// 1. 构建底层内核。
    /// 2. 使用工厂创建客户端。
    /// 3. 在引擎中注册客户端。
    ///
    /// # 错误
    ///
    /// 如果节点构造失败，则返回错误。
    pub fn build(mut self) -> anyhow::Result<LiveNode> {
        log::info!(
            "正在构建 LiveNode，包含 {} 个数据客户端和 {} 个执行客户端",
            self.data_client_factories.len(),
            self.exec_client_factories.len()
        );

        let runner = AsyncRunner::new();
        let kernel = NautilusKernel::new(self.name.clone(), self.config.clone())?;

        for (name, factory) in self.data_client_factories {
            if let Some(config) = self.data_client_configs.remove(&name) {
                log::debug!("正在创建数据客户端 {name}");

                let client =
                    factory.create(&name, config.as_ref(), kernel.cache(), kernel.clock())?;
                let client_id = client.client_id();
                let venue = client.venue();

                let adapter = DataClientAdapter::new(
                    client_id, venue, true, // handles_order_book_deltas
                    true, // handles_order_book_snapshots
                    client,
                );

                kernel
                    .data_engine
                    .borrow_mut()
                    .register_client(adapter, venue);

                log::info!("已注册 DataClient-{client_id}");
            } else {
                log::warn!("未找到数据客户端工厂 {name} 的配置");
            }
        }

        for (name, factory) in self.exec_client_factories {
            if let Some(config) = self.exec_client_configs.remove(&name) {
                log::debug!("正在创建执行客户端 {name}");

                let client = factory.create(&name, config.as_ref(), kernel.cache())?;
                let client_id = client.client_id();

                kernel.exec_engine.borrow_mut().register_client(client)?;

                log::info!("已注册 ExecutionClient-{client_id}");
            } else {
                log::warn!("未找到执行客户端工厂 {name} 的配置");
            }
        }

        let exec_manager_config = ExecutionManagerConfig::from(&self.config.exec_engine)
            .with_trader_id(self.config.trader_id);
        let exec_manager = ExecutionManager::new(
            kernel.clock.clone(),
            kernel.cache.clone(),
            exec_manager_config,
        );

        log::info!("构建成功");

        Ok(LiveNode::new_from_builder(
            kernel,
            runner,
            self.config,
            exec_manager,
        ))
    }
}
