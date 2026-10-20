//! When missing reports warn.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// [`MonitoringSettings::report_freshness_warning_hours`] when nothing is stored.
pub const DEFAULT_REPORT_FRESHNESS_WARNING_HOURS: u32 = 24;

/// Accepted range for [`MonitoringSettings::report_freshness_warning_hours`]: one hour to a week.
pub const REPORT_FRESHNESS_WARNING_HOURS_RANGE: (u32, u32) = (1, 168);

/// [`MonitoringSettings::domain_reports_stopped_days`] when nothing is stored.
pub const DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS: u32 = 14;

/// Accepted range for [`MonitoringSettings::domain_reports_stopped_days`]: a day to a month.
pub const DOMAIN_REPORTS_STOPPED_DAYS_RANGE: (u32, u32) = (1, 30);

/// When missing reports warn. The API carries it unchanged, in both
/// directions; it is saved clamped to the ranges above ([`Self::clamped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MonitoringSettings {
    /// Hours without a new report, for the mailbox as a whole, before the
    /// pipeline status warns.
    pub report_freshness_warning_hours: u32,
    /// Whole days without a report before a domain's reports count as
    /// stopped (the `reports_stopped` issue and its alert).
    pub domain_reports_stopped_days: u32,
}

impl SettingsGroup for MonitoringSettings {
    const KEY: &'static str = "monitoring";
}

impl Default for MonitoringSettings {
    fn default() -> Self {
        Self {
            report_freshness_warning_hours: DEFAULT_REPORT_FRESHNESS_WARNING_HOURS,
            domain_reports_stopped_days: DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS,
        }
    }
}

impl MonitoringSettings {
    /// Each value clamped to its range, as it is saved.
    pub fn clamped(self) -> Self {
        let (min, max) = REPORT_FRESHNESS_WARNING_HOURS_RANGE;
        let (min_days, max_days) = DOMAIN_REPORTS_STOPPED_DAYS_RANGE;
        Self {
            report_freshness_warning_hours: self.report_freshness_warning_hours.clamp(min, max),
            domain_reports_stopped_days: self.domain_reports_stopped_days.clamp(min_days, max_days),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_values_are_clamped_to_their_ranges() {
        let clamp = |hours, days| {
            let m = MonitoringSettings {
                report_freshness_warning_hours: hours,
                domain_reports_stopped_days: days,
            }
            .clamped();
            (
                m.report_freshness_warning_hours,
                m.domain_reports_stopped_days,
            )
        };
        assert_eq!(clamp(0, 0), (1, 1));
        assert_eq!(clamp(500, 90), (168, 30));
        assert_eq!(clamp(48, 10), (48, 10));
    }
}
