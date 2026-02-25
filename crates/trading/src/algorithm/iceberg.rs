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

//! 冰山 (Iceberg) 执行算法。
//!
//! Iceberg 算法将大额订单隐藏在盘口后方，每次只在市场上暴露一小部分
//! 可见数量（Display Quantity）。当暴露的部分被完全成交后，算法立即
//! 以相同的限价补挂下一片，直到总量全部执行完毕。
//!
//! 与 TWAP/VWAP/POV 不同，Iceberg 是**事件驱动**的（由子订单成交事件触发），
//! 而不是时间驱动的。它也是唯一使用**限价单**的执行算法。
//!
//! # 参数
//!
//! 提交给此算法的订单必须是**限价单**，并包含 `exec_algorithm_params`：
//! - `display_qty`: 每次在市场上显示的数量（冰山的"露出部分"）。
//!
//! # 工作原理
//!
//! 1. 算法收到一个大额限价主订单（例如 10000 股 @ 50.00）。
//! 2. 以主订单的限价生成一个 `display_qty` 大小的限价子订单（例如 500 股 @ 50.00）。
//! 3. 等待该子订单被市场完全成交。
//! 4. 成交后立即补挂下一个 `display_qty` 的限价子订单。
//! 5. 重复步骤 3-4，直到主订单的全部数量都被执行。
//! 6. 最后一片的数量可能小于 `display_qty`（剩余量不足时）。
//!
//! # 示例
//!
//! 一个 10000 股 @ 50.00 的限价单，设置 `display_qty=500`：
//! - 第 1 片：挂 500 股 @ 50.00 → 被吃掉
//! - 第 2 片：立即补挂 500 股 @ 50.00 → 被吃掉
//! - ...
//! - 第 20 片：挂 500 股 @ 50.00 → 被吃掉 → 完成

use std::ops::{Deref, DerefMut};

use ahash::AHashMap;
use nautilus_common::actor::{DataActor, DataActorCore};
use nautilus_model::{
    enums::OrderType,
    events::OrderFilled,
    identifiers::ClientOrderId,
    instruments::Instrument,
    orders::{Order, OrderAny},
    types::{Quantity, quantity::QuantityRaw},
};
use ustr::Ustr;

use super::{ExecutionAlgorithm, ExecutionAlgorithmConfig, ExecutionAlgorithmCore};

/// [`IcebergAlgorithm`] 的配置。
pub type IcebergAlgorithmConfig = ExecutionAlgorithmConfig;

/// 每个主订单的冰山跟踪状态。
#[derive(Debug)]
struct IcebergOrderState {
    /// 每片的显示数量 (raw)。
    display_qty_raw: QuantityRaw,
    /// 数量精度。
    precision: u8,
    /// 剩余需执行的原始数量。
    remaining_raw: QuantityRaw,
    /// 当前活跃的子订单 ID。
    current_spawn_id: Option<ClientOrderId>,
    /// 已完成的切片数。
    slices_completed: u64,
}

/// 冰山 (Iceberg) 执行算法。
///
/// 将大额限价订单拆分为多个小额限价子订单，
/// 每次只在市场上暴露 `display_qty`。当前一片被成交后，
/// 通过 `on_algo_order_filled` 事件驱动立即补挂下一片。
#[derive(Debug)]
pub struct IcebergAlgorithm {
    /// 算法核心。
    pub core: ExecutionAlgorithmCore,
    /// 每个主订单的跟踪状态。
    order_states: AHashMap<ClientOrderId, IcebergOrderState>,
    /// 子订单 ID → 主订单 ID 的反查映射。
    spawn_to_primary: AHashMap<ClientOrderId, ClientOrderId>,
}

impl IcebergAlgorithm {
    /// 创建一个新的 [`IcebergAlgorithm`] 实例。
    #[must_use]
    pub fn new(config: IcebergAlgorithmConfig) -> Self {
        Self {
            core: ExecutionAlgorithmCore::new(config),
            order_states: AHashMap::new(),
            spawn_to_primary: AHashMap::new(),
        }
    }

    /// 完成主订单的执行序列。
    fn complete_sequence(&mut self, primary_id: &ClientOrderId) {
        if let Some(state) = self.order_states.remove(primary_id) {
            // 清理反查映射
            if let Some(spawn_id) = &state.current_spawn_id {
                self.spawn_to_primary.remove(spawn_id);
            }
            log::info!(
                "完成 {primary_id} 的 Iceberg 执行 (共 {} 片)",
                state.slices_completed
            );
        }
    }

    /// 为主订单提交下一片限价子订单。
    fn submit_next_slice(&mut self, primary_id: &ClientOrderId) -> anyhow::Result<()> {
        let primary = {
            let cache = self.core.cache();
            cache.order(primary_id).cloned()
        };

        let Some(primary) = primary else {
            log::error!("找不到主订单 {primary_id}");
            return Ok(());
        };

        if primary.is_closed() {
            self.complete_sequence(primary_id);
            return Ok(());
        }

        let Some(state) = self.order_states.get(primary_id) else {
            log::error!("找不到 {primary_id} 的 Iceberg 状态");
            return Ok(());
        };

        if state.remaining_raw == 0 {
            // 提交主订单（数量已为 0，标记完成）
            self.complete_sequence(primary_id);
            return Ok(());
        }

        let remaining_raw = state.remaining_raw;
        let display_qty_raw = state.display_qty_raw;
        let precision = state.precision;

        // 本片数量 = min(display_qty, remaining)
        let slice_raw = std::cmp::min(display_qty_raw, remaining_raw);
        let slice_qty = Quantity::from_raw(slice_raw, precision);
        let is_final = slice_raw >= remaining_raw;

        log::info!(
            "Iceberg {primary_id}: 提交第 {} 片, 数量={slice_qty}{}",
            state.slices_completed + 1,
            if is_final { " (最终切片)" } else { "" }
        );

        // 最终切片：直接提交主订单
        if is_final {
            self.submit_order(primary, None, None)?;
            self.complete_sequence(primary_id);
            return Ok(());
        }

        // 获取主订单的限价
        let Some(price) = primary.price() else {
            log::error!("主订单 {primary_id} 没有限价");
            return Ok(());
        };

        // 生成限价子订单（先提取所有值避免 borrow 冲突）
        let tags = primary.tags().map(|t| t.to_vec());
        let time_in_force = primary.time_in_force();
        let reduce_only = primary.is_reduce_only();
        let expire_time = primary.expire_time();
        let post_only = primary.is_post_only();
        let mut primary = primary;
        let spawned = self.spawn_limit(
            &mut primary,
            slice_qty,
            price,
            time_in_force,
            expire_time,
            post_only,
            reduce_only,
            None, // display_qty
            None, // emulation_trigger
            tags,
            true, // reduce_primary
        );

        let spawn_id = spawned.client_order_id;
        self.submit_order(spawned.into(), None, None)?;

        // 更新缓存和状态
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.update_order(&primary)?;
        }

        // 更新跟踪状态
        if let Some(state) = self.order_states.get_mut(primary_id) {
            state.remaining_raw -= slice_raw;
            state.current_spawn_id = Some(spawn_id);
            state.slices_completed += 1;
        }

        // 注册反查映射
        self.spawn_to_primary.insert(spawn_id, *primary_id);

        Ok(())
    }
}

impl Deref for IcebergAlgorithm {
    type Target = DataActorCore;
    fn deref(&self) -> &Self::Target {
        &self.core.actor
    }
}

impl DerefMut for IcebergAlgorithm {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core.actor
    }
}

impl DataActor for IcebergAlgorithm {}

impl ExecutionAlgorithm for IcebergAlgorithm {
    fn core_mut(&mut self) -> &mut ExecutionAlgorithmCore {
        &mut self.core
    }

    fn on_order(&mut self, order: OrderAny) -> anyhow::Result<()> {
        let primary_id = order.client_order_id();

        if self.order_states.contains_key(&primary_id) {
            anyhow::bail!("订单 {primary_id} 已经在执行中");
        }

        log::info!("收到 Iceberg 执行订单: {order:?}");

        // 仅支持限价单（冰山算法的本质特征）
        if order.order_type() != OrderType::Limit {
            log::error!(
                "无法执行订单：Iceberg 仅支持限价单，当前订单类型={:?}",
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

        // 解析 display_qty
        let Some(display_qty_str) = exec_params.get(&Ustr::from("display_qty")) else {
            log::error!("无法执行订单：在 exec_algorithm_params 中找不到 display_qty");
            return Ok(());
        };

        let display_qty: Quantity = display_qty_str
            .parse::<f64>()
            .map(|v| Quantity::new(v, order.quantity().precision))
            .map_err(|e| {
                log::error!("无法解析 display_qty: {e}");
                anyhow::anyhow!("无效的 display_qty")
            })?;

        // 验证参数
        if display_qty.raw == 0 {
            log::error!("无法执行订单：display_qty 不能为 0");
            return Ok(());
        }

        let total_qty = order.quantity();

        // 如果 display_qty >= total_qty，直接全额提交
        if display_qty >= total_qty {
            log::warn!(
                "以全额提交：display_qty={display_qty} >= 订单总量={total_qty}"
            );
            self.submit_order(order, None, None)?;
            return Ok(());
        }

        // 检查最小数量约束
        if display_qty < instrument.size_increment() {
            log::warn!(
                "以全额提交：display_qty={display_qty} 小于最小交易增量 {}",
                instrument.size_increment()
            );
            self.submit_order(order, None, None)?;
            return Ok(());
        }

        if let Some(min_qty) = instrument.min_quantity() {
            if display_qty < min_qty {
                log::warn!(
                    "以全额提交：display_qty={display_qty} 小于最小订单数量 {min_qty}"
                );
                self.submit_order(order, None, None)?;
                return Ok(());
            }
        }

        let total_slices = (total_qty.raw + display_qty.raw - 1) / display_qty.raw; // ceiling division
        log::info!(
            "Iceberg 执行计划: total_qty={total_qty}, display_qty={display_qty}, \
             预计切片数={total_slices}"
        );

        // 将主订单添加到缓存
        {
            let cache_rc = self.core.cache_rc();
            let mut cache = cache_rc.borrow_mut();
            cache.add_order(order.clone(), None, None, false)?;
        }

        // 初始化跟踪状态
        let state = IcebergOrderState {
            display_qty_raw: display_qty.raw,
            precision: total_qty.precision,
            remaining_raw: total_qty.raw,
            current_spawn_id: None,
            slices_completed: 0,
        };

        self.order_states.insert(primary_id, state);

        // 提交第一片
        self.submit_next_slice(&primary_id)?;

        log::info!("开始执行 {primary_id} 的 Iceberg");

        Ok(())
    }

    /// Iceberg 的核心驱动：当子订单成交时，立即补挂下一片。
    fn on_algo_order_filled(&mut self, event: OrderFilled) {
        let filled_order_id = event.client_order_id;

        // 查找这个成交的子订单对应的主订单
        let Some(primary_id) = self.spawn_to_primary.get(&filled_order_id).copied() else {
            // 不是 Iceberg 管理的子订单，忽略
            return;
        };

        log::info!(
            "Iceberg 子订单 {filled_order_id} 已成交, 主订单={primary_id}"
        );

        // 清理当前子订单的反查映射
        self.spawn_to_primary.remove(&filled_order_id);

        // 检查剩余数量
        let has_remaining = self
            .order_states
            .get(&primary_id)
            .is_some_and(|s| s.remaining_raw > 0);

        if has_remaining {
            if let Err(e) = self.submit_next_slice(&primary_id) {
                log::error!("Iceberg {primary_id}: 提交下一片失败: {e}");
            }
        } else {
            self.complete_sequence(&primary_id);
        }
    }

    /// Iceberg 不使用定时器驱动，此方法为空操作。
    fn on_time_event(&mut self, _event: &nautilus_common::timer::TimeEvent) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_stop(&mut self) -> anyhow::Result<()> {
        // Iceberg 不使用定时器，但清理状态
        self.order_states.clear();
        self.spawn_to_primary.clear();
        Ok(())
    }

    fn on_reset(&mut self) -> anyhow::Result<()> {
        self.unsubscribe_all_strategy_events();
        self.core.reset();
        self.order_states.clear();
        self.spawn_to_primary.clear();
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
        identifiers::{ExecAlgorithmId, InstrumentId, StrategyId, TraderId},
        orders::{LimitOrder, MarketOrder},
        types::Price,
    };
    use rstest::rstest;
    use ustr::Ustr;

    use super::*;

    fn create_iceberg_algorithm() -> IcebergAlgorithm {
        let unique_id = format!("ICEBERG-{}", UUID4::new());
        let config = IcebergAlgorithmConfig {
            exec_algorithm_id: Some(ExecAlgorithmId::new(&unique_id)),
            ..Default::default()
        };
        IcebergAlgorithm::new(config)
    }

    fn register_algorithm(algo: &mut IcebergAlgorithm) {
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

    fn add_instrument_to_cache(algo: &mut IcebergAlgorithm) {
        use nautilus_model::instruments::{InstrumentAny, stubs::crypto_perpetual_ethusdt};

        let instrument = crypto_perpetual_ethusdt();
        let cache_rc = algo.core.cache_rc();
        let mut cache = cache_rc.borrow_mut();
        cache
            .add_instrument(InstrumentAny::CryptoPerpetual(instrument))
            .unwrap();
    }

    fn create_limit_order_with_display_qty(
        display_qty: &str,
        quantity: Quantity,
    ) -> OrderAny {
        let mut params = IndexMap::new();
        params.insert(Ustr::from("display_qty"), Ustr::from(display_qty));

        OrderAny::Limit(LimitOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("ETHUSDT-PERP.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            quantity,
            Price::from("3000.0"),
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
            Some(ExecAlgorithmId::new("ICEBERG")),
            Some(params),
            None,  // exec_spawn_id
            None,  // tags
            UUID4::new(),
            0.into(),
        ))
    }

    // ==================== 基础测试 ====================

    #[rstest]
    fn test_iceberg_creation() {
        let algo = create_iceberg_algorithm();
        assert!(algo.core.exec_algorithm_id.inner().starts_with("ICEBERG"));
        assert!(algo.order_states.is_empty());
        assert!(algo.spawn_to_primary.is_empty());
    }

    #[rstest]
    fn test_iceberg_registration() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);
        assert!(algo.core.trader_id().is_some());
    }

    #[rstest]
    fn test_iceberg_reset_clears_states() {
        let mut algo = create_iceberg_algorithm();

        algo.order_states.insert(
            ClientOrderId::new("O-001"),
            IcebergOrderState {
                display_qty_raw: 100,
                precision: 1,
                remaining_raw: 500,
                current_spawn_id: None,
                slices_completed: 0,
            },
        );
        algo.spawn_to_primary.insert(
            ClientOrderId::new("O-001-E1"),
            ClientOrderId::new("O-001"),
        );

        assert!(!algo.order_states.is_empty());
        assert!(!algo.spawn_to_primary.is_empty());

        ExecutionAlgorithm::on_reset(&mut algo).unwrap();

        assert!(algo.order_states.is_empty());
        assert!(algo.spawn_to_primary.is_empty());
    }

    // ==================== 参数验证测试 ====================

    #[rstest]
    fn test_iceberg_rejects_market_orders() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        let mut params = IndexMap::new();
        params.insert(Ustr::from("display_qty"), Ustr::from("100"));

        let order = OrderAny::Market(MarketOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("BTC/USDT.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1000.0"),
            TimeInForce::Gtc,
            UUID4::new(),
            0.into(),
            false,
            false,
            None,
            None,
            None,
            None,
            Some(ExecAlgorithmId::new("ICEBERG")),
            Some(params),
            None,
            None,
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_iceberg_rejects_missing_display_qty() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // 限价单但没有 display_qty 参数
        let params = IndexMap::new();
        let order = OrderAny::Limit(LimitOrder::new(
            TraderId::from("TRADER-001"),
            StrategyId::from("STRAT-001"),
            InstrumentId::from("ETHUSDT-PERP.BINANCE"),
            ClientOrderId::from("O-001"),
            OrderSide::Buy,
            Quantity::from("1000.0"),
            Price::from("3000.0"),
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
            Some(ExecAlgorithmId::new("ICEBERG")),
            Some(params),
            None,
            None,
            UUID4::new(),
            0.into(),
        ));

        let result = algo.on_order(order);
        assert!(result.is_ok());
        assert!(algo.order_states.is_empty());
    }

    #[rstest]
    fn test_iceberg_submits_full_when_display_qty_ge_total() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // display_qty = 2000 >= total = 1000 → 直接全额提交
        let order = create_limit_order_with_display_qty("2000", Quantity::from("1000.0"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 全额提交后不应有跟踪状态
        assert!(algo.order_states.get(&primary_id).is_none());
    }

    #[rstest]
    fn test_iceberg_rejects_duplicate_order() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let order1 = create_limit_order_with_display_qty("100", Quantity::from("1000.0"));
        let order2 = create_limit_order_with_display_qty("100", Quantity::from("1000.0"));

        algo.on_order(order1).unwrap();
        let result = algo.on_order(order2);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已经在执行中"));
    }

    // ==================== 初始化和切片测试 ====================

    #[rstest]
    fn test_iceberg_initializes_state_and_submits_first_slice() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        // total = 1.0, display = 0.3
        let order = create_limit_order_with_display_qty("0.3", Quantity::from("1.0"));
        let primary_id = order.client_order_id();

        algo.on_order(order).unwrap();

        // 应该有跟踪状态
        let state = algo.order_states.get(&primary_id).unwrap();
        assert_eq!(state.slices_completed, 1); // 第一片已提交
        assert!(state.current_spawn_id.is_some()); // 有活跃的子订单

        // 应该有反查映射
        assert!(!algo.spawn_to_primary.is_empty());
    }

    #[rstest]
    fn test_iceberg_on_stop_clears_states() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        add_instrument_to_cache(&mut algo);

        let order = create_limit_order_with_display_qty("100", Quantity::from("1000.0"));
        algo.on_order(order).unwrap();

        assert!(!algo.order_states.is_empty());

        ExecutionAlgorithm::on_stop(&mut algo).unwrap();

        assert!(algo.order_states.is_empty());
        assert!(algo.spawn_to_primary.is_empty());
    }

    #[rstest]
    fn test_iceberg_on_time_event_is_noop() {
        use nautilus_common::timer::TimeEvent;

        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        // Iceberg 不使用定时器，on_time_event 应当是空操作
        let event = TimeEvent::new(Ustr::from("test"), UUID4::new(), 0.into(), 0.into());
        let result = ExecutionAlgorithm::on_time_event(&mut algo, &event);
        assert!(result.is_ok());
    }

    #[rstest]
    fn test_iceberg_ignores_unrelated_fill_events() {
        let mut algo = create_iceberg_algorithm();
        register_algorithm(&mut algo);

        // 一个不属于 Iceberg 的成交事件
        let fill = OrderFilled::default();
        algo.on_algo_order_filled(fill);

        // 不应崩溃，也不应改变状态
        assert!(algo.order_states.is_empty());
    }
}
