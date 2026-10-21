//! The guide's one figure the other endpoints do not serve: how many
//! domains are at each policy.

use dmarc_assess::EnforcementState;
use serde::Serialize;
use ts_rs::TS;

/// `GET /api/guide`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct GuideResponse {
    pub policies: PolicyCounts,
}

/// The monitored domains by the enforcement of their published `p=`, as
/// each domain's assessment decides it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PolicyCounts {
    /// `p=none`.
    pub monitor: u32,
    pub quarantine: u32,
    pub reject: u32,
    /// No policy known: no record, not checked yet, or the check failed.
    pub unknown: u32,
}

impl PolicyCounts {
    /// Counts `states`, one per domain.
    pub fn of(states: impl IntoIterator<Item = EnforcementState>) -> Self {
        let mut counts = Self::default();
        for state in states {
            let n = match state {
                EnforcementState::Monitor => &mut counts.monitor,
                EnforcementState::Quarantine => &mut counts.quarantine,
                EnforcementState::Reject => &mut counts.reject,
                EnforcementState::Unknown => &mut counts.unknown,
            };
            *n += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_state() {
        use EnforcementState::*;
        let counts = PolicyCounts::of([Monitor, Reject, Monitor, Unknown]);
        assert_eq!(
            counts,
            PolicyCounts {
                monitor: 2,
                quarantine: 0,
                reject: 1,
                unknown: 1
            }
        );
    }
}
