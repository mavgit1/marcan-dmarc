//! The state alerts compare: what one evaluation reduces a domain to
//! ([`AlertState`], [`alert_state`]), its figures, and the facts of its DNS
//! records and their changes.

use crate::dns::{CheckKind, Severity};
use crate::domain::{DomainAssessment, IssueKind};
use crate::mail::{
    enforcement, EnforcementState, HealthState, RATE_PASS_MIN_PCT, RATE_WARN_MIN_PCT,
};
use crate::readiness::{PolicyReadiness, PolicyStep, ReadinessState, READINESS_WINDOW_DAYS};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// The version of the stored JSON of [`AlertState`] and of an event's
/// [`AlertKind`](super::AlertKind) (`alert_state.format`, `alert_events.format`). A change to
/// their serde shape bumps it, with a migration that converts the stored
/// rows; a row of another format is not read.
pub const ALERT_FORMAT: i64 = 1;

/// The judged state of one domain that alerts compare, stored per domain
/// between evaluations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertState {
    /// At least one report was received: the report-derived parts (health,
    /// issues, readiness) were measured.
    pub has_reports: bool,
    pub health: HealthState,
    /// The open issues with their colour, worst first.
    pub issues: Vec<OpenIssue>,
    /// The policy step the domain is ready for, when it is ready for one.
    pub ready_for: Option<PolicyStep>,
    /// The stored DNS result; `None` until one exists.
    pub dns: Option<DnsState>,
    /// Whether the domain is marked parked.
    pub parked: bool,
    /// The figures the events' sentences name (always set by
    /// [`alert_state`]).
    pub figures: Option<AlertFigures>,
    /// The senders new to the domain in the window (no records from them
    /// before it) with at least [`NEW_SENDER_MIN_FAILURES`] failing
    /// messages in it, by source IP. `None` in a state stored before Marcan
    /// measured them: the next evaluation is their baseline.
    #[serde(default)]
    pub new_failing_senders: Option<Vec<NewSender>>,
}

/// A sender new to a domain alerts once it has this many failing messages
/// (failed DMARC, not forwarded) in the window.
pub const NEW_SENDER_MIN_FAILURES: i64 = 10;

/// The senders of `new_senders` with at least [`NEW_SENDER_MIN_FAILURES`]
/// failing messages, by source IP.
pub(super) fn failing_new_senders(new_senders: Vec<NewSender>) -> Vec<NewSender> {
    let mut failing: Vec<NewSender> = new_senders
        .into_iter()
        .filter(|s| s.failed >= NEW_SENDER_MIN_FAILURES)
        .collect();
    failing.sort_by(|a, b| a.source_ip.cmp(&b.source_ip));
    failing
}

/// A sender (a source IP, as the sender pages key it) that sent nothing as
/// the domain before the window, and its failing mail in the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewSender {
    pub source_ip: String,
    /// Its name when Marcan knows one (the operator's, else its domain), as
    /// the sender pages caption it.
    pub name: Option<String>,
    /// Failed DMARC and not forwarded: delivered failures plus blocked.
    pub failed: i64,
    /// Of those, delivered.
    pub delivered_failures: i64,
}

/// The figures of one evaluation an event's sentence names: the mail over
/// the readiness window, how long reports have been missing, and the
/// evidence under the current policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct AlertFigures {
    /// The days the mail figures cover.
    pub window_days: i64,
    /// Passed DMARC plus delivered failures: what delivered compliance is over.
    pub delivered: i64,
    pub delivered_failures: i64,
    /// Failed DMARC and quarantined or rejected.
    pub blocked: i64,
    /// Whole days without a report, once that counts as stopped.
    pub reports_stopped_days: Option<i64>,
    /// The days after which reports count as stopped (the setting judged by).
    pub reports_stopped_after_days: i64,
    /// The current policy the evidence is under.
    pub policy: EnforcementState,
    /// Whole days of the window under it; `None` without a report under it.
    pub policy_days: Option<i64>,
    /// The window's messages evaluated under it.
    pub policy_messages: i64,
    /// Delivered compliance over the window; `None` with nothing delivered.
    pub delivered_compliance_pct: Option<f64>,
}

impl AlertFigures {
    /// The mail figures, measured against the failure share above which
    /// delivered mail reads `status` (more than 1% failing warns, more than
    /// 5% fails: [`RATE_PASS_MIN_PCT`] and [`RATE_WARN_MIN_PCT`]).
    pub(super) fn delivered(&self, status: Status) -> DeliveredFigures {
        let threshold = if status == Status::Fail {
            RATE_WARN_MIN_PCT
        } else {
            RATE_PASS_MIN_PCT
        };
        DeliveredFigures {
            window_days: self.window_days,
            delivered: self.delivered,
            delivered_failures: self.delivered_failures,
            blocked: self.blocked,
            failure_pct: (self.delivered > 0)
                .then(|| self.delivered_failures as f64 * 100.0 / self.delivered as f64),
            threshold_pct: 100.0 - threshold,
        }
    }

    pub(super) fn stopped(&self) -> Option<StoppedFigures> {
        Some(StoppedFigures {
            days: self.reports_stopped_days?,
            threshold_days: self.reports_stopped_after_days,
        })
    }

    pub(super) fn evidence(&self) -> Option<ReadinessEvidence> {
        Some(ReadinessEvidence {
            policy: self.policy,
            days: self.policy_days?,
            messages: self.policy_messages,
            delivered_compliance_pct: self.delivered_compliance_pct,
        })
    }
}

/// Delivered mail over a window, as an alert names it: "12 delivered
/// failures in the last 30 days (1.4% of delivered mail, threshold 1%)".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeliveredFigures {
    pub window_days: i64,
    /// Passed DMARC plus delivered failures.
    pub delivered: i64,
    pub delivered_failures: i64,
    /// Failed DMARC and quarantined or rejected.
    pub blocked: i64,
    /// `delivered_failures` as a share of `delivered`; `null` with nothing delivered.
    pub failure_pct: Option<f64>,
    /// The failure share the colour of the event starts above: 1 (warning)
    /// or 5 (failing).
    pub threshold_pct: f64,
}

/// How long reports were missing: "No reports for 9 days (threshold 7 days)".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoppedFigures {
    /// Whole days without a report.
    pub days: i64,
    /// The setting they count as stopped after.
    pub threshold_days: i64,
}

/// What a readiness step was reached on: "30 days and 1,204 messages under
/// p=none, 99.6% delivered compliance".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ReadinessEvidence {
    /// The current policy.
    pub policy: EnforcementState,
    /// Whole days of the window under it.
    pub days: i64,
    /// The window's messages evaluated under it.
    pub messages: i64,
    /// Over the window; `null` with nothing delivered.
    pub delivered_compliance_pct: Option<f64>,
}

impl AlertState {
    /// The state with its report-derived parts marked unmeasured, so the next
    /// comparison takes them as a new baseline (the DNS parts still compare).
    /// For a domain whose reports arrived during a mailbox import: reports
    /// land in any order then, so the window's figures in between are no
    /// state the domain was ever in.
    pub fn reports_rebaselined(&self) -> Self {
        Self {
            has_reports: false,
            ..self.clone()
        }
    }
}

/// One open issue and its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenIssue {
    pub kind: IssueKind,
    pub status: Status,
}

/// What alerts keep of a stored DNS result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsState {
    /// The checker that produced it: a result of another version was judged
    /// by other rules, so it is a new baseline, not a change.
    pub checker_version: u32,
    /// The worst finding of each check that has findings.
    pub worst: BTreeMap<CheckKind, Severity>,
    /// The records as published, for `dmarc-dns`'s diff.
    pub records: DnsRecords,
}

/// One check's record: published (with its facts), measured absent, or not
/// measured (the lookup failed, so nothing is known and nothing changed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "record", rename_all = "snake_case")]
pub enum Observed<T> {
    Unmeasured,
    Absent,
    Published(T),
}

/// The published records of a domain, reduced to their parsed values so a
/// reformatted record is the same record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsRecords {
    pub dmarc: Observed<DmarcRecordFacts>,
    pub spf: Observed<SpfRecordFacts>,
    /// Every probed selector.
    pub dkim: Vec<DkimSelectorFacts>,
    pub mta_sts: Observed<MtaStsRecordFacts>,
    pub tls_rpt: Observed<TlsRptRecordFacts>,
    pub bimi: Observed<BimiRecordFacts>,
}

/// A DMARC record's tags, with the RFC defaults applied (`pct` 100, relaxed
/// alignment, `t=n`) so writing a default out is no change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DmarcRecordFacts {
    pub p: Option<String>,
    /// `None`: subdomains follow `p`.
    pub sp: Option<String>,
    pub pct: u32,
    /// Report addresses, sorted.
    pub rua: Vec<String>,
    pub adkim: String,
    pub aspf: String,
    /// `t=y` (RFC 9989): published for testing, which RFC 9989 receivers do
    /// not apply. Defaulted: facts stored before it read as not testing.
    #[serde(default)]
    pub testing: bool,
}

impl DmarcRecordFacts {
    /// Whether `other` sets another policy: `p`, `sp`, `pct` or `t` differ.
    pub(super) fn policy_differs(&self, other: &Self) -> bool {
        (&self.p, &self.sp, self.pct, self.testing)
            != (&other.p, &other.sp, other.pct, other.testing)
    }

    /// How strictly failing mail is treated: `none` < `quarantine` <
    /// `reject`, each below 100% (or in testing mode) below its full step.
    /// Unknown is weakest.
    fn strength(p: Option<&str>, pct: u32, testing: bool) -> i32 {
        let partial = i32::from(pct < 100 || testing);
        match enforcement(p).state {
            EnforcementState::Unknown => -1,
            EnforcementState::Monitor => 0,
            EnforcementState::Quarantine => 2 - partial,
            EnforcementState::Reject => 4 - partial,
        }
    }

    /// `p=` strength first, then the subdomains' (`sp=`, else `p=`).
    pub(super) fn rank(&self) -> (i32, i32) {
        let sp = self.sp.as_deref().or(self.p.as_deref());
        (
            Self::strength(self.p.as_deref(), self.pct, self.testing),
            Self::strength(sp, self.pct, self.testing),
        )
    }
}

/// An SPF record's terms (normalized, sorted) and its DNS lookup count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpfRecordFacts {
    pub terms: Vec<String>,
    pub lookups: u32,
}

/// One probed DKIM selector and its key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DkimSelectorFacts {
    pub selector: String,
    pub key: Observed<DkimKeyFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DkimKeyFacts {
    /// A fingerprint of the public key (`p=`); empty when revoked.
    pub key_id: String,
    pub revoked: bool,
}

/// The MTA-STS TXT record's `id=` and, when the policy file was read, its
/// mode and MX patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MtaStsRecordFacts {
    pub id: Option<String>,
    pub policy: Option<MtaStsPolicyFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MtaStsPolicyFacts {
    pub mode: Option<String>,
    /// Sorted.
    pub mx: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsRptRecordFacts {
    /// Sorted.
    pub rua: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BimiRecordFacts {
    pub logo_url: Option<String>,
    pub authority_url: Option<String>,
    pub declined: bool,
}

/// One change between two DNS results of a domain, as a fact: which check,
/// and what changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordChange {
    pub check: CheckKind,
    pub change: RecordDelta,
}

/// What changed in one check's record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecordDelta {
    /// A record is published where there was none.
    Published,
    /// The record is gone.
    Removed,
    /// Terms added and removed (normalized), and the DNS lookups the whole
    /// record costs before and after (includes count, so a provider changing
    /// its own include shows here too).
    SpfChanged {
        added: Vec<String>,
        removed: Vec<String>,
        lookups_before: u32,
        lookups_after: u32,
    },
    DmarcChanged {
        before: DmarcRecordFacts,
        after: DmarcRecordFacts,
    },
    /// A key is published under a selector that had none.
    DkimKeyAppeared { selector: String },
    /// The selector's key is gone.
    DkimKeyRemoved { selector: String },
    /// The selector now publishes another key (or a key again after a revocation).
    DkimKeyChanged { selector: String },
    /// The selector's key was revoked (`p=` emptied).
    DkimKeyRevoked { selector: String },
    MtaStsChanged {
        before: MtaStsRecordFacts,
        after: MtaStsRecordFacts,
    },
    TlsRptChanged {
        rua_before: Vec<String>,
        rua_after: Vec<String>,
    },
    BimiChanged {
        before: BimiRecordFacts,
        after: BimiRecordFacts,
    },
}

/// A domain's [`AlertState`] from what the domain overview shows: its
/// health, assessment and policy readiness (all over the readiness window)
/// and the stored DNS result, whose DMARC record holds the policy alerts
/// compare. `new_senders` are the senders new to the domain in the window;
/// those with at least [`NEW_SENDER_MIN_FAILURES`] failing messages are kept.
pub fn alert_state(
    health: HealthState,
    assessment: &DomainAssessment,
    readiness: &PolicyReadiness,
    has_reports: bool,
    parked: bool,
    dns: Option<DnsState>,
    new_senders: Vec<NewSender>,
) -> AlertState {
    AlertState {
        new_failing_senders: Some(failing_new_senders(new_senders)),
        has_reports,
        health,
        issues: assessment
            .issues
            .iter()
            .map(|i| OpenIssue {
                kind: i.kind,
                status: i.status,
            })
            .collect(),
        ready_for: match readiness.state {
            ReadinessState::Ready { next } => Some(next),
            _ => None,
        },
        dns,
        parked,
        figures: Some(AlertFigures {
            window_days: READINESS_WINDOW_DAYS,
            delivered: readiness.window.compliant + readiness.window.delivered_failures,
            delivered_failures: readiness.window.delivered_failures,
            blocked: readiness.window.blocked,
            reports_stopped_days: assessment.reports_stopped_days,
            reports_stopped_after_days: assessment.reports_stopped_after_days,
            policy: readiness.window.policy,
            policy_days: readiness.history_days,
            policy_messages: readiness.window.policy_messages,
            delivered_compliance_pct: readiness.window.delivered_compliance.pct,
        }),
    }
}
