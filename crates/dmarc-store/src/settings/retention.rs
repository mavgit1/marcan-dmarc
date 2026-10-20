//! How long reports are kept.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// [`RetentionSettings::report_retention_months`] when nothing is stored.
pub const DEFAULT_REPORT_RETENTION_MONTHS: u32 = 12;

/// Accepted range for [`RetentionSettings::report_retention_months`]: a month to ten years.
pub const REPORT_RETENTION_MONTHS_RANGE: (u32, u32) = (1, 120);

/// How long reports are kept. The API carries it unchanged, in both
/// directions; it is saved clamped to the range above ([`Self::clamped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RetentionSettings {
    /// Months a report is kept (by its end date) before the daily clean-up deletes it.
    pub report_retention_months: u32,
}

impl SettingsGroup for RetentionSettings {
    const KEY: &'static str = "retention";
}

impl Default for RetentionSettings {
    fn default() -> Self {
        Self {
            report_retention_months: DEFAULT_REPORT_RETENTION_MONTHS,
        }
    }
}

impl RetentionSettings {
    /// The months clamped to their range, as they are saved.
    pub fn clamped(self) -> Self {
        let (min, max) = REPORT_RETENTION_MONTHS_RANGE;
        Self {
            report_retention_months: self.report_retention_months.clamp(min, max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_months_are_clamped_to_their_range() {
        for (months, saved) in [(0, 1), (24, 24), (500, 120)] {
            let r = RetentionSettings {
                report_retention_months: months,
            };
            assert_eq!(r.clamped().report_retention_months, saved);
        }
    }
}
