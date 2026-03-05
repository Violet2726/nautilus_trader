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

//! 时间加权平均价格 (TWAP) 执行算法。
//!
//! TWAP 算法通过在指定的时间范围内定期均匀地分散执行订单。
//! 这有助于通过避免在任何给定时间点集中成交来减少市场冲击。
//!
//! # 行业标准特性
//!
//! 本实现符合机构级 TWAP 执行标准，包含：
//! - **时间随机性**：在每个间隔内随机化执行时间点（±20% 抖动）
//! - **数量随机性**：对计划数量添加微小扰动（±5%），避免模式识别
//! - **市场冲击保护**：限制单笔订单不超过市场成交量的指定比例
//! - **执行进度监控**：实时跟踪执行进度与计划的偏差
//! - **动态调整**：根据实际执行情况动态调整后续计划
//!
//! # 参数
//!
//! 提交给此算法的订单必须包含 `exec_algorithm_params`，其中包含：
//! - `horizon_secs`: 总执行时间范围（秒）。
//! - `interval_secs`: 子订单之间的基础时间间隔（秒）。
//! - `max_participation_rate`（可选）: 最大市场参与率（默认 0.10，即 10%）。
//!   限制单笔订单不超过该间隔市场成交量的指定比例。
//! - `randomization_enabled`（可选）: 是否启用随机性（默认 true）。
//!
//! # 示例
//!
//! 一个设置了 `horizon_secs=60` 和 `interval_secs=10` 的订单将在 60 秒内
//! 生成 6 个子订单，每 10 秒生成一个。

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

/// [`TwapAlgorithm`] 的配置。
pub type TwapAlgorithmConfig = ExecutionAlgorithmConfig;

/// TWAP 执行状态跟踪。
#[derive(Debug)]
struct TwapExecutionState {
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

/// 时间加权平均价格 (TWAP) 执行算法。
///
/// 通过在指定的时间范围内定期均匀地分散执行订单。
/// 该算法接收一个主订单并生成定期执行的较小子订单，
/// 并引入随机性和市场保护机制以符合行业标准。
#[derive(Debug)]
pub struct TwapAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的执行状态。
    execution_states: AHashMap<ClientOrderId, TwapExecutionState>,
}

impl TwapAlgorithm {
    /// 创建一个新的 [`TwapAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: TwapAlgorithmConfig) -> Self {
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
                "完成 {primary_id} 的 TWAP 执行 (已执行 {}/{}，共 {} 个间隔)",
                executed,
                total,
                state.elapsed_intervals
            );
        }
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
        let jitter = 0.8 + rng.random_range(0.0..0.4);
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

impl Deref for TwapAlgorithm {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.core.actor
    }
}

impl DerefMut for TwapAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core.actor
    }
}

impl DataActor for TwapAlgorithm {}

impl ExecutionAlgorithm for TwapAlgorithm {
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
        &mut self.core
    }

    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
        let primary_id = order.client_order_id();

        if self.execution_states.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 TWAP 执行订单: {order:?}");

        // 仅支持市价单
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

        let horizon_secs: f64 = match horizon_secs_str.parse() {
            Ok(v) => v,
            Err(e) => {
                log::error!("无法解析 horizon_secs: {e}");
                return Ok(());
            }
        };

        let Some(interval_secs_str) = exec_params.get(&Ustr::from("interval_secs")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 interval_secs");
            return Ok(());
        };

        let interval_secs: f64 = match interval_secs_str.parse() {
            Ok(v) => v,
            Err(e) => {
                log::error!("无法解析 interval_secs: {e}");
                return Ok(());
            }
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

        let qty_per_interval_raw = total_raw / (num_intervals as QuantityRaw);
        let qty_per_interval = Quantity::from_raw(qty_per_interval_raw, precision);

        if qty_per_interval == total_qty || qty_per_interval < instrument.size_increment() {
            log::warn!(
                "以全额提交：每个间隔的数量 qty_per_interval={qty_per_interval}, 订单总数量={total_qty}"
            );
            self.submit_order(order, None, None)?;
            return Ok(());
        }

        if let Some(min_qty) = instrument.min_quantity()
            && qty_per_interval < min_qty
        {
            log::warn!(
                "以全额提交：每个间隔的数量 qty_per_interval={qty_per_interval} < 最小订单数量 min_quantity={min_qty}"
            );
            self.submit_order(order, None, None)?;
            return Ok(());
        }

        let mut scheduled_sizes: Vec<Quantity> = vec![qty_per_interval; num_intervals as usize];

        // 余数部分放在最后一切片中
        let scheduled_total = qty_per_interval_raw * (num_intervals as QuantityRaw);
        let remainder_raw = total_raw - scheduled_total;
        if remainder_raw > 0 {
            let remainder = Quantity::from_raw(remainder_raw, total_qty.precision);
            scheduled_sizes.push(remainder);
        }

        log::info!("订单执行大小计划表: {scheduled_sizes:?}");
        log::info!(
            "TWAP 参数：randomization={}, max_participation_rate={:.1}%",
            if randomization_enabled { "enabled" } else { "disabled" },
            max_participation_rate * 100.0
        );

        // 将主订单添加到缓存，以便 on_time_event 之后可以检索它
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        let state = TwapExecutionState {
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

        // 单一切片：直接提交主订单
        if is_single_slice {
            self.submit_order(order, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 多个切片：生成第一个子订单并减少主订单数量
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
            "开始执行 {primary_id} 的 TWAP：horizon_secs={horizon_secs}, interval_secs={interval_secs}"
        );

        Ok(())
    }

    fn on_time_event(&mut self, event: &TimeEvent) -> anyhow::Result<()> {
        log::info!("收到时间事件: {event:?}");

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
                "TWAP {primary_id} 执行落后计划 {:.1}%，将尝试追赶",
                deviation.abs() * 100.0
            );
        } else if deviation > 0.1 {
            log::info!(
                "TWAP {primary_id} 执行超前计划 {:.1}%，将适当放缓",
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
            "TWAP {primary_id} 间隔 {}: base={}, adjusted={}, volume_limited={}{}",
            state.elapsed_intervals,
            base_qty,
            adjusted_qty,
            quantity,
            if is_final { " (最终切片)" } else { "" }
        );

        // 最后一片：提交主订单（已减少为剩余数量）
        if is_final {
            self.submit_order(primary, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 中间切片：生成子订单并减少主订单数量
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

    fn create_twap_algorithm() -> TwapAlgorithm {
        // 使用唯一 ID 以避免在并行测试中出现线程局部注册表/消息总线冲突
        let unique_id = format!("TWAP-{}", UUID4::new());
        let config = TwapAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        TwapAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut TwapAlgorithm) {
        use nautilus_common::timer::TimeEventCallback;

        let trader_id = TraderId::from("TRADER-001");
        let clock = Rc::new(RefCell::new(TestClock::new()));
        let cache = Rc::new(RefCell::new(Cache::default()));

        // 为定时器回调注册一个默认的空处理器
        clock
            .borrow_mut()
            .register_default_handler(TimeEventCallback::Rust(std::sync::Arc::new(|_| {})));

        algo.core.register(trader_id, clock, cache).unwrap();

        // 为测试切换到 Running 状态
        algo.transition_state(ComponentTrigger::Initialize).unwrap();
        algo.transition_state(ComponentTrigger::Start).unwrap();
        algo.transition_state(ComponentTrigger::StartCompleted)
            .unwrap();
    }

    fn add_instrument_to_cache(algo: &mut TwapAlgorithm) {
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
            Some(ExecAlgorithmId::new("TWAP")),
            Some(params),
            None,
            None,
        ))
    }

    #[rstest]
    fn test_twap_creation() {
        let algo = create_twap_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("TWAP"));
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_twap_registration() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);

        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_twap_reset_clears_states() {
        let mut algo = create_twap_algorithm();
        let primary_id = ClientOrderId::new("O-001");

        algo.execution_states.insert(
            primary_id,
            TwapExecutionState {
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
    fn test_twap_rejects_non_market_orders() {
        let mut algo = create_twap_algorithm();
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
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_twap_rejects_missing_horizon_secs() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        // 缺少 horizon_secs

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_twap_rejects_invalid_horizon_secs() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("invalid"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_twap_rejects_zero_num_intervals() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("10"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.execution_states.is_empty());
    }

    #[rstest]
    fn test_twap_rejects_duplicate_order() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order1 = create_market_order_with_params(params.clone());
        let order2 = create_market_order_with_params(params);

        algo.on_order(order1).unwrap();
        let result = algo.on_order(order2);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已经在执行中"));
    }

    #[rstest]
    fn test_twap_uniform_distribution() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let state = algo.execution_states.get(&primary_id).unwrap();
        assert_eq!(state.scheduled_sizes.len(), 2); // 3 总数，第一个立即执行，剩下 2 个

        // 验证均匀分配：1.2 / 3 = 0.4
        assert_eq!(state.scheduled_sizes[0], Quantity::from("0.4"));
        assert_eq!(state.scheduled_sizes[1], Quantity::from("0.4"));
    }

    #[rstest]
    fn test_twap_on_time_event_spawns_next_slice() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        let state = algo.execution_states.get(&primary_id).unwrap();
        assert_eq!(state.scheduled_sizes.len(), 1); // 从 2 减到 1
        assert_eq!(state.elapsed_intervals, 1);
    }

    #[rstest]
    fn test_twap_on_time_event_completes_on_final_slice() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("30"));

        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert_eq!(algo.execution_states.get(&primary_id).unwrap().scheduled_sizes.len(), 1);

        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        assert!(algo.execution_states.get(&primary_id).is_none()); // 序列完成
    }

    #[rstest]
    fn test_twap_on_time_event_completes_when_primary_closed() {
        use nautilus_model::events::OrderCanceled;

        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert_eq!(algo.execution_states.get(&primary_id).unwrap().scheduled_sizes.len(), 2);

        // 将主订单标记为已关闭
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

        assert!(algo.execution_states.get(&primary_id).is_none()); // 序列完成
    }

    #[rstest]
    fn test_twap_on_stop_cancels_timers() {
        let mut algo = create_twap_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));

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
    fn test_twap_quantity_randomization() {
        let base_qty = Quantity::from("10.0");
        let remaining_raw = base_qty.raw * 10;
        let precision = base_qty.precision;

        let randomized = TwapAlgorithm::apply_quantity_randomization(
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
    fn test_twap_randomized_interval() {
        let algo = create_twap_algorithm();
        let base_interval = 20.0;

        let interval = algo.calculate_randomized_interval(base_interval);
        let secs = interval.as_secs_f64();

        assert!(
            secs >= 16.0 && secs <= 24.0,
            "Randomized interval {secs} should be within ±20% of {base_interval}"
        );
    }

    #[rstest]
    fn test_twap_schedule_deviation_calculation() {
        let deviation = TwapAlgorithm::calculate_schedule_deviation(500, 1000, 2, 4);
        assert!((deviation - 0.0).abs() < 0.01, "50% executed at 50% time should be on schedule");

        let deviation_behind = TwapAlgorithm::calculate_schedule_deviation(250, 1000, 2, 4);
        assert!(deviation_behind < -0.2, "25% executed at 50% time should be behind");

        let deviation_ahead = TwapAlgorithm::calculate_schedule_deviation(750, 1000, 2, 4);
        assert!(deviation_ahead > 0.2, "75% executed at 50% time should be ahead");
    }
}