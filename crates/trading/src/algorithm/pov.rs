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

//! 成交量百分比 (POV / Percentage of Volume) 执行算法。
//!
//! POV 算法实时跟踪市场成交量，并按照指定的参与比例
//! (Participation Rate) 跟随市场节奏下单。与 TWAP/VWAP 的预设分配不同，
//! POV 是完全响应式的：市场活跃时多交易，市场冷清时少交易。
//!
//! # 参数
//!
//! 提交给此算法的订单必须包含 `exec_algorithm_params`，其中包含：
//! - `participation_rate`: 参与比例 (0.0 ~ 1.0)，例如 0.10 表示跟踪市场总量的 10%。
//! - `interval_secs`: 检查市场成交量并提交子订单的时间间隔（秒）。
//! - `max_intervals`: 最大检查次数（安全上限），防止无限等待。
//! - `randomization_enabled`（可选）: 是否启用随机性（默认 true）。
//! - `max_display_ratio`（可选）: 最大显示比例（默认 0.05，即 5%）。
//!   限制单次下单量不超过盘口最佳价量的指定比例。
//!
//! # 行业标准特性
//!
//! 本实现符合机构级 POV 执行标准，包含：
//! - **时间随机性**：每个间隔的实际执行时间会在基础间隔时间上添加 ±20% 的随机抖动
//! - **数量随机性**：对计算出的参与量添加微小扰动（±5%），避免模式识别
//! - **市场深度检查**：检查订单簿深度，避免在流动性薄弱时过度暴露
//!
//! # 工作原理
//!
//! 1. 算法在每个 `interval_secs` 间隔查询缓存中的 TradeTick 数据。
//! 2. 计算自上次检查以来的新增市场成交量。
//! 3. 按 `participation_rate` 比例计算本次应发送的数量。
//! 4. 应用随机性和市场深度检查。
//! 5. 提交子订单（不超过剩余总量）。
//! 6. 当总量全部下完或达到 `max_intervals` 时结束。
//!
//! # 注意
//!
//! POV 算法依赖缓存中的 TradeTick 数据来追踪市场成交量。
//! 如果没有行情数据订阅（即缓存中没有对应品种的 TradeTick），
//! 算法将在每个间隔记录警告并跳过，不会下单。
//!
//! # 示例
//!
//! 一个设置了 `participation_rate=0.10`、`interval_secs=5`、`max_intervals=100`
//! 的 1000 股订单：
//! - 每 5 秒检查一次市场成交量
//! - 如果这 5 秒内市场成交了 2000 股，算法下 200 股 (2000 × 10%)
//! - 如果这 5 秒内市场成交了 500 股，算法下 50 股 (500 × 10%)
//! - 如果没有成交，算法不下单
//! - 最多检查 100 次后终止

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
    identifiers::{ClientOrderId, InstrumentId},
    instruments::Instrument,
    orders::{Order, OrderAny},
    types::{Quantity, quantity::QuantityRaw},
};
use rand::RngExt;
use ustr::Ustr;

use super::{ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore};

/// [`PovAlgorithm`] 的配置。
pub type PovAlgorithmConfig = ExecutionAlgorithmConfig;

/// 每个主订单的 POV 跟踪状态。
#[derive(Debug)]
struct PovOrderState {
    /// 目标交易工具 ID。
    instrument_id: InstrumentId,
    /// 参与比例 (0.0 ~ 1.0)。
    participation_rate: f64,
    /// 剩余需执行的原始数量。
    remaining_raw: QuantityRaw,
    /// 数量精度。
    precision: u8,
    /// 上次检查时缓存中 TradeTick 的数量。
    last_trade_count: usize,
    /// 已执行的间隔计数。
    elapsed_intervals: u64,
    /// 最大间隔次数。
    max_intervals: u64,
    /// 是否启用随机性。
    randomization_enabled: bool,
    /// 最大显示比例。
    max_display_ratio: f64,
}

/// 成交量百分比 (POV) 执行算法。
///
/// 实时跟踪市场成交量，按固定的参与比例 (Participation Rate)
/// 响应式地下单。市场活跃时多交易，市场冷清时少交易。
#[derive(Debug)]
pub struct PovAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的跟踪状态。
    order_states: AHashMap<ClientOrderId, PovOrderState>,
}

impl PovAlgorithm {
    /// 创建一个新的 [`PovAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: PovAlgorithmConfig) -> Self {
        Self {
            core: ExecutionAlgorithmCore::new(config),
            order_states: AHashMap::new(),
        }
    }

    /// 完成主订单的执行序列。
    fn complete_sequence(&mut self, primary_id: &ClientOrderId) {
        let timer_name = primary_id.as_str();
        if self.core.clock().timer_names().contains(&timer_name) {
            self.core.clock().cancel_timer(timer_name);
        }
        if let Some(state) = self.order_states.remove(primary_id) {
            let remaining = Quantity::from_raw(state.remaining_raw, state.precision);
            log::info!(
                "完成 {primary_id} 的 POV 执行 (已执行 {} 个间隔, 剩余数量: {remaining})",
                state.elapsed_intervals
            );
        }
    }

    /// 应用时间随机性（±20% 抖动）。
    fn apply_time_randomization(base_interval_secs: f64) -> Duration {
        let mut rng = rand::rng();
        let jitter = 0.8 + rng.random_range(0.0..0.4);
        let randomized_secs = base_interval_secs * jitter;
        Duration::from_secs_f64(randomized_secs.max(1.0))
    }

    /// 应用数量随机性（±5% 扰动）。
    fn apply_quantity_randomization(
        base_qty_raw: QuantityRaw,
        remaining_raw: QuantityRaw,
        is_final: bool,
    ) -> QuantityRaw {
        if is_final {
            return remaining_raw;
        }

        let mut rng = rand::rng();
        let randomization_factor = 0.95 + rng.random_range(0.0..0.10);
        let randomized_raw = (base_qty_raw as f64 * randomization_factor).floor() as QuantityRaw;
        let randomized_raw = randomized_raw.max(1).min(remaining_raw);
        randomized_raw
    }

    /// 检查市场深度限制。
    fn check_market_depth_limit(
        cache: &nautilus_common::cache::Cache,
        instrument_id: &InstrumentId,
        proposed_qty_raw: QuantityRaw,
        max_display_ratio: f64,
        order_side: nautilus_model::enums::OrderSide,
    ) -> QuantityRaw {
        let Some(book) = cache.order_book(instrument_id) else {
            return proposed_qty_raw;
        };

        let depth_qty_raw = match order_side {
            nautilus_model::enums::OrderSide::Buy => book.best_ask_size().map(|s| s.raw).unwrap_or(proposed_qty_raw),
            nautilus_model::enums::OrderSide::Sell => book.best_bid_size().map(|s| s.raw).unwrap_or(proposed_qty_raw),
            _ => proposed_qty_raw,
        };

        let max_allowed = (depth_qty_raw as f64 * max_display_ratio).floor() as QuantityRaw;
        let limited_raw = std::cmp::min(proposed_qty_raw, max_allowed.max(1));
        limited_raw
    }
}

impl Deref for PovAlgorithm {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.core.actor
    }
}

impl DerefMut for PovAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core.actor
    }
}

impl DataActor for PovAlgorithm {}

impl ExecutionAlgorithm for PovAlgorithm {
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
        &mut self.core
    }

    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
        let primary_id = order.client_order_id();

        if self.order_states.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 POV 执行订单: {order:?}");

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

        // 解析 participation_rate
        let Some(rate_str) = exec_params.get(&Ustr::from("participation_rate")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 participation_rate");
            return Ok(());
        };

        let participation_rate: f64 = rate_str.parse().map_err(|e| {
            log::error!("无法解析 participation_rate: {e}");
            anyhow::anyhow!("无效的 participation_rate")
        })?;

        // 解析 interval_secs
        let Some(interval_secs_str) = exec_params.get(&Ustr::from("interval_secs")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 interval_secs");
            return Ok(());
        };

        let interval_secs: f64 = interval_secs_str.parse().map_err(|e| {
            log::error!("无法解析 interval_secs: {e}");
            anyhow::anyhow!("无效的 interval_secs")
        })?;

        // 解析 max_intervals
        let Some(max_intervals_str) = exec_params.get(&Ustr::from("max_intervals")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 max_intervals");
            return Ok(());
        };

        let max_intervals: u64 = max_intervals_str.parse().map_err(|e| {
            log::error!("无法解析 max_intervals: {e}");
            anyhow::anyhow!("无效的 max_intervals")
        })?;

        // 解析可选参数
        let randomization_enabled: bool = exec_params
            .get(&Ustr::from("randomization_enabled"))
            .map(|s| s == "true")
            .unwrap_or(true);

        let max_display_ratio: f64 = exec_params
            .get(&Ustr::from("max_display_ratio"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.05);

        // 验证参数有效性
        if !participation_rate.is_finite() || participation_rate <= 0.0 || participation_rate > 1.0 {
            log::error!(
                "无法执行订单：participation_rate={participation_rate} 必须在 (0.0, 1.0] 范围内"
            );
            return Ok(());
        }

        if !interval_secs.is_finite() || interval_secs <= 0.0 {
            log::error!("无法执行订单：interval_secs={interval_secs} 必须是有限且正数");
            return Ok(());
        }

        if max_intervals == 0 {
            log::error!("无法执行订单：max_intervals 必须大于 0");
            return Ok(());
        }

        let total_qty = order.quantity();

        // 检查最小数量约束
        if let Some(min_qty) = instrument.min_quantity() {
            if total_qty < min_qty {
                log::warn!(
                    "以全额提交：订单总量 {total_qty} 小于最小订单数量 {min_qty}"
                );
                self.submit_order(order, None, None)?;
                return Ok(());
            }
        }

        // 获取当前缓存中的 trade tick 数量作为基准
        let initial_trade_count = {
            let cache = self.core.cache();
            cache.trade_count(&order.instrument_id())
        };

        // 将主订单添加到缓存
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        // 初始化跟踪状态
        let state = PovOrderState {
            instrument_id: order.instrument_id(),
            participation_rate,
            remaining_raw: total_qty.raw,
            precision: total_qty.precision,
            last_trade_count: initial_trade_count,
            elapsed_intervals: 0,
            max_intervals,
            randomization_enabled,
            max_display_ratio,
        };

        self.order_states.insert(primary_id, state);

        // 设置带随机性的定时器
        let randomized_interval = if randomization_enabled {
            Self::apply_time_randomization(interval_secs)
        } else {
            Duration::from_secs_f64(interval_secs)
        };

        self.core.clock().set_timer(
            primary_id.as_str(),
            randomized_interval,
            None,
            None,
            None,
            None,
            None,
        )?;

        log::info!(
            "开始执行 {primary_id} 的 POV：participation_rate={participation_rate}, \
             interval_secs={interval_secs}, max_intervals={max_intervals}, \
             total_qty={total_qty}, randomization={}, max_display_ratio={:.1}%",
            if randomization_enabled { "enabled" } else { "disabled" },
            max_display_ratio * 100.0
        );

        Ok(())
    }

    fn on_time_event(&mut self, event: &TimeEvent) -> anyhow::Result<()> {
        log::info!("收到时间事件: {event:?}");

        let primary_id = ClientOrderId::new(event.name.as_str());

        // 检查主订单是否仍然活跃
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

        let Some(state) = self.order_states.get_mut(&primary_id) else {
            log::error!("找不到 exec_spawn_id={primary_id} 的 POV 状态");
            return Ok(());
        };

        state.elapsed_intervals += 1;

        // 检查是否已达到最大间隔
        if state.elapsed_intervals >= state.max_intervals {
            log::info!(
                "POV {primary_id} 已达最大间隔次数 {}，提交剩余全部数量",
                state.max_intervals
            );
            self.submit_order(primary, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 查询缓存中新增的 TradeTick 数量
        let (current_trade_count, new_market_volume_raw) = {
            let cache = self.core.cache();
            let current_count = cache.trade_count(&state.instrument_id);
            let new_ticks = current_count.saturating_sub(state.last_trade_count);

            if new_ticks == 0 {
                (current_count, 0u64)
            } else {
                // 获取 trade ticks 并计算新增成交量
                let volume_raw = cache
                    .trades(&state.instrument_id)
                    .map(|trades| {
                        // 取最后 new_ticks 笔 trade 的成交量总和
                        trades
                            .iter()
                            .rev()
                            .take(new_ticks)
                            .map(|t| t.size.raw as u64)
                            .sum::<u64>()
                    })
                    .unwrap_or(0);
                (current_count, volume_raw)
            }
        };

        // 更新上次观察的 trade count
        let participation_rate = state.participation_rate;
        let remaining_raw = state.remaining_raw;
        let precision = state.precision;
        state.last_trade_count = current_trade_count;

        // 如果没有新的市场成交，本间隔跳过
        if new_market_volume_raw == 0 {
            log::info!(
                "POV {primary_id} 间隔 {}: 无新增市场成交量，跳过本次下单",
                state.elapsed_intervals
            );
            return Ok(());
        }

        // 计算本次应发送的数量 = 市场成交量 × 参与比例
        let base_target_raw = (new_market_volume_raw as f64 * participation_rate).floor() as QuantityRaw;

        if base_target_raw == 0 {
            log::info!(
                "POV {primary_id} 间隔 {}: 计算的目标数量为 0，跳过本次下单",
                state.elapsed_intervals
            );
            return Ok(());
        }

        // 应用数量随机化
        let randomized_target_raw = if state.randomization_enabled {
            Self::apply_quantity_randomization(base_target_raw, remaining_raw, false)
        } else {
            base_target_raw
        };

        // 应用市场深度限制
        let depth_limited_raw = if state.max_display_ratio > 0.0 {
            let cache = self.core.cache();
            Self::check_market_depth_limit(
                &cache,
                &state.instrument_id,
                randomized_target_raw,
                state.max_display_ratio,
                primary.order_side(),
            )
        } else {
            randomized_target_raw
        };

        // 不超过剩余数量
        let slice_raw = std::cmp::min(depth_limited_raw, remaining_raw);
        let slice_qty = Quantity::from_raw(slice_raw, precision);
        let is_final = slice_raw >= remaining_raw;

        log::info!(
            "POV {primary_id} 间隔 {}: 市场成交量={}, base_target={}, randomized={}, \
             depth_limited={}, actual={}{}",
            state.elapsed_intervals,
            new_market_volume_raw,
            Quantity::from_raw(base_target_raw, precision),
            Quantity::from_raw(randomized_target_raw, precision),
            Quantity::from_raw(depth_limited_raw, precision),
            slice_qty,
            if is_final { " (最终切片)" } else { "" }
        );

        // 最终切片：提交主订单
        if is_final {
            self.submit_order(primary, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 更新剩余数量
        if let Some(state) = self.order_states.get_mut(&primary_id) {
            state.remaining_raw -= slice_raw;
        }

        // 生成子订单
        let tags = primary.tags().map(|t| t.to_vec());
        let time_in_force = primary.time_in_force();
        let reduce_only = primary.is_reduce_only();
        let mut primary = primary;
        let spawned = self.spawn_market(
            &mut primary,
            slice_qty,
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

        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        self.core.clock().cancel_timers();
        Ok(())
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.unsubscribe_all_strategy_events();
        self.core.reset();
        self.order_states.clear();
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

    fn create_pov_algorithm() -> PovAlgorithm {
        let unique_id = format!("POV-{}", UUID4::new());
        let config = PovAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        PovAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut PovAlgorithm) {
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

    fn add_instrument_to_cache(algo: &mut PovAlgorithm) {
        use nautilus_model::instruments::{InstrumentAny, stubs::crypto_perpetual_ethusdt};

        let instrument = crypto_perpetual_ethusdt();
        let cache_rc = algo.core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache
            .add_instrument(InstrumentAny::CryptoPerpetual(instrument))
            .unwrap();
    }

    fn create_pov_params(rate: &str, interval: &str, max: &str) -> IndexMap<Ustr, Ustr> {
        let mut params = IndexMap::new();
        params.insert(Ustr::from("participation_rate"), Ustr::from(rate));
        params.insert(Ustr::from("interval_secs"), Ustr::from(interval));
        params.insert(Ustr::from("max_intervals"), Ustr::from(max));
        params
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
            Some(ExecAlgorithmId::new("POV")),
            Some(params),
            None,
            None,
        ))
    }

    // ==================== 基础测试 ====================

    #[rstest]
    fn test_pov_creation() {
        let algo = create_pov_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("POV"));
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_pov_registration() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);
        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_pov_reset_clears_states() {
        let mut algo = create_pov_algorithm();

        algo.order_states.insert(
            ClientOrderId::new("O-001"),
            PovOrderState {
                instrument_id: InstrumentId::from("ETHUSDT-PERP.BINANCE"),
                participation_rate: 0.1,
                remaining_raw: 1000,
                precision: 1,
                last_trade_count: 0,
                elapsed_intervals: 0,
                max_intervals: 100,
                randomization_enabled: true,
                max_display_ratio: 0.05,
            },
        );

        assert!(!algo.order_states.is_empty());

        ExecutionAlgorithm::on_reset(&mut algo).unwrap();

        assert!(algo.order_states.is_empty());
    }

    // ==================== 参数验证测试 ====================

    #[rstest]
    fn test_pov_rejects_non_market_orders() {
        let mut algo = create_pov_algorithm();
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
            None,  // expire_time
            false, // post_only
            false, // reduce_only
            false, // quote_quantity
            None,  // display_qty
            None,  // emulation_trigger
            None,  // trigger_instrument_id
            None,  // contingency_type
            None,  // order_list_id
            None,  // linked_order_ids
            None,  // parent_order_id
            None,  // exec_algorithm_id
            None,  // exec_algorithm_params
            None,  // exec_spawn_id
            None,  // tags
            UUID4::new(),
            0.into(),
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_pov_rejects_missing_params() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        let order = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
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
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_pov_rejects_invalid_participation_rate_zero() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.0", "5", "100");
        let order = create_market_order_with_params(params);

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_pov_rejects_invalid_participation_rate_over_one() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("1.5", "5", "100");
        let order = create_market_order_with_params(params);

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_pov_rejects_negative_interval() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "-1", "100");
        let order = create_market_order_with_params(params);

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_pov_rejects_zero_max_intervals() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "0");
        let order = create_market_order_with_params(params);

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_pov_rejects_duplicate_order() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order1 = create_market_order_with_params(params.clone());
        let order2 = create_market_order_with_params(params);

        algo.on_order(order1).unwrap();
        let result = algo.on_order(order2);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已经在执行中"));
    }

    // ==================== 初始化测试 ====================

    #[rstest]
    fn test_pov_initializes_order_state() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order = create_market_order_with_params_and_qty(params, Quantity::from("1000.0"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let state = algo.order_states.get(&primary_id).unwrap();
        assert!((state.participation_rate - 0.10).abs() < f64::EPSILON);
        assert_eq!(state.max_intervals, 100);
        assert_eq!(state.elapsed_intervals, 0);
        assert_eq!(state.last_trade_count, 0); // 缓存中没有 trade ticks
    }

    #[rstest]
    fn test_pov_sets_timer() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        assert!(
            algo.core
                .clock()
                .timer_names()
                .contains(&primary_id.as_str())
        );
    }

    // ==================== 时间事件驱动测试 ====================

    #[rstest]
    fn test_pov_on_time_event_skips_when_no_market_volume() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 模拟定时器触发（没有市场成交数据）
        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        // 应该跳过（间隔+1 但没有下单）
        let state = algo.order_states.get(&primary_id).unwrap();
        assert_eq!(state.elapsed_intervals, 1);
    }

    #[rstest]
    fn test_pov_on_time_event_completes_when_primary_closed() {
        use nautilus_model::events::OrderCanceled;

        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert!(algo.order_states.contains_key(&primary_id));

        // 将主订单标记为已取消
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

        // 主订单已关闭，序列应完成
        assert!(algo.order_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_pov_on_time_event_submits_all_on_max_intervals() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // max_intervals = 2
        let params = create_pov_params("0.10", "5", "2");
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 第一个间隔
        let event1 = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event1).unwrap();

        // 第二个间隔 - 应该达到 max_intervals 并提交剩余全量
        let event2 = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event2).unwrap();

        // 序列应已完成
        assert!(algo.order_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_pov_on_stop_cancels_timers() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let params = create_pov_params("0.10", "5", "100");
        let order = create_market_order_with_params(params);

        algo.on_order(order).unwrap();

        ExecutionAlgorithm::on_stop(&mut algo).unwrap();

        assert!(algo.core.clock().timer_names().is_empty());
    }

    #[rstest]
    fn test_pov_allows_participation_rate_one() {
        let mut algo = create_pov_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // participation_rate = 1.0 是允许的 (100% 跟量)
        let params = create_pov_params("1.0", "5", "100");
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let state = algo.order_states.get(&primary_id).unwrap();
        assert!((state.participation_rate - 1.0).abs() < f64::EPSILON);
    }
}
