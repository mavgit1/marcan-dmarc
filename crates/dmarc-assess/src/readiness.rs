//! Policy-step readiness: when a domain can move to the next DMARC policy
//! (`p=none` → `p=quarantine` → `p=reject`).
//!
//! Judged over a fixed window ([`READINESS_WINDOW_DAYS`]) that ignores the
//! UI's time range, so the verdict does not change when the range does, and
//! on the evidence gathered under the current policy (the DNS record's `p=`):
//! each report carries the policy its receiver applied, and mail evaluated
//! under an older one counts toward neither the message minimum nor the days
//! of history. So publishing `p=quarantine` restarts the clock for `p=reject`.

use crate::mail::{enforcement, EnforcementState, Mail, Rate, RATE_PASS_MIN_PCT};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Readiness looks at the last this many days, whatever range the page shows,
/// and needs reports covering all of them.
pub const READINESS_WINDOW_DAYS: i64 = 30;

/// A domain with fewer messages than this in the window, under its current
/// policy, is never called ready: a handful of passing messages says nothing
/// about the senders that did not show up yet.
pub const READINESS_MIN_MESSAGES: i64 = 200;

/// At least this share of the window's messages must come from reports
/// evaluated under the current policy. A receiver that still applies the old
/// one (it read DNS before the change, or reports late) lowers the share; a
/// few such stragglers do not block the verdict.
pub const READINESS_MIN_POLICY_SHARE_PCT: f64 = 95.0;

const DAY_SECS: i64 = 86_400;

/// The policy a domain can move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStep {
    Quarantine,
    Reject,
}

/// Why a domain is not ready for its next step: one reason, the most
/// fundamental first (volume, then history, then the policy share, then
/// compliance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadinessReason {
    /// Fewer than [`READINESS_MIN_MESSAGES`] messages in the window under the
    /// current policy.
    TooLittleMail,
    /// Reports under the current policy cover less than the window; they
    /// cover all of it at `window_full_at` (unix seconds) if they keep
    /// arriving.
    NotEnoughHistory { window_full_at: i64 },
    /// Less than [`READINESS_MIN_POLICY_SHARE_PCT`] of the window's messages
    /// were evaluated under the current policy: receivers still apply the old
    /// one to the rest.
    OldPolicyApplied,
    /// Delivered compliance over the window is below the pass threshold
    /// ([`RATE_PASS_MIN_PCT`]), or nothing was delivered to measure.
    BelowThreshold,
}

/// Why no policy step is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum NotApplicableReason {
    /// No published `p=` is known (and the domain is not parked).
    NoPolicy,
    /// A parked domain publishes `p=reject` at once, whatever its mail, and
    /// whether it has a DMARC record yet or not.
    Parked,
}

/// What a domain should do about its policy next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadinessState {
    /// The window's mail supports moving to `next`.
    Ready { next: PolicyStep },
    /// `p=quarantine` or `p=reject` applies to only `pct` percent of failing
    /// mail: raise it to 100 before taking the next step.
    RaisePct { pct: i64 },
    /// Not yet ready for `next`.
    NotReady {
        next: PolicyStep,
        reason: ReadinessReason,
    },
    /// `p=reject` at 100%: nothing left to tighten.
    Enforced,
    /// No published policy, or a parked domain (which should publish
    /// `p=reject` at once, whatever its mail and with or without a record;
    /// its own issue says so).
    NotApplicable { reason: NotApplicableReason },
}

/// The window the verdict was judged on: the figures it was computed from.
/// The totals cover every message of the window; the `policy_` figures only
/// those from reports evaluated under the current policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ReadinessWindow {
    /// Start of the window (unix seconds): [`READINESS_WINDOW_DAYS`] before
    /// the judgement. A list of the window's mail uses it as its `since`.
    pub since: i64,
    /// The current policy, the DNS record's `p=`, that the `policy_` figures
    /// are under; `unknown` without one.
    pub policy: EnforcementState,
    /// Messages in the window (every outcome, whatever policy was applied).
    pub messages: i64,
    /// Messages in the window from reports whose receiver applied `policy`.
    pub policy_messages: i64,
    /// Where the reports under `policy` begin: the oldest of them since the
    /// newest report under another policy that began before the window
    /// (stragglers inside the window do not move it). The clock starts
    /// there, never before `since`. `None` without such a report.
    pub policy_first_report_at: Option<i64>,
    /// `policy_messages` as a share of `messages`; `None` without messages.
    pub policy_share_pct: Option<f64>,
    /// The policy most of the window's other messages were evaluated under;
    /// `unknown` when there are none or their reports name no known policy.
    pub other_policy: EnforcementState,
    /// Passed DMARC.
    pub compliant: i64,
    /// Failed DMARC and still delivered, forwarded mail aside.
    pub delivered_failures: i64,
    /// Failed DMARC and not forwarded: delivered failures plus blocked by
    /// policy, what a stricter policy acts on.
    pub failed: i64,
    /// Failed DMARC and quarantined or rejected: what the published policy
    /// already stopped (the part of `failed` that was not delivered).
    pub blocked: i64,
    /// Failed DMARC, delivered because the receiver trusted the forwarder;
    /// in neither `failed` nor delivered compliance.
    pub forwarded_failures: i64,
    /// Delivered compliance over the window: forwarded and blocked mail do
    /// not count against it.
    pub delivered_compliance: Rate,
}

/// Each rule of the next step, judged on its own over the window. The
/// verdict's reason is the first unmet one (volume, history, policy share,
/// compliance).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ReadinessCriteria {
    /// At least [`READINESS_MIN_MESSAGES`] messages in the window under the
    /// current policy.
    pub enough_mail: bool,
    /// Reports under the current policy cover the whole window: they begin
    /// at or before its start.
    pub full_history: bool,
    /// At least [`READINESS_MIN_POLICY_SHARE_PCT`] of the window's messages
    /// were evaluated under the current policy.
    pub policy_share_met: bool,
    /// Delivered compliance over the window is measured and at least
    /// [`RATE_PASS_MIN_PCT`].
    pub compliance_met: bool,
}

/// A policy to publish: `p=` at `pct` percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PolicyTarget {
    pub p: PolicyStep,
    pub pct: i64,
}

/// Whether a domain is ready for its next policy step, with what it was
/// judged from.
///
/// Status: `pass` when ready or enforced; `warn` for "raise pct to 100
/// first" (worth fixing); `neutral` for not ready and not applicable (not
/// ready yet is not a fault: the compliance figure carries its own colour).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct PolicyReadiness {
    pub state: ReadinessState,
    pub status: Status,
    pub window: ReadinessWindow,
    /// The rules of the next step over `window`, each met or not.
    pub criteria: ReadinessCriteria,
    /// Whole days of the window under the current policy: from
    /// `window.policy_first_report_at` (never before `window.since`) to the
    /// judgement, so at most [`READINESS_WINDOW_DAYS`]. `None` without a
    /// report under it.
    pub history_days: Option<i64>,
    /// When reports under the current policy cover a whole window
    /// (`window.policy_first_report_at` plus [`READINESS_WINDOW_DAYS`]);
    /// `None` without a report under it.
    pub history_full_at: Option<i64>,
    /// The policies to publish from here, the next first, each at full
    /// `pct`; empty when enforced or without a policy. A parked domain goes
    /// straight to `p=reject`, from a new record when it has none.
    pub plan: Vec<PolicyTarget>,
}

impl PolicyReadiness {
    /// Whether the window's rules decide the state: ready and not ready,
    /// and "raise pct" (they decide the step after it). An enforced domain
    /// and one with nothing to judge are decided by their policy alone.
    pub fn rules_decide(&self) -> bool {
        matches!(
            self.state,
            ReadinessState::Ready { .. }
                | ReadinessState::NotReady { .. }
                | ReadinessState::RaisePct { .. }
        )
    }

    /// Whether the verdict recommends the plan's first step now: ready,
    /// raise pct, or a parked domain's `p=reject`. Not ready, no step is.
    pub fn recommends_first_step(&self) -> bool {
        matches!(
            self.state,
            ReadinessState::Ready { .. }
                | ReadinessState::RaisePct { .. }
                | ReadinessState::NotApplicable { .. }
        )
    }

    /// Whether the next step can be taken now: ready, or raise `pct` first.
    pub fn step_pending(&self) -> bool {
        matches!(
            self.state,
            ReadinessState::Ready { .. } | ReadinessState::RaisePct { .. }
        )
    }

    /// The policy the domain moves to next, ready or not; `None` while
    /// `pct` is to be raised first, when enforced and with nothing to judge.
    pub fn next_step(&self) -> Option<PolicyStep> {
        match self.state {
            ReadinessState::Ready { next } | ReadinessState::NotReady { next, .. } => Some(next),
            _ => None,
        }
    }
}

/// What readiness is judged from.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadinessFacts<'a> {
    /// Message totals of the last [`READINESS_WINDOW_DAYS`] days, whatever
    /// policy the receivers applied.
    pub window_mail: Mail,
    /// Published `p=` of the DMARC record in DNS (the store's `PolicySource`);
    /// `None` without a stored record, so `not_applicable` (a parked domain
    /// still gets its `p=reject`). Never a report's.
    pub policy: Option<&'a str>,
    /// Published `pct=`; absent means 100.
    pub pct: Option<i64>,
    pub parked: bool,
    /// Messages of the window from reports that applied `policy`.
    pub policy_messages: i64,
    /// Where the reports under `policy` begin (see
    /// [`ReadinessWindow::policy_first_report_at`]); `None` without one.
    pub policy_first_report_at: Option<i64>,
    /// The `p=` most of the window's other messages were evaluated under.
    pub other_policy: Option<&'a str>,
}

/// Start of the readiness window that ends at `now` (unix seconds).
pub fn readiness_window_start(now: i64) -> i64 {
    now - READINESS_WINDOW_DAYS * DAY_SECS
}

/// Readiness for the next policy step at `now`.
pub fn policy_readiness(facts: &ReadinessFacts<'_>, now: i64) -> PolicyReadiness {
    let mail = facts.window_mail;
    let since = readiness_window_start(now);
    let window = ReadinessWindow {
        since,
        policy: enforcement(facts.policy).state,
        messages: mail.total,
        policy_messages: facts.policy_messages,
        policy_first_report_at: facts.policy_first_report_at,
        policy_share_pct: (mail.total > 0)
            .then(|| facts.policy_messages as f64 * 100.0 / mail.total as f64),
        other_policy: enforcement(facts.other_policy).state,
        compliant: mail.compliant,
        delivered_failures: mail.delivered_failures,
        failed: mail.delivered_failures + mail.blocked(),
        blocked: mail.blocked(),
        forwarded_failures: mail.forwarded_failures,
        delivered_compliance: mail.delivered_compliance(),
    };
    let criteria = ReadinessCriteria {
        enough_mail: window.policy_messages >= READINESS_MIN_MESSAGES,
        full_history: window.policy_first_report_at.is_some_and(|f| f <= since),
        policy_share_met: window
            .policy_share_pct
            .is_some_and(|pct| pct >= READINESS_MIN_POLICY_SHARE_PCT),
        compliance_met: window
            .delivered_compliance
            .pct
            .is_some_and(|pct| pct >= RATE_PASS_MIN_PCT),
    };
    let history_full_at = window
        .policy_first_report_at
        .map(|f| f + READINESS_WINDOW_DAYS * DAY_SECS);
    let state = readiness_state(facts, criteria, history_full_at);
    let status = match state {
        ReadinessState::Ready { .. } | ReadinessState::Enforced => Status::Pass,
        ReadinessState::RaisePct { .. } => Status::Warn,
        ReadinessState::NotReady { .. } | ReadinessState::NotApplicable { .. } => Status::Neutral,
    };
    PolicyReadiness {
        state,
        status,
        window,
        criteria,
        history_days: window
            .policy_first_report_at
            .map(|f| (now - f.max(since)).max(0) / DAY_SECS),
        history_full_at,
        plan: plan(facts),
    }
}

/// The policies left to publish, the next first, at full `pct` each.
fn plan(facts: &ReadinessFacts<'_>) -> Vec<PolicyTarget> {
    let full = |p| PolicyTarget { p, pct: 100 };
    let partial = facts.pct.unwrap_or(100) < 100;
    match enforcement(facts.policy).state {
        EnforcementState::Reject if !partial => vec![],
        _ if facts.parked => vec![full(PolicyStep::Reject)],
        EnforcementState::Unknown => vec![],
        EnforcementState::Monitor => vec![full(PolicyStep::Quarantine), full(PolicyStep::Reject)],
        EnforcementState::Quarantine if partial => {
            vec![full(PolicyStep::Quarantine), full(PolicyStep::Reject)]
        }
        EnforcementState::Quarantine | EnforcementState::Reject => vec![full(PolicyStep::Reject)],
    }
}

fn readiness_state(
    facts: &ReadinessFacts<'_>,
    criteria: ReadinessCriteria,
    history_full_at: Option<i64>,
) -> ReadinessState {
    let pct = facts.pct.unwrap_or(100);
    let next = match enforcement(facts.policy).state {
        EnforcementState::Reject if pct >= 100 => return ReadinessState::Enforced,
        _ if facts.parked => {
            return ReadinessState::NotApplicable {
                reason: NotApplicableReason::Parked,
            }
        }
        EnforcementState::Unknown => {
            return ReadinessState::NotApplicable {
                reason: NotApplicableReason::NoPolicy,
            }
        }
        // `pct` does nothing under `p=none`; under an enforcing policy it
        // comes before the next step.
        EnforcementState::Quarantine | EnforcementState::Reject => {
            if pct < 100 {
                return ReadinessState::RaisePct { pct };
            }
            PolicyStep::Reject
        }
        EnforcementState::Monitor => PolicyStep::Quarantine,
    };
    let reason = if !criteria.enough_mail {
        Some(ReadinessReason::TooLittleMail)
    } else if let (false, Some(window_full_at)) = (criteria.full_history, history_full_at) {
        Some(ReadinessReason::NotEnoughHistory { window_full_at })
    } else if !criteria.policy_share_met {
        Some(ReadinessReason::OldPolicyApplied)
    } else if !criteria.compliance_met {
        Some(ReadinessReason::BelowThreshold)
    } else {
        None
    };
    match reason {
        Some(reason) => ReadinessState::NotReady { next, reason },
        None => ReadinessState::Ready { next },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;
    const OLD: i64 = NOW - 60 * DAY_SECS;

    fn mail(total: i64, compliant: i64, delivered: i64, forwarded: i64) -> Mail {
        Mail {
            total,
            compliant,
            delivered_failures: delivered,
            forwarded_failures: forwarded,
        }
    }

    /// All of the window's mail evaluated under `policy`, reports under it
    /// for 60 days.
    fn facts(policy: &str, window_mail: Mail) -> ReadinessFacts<'_> {
        ReadinessFacts {
            window_mail,
            policy: Some(policy),
            pct: None,
            parked: false,
            policy_messages: window_mail.total,
            policy_first_report_at: Some(OLD),
            other_policy: None,
        }
    }

    fn state(f: &ReadinessFacts<'_>) -> ReadinessState {
        policy_readiness(f, NOW).state
    }

    /// Which states the rules decide, recommend the first step of the plan
    /// and have a step to take now.
    #[test]
    fn each_state_says_what_follows_from_it() {
        let judged = |f: &ReadinessFacts<'_>| {
            let r = policy_readiness(f, NOW);
            (
                r.rules_decide(),
                r.recommends_first_step(),
                r.step_pending(),
            )
        };
        let good = mail(1000, 995, 5, 0);
        assert_eq!(judged(&facts("none", good)), (true, true, true), "ready");
        assert_eq!(
            judged(&facts("none", mail(10, 10, 0, 0))),
            (true, false, false),
            "not ready"
        );
        let partial = ReadinessFacts {
            pct: Some(50),
            ..facts("quarantine", good)
        };
        assert_eq!(judged(&partial), (true, true, true), "raise pct");
        assert_eq!(
            judged(&facts("reject", good)),
            (false, false, false),
            "enforced"
        );
        let parked = ReadinessFacts {
            parked: true,
            ..facts("none", good)
        };
        assert_eq!(judged(&parked), (false, true, false), "parked");
        let none = ReadinessFacts {
            policy: None,
            ..facts("none", good)
        };
        assert_eq!(judged(&none), (false, true, false), "no policy");
    }

    #[test]
    fn ready_steps_up_one_policy_at_a_time() {
        let r = policy_readiness(&facts("none", mail(1000, 995, 5, 0)), NOW);
        assert_eq!(
            r.state,
            ReadinessState::Ready {
                next: PolicyStep::Quarantine
            }
        );
        assert_eq!(r.status, Status::Pass);
        assert_eq!(r.window.messages, 1000);
        assert_eq!(r.window.delivered_compliance.status, Status::Pass);
        assert_eq!(
            state(&facts("quarantine", mail(1000, 990, 0, 0))),
            ReadinessState::Ready {
                next: PolicyStep::Reject
            }
        );
    }

    #[test]
    fn reject_at_full_pct_is_enforced_whatever_the_mail() {
        let r = policy_readiness(&facts("reject", mail(0, 0, 0, 0)), NOW);
        assert_eq!(r.state, ReadinessState::Enforced);
        assert_eq!(r.status, Status::Pass);
    }

    #[test]
    fn a_partial_pct_comes_first() {
        for policy in ["quarantine", "reject"] {
            let f = ReadinessFacts {
                pct: Some(50),
                ..facts(policy, mail(10, 1, 9, 0))
            };
            let r = policy_readiness(&f, NOW);
            assert_eq!(r.state, ReadinessState::RaisePct { pct: 50 });
            assert_eq!(r.status, Status::Warn);
        }
        // pct does nothing under p=none.
        let f = ReadinessFacts {
            pct: Some(50),
            ..facts("none", mail(1000, 1000, 0, 0))
        };
        assert_eq!(
            state(&f),
            ReadinessState::Ready {
                next: PolicyStep::Quarantine
            }
        );
    }

    #[test]
    fn no_policy_or_parked_is_not_applicable() {
        let f = ReadinessFacts {
            policy: None,
            ..facts("none", mail(1000, 1000, 0, 0))
        };
        let r = policy_readiness(&f, NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotApplicable {
                reason: NotApplicableReason::NoPolicy
            }
        );
        assert_eq!(r.status, Status::Neutral);
        let parked = ReadinessFacts {
            parked: true,
            ..facts("none", mail(0, 0, 0, 0))
        };
        assert_eq!(
            state(&parked),
            ReadinessState::NotApplicable {
                reason: NotApplicableReason::Parked
            }
        );
        let parked_reject = ReadinessFacts {
            parked: true,
            ..facts("reject", mail(0, 0, 0, 0))
        };
        assert_eq!(state(&parked_reject), ReadinessState::Enforced);
        // Without a record a parked domain still publishes p=reject at once.
        let parked_without_record = ReadinessFacts {
            policy: None,
            parked: true,
            ..facts("none", mail(0, 0, 0, 0))
        };
        let r = policy_readiness(&parked_without_record, NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotApplicable {
                reason: NotApplicableReason::Parked
            }
        );
        assert!(r.recommends_first_step());
    }

    #[test]
    fn too_little_mail_comes_before_history_and_compliance() {
        let f = ReadinessFacts {
            policy_first_report_at: Some(NOW - DAY_SECS),
            ..facts("none", mail(READINESS_MIN_MESSAGES - 1, 100, 99, 0))
        };
        let r = policy_readiness(&f, NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotReady {
                next: PolicyStep::Quarantine,
                reason: ReadinessReason::TooLittleMail
            }
        );
        assert_eq!(r.status, Status::Neutral);
    }

    #[test]
    fn history_must_cover_the_window() {
        let first = NOW - 10 * DAY_SECS;
        let f = ReadinessFacts {
            policy_first_report_at: Some(first),
            ..facts("none", mail(500, 400, 100, 0))
        };
        assert_eq!(
            state(&f),
            ReadinessState::NotReady {
                next: PolicyStep::Quarantine,
                reason: ReadinessReason::NotEnoughHistory {
                    window_full_at: first + 30 * DAY_SECS
                }
            }
        );
        // Exactly the window's age is enough.
        let full = ReadinessFacts {
            policy_first_report_at: Some(NOW - READINESS_WINDOW_DAYS * DAY_SECS),
            ..facts("none", mail(500, 500, 0, 0))
        };
        assert!(matches!(state(&full), ReadinessState::Ready { .. }));
    }

    #[test]
    fn compliance_below_the_pass_rate_is_not_ready() {
        let below = |m| {
            ReadinessState::NotReady {
                next: PolicyStep::Reject,
                reason: ReadinessReason::BelowThreshold,
            } == state(&facts("quarantine", m))
        };
        assert!(below(mail(1000, 971, 29, 0)));
        // Nothing delivered at all: nothing shows the policy is safe.
        assert!(below(mail(1000, 0, 0, 0)));
        // Exactly the pass rate is ready.
        assert!(!below(mail(1000, 990, 10, 0)));
    }

    #[test]
    fn forwarded_and_blocked_failures_do_not_count_against_compliance() {
        let f = facts("none", mail(1000, 700, 0, 200));
        assert_eq!(
            state(&f),
            ReadinessState::Ready {
                next: PolicyStep::Quarantine
            }
        );
    }

    #[test]
    fn serializes_as_a_tagged_state() {
        let r = policy_readiness(
            &ReadinessFacts {
                policy_first_report_at: Some(NOW),
                ..facts("none", mail(300, 300, 0, 0))
            },
            NOW,
        );
        let json = serde_json::to_value(r).unwrap();
        assert_eq!(json["state"]["kind"], "not_ready");
        assert_eq!(json["state"]["next"], "quarantine");
        assert_eq!(json["state"]["reason"]["kind"], "not_enough_history");
        assert_eq!(json["status"], "neutral");
        assert_eq!(json["window"]["messages"], 300);
        assert_eq!(json["plan"][0]["p"], "quarantine");
    }

    #[test]
    fn the_evidence_is_what_the_verdict_was_judged_from() {
        // 1000 messages: 950 passed, 20 delivered failures, 10 forwarded, 20 blocked.
        let f = facts("none", mail(1000, 950, 20, 10));
        let r = policy_readiness(&f, NOW);
        assert_eq!(r.window.since, NOW - READINESS_WINDOW_DAYS * DAY_SECS);
        assert_eq!(r.window.compliant, 950);
        assert_eq!(r.window.delivered_failures, 20);
        assert_eq!(r.window.failed, 40);
        assert_eq!(r.window.blocked, 20);
        assert_eq!(r.window.forwarded_failures, 10);
        assert_eq!(
            r.criteria,
            ReadinessCriteria {
                enough_mail: true,
                full_history: true,
                policy_share_met: true,
                compliance_met: false,
            }
        );
        assert_eq!(r.window.policy, EnforcementState::Monitor);
        assert_eq!(r.window.policy_messages, 1000);
        assert_eq!(r.window.policy_first_report_at, Some(OLD));
        assert_eq!(r.window.policy_share_pct, Some(100.0));
        assert_eq!(r.window.other_policy, EnforcementState::Unknown);
        // 60 days under the policy count as the window's 30.
        assert_eq!(r.history_days, Some(READINESS_WINDOW_DAYS));
        assert_eq!(
            r.history_full_at,
            Some(OLD + READINESS_WINDOW_DAYS * DAY_SECS)
        );
        // Every rule is judged, not only the first unmet one.
        let young = ReadinessFacts {
            policy_first_report_at: Some(NOW - DAY_SECS),
            ..facts("none", mail(10, 1, 9, 0))
        };
        let young = policy_readiness(&young, NOW);
        assert_eq!(young.history_days, Some(1));
        let c = young.criteria;
        assert!(!c.enough_mail && !c.full_history && !c.compliance_met);
        assert!(c.policy_share_met);
    }

    /// Published `p=quarantine` `days` ago: `under` of the window's 1000
    /// messages were evaluated under it, the rest still under `p=none`.
    fn switched(days: i64, under: i64) -> ReadinessFacts<'static> {
        ReadinessFacts {
            policy_messages: under,
            policy_first_report_at: Some(NOW - days * DAY_SECS),
            other_policy: (under < 1000).then_some("none"),
            ..facts("quarantine", mail(1000, 1000, 0, 0))
        }
    }

    #[test]
    fn only_mail_under_the_current_policy_counts_toward_volume_and_history() {
        // The morning after publishing p=quarantine: 30 days of p=none mail
        // that said "ready for quarantine" say nothing about reject.
        let r = policy_readiness(&switched(0, 0), NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotReady {
                next: PolicyStep::Reject,
                reason: ReadinessReason::TooLittleMail
            }
        );
        assert_eq!(r.history_days, Some(0));
        assert_eq!(r.window.policy, EnforcementState::Quarantine);
        assert_eq!(r.window.other_policy, EnforcementState::Monitor);
        // Enough mail under it, but only 10 days of it.
        let r = policy_readiness(&switched(10, 300), NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotReady {
                next: PolicyStep::Reject,
                reason: ReadinessReason::NotEnoughHistory {
                    window_full_at: NOW + 20 * DAY_SECS
                }
            }
        );
        assert_eq!(r.history_days, Some(10));
        assert_eq!(r.window.policy_share_pct, Some(30.0));
        // A full window under it: ready.
        assert_eq!(
            state(&switched(31, 1000)),
            ReadinessState::Ready {
                next: PolicyStep::Reject
            }
        );
    }

    #[test]
    fn stragglers_under_the_old_policy_are_tolerated_by_share() {
        // 10% still evaluated under p=none: not ready, and that is why.
        let r = policy_readiness(&switched(40, 900), NOW);
        assert_eq!(
            r.state,
            ReadinessState::NotReady {
                next: PolicyStep::Reject,
                reason: ReadinessReason::OldPolicyApplied
            }
        );
        assert!(!r.criteria.policy_share_met);
        assert!(r.criteria.enough_mail && r.criteria.full_history && r.criteria.compliance_met);
        // 3% does not block; exactly the minimum share does not either.
        assert!(matches!(
            state(&switched(40, 970)),
            ReadinessState::Ready { .. }
        ));
        assert!(matches!(
            state(&switched(40, 950)),
            ReadinessState::Ready { .. }
        ));
        // The share comes after history and before compliance.
        let f = ReadinessFacts {
            window_mail: mail(1000, 500, 500, 0),
            ..switched(40, 900)
        };
        assert_eq!(
            state(&f),
            ReadinessState::NotReady {
                next: PolicyStep::Reject,
                reason: ReadinessReason::OldPolicyApplied
            }
        );
    }

    #[test]
    fn the_plan_lists_the_steps_left_at_full_pct() {
        let plan = |policy, pct, parked| {
            let f = ReadinessFacts {
                pct,
                parked,
                ..facts(policy, mail(0, 0, 0, 0))
            };
            policy_readiness(&f, NOW)
                .plan
                .iter()
                .map(|t| (t.p, t.pct))
                .collect::<Vec<_>>()
        };
        use PolicyStep::{Quarantine, Reject};
        assert_eq!(
            plan("none", None, false),
            [(Quarantine, 100), (Reject, 100)]
        );
        assert_eq!(
            plan("quarantine", Some(20), false),
            [(Quarantine, 100), (Reject, 100)]
        );
        assert_eq!(plan("quarantine", None, false), [(Reject, 100)]);
        assert_eq!(plan("reject", Some(50), false), [(Reject, 100)]);
        assert_eq!(plan("reject", None, false), []);
        assert_eq!(plan("none", None, true), [(Reject, 100)]);
        let none = ReadinessFacts {
            policy: None,
            ..facts("none", mail(0, 0, 0, 0))
        };
        assert!(policy_readiness(&none, NOW).plan.is_empty());
        // A parked domain without a record: a new one at p=reject.
        let parked_none = ReadinessFacts {
            parked: true,
            ..none
        };
        let plan = policy_readiness(&parked_none, NOW).plan;
        assert_eq!(
            plan.iter().map(|t| (t.p, t.pct)).collect::<Vec<_>>(),
            [(Reject, 100)]
        );
    }
}
