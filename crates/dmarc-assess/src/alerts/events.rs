//! Events: what changed between two [`AlertState`]s ([`alert_events`]),
//! as [`AlertEvent`]s of an [`AlertKind`].

use super::state::{
    AlertFigures, AlertState, DeliveredFigures, DmarcRecordFacts, DnsState, NewSender, OpenIssue,
    ReadinessEvidence, RecordChange, RecordDelta, StoppedFigures,
};
use crate::dns::{CheckKind, Severity};
use crate::domain::IssueKind;
use crate::mail::{enforcement, EnforcementState, HealthState};
use crate::readiness::{PolicyStep, READINESS_WINDOW_DAYS};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use ts_rs::TS;

/// What changed. Each kind carries the facts its sentence needs, the
/// numbers it is based on included; the UI and the alert email word them,
/// one table per side.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AlertKind {
    /// A domain issue (as the attention list shows it) opened. Delivered
    /// failures carry the window's mail (`mail`), stopped reports how long
    /// (`stopped`).
    IssueOpened {
        issue: AlertedIssue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        mail: Option<DeliveredFigures>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        stopped: Option<StoppedFigures>,
    },
    /// An open issue cleared. Delivered failures carry the window's mail
    /// now, stopped reports how long they had been missing.
    IssueResolved {
        issue: AlertedIssue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        mail: Option<DeliveredFigures>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        stopped: Option<StoppedFigures>,
    },
    /// Health (is delivered mail authentic?) moved between two measured
    /// states, with the window's mail that moved it.
    HealthChanged {
        from: HealthState,
        to: HealthState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        mail: Option<DeliveredFigures>,
    },
    /// A published DNS record changed (for the DMARC record: anything but
    /// its policy, which is [`AlertKind::PolicyChanged`]).
    DnsRecordChanged {
        check: CheckKind,
        change: RecordDelta,
    },
    /// The DMARC record in DNS changed its policy (`p`, `sp`, `pct` or `t`): the
    /// record before and after, with any other tag changed at the same time.
    PolicyChanged {
        from: DmarcRecordFacts,
        to: DmarcRecordFacts,
    },
    /// The domain became ready for its next policy step, with the evidence.
    ReadinessReached {
        next: PolicyStep,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        evidence: Option<ReadinessEvidence>,
    },
    /// The domain was marked parked (`parked`) or no longer parked. Its
    /// issues, health and readiness start over from this evaluation instead
    /// of raising events of their own.
    ParkedChanged { parked: bool },
    /// A sender with no mail as the domain before the window had at least
    /// [`NEW_SENDER_MIN_FAILURES`](super::NEW_SENDER_MIN_FAILURES) failing
    /// messages in the window (`window_days`). Once per sender: it stays
    /// known while it keeps failing.
    NewFailingSender { sender: NewSender, window_days: i64 },
}

impl AlertKind {
    /// Every kind's name, as stored and as the UI's wording table keys it.
    pub const NAMES: [&'static str; 8] = [
        "issue_opened",
        "issue_resolved",
        "health_changed",
        "dns_record_changed",
        "policy_changed",
        "readiness_reached",
        "parked_changed",
        "new_failing_sender",
    ];

    /// The kind's name (the serialized tag).
    pub fn name(&self) -> &'static str {
        match self {
            Self::IssueOpened { .. } => "issue_opened",
            Self::IssueResolved { .. } => "issue_resolved",
            Self::HealthChanged { .. } => "health_changed",
            Self::DnsRecordChanged { .. } => "dns_record_changed",
            Self::PolicyChanged { .. } => "policy_changed",
            Self::ReadinessReached { .. } => "readiness_reached",
            Self::ParkedChanged { .. } => "parked_changed",
            Self::NewFailingSender { .. } => "new_failing_sender",
        }
    }
}

/// One change worth telling, with its colour: the colour of the state it
/// led to (green for a cleared issue, a tightened policy or a step reached;
/// the issue's own colour when one opens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AlertEvent {
    #[serde(flatten)]
    #[ts(flatten)]
    pub kind: AlertKind,
    pub severity: Status,
}

impl AlertEvent {
    pub(super) fn new(kind: AlertKind, severity: Status) -> Self {
        Self { kind, severity }
    }
}

/// The issues whose opening and clearing are events: delivered failures,
/// and reports stopping. The others are a standing state the
/// dashboard shows, which opens and clears with a policy, DNS or parked
/// change that is an event of its own. Serialized as the [`IssueKind`] it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AlertedIssue {
    DeliveredFailures,
    ReportsStopped,
}

impl AlertedIssue {
    pub const ALL: [Self; 2] = [Self::DeliveredFailures, Self::ReportsStopped];

    /// The issue it is.
    pub fn kind(self) -> IssueKind {
        match self {
            Self::DeliveredFailures => IssueKind::DeliveredFailures,
            Self::ReportsStopped => IssueKind::ReportsStopped,
        }
    }

    /// `kind`, when its opening and clearing are events.
    pub fn of(kind: IssueKind) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.kind() == kind)
    }
}

/// What changed from `previous` to `current`; nothing without a previous
/// state (the first evaluation is the baseline). `dns_changes` is
/// `dmarc-dns`'s diff of the two stored DNS results; it is only used when
/// both states hold a DNS result of the same checker.
pub fn alert_events(
    previous: Option<&AlertState>,
    current: &AlertState,
    dns_changes: &[RecordChange],
) -> Vec<AlertEvent> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut events = Vec::new();
    let figures = current.figures;
    // The issue's figures: the mail measured against the issue's colour, or
    // how long reports were missing (for a cleared issue, the last
    // evaluation's count, when it kept one).
    let mail = |status: Status| figures.map(|f| f.delivered(status));
    let issue_figures =
        |issue: AlertedIssue, status: Status, stopped_from: Option<AlertFigures>| match issue {
            AlertedIssue::DeliveredFailures => (mail(status), None),
            AlertedIssue::ReportsStopped => (None, stopped_from.and_then(|f| f.stopped())),
        };

    // Parked or no longer parked: one event, and what follows from the flag
    // (issues, health, readiness) is a new baseline.
    let parked_changed = previous.parked != current.parked;
    if parked_changed {
        events.push(AlertEvent::new(
            AlertKind::ParkedChanged {
                parked: current.parked,
            },
            Status::Neutral,
        ));
    }

    // DNS: comparable only between two results judged by the same rules.
    let dns_pair = match (&previous.dns, &current.dns) {
        (Some(a), Some(b)) if a.checker_version == b.checker_version => Some((a, b)),
        _ => None,
    };
    // Report-derived parts: only between two measured states. The first
    // reports of a domain (and an empty domain after retention) are baselines.
    let reports_comparable = previous.has_reports && current.has_reports && !parked_changed;

    if reports_comparable {
        // The alerted issues, both report-derived.
        let alerted = |i: &OpenIssue| AlertedIssue::of(i.kind).map(|a| (a, i.status));
        let before: BTreeSet<IssueKind> = previous.issues.iter().map(|i| i.kind).collect();
        for (issue, status) in current.issues.iter().filter_map(alerted) {
            if !before.contains(&issue.kind()) {
                let (mail, stopped) = issue_figures(issue, status, figures);
                events.push(AlertEvent::new(
                    AlertKind::IssueOpened {
                        issue,
                        mail,
                        stopped,
                    },
                    status,
                ));
            }
        }
        let after: BTreeSet<IssueKind> = current.issues.iter().map(|i| i.kind).collect();
        for (issue, _) in previous.issues.iter().filter_map(alerted) {
            if !after.contains(&issue.kind()) {
                let (mail, stopped) = issue_figures(issue, Status::Pass, previous.figures);
                events.push(AlertEvent::new(
                    AlertKind::IssueResolved {
                        issue,
                        mail,
                        stopped,
                    },
                    Status::Pass,
                ));
            }
        }

        let measured = |h: HealthState| h != HealthState::NoData;
        if previous.health != current.health
            && measured(previous.health)
            && measured(current.health)
        {
            events.push(AlertEvent::new(
                AlertKind::HealthChanged {
                    from: previous.health,
                    to: current.health,
                    mail: mail(current.health.status()),
                },
                current.health.status(),
            ));
        }

        if let Some(next) = current.ready_for {
            if previous.ready_for != Some(next) {
                events.push(AlertEvent::new(
                    AlertKind::ReadinessReached {
                        next,
                        evidence: figures.and_then(|f| f.evidence()),
                    },
                    Status::Pass,
                ));
            }
        }

        // New failing senders: only between two states that measured them
        // (a state stored before Marcan did is their baseline).
        if let (Some(known), Some(now)) =
            (&previous.new_failing_senders, &current.new_failing_senders)
        {
            let known: BTreeSet<&str> = known.iter().map(|s| s.source_ip.as_str()).collect();
            for sender in now.iter().filter(|s| !known.contains(s.source_ip.as_str())) {
                events.push(AlertEvent::new(
                    AlertKind::NewFailingSender {
                        sender: sender.clone(),
                        window_days: READINESS_WINDOW_DAYS,
                    },
                    new_sender_status(sender),
                ));
            }
        }
    }

    if let Some((_, after)) = dns_pair {
        for change in dns_changes {
            events.push(record_change_event(change, after));
        }
    }

    events
}

/// The event of one record change: a DMARC record whose policy changed is a
/// [`AlertKind::PolicyChanged`] (one event, whatever else changed with it),
/// coloured by the policy's direction; any other change takes the colour of
/// its check.
fn record_change_event(change: &RecordChange, after: &DnsState) -> AlertEvent {
    if let RecordDelta::DmarcChanged { before, after } = &change.change {
        if before.policy_differs(after) {
            return AlertEvent::new(
                AlertKind::PolicyChanged {
                    from: before.clone(),
                    to: after.clone(),
                },
                policy_change_status(before, after),
            );
        }
    }
    AlertEvent::new(
        AlertKind::DnsRecordChanged {
            check: change.check,
            change: change.change.clone(),
        },
        severity_status(after.worst.get(&change.check).copied()),
    )
}

/// A new failing sender is red when any of its failing mail was delivered
/// (spoofed mail, or a new system's, reached inboxes), amber when the policy
/// blocked it all.
fn new_sender_status(sender: &NewSender) -> Status {
    if sender.delivered_failures > 0 {
        Status::Fail
    } else {
        Status::Warn
    }
}

/// A tightened policy is green; a loosened one amber, or red when spoofed
/// mail would now be delivered (`p=none` or no policy); a change that is
/// neither (`sp=` written out as `p=`, say) states nothing.
fn policy_change_status(from: &DmarcRecordFacts, to: &DmarcRecordFacts) -> Status {
    use std::cmp::Ordering;
    match to.rank().cmp(&from.rank()) {
        Ordering::Greater => Status::Pass,
        Ordering::Equal => Status::Neutral,
        Ordering::Less => match enforcement(to.p.as_deref()).state {
            EnforcementState::Monitor | EnforcementState::Unknown => Status::Fail,
            _ => Status::Warn,
        },
    }
}

/// The colour of a changed check: its worst finding's, when that is a
/// warning or worse; otherwise the change is information.
fn severity_status(worst: Option<Severity>) -> Status {
    match worst {
        Some(Severity::Critical) => Status::Fail,
        Some(Severity::Warning) => Status::Warn,
        _ => Status::Neutral,
    }
}
