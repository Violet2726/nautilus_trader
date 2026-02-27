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

//! 实盘交易的执行状态管理器。
//!
//! 此模块提供了执行管理器，用于协调本地缓存与连接的交易平台之间的执行状态，
//! 以及在实盘交易期间清除旧状态。

use std::{cell::RefCell, fmt::Debug, rc::Rc, str::FromStr, sync::LazyLock};

use ahash::{AHashMap, AHashSet};
use indexmap::IndexMap;
use nautilus_common::{
    cache::Cache,
    clients::ExecutionClient,
    clock::Clock,
    enums::{LogColor, LogLevel},
    log_info,
    messages::execution::report::{GenerateOrderStatusReports, GeneratePositionStatusReports},
};
use nautilus_core::{
    UUID4, UnixNanos,
    datetime::{NANOSECONDS_IN_MILLISECOND, NANOSECONDS_IN_SECOND, nanos_to_millis},
};
use nautilus_execution::{
    engine::ExecutionEngine,
    reconciliation::{
        calculate_reconciliation_price, create_inferred_fill_for_qty,
        create_reconciliation_rejected, create_reconciliation_triggered,
        create_synthetic_venue_order_id, generate_external_order_status_events,
        process_mass_status_for_reconciliation, reconcile_order_report,
        should_reconciliation_update,
    },
};
use nautilus_model::{
    enums::{OrderSide, OrderStatus, OrderType, TimeInForce},
    events::{OrderEventAny, OrderFilled, OrderInitialized},
    identifiers::{
        AccountId, ClientOrderId, InstrumentId, PositionId, StrategyId, TradeId, TraderId,
        VenueOrderId,
    },
    instruments::{Instrument, InstrumentAny},
    orders::{Order, OrderAny},
    position::Position,
    reports::{ExecutionMassStatus, FillReport, OrderStatusReport, PositionStatusReport},
    types::Quantity,
};
use rust_decimal::{Decimal, prelude::ToPrimitive};
use ustr::Ustr;

use crate::config::LiveExecEngineConfig;

/// 来自交易平台订单的标签（外部订单）。
static TAG_VENUE: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("VENUE"));

/// 由对账逻辑生成的订单标签（合成订单）。
static TAG_RECONCILIATION: LazyLock<Ustr> = LazyLock::new(|| Ustr::from("RECONCILIATION"));

/// 需要在执行客户端注册的外部订单元数据。
#[derive(Debug, Clone)]
pub struct ExternalOrderMetadata {
    pub client_order_id: ClientOrderId,
    pub venue_order_id: VenueOrderId,
    pub instrument_id: InstrumentId,
    pub strategy_id: StrategyId,
    pub ts_init: UnixNanos,
}

/// 包含事件和外部订单元数据的对账结果。
#[derive(Debug, Default)]
pub struct ReconciliationResult {
    /// 对账期间生成的订单事件。
    pub events: Vec<OrderEventAny>,
    /// 需要在执行客户端注册的外部订单。
    pub external_orders: Vec<ExternalOrderMetadata>,
}

/// 执行管理器的配置。
#[derive(Debug, Clone)]
pub struct ExecutionManagerConfig {
    /// 生成订单的交易员 ID。
    pub trader_id: TraderId,
    /// 启动时是否激活对账。
    pub reconciliation: bool,
    /// 启动时开始对账前的延迟（秒）。
    pub reconciliation_startup_delay_secs: f64,
    /// 对账期间的回顾分钟数。
    pub lookback_mins: Option<u64>,
    /// 对账包含的标的 ID（为空表示全选）。
    pub reconciliation_instrument_ids: AHashSet<InstrumentId>,
    /// 是否过滤未申领的外部订单。
    pub filter_unclaimed_external: bool,
    /// 对账期间是否过滤持仓状态报告。
    pub filter_position_reports: bool,
    /// 排除在对账之外的客户订单 ID。
    pub filtered_client_order_ids: AHashSet<ClientOrderId>,
    /// 是否根据报告生成缺失的订单。
    pub generate_missing_orders: bool,
    /// 检查在途订单是否超过其阈值的间隔（毫秒）。
    pub inflight_check_interval_ms: u32,
    /// 在途订单检查的阈值（毫秒）。
    pub inflight_threshold_ms: u64,
    /// 在途检查的最大重试次数。
    pub inflight_max_retries: u32,
    /// 检查交易平台开仓订单的间隔（秒）。
    pub open_check_interval_secs: Option<f64>,
    /// 开仓订单检查的回顾分钟数。
    pub open_check_lookback_mins: Option<u64>,
    /// 针对开仓订单，在处理交易平台差异前的阈值（纳秒）。
    pub open_check_threshold_ns: u64,
    /// 解决交易平台缺失开仓订单前的最大重试次数。
    pub open_check_missing_retries: u32,
    /// 开仓订单轮询是否应仅从交易平台请求开仓订单。
    pub open_check_open_only: bool,
    /// 每个一致性检查周期中单笔订单查询的最大数量。
    pub max_single_order_queries_per_cycle: u32,
    /// 连续单笔订单查询之间的延迟（毫秒）。
    pub single_order_query_delay_ms: u32,
    /// 检查交易平台持仓的间隔（秒）。
    pub position_check_interval_secs: Option<f64>,
    /// 持仓一致性检查的回顾分钟数。
    pub position_check_lookback_mins: u64,
    /// 针对持仓，在处理交易平台差异前的阈值（纳秒）。
    pub position_check_threshold_ns: u64,
    /// 已关闭订单可被清除前的时间缓冲（分钟）。
    pub purge_closed_orders_buffer_mins: Option<u32>,
    /// 已关闭持仓可被清除前的时间缓冲（分钟）。
    pub purge_closed_positions_buffer_mins: Option<u32>,
    /// 账户事件可被清除前的回顾时间缓冲（分钟）。
    pub purge_account_events_lookback_mins: Option<u32>,
    /// 清除操作是否也应从后端数据库中删除。
    pub purge_from_database: bool,
}

impl Default for ExecutionManagerConfig {
    fn default() -> Self {
        Self {
            trader_id: TraderId::default(),
            reconciliation: true,
            reconciliation_startup_delay_secs: 10.0,
            lookback_mins: Some(60),
            reconciliation_instrument_ids: AHashSet::new(),
            filter_unclaimed_external: false,
            filter_position_reports: false,
            filtered_client_order_ids: AHashSet::new(),
            generate_missing_orders: true,
            inflight_check_interval_ms: 2_000,
            inflight_threshold_ms: 5_000,
            inflight_max_retries: 5,
            open_check_interval_secs: None,
            open_check_lookback_mins: Some(60),
            open_check_threshold_ns: 5_000_000_000,
            open_check_missing_retries: 5,
            open_check_open_only: true,
            max_single_order_queries_per_cycle: 5,
            single_order_query_delay_ms: 100,
            position_check_interval_secs: None,
            position_check_lookback_mins: 60,
            position_check_threshold_ns: 60_000_000_000,
            purge_closed_orders_buffer_mins: None,
            purge_closed_positions_buffer_mins: None,
            purge_account_events_lookback_mins: None,
            purge_from_database: false,
        }
    }
}

impl From<&LiveExecEngineConfig> for ExecutionManagerConfig {
    fn from(config: &LiveExecEngineConfig) -> Self {
        let filtered_client_order_ids: AHashSet<ClientOrderId> = config
            .filtered_client_order_ids
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|value| ClientOrderId::from(value.as_str()))
            .collect();

        let reconciliation_instrument_ids: AHashSet<InstrumentId> = config
            .reconciliation_instrument_ids
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(InstrumentId::from)
            .collect();

        let open_check_threshold_ns =
            (config.open_check_threshold_ms as u64) * NANOSECONDS_IN_MILLISECOND;
        let position_check_threshold_ns =
            (config.position_check_threshold_ms as u64) * NANOSECONDS_IN_MILLISECOND;

        Self {
            trader_id: TraderId::default(), // Must be set separately via with_trader_id
            reconciliation: config.reconciliation,
            reconciliation_startup_delay_secs: config.reconciliation_startup_delay_secs,
            lookback_mins: config.reconciliation_lookback_mins.map(|m| m as u64),
            reconciliation_instrument_ids,
            filter_unclaimed_external: config.filter_unclaimed_external_orders,
            filter_position_reports: config.filter_position_reports,
            filtered_client_order_ids,
            generate_missing_orders: config.generate_missing_orders,
            inflight_check_interval_ms: config.inflight_check_interval_ms,
            inflight_threshold_ms: config.inflight_check_threshold_ms as u64,
            inflight_max_retries: config.inflight_check_retries,
            open_check_interval_secs: config.open_check_interval_secs,
            open_check_lookback_mins: config.open_check_lookback_mins.map(|m| m as u64),
            open_check_threshold_ns,
            open_check_missing_retries: config.open_check_missing_retries,
            open_check_open_only: config.open_check_open_only,
            max_single_order_queries_per_cycle: config.max_single_order_queries_per_cycle,
            single_order_query_delay_ms: config.single_order_query_delay_ms,
            position_check_interval_secs: config.position_check_interval_secs,
            position_check_lookback_mins: config.position_check_lookback_mins as u64,
            position_check_threshold_ns,
            purge_closed_orders_buffer_mins: config.purge_closed_orders_buffer_mins,
            purge_closed_positions_buffer_mins: config.purge_closed_positions_buffer_mins,
            purge_account_events_lookback_mins: config.purge_account_events_lookback_mins,
            purge_from_database: config.purge_from_database,
        }
    }
}

impl ExecutionManagerConfig {
    /// 在配置上设置交易员 ID。
    #[must_use]
    pub fn with_trader_id(mut self, trader_id: TraderId) -> Self {
        self.trader_id = trader_id;
        self
    }
}

/// 用于持续对账的执行报告。
/// 这是运行时对账期间使用的简化报告类型。
#[derive(Debug, Clone)]
pub struct ExecutionReport {
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub status: OrderStatus,
    pub filled_qty: Quantity,
    pub avg_px: Option<f64>,
    pub ts_event: UnixNanos,
}

/// 关于在途订单检查的信息。
#[derive(Debug, Clone)]
struct InflightCheck {
    #[allow(dead_code)]
    pub client_order_id: ClientOrderId,
    /// 提交时间戳。
    pub ts_submitted: UnixNanos,
    /// 重试计数。
    pub retry_count: u32,
    /// 上次查询时间戳。
    pub last_query_ts: Option<UnixNanos>,
}

/// 执行状态管理器。
///
/// `ExecutionManager` 负责：
/// - 启动对账，以便在系统启动时对齐状态。
/// - 持续对账在途订单。
/// - 外部订单发现和申领。
/// - 成交报告 (Fill report) 处理和验证。
/// - 清除旧订单、持仓和账户事件。
///
/// # 线程安全
///
/// 此结构体 **不是线程安全的**，专为在异步运行时内的单线程使用而设计。
/// 内部状态使用 `AHashMap` 管理且未进行同步，`clock` 和 `cache` 使用 `Rc<RefCell<>>`，
/// 这提供了运行时借用检查，但不提供线程安全保证。
///
/// 如果需要并发访问，此结构体必须封装在 `Arc<Mutex<>>` 或类似的同步原语中。
/// 或者，确保所有方法都从异步运行时中的同一个线程/任务调用。
///
/// **警告：** 并发修改内部 AHashMap 或并发借用 `RefCell` 内容会导致运行时 panic。
#[derive(Clone)]
pub struct ExecutionManager {
    clock: Rc<RefCell<dyn Clock>>,
    cache: Rc<RefCell<Cache>>,
    config: ExecutionManagerConfig,
    inflight_checks: AHashMap<ClientOrderId, InflightCheck>,
    external_order_claims: AHashMap<InstrumentId, StrategyId>,
    processed_fills: AHashMap<TradeId, ClientOrderId>,
    recon_check_retries: AHashMap<ClientOrderId, u32>,
    ts_last_query: AHashMap<ClientOrderId, UnixNanos>,
    order_local_activity_ns: AHashMap<ClientOrderId, UnixNanos>,
    position_local_activity_ns: AHashMap<InstrumentId, UnixNanos>,
    recent_fills_cache: AHashMap<TradeId, UnixNanos>,
}

impl Debug for ExecutionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(stringify!(ExecutionManager))
            .field("config", &self.config)
            .field("inflight_checks", &self.inflight_checks)
            .field("external_order_claims", &self.external_order_claims)
            .field("processed_fills", &self.processed_fills)
            .field("recon_check_retries", &self.recon_check_retries)
            .finish()
    }
}

impl ExecutionManager {
    /// 创建一个新的 [`ExecutionManager`] 实例。
    pub fn new(
        clock: Rc<RefCell<dyn Clock>>,
        cache: Rc<RefCell<Cache>>,
        config: ExecutionManagerConfig,
    ) -> Self {
        Self {
            clock,
            cache,
            config,
            inflight_checks: AHashMap::new(),
            external_order_claims: AHashMap::new(),
            processed_fills: AHashMap::new(),
            recon_check_retries: AHashMap::new(),
            ts_last_query: AHashMap::new(),
            order_local_activity_ns: AHashMap::new(),
            position_local_activity_ns: AHashMap::new(),
            recent_fills_cache: AHashMap::new(),
        }
    }

    /// 对齐批量状态报告中的订单和成交。
    ///
    /// 订单事件被收集并按 ts_event 进行全局排序，然后通过执行引擎进行处理，以确保所有订单的时序正确。
    /// 持仓事件在所有订单事件之后处理，以确保成交已率先应用。
    pub async fn reconcile_execution_mass_status(
        &mut self,
        mass_status: ExecutionMassStatus,
        exec_engine: Rc<RefCell<ExecutionEngine>>,
    ) -> ReconciliationResult {
        let venue = mass_status.venue;
        let order_count = mass_status.order_reports().len();
        let fill_count: usize = mass_status.fill_reports().values().map(|v| v.len()).sum();
        let position_count = mass_status.position_reports().len();

        log_info!(
            "Reconciling ExecutionMassStatus for {venue}",
            color = LogColor::Blue
        );
        log_info!(
            "Received {order_count} order(s), {fill_count} fill(s), {position_count} position(s)",
            color = LogColor::Blue
        );

        let (adjusted_order_reports, adjusted_fill_reports) =
            self.adjust_mass_status_fills(&mass_status);

        let mut events = Vec::new();
        let mut external_orders = Vec::new();
        let mut orders_reconciled = 0usize;
        let mut external_orders_created = 0usize;
        let mut open_orders_initialized = 0usize;
        let mut orders_skipped_no_instrument = 0usize;
        let mut orders_skipped_duplicate = 0usize;
        let mut fills_applied = 0usize;

        let fill_reports = &adjusted_fill_reports;
        let mut seen_trade_ids: AHashSet<TradeId> = AHashSet::new();

        for fills in fill_reports.values() {
            for fill in fills {
                if !seen_trade_ids.insert(fill.trade_id) {
                    log::warn!("Duplicate trade_id {} in mass status", fill.trade_id);
                }
            }
        }

        // 按 venue_order_id 对报告进行去重，保留最先进的状态。
        let order_reports = self.deduplicate_order_reports(adjusted_order_reports.values());
        let mut orders_skipped_filtered = 0usize;

        for report in order_reports.values() {
            if self.should_skip_order_report(report) {
                orders_skipped_filtered += 1;
                continue;
            }

            if let Some(client_order_id) = &report.client_order_id {
                if let Some(cached_order) = self.get_order(client_order_id)
                    && self.is_exact_order_match(&cached_order, report)
                {
                    log::debug!("Skipping order {client_order_id}: already in sync with venue");
                    orders_skipped_duplicate += 1;

                    // 即使跳过，也要确保 venue_order_id 已编入索引
                    if let Err(e) = self.cache.borrow_mut().add_venue_order_id(
                        client_order_id,
                        &report.venue_order_id,
                        false,
                    ) {
                        log::warn!("Failed to add venue order ID index: {e}");
                    }

                    continue;
                }

                // 跳过已关闭的对账订单，以防止重启时出现重复的推断成交。
                if let Some(cached_order) = self.get_order(client_order_id)
                    && cached_order.is_closed()
                    && cached_order
                        .tags()
                        .is_some_and(|tags| tags.contains(&*TAG_RECONCILIATION))
                {
                    log::debug!(
                        "跳过已关闭的对账订单 {client_order_id}：\
                         来自上一个会话的合成持仓调整",
                    );
                    orders_skipped_duplicate += 1;
                    continue;
                }

                if let Some(mut order) = self.get_order(client_order_id) {
                    let instrument = self.get_instrument(&report.instrument_id);
                    log::info!(
                        color = LogColor::Blue as u8;
                        "Reconciling {} {} {} [{}] -> [{}]",
                        client_order_id,
                        report.venue_order_id,
                        report.instrument_id,
                        order.status(),
                        report.order_status,
                    );

                    // 即使在没有缓存订单的情况下，也要处理成交（可能有孤立成交）
                    let order_fills: Vec<&FillReport> = fill_reports
                        .get(&report.venue_order_id)
                        .map(|f| f.iter().collect())
                        .unwrap_or_default();

                    let order_events = self.reconcile_order_with_fills(
                        &mut order,
                        report,
                        &order_fills,
                        instrument.as_ref(),
                    );
                    if !order_events.is_empty() {
                        orders_reconciled += 1;
                        fills_applied += order_events
                            .iter()
                            .filter(|e| matches!(e, OrderEventAny::Filled(_)))
                            .count();
                        events.extend(order_events);
                    }

                    // 始终确保在对账后对 venue_order_id 进行索引。
                    if let Err(e) = self.cache.borrow_mut().add_venue_order_id(
                        client_order_id,
                        &report.venue_order_id,
                        false,
                    ) {
                        log::warn!("Failed to add venue order ID index: {e}");
                    }
                } else if let Some(mut order) =
                    self.get_order_by_venue_order_id(&report.venue_order_id)
                {
                    // 备选方案：通过 venue_order_id 进行匹配。
                    let instrument = self.get_instrument(&report.instrument_id);

                    log::info!(
                        color = LogColor::Blue as u8;
                        "Reconciling {} (matched by venue_order_id {}) {} [{}] -> [{}]",
                        order.client_order_id(),
                        report.venue_order_id,
                        report.instrument_id,
                        order.status(),
                        report.order_status,
                    );

                    let order_fills: Vec<&FillReport> = fill_reports
                        .get(&report.venue_order_id)
                        .map(|f| f.iter().collect())
                        .unwrap_or_default();
                    let order_events = self.reconcile_order_with_fills(
                        &mut order,
                        report,
                        &order_fills,
                        instrument.as_ref(),
                    );

                    if !order_events.is_empty() {
                        orders_reconciled += 1;
                        fills_applied += order_events
                            .iter()
                            .filter(|e| matches!(e, OrderEventAny::Filled(_)))
                            .count();
                        events.extend(order_events);
                    }

                    if let Err(e) = self.cache.borrow_mut().add_venue_order_id(
                        &order.client_order_id(),
                        &report.venue_order_id,
                        false,
                    ) {
                        log::warn!("Failed to add venue order ID index: {e}");
                    }
                } else if !self.config.filter_unclaimed_external {
                    if let Some(instrument) = self.get_instrument(&report.instrument_id) {
                        let order_fills: Vec<&FillReport> = fill_reports
                            .get(&report.venue_order_id)
                            .map(|f| f.iter().collect())
                            .unwrap_or_default();
                        let (external_events, metadata) = self.handle_external_order(
                            report,
                            &mass_status.account_id,
                            &instrument,
                            &order_fills,
                            false, // 非合成订单（交易所订单）
                        );

                        if !external_events.is_empty() {
                            external_orders_created += 1;
                            fills_applied += external_events
                                .iter()
                                .filter(|e| matches!(e, OrderEventAny::Filled(_)))
                                .count();

                            if report.order_status.is_open() {
                                open_orders_initialized += 1;
                            }

                            events.extend(external_events);

                            if let Some(m) = metadata {
                                external_orders.push(m);
                            }
                        }
                    } else {
                        orders_skipped_no_instrument += 1;
                    }
                }
            } else if let Some(mut order) = self.get_order_by_venue_order_id(&report.venue_order_id)
            {
                // 备选方案：通过 venue_order_id 进行匹配。
                let instrument = self.get_instrument(&report.instrument_id);
                log::info!(
                    color = LogColor::Blue as u8;
                    "Reconciling {} (matched by venue_order_id {}) {} [{}] -> [{}]",
                    order.client_order_id(),
                    report.venue_order_id,
                    report.instrument_id,
                    order.status(),
                    report.order_status,
                );

                let order_fills: Vec<&FillReport> = fill_reports
                    .get(&report.venue_order_id)
                    .map(|f| f.iter().collect())
                    .unwrap_or_default();
                let order_events = self.reconcile_order_with_fills(
                    &mut order,
                    report,
                    &order_fills,
                    instrument.as_ref(),
                );

                if !order_events.is_empty() {
                    orders_reconciled += 1;
                    fills_applied += order_events
                        .iter()
                        .filter(|e| matches!(e, OrderEventAny::Filled(_)))
                        .count();
                    events.extend(order_events);
                }

                if let Err(e) = self.cache.borrow_mut().add_venue_order_id(
                    &order.client_order_id(),
                    &report.venue_order_id,
                    false,
                ) {
                    log::warn!("Failed to add venue order ID index: {e}");
                }
            } else if let Some(instrument) = self.get_instrument(&report.instrument_id) {
                // 合成订单（S- 前缀）由对账逻辑生成。
                let is_synthetic = report.venue_order_id.as_str().starts_with("S-");

                let order_fills: Vec<&FillReport> = fill_reports
                    .get(&report.venue_order_id)
                    .map(|f| f.iter().collect())
                    .unwrap_or_default();
                let (external_events, metadata) = self.handle_external_order(
                    report,
                    &mass_status.account_id,
                    &instrument,
                    &order_fills,
                    is_synthetic,
                );

                if !external_events.is_empty() {
                    external_orders_created += 1;
                    fills_applied += external_events
                        .iter()
                        .filter(|e| matches!(e, OrderEventAny::Filled(_)))
                        .count();

                    if report.order_status.is_open() {
                        open_orders_initialized += 1;
                    }

                    events.extend(external_events);

                    if let Some(m) = metadata {
                        external_orders.push(m);
                    }
                }
            } else {
                orders_skipped_no_instrument += 1;
            }
        }

        // 处理孤立成交（没有匹配订单报告的成交）。
        let processed_venue_order_ids: AHashSet<VenueOrderId> =
            order_reports.keys().copied().collect();

        for (venue_order_id, fills) in fill_reports {
            if processed_venue_order_ids.contains(venue_order_id) {
                continue;
            }

            let Some(first_fill) = fills.first() else {
                continue;
            };

            if !self.should_reconcile_instrument(&first_fill.instrument_id) {
                log::debug!(
                    "Skipping orphan fills for {}: not in reconciliation_instrument_ids",
                    first_fill.instrument_id
                );
                continue;
            }

            // Skip if fill's client_order_id is in filtered list
            if let Some(client_order_id) = &first_fill.client_order_id
                && self
                    .config
                    .filtered_client_order_ids
                    .contains(client_order_id)
            {
                log::debug!(
                    "Skipping orphan fills for {client_order_id}: in filtered_client_order_ids"
                );
                continue;
            }

            let order = first_fill
                .client_order_id
                .as_ref()
                .and_then(|id| self.get_order(id))
                .or_else(|| self.get_order_by_venue_order_id(venue_order_id));

            // Skip if resolved order's client_order_id is filtered (venue_order_id lookup path)
            if let Some(ref order) = order
                && self
                    .config
                    .filtered_client_order_ids
                    .contains(&order.client_order_id())
            {
                log::debug!(
                    "Skipping orphan fills for {}: in filtered_client_order_ids",
                    order.client_order_id()
                );
                continue;
            }

            if let Some(mut order) = order {
                let instrument_id = order.instrument_id();
                if let Some(instrument) = self.get_instrument(&instrument_id) {
                    let mut sorted_fills: Vec<&FillReport> = fills.iter().collect();
                    sorted_fills.sort_by_key(|f| f.ts_event);

                    for fill in sorted_fills {
                        if let Some(event) = self.create_order_fill(&mut order, fill, &instrument) {
                            fills_applied += 1;
                            events.push(event);
                        }
                    }
                }
            }
        }

        events.sort_by_key(|e| e.ts_event());

        for event in &events {
            exec_engine.borrow_mut().process(event.clone());
        }

        let mut positions_created = 0usize;
        if !self.config.filter_position_reports {
            // 处理在批量报告中缺少 venue_position_id 的标的成交
            // （无法归因于特定的对冲持仓，因此必须跳过该标的所有对冲报告）
            let instruments_with_unattributed_fills: AHashSet<InstrumentId> = mass_status
                .fill_reports()
                .values()
                .flatten()
                .filter(|f| f.venue_position_id.is_none())
                .map(|f| f.instrument_id)
                .chain(
                    mass_status
                        .order_reports()
                        .values()
                        .filter(|r| !r.filled_qty.is_zero() && r.venue_position_id.is_none())
                        .map(|r| r.instrument_id),
                )
                .collect();

            let positions_with_fills: AHashSet<PositionId> = mass_status
                .fill_reports()
                .values()
                .flatten()
                .filter_map(|f| f.venue_position_id)
                .chain(
                    mass_status
                        .order_reports()
                        .values()
                        .filter(|r| !r.filled_qty.is_zero())
                        .filter_map(|r| r.venue_position_id),
                )
                .collect();

            for (instrument_id, reports) in mass_status.position_reports() {
                if !self.should_reconcile_instrument(&instrument_id) {
                    log::debug!(
                        "Skipping position reports for {instrument_id}: not in reconciliation_instrument_ids"
                    );
                    continue;
                }

                for report in reports {
                    if let Some(position_events) = self.reconcile_position_report(
                        &report,
                        &mass_status.account_id,
                        &instruments_with_unattributed_fills,
                        &positions_with_fills,
                    ) {
                        for event in position_events {
                            exec_engine.borrow_mut().process(event.clone());
                            events.push(event);
                        }
                        positions_created += 1;
                    }
                }
            }
        }

        if orders_skipped_no_instrument > 0 {
            log::warn!("{orders_skipped_no_instrument} orders skipped (instrument not in cache)");
        }

        if orders_skipped_duplicate > 0 {
            log::debug!("{orders_skipped_duplicate} orders skipped (already in sync)");
        }

        if orders_skipped_filtered > 0 {
            log::debug!("{orders_skipped_filtered} orders skipped (filtered by config)");
        }

        log::info!(
            color = LogColor::Blue as u8;
            "Reconciliation complete for {venue}: reconciled={orders_reconciled}, external={external_orders_created}, open={open_orders_initialized}, fills={fills_applied}, positions={positions_created}, skipped={orders_skipped_duplicate}, filtered={orders_skipped_filtered}",
        );

        ReconciliationResult {
            events,
            external_orders,
        }
    }

    /// 在运行时对单个执行报告进行对账。
    ///
    /// # 错误
    ///
    /// 如果平均价格无法转换为有效的 `Decimal`，则返回错误。
    pub fn reconcile_report(
        &mut self,
        report: ExecutionReport,
    ) -> anyhow::Result<Vec<OrderEventAny>> {
        let mut events = Vec::new();

        self.clear_recon_tracking(&report.client_order_id, true);

        if let Some(order) = self.get_order(&report.client_order_id) {
            let Some(account_id) = order.account_id() else {
                log::error!("Cannot process fill report: order has no account_id");
                return Ok(vec![]);
            };
            let Some(venue_order_id) = report.venue_order_id else {
                log::error!("Cannot process fill report: report has no venue_order_id");
                return Ok(vec![]);
            };
            let mut order_report = OrderStatusReport::new(
                account_id,
                order.instrument_id(),
                Some(report.client_order_id),
                venue_order_id,
                order.order_side(),
                order.order_type(),
                order.time_in_force(),
                report.status,
                order.quantity(),
                report.filled_qty,
                report.ts_event, // 使用 ts_event 作为 ts_accepted
                report.ts_event, // 使用 ts_event 作为 ts_last
                self.clock.borrow().timestamp_ns(),
                Some(UUID4::new()),
            );

            if let Some(avg_px) = report.avg_px {
                order_report = order_report.with_avg_px(avg_px)?;
            }

            let instrument = self.get_instrument(&order.instrument_id());
            if let Some(event) =
                self.reconcile_order_report(&order, &order_report, instrument.as_ref())
            {
                events.push(event);
            }
        }

        Ok(events)
    }

    /// 检查在途订单，并为任何需要对账的订单返回事件。
    pub fn check_inflight_orders(&mut self) -> Vec<OrderEventAny> {
        let mut events = Vec::new();
        let current_time = self.clock.borrow().timestamp_ns();
        let threshold_ns = self.config.inflight_threshold_ms * NANOSECONDS_IN_MILLISECOND;

        let mut to_check = Vec::new();

        for (client_order_id, check) in &self.inflight_checks {
            if current_time - check.ts_submitted > threshold_ns {
                to_check.push(*client_order_id);
            }
        }

        for client_order_id in to_check {
            if self
                .config
                .filtered_client_order_ids
                .contains(&client_order_id)
            {
                continue;
            }

            if let Some(check) = self.inflight_checks.get_mut(&client_order_id) {
                if let Some(last_query_ts) = check.last_query_ts
                    && current_time - last_query_ts < threshold_ns
                {
                    continue;
                }

                check.retry_count += 1;
                check.last_query_ts = Some(current_time);
                self.ts_last_query.insert(client_order_id, current_time);
                self.recon_check_retries
                    .insert(client_order_id, check.retry_count);

                if check.retry_count >= self.config.inflight_max_retries {
                    // 超过最大重试次数后生成驳回事件。
                    let ts_now = self.clock.borrow().timestamp_ns();
                    if let Some(order) = self.get_order(&client_order_id)
                        && let Some(event) =
                            create_reconciliation_rejected(&order, Some("INFLIGHT_TIMEOUT"), ts_now)
                    {
                        events.push(event);
                    }
                    // 无论订单是否存在，都从在途检查中移除
                    self.clear_recon_tracking(&client_order_id, true);
                }
            }
        }

        events
    }

    /// 检查缓存与交易平台之间开仓订单的一致性。
    ///
    /// 此方法验证缓存中的开销订单是否与交易平台状态匹配，比较订单状态和已成交数量，
    /// 并为检测到的任何差异生成对账事件。
    ///
    /// # 返回
    ///
    /// 生成的用于对齐差异的订单事件向量。
    pub async fn check_open_orders(
        &mut self,
        clients: &[Rc<dyn ExecutionClient>],
    ) -> Vec<OrderEventAny> {
        log::debug!("正在检查缓存状态与交易平台之间的订单一致性");

        let filtered_orders: Vec<OrderAny> = {
            let cache = self.cache.borrow();
            let mut orders = cache.orders_open(None, None, None, None, None);
            orders.extend(cache.orders_inflight(None, None, None, None, None));

            if self.config.reconciliation_instrument_ids.is_empty() {
                orders.iter().map(|o| (*o).clone()).collect()
            } else {
                orders
                    .iter()
                    .filter(|o| {
                        self.config
                            .reconciliation_instrument_ids
                            .contains(&o.instrument_id())
                    })
                    .map(|o| (*o).clone())
                    .collect()
            }
        };

        log::debug!("在缓存中发现 {} 个开仓订单", filtered_orders.len(),);

        let mut all_reports = Vec::new();
        let mut venue_reported_ids = AHashSet::new();

        for client in clients {
            let mut cmd = GenerateOrderStatusReports::new(
                UUID4::new(),
                self.clock.borrow().timestamp_ns(),
                true, // 仅限开仓 (open_only)
                None, // instrument_id - 查询全部
                None, // 开始时间
                None, // 结束时间
                None, // 参数
                None, // 关联 ID (correlation_id)
            );
            cmd.log_receipt_level = LogLevel::Debug;

            match client.generate_order_status_reports(&cmd).await {
                Ok(reports) => {
                    for report in reports {
                        if let Some(client_order_id) = &report.client_order_id {
                            venue_reported_ids.insert(*client_order_id);
                        }
                        all_reports.push(report);
                    }
                }
                Err(e) => {
                    log::error!("无法从 {} 查询订单报告：{e}", client.client_id());
                }
            }
        }

        // 根据缓存的订单对报告进行对账
        let ts_now = self.clock.borrow().timestamp_ns();
        let mut events = Vec::new();

        for report in all_reports {
            if let Some(client_order_id) = &report.client_order_id
                && let Some(order) = self.get_order(client_order_id)
            {
                // 检查最近的本地活动，以避免与在途成交产生竞争条件
                if let Some(&last_activity) = self.order_local_activity_ns.get(client_order_id)
                    && (ts_now - last_activity) < self.config.open_check_threshold_ns
                {
                    let elapsed_ms = nanos_to_millis((ts_now - last_activity).as_u64());
                    let threshold_ms = nanos_to_millis(self.config.open_check_threshold_ns);
                    log::info!(
                        "延迟对 {client_order_id} 的对账：存在最近的本地活动（{elapsed_ms}ms < 阈值={threshold_ms}ms）",
                    );
                    continue;
                }

                let instrument = self.get_instrument(&report.instrument_id);

                if let Some(event) =
                    self.reconcile_order_report(&order, &report, instrument.as_ref())
                {
                    events.push(event);
                }
            }
        }

        // 处理在交易平台缺失的订单。
        if !self.config.open_check_open_only {
            let cached_ids: AHashSet<ClientOrderId> = filtered_orders
                .iter()
                .map(|o| o.client_order_id())
                .collect();
            let missing_at_venue: AHashSet<ClientOrderId> = cached_ids
                .difference(&venue_reported_ids)
                .copied()
                .collect();

            for client_order_id in missing_at_venue {
                events.extend(self.handle_missing_order(client_order_id));
            }
        }

        events
    }

    /// 检查缓存与交易平台之间持仓的一致性。
    ///
    /// 此方法验证缓存中的持仓是否与交易平台状态匹配，检测持仓偏差，
    /// 并在发现差异时查询缺失的成交。
    ///
    /// # 返回
    ///
    /// 生成的用于协调持仓差异的成交事件向量。
    pub async fn check_positions_consistency(
        &mut self,
        clients: &[Rc<dyn ExecutionClient>],
    ) -> Vec<OrderEventAny> {
        log::debug!("正在检查缓存状态与交易平台之间的持仓一致性");

        let open_positions = {
            let cache = self.cache.borrow();
            let positions = cache.positions_open(None, None, None, None, None);

            if self.config.reconciliation_instrument_ids.is_empty() {
                positions.iter().map(|p| (*p).clone()).collect()
            } else {
                positions
                    .iter()
                    .filter(|p| {
                        self.config
                            .reconciliation_instrument_ids
                            .contains(&p.instrument_id)
                    })
                    .map(|p| (*p).clone())
                    .collect::<Vec<_>>()
            }
        };

        log::debug!("发现 {} 个持仓待检查", open_positions.len(),);

        // 向交易平台查询持仓报告
        let mut venue_positions = AHashMap::new();

        for client in clients {
            let mut cmd = GeneratePositionStatusReports::new(
                UUID4::new(),
                self.clock.borrow().timestamp_ns(),
                None, // instrument_id - 查询全部
                None, // 开始时间
                None, // 结束时间
                None, // 参数
                None, // 关联 ID (correlation_id)
            );
            cmd.log_receipt_level = LogLevel::Debug;

            match client.generate_position_status_reports(&cmd).await {
                Ok(reports) => {
                    for report in reports {
                        venue_positions.insert(report.instrument_id, report);
                    }
                }
                Err(e) => {
                    log::error!("无法从 {} 查询持仓报告：{e}", client.client_id());
                }
            }
        }

        // 检查差异
        let mut events = Vec::new();

        for position in &open_positions {
            // 如果不在过滤器中则跳过
            if !self.config.reconciliation_instrument_ids.is_empty()
                && !self
                    .config
                    .reconciliation_instrument_ids
                    .contains(&position.instrument_id)
            {
                continue;
            }

            let venue_report = venue_positions.get(&position.instrument_id);

            if let Some(discrepancy_events) =
                self.check_position_discrepancy(position, venue_report)
            {
                events.extend(discrepancy_events);
            }
        }

        events
    }

    /// 将订单注册为在途 (inflight) 以进行跟踪。
    pub fn register_inflight(&mut self, client_order_id: ClientOrderId) {
        let ts_submitted = self.clock.borrow().timestamp_ns();
        self.inflight_checks.insert(
            client_order_id,
            InflightCheck {
                client_order_id,
                ts_submitted,
                retry_count: 0,
                last_query_ts: None,
            },
        );
        self.recon_check_retries.insert(client_order_id, 0);
        self.ts_last_query.remove(&client_order_id);
        self.order_local_activity_ns.remove(&client_order_id);
    }

    /// 记录指定订单的本地活动。
    ///
    /// 使用当前时钟时间（受理时间）而非交易平台时间，以准确跟踪我们上次处理此订单活动的时间。
    /// 这可以避免因网络/队列延迟导致事件虽然刚到达但看起来“陈旧”而产生的竞争条件。
    pub fn record_local_activity(&mut self, client_order_id: ClientOrderId) {
        let ts_now = self.clock.borrow().timestamp_ns();
        self.order_local_activity_ns.insert(client_order_id, ts_now);
    }

    /// 清除订单的对账跟踪状态。
    pub fn clear_recon_tracking(&mut self, client_order_id: &ClientOrderId, drop_last_query: bool) {
        self.inflight_checks.remove(client_order_id);
        self.recon_check_retries.remove(client_order_id);
        if drop_last_query {
            self.ts_last_query.remove(client_order_id);
        }
        self.order_local_activity_ns.remove(client_order_id);
    }

    /// 为特定的策略和标的申领外部订单。
    pub fn claim_external_orders(&mut self, instrument_id: InstrumentId, strategy_id: StrategyId) {
        self.external_order_claims
            .insert(instrument_id, strategy_id);
    }

    /// 记录持仓活动以进行对账跟踪。
    pub fn record_position_activity(&mut self, instrument_id: InstrumentId, ts_event: UnixNanos) {
        self.position_local_activity_ns
            .insert(instrument_id, ts_event);
    }

    /// 检查成交是否在最近处理过（用于去重）。
    pub fn is_fill_recently_processed(&self, trade_id: &TradeId) -> bool {
        self.recent_fills_cache.contains_key(trade_id)
    }

    /// 将成交标记为已处理，并记录当前时间戳。
    pub fn mark_fill_processed(&mut self, trade_id: TradeId) {
        let ts_now = self.clock.borrow().timestamp_ns();
        self.recent_fills_cache.insert(trade_id, ts_now);
    }

    /// 从最近成交缓存中清除过期的成交。
    ///
    /// 默认生存时间 (TTL) 为 60 秒。
    pub fn prune_recent_fills_cache(&mut self, ttl_secs: f64) {
        let ts_now = self.clock.borrow().timestamp_ns();
        let ttl_ns = (ttl_secs * NANOSECONDS_IN_SECOND as f64) as u64;

        self.recent_fills_cache
            .retain(|_, &mut ts_cached| ts_now - ts_cached <= ttl_ns);
    }

    /// 从缓存中清除早于配置缓冲时间的已关闭订单。
    pub fn purge_closed_orders(&mut self) {
        let Some(buffer_mins) = self.config.purge_closed_orders_buffer_mins else {
            return;
        };

        let ts_now = self.clock.borrow().timestamp_ns();
        let buffer_secs = (buffer_mins as u64) * 60;

        self.cache
            .borrow_mut()
            .purge_closed_orders(ts_now, buffer_secs);
    }

    /// 从缓存中清除早于配置缓冲时间的已关闭持仓。
    pub fn purge_closed_positions(&mut self) {
        let Some(buffer_mins) = self.config.purge_closed_positions_buffer_mins else {
            return;
        };

        let ts_now = self.clock.borrow().timestamp_ns();
        let buffer_secs = (buffer_mins as u64) * 60;

        self.cache
            .borrow_mut()
            .purge_closed_positions(ts_now, buffer_secs);
    }

    /// 根据配置的回顾时间从缓存中清除旧的账户事件。
    pub fn purge_account_events(&mut self) {
        let Some(lookback_mins) = self.config.purge_account_events_lookback_mins else {
            return;
        };

        let ts_now = self.clock.borrow().timestamp_ns();
        let lookback_secs = (lookback_mins as u64) * 60;

        self.cache
            .borrow_mut()
            .purge_account_events(ts_now, lookback_secs);
    }

    // 私有辅助方法

    fn get_order(&self, client_order_id: &ClientOrderId) -> Option<OrderAny> {
        self.cache.borrow().order(client_order_id).cloned()
    }

    fn get_order_by_venue_order_id(&self, venue_order_id: &VenueOrderId) -> Option<OrderAny> {
        let cache = self.cache.borrow();
        cache
            .client_order_id(venue_order_id)
            .and_then(|client_order_id| cache.order(client_order_id).cloned())
    }

    fn get_instrument(&self, instrument_id: &InstrumentId) -> Option<InstrumentAny> {
        self.cache.borrow().instrument(instrument_id).cloned()
    }

    fn should_skip_order_report(&self, report: &OrderStatusReport) -> bool {
        if let Some(client_order_id) = &report.client_order_id
            && self
                .config
                .filtered_client_order_ids
                .contains(client_order_id)
        {
            log::debug!("跳过订单报告 {client_order_id}：在 filtered_client_order_ids 列表中");
            return true;
        }

        if !self.should_reconcile_instrument(&report.instrument_id) {
            log::debug!(
                "跳过 {} 的订单报告：不在 reconciliation_instrument_ids 中",
                report.instrument_id
            );
            return true;
        }

        false
    }

    fn should_reconcile_instrument(&self, instrument_id: &InstrumentId) -> bool {
        self.config.reconciliation_instrument_ids.is_empty()
            || self
                .config
                .reconciliation_instrument_ids
                .contains(instrument_id)
    }

    fn handle_missing_order(&mut self, client_order_id: ClientOrderId) -> Vec<OrderEventAny> {
        let mut events = Vec::new();

        let Some(order) = self.get_order(&client_order_id) else {
            return events;
        };

        let ts_now = self.clock.borrow().timestamp_ns();
        let ts_last = order.ts_last();

        // 检查订单是否太新
        if (ts_now - ts_last) < self.config.open_check_threshold_ns {
            return events;
        }

        // 检查本地活动阈值
        if let Some(&last_activity) = self.order_local_activity_ns.get(&client_order_id)
            && (ts_now - last_activity) < self.config.open_check_threshold_ns
        {
            return events;
        }

        // 增加重试计数
        let retries = self.recon_check_retries.entry(client_order_id).or_insert(0);
        *retries += 1;

        // 如果超过最大重试次数，生成驳回事件
        if *retries >= self.config.open_check_missing_retries {
            log::warn!(
                "重试 {retries} 次后订单 {client_order_id} 在交易平台未找到，标记为 REJECTED"
            );

            let ts_now = self.clock.borrow().timestamp_ns();
            if let Some(rejected) =
                create_reconciliation_rejected(&order, Some("NOT_FOUND_AT_VENUE"), ts_now)
            {
                events.push(rejected);
            }

            self.clear_recon_tracking(&client_order_id, true);
        } else {
            log::debug!(
                "订单 {} 在交易平台未找到，重试 {}/{}",
                client_order_id,
                retries,
                self.config.open_check_missing_retries
            );
        }

        events
    }

    fn check_position_discrepancy(
        &mut self,
        position: &Position,
        venue_report: Option<&PositionStatusReport>,
    ) -> Option<Vec<OrderEventAny>> {
        // 使用有符号的数量来检测量级和方向上的差异
        let cached_signed_qty = position.signed_decimal_qty();
        let venue_signed_qty = venue_report.map_or(Decimal::ZERO, |r| r.signed_decimal_qty);

        let tolerance = Decimal::from_str("0.00000001").unwrap();
        if (cached_signed_qty - venue_signed_qty).abs() <= tolerance {
            return None; // 无差异
        }

        // 检查活动阈值
        let ts_now = self.clock.borrow().timestamp_ns();
        if let Some(&last_activity) = self.position_local_activity_ns.get(&position.instrument_id)
            && (ts_now - last_activity) < self.config.position_check_threshold_ns
        {
            log::debug!(
                "跳过 {} 的持仓对账：最近活动在阈值范围内",
                position.instrument_id
            );
            return None;
        }

        log::warn!(
            "检测到 {} 的持仓差异：cached_signed_qty={}，venue_signed_qty={}",
            position.instrument_id,
            cached_signed_qty,
            venue_signed_qty
        );

        let instrument = self
            .cache
            .borrow()
            .instrument(&position.instrument_id)?
            .clone();

        let account_id = position.account_id;
        let instrument_id = position.instrument_id;

        let cached_avg_px = if position.avg_px_open > 0.0 {
            Decimal::from_str(&position.avg_px_open.to_string()).ok()
        } else {
            None
        };
        let venue_avg_px = venue_report.and_then(|r| r.avg_px_open);

        // 检查持仓是否跨越零（反向）
        let crosses_zero = (cached_signed_qty > Decimal::ZERO && venue_signed_qty < Decimal::ZERO)
            || (cached_signed_qty < Decimal::ZERO && venue_signed_qty > Decimal::ZERO);

        if crosses_zero {
            // 拆分为两个成交：首先关闭现有持仓，然后开设新持仓
            return self.reconcile_cross_zero_position(
                &instrument,
                account_id,
                instrument_id,
                cached_signed_qty,
                cached_avg_px,
                venue_signed_qty,
                venue_avg_px,
                ts_now,
            );
        }

        let qty_diff = venue_signed_qty - cached_signed_qty;
        let order_side = if qty_diff > Decimal::ZERO {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };

        let reconciliation_px = calculate_reconciliation_price(
            cached_signed_qty,
            cached_avg_px,
            venue_signed_qty,
            venue_avg_px,
        );

        let fill_px = reconciliation_px.or(venue_avg_px).or(cached_avg_px)?;
        let fill_qty = qty_diff.abs();

        let ts_event = ts_now.as_u64();
        let venue_order_id = create_synthetic_venue_order_id(ts_event);
        let order_qty = Quantity::from_decimal_dp(fill_qty, instrument.size_precision()).ok()?;

        let order_report = OrderStatusReport::new(
            account_id,
            instrument_id,
            None,
            venue_order_id,
            order_side,
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Filled,
            order_qty,
            order_qty,
            ts_now,
            ts_now,
            ts_now,
            None,
        )
        .with_avg_px(fill_px.to_f64().unwrap_or(0.0))
        .ok()?;

        log::info!(
            color = LogColor::Blue as u8;
            "正在为 {} 生成持仓对账合成成交：side={:?}，qty={}，px={}",
            instrument_id,
            order_side,
            fill_qty,
            fill_px,
        );

        let (events, _) =
            self.handle_external_order(&order_report, &account_id, &instrument, &[], true);
        Some(events)
    }

    /// 处理持仓正负号翻转时的持仓对账，通过拆分为两个成交：首先关闭现有持仓，然后在相反方向开设新持仓。
    #[allow(clippy::too_many_arguments)]
    fn reconcile_cross_zero_position(
        &mut self,
        instrument: &InstrumentAny,
        account_id: AccountId,
        instrument_id: InstrumentId,
        cached_signed_qty: Decimal,
        cached_avg_px: Option<Decimal>,
        venue_signed_qty: Decimal,
        venue_avg_px: Option<Decimal>,
        ts_now: UnixNanos,
    ) -> Option<Vec<OrderEventAny>> {
        log::info!(
            color = LogColor::Blue as u8;
            "{} 的持仓跨越零：缓存={}，交易平台={}。正在拆分为两个成交",
            instrument_id,
            cached_signed_qty,
            venue_signed_qty,
        );

        let mut all_events = Vec::new();

        // 首先关闭现有持仓
        let close_qty = cached_signed_qty.abs();
        let close_side = if cached_signed_qty < Decimal::ZERO {
            OrderSide::Buy // 通过买入平掉空头
        } else {
            OrderSide::Sell // 通过卖出平掉多头
        };

        if let Some(close_px) = cached_avg_px {
            let close_venue_order_id = create_synthetic_venue_order_id(ts_now.as_u64());
            let close_order_qty =
                Quantity::from_decimal_dp(close_qty, instrument.size_precision()).ok()?;

            let close_report = OrderStatusReport::new(
                account_id,
                instrument_id,
                None,
                close_venue_order_id,
                close_side,
                OrderType::Market,
                TimeInForce::Gtc,
                OrderStatus::Filled,
                close_order_qty,
                close_order_qty,
                ts_now,
                ts_now,
                ts_now,
                None,
            )
            .with_avg_px(close_px.to_f64().unwrap_or(0.0))
            .ok()?;

            log::info!(
                color = LogColor::Blue as u8;
                "正在为跨零 {} 生成平仓成交：side={:?}，qty={}，px={}",
                instrument_id,
                close_side,
                close_qty,
                close_px,
            );

            let (close_events, _) =
                self.handle_external_order(&close_report, &account_id, instrument, &[], true);
            all_events.extend(close_events);
        } else {
            log::warn!("无法关闭 {} 的持仓：没有缓存的平均价格", instrument_id);
            return None;
        }

        // 然后在相反方向开设新持仓
        let open_qty = venue_signed_qty.abs();
        let open_side = if venue_signed_qty > Decimal::ZERO {
            OrderSide::Buy // 开设多头
        } else {
            OrderSide::Sell // 开设空头
        };

        if let Some(open_px) = venue_avg_px {
            let open_venue_order_id = create_synthetic_venue_order_id(ts_now.as_u64() + 1);
            let open_order_qty =
                Quantity::from_decimal_dp(open_qty, instrument.size_precision()).ok()?;

            let open_report = OrderStatusReport::new(
                account_id,
                instrument_id,
                None,
                open_venue_order_id,
                open_side,
                OrderType::Market,
                TimeInForce::Gtc,
                OrderStatus::Filled,
                open_order_qty,
                open_order_qty,
                ts_now,
                ts_now,
                ts_now,
                None,
            )
            .with_avg_px(open_px.to_f64().unwrap_or(0.0))
            .ok()?;

            log::info!(
                color = LogColor::Blue as u8;
                "正在为跨零 {} 生成开仓成交：side={:?}，qty={}，px={}",
                instrument_id,
                open_side,
                open_qty,
                open_px,
            );

            let (open_events, _) =
                self.handle_external_order(&open_report, &account_id, instrument, &[], true);
            all_events.extend(open_events);
        } else {
            log::warn!(
                "无法为 {} 开设新持仓：没有交易平台的平均价格",
                instrument_id
            );
            return Some(all_events);
        }

        Some(all_events)
    }

    /// 在没有任何订单/成交存在时，根据交易平台持仓报告创建持仓。
    ///
    /// 这处理了由于交易平台报告存在开仓持仓，但没有任何订单或成交报告可供创建的情形（例如，订单已关闭）。
    fn create_position_from_report(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
        instrument: &InstrumentAny,
    ) -> Option<Vec<OrderEventAny>> {
        let instrument_id = report.instrument_id;
        let venue_signed_qty = report.signed_decimal_qty;

        if venue_signed_qty == Decimal::ZERO {
            return None;
        }

        let order_side = if venue_signed_qty > Decimal::ZERO {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };

        let qty_abs = venue_signed_qty.abs();
        let venue_avg_px = report.avg_px_open?;

        let ts_now = self.clock.borrow().timestamp_ns();
        let venue_order_id = create_synthetic_venue_order_id(ts_now.as_u64());
        let order_qty = Quantity::from_decimal_dp(qty_abs, instrument.size_precision()).ok()?;

        let mut order_report = OrderStatusReport::new(
            *account_id,
            instrument_id,
            None,
            venue_order_id,
            order_side,
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Filled,
            order_qty,
            order_qty,
            ts_now,
            ts_now,
            ts_now,
            None,
        )
        .with_avg_px(venue_avg_px.to_f64().unwrap_or(0.0))
        .ok()?;

        // Preserve venue_position_id for hedging mode
        if let Some(venue_position_id) = report.venue_position_id {
            order_report = order_report.with_venue_position_id(venue_position_id);
        }

        log::info!(
            color = LogColor::Blue as u8;
            "正在根据交易平台报告为 {} 创建持仓：side={:?}，qty={}，avg_px={}",
            instrument_id,
            order_side,
            qty_abs,
            venue_avg_px,
        );

        let (events, _) =
            self.handle_external_order(&order_report, account_id, instrument, &[], true);
        Some(events)
    }

    fn reconcile_position_report(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
        instruments_with_unattributed_fills: &AHashSet<InstrumentId>,
        positions_with_fills: &AHashSet<PositionId>,
    ) -> Option<Vec<OrderEventAny>> {
        if report.venue_position_id.is_some() {
            self.reconcile_position_report_hedging(
                report,
                account_id,
                instruments_with_unattributed_fills,
                positions_with_fills,
            )
        } else {
            self.reconcile_position_report_netting(report, account_id)
        }
    }

    fn reconcile_position_report_hedging(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
        instruments_with_unattributed_fills: &AHashSet<InstrumentId>,
        positions_with_fills: &AHashSet<PositionId>,
    ) -> Option<Vec<OrderEventAny>> {
        let venue_position_id = report.venue_position_id?;

        // 如果批次中已包含此持仓的成交，则跳过（将从成交中创建）。
        if positions_with_fills.contains(&venue_position_id) {
            log::debug!("跳过对冲持仓 {venue_position_id} 对账：批次中已存在成交");
            return None;
        }

        // 如果此标的存在成交但缺少 venue_position_id，则跳过
        // （无法确定它们属于哪个对冲持仓）。
        if instruments_with_unattributed_fills.contains(&report.instrument_id) {
            log::debug!("跳过对冲持仓 {venue_position_id} 对账：批次中存在未归因的成交");
            return None;
        }

        log::debug!(
            "正在对 {} 进行对冲持仓对账，venue_position_id={}",
            report.instrument_id,
            venue_position_id
        );

        let position = {
            let cache = self.cache.borrow();
            cache.position(&venue_position_id).cloned()
        };

        match position {
            Some(position) => {
                let cached_signed_qty = position.signed_decimal_qty();
                let venue_signed_qty = report.signed_decimal_qty;

                if cached_signed_qty == venue_signed_qty {
                    log::debug!(
                        "对冲持仓 {venue_position_id} 与交易平台匹配：qty={cached_signed_qty}"
                    );
                    return None;
                }

                if venue_signed_qty == Decimal::ZERO && cached_signed_qty == Decimal::ZERO {
                    return None;
                }

                if !self.config.generate_missing_orders {
                    log::error!(
                        "无法对账 {} {}：持仓净数量 {} != 报告净数量 {} \
                         且 `generate_missing_orders` 已禁用",
                        report.instrument_id,
                        venue_position_id,
                        cached_signed_qty,
                        venue_signed_qty
                    );
                    return None;
                }

                self.reconcile_hedge_position_discrepancy(
                    report,
                    account_id,
                    &position,
                    cached_signed_qty,
                )
            }
            None => {
                if report.signed_decimal_qty == Decimal::ZERO {
                    return None;
                }

                if !self.config.generate_missing_orders {
                    log::error!(
                        "无法对账持仓：未找到 {venue_position_id} 且 `generate_missing_orders` 已禁用"
                    );
                    return None;
                }

                self.reconcile_missing_hedge_position(report, account_id)
            }
        }
    }

    fn reconcile_hedge_position_discrepancy(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
        position: &Position,
        cached_signed_qty: Decimal,
    ) -> Option<Vec<OrderEventAny>> {
        let instrument = self.get_instrument(&report.instrument_id)?;
        let venue_signed_qty = report.signed_decimal_qty;

        let diff = (cached_signed_qty - venue_signed_qty).abs();
        let diff_qty = Quantity::from_decimal_dp(diff, instrument.size_precision()).ok()?;

        if diff_qty.is_zero() {
            log::debug!("{} 的差异数量经舍入后为零，跳过", instrument.id());
            return None;
        }

        let venue_position_id = report.venue_position_id?;
        log::warn!(
            "{} {} 的对冲持仓差异：缓存={}，交易平台={}，正在生成对账订单",
            report.instrument_id,
            venue_position_id,
            cached_signed_qty,
            venue_signed_qty
        );

        let current_avg_px = if position.avg_px_open > 0.0 {
            Decimal::from_str(&position.avg_px_open.to_string()).ok()
        } else {
            None
        };

        self.create_position_reconciliation_order(
            report,
            account_id,
            &instrument,
            cached_signed_qty,
            diff_qty,
            current_avg_px,
        )
    }

    fn reconcile_missing_hedge_position(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
    ) -> Option<Vec<OrderEventAny>> {
        let instrument = self.get_instrument(&report.instrument_id)?;
        let venue_signed_qty = report.signed_decimal_qty;

        let qty = venue_signed_qty.abs();
        let diff_qty = Quantity::from_decimal_dp(qty, instrument.size_precision()).ok()?;

        if diff_qty.is_zero() {
            return None;
        }

        let venue_position_id = report.venue_position_id?;
        log::warn!(
            "{} {} 缺少对冲持仓：交易平台报告为 {}，正在生成对账订单",
            report.instrument_id,
            venue_position_id,
            venue_signed_qty
        );

        self.create_position_reconciliation_order(
            report,
            account_id,
            &instrument,
            Decimal::ZERO,
            diff_qty,
            None,
        )
    }

    fn reconcile_position_report_netting(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
    ) -> Option<Vec<OrderEventAny>> {
        let instrument_id = report.instrument_id;

        log::debug!("正在为 {instrument_id} 对账 NET 持仓");

        let instrument = self.get_instrument(&instrument_id)?;

        let (cached_signed_qty, cached_avg_px) = {
            let cache = self.cache.borrow();
            let positions = cache.positions_open(None, Some(&instrument_id), None, None, None);

            if positions.is_empty() {
                (Decimal::ZERO, None)
            } else {
                let mut total_signed_qty = Decimal::ZERO;
                let mut total_value = Decimal::ZERO;
                let mut total_qty = Decimal::ZERO;

                for pos in positions {
                    total_signed_qty += pos.signed_decimal_qty();
                    let qty = pos.signed_decimal_qty().abs();
                    if pos.avg_px_open > 0.0
                        && qty > Decimal::ZERO
                        && let Ok(avg_px) = Decimal::from_str(&pos.avg_px_open.to_string())
                    {
                        total_value += avg_px * qty;
                        total_qty += qty;
                    }
                }

                let avg_px = if total_qty > Decimal::ZERO {
                    Some(total_value / total_qty)
                } else {
                    None
                };

                (total_signed_qty, avg_px)
            }
        };

        let venue_signed_qty = report.signed_decimal_qty;

        log::debug!("venue_signed_qty={venue_signed_qty}, cached_signed_qty={cached_signed_qty}");

        let tolerance = Decimal::from_str("0.00000001").unwrap_or(Decimal::ZERO);
        if (cached_signed_qty - venue_signed_qty).abs() <= tolerance {
            log::debug!("{instrument_id} 的持仓数量匹配，无需对账");
            return None;
        }

        if !self.config.generate_missing_orders {
            log::warn!("{instrument_id} 的持仓存在差异，但 `generate_missing_orders` 已禁用，跳过");
            return None;
        }

        let diff = (cached_signed_qty - venue_signed_qty).abs();
        let diff_qty = Quantity::from_decimal_dp(diff, instrument.size_precision()).ok()?;

        if diff_qty.is_zero() {
            log::debug!("{instrument_id} 的差异数量经舍入后为零，跳过订单生成");
            return None;
        }

        let crosses_zero = cached_signed_qty != Decimal::ZERO
            && venue_signed_qty != Decimal::ZERO
            && ((cached_signed_qty > Decimal::ZERO && venue_signed_qty < Decimal::ZERO)
                || (cached_signed_qty < Decimal::ZERO && venue_signed_qty > Decimal::ZERO));

        if crosses_zero {
            let ts_now = self.clock.borrow().timestamp_ns();
            return self.reconcile_cross_zero_position(
                &instrument,
                *account_id,
                instrument_id,
                cached_signed_qty,
                cached_avg_px,
                venue_signed_qty,
                report.avg_px_open,
                ts_now,
            );
        }

        if cached_signed_qty == Decimal::ZERO {
            return self.create_position_from_report(report, account_id, &instrument);
        }

        self.create_position_reconciliation_order(
            report,
            account_id,
            &instrument,
            cached_signed_qty,
            diff_qty,
            cached_avg_px,
        )
    }

    fn create_position_reconciliation_order(
        &mut self,
        report: &PositionStatusReport,
        account_id: &AccountId,
        instrument: &InstrumentAny,
        cached_signed_qty: Decimal,
        diff_qty: Quantity,
        current_avg_px: Option<Decimal>,
    ) -> Option<Vec<OrderEventAny>> {
        let venue_signed_qty = report.signed_decimal_qty;
        let instrument_id = report.instrument_id;

        let order_side = if venue_signed_qty > cached_signed_qty {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };

        let reconciliation_px = calculate_reconciliation_price(
            cached_signed_qty,
            current_avg_px,
            venue_signed_qty,
            report.avg_px_open,
        );

        let fill_px = reconciliation_px
            .or(report.avg_px_open)
            .or(current_avg_px)?;

        let ts_now = self.clock.borrow().timestamp_ns();
        let venue_order_id = create_synthetic_venue_order_id(ts_now.as_u64());

        let mut order_report = OrderStatusReport::new(
            *account_id,
            instrument_id,
            None,
            venue_order_id,
            order_side,
            OrderType::Market,
            TimeInForce::Gtc,
            OrderStatus::Filled,
            diff_qty,
            diff_qty,
            ts_now,
            ts_now,
            ts_now,
            None,
        )
        .with_avg_px(fill_px.to_f64().unwrap_or(0.0))
        .ok()?;

        if let Some(venue_position_id) = report.venue_position_id {
            order_report = order_report.with_venue_position_id(venue_position_id);
        }

        log::info!(
            color = LogColor::Blue as u8;
            "正在为 {instrument_id} 生成对账订单：side={:?}，qty={}，px={}",
            order_side,
            diff_qty,
            fill_px,
        );

        let (events, _) =
            self.handle_external_order(&order_report, account_id, instrument, &[], true);
        Some(events)
    }

    fn reconcile_order_report(
        &self,
        order: &OrderAny,
        report: &OrderStatusReport,
        instrument: Option<&InstrumentAny>,
    ) -> Option<OrderEventAny> {
        let ts_now = self.clock.borrow().timestamp_ns();
        reconcile_order_report(order, report, instrument, ts_now)
    }

    /// 以原子方式对账订单及其关联成交。
    ///
    /// 对于终端状态（已取消），成交会在终端事件之前应用，
    /// 以确保正确的状态转换（与 Python 行为一致）。
    fn reconcile_order_with_fills(
        &mut self,
        order: &mut OrderAny,
        report: &OrderStatusReport,
        fills: &[&FillReport],
        instrument: Option<&InstrumentAny>,
    ) -> Vec<OrderEventAny> {
        let mut events = Vec::new();
        let mut sorted_fills: Vec<&FillReport> = fills.to_vec();
        sorted_fills.sort_by_key(|f| f.ts_event);

        let ts_now = self.clock.borrow().timestamp_ns();

        match report.order_status {
            OrderStatus::Canceled => {
                // 如果设置了 ts_triggered，则生成 Triggered 事件（与 Python 行为一致）
                if report.ts_triggered.is_some() && order.status() != OrderStatus::Triggered {
                    events.push(create_reconciliation_triggered(order, report, ts_now));
                }

                // 无论当前状态如何，都在 Canceled 事件之前应用成交，
                // 因为订单可能包含我们尚未见到的部分成交
                if let Some(inst) = instrument {
                    for fill in &sorted_fills {
                        if let Some(event) = self.create_order_fill(order, fill, inst) {
                            events.push(event);
                        }
                    }
                }
                if let Some(event) = self.reconcile_order_report(order, report, instrument) {
                    events.push(event);
                }
            }
            OrderStatus::Expired => {
                // 如果设置了 ts_triggered，则生成 Triggered 事件（与 Python 行为一致）
                if report.ts_triggered.is_some() && order.status() != OrderStatus::Triggered {
                    events.push(create_reconciliation_triggered(order, report, ts_now));
                }

                // 在 Expired 事件之前应用成交（与 Canceled 相同）
                if let Some(inst) = instrument {
                    for fill in &sorted_fills {
                        if let Some(event) = self.create_order_fill(order, fill, inst) {
                            events.push(event);
                        }
                    }
                }
                if let Some(event) = self.reconcile_order_report(order, report, instrument) {
                    events.push(event);
                }
            }
            _ => {
                if let Some(event) = self.reconcile_order_report(order, report, instrument) {
                    events.push(event);
                }
                if let Some(inst) = instrument {
                    for fill in &sorted_fills {
                        if let Some(event) = self.create_order_fill(order, fill, inst) {
                            events.push(event);
                        }
                    }
                }
            }
        }

        events
    }

    fn handle_external_order(
        &mut self,
        report: &OrderStatusReport,
        account_id: &AccountId,
        instrument: &InstrumentAny,
        fills: &[&FillReport],
        is_synthetic: bool,
    ) -> (Vec<OrderEventAny>, Option<ExternalOrderMetadata>) {
        let (strategy_id, tags) =
            if let Some(claimed_strategy) = self.external_order_claims.get(&report.instrument_id) {
                let order_id = report
                    .client_order_id
                    .map_or_else(|| report.venue_order_id.to_string(), |id| id.to_string());
                log::info!(
                    color = LogColor::Blue as u8;
                    "策略 {} 申领了 {} 的外部订单 {}",
                    order_id,
                    report.instrument_id,
                    claimed_strategy,
                );
                (*claimed_strategy, None)
            } else {
                // 未申领的订单使用 EXTERNAL 策略 ID，并带有区分来源的标签
                let tag = if is_synthetic {
                    *TAG_RECONCILIATION
                } else {
                    *TAG_VENUE
                };
                (StrategyId::from("EXTERNAL"), Some(vec![tag]))
            };

        // 过滤未申领的交易平台订单（但不包括合成的对账订单）
        if self.config.filter_unclaimed_external && !is_synthetic {
            return (Vec::new(), None);
        }

        let client_order_id = report
            .client_order_id
            .unwrap_or_else(|| ClientOrderId::from(report.venue_order_id.as_str()));

        let ts_now = self.clock.borrow().timestamp_ns();

        let initialized = OrderInitialized::new(
            self.config.trader_id,
            strategy_id,
            report.instrument_id,
            client_order_id,
            report.order_side,
            report.order_type,
            report.quantity,
            report.time_in_force,
            report.post_only,
            report.reduce_only,
            false, // 报价数量 (quote_quantity)
            true,  // 对账 (reconciliation)
            UUID4::new(),
            ts_now,
            ts_now,
            report.price,
            report.trigger_price,
            report.trigger_type,
            report.limit_offset,
            report.trailing_offset,
            Some(report.trailing_offset_type),
            report.expire_time,
            report.display_qty,
            None, // emulation_trigger
            None, // trigger_instrument_id
            Some(report.contingency_type),
            report.order_list_id,
            report.linked_order_ids.clone(),
            report.parent_order_id,
            None, // 执行算法 ID (exec_algorithm_id)
            None, // 执行算法参数 (exec_algorithm_params)
            None, // 执行衍生 ID (exec_spawn_id)
            tags,
        );

        let events = vec![OrderEventAny::Initialized(initialized)];
        let order = match OrderAny::from_events(events) {
            Ok(order) => order,
            Err(e) => {
                log::error!("Failed to create order from report: {e}");
                return (Vec::new(), None);
            }
        };

        {
            let mut cache = self.cache.borrow_mut();
            if let Err(e) = cache.add_order(order.clone(), None, None, false) {
                log::error!("Failed to add external order to cache: {e}");
                return (Vec::new(), None);
            }

            if let Err(e) =
                cache.add_venue_order_id(&client_order_id, &report.venue_order_id, false)
            {
                log::warn!("Failed to add venue order ID index: {e}");
            }
        }

        log::info!(
            color = LogColor::Blue as u8;
            "已创建外部订单 {} ({})，标的为 {} [{}]",
            client_order_id,
            report.venue_order_id,
            report.instrument_id,
            report.order_status,
        );

        let ts_now = self.clock.borrow().timestamp_ns();

        // 为外部订单生成事件：首先是 Accepted，然后是成交（针对终端状态），
        // 最后是终端状态。这与 Python 的行为一致。
        let mut order_events =
            generate_external_order_status_events(&order, report, account_id, instrument, ts_now);

        if !fills.is_empty() {
            let mut cached_order = self.get_order(&client_order_id).unwrap();
            let mut sorted_fills: Vec<&FillReport> = fills.to_vec();
            sorted_fills.sort_by_key(|f| f.ts_event);

            match report.order_status {
                OrderStatus::Canceled | OrderStatus::Expired => {
                    let terminal_event = order_events.pop();
                    for fill in sorted_fills {
                        if let Some(fill_event) =
                            self.create_order_fill(&mut cached_order, fill, instrument)
                        {
                            order_events.push(fill_event);
                        }
                    }
                    if let Some(event) = terminal_event {
                        order_events.push(event);
                    }
                }
                OrderStatus::Filled | OrderStatus::PartiallyFilled => {
                    // 仅当最后一个事件是 Filled 事件（推断成交）时才弹出
                    if order_events
                        .last()
                        .is_some_and(|e| matches!(e, OrderEventAny::Filled(_)))
                    {
                        order_events.pop();
                    }

                    let mut real_fill_total = Decimal::ZERO;
                    for fill in &sorted_fills {
                        if let Some(fill_event) =
                            self.create_order_fill(&mut cached_order, fill, instrument)
                        {
                            real_fill_total += fill.last_qty.as_decimal();
                            order_events.push(fill_event);
                        }
                    }

                    let report_filled = report.filled_qty.as_decimal();
                    if real_fill_total < report_filled {
                        let diff_decimal = report_filled - real_fill_total;
                        if let Ok(diff) =
                            Quantity::from_decimal_dp(diff_decimal, instrument.size_precision())
                            && let Some(inferred_fill) = create_inferred_fill_for_qty(
                                &cached_order,
                                report,
                                account_id,
                                instrument,
                                diff,
                                ts_now,
                            )
                        {
                            order_events.push(inferred_fill);
                        }
                    }
                }
                _ => {}
            }
        }

        let metadata = ExternalOrderMetadata {
            client_order_id,
            venue_order_id: report.venue_order_id,
            instrument_id: report.instrument_id,
            strategy_id,
            ts_init: ts_now,
        };

        (order_events, Some(metadata))
    }

    /// 为首个生命周期不完整的标的调整成交（部分窗口）。
    ///
    /// 当历史成交不能完全解释当前持仓时（例如，回顾窗口从持仓中期开始），
    /// 这会创建合成成交以与交易平台持仓对齐。
    fn adjust_mass_status_fills(
        &self,
        mass_status: &ExecutionMassStatus,
    ) -> (
        IndexMap<VenueOrderId, OrderStatusReport>,
        IndexMap<VenueOrderId, Vec<FillReport>>,
    ) {
        let mut final_orders: IndexMap<VenueOrderId, OrderStatusReport> =
            mass_status.order_reports();
        let mut final_fills: IndexMap<VenueOrderId, Vec<FillReport>> = mass_status.fill_reports();

        let mut instruments_to_adjust = Vec::new();
        for (instrument_id, position_reports) in mass_status.position_reports() {
            if !self.should_reconcile_instrument(&instrument_id) {
                log::debug!(
                    "Skipping fill adjustment for {instrument_id}: not in reconciliation_instrument_ids"
                );
                continue;
            }

            // 跳过对冲模式标的（具有 venue_position_id），因为部分窗口
            // 调整假设每个标的只有一个净持仓
            let is_hedge_mode = position_reports
                .iter()
                .any(|r| r.venue_position_id.is_some());
            if is_hedge_mode {
                log::debug!(
                    "跳过 {} 的成交调整：对冲模式（具有 venue_position_id）",
                    instrument_id
                );
                continue;
            }

            if let Some(instrument) = self.get_instrument(&instrument_id) {
                instruments_to_adjust.push(instrument);
            } else {
                log::debug!("跳过 {} 的成交调整：缓存中未找到该标的", instrument_id);
            }
        }

        if instruments_to_adjust.is_empty() {
            return (final_orders, final_fills);
        }

        log_info!(
            "正在为具有持仓报告的 {} 个标的调整成交",
            instruments_to_adjust.len(),
            color = LogColor::Blue
        );

        for instrument in &instruments_to_adjust {
            let instrument_id = instrument.id();

            match process_mass_status_for_reconciliation(mass_status, instrument, None) {
                Ok(result) => {
                    final_orders.retain(|_, order| order.instrument_id != instrument_id);
                    final_fills.retain(|_, fills| {
                        fills
                            .first()
                            .is_none_or(|f| f.instrument_id != instrument_id)
                    });

                    for (venue_order_id, order) in result.orders {
                        final_orders.insert(venue_order_id, order);
                    }
                    for (venue_order_id, fills) in result.fills {
                        final_fills.insert(venue_order_id, fills);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to adjust fills for {instrument_id}: {e}");
                }
            }
        }

        log_info!(
            "调整后：{} 个订单，{} 个成交组",
            final_orders.len(),
            final_fills.len(),
            color = LogColor::Blue
        );

        (final_orders, final_fills)
    }

    /// 对订单报告进行去重，每个 venue_order_id 保留最先进的状态。
    ///
    /// 当一个批次中包含同一订单的多个报告时，我们保留 fill_qty 最高的报告（进度最快），
    /// 或者如果相等，则保留最趋近终端状态的报告。
    fn deduplicate_order_reports<'a>(
        &self,
        reports: impl Iterator<Item = &'a OrderStatusReport>,
    ) -> AHashMap<VenueOrderId, &'a OrderStatusReport> {
        let mut best_reports: AHashMap<VenueOrderId, &'a OrderStatusReport> = AHashMap::new();

        for report in reports {
            let dominated = best_reports
                .get(&report.venue_order_id)
                .is_some_and(|existing| self.is_more_advanced(existing, report));

            if !dominated {
                best_reports.insert(report.venue_order_id, report);
            }
        }

        best_reports
    }

    fn is_more_advanced(&self, a: &OrderStatusReport, b: &OrderStatusReport) -> bool {
        if a.filled_qty > b.filled_qty {
            return true;
        }
        if a.filled_qty < b.filled_qty {
            return false;
        }

        // 填充数量相等 - 比较状态（终端状态更先进）
        Self::status_priority(a.order_status) > Self::status_priority(b.order_status)
    }

    const fn status_priority(status: OrderStatus) -> u8 {
        match status {
            OrderStatus::Initialized | OrderStatus::Submitted | OrderStatus::Emulated => 0,
            OrderStatus::Released | OrderStatus::Denied => 1,
            OrderStatus::Accepted | OrderStatus::PendingUpdate | OrderStatus::PendingCancel => 2,
            OrderStatus::Triggered => 3,
            OrderStatus::PartiallyFilled => 4,
            OrderStatus::Canceled | OrderStatus::Expired | OrderStatus::Rejected => 5,
            OrderStatus::Filled => 6,
        }
    }

    fn is_exact_order_match(&self, order: &OrderAny, report: &OrderStatusReport) -> bool {
        order.status() == report.order_status
            && order.filled_qty() == report.filled_qty
            && !should_reconciliation_update(order, report)
    }

    fn create_order_fill(
        &mut self,
        order: &mut OrderAny,
        fill: &FillReport,
        instrument: &InstrumentAny,
    ) -> Option<OrderEventAny> {
        if self.processed_fills.contains_key(&fill.trade_id) {
            return None;
        }

        self.processed_fills
            .insert(fill.trade_id, order.client_order_id());

        Some(OrderEventAny::Filled(OrderFilled::new(
            order.trader_id(),
            order.strategy_id(),
            order.instrument_id(),
            order.client_order_id(),
            fill.venue_order_id,
            fill.account_id,
            fill.trade_id,
            fill.order_side,
            order.order_type(),
            fill.last_qty,
            fill.last_px,
            instrument.quote_currency(),
            fill.liquidity_side,
            fill.report_id,
            fill.ts_event,
            self.clock.borrow().timestamp_ns(),
            false,
            fill.venue_position_id,
            Some(fill.commission),
        )))
    }
}
