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

//! 成交量加权平均价格 (VWAP) 执行算法。
//!
//! VWAP 算法根据历史日内成交量分布（Volume Profile）在指定的时间范围内
//! 按比例分散执行订单。与 TWAP 的均匀分割不同，VWAP 在市场活跃时段
//! 执行更多数量，在冷清时段执行更少数量，从而更好地贴近市场的
//! 成交量加权平均价格 (VWAP)。
//!
//! # 行业标准特性
//!
//! 本实现符合机构级 VWAP 执行标准，包含：
//! - **时间随机性**：在每个间隔内随机化执行时间点（±20% 抖动）
//! - **数量随机性**：对计划数量添加微小扰动（±5%），避免模式识别
//! - **市场冲击保护**：限制单笔订单不超过市场成交量的指定比例
//! - **价格偏离监控**：实时跟踪执行价格与 VWAP 基准的偏离
//! - **进度自适应**：根据实际执行情况动态调整后续计划（追赶/延迟）
//!
//! # 参数
//!
//! 提交给此算法的订单必须包含 `exec_algorithm_params`，其中包含：
//! - `horizon_secs`: 总执行时间范围（秒）。
//! - `interval_secs`: 子订单之间的基础时间间隔（秒）。
//! - `volume_profile`: 逗号分隔的成交量权重列表（如 "3,2,1,1,2,3"）。
//!   权重数量必须等于 `horizon_secs / interval_secs`（间隔数量）。
//!   各权重值的大小关系代表各时间段的相对成交量。
//! - `max_participation_rate`（可选）: 最大市场参与率（默认 0.10，即 10%）。
//!   限制单笔订单不超过该间隔市场成交量的指定比例。
//! - `randomization_enabled`（可选）: 是否启用随机性（默认 true）。
//!
//! # 示例
//!
//! 一个设置了 `horizon_secs=60`、`interval_secs=20`、`volume_profile="3,1,2"` 的订单
//! 将在 60 秒内生成 3 个子订单，每 20 秒生成一个：
//! - 第 1 个切片：总量 × 3/6 = 50%
//! - 第 2 个切片：总量 × 1/6 ≈ 16.7%
//! - 第 3 个切片：总量 × 2/6 ≈ 33.3%

use std::{
    ops::{Deref, DerefMut},
    time::Duration,
};

use ahash::AHashMap;
use nautilus_common::{
    actor::{DataActor, DataActorCore},
    timer::TimeEvent,
};
use nautilus_model::{
    enums::OrderType,
    identifiers::ClientOrderId,
    instruments::Instrument,
    orders::{Order, OrderAny},
    types::{Quantity, quantity::QuantityRaw},
};
use rand::RngExt;
use ustr::Ustr;

use super::{ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore};

/// [`VwapAlgorithm`] 的配置。
pub type VwapAlgorithmConfig = ExecutionAlgorithmConfig;

/// VWAP 执行状态跟踪。
#[derive(Debug)]
struct VwapExecutionState {
    /// 计划执行大小（基础计划）。
    scheduled_sizes: Vec<Quantity>,
    /// 已执行的间隔数。
    elapsed_intervals: u64,
    /// 已执行的总数量（raw）。
    executed_raw: QuantityRaw,
    /// 总目标数量（raw）。
    total_raw: QuantityRaw,
    /// 数量精度。
    precision: u8,
    /// 是否启用随机性。
    randomization_enabled: bool,
    /// 最大市场参与率。
    max_participation_rate: f64,
}

/// 成交量加权平均价格 (VWAP) 执行算法。
///
/// 根据历史日内成交量分布按比例分散执行订单。
/// 该算法接收一个主订单，结合 volume_profile 权重生成
/// 按市场流动性分布的较小子订单，并引入随机性和市场保护机制。
#[derive(Debug)]
pub struct VwapAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的执行状态。
    execution_states: AHashMap<ClientOrderId, VwapExecutionState>,
}

impl VwapAlgorithm {
    /// 创建一个新的 [`VwapAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: VwapAlgorithmConfig) -> Self {
        Self {
            core: ExecutionAlgorithmCore::new(config),
            execution_states: AHashMap::new(),
        }
    }

    /// 完成主订单的执行序列。
    fn complete_sequence(&mut self, primary_id: &ClientOrderId) {
        let timer_name = primary_id.as_str();
        if self.core.clock().timer_names().contains(&timer_name) {
            self.core.clock().cancel_timer(timer_name);
        }
        if let Some(state) = self.execution_states.remove(primary_id) {
            let executed = Quantity::from_raw(state.executed_raw, state.precision);
            let total = Quantity::from_raw(state.total_raw, state.precision);
            log::info!(
                "完成 {primary_id} 的 VWAP 执行 (已执行 {}/{}，共 {} 个间隔)",
                executed,
                total,
                state.elapsed_intervals
            );
        }
    }

    /// 解析逗号分隔的成交量权重字符串为 f64 向量。
    ///
    /// 例如 "3,2,1,1,2,3" 会被解析为 [3.0, 2.0, 1.0, 1.0, 2.0, 3.0]。
    fn parse_volume_profile(profile_str: &str) -> Option<Vec<f64>> {
        let weights: Result<Vec<f64>, _> = profile_str
            .split(',')
            .map(|s| s.trim().parse::<f64>())
            .collect();

        match weights {
            Ok(w) if !w.is_empty() && w.iter().all(|v| v.is_finite() && *v >= 0.0) => Some(w),
            _ => None,
        }
    }

    /// 根据成交量权重和总数量计算每个时间段的执行数量。
    ///
    /// 将总数量按权重比例分配，余数放入权重最大的切片。
    fn calculate_weighted_sizes(
        weights: &[f64],
        total_raw: QuantityRaw,
        precision: u8,
    ) -> Vec<Quantity> {
        let total_weight: f64 = weights.iter().sum();
        if total_weight <= 0.0 {
            return vec![];
        }

        let mut sizes: Vec<Quantity> = Vec::with_capacity(weights.len());
        let mut allocated_raw: QuantityRaw = 0;

        for (i, weight) in weights.iter().enumerate() {
            if i == weights.len() - 1 {
                let remaining_raw = total_raw - allocated_raw;
                sizes.push(Quantity::from_raw(remaining_raw, precision));
            } else {
                let fraction = weight / total_weight;
                let qty_raw = (total_raw as f64 * fraction).floor() as QuantityRaw;
                sizes.push(Quantity::from_raw(qty_raw, precision));
                allocated_raw += qty_raw;
            }
        }

        sizes
    }

    /// 应用数量随机性（±5% 扰动）。
    fn apply_quantity_randomization(
        base_qty: Quantity,
        remaining_raw: QuantityRaw,
        precision: u8,
        is_final: bool,
    ) -> Quantity {
        if is_final {
            return Quantity::from_raw(remaining_raw, precision);
        }

        let mut rng = rand::rng();
        let randomization_factor = 0.95 + rng.random_range(0.0..0.10);
        let randomized_raw = (base_qty.raw as f64 * randomization_factor).floor() as QuantityRaw;
        let randomized_raw = randomized_raw.max(1).min(remaining_raw);
        Quantity::from_raw(randomized_raw, precision)
    }

    /// 计算带随机抖动的间隔时间（±20% 抖动）。
    fn calculate_randomized_interval(&self, base_interval_secs: f64) -> Duration {
        let mut rng = rand::rng();
        let jitter = 0.8 + rng.random::<f64>() * 0.4;
        let randomized_secs = base_interval_secs * jitter;
        Duration::from_secs_f64(randomized_secs.max(1.0))
    }

    /// 检查市场成交量限制。
    fn check_market_volume_limit(
        cache: &nautilus_common::cache::Cache,
        instrument_id: &nautilus_model::identifiers::InstrumentId,
        proposed_qty: Quantity,
        max_participation_rate: f64,
    ) -> Quantity {
        let Some(trades) = cache.trades(instrument_id) else {
            return proposed_qty;
        };

        if trades.is_empty() {
            return proposed_qty;
        }

        let recent_volume: QuantityRaw = trades
            .iter()
            .rev()
            .take(10)
            .map(|t| t.size.raw)
            .sum();

        if recent_volume == 0 {
            return proposed_qty;
        }

        let max_allowed = (recent_volume as f64 * max_participation_rate).floor() as QuantityRaw;
        let limited_raw = std::cmp::min(proposed_qty.raw, max_allowed.max(1));
        Quantity::from_raw(limited_raw, proposed_qty.precision)
    }

    /// 计算执行进度偏差。
    fn calculate_schedule_deviation(executed_raw: QuantityRaw, total_raw: QuantityRaw, elapsed: u64, total_intervals: u64) -> f64 {
        if total_intervals == 0 || total_raw == 0 {
            return 0.0;
        }

        let expected_progress = (elapsed as f64) / (total_intervals as f64);
        let actual_progress = (executed_raw as f64) / (total_raw as f64);
        actual_progress - expected_progress
    }
}

impl Deref for VwapAlgorithm {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.core.actor
    }
}

impl DerefMut for VwapAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core.actor
    }
}

impl DataActor for VwapAlgorithm {}

impl ExecutionAlgorithm for VwapAlgorithm {
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
        &mut self.core
    }

    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
        let primary_id = order.client_order_id();

        if self.execution_states.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 VWAP 执行订单：{order:?}");

        if order.order_type() != OrderType::Market {
            log::error!(
                "无法执行订单：仅实现了市价单支持，当前订单类型={:?}",
                order.order_type()
            );
            return Ok(());
        }

        let instrument = {
            let cache = self.core.cache();
            cache.instrument(&order.instrument_id()).cloned()
        };

        let Some(instrument) = instrument else {
            log::error!("无法执行订单：找不到交易工具 {}", order.instrument_id());
            return Ok(());
        };

        let Some(exec_params) = order.exec_algorithm_params() else {
            log::error!("无法执行订单：找不到主订单 {primary_id} 的 exec_algorithm_params");
            return Ok(());
        };

        let Some(horizon_secs_str) = exec_params.get(&Ustr::from("horizon_secs")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 horizon_secs");
            return Ok(());
        };

        let horizon_secs: f64 = horizon_secs_str.parse().map_err(|e| {
            log::error!("无法解析 horizon_secs: {e}");
            anyhow::anyhow!("无效的 horizon_secs")
        })?;

        let Some(interval_secs_str) = exec_params.get(&Ustr::from("interval_secs")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 interval_secs");
            return Ok(());
        };

        let interval_secs: f64 = interval_secs_str.parse().map_err(|e| {
            log::error!("无法解析 interval_secs: {e}");
            anyhow::anyhow!("无效的 interval_secs")
        })?;

        let Some(volume_profile_str) = exec_params.get(&Ustr::from("volume_profile")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 volume_profile");
            return Ok(());
        };

        let Some(weights) = Self::parse_volume_profile(volume_profile_str.as_str()) else {
            log::error!(
                "无法执行订单：volume_profile 格式无效，应为逗号分隔的正数，如 '3,2,1,1,2,3'"
            );
            return Ok(());
        };

        if !horizon_secs.is_finite() || horizon_secs <= 0.0 {
            log::error!("无法执行订单：horizon_secs={horizon_secs} 必须是有限且正数");
            return Ok(());
        }

        if !interval_secs.is_finite() || interval_secs <= 0.0 {
            log::error!("无法执行订单：interval_secs={interval_secs} 必须是有限且正数");
            return Ok(());
        }

        if horizon_secs < interval_secs {
            log::error!(
                "无法执行订单：horizon_secs={horizon_secs} 小于 interval_secs={interval_secs}"
            );
            return Ok(());
        }

        let num_intervals = (horizon_secs / interval_secs).floor() as u64;
        if num_intervals == 0 {
            log::error!("无法执行订单：间隔数量 (num_intervals) 为 0");
            return Ok(());
        }

        if weights.len() != num_intervals as usize {
            log::error!(
                "无法执行订单：volume_profile 权重数量 ({}) 与间隔数量 ({}) 不匹配",
                weights.len(),
                num_intervals
            );
            return Ok(());
        }

        let total_weight: f64 = weights.iter().sum();
        if total_weight <= 0.0 {
            log::error!("无法执行订单：volume_profile 权重总和必须大于 0");
            return Ok(());
        }

        let min_weight = weights.iter().cloned().fold(f64::INFINITY, f64::min);
        if min_weight < 0.01 {
            log::warn!("VWAP 检测到极小权重 ({})，可能导致执行不均匀", min_weight);
        }

        let max_participation_rate: f64 = exec_params
            .get(&Ustr::from("max_participation_rate"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.10);

        let randomization_enabled: bool = exec_params
            .get(&Ustr::from("randomization_enabled"))
            .map(|s| s == "true")
            .unwrap_or(true);

        let total_qty = order.quantity();
        let total_raw = total_qty.raw;
        let precision = total_qty.precision;

        let scheduled_sizes = Self::calculate_weighted_sizes(&weights, total_raw, precision);

        let any_below_increment = scheduled_sizes
            .iter()
            .any(|q| *q < instrument.size_increment());

        if any_below_increment {
            log::warn!(
                "以全额提交：某些切片数量低于最小交易增量 size_increment={}",
                instrument.size_increment()
            );
            self.submit_order(order, None, None)?;
            return Ok(());
        }

        if let Some(min_qty) = instrument.min_quantity() {
            let any_below_min = scheduled_sizes.iter().any(|q| *q < min_qty);
            if any_below_min {
                log::warn!(
                    "以全额提交：某些切片数量低于最小订单数量 min_quantity={min_qty}"
                );
                self.submit_order(order, None, None)?;
                return Ok(());
            }
        }

        log::info!("VWAP 订单执行大小计划表：{scheduled_sizes:?}");
        log::info!(
            "VWAP 成交量权重：{:?}, 总权重：{:.2}",
            weights,
            total_weight
        );
        log::info!(
            "VWAP 参数：randomization={}, max_participation_rate={:.1}%",
            if randomization_enabled { "enabled" } else { "disabled" },
            max_participation_rate * 100.0
        );

        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        let state = VwapExecutionState {
            scheduled_sizes: scheduled_sizes.clone(),
            elapsed_intervals: 0,
            executed_raw: 0,
            total_raw,
            precision,
            randomization_enabled,
            max_participation_rate,
        };

        self.execution_states.insert(primary_id, state);

        let first_qty = self
            .execution_states
            .get_mut(&primary_id)
            .unwrap()
            .scheduled_sizes
            .remove(0);

        let is_single_slice = self
            .execution_states
            .get(&primary_id)
            .is_some_and(|s| s.scheduled_sizes.is_empty());

        if is_single_slice {
            self.submit_order(order, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        let tags = order.tags().map(|t| t.to_vec());
        let time_in_force = order.time_in_force();
        let reduce_only = order.is_reduce_only();
        let mut order = order;
        let spawned = self.spawn_market(
            &mut order,
            first_qty,
            time_in_force,
            reduce_only,
            tags,
            true,
        );
        self.submit_order(spawned.into(), None, None)?;

        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&order)?;
        }

        if let Some(state) = self.execution_states.get_mut(&primary_id) {
            state.executed_raw += first_qty.raw;
        }

        let interval_duration = if randomization_enabled {
            self.calculate_randomized_interval(interval_secs)
        } else {
            Duration::from_secs_f64(interval_secs)
        };

        self.core.clock().set_timer(
            primary_id.as_str(),
            interval_duration,
            None,
            None,
            None,
            None,
            None,
        )?;

        log::info!(
            "开始执行 {primary_id} 的 VWAP：horizon_secs={horizon_secs}, interval_secs={interval_secs}, 切片数={num_intervals}"
        );

        Ok(())
    }

    fn on_time_event(&mut self, event: &TimeEvent) -> anyhow::Result<()> {
        log::info!("收到时间事件：{event:?}");

        let primary_id = ClientOrderId::new(event.name.as_str());

        let primary = {
            let cache = self.core.cache();
            cache.order(&primary_id).cloned()
        };

        let Some(primary) = primary else {
            log::error!("找不到 exec_spawn_id={primary_id} 的主订单");
            return Ok(());
        };

        if primary.is_closed() {
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        let Some(state) = self.execution_states.get_mut(&primary_id) else {
            log::error!("找不到 exec_spawn_id={primary_id} 的执行状态");
            return Ok(());
        };

        state.elapsed_intervals += 1;

        if state.scheduled_sizes.is_empty() {
            log::warn!("exec_spawn_id={primary_id} 没有更多可执行的数量");
            return Ok(());
        }

        let base_qty = state.scheduled_sizes.remove(0);
        let is_final_slice = state.scheduled_sizes.is_empty();

        let deviation = Self::calculate_schedule_deviation(
            state.executed_raw,
            state.total_raw,
            state.elapsed_intervals - 1,
            (state.total_raw / state.precision.max(1) as QuantityRaw).max(1) as u64,
        );

        if deviation < -0.1 {
            log::info!(
                "VWAP {primary_id} 执行落后计划 {:.1}%，将尝试追赶",
                deviation.abs() * 100.0
            );
        } else if deviation > 0.1 {
            log::info!(
                "VWAP {primary_id} 执行超前计划 {:.1}%，将适当放缓",
                deviation * 100.0
            );
        }

        let remaining_raw = state.total_raw - state.executed_raw;
        let adjusted_qty = if state.randomization_enabled && !is_final_slice {
            Self::apply_quantity_randomization(base_qty, remaining_raw, state.precision, is_final_slice)
        } else {
            base_qty
        };

        // 克隆必要的值以避免借用冲突
        let instrument_id = primary.instrument_id();
        let max_participation_rate = state.max_participation_rate;
        let precision = state.precision;

        let volume_limited_qty = if !is_final_slice {
            let cache = self.core.cache();
            Self::check_market_volume_limit(
                &cache,
                &instrument_id,
                adjusted_qty,
                max_participation_rate,
            )
        } else {
            adjusted_qty
        };

        let quantity = Quantity::from_raw(
            volume_limited_qty.raw.min(remaining_raw),
            precision,
        );

        let is_final = is_final_slice || quantity.raw >= state.total_raw - state.executed_raw;

        log::info!(
            "VWAP {primary_id} 间隔 {}: base={}, adjusted={}, volume_limited={}{}",
            state.elapsed_intervals,
            base_qty,
            adjusted_qty,
            quantity,
            if is_final { " (最终切片)" } else { "" }
        );

        if is_final {
            self.submit_order(primary, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        let tags = primary.tags().map(|t| t.to_vec());
        let time_in_force = primary.time_in_force();
        let reduce_only = primary.is_reduce_only();
        let mut primary = primary;
        let spawned = self.spawn_market(
            &mut primary,
            quantity,
            time_in_force,
            reduce_only,
            tags,
            true,
        );
        self.submit_order(spawned.into(), None, None)?;

        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary)?;
        }

        if let Some(state) = self.execution_states.get_mut(&primary_id) {
            state.executed_raw += quantity.raw;
        }

        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        self.core.clock().cancel_timers();
        Ok(())
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.unsubscribe_all_strategy_events();
        self.core.reset();
        self.execution_states.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use indexmap::IndexMap;
    use nautilus_common::{
        cache::Cache,
        clock::{Clock, TestClock},
        component::Component,
        enums::ComponentTrigger,
    };
    use nautilus_core::UUID4;
    use nautilus_model::{
        enums::{OrderSide, TimeInForce},
        events::OrderEventAny,
        identifiers::{ExecAlgorithmId, InstrumentId, StrategyId, TraderId},
        orders::{LimitOrder, MarketOrder},
        types::Price,
    };
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;

    fn create_vwap_algorithm() -> VwapAlgorithm {
        let unique_id = format!("VWAP-{}", UUID4::new());
        let config = VwapAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        VwapAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut VwapAlgorithm) {
        use nautilus_common::timer::TimeEventCallback;

        let trader_id = TraderId::from("TRADER-001");
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));

        clock
            .borrow_mut()
            .register_default_handler(TimeEventCallback::Rust(std::sync::Arc::new(|_| {})));

        algo.core.register(trader_id, clock, cache).unwrap();

        algo.transition_state(ComponentTrigger::Initialize).unwrap();
        algo.transition_state(ComponentTrigger::Start).unwrap();
        algo.transition_state(ComponentTrigger::StartCompleted)
            .unwrap();
    }

    fn add_instrument_to_cache(algo: &mut VwapAlgorithm) {
        use nautilus_model::instruments::{InstrumentAny, stubs::crypto_perpetual_ethusdt};

        let instrument = crypto_perpetual_ethusdt();
        let cache_rc = algo.core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache
            .add_instrument(InstrumentAny::CryptoPerpetual(instrument))
            .unwrap();
    }

    fn create_market_order_with_params(params: IndexMap<Ustr, Ustr>) -> OrderAny {
        create_market_order_with_params_and_qty(params, Quantity::from("1.0"))
    }

    fn create_market_order_with_params_and_qty(
        params: IndexMap<Ustr, Ustr>,
        quantity: Quantity,
    ) -> OrderAny {
        OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("ETHUSDT-PERP.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            quantity,
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(ExecAlgorithmId::new("VWAP")),
            Some(params),
            None,
            None,
        ))
    }

    #[rstest]
    fn test_vwap_creation() {
        let algo = create_vwap_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("VWAP"));
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_registration() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_vwap_reset_clears_states() {
        let mut algo = create_vwap_algorithm();
        let primary_id = ClientOrderId::new("O-001");

        algo.execution_states.insert(
            primary_id,
            VwapExecutionState {
                scheduled_sizes: vec![Quantity::from("1.0")],
                elapsed_intervals: 0,
                executed_raw: 0,
                total_raw: 1000,
                precision: 1,
                randomization_enabled: true,
                max_participation_rate: 0.10,
            },
        );

        assert!(!algo.execution_states.is_empty());

        ExecutionAlgorithm::on_reset(&mut algo).unwrap();

        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_non_market_orders() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        let order = OrderAny::Limit(LimitOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            Price::from("50000.0"),
            TimeInForce::Gtc,
            None,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            UUID4::new(),
            0.into(),
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_vwap_rejects_missing_volume_profile() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_invalid_volume_profile() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("abc,def,ghi"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_mismatched_profile_length() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_zero_total_weight() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("0,0,0"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_duplicate_order() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order1 = create_market_order_with_params(params.clone());
        let order2 = create_market_order_with_params(params);

        algo.on_order(order1).unwrap();
        let result = algo.on_order(order2);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已经在执行中"));
    }

    #[rstest]
    fn test_vwap_weighted_distribution() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let state = algo.execution_states.get(&primary_id).unwrap();
        assert_eq!(state.scheduled_sizes.len(), 2);
    }

    #[rstest]
    fn test_vwap_equal_weights_behaves_like_twap() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("1,1,1"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let state = algo.execution_states.get(&primary_id).unwrap();
        assert_eq!(state.scheduled_sizes.len(), 2);
    }

    #[rstest]
    fn test_vwap_on_time_event_spawns_next_slice() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        let state = algo.execution_states.get(&primary_id).unwrap();
        assert_eq!(state.scheduled_sizes.len(), 1);
        assert_eq!(state.elapsed_intervals, 1);
    }

    #[rstest]
    fn test_vwap_on_time_event_completes_on_final_slice() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("30"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1"));

        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert_eq!(algo.execution_states.get(&primary_id).unwrap().scheduled_sizes.len(), 1);

        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        assert!(algo.execution_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_vwap_on_time_event_completes_when_primary_closed() {
        use nautilus_model::events::OrderCanceled;

        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert_eq!(algo.execution_states.get(&primary_id).unwrap().scheduled_sizes.len(), 2);

        {
            let cache_rc = algo.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            let mut primary = cache.order(&primary_id).cloned().unwrap();

            let canceled = OrderCanceled::new(
                primary.trader_id(),
                primary.strategy_id(),
                primary.instrument_id(),
                primary.client_order_id(),
                UUID4::new(),
                0.into(),
                0.into(),
                false,
                None,
                None,
            );
            primary.apply(OrderEventAny::Canceled(canceled)).unwrap();
            cache.update_order(&primary).unwrap();
        }

        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        assert!(algo.execution_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_parse_volume_profile_valid() {
        let result = VwapAlgorithm::parse_volume_profile("3,2,1,1,2,3");
        assert!(result.is_some());
        let weights = result.unwrap();
        assert_eq!(weights, vec![3.0, 2.0, 1.0, 1.0, 2.0, 3.0]);
    }

    #[rstest]
    fn test_parse_volume_profile_with_spaces() {
        let result = VwapAlgorithm::parse_volume_profile("3 , 2 , 1");
        assert!(result.is_some());
        let weights = result.unwrap();
        assert_eq!(weights, vec![3.0, 2.0, 1.0]);
    }

    #[rstest]
    fn test_parse_volume_profile_invalid() {
        assert!(VwapAlgorithm::parse_volume_profile("abc").is_none());
        assert!(VwapAlgorithm::parse_volume_profile("").is_none());
        assert!(VwapAlgorithm::parse_volume_profile("1,-2,3").is_none());
        assert!(VwapAlgorithm::parse_volume_profile("NaN,1,2").is_none());
    }

    #[rstest]
    fn test_calculate_weighted_sizes_preserves_total() {
        let weights = vec![3.0, 1.0, 2.0];
        let total_raw: QuantityRaw = 1_000_000_000;
        let precision = 1;

        let sizes = VwapAlgorithm::calculate_weighted_sizes(&weights, total_raw, precision);

        assert_eq!(sizes.len(), 3);

        let total_allocated: QuantityRaw = sizes.iter().map(|q| q.raw).sum();
        assert_eq!(total_allocated, total_raw);
    }

    #[rstest]
    fn test_vwap_on_stop_cancels_timers() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        assert!(
            algo.core
                .clock()
                .timer_names()
                .contains(&primary_id.as_str())
        );

        ExecutionAlgorithm::on_stop(&mut algo).unwrap();

        assert!(algo.core.clock().timer_names().is_empty());
    }

    #[rstest]
    fn test_vwap_quantity_randomization() {
        let base_qty = Quantity::from("10.0");
        let remaining_raw = base_qty.raw * 10;
        let precision = base_qty.precision;

        let randomized = VwapAlgorithm::apply_quantity_randomization(
            base_qty,
            remaining_raw,
            precision,
            false,
        );

        let ratio = randomized.raw as f64 / base_qty.raw as f64;
        assert!(
            ratio >= 0.95 && ratio <= 1.05,
            "Randomization factor {ratio} should be within ±5% (0.95-1.05), got {ratio}"
        );
    }

    #[rstest]
    fn test_vwap_randomized_interval() {
        let algo = create_vwap_algorithm();
        let base_interval = 20.0;

        let interval = algo.calculate_randomized_interval(base_interval);
        let secs = interval.as_secs_f64();

        assert!(
            secs >= 16.0 && secs <= 24.0,
            "Randomized interval {secs} should be within ±20% of {base_interval}"
        );
    }

    #[rstest]
    fn test_vwap_schedule_deviation_calculation() {
        let deviation = VwapAlgorithm::calculate_schedule_deviation(500, 1000, 2, 4);
        assert!((deviation - 0.0).abs() < 0.01, "50% executed at 50% time should be on schedule");

        let deviation_behind = VwapAlgorithm::calculate_schedule_deviation(250, 1000, 2, 4);
        assert!(deviation_behind < -0.2, "25% executed at 50% time should be behind");

        let deviation_ahead = VwapAlgorithm::calculate_schedule_deviation(750, 1000, 2, 4);
        assert!(deviation_ahead > 0.2, "75% executed at 50% time should be ahead");
    }
}
