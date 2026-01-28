// -------------------------------------------------------------------------------------------------
//  版权所有 (C) 2015-2026 Nautech Systems Pty Ltd。保留所有权利。
//  https://nautechsystems.io
//
//  基于 GNU Lesser General Public License 3.0 版本（“许可证”）获得许可；
//  除非符合许可证，否则您不得使用此文件。
//  您可以在 https://www.gnu.org/licenses/lgpl-3.0.en.html 获取许可证副本。
//
//  除非适用法律要求或书面同意，
//  否则根据许可证分发的软件是基于“按原样”基础分发的，
//  不附带任何明示或暗示的保证或条件。
//  请参阅许可证以了解管理许可证下的权限和限制的具体语言。
// -------------------------------------------------------------------------------------------------

//! 交易领域模型的订单类型。

pub mod any;
#[cfg(any(test, feature = "stubs"))]
pub mod builder;
pub mod limit;
pub mod limit_if_touched;
pub mod list;
pub mod market;
pub mod market_if_touched;
pub mod market_to_limit;
pub mod stop_limit;
pub mod stop_market;
pub mod trailing_stop_limit;
pub mod trailing_stop_market;

#[cfg(any(test, feature = "stubs"))]
pub mod stubs;

// Re-exports
use ahash::AHashSet;
use enum_dispatch::enum_dispatch;
use indexmap::IndexMap;
use nautilus_core::{UUID4, UnixNanos};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ustr::Ustr;

#[cfg(any(test, feature = "stubs"))]
pub use crate::orders::builder::OrderTestBuilder;
pub use crate::orders::{
    any::{LimitOrderAny, OrderAny, PassiveOrderAny, StopOrderAny},
    limit::LimitOrder,
    limit_if_touched::LimitIfTouchedOrder,
    list::OrderList,
    market::MarketOrder,
    market_if_touched::MarketIfTouchedOrder,
    market_to_limit::MarketToLimitOrder,
    stop_limit::StopLimitOrder,
    stop_market::StopMarketOrder,
    trailing_stop_limit::TrailingStopLimitOrder,
    trailing_stop_market::TrailingStopMarketOrder,
};
use crate::{
    enums::{
        ContingencyType, LiquiditySide, OrderSide, OrderSideSpecified, OrderStatus, OrderType,
        PositionSide, TimeInForce, TrailingOffsetType, TriggerType,
    },
    events::{
        OrderAccepted, OrderCancelRejected, OrderCanceled, OrderDenied, OrderEmulated,
        OrderEventAny, OrderExpired, OrderFilled, OrderInitialized, OrderModifyRejected,
        OrderPendingCancel, OrderPendingUpdate, OrderRejected, OrderReleased, OrderSubmitted,
        OrderTriggered, OrderUpdated,
    },
    identifiers::{
        AccountId, ClientOrderId, ExecAlgorithmId, InstrumentId, OrderListId, PositionId,
        StrategyId, Symbol, TradeId, TraderId, Venue, VenueOrderId,
    },
    orderbook::OwnBookOrder,
    types::{Currency, Money, Price, Quantity},
};

/// 具有止损/触发价格的订单类型。
pub const STOP_ORDER_TYPES: &[OrderType] = &[
    OrderType::StopMarket,
    OrderType::StopLimit,
    OrderType::MarketIfTouched,
    OrderType::LimitIfTouched,
];

/// 具有限价价格的订单类型。
pub const LIMIT_ORDER_TYPES: &[OrderType] = &[
    OrderType::Limit,
    OrderType::StopLimit,
    OrderType::LimitIfTouched,
    OrderType::MarketIfTouched,
];

/// 本地活动订单（提交到交易所之前）的状态。
pub const LOCAL_ACTIVE_ORDER_STATUSES: &[OrderStatus] = &[
    OrderStatus::Initialized,
    OrderStatus::Emulated,
    OrderStatus::Released,
];

/// 可以安全进行取消查询的订单状态。
///
/// 这些状态表示订单正在交易所工作，且尚未处于取消处理过程中。
/// 在取消过滤器中包含 `PENDING_CANCEL` 可能会导致重复的取消尝试或未结订单计数不正确。
///
/// 注意：包含 `PENDING_UPDATE` 是因为正在更新的订单通常仍可以被取消
/// （在大多数交易所上，更新和取消是相互独立的操作）。
pub const CANCELLABLE_ORDER_STATUSES: &[OrderStatus] = &[
    OrderStatus::Accepted,
    OrderStatus::Triggered,
    OrderStatus::PendingUpdate,
    OrderStatus::PartiallyFilled,
];

/// 返回一个缓存的 `AHashSet`，包含可取消的订单状态，用于 O(1) 查找。
///
/// 对于小集合（4个元素），使用 `CANCELLABLE_ORDER_STATUSES.contains()` 可能
/// 同样快，因为缓存局部性更好。当你需要集合操作或构建基于 HashSet 的过滤器时，请使用此函数。
///
/// 注意：这是一个模块级别的便捷函数。你也可以直接使用
/// `OrderStatus::cancellable_statuses_set()`。
#[must_use]
pub fn cancellable_order_statuses_set() -> &'static AHashSet<OrderStatus> {
    OrderStatus::cancellable_statuses_set()
}

#[derive(thiserror::Error, Debug)]
pub enum OrderError {
    #[error("未找到订单: {0}")]
    NotFound(ClientOrderId),
    #[error("订单不变量失败: 对于此操作，必须有订单方向")]
    NoOrderSide,
    #[error("订单类型的事件无效")]
    InvalidOrderEvent,
    #[error("无效的订单状态转换")]
    InvalidStateTransition,
    #[error("订单已初始化")]
    AlreadyInitialized,
    #[error("订单没有先前状态")]
    NoPreviousState,
    #[error("重复成交: trade_id {0} 已应用到订单")]
    DuplicateFill(TradeId),
    #[error("{0}")]
    Invariant(#[from] anyhow::Error),
}

/// 将具有 `Ustr` 键和值的 IndexMap 转换为 `String` 键和值。
#[must_use]
pub fn ustr_indexmap_to_str(h: IndexMap<Ustr, Ustr>) -> IndexMap<String, String> {
    h.into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// 将具有 `String` 键和值的 IndexMap 转换为 `Ustr` 键和值。
#[must_use]
pub fn str_indexmap_to_ustr(h: IndexMap<String, String>) -> IndexMap<Ustr, Ustr> {
    h.into_iter()
        .map(|(k, v)| (Ustr::from(&k), Ustr::from(&v)))
        .collect()
}

#[inline]
pub(crate) fn check_display_qty(
    display_qty: Option<Quantity>,
    quantity: Quantity,
) -> Result<(), OrderError> {
    if let Some(q) = display_qty
        && q > quantity
    {
        return Err(OrderError::Invariant(anyhow::anyhow!(
            "`display_qty` may not exceed `quantity`"
        )));
    }
    Ok(())
}

#[inline]
pub(crate) fn check_time_in_force(
    time_in_force: TimeInForce,
    expire_time: Option<UnixNanos>,
) -> Result<(), OrderError> {
    if time_in_force == TimeInForce::Gtd && expire_time.unwrap_or_default() == 0 {
        return Err(OrderError::Invariant(anyhow::anyhow!(
            "`expire_time` is required for `GTD` order"
        )));
    }
    Ok(())
}

impl OrderStatus {
    /// 基于给定的 `event` 转换订单状态机。
    ///
    /// # Errors
    ///
    /// 如果从当前状态进行的转换无效，则返回错误。
    #[rustfmt::skip]
    pub fn transition(&mut self, event: &OrderEventAny) -> Result<Self, OrderError> {
        let new_state = match (self, event) {
            (Self::Initialized, OrderEventAny::Denied(_)) => Self::Denied,
            (Self::Initialized, OrderEventAny::Emulated(_)) => Self::Emulated,  // Emulated orders
            (Self::Initialized, OrderEventAny::Released(_)) => Self::Released,  // Emulated orders
            (Self::Initialized, OrderEventAny::Submitted(_)) => Self::Submitted,
            (Self::Initialized, OrderEventAny::Rejected(_)) => Self::Rejected,  // External orders
            (Self::Initialized, OrderEventAny::Accepted(_)) => Self::Accepted,  // External orders
            (Self::Initialized, OrderEventAny::Canceled(_)) => Self::Canceled,  // External orders
            (Self::Initialized, OrderEventAny::Expired(_)) => Self::Expired,  // External orders
            (Self::Initialized, OrderEventAny::Triggered(_)) => Self::Triggered, // External orders
            (Self::Initialized, OrderEventAny::Updated(_)) => Self::Initialized, // In-place modification
            (Self::Emulated, OrderEventAny::Canceled(_)) => Self::Canceled,  // Emulated orders
            (Self::Emulated, OrderEventAny::Expired(_)) => Self::Expired,  // Emulated orders
            (Self::Emulated, OrderEventAny::Released(_)) => Self::Released,  // Emulated orders
            (Self::Released, OrderEventAny::Submitted(_)) => Self::Submitted,  // Emulated orders
            (Self::Released, OrderEventAny::Denied(_)) => Self::Denied,  // Emulated orders
            (Self::Released, OrderEventAny::Canceled(_)) => Self::Canceled,  // Execution algo
            (Self::Released, OrderEventAny::Updated(_)) => Self::Released, // In-place modification
            (Self::Submitted, OrderEventAny::PendingUpdate(_)) => Self::PendingUpdate,
            (Self::Submitted, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,
            (Self::Submitted, OrderEventAny::Rejected(_)) => Self::Rejected,
            (Self::Submitted, OrderEventAny::Canceled(_)) => Self::Canceled,  // FOK and IOC cases
            (Self::Submitted, OrderEventAny::Accepted(_)) => Self::Accepted,
            (Self::Submitted, OrderEventAny::Updated(_)) => Self::Submitted,
            (Self::Submitted, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::Accepted, OrderEventAny::Rejected(_)) => Self::Rejected,  // StopLimit order
            (Self::Accepted, OrderEventAny::PendingUpdate(_)) => Self::PendingUpdate,
            (Self::Accepted, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,
            (Self::Accepted, OrderEventAny::Canceled(_)) => Self::Canceled,
            (Self::Accepted, OrderEventAny::Triggered(_)) => Self::Triggered,
            (Self::Accepted, OrderEventAny::Updated(_)) => Self::Accepted,  // Updates should preserve state
            (Self::Accepted, OrderEventAny::Expired(_)) => Self::Expired,
            (Self::Accepted, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::Canceled, OrderEventAny::Filled(_)) => Self::Filled,  // Real world possibility
            (Self::PendingUpdate, OrderEventAny::Rejected(_)) => Self::Rejected,
            (Self::PendingUpdate, OrderEventAny::Accepted(_)) => Self::Accepted,
            (Self::PendingUpdate, OrderEventAny::Canceled(_)) => Self::Canceled,
            (Self::PendingUpdate, OrderEventAny::Expired(_)) => Self::Expired,
            (Self::PendingUpdate, OrderEventAny::Triggered(_)) => Self::Triggered,
            (Self::PendingUpdate, OrderEventAny::PendingUpdate(_)) => Self::PendingUpdate,  // Allow multiple requests
            (Self::PendingUpdate, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,
            (Self::PendingUpdate, OrderEventAny::ModifyRejected(_)) => Self::PendingUpdate,  // Handled by modify_rejected to restore previous_status
            (Self::PendingUpdate, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::PendingCancel, OrderEventAny::Rejected(_)) => Self::Rejected,
            (Self::PendingCancel, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,  // Allow multiple requests
            (Self::PendingCancel, OrderEventAny::CancelRejected(_)) => Self::PendingCancel,  // Handled by cancel_rejected to restore previous_status
            (Self::PendingCancel, OrderEventAny::Canceled(_)) => Self::Canceled,
            (Self::PendingCancel, OrderEventAny::Expired(_)) => Self::Expired,
            (Self::PendingCancel, OrderEventAny::Accepted(_)) => Self::Accepted,  // Allow failed cancel requests
            (Self::PendingCancel, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::Triggered, OrderEventAny::Rejected(_)) => Self::Rejected,
            (Self::Triggered, OrderEventAny::PendingUpdate(_)) => Self::PendingUpdate,
            (Self::Triggered, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,
            (Self::Triggered, OrderEventAny::Canceled(_)) => Self::Canceled,
            (Self::Triggered, OrderEventAny::Expired(_)) => Self::Expired,
            (Self::Triggered, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::Triggered, OrderEventAny::Updated(_)) => Self::Triggered,
            (Self::PartiallyFilled, OrderEventAny::PendingUpdate(_)) => Self::PendingUpdate,
            (Self::PartiallyFilled, OrderEventAny::PendingCancel(_)) => Self::PendingCancel,
            (Self::PartiallyFilled, OrderEventAny::Canceled(_)) => Self::Canceled,
            (Self::PartiallyFilled, OrderEventAny::Expired(_)) => Self::Expired,
            (Self::PartiallyFilled, OrderEventAny::Filled(_)) => Self::Filled,
            (Self::PartiallyFilled, OrderEventAny::Accepted(_)) => Self::Accepted,
            (Self::PartiallyFilled, OrderEventAny::Updated(_)) => Self::PartiallyFilled,
            _ => return Err(OrderError::InvalidStateTransition),
        };
        Ok(new_state)
    }
}

#[enum_dispatch]
pub trait Order: 'static + Send {
    fn into_any(self) -> OrderAny;
    fn status(&self) -> OrderStatus;
    fn trader_id(&self) -> TraderId;
    fn strategy_id(&self) -> StrategyId;
    fn instrument_id(&self) -> InstrumentId;
    fn symbol(&self) -> Symbol;
    fn venue(&self) -> Venue;
    fn client_order_id(&self) -> ClientOrderId;
    fn venue_order_id(&self) -> Option<VenueOrderId>;
    fn position_id(&self) -> Option<PositionId>;
    fn account_id(&self) -> Option<AccountId>;
    fn last_trade_id(&self) -> Option<TradeId>;
    fn order_side(&self) -> OrderSide;
    fn order_type(&self) -> OrderType;
    fn quantity(&self) -> Quantity;
    fn time_in_force(&self) -> TimeInForce;
    fn expire_time(&self) -> Option<UnixNanos>;
    fn price(&self) -> Option<Price>;
    fn trigger_price(&self) -> Option<Price>;
    fn activation_price(&self) -> Option<Price> {
        None
    }
    fn trigger_type(&self) -> Option<TriggerType>;
    fn liquidity_side(&self) -> Option<LiquiditySide>;
    fn is_post_only(&self) -> bool;
    fn is_reduce_only(&self) -> bool;
    fn is_quote_quantity(&self) -> bool;
    fn display_qty(&self) -> Option<Quantity>;
    fn limit_offset(&self) -> Option<Decimal>;
    fn trailing_offset(&self) -> Option<Decimal>;
    fn trailing_offset_type(&self) -> Option<TrailingOffsetType>;
    fn emulation_trigger(&self) -> Option<TriggerType>;
    fn trigger_instrument_id(&self) -> Option<InstrumentId>;
    fn contingency_type(&self) -> Option<ContingencyType>;
    fn order_list_id(&self) -> Option<OrderListId>;
    fn linked_order_ids(&self) -> Option<&[ClientOrderId]>;
    fn parent_order_id(&self) -> Option<ClientOrderId>;
    fn exec_algorithm_id(&self) -> Option<ExecAlgorithmId>;
    fn exec_algorithm_params(&self) -> Option<&IndexMap<Ustr, Ustr>>;
    fn exec_spawn_id(&self) -> Option<ClientOrderId>;
    fn tags(&self) -> Option<&[Ustr]>;
    fn filled_qty(&self) -> Quantity;
    fn leaves_qty(&self) -> Quantity;
    fn overfill_qty(&self) -> Quantity;

    /// 计算潜在的超额成交数量，不改变订单状态。
    fn calculate_overfill(&self, fill_qty: Quantity) -> Quantity {
        let potential_filled = self.filled_qty() + fill_qty;
        potential_filled.saturating_sub(self.quantity())
    }

    fn avg_px(&self) -> Option<f64>;
    fn slippage(&self) -> Option<f64>;
    fn init_id(&self) -> UUID4;
    fn ts_init(&self) -> UnixNanos;
    fn ts_submitted(&self) -> Option<UnixNanos>;
    fn ts_accepted(&self) -> Option<UnixNanos>;
    fn ts_closed(&self) -> Option<UnixNanos>;
    fn ts_last(&self) -> UnixNanos;

    fn order_side_specified(&self) -> OrderSideSpecified {
        self.order_side().as_specified()
    }
    fn commissions(&self) -> &IndexMap<Currency, Money>;

    /// 将 `event` 应用于订单。
    ///
    /// # Errors
    ///
    /// 如果事件对于当前订单状态无效，则返回错误。
    fn apply(&mut self, event: OrderEventAny) -> Result<(), OrderError>;
    fn update(&mut self, event: &OrderUpdated);

    fn events(&self) -> Vec<&OrderEventAny>;

    fn last_event(&self) -> &OrderEventAny {
        // 安全性：订单规范保证至少有一个事件 (OrderInitialized)
        self.events().last().expect("订单不变量被破坏: 没有事件")
    }

    fn event_count(&self) -> usize {
        self.events().len()
    }

    fn venue_order_ids(&self) -> Vec<&VenueOrderId>;

    fn trade_ids(&self) -> Vec<&TradeId>;

    fn has_price(&self) -> bool;

    /// 如果存在具有匹配的 trade_id、side、qty 和 price 的成交，则返回 `true`。
    fn is_duplicate_fill(&self, fill: &OrderFilled) -> bool {
        self.events().iter().any(|event| {
            if let OrderEventAny::Filled(existing) = event {
                existing.trade_id == fill.trade_id
                    && existing.order_side == fill.order_side
                    && existing.last_qty == fill.last_qty
                    && existing.last_px == fill.last_px
            } else {
                false
            }
        })
    }

    fn is_buy(&self) -> bool {
        self.order_side() == OrderSide::Buy
    }

    fn is_sell(&self) -> bool {
        self.order_side() == OrderSide::Sell
    }

    fn is_passive(&self) -> bool {
        self.order_type() != OrderType::Market
    }

    fn is_aggressive(&self) -> bool {
        self.order_type() == OrderType::Market
    }

    fn is_emulated(&self) -> bool {
        self.status() == OrderStatus::Emulated
    }

    fn is_active_local(&self) -> bool {
        matches!(
            self.status(),
            OrderStatus::Initialized | OrderStatus::Emulated | OrderStatus::Released
        )
    }

    fn is_primary(&self) -> bool {
        self.exec_algorithm_id().is_some()
            && self
                .exec_spawn_id()
                .is_some_and(|spawn_id| self.client_order_id() == spawn_id)
    }

    fn is_spawned(&self) -> bool {
        self.exec_algorithm_id().is_some()
            && self
                .exec_spawn_id()
                .is_some_and(|spawn_id| self.client_order_id() != spawn_id)
    }

    fn is_contingency(&self) -> bool {
        self.contingency_type().is_some()
    }

    fn is_parent_order(&self) -> bool {
        match self.contingency_type() {
            Some(c) => c == ContingencyType::Oto,
            None => false,
        }
    }

    fn is_child_order(&self) -> bool {
        self.parent_order_id().is_some()
    }

    fn is_open(&self) -> bool {
        if let Some(emulation_trigger) = self.emulation_trigger()
            && emulation_trigger != TriggerType::NoTrigger
        {
            return false;
        }

        matches!(
            self.status(),
            OrderStatus::Accepted
                | OrderStatus::Triggered
                | OrderStatus::PendingCancel
                | OrderStatus::PendingUpdate
                | OrderStatus::PartiallyFilled
        )
    }

    fn is_canceled(&self) -> bool {
        self.status() == OrderStatus::Canceled
    }

    fn is_closed(&self) -> bool {
        matches!(
            self.status(),
            OrderStatus::Denied
                | OrderStatus::Rejected
                | OrderStatus::Canceled
                | OrderStatus::Expired
                | OrderStatus::Filled
        )
    }

    fn is_inflight(&self) -> bool {
        if let Some(emulation_trigger) = self.emulation_trigger()
            && emulation_trigger != TriggerType::NoTrigger
        {
            return false;
        }

        matches!(
            self.status(),
            OrderStatus::Submitted | OrderStatus::PendingCancel | OrderStatus::PendingUpdate
        )
    }

    fn is_pending_update(&self) -> bool {
        self.status() == OrderStatus::PendingUpdate
    }

    fn is_pending_cancel(&self) -> bool {
        self.status() == OrderStatus::PendingCancel
    }

    fn to_own_book_order(&self) -> OwnBookOrder {
        OwnBookOrder::new(
            self.trader_id(),
            self.client_order_id(),
            self.venue_order_id(),
            self.order_side().as_specified(),
            self.price().expect("`OwnBookOrder` must have a price"), // TBD
            self.quantity(),
            self.order_type(),
            self.time_in_force(),
            self.status(),
            self.ts_last(),
            self.ts_accepted().unwrap_or_default(),
            self.ts_submitted().unwrap_or_default(),
            self.ts_init(),
        )
    }

    fn is_triggered(&self) -> Option<bool>; // TODO: Temporary on trait
    fn set_position_id(&mut self, position_id: Option<PositionId>);
    fn set_quantity(&mut self, quantity: Quantity);
    fn set_leaves_qty(&mut self, leaves_qty: Quantity);
    fn set_emulation_trigger(&mut self, emulation_trigger: Option<TriggerType>);
    fn set_is_quote_quantity(&mut self, is_quote_quantity: bool);
    fn set_liquidity_side(&mut self, liquidity_side: LiquiditySide);
    fn would_reduce_only(&self, side: PositionSide, position_qty: Quantity) -> bool;
    fn previous_status(&self) -> Option<OrderStatus>;
}

impl<T> From<&T> for OrderInitialized
where
    T: Order,
{
    fn from(order: &T) -> Self {
        Self {
            trader_id: order.trader_id(),
            strategy_id: order.strategy_id(),
            instrument_id: order.instrument_id(),
            client_order_id: order.client_order_id(),
            order_side: order.order_side(),
            order_type: order.order_type(),
            quantity: order.quantity(),
            price: order.price(),
            trigger_price: order.trigger_price(),
            trigger_type: order.trigger_type(),
            time_in_force: order.time_in_force(),
            expire_time: order.expire_time(),
            post_only: order.is_post_only(),
            reduce_only: order.is_reduce_only(),
            quote_quantity: order.is_quote_quantity(),
            display_qty: order.display_qty(),
            limit_offset: order.limit_offset(),
            trailing_offset: order.trailing_offset(),
            trailing_offset_type: order.trailing_offset_type(),
            emulation_trigger: order.emulation_trigger(),
            trigger_instrument_id: order.trigger_instrument_id(),
            contingency_type: order.contingency_type(),
            order_list_id: order.order_list_id(),
            linked_order_ids: order.linked_order_ids().map(|x| x.to_vec()),
            parent_order_id: order.parent_order_id(),
            exec_algorithm_id: order.exec_algorithm_id(),
            exec_algorithm_params: order.exec_algorithm_params().map(|x| x.to_owned()),
            exec_spawn_id: order.exec_spawn_id(),
            tags: order.tags().map(|x| x.to_vec()),
            event_id: order.init_id(),
            ts_event: order.ts_init(),
            ts_init: order.ts_init(),
            reconciliation: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderCore {
    pub events: Vec<OrderEventAny>,
    pub commissions: IndexMap<Currency, Money>,
    pub venue_order_ids: Vec<VenueOrderId>,
    pub trade_ids: Vec<TradeId>,
    pub previous_status: Option<OrderStatus>,
    pub status: OrderStatus,
    pub trader_id: TraderId,
    pub strategy_id: StrategyId,
    pub instrument_id: InstrumentId,
    pub client_order_id: ClientOrderId,
    pub venue_order_id: Option<VenueOrderId>,
    pub position_id: Option<PositionId>,
    pub account_id: Option<AccountId>,
    pub last_trade_id: Option<TradeId>,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: Quantity,
    pub time_in_force: TimeInForce,
    pub liquidity_side: Option<LiquiditySide>,
    pub is_reduce_only: bool,
    pub is_quote_quantity: bool,
    pub emulation_trigger: Option<TriggerType>,
    pub contingency_type: Option<ContingencyType>,
    pub order_list_id: Option<OrderListId>,
    pub linked_order_ids: Option<Vec<ClientOrderId>>,
    pub parent_order_id: Option<ClientOrderId>,
    pub exec_algorithm_id: Option<ExecAlgorithmId>,
    pub exec_algorithm_params: Option<IndexMap<Ustr, Ustr>>,
    pub exec_spawn_id: Option<ClientOrderId>,
    pub tags: Option<Vec<Ustr>>,
    pub filled_qty: Quantity,
    pub leaves_qty: Quantity,
    pub overfill_qty: Quantity,
    pub avg_px: Option<f64>,
    pub slippage: Option<f64>,
    pub init_id: UUID4,
    pub ts_init: UnixNanos,
    pub ts_submitted: Option<UnixNanos>,
    pub ts_accepted: Option<UnixNanos>,
    pub ts_closed: Option<UnixNanos>,
    pub ts_last: UnixNanos,
}

impl OrderCore {
    /// 创建一个新的 [`OrderCore`] 实例。
    pub fn new(init: OrderInitialized) -> Self {
        let events: Vec<OrderEventAny> = vec![OrderEventAny::Initialized(init.clone())];
        Self {
            events,
            commissions: IndexMap::new(),
            venue_order_ids: Vec::new(),
            trade_ids: Vec::new(),
            previous_status: None,
            status: OrderStatus::Initialized,
            trader_id: init.trader_id,
            strategy_id: init.strategy_id,
            instrument_id: init.instrument_id,
            client_order_id: init.client_order_id,
            venue_order_id: None,
            position_id: None,
            account_id: None,
            last_trade_id: None,
            side: init.order_side,
            order_type: init.order_type,
            quantity: init.quantity,
            time_in_force: init.time_in_force,
            liquidity_side: Some(LiquiditySide::NoLiquiditySide),
            is_reduce_only: init.reduce_only,
            is_quote_quantity: init.quote_quantity,
            emulation_trigger: init.emulation_trigger.or(Some(TriggerType::NoTrigger)),
            contingency_type: init
                .contingency_type
                .or(Some(ContingencyType::NoContingency)),
            order_list_id: init.order_list_id,
            linked_order_ids: init.linked_order_ids,
            parent_order_id: init.parent_order_id,
            exec_algorithm_id: init.exec_algorithm_id,
            exec_algorithm_params: init.exec_algorithm_params,
            exec_spawn_id: init.exec_spawn_id,
            tags: init.tags,
            filled_qty: Quantity::zero(init.quantity.precision),
            leaves_qty: init.quantity,
            overfill_qty: Quantity::zero(init.quantity.precision),
            avg_px: None,
            slippage: None,
            init_id: init.event_id,
            ts_init: init.ts_event,
            ts_submitted: None,
            ts_accepted: None,
            ts_closed: None,
            ts_last: init.ts_event,
        }
    }

    /// 将 `event` 应用于订单。
    ///
    /// # Errors
    ///
    /// 如果事件对于当前订单状态无效，或者 `event.client_order_id()` 或 `event.strategy_id()` 与订单不匹配，则返回错误。
    pub fn apply(&mut self, event: OrderEventAny) -> Result<(), OrderError> {
        if self.client_order_id != event.client_order_id() {
            return Err(OrderError::Invariant(anyhow::anyhow!(
                "Event client_order_id {} does not match order client_order_id {}",
                event.client_order_id(),
                self.client_order_id
            )));
        }
        if self.strategy_id != event.strategy_id() {
            return Err(OrderError::Invariant(anyhow::anyhow!(
                "Event strategy_id {} does not match order strategy_id {}",
                event.strategy_id(),
                self.strategy_id
            )));
        }

        // 除了以下情况，将当前状态保存为 previous_status 以用于所有转换：
        // - Initialized (不存在先前状态)
        // - ModifyRejected/CancelRejected (需要保留 Pending 之前的状态)
        // - 已经处于 Pending* 状态时 (避免在收到多个 pending 请求时覆盖 Pending 之前的状态)
        if !matches!(
            event,
            OrderEventAny::Initialized(_)
                | OrderEventAny::ModifyRejected(_)
                | OrderEventAny::CancelRejected(_)
        ) && !matches!(
            self.status,
            OrderStatus::PendingUpdate | OrderStatus::PendingCancel
        ) {
            self.previous_status = Some(self.status);
        }

        // 在状态转换之前检查重复成交以保持一致性
        if let OrderEventAny::Filled(fill) = &event
            && self.trade_ids.contains(&fill.trade_id)
        {
            return Err(OrderError::DuplicateFill(fill.trade_id));
        }

        let new_status = self.status.transition(&event)?;
        self.status = new_status;

        match &event {
            OrderEventAny::Initialized(_) => return Err(OrderError::AlreadyInitialized),
            OrderEventAny::Denied(event) => self.denied(event),
            OrderEventAny::Emulated(event) => self.emulated(event),
            OrderEventAny::Released(event) => self.released(event),
            OrderEventAny::Submitted(event) => self.submitted(event),
            OrderEventAny::Rejected(event) => self.rejected(event),
            OrderEventAny::Accepted(event) => self.accepted(event),
            OrderEventAny::PendingUpdate(event) => self.pending_update(event),
            OrderEventAny::PendingCancel(event) => self.pending_cancel(event),
            OrderEventAny::ModifyRejected(event) => self.modify_rejected(event)?,
            OrderEventAny::CancelRejected(event) => self.cancel_rejected(event)?,
            OrderEventAny::Updated(event) => self.updated(event),
            OrderEventAny::Triggered(event) => self.triggered(event),
            OrderEventAny::Canceled(event) => self.canceled(event),
            OrderEventAny::Expired(event) => self.expired(event),
            OrderEventAny::Filled(event) => self.filled(event),
        }

        self.ts_last = event.ts_event();
        self.events.push(event);
        Ok(())
    }

    fn denied(&mut self, event: &OrderDenied) {
        self.ts_closed = Some(event.ts_event);
    }

    fn emulated(&self, _event: &OrderEmulated) {
        // Do nothing else
    }

    fn released(&mut self, _event: &OrderReleased) {
        self.emulation_trigger = None;
    }

    fn submitted(&mut self, event: &OrderSubmitted) {
        self.account_id = Some(event.account_id);
        self.ts_submitted = Some(event.ts_event);
    }

    fn accepted(&mut self, event: &OrderAccepted) {
        self.account_id = Some(event.account_id);
        self.venue_order_id = Some(event.venue_order_id);
        self.venue_order_ids.push(event.venue_order_id);
        self.ts_accepted = Some(event.ts_event);
    }

    fn rejected(&mut self, event: &OrderRejected) {
        self.ts_closed = Some(event.ts_event);
    }

    fn pending_update(&self, _event: &OrderPendingUpdate) {
        // Do nothing else
    }

    fn pending_cancel(&self, _event: &OrderPendingCancel) {
        // Do nothing else
    }

    fn modify_rejected(&mut self, _event: &OrderModifyRejected) -> Result<(), OrderError> {
        self.status = self.previous_status.ok_or(OrderError::NoPreviousState)?;
        Ok(())
    }

    fn cancel_rejected(&mut self, _event: &OrderCancelRejected) -> Result<(), OrderError> {
        self.status = self.previous_status.ok_or(OrderError::NoPreviousState)?;
        Ok(())
    }

    fn triggered(&mut self, _event: &OrderTriggered) {}

    fn canceled(&mut self, event: &OrderCanceled) {
        self.ts_closed = Some(event.ts_event);
    }

    fn expired(&mut self, event: &OrderExpired) {
        self.ts_closed = Some(event.ts_event);
    }

    fn updated(&mut self, event: &OrderUpdated) {
        if let Some(venue_order_id) = &event.venue_order_id
            && (self.venue_order_id.is_none()
                || venue_order_id != self.venue_order_id.as_ref().unwrap())
        {
            self.venue_order_id = Some(*venue_order_id);
            self.venue_order_ids.push(*venue_order_id);
        }
    }

    fn filled(&mut self, event: &OrderFilled) {
        // Use saturating arithmetic to prevent overflow
        let new_filled_qty = Quantity::from_raw(
            self.filled_qty.raw.saturating_add(event.last_qty.raw),
            self.filled_qty.precision,
        );

        // Calculate overfill if any
        if new_filled_qty > self.quantity {
            let overfill_raw = new_filled_qty.raw - self.quantity.raw;
            self.overfill_qty = Quantity::from_raw(
                self.overfill_qty.raw.saturating_add(overfill_raw),
                self.filled_qty.precision,
            );
        }

        if new_filled_qty < self.quantity {
            self.status = OrderStatus::PartiallyFilled;
        } else {
            self.status = OrderStatus::Filled;
            self.ts_closed = Some(event.ts_event);
        }

        self.venue_order_id = Some(event.venue_order_id);
        self.position_id = event.position_id;
        self.trade_ids.push(event.trade_id);
        self.last_trade_id = Some(event.trade_id);
        self.liquidity_side = Some(event.liquidity_side);
        self.filled_qty = new_filled_qty;
        self.leaves_qty = self.leaves_qty.saturating_sub(event.last_qty);
        self.ts_last = event.ts_event;
        if self.ts_accepted.is_none() {
            // Set ts_accepted to time of first fill if not previously set
            self.ts_accepted = Some(event.ts_event);
        }

        self.set_avg_px(event.last_qty, event.last_px);
    }

    fn set_avg_px(&mut self, last_qty: Quantity, last_px: Price) {
        if self.avg_px.is_none() {
            self.avg_px = Some(last_px.as_f64());
            return;
        }

        // 使用之前的成交数量（在当前成交之前）以避免重复计算
        let prev_filled_qty = (self.filled_qty - last_qty).as_f64();
        let last_qty_f64 = last_qty.as_f64();
        let total_qty = prev_filled_qty + last_qty_f64;

        let avg_px = self
            .avg_px
            .unwrap()
            .mul_add(prev_filled_qty, last_px.as_f64() * last_qty_f64)
            / total_qty;
        self.avg_px = Some(avg_px);
    }

    pub fn set_slippage(&mut self, price: Price) {
        self.slippage = self.avg_px.and_then(|avg_px| {
            let current_price = price.as_f64();
            match self.side {
                OrderSide::Buy if avg_px > current_price => Some(avg_px - current_price),
                OrderSide::Sell if avg_px < current_price => Some(current_price - avg_px),
                _ => None,
            }
        });
    }

    /// Returns the opposite order side.
    #[must_use]
    pub fn opposite_side(side: OrderSide) -> OrderSide {
        match side {
            OrderSide::Buy => OrderSide::Sell,
            OrderSide::Sell => OrderSide::Buy,
            OrderSide::NoOrderSide => OrderSide::NoOrderSide,
        }
    }

    /// Returns the order side needed to close a position.
    #[must_use]
    pub fn closing_side(side: PositionSide) -> OrderSide {
        match side {
            PositionSide::Long => OrderSide::Sell,
            PositionSide::Short => OrderSide::Buy,
            PositionSide::Flat => OrderSide::NoOrderSide,
            PositionSide::NoPositionSide => OrderSide::NoOrderSide,
        }
    }

    /// # Panics
    ///
    /// Panics if the order side is neither `Buy` nor `Sell`.
    #[must_use]
    pub fn signed_decimal_qty(&self) -> Decimal {
        match self.side {
            OrderSide::Buy => self.quantity.as_decimal(),
            OrderSide::Sell => -self.quantity.as_decimal(),
            _ => panic!("Invalid order side"),
        }
    }

    #[must_use]
    pub fn would_reduce_only(&self, side: PositionSide, position_qty: Quantity) -> bool {
        if side == PositionSide::Flat {
            return false;
        }

        match (self.side, side) {
            (OrderSide::Buy, PositionSide::Long) => false,
            (OrderSide::Buy, PositionSide::Short) => self.leaves_qty <= position_qty,
            (OrderSide::Sell, PositionSide::Short) => false,
            (OrderSide::Sell, PositionSide::Long) => self.leaves_qty <= position_qty,
            _ => true,
        }
    }

    #[must_use]
    pub fn commission(&self, currency: &Currency) -> Option<Money> {
        self.commissions.get(currency).copied()
    }

    #[must_use]
    pub fn commissions(&self) -> IndexMap<Currency, Money> {
        self.commissions.clone()
    }

    #[must_use]
    pub fn commissions_vec(&self) -> Vec<Money> {
        self.commissions.values().copied().collect()
    }

    #[must_use]
    pub fn init_event(&self) -> Option<OrderEventAny> {
        self.events.first().cloned()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        enums::{OrderSide, OrderStatus, PositionSide},
        events::order::{
            accepted::OrderAcceptedBuilder, canceled::OrderCanceledBuilder,
            denied::OrderDeniedBuilder, filled::OrderFilledBuilder,
            initialized::OrderInitializedBuilder, submitted::OrderSubmittedBuilder,
            triggered::OrderTriggeredBuilder, updated::OrderUpdatedBuilder,
        },
        orders::MarketOrder,
    };

    // TODO: WIP
    // fn test_display_market_order() {
    //     let order = MarketOrder::default();
    //     assert_eq!(order.events().len(), 1);
    //     assert_eq!(
    //         stringify!(order.events().get(0)),
    //         stringify!(OrderInitialized)
    //     );
    // }

    #[rstest]
    #[case(OrderSide::Buy, OrderSide::Sell)]
    #[case(OrderSide::Sell, OrderSide::Buy)]
    #[case(OrderSide::NoOrderSide, OrderSide::NoOrderSide)]
    fn test_order_opposite_side(#[case] order_side: OrderSide, #[case] expected_side: OrderSide) {
        let result = OrderCore::opposite_side(order_side);
        assert_eq!(result, expected_side);
    }

    #[rstest]
    #[case(PositionSide::Long, OrderSide::Sell)]
    #[case(PositionSide::Short, OrderSide::Buy)]
    #[case(PositionSide::NoPositionSide, OrderSide::NoOrderSide)]
    fn test_closing_side(#[case] position_side: PositionSide, #[case] expected_side: OrderSide) {
        let result = OrderCore::closing_side(position_side);
        assert_eq!(result, expected_side);
    }

    #[rstest]
    #[case(OrderSide::Buy, dec!(10_000))]
    #[case(OrderSide::Sell, dec!(-10_000))]
    fn test_signed_decimal_qty(#[case] order_side: OrderSide, #[case] expected: Decimal) {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .order_side(order_side)
            .quantity(Quantity::from(10_000))
            .build()
            .unwrap()
            .into();

        let result = order.signed_decimal_qty();
        assert_eq!(result, expected);
    }

    #[rustfmt::skip]
    #[rstest]
    #[case(OrderSide::Buy, Quantity::from(100), PositionSide::Long, Quantity::from(50), false)]
    #[case(OrderSide::Buy, Quantity::from(50), PositionSide::Short, Quantity::from(50), true)]
    #[case(OrderSide::Buy, Quantity::from(50), PositionSide::Short, Quantity::from(100), true)]
    #[case(OrderSide::Buy, Quantity::from(50), PositionSide::Flat, Quantity::from(0), false)]
    #[case(OrderSide::Sell, Quantity::from(50), PositionSide::Flat, Quantity::from(0), false)]
    #[case(OrderSide::Sell, Quantity::from(50), PositionSide::Long, Quantity::from(50), true)]
    #[case(OrderSide::Sell, Quantity::from(50), PositionSide::Long, Quantity::from(100), true)]
    #[case(OrderSide::Sell, Quantity::from(100), PositionSide::Short, Quantity::from(50), false)]
    fn test_would_reduce_only(
        #[case] order_side: OrderSide,
        #[case] order_qty: Quantity,
        #[case] position_side: PositionSide,
        #[case] position_qty: Quantity,
        #[case] expected: bool,
    ) {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .order_side(order_side)
            .quantity(order_qty)
            .build()
            .unwrap()
            .into();

        assert_eq!(
            order.would_reduce_only(position_side, position_qty),
            expected
        );
    }

    #[rstest]
    fn test_order_state_transition_denied() {
        let mut order: MarketOrder = OrderInitializedBuilder::default().build().unwrap().into();
        let denied = OrderDeniedBuilder::default().build().unwrap();
        let event = OrderEventAny::Denied(denied);

        order.apply(event.clone()).unwrap();

        assert_eq!(order.status, OrderStatus::Denied);
        assert!(order.is_closed());
        assert!(!order.is_open());
        assert_eq!(order.event_count(), 2);
        assert_eq!(order.last_event(), &event);
    }

    #[rstest]
    fn test_order_life_cycle_to_filled() {
        let init = OrderInitializedBuilder::default().build().unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let filled = OrderFilledBuilder::default().build().unwrap();

        let mut order: MarketOrder = init.clone().into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(filled)).unwrap();

        assert_eq!(order.client_order_id, init.client_order_id);
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.filled_qty(), Quantity::from(100_000));
        assert_eq!(order.leaves_qty(), Quantity::from(0));
        assert_eq!(order.avg_px(), Some(1.0));
        assert!(!order.is_open());
        assert!(order.is_closed());
        assert_eq!(order.commission(&Currency::USD()), None);
        assert_eq!(order.commissions(), &IndexMap::new());
    }

    #[rstest]
    fn test_order_state_transition_to_canceled() {
        let mut order: MarketOrder = OrderInitializedBuilder::default().build().unwrap().into();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let canceled = OrderCanceledBuilder::default().build().unwrap();

        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Canceled(canceled)).unwrap();

        assert_eq!(order.status(), OrderStatus::Canceled);
        assert!(order.is_closed());
        assert!(!order.is_open());
    }

    #[rstest]
    fn test_order_life_cycle_to_partially_filled() {
        let init = OrderInitializedBuilder::default().build().unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let filled = OrderFilledBuilder::default()
            .last_qty(Quantity::from(50_000))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.clone().into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(filled)).unwrap();

        assert_eq!(order.client_order_id, init.client_order_id);
        assert_eq!(order.status(), OrderStatus::PartiallyFilled);
        assert_eq!(order.filled_qty(), Quantity::from(50_000));
        assert_eq!(order.leaves_qty(), Quantity::from(50_000));
        assert!(order.is_open());
        assert!(!order.is_closed());
    }

    #[rstest]
    fn test_order_commission_calculation() {
        let mut order: MarketOrder = OrderInitializedBuilder::default().build().unwrap().into();
        order
            .commissions
            .insert(Currency::USD(), Money::new(10.0, Currency::USD()));

        assert_eq!(
            order.commission(&Currency::USD()),
            Some(Money::new(10.0, Currency::USD()))
        );
        assert_eq!(
            order.commissions_vec(),
            vec![Money::new(10.0, Currency::USD())]
        );
    }

    #[rstest]
    fn test_order_is_primary() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .exec_algorithm_id(Some(ExecAlgorithmId::from("ALGO-001")))
            .exec_spawn_id(Some(ClientOrderId::from("O-001")))
            .client_order_id(ClientOrderId::from("O-001"))
            .build()
            .unwrap()
            .into();

        assert!(order.is_primary());
        assert!(!order.is_spawned());
    }

    #[rstest]
    fn test_order_is_spawned() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .exec_algorithm_id(Some(ExecAlgorithmId::from("ALGO-001")))
            .exec_spawn_id(Some(ClientOrderId::from("O-002")))
            .client_order_id(ClientOrderId::from("O-001"))
            .build()
            .unwrap()
            .into();

        assert!(!order.is_primary());
        assert!(order.is_spawned());
    }

    #[rstest]
    fn test_order_is_contingency() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .contingency_type(Some(ContingencyType::Oto))
            .build()
            .unwrap()
            .into();

        assert!(order.is_contingency());
        assert!(order.is_parent_order());
        assert!(!order.is_child_order());
    }

    #[rstest]
    fn test_order_is_child_order() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .parent_order_id(Some(ClientOrderId::from("PARENT-001")))
            .build()
            .unwrap()
            .into();

        assert!(order.is_child_order());
        assert!(!order.is_parent_order());
    }

    #[rstest]
    fn test_to_own_book_order_timestamp_ordering() {
        use crate::orders::limit::LimitOrder;

        // 创建具有不同时间戳的订单以验证参数顺序
        let init = OrderInitializedBuilder::default()
            .price(Some(Price::from("100.00")))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default()
            .ts_event(UnixNanos::from(1_000_000))
            .build()
            .unwrap();
        let accepted = OrderAcceptedBuilder::default()
            .ts_event(UnixNanos::from(2_000_000))
            .build()
            .unwrap();

        let mut order: LimitOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();

        let own_book_order = order.to_own_book_order();

        // 验证时间戳在正确的位置
        assert_eq!(own_book_order.ts_submitted, UnixNanos::from(1_000_000));
        assert_eq!(own_book_order.ts_accepted, UnixNanos::from(2_000_000));
        assert_eq!(own_book_order.ts_last, UnixNanos::from(2_000_000));
    }

    #[rstest]
    fn test_order_accepted_without_submitted_sets_account_id() {
        // 测试外部订单流程：Initialized -> Accepted (无 Submitted)
        let init = OrderInitializedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default()
            .account_id(AccountId::from("EXTERNAL-001"))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();

        // 验证 account_id 初始为 None
        assert_eq!(order.account_id(), None);

        // 直接应用 accepted 事件（外部订单情况）
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();

        // 验证 account_id 现在是否已从 accepted 事件中设置
        assert_eq!(order.account_id(), Some(AccountId::from("EXTERNAL-001")));
        assert_eq!(order.status(), OrderStatus::Accepted);
    }

    #[rstest]
    fn test_order_accepted_after_submitted_preserves_account_id() {
        // 测试正常订单流程：Initialized -> Submitted -> Accepted
        let init = OrderInitializedBuilder::default().build().unwrap();
        let submitted = OrderSubmittedBuilder::default()
            .account_id(AccountId::from("SUBMITTED-001"))
            .build()
            .unwrap();
        let accepted = OrderAcceptedBuilder::default()
            .account_id(AccountId::from("ACCEPTED-001"))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();

        // 提交后，account_id 应该被设置
        assert_eq!(order.account_id(), Some(AccountId::from("SUBMITTED-001")));

        // 应用 accepted 事件
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();

        // account_id 现在应该更新为 accepted 事件的 account_id
        assert_eq!(order.account_id(), Some(AccountId::from("ACCEPTED-001")));
        assert_eq!(order.status(), OrderStatus::Accepted);
    }

    #[rstest]
    fn test_overfill_tracks_overfill_qty() {
        // 测试订单是否跟踪超额成交
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let overfill = OrderFilledBuilder::default()
            .last_qty(Quantity::from(110_000)) // 超额成交：110k > 100k
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(overfill)).unwrap();

        // 订单应该跟踪超额成交
        assert_eq!(order.overfill_qty(), Quantity::from(10_000));
        assert_eq!(order.filled_qty(), Quantity::from(110_000));
        assert_eq!(order.leaves_qty(), Quantity::from(0));
        assert_eq!(order.status(), OrderStatus::Filled);
    }

    #[rstest]
    fn test_partial_fill_then_overfill() {
        // 测试多次成交导致的超额成交
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let fill1 = OrderFilledBuilder::default()
            .last_qty(Quantity::from(80_000))
            .trade_id(TradeId::from("TRADE-1"))
            .build()
            .unwrap();
        let fill2 = OrderFilledBuilder::default()
            .last_qty(Quantity::from(30_000)) // 总计 110k > 100k
            .trade_id(TradeId::from("TRADE-2"))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(fill1)).unwrap();

        // 第一次成交后，无超额成交
        assert_eq!(order.overfill_qty(), Quantity::from(0));
        assert_eq!(order.filled_qty(), Quantity::from(80_000));
        assert_eq!(order.leaves_qty(), Quantity::from(20_000));

        order.apply(OrderEventAny::Filled(fill2)).unwrap();

        // 第二次成交后，检测到超额成交
        assert_eq!(order.overfill_qty(), Quantity::from(10_000));
        assert_eq!(order.filled_qty(), Quantity::from(110_000));
        assert_eq!(order.leaves_qty(), Quantity::from(0));
        assert_eq!(order.status(), OrderStatus::Filled);
    }

    #[rstest]
    fn test_exact_fill_no_overfill() {
        // 测试精确成交不会触发超额成交跟踪
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let filled = OrderFilledBuilder::default()
            .last_qty(Quantity::from(100_000)) // 精确成交
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(filled)).unwrap();

        // 无超额成交
        assert_eq!(order.overfill_qty(), Quantity::from(0));
        assert_eq!(order.filled_qty(), Quantity::from(100_000));
        assert_eq!(order.leaves_qty(), Quantity::from(0));
    }

    #[rstest]
    fn test_partial_fill_then_overfill_with_fractional_quantities() {
        // 模拟具有小数成交的真实交易所场景：
        // 订单数量 2450.5，部分成交 1202.5，然后 1285.5 的成交到达
        // 总成交：2488.0，超额成交：37.5
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from("2450.5"))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let fill1 = OrderFilledBuilder::default()
            .last_qty(Quantity::from("1202.5"))
            .trade_id(TradeId::from("TRADE-1"))
            .build()
            .unwrap();
        let fill2 = OrderFilledBuilder::default()
            .last_qty(Quantity::from("1285.5")) // 1202.5 + 1285.5 = 2488 > 2450.5
            .trade_id(TradeId::from("TRADE-2"))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(fill1)).unwrap();

        // 第一次成交后，无超额成交
        assert_eq!(order.overfill_qty(), Quantity::from(0));
        assert_eq!(order.filled_qty(), Quantity::from("1202.5"));
        assert_eq!(order.leaves_qty(), Quantity::from("1248.0"));
        assert_eq!(order.status(), OrderStatus::PartiallyFilled);

        order.apply(OrderEventAny::Filled(fill2)).unwrap();

        // 第二次成交后，检测并跟踪到超额成交
        assert_eq!(order.overfill_qty(), Quantity::from("37.5"));
        assert_eq!(order.filled_qty(), Quantity::from("2488.0"));
        assert_eq!(order.leaves_qty(), Quantity::from(0));
        assert_eq!(order.status(), OrderStatus::Filled);
    }

    #[rstest]
    fn test_calculate_overfill_returns_zero_when_no_overfill() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap()
            .into();

        // 成交数量小于订单数量 - 无超额成交
        let overfill = order.calculate_overfill(Quantity::from(50_000));
        assert_eq!(overfill, Quantity::from(0));

        // Fill qty equals order qty - no overfill
        let overfill = order.calculate_overfill(Quantity::from(100_000));
        assert_eq!(overfill, Quantity::from(0));
    }

    #[rstest]
    fn test_calculate_overfill_returns_overfill_amount() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap()
            .into();

        // 成交数量超过订单数量
        let overfill = order.calculate_overfill(Quantity::from(110_000));
        assert_eq!(overfill, Quantity::from(10_000));
    }

    #[rstest]
    fn test_calculate_overfill_accounts_for_existing_fills() {
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let partial_fill = OrderFilledBuilder::default()
            .last_qty(Quantity::from(60_000))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(partial_fill)).unwrap();

        // 订单已成交 60k，剩余 40k
        // 50k 的成交将超额成交 10k
        let overfill = order.calculate_overfill(Quantity::from(50_000));
        assert_eq!(overfill, Quantity::from(10_000));

        // Fill of 40k would not overfill
        let overfill = order.calculate_overfill(Quantity::from(40_000));
        assert_eq!(overfill, Quantity::from(0));
    }

    #[rstest]
    fn test_calculate_overfill_with_fractional_quantities() {
        let order: MarketOrder = OrderInitializedBuilder::default()
            .quantity(Quantity::from("2450.5"))
            .build()
            .unwrap()
            .into();

        // 模拟用户日志中的确切场景
        // 订单 2450.5，如果 2488.0 的成交到达
        let overfill = order.calculate_overfill(Quantity::from("2488.0"));
        assert_eq!(overfill, Quantity::from("37.5"));
    }

    #[rstest]
    fn test_duplicate_fill_rejected() {
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let fill1 = OrderFilledBuilder::default()
            .last_qty(Quantity::from(50_000))
            .trade_id(TradeId::from("TRADE-001"))
            .build()
            .unwrap();
        let fill2_duplicate = OrderFilledBuilder::default()
            .last_qty(Quantity::from(50_000))
            .trade_id(TradeId::from("TRADE-001")) // 与 fill1 相同的 trade_id
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(fill1)).unwrap();

        // 验证第一次成交应用成功
        assert_eq!(order.filled_qty(), Quantity::from(50_000));
        assert_eq!(order.status(), OrderStatus::PartiallyFilled);

        // 应用重复的成交应返回 DuplicateFill 错误
        let result = order.apply(OrderEventAny::Filled(fill2_duplicate));
        assert!(result.is_err());
        match result.unwrap_err() {
            OrderError::DuplicateFill(trade_id) => {
                assert_eq!(trade_id, TradeId::from("TRADE-001"));
            }
            e => panic!("Expected DuplicateFill error, was: {e:?}"),
        }

        // 拒绝重复后订单状态应保持不变
        assert_eq!(order.filled_qty(), Quantity::from(50_000));
        assert_eq!(order.status(), OrderStatus::PartiallyFilled);
    }

    #[rstest]
    fn test_different_trade_ids_allowed() {
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let fill1 = OrderFilledBuilder::default()
            .last_qty(Quantity::from(50_000))
            .trade_id(TradeId::from("TRADE-001"))
            .build()
            .unwrap();
        let fill2 = OrderFilledBuilder::default()
            .last_qty(Quantity::from(50_000))
            .trade_id(TradeId::from("TRADE-002")) // 不同的 trade_id
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(fill1)).unwrap();
        order.apply(OrderEventAny::Filled(fill2)).unwrap();

        // 两个成交都应该被应用
        assert_eq!(order.filled_qty(), Quantity::from(100_000));
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.trade_ids.len(), 2);
    }

    #[rstest]
    fn test_partially_filled_order_can_be_updated() {
        // 测试部分成交的订单可以接收 Updated 事件
        // 并保持 PartiallyFilled 状态
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let partial_fill = OrderFilledBuilder::default()
            .last_qty(Quantity::from(40_000))
            .build()
            .unwrap();
        let updated = OrderUpdatedBuilder::default()
            .quantity(Quantity::from(80_000)) // 减少到 80k (仍然 > 40k 已成交)
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Filled(partial_fill)).unwrap();

        assert_eq!(order.status(), OrderStatus::PartiallyFilled);
        assert_eq!(order.filled_qty(), Quantity::from(40_000));

        order.apply(OrderEventAny::Updated(updated)).unwrap();

        assert_eq!(order.status(), OrderStatus::PartiallyFilled);
        assert_eq!(order.quantity(), Quantity::from(80_000));
        assert_eq!(order.leaves_qty(), Quantity::from(40_000)); // 80k - 40k 已成交
    }

    #[rstest]
    fn test_triggered_order_can_be_updated() {
        // 测试已触发的订单可以接收 Updated 事件
        // 并保持 Triggered 状态
        let init = OrderInitializedBuilder::default()
            .quantity(Quantity::from(100_000))
            .build()
            .unwrap();
        let submitted = OrderSubmittedBuilder::default().build().unwrap();
        let accepted = OrderAcceptedBuilder::default().build().unwrap();
        let triggered = OrderTriggeredBuilder::default().build().unwrap();
        let updated = OrderUpdatedBuilder::default()
            .quantity(Quantity::from(80_000))
            .build()
            .unwrap();

        let mut order: MarketOrder = init.into();
        order.apply(OrderEventAny::Submitted(submitted)).unwrap();
        order.apply(OrderEventAny::Accepted(accepted)).unwrap();
        order.apply(OrderEventAny::Triggered(triggered)).unwrap();

        assert_eq!(order.status(), OrderStatus::Triggered);

        order.apply(OrderEventAny::Updated(updated)).unwrap();

        assert_eq!(order.status(), OrderStatus::Triggered);
        assert_eq!(order.quantity(), Quantity::from(80_000));
    }
}
