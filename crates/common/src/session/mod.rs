pub mod ashare;

use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;

/// A 股交易阶段枚举
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(
        frozen, eq, eq_int,
        module = "nautilus_trader.core.nautilus_pyo3.common",
        rename_all = "SCREAMING_SNAKE_CASE",
    )
)]
pub enum TradingPhase {
    /// 09:15-09:20 开盘集合竞价（可撤）
    PreAuctionOpen = 1,
    /// 09:20-09:25 开盘集合竞价（不可撤）
    PreAuctionLocked = 2,
    /// 09:25-09:30 静默期
    PreAuctionSilent = 3,
    /// 09:30-11:30 上午连续竞价
    ContinuousAm = 4,
    /// 11:30-13:00 午间休市
    MiddayBreak = 5,
    /// 13:00-14:57 下午连续竞价
    ContinuousPm = 6,
    /// 14:57-15:00 收盘集合竞价
    ClosingAuction = 7,
    /// 非交易时段
    Closed = 8,
}

impl TradingPhase {
    /// 该阶段是否接受新订单
    #[must_use]
    pub const fn can_accept_order(&self) -> bool {
        matches!(
            self,
            Self::PreAuctionOpen
                | Self::PreAuctionLocked
                | Self::ContinuousAm
                | Self::ContinuousPm
                | Self::ClosingAuction
        )
    }

    /// 该阶段是否接受撤单
    #[must_use]
    pub const fn can_cancel_order(&self) -> bool {
        matches!(
            self,
            Self::PreAuctionOpen | Self::ContinuousAm | Self::ContinuousPm
        )
    }

    /// 该阶段是否为连续竞价
    #[must_use]
    pub const fn is_continuous(&self) -> bool {
        matches!(self, Self::ContinuousAm | Self::ContinuousPm)
    }

    /// 该阶段是否为集合竞价
    #[must_use]
    pub const fn is_auction(&self) -> bool {
        matches!(
            self,
            Self::PreAuctionOpen | Self::PreAuctionLocked | Self::ClosingAuction
        )
    }
}

/// 交易时段查询 trait
pub trait SessionProvider: Send + Sync {
    fn phase_at(&self, venue: &Venue, ts_ns: UnixNanos) -> TradingPhase;
}
