use super::{SessionProvider, TradingPhase};
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;

/// A 股交易时段查表
/// 所有时间边界以 HH*3600 + MM*60 + SS 编码为秒级偏移
const PHASES: &[(u32, u32, TradingPhase)] = &[
    (9 * 3600 + 15 * 60, 9 * 3600 + 20 * 60, TradingPhase::PreAuctionOpen),
    (9 * 3600 + 20 * 60, 9 * 3600 + 25 * 60, TradingPhase::PreAuctionLocked),
    (9 * 3600 + 25 * 60, 9 * 3600 + 30 * 60, TradingPhase::PreAuctionSilent),
    (9 * 3600 + 30 * 60, 11 * 3600 + 30 * 60, TradingPhase::ContinuousAm),
    (11 * 3600 + 30 * 60, 13 * 3600, TradingPhase::MiddayBreak),
    (13 * 3600, 14 * 3600 + 57 * 60, TradingPhase::ContinuousPm),
    (14 * 3600 + 57 * 60, 15 * 3600, TradingPhase::ClosingAuction),
];

#[derive(Debug)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.common")
)]
pub struct AShareSessionProvider;

#[cfg_attr(feature = "python", pyo3::pymethods)]
impl AShareSessionProvider {
    #[cfg(feature = "python")]
    #[new]
    pub fn new() -> Self {
        Self
    }

    #[cfg(feature = "python")]
    #[pyo3(name = "phase_at")]
    pub fn py_phase_at(&self, ts_ns: u64) -> super::TradingPhase {
        // We can just create a dummy venue since the phase calculation for Ashare only looks at time
        let venue = Venue::new(ustr::ustr("XSHG"));
        <Self as SessionProvider>::phase_at(self, &venue, nautilus_core::UnixNanos::from(ts_ns))
    }
}

impl SessionProvider for AShareSessionProvider {
    fn phase_at(&self, _venue: &Venue, ts_ns: UnixNanos) -> TradingPhase {
        let secs = ts_ns.as_u64() / 1_000_000_000;
        // UTC+8 offset, then modulo seconds in a day
        let seconds_of_day = ((secs + 8 * 3600) % 86400) as u32;

        for &(start, end, phase) in PHASES {
            if seconds_of_day >= start && seconds_of_day < end {
                return phase;
            }
        }
        TradingPhase::Closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nautilus_core::UnixNanos;
    use nautilus_model::identifiers::Venue;
    use ustr::ustr;

    fn get_ts(hour: u32, minute: u32) -> UnixNanos {
        // Base timestamp format: we just need SECONDS OF DAY to be correctly offset.
        // let seconds_of_day = ((secs + 8 * 3600) % 86400) as u32;
        // So secs = hour_utc * 3600 + minute * 60
        // hour_utc = hour - 8
        let hour_utc = if hour >= 8 { hour - 8 } else { hour + 24 - 8 };
        let secs = hour_utc * 3600 + minute * 60;
        UnixNanos::from(secs as u64 * 1_000_000_000)
    }

    #[test]
    fn test_ashare_phases() {
        let provider = AShareSessionProvider;
        let venue = Venue::new(ustr("XSHG"));

        // 09:14 -> Closed
        assert_eq!(
            provider.phase_at(&venue, get_ts(9, 14)),
            TradingPhase::Closed
        );
        // 09:15 -> PreAuctionOpen
        assert_eq!(
            provider.phase_at(&venue, get_ts(9, 15)),
            TradingPhase::PreAuctionOpen
        );
        // 09:21 -> PreAuctionLocked
        assert_eq!(
            provider.phase_at(&venue, get_ts(9, 21)),
            TradingPhase::PreAuctionLocked
        );
        // 09:26 -> PreAuctionSilent
        assert_eq!(
            provider.phase_at(&venue, get_ts(9, 26)),
            TradingPhase::PreAuctionSilent
        );
        // 09:30 -> ContinuousAm
        assert_eq!(
            provider.phase_at(&venue, get_ts(9, 30)),
            TradingPhase::ContinuousAm
        );
        // 11:30 -> MiddayBreak
        assert_eq!(
            provider.phase_at(&venue, get_ts(11, 30)),
            TradingPhase::MiddayBreak
        );
        // 13:00 -> ContinuousPm
        assert_eq!(
            provider.phase_at(&venue, get_ts(13, 0)),
            TradingPhase::ContinuousPm
        );
        // 14:57 -> ClosingAuction
        assert_eq!(
            provider.phase_at(&venue, get_ts(14, 57)),
            TradingPhase::ClosingAuction
        );
        // 15:00 -> Closed
        assert_eq!(
            provider.phase_at(&venue, get_ts(15, 00)),
            TradingPhase::Closed
        );
    }
}
