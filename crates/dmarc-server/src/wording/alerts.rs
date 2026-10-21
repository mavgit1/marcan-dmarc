//! An alert event in words: one sentence per kind ([`alert_line`]), naming
//! the numbers the event carries, and its colour in a word
//! ([`status_word`]); the catalogue of kinds Settings → Alerts lists
//! ([`catalogue_title`], [`catalogue_trigger`], [`group_title`]). The alert email, the digest and the client report
//! word events with these; `GET /api/alerts` sends both with every event
//! (`AlertItem::text`, `AlertItem::severity_label`).

use super::issues::issue_name;
use super::{capitalized, count_noun, format_pct};
use dmarc_assess::{
    AlertGroup, AlertKind, AlertedIssue, BimiRecordFacts, CatalogueKind, CheckKind,
    DeliveredFigures, DmarcRecordFacts, EnforcementState, HealthState, MtaStsRecordFacts,
    NewSender, PolicyStep, RecordDelta, Status, NEW_SENDER_MIN_FAILURES, READINESS_WINDOW_DAYS,
};

/// A line's colour in a word: the alert email's plain text, and the badge
/// beside each event in the UI.
pub fn status_word(status: Status) -> &'static str {
    match status {
        Status::Fail => "Problem",
        Status::Warn => "Warning",
        Status::Pass => "Good",
        Status::Info | Status::Forward | Status::Neutral => "Note",
    }
}

/// The heading of a group of the alert catalogue (Settings → Alerts).
pub fn group_title(group: AlertGroup) -> &'static str {
    match group {
        AlertGroup::Issues => "Issues",
        AlertGroup::Health => "Health",
        AlertGroup::DnsRecords => "DNS records",
        AlertGroup::Policy => "Policy",
        AlertGroup::Readiness => "Readiness",
        AlertGroup::Parked => "Parked",
        AlertGroup::Senders => "Senders",
    }
}

/// A kind of alert as Settings → Alerts lists it: `New issue: delivered
/// failures`, `SPF record changed`.
pub fn catalogue_title(kind: CatalogueKind) -> String {
    match kind {
        CatalogueKind::IssueOpened(issue) => format!("New issue: {}", issue_name(issue.kind())),
        CatalogueKind::IssueResolved(issue) => format!("Resolved: {}", issue_name(issue.kind())),
        CatalogueKind::HealthChanged => "Health changed".to_owned(),
        CatalogueKind::DnsRecordChanged(CheckKind::Dkim) => "DKIM key changed".to_owned(),
        CatalogueKind::DnsRecordChanged(CheckKind::MtaSts) => "MTA-STS changed".to_owned(),
        CatalogueKind::DnsRecordChanged(check) => format!("{} record changed", check_name(check)),
        CatalogueKind::PolicyChanged => "DMARC policy changed".to_owned(),
        CatalogueKind::ReadinessReached => "Ready for the next policy step".to_owned(),
        CatalogueKind::ParkedChanged { parked: true } => "Marked as parked".to_owned(),
        CatalogueKind::ParkedChanged { parked: false } => "No longer parked".to_owned(),
        CatalogueKind::NewFailingSender => "New failing sender".to_owned(),
    }
}

/// What makes a kind of alert, in one sentence.
pub fn catalogue_trigger(kind: CatalogueKind) -> String {
    match kind {
        CatalogueKind::NewFailingSender => format!(
            "A sender that sent nothing as the domain before the last {READINESS_WINDOW_DAYS} days had at least {NEW_SENDER_MIN_FAILURES} messages fail DMARC in them (forwarded mail not counted)."
        ),
        _ => catalogue_trigger_line(kind).to_owned(),
    }
}

fn catalogue_trigger_line(kind: CatalogueKind) -> &'static str {
    use AlertedIssue::{DeliveredFailures, ReportsStopped};
    match kind {
        CatalogueKind::IssueOpened(DeliveredFailures) => {
            "An issue opened: mail that failed DMARC is delivered at a warning or failing rate."
        }
        CatalogueKind::IssueResolved(DeliveredFailures) => {
            "That issue cleared: failing mail is no longer delivered at that rate."
        }
        CatalogueKind::IssueOpened(ReportsStopped) => {
            "An issue opened: reports for the domain arrived before and then none for the days of the domain warning (Settings → Mailbox)."
        }
        CatalogueKind::IssueResolved(ReportsStopped) => {
            "That issue cleared: reports for the domain arrive again."
        }
        CatalogueKind::HealthChanged => {
            "The domain's health moved between two measured states (to or from no data is not alerted)."
        }
        CatalogueKind::DnsRecordChanged(check) => match check {
            CheckKind::Dmarc => {
                "A DMARC record was published or removed, or its rua or alignment changed (a new p, sp or pct is a DMARC policy change)."
            }
            CheckKind::Spf => {
                "An SPF record was published or removed, terms were added or removed, or the lookup count moved."
            }
            CheckKind::Dkim => {
                "A DKIM key was published, changed, revoked or removed under a selector checked before."
            }
            CheckKind::MtaSts => {
                "An MTA-STS record was published or removed, its id changed, or the policy's mode or MX hosts changed."
            }
            CheckKind::TlsRpt => {
                "A TLS-RPT record was published or removed, or its report address changed."
            }
            CheckKind::Bimi => {
                "A BIMI record was published or removed, or its logo changed or was declined."
            }
        },
        CatalogueKind::PolicyChanged => {
            "The DMARC record in DNS changed its p, sp or pct (other tags changed with them are named too)."
        }
        CatalogueKind::ReadinessReached => {
            "The domain became ready for its next policy step: quarantine, then reject."
        }
        CatalogueKind::ParkedChanged { parked: true } => {
            "The domain was marked as parked: one alert, and its issues, health and readiness start over from there instead of each raising one."
        }
        CatalogueKind::ParkedChanged { parked: false } => {
            "The domain is no longer marked as parked: one alert, and its issues, health and readiness start over from there instead of each raising one."
        }
        CatalogueKind::NewFailingSender => unreachable!("worded with its numbers"),
    }
}

fn health_name(health: HealthState) -> &'static str {
    match health {
        HealthState::NoData => "No data",
        HealthState::AllBlocked => "All blocked",
        HealthState::Protected => "Protected",
        HealthState::Warning => "Warning",
        HealthState::Critical => "Critical",
    }
}

fn check_name(check: CheckKind) -> &'static str {
    match check {
        CheckKind::Dmarc => "DMARC",
        CheckKind::Spf => "SPF",
        CheckKind::Dkim => "DKIM",
        CheckKind::MtaSts => "MTA-STS",
        CheckKind::TlsRpt => "TLS-RPT",
        CheckKind::Bimi => "BIMI",
    }
}

/// `p=quarantine`, `p=reject`.
pub(crate) fn step_name(step: PolicyStep) -> &'static str {
    match step {
        PolicyStep::Quarantine => "p=quarantine",
        PolicyStep::Reject => "p=reject",
    }
}

/// A policy mail was evaluated under: the readiness evidence's current
/// policy, or the one the rest of the window's mail was still under;
/// `unknown` when reports name none or one Marcan does not know.
pub(crate) fn policy_name(policy: EnforcementState) -> &'static str {
    match policy {
        EnforcementState::Monitor => "p=none",
        EnforcementState::Quarantine => "p=quarantine",
        EnforcementState::Reject => "p=reject",
        EnforcementState::Unknown => "an unknown policy",
    }
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items.join(", ")
    }
}

/// `tag before → after` for each DMARC tag that differs.
fn dmarc_tag_changes(before: &DmarcRecordFacts, after: &DmarcRecordFacts) -> Vec<String> {
    let opt = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".into());
    let mut out = Vec::new();
    let mut tag = |name: &str, a: String, b: String| {
        if a != b {
            out.push(format!("{name} {a} → {b}"));
        }
    };
    tag("p", opt(&before.p), opt(&after.p));
    tag("sp", opt(&before.sp), opt(&after.sp));
    tag("pct", before.pct.to_string(), after.pct.to_string());
    tag("rua", list_or_none(&before.rua), list_or_none(&after.rua));
    tag("adkim", before.adkim.clone(), after.adkim.clone());
    tag("aspf", before.aspf.clone(), after.aspf.clone());
    out
}

fn mta_sts_changes(before: &MtaStsRecordFacts, after: &MtaStsRecordFacts) -> Vec<String> {
    let opt = |v: Option<&str>| v.unwrap_or("—").to_owned();
    let mut out = Vec::new();
    if before.id != after.id {
        out.push(format!(
            "id {} → {}",
            opt(before.id.as_deref()),
            opt(after.id.as_deref())
        ));
    }
    if let (Some(a), Some(b)) = (&before.policy, &after.policy) {
        if a.mode != b.mode {
            out.push(format!(
                "mode {} → {}",
                opt(a.mode.as_deref()),
                opt(b.mode.as_deref())
            ));
        }
        if a.mx != b.mx {
            out.push(format!(
                "mx {} → {}",
                list_or_none(&a.mx),
                list_or_none(&b.mx)
            ));
        }
    }
    out
}

fn bimi_logo(b: &BimiRecordFacts) -> String {
    if b.declined {
        "declined".to_owned()
    } else {
        b.logo_url.clone().unwrap_or_else(|| "—".into())
    }
}

fn record_change(check: CheckKind, change: &RecordDelta) -> String {
    let name = check_name(check);
    match change {
        RecordDelta::Published => format!("{name} record published."),
        RecordDelta::Removed => format!("{name} record removed."),
        RecordDelta::SpfChanged {
            added,
            removed,
            lookups_before,
            lookups_after,
        } => {
            let mut parts = Vec::new();
            if !added.is_empty() {
                parts.push(format!("added {}", added.join(", ")));
            }
            if !removed.is_empty() {
                parts.push(format!("removed {}", removed.join(", ")));
            }
            if lookups_before != lookups_after {
                parts.push(format!("DNS lookups {lookups_before} → {lookups_after}"));
            }
            format!("SPF record changed: {}.", parts.join("; "))
        }
        RecordDelta::DmarcChanged { before, after } => format!(
            "DMARC record changed: {}.",
            dmarc_tag_changes(before, after).join(", ")
        ),
        RecordDelta::DkimKeyAppeared { selector } => {
            format!("DKIM key published for selector {selector}.")
        }
        RecordDelta::DkimKeyRemoved { selector } => {
            format!("DKIM key removed for selector {selector}.")
        }
        RecordDelta::DkimKeyChanged { selector } => {
            format!("DKIM key changed for selector {selector}.")
        }
        RecordDelta::DkimKeyRevoked { selector } => {
            format!("DKIM key revoked for selector {selector}.")
        }
        RecordDelta::MtaStsChanged { before, after } => format!(
            "MTA-STS policy changed: {}.",
            mta_sts_changes(before, after).join(", ")
        ),
        RecordDelta::TlsRptChanged {
            rua_before,
            rua_after,
        } => format!(
            "TLS-RPT report address changed: {} → {}.",
            list_or_none(rua_before),
            list_or_none(rua_after)
        ),
        RecordDelta::BimiChanged { before, after } => format!(
            "BIMI logo changed: {} → {}.",
            bimi_logo(before),
            bimi_logo(after)
        ),
    }
}

/// The evidence a step was reached on: `30 days and 1,204 messages under
/// p=none, 99.6% delivered compliance`. The alert of a step reached and the
/// readiness verdict "Ready for …" both say it so.
pub(crate) fn ready_evidence(
    days: i64,
    messages: i64,
    policy: EnforcementState,
    delivered_compliance_pct: Option<f64>,
) -> String {
    let compliance = delivered_compliance_pct
        .map(|pct| format!(", {} delivered compliance", format_pct(pct)))
        .unwrap_or_default();
    format!(
        "{} and {} under {}{compliance}",
        count_noun(days, "day"),
        count_noun(messages, "message"),
        policy_name(policy)
    )
}

/// The sentence of one event.
pub fn alert_line(kind: &AlertKind) -> String {
    match kind {
        AlertKind::IssueOpened {
            issue,
            mail,
            stopped,
        } => match (issue, mail, stopped) {
            (AlertedIssue::DeliveredFailures, Some(mail), _) => {
                format!("{}.", capitalized(&delivered_phrase(mail)))
            }
            (AlertedIssue::ReportsStopped, _, Some(s)) => format!(
                "No reports for {}. (Threshold {}.)",
                count_noun(s.days, "day"),
                count_noun(s.threshold_days, "day")
            ),
            _ => format!("New issue: {}.", issue_name(issue.kind())),
        },
        AlertKind::IssueResolved {
            issue,
            mail,
            stopped,
        } => match (issue, mail, stopped) {
            (AlertedIssue::DeliveredFailures, Some(mail), _) => {
                format!("Delivered failures resolved: {}.", delivered_phrase(mail))
            }
            (AlertedIssue::ReportsStopped, _, Some(s)) => format!(
                "Reports are arriving again after {} without any. (Threshold {}.)",
                count_noun(s.days, "day"),
                count_noun(s.threshold_days, "day")
            ),
            _ => format!("Resolved: {}.", issue_name(issue.kind())),
        },
        AlertKind::HealthChanged { from, to, mail } => match mail {
            Some(mail) => format!(
                "Health changed from {} to {}: {}.",
                health_name(*from),
                health_name(*to),
                delivered_phrase(mail)
            ),
            None => format!(
                "Health changed from {} to {}.",
                health_name(*from),
                health_name(*to)
            ),
        },
        AlertKind::DnsRecordChanged { check, change } => record_change(*check, change),
        AlertKind::PolicyChanged { from, to } => format!(
            "DMARC policy changed: {}.",
            dmarc_tag_changes(from, to).join(", ")
        ),
        AlertKind::ReadinessReached { next, evidence } => match evidence {
            Some(e) => format!(
                "Ready for {}: {}.",
                step_name(*next),
                ready_evidence(e.days, e.messages, e.policy, e.delivered_compliance_pct)
            ),
            None => format!("Ready for the next policy step: {}.", step_name(*next)),
        },
        AlertKind::ParkedChanged { parked: true } => "Marked as parked.".to_owned(),
        AlertKind::ParkedChanged { parked: false } => "No longer parked.".to_owned(),
        AlertKind::NewFailingSender {
            sender,
            window_days,
        } => new_sender_line(sender, *window_days),
    }
}

/// "New sender 192.0.2.1 (Example Mail) failed DMARC with 42 messages in the
/// last 30 days (12 delivered) and sent nothing as the domain before. A new
/// system sending as the domain, or spoofing: check its sender page."
fn new_sender_line(sender: &NewSender, window_days: i64) -> String {
    let name = sender
        .name
        .as_ref()
        .map(|n| format!(" ({n})"))
        .unwrap_or_default();
    let delivered = match sender.delivered_failures {
        0 => "all blocked by policy".to_owned(),
        d if d >= sender.failed => "all delivered".to_owned(),
        d => format!("{} delivered", super::format_count(d)),
    };
    format!(
        "New sender {}{name} failed DMARC with {} in the last {} ({delivered}) and sent nothing as the domain before. \
         A new system sending as the domain, or spoofing: check its sender page.",
        sender.source_ip,
        count_noun(sender.failed, "message"),
        count_noun(window_days, "day"),
    )
}

/// The window's delivered mail against the event's threshold: "12
/// delivered failures in the last 30 days (6.0% of delivered mail,
/// threshold 5%)"; "nothing delivered in the last 30 days, 40 messages
/// blocked by policy" when nothing was.
fn delivered_phrase(m: &DeliveredFigures) -> String {
    let window = count_noun(m.window_days, "day");
    match m.failure_pct {
        Some(pct) => format!(
            "{} in the last {window} ({} of delivered mail, threshold {}%)",
            count_noun(m.delivered_failures, "delivered failure"),
            format_pct(pct),
            m.threshold_pct
        ),
        None => format!(
            "nothing delivered in the last {window}, {} blocked by policy",
            count_noun(m.blocked, "message")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_assess::{
        BimiRecordFacts, MtaStsPolicyFacts, ReadinessEvidence, StoppedFigures, ALERT_CATALOGUE,
    };

    fn dmarc() -> DmarcRecordFacts {
        DmarcRecordFacts {
            p: Some("none".into()),
            sp: None,
            pct: 100,
            rua: vec!["mailto:d@example.com".into()],
            adkim: "r".into(),
            aspf: "r".into(),
        }
    }

    fn mail(delivered_failures: i64, delivered: i64, threshold_pct: f64) -> DeliveredFigures {
        DeliveredFigures {
            window_days: 30,
            delivered,
            delivered_failures,
            blocked: 40,
            failure_pct: (delivered > 0)
                .then(|| delivered_failures as f64 * 100.0 / delivered as f64),
            threshold_pct,
        }
    }

    fn stopped(days: i64, threshold_days: i64) -> StoppedFigures {
        StoppedFigures {
            days,
            threshold_days,
        }
    }

    /// Every kind, every record change and every form with and without its
    /// figures, with the sentence the email and the UI show for it.
    fn sentence_cases() -> Vec<(AlertKind, &'static str)> {
        let sts = |id: &str, mode: &str, mx: &[&str]| MtaStsRecordFacts {
            id: Some(id.into()),
            policy: Some(MtaStsPolicyFacts {
                mode: Some(mode.into()),
                mx: mx.iter().map(|m| (*m).to_owned()).collect(),
            }),
        };
        let bimi = |logo: Option<&str>, declined| BimiRecordFacts {
            logo_url: logo.map(str::to_owned),
            authority_url: None,
            declined,
        };
        let change = |check, change| AlertKind::DnsRecordChanged { check, change };
        let selector = || "s1".to_owned();
        vec![
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: Some(mail(12, 840, 1.0)),
                    stopped: None,
                },
                "12 delivered failures in the last 30 days (1.4% of delivered mail, threshold 1%).",
            ),
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: Some(mail(1_250, 12_480, 5.0)),
                    stopped: None,
                },
                "1,250 delivered failures in the last 30 days (10.0% of delivered mail, threshold 5%).",
            ),
            (
                AlertKind::IssueResolved {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: Some(mail(1, 1_000, 1.0)),
                    stopped: None,
                },
                "Delivered failures resolved: 1 delivered failure in the last 30 days (0.1% of delivered mail, threshold 1%).",
            ),
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: Some(stopped(9, 7)),
                },
                "No reports for 9 days. (Threshold 7 days.)",
            ),
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: Some(stopped(1, 1)),
                },
                "No reports for 1 day. (Threshold 1 day.)",
            ),
            (
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: Some(stopped(12, 7)),
                },
                "Reports are arriving again after 12 days without any. (Threshold 7 days.)",
            ),
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                "New issue: delivered failures.",
            ),
            (
                AlertKind::IssueOpened {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                "New issue: reports stopped.",
            ),
            (
                AlertKind::IssueResolved {
                    issue: AlertedIssue::DeliveredFailures,
                    mail: None,
                    stopped: None,
                },
                "Resolved: delivered failures.",
            ),
            (
                AlertKind::IssueResolved {
                    issue: AlertedIssue::ReportsStopped,
                    mail: None,
                    stopped: None,
                },
                "Resolved: reports stopped.",
            ),
            (
                AlertKind::HealthChanged {
                    from: HealthState::Protected,
                    to: HealthState::Critical,
                    mail: Some(mail(30, 400, 5.0)),
                },
                "Health changed from Protected to Critical: 30 delivered failures in the last 30 days (7.5% of delivered mail, threshold 5%).",
            ),
            (
                AlertKind::HealthChanged {
                    from: HealthState::Warning,
                    to: HealthState::AllBlocked,
                    mail: Some(mail(0, 0, 1.0)),
                },
                "Health changed from Warning to All blocked: nothing delivered in the last 30 days, 40 messages blocked by policy.",
            ),
            (
                AlertKind::HealthChanged {
                    from: HealthState::NoData,
                    to: HealthState::Warning,
                    mail: None,
                },
                "Health changed from No data to Warning.",
            ),
            (
                AlertKind::PolicyChanged {
                    from: dmarc(),
                    to: DmarcRecordFacts {
                        p: Some("quarantine".into()),
                        pct: 50,
                        ..dmarc()
                    },
                },
                "DMARC policy changed: p none → quarantine, pct 100 → 50.",
            ),
            (
                AlertKind::PolicyChanged {
                    from: dmarc(),
                    to: DmarcRecordFacts {
                        p: Some("reject".into()),
                        sp: Some("none".into()),
                        rua: vec![],
                        ..dmarc()
                    },
                },
                "DMARC policy changed: p none → reject, sp — → none, rua mailto:d@example.com → none.",
            ),
            (
                AlertKind::ReadinessReached {
                    next: PolicyStep::Quarantine,
                    evidence: Some(ReadinessEvidence {
                        policy: EnforcementState::Monitor,
                        days: 30,
                        messages: 1_204,
                        delivered_compliance_pct: Some(99.6),
                    }),
                },
                "Ready for p=quarantine: 30 days and 1,204 messages under p=none, 99.6% delivered compliance.",
            ),
            (
                AlertKind::ReadinessReached {
                    next: PolicyStep::Reject,
                    evidence: Some(ReadinessEvidence {
                        policy: EnforcementState::Quarantine,
                        days: 31,
                        messages: 1,
                        delivered_compliance_pct: None,
                    }),
                },
                "Ready for p=reject: 31 days and 1 message under p=quarantine.",
            ),
            (
                AlertKind::ReadinessReached {
                    next: PolicyStep::Reject,
                    evidence: None,
                },
                "Ready for the next policy step: p=reject.",
            ),
            (
                AlertKind::ParkedChanged { parked: true },
                "Marked as parked.",
            ),
            (
                AlertKind::ParkedChanged { parked: false },
                "No longer parked.",
            ),
            (
                change(CheckKind::MtaSts, RecordDelta::Published),
                "MTA-STS record published.",
            ),
            (
                change(CheckKind::Bimi, RecordDelta::Removed),
                "BIMI record removed.",
            ),
            (
                change(
                    CheckKind::Spf,
                    RecordDelta::SpfChanged {
                        added: vec!["include:mail.example.org".into()],
                        removed: vec!["-all".into()],
                        lookups_before: 9,
                        lookups_after: 11,
                    },
                ),
                "SPF record changed: added include:mail.example.org; removed -all; DNS lookups 9 → 11.",
            ),
            (
                change(
                    CheckKind::Dmarc,
                    RecordDelta::DmarcChanged {
                        before: dmarc(),
                        after: DmarcRecordFacts {
                            adkim: "s".into(),
                            ..dmarc()
                        },
                    },
                ),
                "DMARC record changed: adkim r → s.",
            ),
            (
                change(
                    CheckKind::Dkim,
                    RecordDelta::DkimKeyAppeared {
                        selector: selector(),
                    },
                ),
                "DKIM key published for selector s1.",
            ),
            (
                change(
                    CheckKind::Dkim,
                    RecordDelta::DkimKeyRemoved {
                        selector: selector(),
                    },
                ),
                "DKIM key removed for selector s1.",
            ),
            (
                change(
                    CheckKind::Dkim,
                    RecordDelta::DkimKeyChanged {
                        selector: selector(),
                    },
                ),
                "DKIM key changed for selector s1.",
            ),
            (
                change(
                    CheckKind::Dkim,
                    RecordDelta::DkimKeyRevoked {
                        selector: selector(),
                    },
                ),
                "DKIM key revoked for selector s1.",
            ),
            (
                change(
                    CheckKind::MtaSts,
                    RecordDelta::MtaStsChanged {
                        before: sts("1", "testing", &["mx1.example.com"]),
                        after: sts("2", "enforce", &["mx1.example.com", "mx2.example.com"]),
                    },
                ),
                "MTA-STS policy changed: id 1 → 2, mode testing → enforce, mx mx1.example.com → mx1.example.com, mx2.example.com.",
            ),
            (
                change(
                    CheckKind::TlsRpt,
                    RecordDelta::TlsRptChanged {
                        rua_before: vec![],
                        rua_after: vec!["mailto:tls@example.com".into()],
                    },
                ),
                "TLS-RPT report address changed: none → mailto:tls@example.com.",
            ),
            (
                change(
                    CheckKind::Bimi,
                    RecordDelta::BimiChanged {
                        before: bimi(Some("https://example.com/logo.svg"), false),
                        after: bimi(None, true),
                    },
                ),
                "BIMI logo changed: https://example.com/logo.svg → declined.",
            ),
            (
                change(
                    CheckKind::Spf,
                    RecordDelta::SpfChanged {
                        added: vec!["include:mailgun.org".into()],
                        removed: vec![],
                        lookups_before: 7,
                        lookups_after: 8,
                    },
                ),
                "SPF record changed: added include:mailgun.org; DNS lookups 7 → 8.",
            ),
            (
                AlertKind::PolicyChanged {
                    from: DmarcRecordFacts {
                        p: Some("reject".into()),
                        ..dmarc()
                    },
                    to: DmarcRecordFacts {
                        pct: 50,
                        ..dmarc()
                    },
                },
                "DMARC policy changed: p reject → none, pct 100 → 50.",
            ),
            (
                new_sender(Some("Example Mail"), 42, 12),
                "New sender 203.0.113.5 (Example Mail) failed DMARC with 42 messages in the last 30 days (12 delivered) and sent nothing as the domain before. A new system sending as the domain, or spoofing: check its sender page.",
            ),
            (
                new_sender(None, 1_250, 0),
                "New sender 203.0.113.5 failed DMARC with 1,250 messages in the last 30 days (all blocked by policy) and sent nothing as the domain before. A new system sending as the domain, or spoofing: check its sender page.",
            ),
            (
                new_sender(None, 10, 10),
                "New sender 203.0.113.5 failed DMARC with 10 messages in the last 30 days (all delivered) and sent nothing as the domain before. A new system sending as the domain, or spoofing: check its sender page.",
            ),
        ]
    }

    fn new_sender(name: Option<&str>, failed: i64, delivered_failures: i64) -> AlertKind {
        AlertKind::NewFailingSender {
            sender: NewSender {
                source_ip: "203.0.113.5".into(),
                name: name.map(str::to_owned),
                failed,
                delivered_failures,
            },
            window_days: 30,
        }
    }

    /// The trigger names the threshold and the window the rule uses.
    #[test]
    fn the_new_sender_trigger_names_its_numbers() {
        let trigger = catalogue_trigger(CatalogueKind::NewFailingSender);
        assert!(trigger.contains(&format!("last {READINESS_WINDOW_DAYS} days")));
        assert!(trigger.contains(&format!("at least {NEW_SENDER_MIN_FAILURES} messages")));
    }

    #[test]
    fn every_kind_reads_as_a_sentence() {
        let cases = sentence_cases();
        for (kind, sentence) in &cases {
            assert_eq!(alert_line(kind), *sentence, "{kind:?}");
        }
        // Every catalogue entry (every kind the evaluation raises) is among them.
        for entry in &ALERT_CATALOGUE {
            assert!(
                cases
                    .iter()
                    .any(|(kind, _)| dmarc_assess::catalogue_id(kind) == entry.id()),
                "no sentence tested for {}",
                entry.id()
            );
        }
    }

    #[test]
    fn every_catalogue_kind_has_a_title_and_one_sentence() {
        for &entry in &ALERT_CATALOGUE {
            let (title, trigger) = (catalogue_title(entry), catalogue_trigger(entry));
            assert!(
                !title.ends_with('.') && trigger.ends_with('.'),
                "{}",
                entry.id()
            );
        }
        assert_eq!(
            catalogue_title(CatalogueKind::IssueResolved(
                AlertedIssue::DeliveredFailures
            )),
            "Resolved: delivered failures"
        );
        assert_eq!(
            catalogue_title(CatalogueKind::DnsRecordChanged(CheckKind::TlsRpt)),
            "TLS-RPT record changed"
        );
    }

    /// The table cells of `docs/alerts.md` under the header `header`, until
    /// the table ends.
    fn docs_table(header: &str) -> Vec<Vec<String>> {
        let docs =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/alerts.md"))
                .expect("docs/alerts.md");
        docs.lines()
            .skip_while(|l| *l != header)
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                l.trim_matches('|')
                    .split(" | ")
                    .map(|c| c.trim().to_owned())
                    .collect()
            })
            .collect()
    }

    /// The docs list the catalogue in these words (each group named on its
    /// first row), and every example sentence is one this module writes: a
    /// changed word fails here until the docs follow.
    #[test]
    fn the_docs_say_what_the_catalogue_and_the_sentences_say() {
        let mut expected = Vec::new();
        let mut last_group = None;
        for &entry in &ALERT_CATALOGUE {
            let group = entry.group();
            let shown = if last_group == Some(group) {
                String::new()
            } else {
                group_title(group).to_owned()
            };
            last_group = Some(group);
            expected.push(vec![
                shown,
                catalogue_title(entry),
                catalogue_trigger(entry),
            ]);
        }
        assert_eq!(
            docs_table("| Group | Alert | When |"),
            expected,
            "docs/alerts.md's table of alert kinds"
        );
        let sentences: Vec<String> = sentence_cases()
            .iter()
            .map(|(kind, _)| alert_line(kind))
            .collect();
        let examples = docs_table("| Change | Example | Colour |");
        assert!(!examples.is_empty());
        for row in examples {
            assert!(
                sentences.contains(&row[1]),
                "docs/alerts.md's example is no sentence Marcan writes: {}",
                row[1]
            );
        }
    }

    #[test]
    fn every_colour_and_group_has_a_word() {
        assert_eq!(status_word(Status::Fail), "Problem");
        assert_eq!(status_word(Status::Warn), "Warning");
        assert_eq!(status_word(Status::Pass), "Good");
        assert_eq!(status_word(Status::Neutral), "Note");
        for entry in &ALERT_CATALOGUE {
            assert!(!group_title(entry.group()).is_empty());
        }
        assert_eq!(group_title(AlertGroup::DnsRecords), "DNS records");
    }
}
