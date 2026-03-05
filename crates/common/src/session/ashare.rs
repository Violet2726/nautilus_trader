use super::{SessionProvider, TradingPhase};
use chrono::{Datelike, TimeZone, Utc, Weekday};
use nautilus_core::UnixNanos;
use nautilus_model::identifiers::Venue;
use std::collections::HashSet;

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

#[derive(Debug, Default, Clone)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(module = "nautilus_trader.core.nautilus_pyo3.common", from_py_object)
)]
pub struct AShareSessionProvider {
    /// 休市日集合，格式为 YYYYMMDD 的整数（例如 20240501）
    holidays: HashSet<u32>,
}

#[cfg_attr(feature = "python", pyo3::pymethods)]
impl AShareSessionProvider {
    #[cfg(feature = "python")]
    #[new]
    #[pyo3(signature = (holidays=None))]
    pub fn new(holidays: Option<Vec<u32>>) -> Self {
        Self {
            holidays: holidays.unwrap_or_default().into_iter().collect(),
        }
    }

    #[cfg(not(feature = "python"))]
    pub fn new(holidays: Option<Vec<u32>>) -> Self {
        Self {
            holidays: holidays.unwrap_or_default().into_iter().collect(),
        }
    }

    #[cfg(feature = "python")]
    #[pyo3(name = "phase_at")]
    pub fn py_phase_at(&self, ts_ns: u64) -> super::TradingPhase {
        // 我们可以直接创建一个虚拟场地，因为 A 股的阶段计算仅由时间决定
        let venue = Venue::new(ustr::ustr("XSHG"));
        <Self as SessionProvider>::phase_at(self, &venue, nautilus_core::UnixNanos::from(ts_ns))
    }
}

impl SessionProvider for AShareSessionProvider {
    fn phase_at(&self, _venue: &Venue, ts_ns: UnixNanos) -> TradingPhase {
        let secs = ts_ns.as_u64() / 1_000_000_000;

        // 1. 获取北京时间对应的日期和星期
        // 我们需要 +8 小时的偏移量
        if let Some(dt) = Utc.timestamp_opt(secs as i64 + 8 * 3600, 0).single() {
            // 周六和周日一律闭市
            let weekday = dt.weekday();
            if weekday == Weekday::Sat || weekday == Weekday::Sun {
                return TradingPhase::Closed;
            }

            // 检查是否在节假日表中
            let yyyymmdd =
                (dt.year() as u32) * 10000 + (dt.month() as u32) * 100 + (dt.day() as u32);
            if self.holidays.contains(&yyyymmdd) {
                return TradingPhase::Closed;
            }
        } else {
            return TradingPhase::Closed;
        }

        // 2. 正常交易日的按秒偏移段判断
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
        // 基础时间戳格式：我们只需要确保“当天的秒数”具有正确的偏移量。
        // let seconds_of_day = ((secs + 8 * 3600) % 86400) as u32;
        // 因此 secs = hour_utc * 3600 + minute * 60
        // hour_utc = hour - 8 (UTC)
        let hour_utc = if hour >= 8 { hour - 8 } else { hour + 24 - 8 };
        let secs = hour_utc * 3600 + minute * 60;
        UnixNanos::from(secs as u64 * 1_000_000_000)
    }

    #[test]
    fn test_ashare_phases() {
        let provider = AShareSessionProvider::default();
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
