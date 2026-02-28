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

//! 执行缺口 (Implementation Shortfall / IS) 执行算法。
//!
//! IS 算法的目标是最小化"执行缺口"——也就是**决策价格**（Arrival Price，
//! 即决定交易时的市场价格）与**实际成交均价**之间的差距。
//!
//! 它通过在**市场冲击成本**和**时机风险**之间寻找最佳平衡来实现这一目标：
//! - 执行太快 → 大额订单冲击盘口，市场冲击成本高。
//! - 执行太慢 → 价格可能不利变动，时机风险高。
//!
//! # 参数
//!
//! 提交给此算法的订单必须包含 `exec_algorithm_params`，其中包含：
//! - `horizon_secs`: 总执行时间范围（秒）。
//! - `interval_secs`: 子订单之间的时间间隔（秒）。
//! - `urgency`: 紧迫度参数 (0.0 ~ 1.0)，控制执行节奏的前倾程度：
//!   - 接近 0.0：接近均匀分配（类似 TWAP），最小化市场冲击
//!   - 接近 1.0：极度前倾，尽快完成，最小化时机风险
//! - `arrival_price`（可选）：决策价格。如果不提供，算法将使用
//!   收到订单时缓存中的最新成交价 (`PriceType::Last`)。
//!
//! # 工作原理
//!
//! 1. **紧迫度加权调度**：根据 `urgency` 参数生成一个前倾的执行调度表。
//!    权重公式为 `w_i = (N - i)^(urgency × 2)`，`urgency` 越大越前倾。
//!
//! 2. **时间随机性**：每个间隔的实际执行时间会在基础间隔时间上添加 ±20% 的随机抖动，
//!    避免被市场参与者识别出规律性。
//!
//! 3. **价格自适应**：每个间隔检查当前市场价格与到达价格的偏差。
//!    - 价格有利时（买入时价格下跌/卖出时价格上涨），增加执行量（最多 +50%）。
//!    - 价格不利时，减少执行量（最多 -50%）。
//!
//! 4. **市场冲击保护**：检查订单簿深度，确保单个切片不超过盘口最佳价量的一定比例（默认 5%），
//!    避免在流动性薄弱时过度暴露。
//!
//! 5. **兜底机制**：最后一个间隔无论如何都会提交剩余全部数量。
//!
//! # 示例
//!
//! 一个 1000 股的买入订单，`urgency=0.7`、`horizon_secs=60`、`interval_secs=20`：
//! - 紧迫度加权调度（前倾）：约 46%、33%、21%
//! - 第 1 个间隔（最初 20 秒）：下 ~460 股
//! - 如果价格有利（低于到达价）：增量下更多
//! - 如果价格不利（高于到达价）：减量下更少
//! - 最终间隔：下剩余全部

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
    enums::{OrderSide, OrderType, PriceType},
    identifiers::{ClientOrderId, InstrumentId},
    instruments::Instrument,
    orders::{Order, OrderAny},
    types::{Quantity, quantity::QuantityRaw},
};
use rand::RngExt;
use ustr::Ustr;

use super::{ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore};

/// [`IsAlgorithm`] 的配置。
pub type IsAlgorithmConfig = ExecutionAlgorithmConfig;

/// 每个主订单的 IS 跟踪状态。
#[derive(Debug)]
struct IsOrderState {
    /// 目标交易工具 ID。
    instrument_id: InstrumentId,
    /// 订单方向（买/卖），用于判断价格有利/不利。
    order_side: OrderSide,
    /// 到达价格（决策价格），作为执行基准。
    arrival_price: f64,
    /// 剩余需执行的原始数量。
    remaining_raw: QuantityRaw,
    /// 数量精度。
    precision: u8,
    /// 预计算的执行调度表（每个间隔的目标量）。
    scheduled_sizes: Vec<Quantity>,
    /// 已执行的间隔计数。
    elapsed_intervals: u64,
    /// 是否启用随机性。
    randomization_enabled: bool,
    /// 市场冲击保护比例。
    max_market_impact_ratio: f64,
}

/// 执行缺口 (IS) 执行算法。
///
/// 基于 Almgren-Chriss 框架的简化实现，通过紧迫度参数和
/// 实时价格自适应在市场冲击与时机风险之间寻求最优平衡。
#[derive(Debug)]
pub struct IsAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的跟踪状态。
    order_states: AHashMap<ClientOrderId, IsOrderState>,
}

impl IsAlgorithm {
    /// 创建一个新的 [`IsAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: IsAlgorithmConfig) -> Self {
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
                "完成 {primary_id} 的 IS 执行 (已执行 {} 个间隔, 剩余数量: {remaining})",
                state.elapsed_intervals
            );
        }
    }

    /// 基于紧迫度参数生成前倾的执行调度表。
    ///
    /// 权重公式: `w_i = (N - i) ^ (urgency * 2)`
    /// - `urgency → 0`: 权重接近均匀（类似 TWAP）
    /// - `urgency → 1`: 重度前倾，前面的切片远大于后面的
    ///
    /// 返回每个间隔的 Quantity（总和精确等于 total_raw）。
    fn generate_urgency_schedule(
        num_intervals: u64,
        urgency: f64,
        total_raw: QuantityRaw,
        precision: u8,
    ) -> Vec<Quantity> {
        let n = num_intervals as usize;
        let exponent = urgency * 2.0;

        // 计算前倾权重
        let weights: Vec<f64> = (0..n)
            .map(|i| ((n - i) as f64).powf(exponent))
            .collect();

        let total_weight: f64 = weights.iter().sum();
        if total_weight <= 0.0 {
            return vec![Quantity::from_raw(total_raw, precision)];
        }

        let mut sizes: Vec<Quantity> = Vec::with_capacity(n);
        let mut allocated_raw: QuantityRaw = 0;

        for (i, weight) in weights.iter().enumerate() {
            if i == n - 1 {
                // 最后一个切片获得所有剩余数量
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

    /// 计算价格自适应因子。
    ///
    /// - 买入时：价格低于到达价 → 有利 → 因子 > 1.0
    /// - 卖出时：价格高于到达价 → 有利 → 因子 > 1.0
    /// - 返回值被限制在 [0.5, 1.5] 范围内，防止极端调整。
    fn price_adaptation_factor(
        arrival_price: f64,
        current_price: f64,
        order_side: OrderSide,
    ) -> f64 {
        if arrival_price <= 0.0 {
            return 1.0;
        }

        // 计算价格偏差（基点）
        let deviation_bps = match order_side {
            OrderSide::Buy => (arrival_price - current_price) / arrival_price * 10000.0,
            OrderSide::Sell => (current_price - arrival_price) / arrival_price * 10000.0,
            _ => 0.0,
        };

        // 灵敏度：每偏离 100bp 调整 50%
        let sensitivity = 0.005; // 0.5% per bp
        let factor = 1.0 + deviation_bps * sensitivity;

        factor.clamp(0.5, 1.5)
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

    /// 检查市场冲击限制。
    fn check_market_impact_limit(
        cache: &nautilus_common::cache::Cache,
        instrument_id: &InstrumentId,
        proposed_qty_raw: QuantityRaw,
        max_impact_ratio: f64,
        order_side: OrderSide,
    ) -> QuantityRaw {
        let Some(book) = cache.order_book(instrument_id) else {
            return proposed_qty_raw;
        };

        let depth_qty_raw = match order_side {
            OrderSide::Buy => book.best_ask_size().map(|s| s.raw).unwrap_or(proposed_qty_raw),
            OrderSide::Sell => book.best_bid_size().map(|s| s.raw).unwrap_or(proposed_qty_raw),
            _ => proposed_qty_raw,
        };

        let max_allowed = (depth_qty_raw as f64 * max_impact_ratio).floor() as QuantityRaw;
        let limited_raw = std::cmp::min(proposed_qty_raw, max_allowed.max(1));
        limited_raw
    }
}

impl Deref for IsAlgorithm {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.core.actor
    }
}

impl DerefMut for IsAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core.actor
    }
}

impl DataActor for IsAlgorithm {}

impl ExecutionAlgorithm for IsAlgorithm {
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
        &mut self.core
    }

    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
        let primary_id = order.client_order_id();

        if self.order_states.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 IS 执行订单: {order:?}");

        // 仅支持市价单
        if order.order_type() != OrderType::Market {
            log::error!(
                "无法执行订单：IS 仅实现了市价单支持，当前订单类型={:?}",
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

        // 解析 horizon_secs
        let Some(horizon_secs_str) = exec_params.get(&Ustr::from("horizon_secs")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 horizon_secs");
            return Ok(());
        };
        let horizon_secs: f64 = horizon_secs_str.parse().map_err(|e| {
            log::error!("无法解析 horizon_secs: {e}");
            anyhow::anyhow!("无效的 horizon_secs")
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

        // 解析 urgency
        let Some(urgency_str) = exec_params.get(&Ustr::from("urgency")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 urgency");
            return Ok(());
        };
        let urgency: f64 = urgency_str.parse().map_err(|e| {
            log::error!("无法解析 urgency: {e}");
            anyhow::anyhow!("无效的 urgency")
        })?;

        // 解析 arrival_price（可选，默认使用当前市场价）
        let arrival_price: f64 = if let Some(ap_str) = exec_params.get(&Ustr::from("arrival_price"))
        {
            ap_str.parse().map_err(|e| {
                log::error!("无法解析 arrival_price: {e}");
                anyhow::anyhow!("无效的 arrival_price")
            })?
        } else {
            // 尝试从缓存获取当前价格
            let cached_price = {
                let cache = self.core.cache();
                cache.price(&order.instrument_id(), PriceType::Last)
            };
            match cached_price {
                Some(p) => p.as_f64(),
                None => {
                    log::error!(
                        "无法执行订单：未提供 arrival_price 且缓存中无 {} 的最新价格",
                        order.instrument_id()
                    );
                    return Ok(());
                }
            }
        };

        // 解析可选参数
        let randomization_enabled: bool = exec_params
            .get(&Ustr::from("randomization_enabled"))
            .map(|s| s == "true")
            .unwrap_or(true);

        let max_market_impact_ratio: f64 = exec_params
            .get(&Ustr::from("max_market_impact_ratio"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.05);

        // 验证参数有效性
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
        if !urgency.is_finite() || urgency < 0.0 || urgency > 1.0 {
            log::error!("无法执行订单：urgency={urgency} 必须在 [0.0, 1.0] 范围内");
            return Ok(());
        }
        if !arrival_price.is_finite() || arrival_price <= 0.0 {
            log::error!("无法执行订单：arrival_price={arrival_price} 必须是有限且正数");
            return Ok(());
        }

        let num_intervals = (horizon_secs / interval_secs).floor() as u64;
        if num_intervals == 0 {
            log::error!("无法执行订单：间隔数量 (num_intervals) 为 0");
            return Ok(());
        }

        let total_qty = order.quantity();
        let total_raw = total_qty.raw;
        let precision = total_qty.precision;
        let order_side = order.order_side();

        // 生成紧迫度加权调度表
        let scheduled_sizes =
            Self::generate_urgency_schedule(num_intervals, urgency, total_raw, precision);

        // 检查最小数量约束
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

        log::info!("IS 紧迫度加权调度表：{scheduled_sizes:?}");
        log::info!(
            "IS 参数：urgency={urgency}, arrival_price={arrival_price}, \
             horizon_secs={horizon_secs}, interval_secs={interval_secs}, 切片数={num_intervals}, \
             randomization={}, max_market_impact_ratio={:.1}%",
            if randomization_enabled { "enabled" } else { "disabled" },
            max_market_impact_ratio * 100.0
        );

        // 将主订单添加到缓存
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        let state = IsOrderState {
            instrument_id: order.instrument_id(),
            order_side,
            arrival_price,
            remaining_raw: total_raw,
            precision,
            scheduled_sizes: scheduled_sizes.clone(),
            elapsed_intervals: 0,
            randomization_enabled,
            max_market_impact_ratio,
        };

        self.order_states.insert(primary_id, state);

        // 提交第一片（不做价格调整，因为此刻就是到达时刻）
        let first_qty = self
            .order_states
            .get_mut(&primary_id)
            .unwrap()
            .scheduled_sizes
            .remove(0);

        let is_single_slice = self
            .order_states
            .get(&primary_id)
            .is_some_and(|s| s.scheduled_sizes.is_empty());

        // 单一切片：直接提交
        if is_single_slice {
            self.submit_order(order, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 多个切片：生成第一个子订单
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

        // 更新剩余量
        if let Some(state) = self.order_states.get_mut(&primary_id) {
            state.remaining_raw -= first_qty.raw;
        }

        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&order)?;
        }

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
            "开始执行 {primary_id} 的 IS：urgency={urgency}, arrival_price={arrival_price}, \
             interval={:.2}s{}",
            interval_secs,
            if randomization_enabled {
                format!(" (randomized: {:.2}s)", randomized_interval.as_secs_f64())
            } else {
                String::new()
            }
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

        let Some(state) = self.order_states.get_mut(&primary_id) else {
            log::error!("找不到 exec_spawn_id={primary_id} 的 IS 状态");
            return Ok(());
        };

        state.elapsed_intervals += 1;

        if state.scheduled_sizes.is_empty() {
            log::warn!("exec_spawn_id={primary_id} 没有更多可执行的数量");
            return Ok(());
        }

        let base_qty = state.scheduled_sizes.remove(0);
        let is_final_slice = state.scheduled_sizes.is_empty();

        // 最终切片：提交主订单（所有剩余数量）
        if is_final_slice {
            self.submit_order(primary, None, None)?;
            self.complete_sequence(&primary_id);
            return Ok(());
        }

        // 应用数量随机化
        let randomized_base_raw = if state.randomization_enabled {
            Self::apply_quantity_randomization(base_qty.raw, state.remaining_raw, is_final_slice)
        } else {
            base_qty.raw
        };

        // 查询当前市场价格，进行价格自适应调整
        let current_price = {
            let cache = self.core.cache();
            cache
                .price(&state.instrument_id, PriceType::Last)
                .map(|p| p.as_f64())
        };

        let adaptation_factor = current_price
            .map(|cp| Self::price_adaptation_factor(state.arrival_price, cp, state.order_side))
            .unwrap_or(1.0);

        // 应用价格自适应因子到随机化后的数量
        let adjusted_raw =
            (randomized_base_raw as f64 * adaptation_factor).floor() as QuantityRaw;

        // 应用市场冲击保护
        let market_impact_limited_raw = if state.max_market_impact_ratio > 0.0 {
            let cache = self.core.cache();
            Self::check_market_impact_limit(
                &cache,
                &state.instrument_id,
                adjusted_raw,
                state.max_market_impact_ratio,
                state.order_side,
            )
        } else {
            adjusted_raw
        };

        // 不超过剩余数量
        let slice_raw = std::cmp::min(market_impact_limited_raw, state.remaining_raw);
        // 保证至少为 1（如果 remaining_raw > 0）
        let slice_raw = if slice_raw == 0 && state.remaining_raw > 0 {
            1
        } else {
            slice_raw
        };
        let slice_qty = Quantity::from_raw(slice_raw, state.precision);

        log::info!(
            "IS {primary_id} 间隔 {}: base={}, randomized={}, adaptation={:.3}, \
             market_limited={}, adjusted={}{}",
            state.elapsed_intervals,
            base_qty,
            Quantity::from_raw(randomized_base_raw, state.precision),
            adaptation_factor,
            Quantity::from_raw(market_impact_limited_raw, state.precision),
            slice_qty,
            current_price.map_or(String::new(), |cp| {
                format!(
                    ", current_price={cp:.4}, arrival_price={:.4}",
                    state.arrival_price
                )
            })
        );

        // 更新剩余量
        state.remaining_raw -= slice_raw;

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

    fn create_is_algorithm() -> IsAlgorithm {
        let unique_id = format!("IS-{}", UUID4::new());
        let config = IsAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        IsAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut IsAlgorithm) {
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

    fn add_instrument_to_cache(algo: &mut IsAlgorithm) {
        use nautilus_model::instruments::{InstrumentAny, stubs::crypto_perpetual_ethusdt};

        let instrument = crypto_perpetual_ethusdt();
        let cache_rc = algo.core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache
            .add_instrument(InstrumentAny::CryptoPerpetual(instrument))
            .unwrap();
    }

    fn create_is_params(
        urgency: &str,
        horizon: &str,
        interval: &str,
        arrival_price: Option<&str>,
    ) -> IndexMap<Ustr, Ustr> {
        let mut params = IndexMap::new();
        params.insert(Ustr::from("urgency"), Ustr::from(urgency));
        params.insert(Ustr::from("horizon_secs"), Ustr::from(horizon));
        params.insert(Ustr::from("interval_secs"), Ustr::from(interval));
        if let Some(ap) = arrival_price {
            params.insert(Ustr::from("arrival_price"), Ustr::from(ap));
        }
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
            Some(ExecAlgorithmId::new("IS")),
            Some(params),
            None,
            None,
        ))
    }

    // ==================== 基础测试 ====================

    #[rstest]
    fn test_is_creation() {
        let algo = create_is_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("IS"));
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_is_registration() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_is_reset_clears_states() {
        let mut algo = create_is_algorithm();

        algo.order_states.insert(
            ClientOrderId::new("O-001"),
            IsOrderState {
                instrument_id: InstrumentId::from("ETHUSDT-PERP.BINANCE"),
                order_side: OrderSide::Buy,
                arrival_price: 3000.0,
                remaining_raw: 1000,
                precision: 1,
                scheduled_sizes: vec![],
                elapsed_intervals: 0,
                randomization_enabled: true,
                max_market_impact_ratio: 0.05,
            },
        );

        assert!(!algo.order_states.is_empty());
        ExecutionAlgorithm::on_reset(&mut algo).unwrap();
        assert!(algo.order_states.is_empty());
    }

    // ==================== 参数验证测试 ====================

    #[rstest]
    fn test_is_rejects_non_market_orders() {
        let mut algo = create_is_algorithm();
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
            None,  // tags
            UUID4::new(),
            0.into(),
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_is_rejects_missing_urgency() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("arrival_price"), Ustr::from("3000"));
        // 缺少 urgency

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_is_rejects_urgency_out_of_range() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("1.5", "60", "20", Some("3000"));
        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_is_rejects_horizon_less_than_interval() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "10", "30", Some("3000"));
        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_is_rejects_duplicate_order() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "60", "20", Some("3000"));
        let order1 = create_market_order_with_params(params.clone());
        let order2 = create_market_order_with_params(params);

        algo.on_order(order1).unwrap();
        let result = algo.on_order(order2);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已经在执行中"));
    }

    // ==================== 调度生成测试 ====================

    #[rstest]
    fn test_urgency_schedule_preserves_total() {
        let total_raw: QuantityRaw = 1_000_000_000;
        let precision = 1;

        for urgency in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let sizes = IsAlgorithm::generate_urgency_schedule(5, urgency, total_raw, precision);
            assert_eq!(sizes.len(), 5);

            let total_allocated: QuantityRaw = sizes.iter().map(|q| q.raw).sum();
            assert_eq!(
                total_allocated, total_raw,
                "Total not preserved for urgency={urgency}"
            );
        }
    }

    #[rstest]
    fn test_urgency_zero_is_uniform() {
        let total_raw: QuantityRaw = 1_000_000_000;
        let precision = 1;

        // urgency = 0 → 指数 = 0 → 所有权重 = 1.0 → 均匀
        let sizes = IsAlgorithm::generate_urgency_schedule(4, 0.0, total_raw, precision);
        assert_eq!(sizes.len(), 4);

        // 前 3 个应该大约相等，第 4 个取余数
        let expected_each = total_raw / 4;
        for (i, size) in sizes.iter().enumerate() {
            if i < 3 {
                assert_eq!(
                    size.raw, expected_each,
                    "Slice {i} should be ~{expected_each}"
                );
            }
        }
    }

    #[rstest]
    fn test_urgency_high_is_front_loaded() {
        let total_raw: QuantityRaw = 1_000_000_000;
        let precision = 1;

        let sizes = IsAlgorithm::generate_urgency_schedule(4, 0.9, total_raw, precision);

        // 高紧迫度时，第一个切片应显著大于最后一个
        assert!(sizes[0].raw > sizes[3].raw, "First slice should be largest");
        assert!(sizes[0].raw > sizes[1].raw, "Schedule should be decreasing");
    }

    // ==================== 价格自适应测试 ====================

    #[rstest]
    fn test_price_adaptation_neutral() {
        // 当前价格等于到达价 → 因子 = 1.0
        let factor =
            IsAlgorithm::price_adaptation_factor(3000.0, 3000.0, OrderSide::Buy);
        assert!((factor - 1.0).abs() < f64::EPSILON);
    }

    #[rstest]
    fn test_price_adaptation_favorable_buy() {
        // 买入时价格下跌 → 有利 → 因子 > 1.0
        let factor =
            IsAlgorithm::price_adaptation_factor(3000.0, 2970.0, OrderSide::Buy);
        assert!(factor > 1.0, "Factor should be > 1.0 for favorable buy, got {factor}");
    }

    #[rstest]
    fn test_price_adaptation_unfavorable_buy() {
        // 买入时价格上涨 → 不利 → 因子 < 1.0
        let factor =
            IsAlgorithm::price_adaptation_factor(3000.0, 3030.0, OrderSide::Buy);
        assert!(factor < 1.0, "Factor should be < 1.0 for unfavorable buy, got {factor}");
    }

    #[rstest]
    fn test_price_adaptation_favorable_sell() {
        // 卖出时价格上涨 → 有利 → 因子 > 1.0
        let factor =
            IsAlgorithm::price_adaptation_factor(3000.0, 3030.0, OrderSide::Sell);
        assert!(factor > 1.0, "Factor should be > 1.0 for favorable sell, got {factor}");
    }

    #[rstest]
    fn test_price_adaptation_clamped() {
        // 极端价格偏差应被限制在 [0.5, 1.5]
        let factor_max =
            IsAlgorithm::price_adaptation_factor(3000.0, 1000.0, OrderSide::Buy);
        assert!(
            (factor_max - 1.5).abs() < f64::EPSILON,
            "Should be clamped to 1.5, got {factor_max}"
        );

        let factor_min =
            IsAlgorithm::price_adaptation_factor(3000.0, 5000.0, OrderSide::Buy);
        assert!(
            (factor_min - 0.5).abs() < f64::EPSILON,
            "Should be clamped to 0.5, got {factor_min}"
        );
    }

    // ==================== 执行流程测试 ====================

    #[rstest]
    fn test_is_initializes_and_submits_first_slice() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "60", "20", Some("3000"));
        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 应该有跟踪状态
        let state = algo.order_states.get(&primary_id).unwrap();
        assert_eq!(state.elapsed_intervals, 0);
        assert_eq!(state.scheduled_sizes.len(), 2); // 初始 3 片，第 1 片已提交
        assert!(state.remaining_raw < Quantity::from("1.2").raw);
    }

    #[rstest]
    fn test_is_on_time_event_spawns_next_slice() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "60", "20", Some("3000"));
        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        let scheduled_before = algo
            .order_states
            .get(&primary_id)
            .unwrap()
            .scheduled_sizes
            .len();

        let event = TimeEvent::new(
            primary_id.inner(),
            UUID4::new(),
            0.into(),
            0.into(),
        );
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        let scheduled_after = algo
            .order_states
            .get(&primary_id)
            .unwrap()
            .scheduled_sizes
            .len();

        assert_eq!(scheduled_after, scheduled_before - 1);
    }

    #[rstest]
    fn test_is_on_time_event_completes_on_final_slice() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        // 2 个间隔：第 1 片立即提交，1 片在 scheduled_sizes 中
        let params = create_is_params("0.5", "60", "30", Some("3000"));
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        assert_eq!(
            algo.order_states
                .get(&primary_id)
                .unwrap()
                .scheduled_sizes
                .len(),
            1
        );

        let event = TimeEvent::new(
            primary_id.inner(),
            UUID4::new(),
            0.into(),
            0.into(),
        );
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        // 序列完成
        assert!(algo.order_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_is_on_time_event_completes_when_primary_closed() {
        use nautilus_model::events::OrderCanceled;

        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "60", "20", Some("3000"));
        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

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

        let event = TimeEvent::new(
            primary_id.inner(),
            UUID4::new(),
            0.into(),
            0.into(),
        );
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        assert!(algo.order_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_is_on_stop_cancels_timers() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let params = create_is_params("0.5", "60", "20", Some("3000"));
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        assert!(algo.core.clock().timer_names().contains(&primary_id.as_str()));

        ExecutionAlgorithm::on_stop(&mut algo).unwrap();
        assert!(algo.core.clock().timer_names().is_empty());
    }

    #[rstest]
    fn test_is_urgency_zero_allowed() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        // urgency = 0 是允许的（均匀分配，类似 TWAP）
        let params = create_is_params("0.0", "60", "20", Some("3000"));
        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert!(algo.order_states.contains_key(&primary_id));
    }

    // ==================== 随机性和市场冲击保护测试 ====================

    #[rstest]
    fn test_is_time_randomization() {
        let base_interval = 20.0;
        let randomized = IsAlgorithm::apply_time_randomization(base_interval);
        let ratio = randomized.as_secs_f64() / base_interval;
        
        assert!(
            ratio >= 0.8 && ratio <= 1.2,
            "Time randomization factor {ratio} should be within ±20% (0.8-1.2)"
        );
    }

    #[rstest]
    fn test_is_quantity_randomization() {
        let base_qty_raw = 1000;
        let remaining_raw = 10000;
        
        let randomized = IsAlgorithm::apply_quantity_randomization(
            base_qty_raw,
            remaining_raw,
            false,
        );
        
        let ratio = randomized as f64 / base_qty_raw as f64;
        assert!(
            ratio >= 0.95 && ratio <= 1.05,
            "Quantity randomization factor {ratio} should be within ±5% (0.95-1.05)"
        );
    }

    #[rstest]
    fn test_is_final_slice_no_randomization() {
        let base_qty_raw = 1000;
        let remaining_raw = 500;
        
        let randomized = IsAlgorithm::apply_quantity_randomization(
            base_qty_raw,
            remaining_raw,
            true,
        );
        
        assert_eq!(
            randomized, remaining_raw,
            "Final slice should use exact remaining amount"
        );
    }

    #[rstest]
    fn test_is_market_impact_limit() {
        let mut algo = create_is_algorithm();
        register_algorithm(&mut algo);
        add_instrument_to_cache(&mut algo);

        let proposed_qty_raw = 1000;
        let max_impact_ratio = 0.05;
        
        let cache = algo.core.cache();
        let limited = IsAlgorithm::check_market_impact_limit(
            &cache,
            &InstrumentId::from("ETHUSDT-PERP.BINANCE"),
            proposed_qty_raw,
            max_impact_ratio,
            OrderSide::Buy,
        );
        
        // 如果没有订单簿数据，应该返回原始数量
        assert_eq!(limited, proposed_qty_raw);
    }
}
