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

use std::{
    fmt::Debug,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

use nautilus_common::{
    actor::{Actor, DataActor},
    cache::database::CacheDatabaseAdapter,
    component::Component,
    enums::{Environment, LogColor},
    log_info,
    messages::{DataEvent, ExecutionEvent, data::DataCommand, execution::TradingCommand},
    timer::TimeEventHandler,
};
use nautilus_core::UUID4;
use nautilus_model::{
    events::OrderEventAny,
    identifiers::{StrategyId, TraderId},
};
use nautilus_system::{config::NautilusKernelConfig, kernel::NautilusKernel};
use nautilus_trading::strategy::Strategy;
use tabled::{Table, Tabled, settings::Style};

use crate::{
    builder::LiveNodeBuilder,
    config::LiveNodeConfig,
    manager::{ExecutionManager, ExecutionManagerConfig},
    runner::{AsyncRunner, AsyncRunnerChannels},
};

/// `LiveNode` 运行器的生命周期状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeState {
    #[default]
    Idle = 0,
    Starting = 1,
    Running = 2,
    ShuttingDown = 3,
    Stopped = 4,
}

impl NodeState {
    /// 从 `u8` 表示形式创建一个 `NodeState`。
    ///
    /// # Panics
    ///
    /// 如果值不是有效的 `NodeState` 判别值 (0-4)，则会发生恐慌。
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Idle,
            1 => Self::Starting,
            2 => Self::Running,
            3 => Self::ShuttingDown,
            4 => Self::Stopped,
            _ => panic!("无效的 NodeState 值"),
        }
    }

    /// 返回此状态的 `u8` 表示形式。
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// 返回状态是否为 `Running`。
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }
}

/// 用于从其他线程控制 `LiveNode` 的线程安全句柄。
///
/// 这允许在不要求节点本身为 Send + Sync 的情况下停止和查询节点状态。
#[derive(Clone, Debug)]
pub struct LiveNodeHandle {
    /// 指示节点是否应该停止的原子标志。
    pub(crate) stop_flag: Arc<AtomicBool>,
    /// 原子状态，对应 `NodeState::as_u8()`。
    pub(crate) state: Arc<AtomicU8>,
}

impl Default for LiveNodeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveNodeHandle {
    /// 创建一个具有默认（`Idle`）状态的新句柄。
    #[must_use]
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(NodeState::Idle.as_u8())),
        }
    }

    /// 设置节点状态（内部使用）。
    pub(crate) fn set_state(&self, state: NodeState) {
        self.state.store(state.as_u8(), Ordering::Relaxed);
        if state == NodeState::Running {
            // 进入运行状态时清除停止标志
            self.stop_flag.store(false, Ordering::Relaxed);
        }
    }

    /// 返回当前节点状态。
    #[must_use]
    pub fn state(&self) -> NodeState {
        NodeState::from_u8(self.state.load(Ordering::Relaxed))
    }

    /// 返回节点是否应该停止。
    #[must_use]
    pub fn should_stop(&self) -> bool {
        self.stop_flag.load(Ordering::Relaxed)
    }

    /// 返回节点当前是否正在运行。
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state().is_running()
    }

    /// 向节点发送停止信号。
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// 实盘 Nautilus 系统节点的高层抽象。
///
/// 提供了一个简化的接口，用于运行实盘系统，
/// 具有自动客户端管理和生命周期处理功能。
#[derive(Debug)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.live", unsendable)
)]
pub struct LiveNode {
    kernel: NautilusKernel,
    runner: Option<AsyncRunner>,
    config: LiveNodeConfig,
    handle: LiveNodeHandle,
    exec_manager: ExecutionManager,
    shutdown_deadline: Option<tokio::time::Instant>,
    #[cfg(feature = "python")]
    #[allow(dead_code)] // TODO: Under development
    python_actors: Vec<pyo3::Py<pyo3::PyAny>>,
}

impl LiveNode {
    /// 从构建器组件创建一个新的 `LiveNode`。
    ///
    /// 这是由 `LiveNodeBuilder` 使用的内部构造函数。
    #[must_use]
    pub(crate) fn new_from_builder(
        kernel: NautilusKernel,
        runner: AsyncRunner,
        config: LiveNodeConfig,
        exec_manager: ExecutionManager,
    ) -> Self {
        Self {
            kernel,
            runner: Some(runner),
            config,
            handle: LiveNodeHandle::new(),
            exec_manager,
            shutdown_deadline: None,
            #[cfg(feature = "python")]
            python_actors: Vec::new(),
        }
    }

    /// 创建一个新的 [`LiveNodeBuilder`] 以进行流式配置。
    ///
    /// # 错误
    ///
    /// 如果环境对实盘交易无效，则返回错误。
    pub fn builder(
        trader_id: TraderId,
        environment: Environment,
    ) -> anyhow::Result<LiveNodeBuilder> {
        LiveNodeBuilder::new(trader_id, environment)
    }

    /// 直接从内核名称和可选配置创建一个新的 [`LiveNode`]。
    ///
    /// 这是一个便捷方法，用于使用预配置的内核配置创建实盘节点，
    /// 绕过构建器模式。如果未提供配置，将使用默认配置。
    ///
    /// # 错误
    ///
    /// 如果内核构造失败，则返回错误。
    pub fn build(name: String, config: Option<LiveNodeConfig>) -> anyhow::Result<Self> {
        let mut config = config.unwrap_or_default();
        config.environment = Environment::Live;

        match config.environment() {
            Environment::Sandbox | Environment::Live => {}
            Environment::Backtest => {
                anyhow::bail!("LiveNode 不能用于回测 (Backtest) 环境");
            }
        }

        let runner = AsyncRunner::new();
        let kernel = NautilusKernel::new(name, config.clone())?;

        let exec_manager_config =
            ExecutionManagerConfig::from(&config.exec_engine).with_trader_id(config.trader_id);
        let exec_manager = ExecutionManager::new(
            kernel.clock.clone(),
            kernel.cache.clone(),
            exec_manager_config,
        );

        log::info!("LiveNode 使用内核配置构建成功");

        Ok(Self {
            kernel,
            runner: Some(runner),
            config,
            handle: LiveNodeHandle::new(),
            exec_manager,
            shutdown_deadline: None,
            #[cfg(feature = "python")]
            python_actors: Vec::new(),
        })
    }

    /// 返回用于控制此节点的线程安全句柄。
    #[must_use]
    pub fn handle(&self) -> LiveNodeHandle {
        self.handle.clone()
    }

    /// 启动实盘节点。
    ///
    /// # 错误
    ///
    /// 如果启动失败，则返回错误。
    pub async fn start(&mut self) -> anyhow::Result<()> {
        if self.state().is_running() {
            anyhow::bail!("已经在运行中");
        }

        self.handle.set_state(NodeState::Starting);

        self.kernel.start_async().await;
        self.kernel.connect_clients().await;

        if !self.await_engines_connected().await {
            log::error!("无法启动交易员：执行引擎客户端未连接");
            self.handle.set_state(NodeState::Running);
            return Ok(());
        }

        // 在对账和启动交易员之前处理挂起的数据事件
        if let Some(runner) = self.runner.as_mut() {
            runner.drain_pending_data_events();
        }

        self.perform_startup_reconciliation().await?;

        self.kernel.start_trader();

        self.handle.set_state(NodeState::Running);

        Ok(())
    }

    /// 停止实盘节点。
    ///
    /// 此方法会停止交易员，等待配置的宽限期以允许处理残余事件，
    /// 然后完成停机序列。
    ///
    /// # 错误
    ///
    /// 如果停机失败，则返回错误。
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if !self.state().is_running() {
            anyhow::bail!("未在运行中");
        }

        self.handle.set_state(NodeState::ShuttingDown);

        self.kernel.stop_trader();
        let delay = self.kernel.delay_post_stop();
        log::info!("正在等待残余事件 ({delay:?})...");

        tokio::time::sleep(delay).await;
        self.finalize_stop().await
    }

    /// 等待执行引擎客户端连接，并设置超时。
    ///
    /// 如果所有引擎均已连接，则返回 `true`；如果超时，则返回 `false`。
    async fn await_engines_connected(&self) -> bool {
        log::info!(
            "正在等待引擎连接（超时时间 {:?}）...",
            self.config.timeout_connection
        );

        let start = Instant::now();
        let timeout = self.config.timeout_connection;
        let interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            if self.kernel.check_engines_connected() {
                log::info!("所有引擎客户端已连接");
                return true;
            }
            tokio::time::sleep(interval).await;
        }

        self.log_connection_status();
        false
    }

    /// 等待执行引擎客户端断开连接，并设置超时。
    ///
    /// 超时时记录带有客户端状态的错误，但不会失败。
    async fn await_engines_disconnected(&self) {
        log::info!(
            "正在等待引擎断开连接（超时时间 {:?}）...",
            self.config.timeout_disconnection
        );

        let start = Instant::now();
        let timeout = self.config.timeout_disconnection;
        let interval = Duration::from_millis(100);

        while start.elapsed() < timeout {
            if self.kernel.check_engines_disconnected() {
                log::info!("所有引擎客户端已断开连接");
                return;
            }
            tokio::time::sleep(interval).await;
        }

        log::error!(
            "等待引擎断开连接超时 ({:?})\n\
             DataEngine.check_disconnected() == {}\n\
             ExecEngine.check_disconnected() == {}",
            timeout,
            self.kernel.data_engine().check_disconnected(),
            self.kernel.exec_engine().borrow().check_disconnected(),
        );
    }

    fn log_connection_status(&self) {
        #[derive(Tabled)]
        struct ClientStatus {
            #[tabled(rename = "Client")]
            client: String,
            #[tabled(rename = "Type")]
            client_type: &'static str,
            #[tabled(rename = "Connected")]
            connected: bool,
        }

        let data_status = self.kernel.data_client_connection_status();
        let exec_status = self.kernel.exec_client_connection_status();

        let mut rows: Vec<ClientStatus> = Vec::new();

        for (client_id, connected) in data_status {
            rows.push(ClientStatus {
                client: client_id.to_string(),
                client_type: "Data",
                connected,
            });
        }

        for (client_id, connected) in exec_status {
            rows.push(ClientStatus {
                client: client_id.to_string(),
                client_type: "Execution",
                connected,
            });
        }

        let table = Table::new(&rows).with(Style::rounded()).to_string();

        log::warn!(
            "等待引擎连接超时 ({:?})\n\n{table}\n\n\
             DataEngine.check_connected() == {}\n\
             ExecEngine.check_connected() == {}",
            self.config.timeout_connection,
            self.kernel.data_engine().check_connected(),
            self.kernel.exec_engine().borrow().check_connected(),
        );
    }

    /// 执行启动对账以使内部状态与交易平台状态对齐。
    ///
    /// 此方法会向每个执行客户端查询批量状态（订单、成交、持仓），
    /// 并解决与本地缓存状态之间的任何差异。
    ///
    /// # 错误
    ///
    /// 如果对账失败或超时，则返回错误。
    #[allow(clippy::await_holding_refcell_ref)] // 单线程运行时，有意设计的
    async fn perform_startup_reconciliation(&mut self) -> anyhow::Result<()> {
        if !self.config.exec_engine.reconciliation {
            log::info!("启动对账已禁用");
            return Ok(());
        }

        log_info!("正在开始执行状态对账...", color = LogColor::Blue);

        let lookback_mins = self
            .config
            .exec_engine
            .reconciliation_lookback_mins
            .map(|m| m as u64);

        let timeout = self.config.timeout_reconciliation;
        let start = Instant::now();
        let client_ids = self.kernel.exec_engine.borrow().client_ids();

        for client_id in client_ids {
            if start.elapsed() > timeout {
                log::warn!("已达到对账超时时间，提前停止");
                break;
            }

            log_info!(
                "正在从 {} 请求批量状态...",
                client_id,
                color = LogColor::Blue
            );

            let mass_status_result = self
                .kernel
                .exec_engine
                .borrow_mut()
                .generate_mass_status(&client_id, lookback_mins)
                .await;

            match mass_status_result {
                Ok(Some(mass_status)) => {
                    log_info!(
                        "正在为 {} 对账 ExecutionMassStatus",
                        client_id,
                        color = LogColor::Blue
                    );

                    // 安全提示：不要在 await 点跨越持有 Rc
                    let exec_engine_rc = self.kernel.exec_engine.clone();

                    let result = self
                        .exec_manager
                        .reconcile_execution_mass_status(mass_status, exec_engine_rc)
                        .await;

                    if result.events.is_empty() {
                        log_info!("{} 的对账已成功", client_id, color = LogColor::Blue);
                    } else {
                        log::info!(
                            color = LogColor::Blue as u8;
                            "{} 的对账已处理 {} 个事件",
                            client_id,
                            result.events.len()
                        );
                    }

                    // 在执行客户端注册外部订单以进行跟踪
                    if !result.external_orders.is_empty() {
                        let exec_engine = self.kernel.exec_engine.borrow();
                        for external in result.external_orders {
                            exec_engine.register_external_order(
                                external.client_order_id,
                                external.venue_order_id,
                                external.instrument_id,
                                external.strategy_id,
                                external.ts_init,
                            );
                        }
                    }
                }
                Ok(None) => {
                    log::warn!(
                        "来自 {client_id} 的批量状态不可用 \
                         （生成报告时可能出现适配器错误）"
                    );
                }
                Err(e) => {
                    log::warn!("无法从 {client_id} 获取批量状态：{e}");
                }
            }
        }

        self.kernel.portfolio.borrow_mut().initialize_orders();
        self.kernel.portfolio.borrow_mut().initialize_positions();

        let elapsed_secs = start.elapsed().as_secs_f64();
        log_info!(
            "启动对账已完成，共耗时 {:.2}s",
            elapsed_secs,
            color = LogColor::Blue
        );

        Ok(())
    }

    /// 运行实盘节点，并具有自动关机处理功能。
    ///
    /// 此方法会启动节点，并无限期运行，且处理中断信号以实现优雅关机。
    ///
    /// # 线程安全
    ///
    /// 事件循环直接在当前线程上运行（不衍生新线程），因为
    /// msgbus 使用了线程局部存储。内核注册的端点仅可从同一线程访问。
    ///
    /// # 关机序列
    ///
    /// 1. 收到信号（SIGINT 或通过句柄停止）。
    /// 2. 交易员组件停止（触发订单取消等）。
    /// 3. 事件循环在配置的宽限期内继续处理残余事件。
    /// 4. 内核完成停机，客户端断开连接，排空剩余事件。
    ///
    /// # 错误
    ///
    /// 如果节点启动失败或遇到运行时错误，则返回错误。
    pub async fn run(&mut self) -> anyhow::Result<()> {
        if self.state().is_running() {
            anyhow::bail!("已经在运行中");
        }

        let Some(runner) = self.runner.take() else {
            anyhow::bail!("运行器已被消耗 - run() 被调用了两次");
        };

        let AsyncRunnerChannels {
            mut time_evt_rx,
            mut data_evt_rx,
            mut data_cmd_rx,
            mut exec_evt_rx,
            mut exec_cmd_rx,
        } = runner.take_channels();

        log::info!("事件循环正在启动");

        self.handle.set_state(NodeState::Starting);
        self.kernel.start_async().await;

        let stop_handle = self.handle.clone();
        let mut pending = PendingEvents::default();

        // 启动阶段：在完成启动的同时处理事件
        // TODO: 在此处添加对 ctrl_c 和 stop_handle 的监控，以允许终止
        // 挂起的启动。目前启动期间的信号会被忽略，
        // 且任何挂起的 stop_flag 在转换为 Running 时将被清除。
        let engines_connected = {
            let startup_future = self.complete_startup();
            tokio::pin!(startup_future);

            loop {
                tokio::select! {
                    biased;

                    result = &mut startup_future => {
                        break result?;
                    }
                    Some(handler) = time_evt_rx.recv() => {
                        AsyncRunner::handle_time_event(handler);
                    }
                    Some(evt) = data_evt_rx.recv() => {
                        pending.data_evts.push(evt);
                    }
                    Some(cmd) = data_cmd_rx.recv() => {
                        pending.data_cmds.push(cmd);
                    }
                    Some(evt) = exec_evt_rx.recv() => {
                        // 账户和报告事件是安全的，订单事件会产生冲突
                        match evt {
                            ExecutionEvent::Account(_) | ExecutionEvent::Report(_) => {
                                AsyncRunner::handle_exec_event(evt);
                            }
                            ExecutionEvent::Order(order_evt) => {
                                pending.order_evts.push(order_evt);
                            }
                        }
                    }
                    Some(cmd) = exec_cmd_rx.recv() => {
                        pending.exec_cmds.push(cmd);
                    }
                }
            }
        };

        pending.drain();

        if engines_connected {
            // 既然标的已在缓存中，现在运行对账并启动交易员
            self.perform_startup_reconciliation().await?;
            self.kernel.start_trader();
        } else {
            log::error!("未启动交易员：执行引擎客户端未连接");
        }

        self.handle.set_state(NodeState::Running);

        // 运行阶段：持续运行直至停机截止时间到期
        let mut residual_events = 0usize;

        loop {
            let shutdown_deadline = self.shutdown_deadline;
            let is_shutting_down = self.state() == NodeState::ShuttingDown;

            tokio::select! {
                Some(handler) = time_evt_rx.recv() => {
                    AsyncRunner::handle_time_event(handler);
                    if is_shutting_down {
                        log::debug!("残余时间事件");
                        residual_events += 1;
                    }
                }
                Some(evt) = data_evt_rx.recv() => {
                    if is_shutting_down {
                        log::debug!("残余数据事件：{evt:?}");
                        residual_events += 1;
                    }
                    AsyncRunner::handle_data_event(evt);
                }
                Some(cmd) = data_cmd_rx.recv() => {
                    if is_shutting_down {
                        log::debug!("残余数据命令：{cmd:?}");
                        residual_events += 1;
                    }
                    AsyncRunner::handle_data_command(cmd);
                }
                Some(evt) = exec_evt_rx.recv() => {
                    if is_shutting_down {
                        log::debug!("残余执行事件：{evt:?}");
                        residual_events += 1;
                    }
                    AsyncRunner::handle_exec_event(evt);
                }
                Some(cmd) = exec_cmd_rx.recv() => {
                    if is_shutting_down {
                        log::debug!("残余执行命令：{cmd:?}");
                        residual_events += 1;
                    }
                    AsyncRunner::handle_exec_command(cmd);
                }
                result = tokio::signal::ctrl_c(), if self.state() == NodeState::Running => {
                    match result {
                        Ok(()) => log::info!("收到 SIGINT，正在关机"),
                        Err(e) => log::error!("监听 SIGINT 失败：{e}"),
                    }
                    self.initiate_shutdown();
                }
                () = async {
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        if stop_handle.should_stop() {
                            log::info!("收到来自句柄的停止信号");
                            return;
                        }
                    }
                }, if self.state() == NodeState::Running => {
                    self.initiate_shutdown();
                }
                () = async {
                    match shutdown_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                }, if self.state() == NodeState::ShuttingDown => {
                    break;
                }
            }
        }

        if residual_events > 0 {
            log::debug!("关机期间处理了 {residual_events} 个残余事件");
        }

        let _ = self.kernel.cache().borrow().check_residuals();

        self.finalize_stop().await?;

        // 处理在 finalize_stop 期间到达的事件
        self.drain_channels(
            &mut time_evt_rx,
            &mut data_evt_rx,
            &mut data_cmd_rx,
            &mut exec_evt_rx,
            &mut exec_cmd_rx,
        );

        log::info!("事件循环已停止");

        Ok(())
    }

    /// 如果所有引擎连接成功，则返回 `true`，否则返回 `false`。
    /// 注意：此方法不会运行对账 - 对账会在排空挂起事件后发生。
    async fn complete_startup(&mut self) -> anyhow::Result<bool> {
        self.kernel.connect_clients().await;

        if !self.await_engines_connected().await {
            return Ok(false);
        }

        Ok(true)
    }

    fn initiate_shutdown(&mut self) {
        self.kernel.stop_trader();
        let delay = self.kernel.delay_post_stop();
        log::info!("正在等待残余事件 ({delay:?})...");

        self.shutdown_deadline = Some(tokio::time::Instant::now() + delay);
        self.handle.set_state(NodeState::ShuttingDown);
    }

    async fn finalize_stop(&mut self) -> anyhow::Result<()> {
        self.kernel.disconnect_clients().await?;
        self.await_engines_disconnected().await;
        self.kernel.finalize_stop().await;

        self.handle.set_state(NodeState::Stopped);

        Ok(())
    }

    fn drain_channels(
        &self,
        time_evt_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TimeEventHandler>,
        data_evt_rx: &mut tokio::sync::mpsc::UnboundedReceiver<DataEvent>,
        data_cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<DataCommand>,
        exec_evt_rx: &mut tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
        exec_cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TradingCommand>,
    ) {
        let mut drained = 0;

        while let Ok(handler) = time_evt_rx.try_recv() {
            AsyncRunner::handle_time_event(handler);
            drained += 1;
        }
        while let Ok(cmd) = data_cmd_rx.try_recv() {
            AsyncRunner::handle_data_command(cmd);
            drained += 1;
        }
        while let Ok(evt) = data_evt_rx.try_recv() {
            AsyncRunner::handle_data_event(evt);
            drained += 1;
        }
        while let Ok(cmd) = exec_cmd_rx.try_recv() {
            AsyncRunner::handle_exec_command(cmd);
            drained += 1;
        }
        while let Ok(evt) = exec_evt_rx.try_recv() {
            AsyncRunner::handle_exec_event(evt);
            drained += 1;
        }

        if drained > 0 {
            log::info!("关机期间排空了 {drained} 个剩余事件");
        }
    }

    /// 获取节点的运行环境。
    #[must_use]
    pub fn environment(&self) -> Environment {
        self.kernel.environment()
    }

    /// 获取对底层内核的引用。
    #[must_use]
    pub const fn kernel(&self) -> &NautilusKernel {
        &self.kernel
    }

    /// 获取对底层内核的独占引用。
    #[must_use]
    pub const fn kernel_mut(&mut self) -> &mut NautilusKernel {
        &mut self.kernel
    }

    /// 获取节点的交易员 ID。
    #[must_use]
    pub fn trader_id(&self) -> TraderId {
        self.kernel.trader_id()
    }

    /// 获取节点的实例 ID。
    #[must_use]
    pub const fn instance_id(&self) -> UUID4 {
        self.kernel.instance_id()
    }

    /// 返回当前节点状态。
    #[must_use]
    pub fn state(&self) -> NodeState {
        self.handle.state()
    }

    /// 检查实盘节点当前是否正在运行。
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state().is_running()
    }

    /// 设置用于持久化的缓存数据库适配器。
    ///
    /// 这允许在节点构建后但在开始运行前设置数据库适配器（例如 PostgreSQL、Redis）。
    /// 数据库适配器用于持久化缓存数据，以进行恢复和状态管理。
    ///
    /// # 错误
    ///
    /// 如果节点已经在运行，则返回错误。
    pub fn set_cache_database(
        &mut self,
        database: Box<dyn CacheDatabaseAdapter>,
    ) -> anyhow::Result<()> {
        if self.state() != NodeState::Idle {
            anyhow::bail!("无法在节点运行时设置缓存数据库，请在调用 start() 之前进行设置");
        }

        self.kernel.cache().borrow_mut().set_database(database);
        Ok(())
    }

    /// 获取对执行管理器的引用。
    #[must_use]
    pub const fn exec_manager(&self) -> &ExecutionManager {
        &self.exec_manager
    }

    /// 获取对执行管理器的独占引用。
    #[must_use]
    pub fn exec_manager_mut(&mut self) -> &mut ExecutionManager {
        &mut self.exec_manager
    }

    /// 向交易员添加一个参与者 (actor)。
    ///
    /// 此方法提供了一个高层接口，用于向底层交易员添加参与者，
    /// 而无需直接访问内核。参与者应在节点构建后但在启动节点前添加。
    ///
    /// # 错误
    ///
    /// 如果满足以下条件，则返回错误：
    /// - 交易员不处于添加组件的有效状态。
    /// - 具有相同 ID 的参与者已注册。
    /// - 节点当前正在运行。
    pub fn add_actor<T>(&mut self, actor: T) -> anyhow::Result<()>
    where
        T: DataActor + Component + Actor + 'static,
    {
        if self.state() != NodeState::Idle {
            anyhow::bail!("无法在节点运行时添加参与者，请在调用 start() 之前添加参与者");
        }

        self.kernel.trader.add_actor(actor)
    }

    /// 使用工厂函数向实盘节点添加参与者。
    ///
    /// 工厂函数在注册时被调用以创建参与者，
    /// 从而避免不可克隆参与者类型的克隆问题。
    ///
    /// # 错误
    ///
    /// 如果满足以下条件，则返回错误：
    /// - 节点当前正在运行。
    /// - 工厂函数创建参与者失败。
    /// - 底层交易员注册失败。
    pub fn add_actor_from_factory<F, T>(&mut self, factory: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> anyhow::Result<T>,
        T: DataActor + Component + Actor + 'static,
    {
        if self.state() != NodeState::Idle {
            anyhow::bail!("无法在节点运行时添加参与者，请在调用 start() 之前添加参与者");
        }

        self.kernel.trader.add_actor_from_factory(factory)
    }

    /// 向交易员添加策略。
    ///
    /// 策略会同时在组件注册表（用于生命周期管理）
    /// 和参与者注册表（用于通过 msgbus 进行数据回调）中注册。
    ///
    /// # 错误
    ///
    /// 如果满足以下条件，则返回错误：
    /// - 节点当前正在运行。
    /// - 具有相同 ID 的策略已注册。
    pub fn add_strategy<T>(&mut self, strategy: T) -> anyhow::Result<()>
    where
        T: Strategy + Component + Debug + 'static,
    {
        if self.state() != NodeState::Idle {
            anyhow::bail!("无法在节点运行时添加策略，请在调用 start() 之前添加策略");
        }

        // 在添加策略（这会移动策略）之前注册外部订单申领
        let strategy_id = StrategyId::from(strategy.component_id().inner().as_str());
        if let Some(claims) = strategy.external_order_claims() {
            for instrument_id in claims {
                self.exec_manager
                    .claim_external_orders(instrument_id, strategy_id);
            }
            log_info!(
                "已为 {} 注册外部订单申领：{:?}",
                strategy_id,
                strategy.external_order_claims(),
                color = LogColor::Blue
            );
        }

        self.kernel.trader.add_strategy(strategy)
    }
}

/// 启动期间排队的事件，以避免 RefCell 借用冲突。
///
/// 在 `connect_clients()` 期间，data_engine 和 exec_engine 会跨越 await 被借用。
/// 处理触发 msgbus 处理程序的命令/事件会尝试借用相同的引擎，从而导致恐慌。
#[derive(Default)]
struct PendingEvents {
    data_cmds: Vec<DataCommand>,
    data_evts: Vec<DataEvent>,
    exec_cmds: Vec<TradingCommand>,
    order_evts: Vec<OrderEventAny>,
}

impl PendingEvents {
    fn drain(&mut self) {
        let total = self.data_evts.len()
            + self.data_cmds.len()
            + self.exec_cmds.len()
            + self.order_evts.len();

        if total > 0 {
            log::debug!(
                "正在处理启动期间排队的 {total} 个事件/命令 \
                 （data_evts={}，data_cmds={}，exec_cmds={}，order_evts={}）",
                self.data_evts.len(),
                self.data_cmds.len(),
                self.exec_cmds.len(),
                self.order_evts.len()
            );
        }

        for evt in self.data_evts.drain(..) {
            AsyncRunner::handle_data_event(evt);
        }
        for cmd in self.data_cmds.drain(..) {
            AsyncRunner::handle_data_command(cmd);
        }
        for cmd in self.exec_cmds.drain(..) {
            AsyncRunner::handle_exec_command(cmd);
        }
        for evt in self.order_evts.drain(..) {
            AsyncRunner::handle_exec_event(ExecutionEvent::Order(evt));
        }
    }
}

#[cfg(test)]
mod tests {
    use nautilus_model::identifiers::TraderId;
    use rstest::*;

    use super::*;

    #[rstest]
    #[case(0, NodeState::Idle)]
    #[case(1, NodeState::Starting)]
    #[case(2, NodeState::Running)]
    #[case(3, NodeState::ShuttingDown)]
    #[case(4, NodeState::Stopped)]
    fn test_node_state_from_u8_valid(#[case] value: u8, #[case] expected: NodeState) {
        assert_eq!(NodeState::from_u8(value), expected);
    }

    #[rstest]
    #[case(5)]
    #[case(255)]
    #[should_panic(expected = "Invalid NodeState value")]
    fn test_node_state_from_u8_invalid_panics(#[case] value: u8) {
        let _ = NodeState::from_u8(value);
    }

    #[rstest]
    fn test_node_state_roundtrip() {
        for state in [
            NodeState::Idle,
            NodeState::Starting,
            NodeState::Running,
            NodeState::ShuttingDown,
            NodeState::Stopped,
        ] {
            assert_eq!(NodeState::from_u8(state.as_u8()), state);
        }
    }

    #[rstest]
    fn test_node_state_is_running_only_for_running() {
        assert!(!NodeState::Idle.is_running());
        assert!(!NodeState::Starting.is_running());
        assert!(NodeState::Running.is_running());
        assert!(!NodeState::ShuttingDown.is_running());
        assert!(!NodeState::Stopped.is_running());
    }

    #[rstest]
    fn test_handle_initial_state() {
        let handle = LiveNodeHandle::new();

        assert_eq!(handle.state(), NodeState::Idle);
        assert!(!handle.should_stop());
        assert!(!handle.is_running());
    }

    #[rstest]
    fn test_handle_stop_sets_flag() {
        let handle = LiveNodeHandle::new();

        handle.stop();

        assert!(handle.should_stop());
    }

    #[rstest]
    fn test_handle_set_state_running_clears_stop_flag() {
        let handle = LiveNodeHandle::new();
        handle.stop();
        assert!(handle.should_stop());

        handle.set_state(NodeState::Running);

        assert!(!handle.should_stop());
        assert!(handle.is_running());
        assert_eq!(handle.state(), NodeState::Running);
    }

    #[rstest]
    fn test_handle_node_state_transitions() {
        let handle = LiveNodeHandle::new();
        assert_eq!(handle.state(), NodeState::Idle);

        handle.set_state(NodeState::Starting);
        assert_eq!(handle.state(), NodeState::Starting);
        assert!(!handle.is_running());

        handle.set_state(NodeState::Running);
        assert_eq!(handle.state(), NodeState::Running);
        assert!(handle.is_running());

        handle.set_state(NodeState::ShuttingDown);
        assert_eq!(handle.state(), NodeState::ShuttingDown);
        assert!(!handle.is_running());

        handle.set_state(NodeState::Stopped);
        assert_eq!(handle.state(), NodeState::Stopped);
        assert!(!handle.is_running());
    }

    #[rstest]
    fn test_handle_clone_shares_state_bidirectionally() {
        let handle1 = LiveNodeHandle::new();
        let handle2 = handle1.clone();

        // Mutation from handle1 visible in handle2
        handle1.stop();
        assert!(handle2.should_stop());

        // Mutation from handle2 visible in handle1
        handle2.set_state(NodeState::Running);
        assert_eq!(handle1.state(), NodeState::Running);
    }

    #[rstest]
    fn test_handle_stop_flag_independent_of_state() {
        let handle = LiveNodeHandle::new();

        // Stop flag can be set regardless of state
        handle.set_state(NodeState::Starting);
        handle.stop();
        assert!(handle.should_stop());
        assert_eq!(handle.state(), NodeState::Starting);

        // Only Running state clears the stop flag
        handle.set_state(NodeState::ShuttingDown);
        assert!(handle.should_stop()); // Still set

        handle.set_state(NodeState::Running);
        assert!(!handle.should_stop()); // Cleared
    }

    #[rstest]
    fn test_builder_creation() {
        let result = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Sandbox);

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_builder_rejects_backtest() {
        let result = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Backtest);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Backtest"));
    }

    #[rstest]
    fn test_builder_accepts_live_environment() {
        let result = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Live);

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_builder_accepts_sandbox_environment() {
        let result = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Sandbox);

        assert!(result.is_ok());
    }

    #[rstest]
    fn test_builder_fluent_api_chaining() {
        let builder = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Live)
            .unwrap()
            .with_name("TestNode")
            .with_instance_id(UUID4::new())
            .with_load_state(false)
            .with_save_state(true)
            .with_timeout_connection(30)
            .with_timeout_reconciliation(60)
            .with_reconciliation(true)
            .with_reconciliation_lookback_mins(120)
            .with_timeout_portfolio(10)
            .with_timeout_disconnection_secs(5)
            .with_delay_post_stop_secs(3)
            .with_delay_shutdown_secs(10);

        assert_eq!(builder.name(), "TestNode");
    }

    #[cfg(feature = "python")]
    #[rstest]
    fn test_node_build_and_initial_state() {
        let node = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Sandbox)
            .unwrap()
            .with_name("TestNode")
            .build()
            .unwrap();

        assert_eq!(node.state(), NodeState::Idle);
        assert!(!node.is_running());
        assert_eq!(node.environment(), Environment::Sandbox);
        assert_eq!(node.trader_id(), TraderId::from("TRADER-001"));
    }

    #[cfg(feature = "python")]
    #[rstest]
    fn test_node_handle_reflects_node_state() {
        let node = LiveNode::builder(TraderId::from("TRADER-001"), Environment::Sandbox)
            .unwrap()
            .with_name("TestNode")
            .build()
            .unwrap();

        let handle = node.handle();

        assert_eq!(handle.state(), NodeState::Idle);
        assert!(!handle.is_running());
    }
}
