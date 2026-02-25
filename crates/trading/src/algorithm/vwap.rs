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
//! # 参数
//!
//! 提交给此算法的订单必须包含 `exec_algorithm_params`，其中包含：
//! - `horizon_secs`: 总执行时间范围（秒）。
//! - `interval_secs`: 子订单之间的时间间隔（秒）。
//! - `volume_profile`: 逗号分隔的成交量权重列表（如 "3,2,1,1,2,3"）。
//!   权重数量必须等于 `horizon_secs / interval_secs`（间隔数量）。
//!   各权重值的大小关系代表各时间段的相对成交量。
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
use ustr::Ustr;

use super::{ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore};

/// [`VwapAlgorithm`] 的配置。
pub type VwapAlgorithmConfig = ExecutionAlgorithmConfig;

/// 成交量加权平均价格 (VWAP) 执行算法。
///
/// 根据历史日内成交量分布按比例分散执行订单。
/// 该算法接收一个主订单，结合 volume_profile 权重生成
/// 按市场流动性分布的较小子订单。
#[derive(Debug)]
pub struct VwapAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的计划执行大小。
    scheduled_sizes: AHashMap<ClientOrderId, Vec<Quantity>>,
}

impl VwapAlgorithm {
    /// 创建一个新的 [`VwapAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: VwapAlgorithmConfig) -> Self {
        Self {
            core: ExecutionAlgorithmCore::new(config),
            scheduled_sizes: AHashMap::new(),
        }
    }

    /// 完成主订单的执行序列。
    fn complete_sequence(&mut self, primary_id: &ClientOrderId) {
        let timer_name = primary_id.as_str();
        if self.core.clock().timer_names().contains(&timer_name) {
            self.core.clock().cancel_timer(timer_name);
        }
        self.scheduled_sizes.remove(primary_id);
        log::info!("完成 {primary_id} 的 VWAP 执行");
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
                // 最后一个切片获得所有剩余数量，确保不丢失精度
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

        if self.scheduled_sizes.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 VWAP 执行订单: {order:?}");

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

        // 解析 volume_profile
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

        let num_intervals = (horizon_secs / interval_secs).floor() as u64;
        if num_intervals == 0 {
            log::error!("无法执行订单：间隔数量 (num_intervals) 为 0");
            return Ok(());
        }

        // 验证权重数量与间隔数量一致
        if weights.len() != num_intervals as usize {
            log::error!(
                "无法执行订单：volume_profile 权重数量 ({}) 与间隔数量 ({}) 不匹配",
                weights.len(),
                num_intervals
            );
            return Ok(());
        }

        let total_qty = order.quantity();
        let total_raw = total_qty.raw;
        let precision = total_qty.precision;

        // 按成交量权重计算各切片的数量
        let scheduled_sizes = Self::calculate_weighted_sizes(&weights, total_raw, precision);

        // 检查是否所有切片都有效
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

        log::info!("VWAP 订单执行大小计划表: {scheduled_sizes:?}");
        log::info!(
            "VWAP 成交量权重: {:?}, 总权重: {:.2}",
            weights,
            weights.iter().sum::<f64>()
        );

        // 将主订单添加到缓存，以便 on_time_event 之后可以检索它
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        self.scheduled_sizes
            .insert(primary_id, scheduled_sizes.clone());

        let first_qty = self.scheduled_sizes.get_mut(&primary_id).unwrap().remove(0);
        let is_single_slice = self
            .scheduled_sizes
            .get(&primary_id)
            .is_some_and(|s| s.is_empty());

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

        self.core.clock().set_timer(
            primary_id.as_str(),
            Duration::from_secs_f64(interval_secs),
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

        let Some(scheduled_sizes) = self.scheduled_sizes.get_mut(&primary_id) else {
            log::error!("找不到 exec_spawn_id={primary_id} 的计划大小");
            return Ok(());
        };

        if scheduled_sizes.is_empty() {
            log::warn!("exec_spawn_id={primary_id} 没有更多可执行的数量");
            return Ok(());
        }

        let quantity = scheduled_sizes.remove(0);
        let is_final_slice = scheduled_sizes.is_empty();

        // 最后一片：提交主订单（已减少为剩余数量）
        if is_final_slice {
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

        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        self.core.clock().cancel_timers();
        Ok(())
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.unsubscribe_all_strategy_events();
        self.core.reset();
        self.scheduled_sizes.clear();
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

    // ==================== 基础测试 ====================

    #[rstest]
    fn test_vwap_creation() {
        let algo = create_vwap_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("VWAP"));
        assert!(algo.scheduled_sizes.is_empty());
    }

    #[rstest]
    fn test_vwap_registration() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_vwap_reset_clears_scheduled_sizes() {
        let mut algo = create_vwap_algorithm();
        let primary_id = ClientOrderId::new("O-001");

        algo.scheduled_sizes
            .insert(primary_id, vec![Quantity::from("1.0")]);

        assert!(!algo.scheduled_sizes.is_empty());

        ExecutionAlgorithm::on_reset(&mut algo).unwrap();

        assert!(algo.scheduled_sizes.is_empty());
    }

    // ==================== 参数验证测试 ====================

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
    fn test_vwap_rejects_missing_volume_profile() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        // 缺少 volume_profile

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.scheduled_sizes.is_empty());
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
        assert!(algo.scheduled_sizes.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_mismatched_profile_length() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        // 3 个间隔但只有 2 个权重
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.scheduled_sizes.is_empty());
    }

    #[rstest]
    fn test_vwap_rejects_horizon_less_than_interval() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("30"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("60"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("1"));

        let order = create_market_order_with_params(params);
        let result = algo.on_order(order);

        assert!(result.is_ok());
        assert!(algo.scheduled_sizes.is_empty());
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

    // ==================== 分配计算测试 ====================

    #[rstest]
    fn test_vwap_weighted_distribution() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // 权重 3:1:2，总量 1.2
        // 总权重 = 6
        // 切片1: 1.2 * 3/6 = 0.6
        // 切片2: 1.2 * 1/6 = 0.2
        // 切片3: 1.2 * 2/6 = 0.4 (余数放在最后)
        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1,2"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 第一个切片已立即生成（0.6），剩余 2 个切片排队
        let remaining = algo.scheduled_sizes.get(&primary_id).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[rstest]
    fn test_vwap_equal_weights_behaves_like_twap() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // 权重全部相等 1:1:1，效果应与 TWAP 相同
        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("20"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("1,1,1"));

        let order = create_market_order_with_params_and_qty(params, Quantity::from("1.2"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 第一个切片立即生成，剩余 2 个切片排队
        let remaining = algo.scheduled_sizes.get(&primary_id).unwrap();
        assert_eq!(remaining.len(), 2);

        // 等权重时每个切片应相等
        for qty in remaining {
            assert_eq!(*qty, Quantity::from("0.4"));
        }
    }

    // ==================== 时间事件驱动测试 ====================

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

        assert_eq!(algo.scheduled_sizes.get(&primary_id).unwrap().len(), 2);

        // 模拟定时器触发
        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        // 消耗掉一个切片
        assert_eq!(algo.scheduled_sizes.get(&primary_id).unwrap().len(), 1);
    }

    #[rstest]
    fn test_vwap_on_time_event_completes_on_final_slice() {
        let mut algo = create_vwap_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // 2 个间隔：第一个立即生成，一个在 scheduled_sizes 中
        let mut params = IndexMap::new();
        params.insert(Ustr::from("horizon_secs"), Ustr::from("60"));
        params.insert(Ustr::from("interval_secs"), Ustr::from("30"));
        params.insert(Ustr::from("volume_profile"), Ustr::from("3,1"));

        let order = create_market_order_with_params(params);
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();
        assert_eq!(algo.scheduled_sizes.get(&primary_id).unwrap().len(), 1);

        // 为最后一个切片模拟定时器触发
        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        // 序列完成，scheduled_sizes 已移除
        assert!(algo.scheduled_sizes.get(&primary_id).is_none());
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
        assert_eq!(algo.scheduled_sizes.get(&primary_id).unwrap().len(), 2);

        // 将主订单标记为已关闭 (取消)
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

        // 定时器触发但主订单已关闭
        let event = TimeEvent::new(primary_id.inner(), UUID4::new(), 0.into(), 0.into());
        ExecutionAlgorithm::on_time_event(&mut algo, &event).unwrap();

        // 由于主订单已关闭，序列应提前完成
        assert!(algo.scheduled_sizes.get(&primary_id).is_none());
    }

    // ==================== 辅助函数测试 ====================

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
        let total_raw: QuantityRaw = 1_000_000_000; // 1.0 with 9 decimal precision
        let precision = 1;

        let sizes = VwapAlgorithm::calculate_weighted_sizes(&weights, total_raw, precision);

        assert_eq!(sizes.len(), 3);

        // 验证总和守恒
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

        // 验证定时器已设置
        assert!(
            algo.core
                .clock()
                .timer_names()
                .contains(&primary_id.as_str())
        );

        // 停止算法
        ExecutionAlgorithm::on_stop(&mut algo).unwrap();

        // 定时器应当已被取消
        assert!(algo.core.clock().timer_names().is_empty());
    }
}
