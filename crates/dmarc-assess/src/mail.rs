//! Rates and posture: delivered compliance, DKIM coverage, health,
//! enforcement and the DNS grade.

use crate::status::Status;
use crate::trend::{points_trend, Trend, TrendSense};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// From this share of good mail on, a rate passes (green).
pub const RATE_PASS_MIN_PCT: f64 = 99.0;
/// From this share on, a rate warns (amber); below it, it fails (red).
pub const RATE_WARN_MIN_PCT: f64 = 95.0;

/// The one rule for every share of good mail (delivered compliance, DKIM
/// signed share, a sender's pass rate): 99 / 95, with no volume minimum.
pub fn rate_status(pct: f64) -> Status {
    if pct >= RATE_PASS_MIN_PCT {
        Status::Pass
    } else if pct >= RATE_WARN_MIN_PCT {
        Status::Warn
    } else {
        Status::Fail
    }
}

/// A percentage (0–100) and its verdict. `pct` is `null` when there is
/// nothing to measure; the status is then `neutral`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Rate {
    pub pct: Option<f64>,
    pub status: Status,
}

impl Rate {
    /// `part / whole` judged by [`rate_status`]; unmeasured without a whole.
    pub fn of(part: i64, whole: i64) -> Self {
        if whole <= 0 {
            return Self::unmeasured();
        }
        let pct = part as f64 * 100.0 / whole as f64;
        Self {
            pct: Some(pct),
            status: rate_status(pct),
        }
    }

    pub fn unmeasured() -> Self {
        Self::default()
    }
}

/// The raw pass rate, `compliant / total`: a sender's figure, where blocked
/// mail from a legitimate source is lost mail and counts against it.
/// Unmeasured without mail.
pub fn pass_rate(compliant: i64, total: i64) -> Rate {
    Rate::of(compliant, total)
}

/// Message totals of a scope by outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mail {
    pub total: i64,
    pub compliant: i64,
    pub delivered_failures: i64,
    pub forwarded_failures: i64,
}

impl Mail {
    /// Failures the receiver quarantined or rejected: the policy working.
    pub fn blocked(&self) -> i64 {
        (self.total - self.compliant - self.delivered_failures - self.forwarded_failures).max(0)
    }

    /// Every DMARC failure, whatever became of it: delivered failures,
    /// forwarded and blocked mail together. Not a bare "failed" count, which
    /// leaves forwarded mail out; the name says so.
    pub fn dmarc_failed_incl_forwarded(&self) -> i64 {
        (self.total - self.compliant).max(0)
    }

    /// The mail delivered compliance is measured over: what passed DMARC
    /// and the failures still delivered. Blocked and forwarded mail is not
    /// in it.
    pub fn delivered(&self) -> i64 {
        self.compliant + self.delivered_failures
    }

    /// Delivered compliance: the share of *delivered* mail that authenticated.
    ///
    /// Blocked failures are left out on purpose (DMARC working as designed),
    /// and so is forwarded mail. Unmeasured when nothing was delivered.
    pub fn delivered_compliance(&self) -> Rate {
        Rate::of(self.compliant, self.delivered())
    }

    /// The change of [`Self::delivered_compliance`] from `previous`, in
    /// percentage points; `None` when either delivered nothing.
    pub fn delivered_compliance_trend(&self, previous: &Self) -> Option<Trend> {
        let now = self.delivered_compliance().pct?;
        let before = previous.delivered_compliance().pct?;
        Some(points_trend(now - before, TrendSense::HigherIsBetter))
    }

    /// The raw [`pass_rate`] of this mail.
    pub fn pass_rate(&self) -> Rate {
        pass_rate(self.compliant, self.total)
    }

    /// The figures and verdicts sent beside these counts.
    pub fn verdicts(&self) -> MailVerdicts {
        MailVerdicts {
            blocked_messages: self.blocked(),
            delivered_messages: self.delivered(),
            delivered_compliance: self.delivered_compliance(),
            health: self.health(),
        }
    }

    /// Health from delivered compliance alone; enforcement never feeds in.
    pub fn health(&self) -> Health {
        let state = if self.total <= 0 {
            HealthState::NoData
        } else {
            match self.delivered_compliance().pct {
                None => HealthState::AllBlocked,
                Some(pct) => match rate_status(pct) {
                    Status::Pass => HealthState::Protected,
                    Status::Warn => HealthState::Warning,
                    _ => HealthState::Critical,
                },
            }
        };
        Health {
            state,
            status: state.status(),
        }
    }

    /// One verdict for a set of outcomes, worst wins: any delivered failure
    /// fails, else forwarded, else blocked (info), else passed.
    pub fn outcome_status(&self) -> Status {
        if self.delivered_failures > 0 {
            Status::Fail
        } else if self.forwarded_failures > 0 {
            Status::Forward
        } else if self.blocked() > 0 {
            Status::Info
        } else if self.compliant > 0 {
            Status::Pass
        } else {
            Status::Neutral
        }
    }
}

/// What is worked out from a set of message counts, sent beside them at the
/// same level wherever both are (statistics, a report, a domain): the
/// blocked count, delivered compliance and health.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct MailVerdicts {
    /// Failures quarantined or rejected.
    pub blocked_messages: i64,
    /// Messages receivers delivered: passed plus delivered failures, the
    /// whole that delivered compliance is a share of.
    pub delivered_messages: i64,
    pub delivered_compliance: Rate,
    pub health: Health,
}

/// Is the delivered mail authentic?
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    #[default]
    NoData,
    /// Mail was seen, nothing was delivered: every failure was blocked.
    AllBlocked,
    Protected,
    Warning,
    Critical,
}

impl HealthState {
    /// The verdict colour of each state: all blocked is blue (spoofing
    /// stopped), no data states nothing.
    pub fn status(self) -> Status {
        match self {
            Self::NoData => Status::Neutral,
            Self::AllBlocked => Status::Info,
            Self::Protected => Status::Pass,
            Self::Warning => Status::Warn,
            Self::Critical => Status::Fail,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Health {
    pub state: HealthState,
    pub status: Status,
}

/// What receivers are asked to do with failing mail (`p=`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementState {
    Reject,
    Quarantine,
    /// `p=none`: nothing is blocked; spoofed mail would be delivered.
    Monitor,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Enforcement {
    pub state: EnforcementState,
    pub status: Status,
}

/// Enforcement posture of a published `p=` (or `sp=`) value. Exposure, not
/// damage: `none` fails even while authentication looks perfect.
pub fn enforcement(policy: Option<&str>) -> Enforcement {
    let policy = policy.map(|p| p.trim().to_ascii_lowercase());
    let (state, status) = match policy.as_deref() {
        Some("reject") => (EnforcementState::Reject, Status::Pass),
        Some("quarantine") => (EnforcementState::Quarantine, Status::Warn),
        Some("none") => (EnforcementState::Monitor, Status::Fail),
        _ => (EnforcementState::Unknown, Status::Neutral),
    };
    Enforcement { state, status }
}

/// How much of the passing mail the domain signs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    /// Nothing passed DMARC.
    #[default]
    Unmeasured,
    /// None of the passing mail is signed by the domain.
    None,
    Partial,
    Full,
}

/// DKIM coverage: a [`Rate`] plus which of the coverage states applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DkimCoverage {
    pub pct: Option<f64>,
    pub status: Status,
    pub state: CoverageState,
}

/// DKIM coverage: the share of DMARC-passing mail signed by the domain
/// itself. The rest passed on SPF alone and breaks when forwarded.
pub fn dkim_coverage(compliant: i64, no_aligned_dkim: i64) -> DkimCoverage {
    let rate = Rate::of((compliant - no_aligned_dkim).max(0), compliant);
    let state = if compliant <= 0 {
        CoverageState::Unmeasured
    } else if no_aligned_dkim >= compliant {
        CoverageState::None
    } else if no_aligned_dkim > 0 {
        CoverageState::Partial
    } else {
        CoverageState::Full
    };
    DkimCoverage {
        pct: rate.pct,
        status: rate.status,
        state,
    }
}

/// Verdict on a DNS grade. A critical finding fails the domain whatever the
/// letter (the score gives partial credit). A/B pass, C warns, lower fails;
/// no grade is unknown, not broken.
pub fn grade_status(grade: Option<&str>, critical_count: i64) -> Status {
    let Some(grade) = grade.map(str::trim).filter(|g| !g.is_empty()) else {
        return Status::Neutral;
    };
    if critical_count > 0 {
        return Status::Fail;
    }
    match grade.to_ascii_uppercase().as_str() {
        "A" | "B" => Status::Pass,
        "C" => Status::Warn,
        _ => Status::Fail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(total: i64, compliant: i64, delivered: i64, forwarded: i64) -> Mail {
        Mail {
            total,
            compliant,
            delivered_failures: delivered,
            forwarded_failures: forwarded,
        }
    }

    #[test]
    fn the_99_95_rule() {
        assert_eq!(rate_status(99.0), Status::Pass);
        assert_eq!(rate_status(98.99), Status::Warn);
        assert_eq!(rate_status(95.0), Status::Warn);
        assert_eq!(rate_status(94.9), Status::Fail);
    }

    #[test]
    fn delivered_compliance_ignores_blocked_and_forwarded() {
        let m = mail(1000, 900, 5, 20);
        assert_eq!(m.blocked(), 75);
        assert_eq!(m.dmarc_failed_incl_forwarded(), 100);
        assert_eq!(m.delivered(), 905);
        // The verdicts carry the whole the rate is a share of: 18 of 19
        // delivered messages passed, all 19 were delivered.
        let one_failure = mail(19, 18, 1, 0).verdicts();
        assert_eq!(one_failure.delivered_messages, 19);
        assert!((one_failure.delivered_compliance.pct.unwrap() - 18.0 * 100.0 / 19.0).abs() < 1e-9);
        let rate = m.delivered_compliance();
        assert!((rate.pct.unwrap() - 900.0 * 100.0 / 905.0).abs() < 1e-9);
        assert_eq!(rate.status, Status::Pass);
        assert_eq!(mail(10, 0, 0, 0).delivered_compliance(), Rate::unmeasured());
    }

    #[test]
    fn delivered_compliance_trend_needs_delivered_mail_on_both_sides() {
        // 90% then 95% delivered compliance: up five points, green.
        let before = mail(200, 90, 10, 0);
        let now = mail(300, 95, 5, 100);
        let trend = now.delivered_compliance_trend(&before).unwrap();
        assert!((trend.delta.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(trend.status, Status::Pass);
        let all_blocked = mail(10, 0, 0, 0);
        assert_eq!(now.delivered_compliance_trend(&all_blocked), None);
        assert_eq!(all_blocked.delivered_compliance_trend(&now), None);
    }

    #[test]
    fn pass_rate_counts_blocked_mail_against_the_sender() {
        let m = mail(100, 60, 0, 0);
        assert_eq!(m.pass_rate().pct, Some(60.0));
        assert_eq!(m.pass_rate().status, Status::Fail);
        assert_eq!(m.delivered_compliance().pct, Some(100.0));
        assert_eq!(mail(0, 0, 0, 0).pass_rate(), Rate::unmeasured());
    }

    #[test]
    fn health_states() {
        assert_eq!(mail(0, 0, 0, 0).health().state, HealthState::NoData);
        assert_eq!(mail(10, 0, 0, 0).health().state, HealthState::AllBlocked);
        assert_eq!(mail(10, 0, 0, 0).health().status, Status::Info);
        assert_eq!(mail(100, 99, 1, 0).health().state, HealthState::Protected);
        assert_eq!(mail(100, 96, 4, 0).health().state, HealthState::Warning);
        assert_eq!(mail(100, 50, 50, 0).health().state, HealthState::Critical);
    }

    #[test]
    fn outcome_status_worst_wins() {
        assert_eq!(mail(10, 5, 1, 1).outcome_status(), Status::Fail);
        assert_eq!(mail(10, 5, 0, 1).outcome_status(), Status::Forward);
        assert_eq!(mail(10, 5, 0, 0).outcome_status(), Status::Info);
        assert_eq!(mail(5, 5, 0, 0).outcome_status(), Status::Pass);
        assert_eq!(mail(0, 0, 0, 0).outcome_status(), Status::Neutral);
    }

    #[test]
    fn enforcement_states() {
        assert_eq!(enforcement(Some(" Reject")).state, EnforcementState::Reject);
        assert_eq!(enforcement(Some("quarantine")).status, Status::Warn);
        assert_eq!(enforcement(Some("none")).state, EnforcementState::Monitor);
        assert_eq!(enforcement(Some("none")).status, Status::Fail);
        assert_eq!(enforcement(None).state, EnforcementState::Unknown);
    }

    #[test]
    fn dkim_coverage_is_the_signed_share() {
        let c = dkim_coverage(1000, 5);
        assert!((c.pct.unwrap() - 99.5).abs() < 1e-9);
        assert_eq!(c.status, Status::Pass);
        assert_eq!(c.state, CoverageState::Partial);
        assert_eq!(dkim_coverage(100, 100).pct, Some(0.0));
        assert_eq!(dkim_coverage(0, 0), DkimCoverage::default());
    }

    #[test]
    fn dkim_coverage_states() {
        assert_eq!(dkim_coverage(0, 0).state, CoverageState::Unmeasured);
        assert_eq!(dkim_coverage(10, 10).state, CoverageState::None);
        assert_eq!(dkim_coverage(10, 10).status, Status::Fail);
        assert_eq!(dkim_coverage(10, 3).state, CoverageState::Partial);
        assert_eq!(dkim_coverage(10, 0).state, CoverageState::Full);
        assert_eq!(dkim_coverage(10, 0).status, Status::Pass);
    }

    #[test]
    fn grade_status_lets_a_critical_finding_fail_any_letter() {
        assert_eq!(grade_status(Some("A"), 0), Status::Pass);
        assert_eq!(grade_status(Some("b"), 0), Status::Pass);
        assert_eq!(grade_status(Some("C"), 0), Status::Warn);
        assert_eq!(grade_status(Some("D"), 0), Status::Fail);
        assert_eq!(grade_status(Some("A"), 1), Status::Fail);
        assert_eq!(grade_status(None, 3), Status::Neutral);
    }
}
