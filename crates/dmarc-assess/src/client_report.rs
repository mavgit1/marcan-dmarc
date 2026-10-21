//! What a client report says, decided before it is worded: the
//! organization's verdict and its colour ([`report_verdict`]), which domains
//! get a line of their own and what the line holds ([`report_line`]), and
//! the line over the parked domains ([`parked_line`]). The server's renderer
//! (`mail::client_report`) words and lays out what is decided here and
//! decides nothing itself.
//!
//! The figures carry a domain's line: its messages, the share that passed
//! DMARC, what failed and was still delivered, what the policy blocked and
//! what was forwarded, beside its policy. A verdict word stays only where
//! no figure says it: the next step when it can be taken now, and the open
//! issues none of the figures shows.

use crate::domain::{organization_attention, DomainAssessment, IssueKind};
use crate::readiness::PolicyReadiness;
use crate::status::Status;

/// One domain as its client report judges it.
#[derive(Debug, Clone, Copy)]
pub struct ReportDomainFacts<'a> {
    /// Its issues over the readiness window before sending.
    pub assessment: &'a DomainAssessment,
    /// Its next step, judged on the same window, whose figures the line
    /// shows.
    pub readiness: &'a PolicyReadiness,
    pub parked: bool,
    /// A stored DNS grade from the current checker: an outdated one is
    /// being redone and is left out, as is a domain not checked yet.
    pub dns_current: bool,
}

/// The organization in one line: how many of its domains need attention
/// (an open issue), and the line's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportVerdict {
    pub domains: usize,
    pub needing_attention: usize,
    /// The worst of the open issues; `pass` when every domain is protected
    /// (an all-clear is good news, so green, where the organization's
    /// attention summary is `neutral` for none); `neutral` without domains.
    pub status: Status,
}

/// The verdict over `domains`: [`organization_attention`] with the
/// all-clear in green.
pub fn report_verdict(domains: &[ReportDomainFacts<'_>]) -> ReportVerdict {
    let attention = organization_attention(domains.iter().map(|d| (d.assessment, None)));
    let needing_attention = usize::try_from(attention.domains_needing_attention).unwrap_or(0);
    let status = match (domains.len(), needing_attention) {
        (0, _) => Status::Neutral,
        (_, 0) => Status::Pass,
        _ => attention.status,
    };
    ReportVerdict {
        domains: domains.len(),
        needing_attention,
        status,
    }
}

/// The issues a line's figures and policy already show: delivered failures
/// (their count), not enforcing (the policy beside it) and a parked domain
/// without `p=reject` (the parked line).
const SHOWN_BY_FIGURES: [IssueKind; 3] = [
    IssueKind::DeliveredFailures,
    IssueKind::NotEnforcing,
    IssueKind::ParkedNotRejecting,
];

/// A domain's line: its mail over the readiness window, as the next step
/// is judged on it, then what no figure says.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportLine {
    /// Messages in the window, every outcome.
    pub messages: i64,
    /// The share of `messages` that passed DMARC; `None` without messages.
    pub passed_pct: Option<f64>,
    /// Failed DMARC and still delivered, forwarded mail aside.
    pub delivered_failures: i64,
    /// Their colour: the delivered-failures issue's, `neutral` without it
    /// (a few failures at a passing rate are a figure, not a fault).
    pub delivered_failures_status: Status,
    /// Failed DMARC and quarantined or rejected by the policy.
    pub blocked: i64,
    /// Failed DMARC and delivered because the receiver trusted a forwarder.
    pub forwarded: i64,
    /// The next step is said beside the policy, in this colour, when it
    /// can be taken now: ready for it (`pass`), or raise `pct` or end
    /// testing first (`warn`). `None` otherwise: not ready yet (the figures
    /// show why), enforced, or nothing to judge.
    pub step: Option<Status>,
    /// The open issues none of the figures shows, worst first: a DNS grade
    /// (only a current one), DKIM coverage, stopped reports, a parked
    /// domain's passing mail.
    pub notes: Vec<IssueKind>,
}

/// Where a domain is said in the report.
#[derive(Debug, Clone, PartialEq)]
pub enum ReportPlace {
    /// A line of its own: every domain that is not parked, and a parked one
    /// with an issue besides its policy.
    Line(ReportLine),
    /// A parked domain with nothing but its policy to say: counted on the
    /// parked line only.
    ParkedOnly,
}

/// Where `d` is said, and with a line, what its line holds.
pub fn report_place(d: &ReportDomainFacts<'_>) -> ReportPlace {
    let issues = &d.assessment.issues;
    if d.parked
        && issues
            .iter()
            .all(|i| i.kind == IssueKind::ParkedNotRejecting)
    {
        return ReportPlace::ParkedOnly;
    }
    let w = &d.readiness.window;
    #[allow(clippy::cast_precision_loss)] // message counts are far below 2^52
    let passed_pct = (w.messages > 0).then(|| w.compliant as f64 * 100.0 / w.messages as f64);
    let delivered_failures_status = issues
        .iter()
        .find(|i| i.kind == IssueKind::DeliveredFailures)
        .map_or(Status::Neutral, |i| i.status);
    let notes = issues
        .iter()
        .map(|i| i.kind)
        .filter(|kind| !SHOWN_BY_FIGURES.contains(kind))
        .filter(|&kind| kind != IssueKind::DnsGrade || d.dns_current)
        .collect();
    ReportPlace::Line(ReportLine {
        messages: w.messages,
        passed_pct,
        delivered_failures: w.delivered_failures,
        delivered_failures_status,
        blocked: w.blocked,
        forwarded: w.forwarded_failures,
        step: d.readiness.step_pending().then_some(d.readiness.status),
        notes,
    })
}

/// The line of `d`; `None` when it has none.
pub fn report_line(d: &ReportDomainFacts<'_>) -> Option<ReportLine> {
    match report_place(d) {
        ReportPlace::Line(line) => Some(line),
        ReportPlace::ParkedOnly => None,
    }
}

/// The line over the parked domains: how many there are, which of them
/// (indexes into the report's domains) do not publish `p=reject`, and the
/// line's colour: the worst of their issue, `neutral` when all reject
/// (nothing to act on, said plainly).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParkedLine {
    pub parked: usize,
    pub not_rejecting: Vec<usize>,
    pub status: Status,
}

/// The parked line over `domains`; `None` without a parked domain.
pub fn parked_line(domains: &[ReportDomainFacts<'_>]) -> Option<ParkedLine> {
    let parked = domains.iter().filter(|d| d.parked).count();
    if parked == 0 {
        return None;
    }
    let not_rejecting_issue = |d: &ReportDomainFacts<'_>| {
        d.assessment
            .issues
            .iter()
            .find(|i| i.kind == IssueKind::ParkedNotRejecting)
            .map(|i| i.status)
    };
    let not_rejecting: Vec<usize> = domains
        .iter()
        .enumerate()
        .filter(|(_, d)| d.parked && not_rejecting_issue(d).is_some())
        .map(|(i, _)| i)
        .collect();
    let status = Status::worst_of(
        not_rejecting
            .iter()
            .filter_map(|&i| not_rejecting_issue(&domains[i])),
    );
    Some(ParkedLine {
        parked,
        not_rejecting,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{assess_domain, DomainFacts};
    use crate::mail::Mail;
    use crate::readiness::{policy_readiness, ReadinessFacts};

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    /// `total` messages, `failures` of them failing and delivered, under
    /// `policy`, reports for 60 days.
    fn domain(
        policy: &str,
        total: i64,
        failures: i64,
        parked: bool,
    ) -> (DomainAssessment, PolicyReadiness) {
        with_mail(
            policy,
            Mail {
                total,
                compliant: total - failures,
                delivered_failures: failures,
                forwarded_failures: 0,
            },
            parked,
        )
    }

    fn with_mail(policy: &str, mail: Mail, parked: bool) -> (DomainAssessment, PolicyReadiness) {
        let assessment = assess_domain(
            &DomainFacts {
                mail,
                parked,
                policy: Some(policy),
                dns_grade: Some("A"),
                last_report_at: Some(NOW - DAY),
                ..Default::default()
            },
            NOW,
            7,
        );
        let readiness = policy_readiness(
            &ReadinessFacts {
                window_mail: mail,
                policy: Some(policy),
                pct: None,
                testing: false,
                parked,
                policy_messages: mail.total,
                policy_first_report_at: Some(NOW - 60 * DAY),
                other_policy: None,
            },
            NOW,
        );
        (assessment, readiness)
    }

    fn facts(d: &(DomainAssessment, PolicyReadiness), parked: bool) -> ReportDomainFacts<'_> {
        ReportDomainFacts {
            assessment: &d.0,
            readiness: &d.1,
            parked,
            dns_current: true,
        }
    }

    #[test]
    fn the_verdict_counts_domains_needing_attention_and_an_all_clear_is_green() {
        let good = domain("reject", 1000, 0, false);
        let bad = domain("none", 1000, 60, false);
        assert_eq!(
            report_verdict(&[facts(&good, false)]),
            ReportVerdict {
                domains: 1,
                needing_attention: 0,
                status: Status::Pass
            }
        );
        let verdict = report_verdict(&[facts(&good, false), facts(&bad, false)]);
        assert_eq!((verdict.domains, verdict.needing_attention), (2, 1));
        assert_eq!(verdict.status, Status::Fail);
        // It agrees with the organization's attention summary but for the
        // all-clear's colour.
        let attention = organization_attention([(&good.0, None), (&bad.0, None)]);
        assert_eq!(verdict.status, attention.status);
        assert_eq!(
            organization_attention([(&good.0, None)]).status,
            Status::Neutral
        );
        assert_eq!(report_verdict(&[]).status, Status::Neutral);
    }

    /// Every sending domain gets a line of figures: an enforced one with
    /// nothing to say as much as one with issues.
    #[test]
    fn a_line_carries_the_windows_figures() {
        // 1,000 messages: 900 passed, 50 delivered failures, 20 forwarded,
        // 30 blocked.
        let mail = Mail {
            total: 1000,
            compliant: 900,
            delivered_failures: 50,
            forwarded_failures: 20,
        };
        let d = with_mail("quarantine", mail, false);
        let line = report_line(&facts(&d, false)).expect("a line");
        assert_eq!(
            (
                line.messages,
                line.delivered_failures,
                line.blocked,
                line.forwarded
            ),
            (1000, 50, 30, 20)
        );
        assert_eq!(line.passed_pct, Some(90.0));
        assert_eq!(line.delivered_failures_status, Status::Fail);
        // Not ready for p=reject: the figures say why, no word does.
        assert_eq!(line.step, None);
        assert!(line.notes.is_empty(), "{:?}", line.notes);

        let enforced = domain("reject", 1000, 0, false);
        let line = report_line(&facts(&enforced, false)).expect("a line");
        assert_eq!(line.passed_pct, Some(100.0));
        assert_eq!(line.delivered_failures_status, Status::Neutral);
        assert_eq!((line.step, line.notes.len()), (None, 0));

        let silent = domain("reject", 0, 0, false);
        assert_eq!(
            report_line(&facts(&silent, false)).unwrap().passed_pct,
            None
        );
    }

    /// A step that can be taken now is the one verdict word a line keeps;
    /// not enforcing is the policy beside the figures, not a note.
    #[test]
    fn the_step_is_said_only_when_it_can_be_taken() {
        let ready = domain("none", 1000, 0, false);
        assert!(ready
            .0
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::NotEnforcing));
        let line = report_line(&facts(&ready, false)).expect("a line");
        assert_eq!(line.step, Some(Status::Pass));
        assert!(line.notes.is_empty(), "{:?}", line.notes);

        let failing = domain("none", 1000, 60, false);
        let line = report_line(&facts(&failing, false)).expect("a line");
        assert_eq!(line.step, None);
        assert_eq!(line.delivered_failures, 60);
    }

    #[test]
    fn a_grade_is_a_note_only_when_current() {
        let mut failing = domain("reject", 1000, 0, false);
        failing.0.dns = Status::Fail;
        failing.0.issues.push(crate::domain::Issue {
            kind: IssueKind::DnsGrade,
            severity: crate::domain::IssueSeverity::High,
            status: Status::Fail,
        });
        let mut f = facts(&failing, false);
        assert_eq!(report_line(&f).unwrap().notes, [IssueKind::DnsGrade]);
        f.dns_current = false;
        assert!(report_line(&f).unwrap().notes.is_empty());
    }

    #[test]
    fn parked_domains_are_said_on_their_own_line() {
        let rejecting = domain("reject", 0, 0, true);
        let lax = domain("none", 0, 0, true);
        let other = domain("reject", 1000, 0, false);
        let domains = [
            facts(&other, false),
            facts(&rejecting, true),
            facts(&lax, true),
        ];
        // Not rejecting is said on the parked line only; quiet parked
        // domains have no line of figures.
        assert_eq!(report_place(&domains[1]), ReportPlace::ParkedOnly);
        assert_eq!(report_place(&domains[2]), ReportPlace::ParkedOnly);
        let line = parked_line(&domains).expect("a parked line");
        assert_eq!(line.parked, 2);
        assert_eq!(line.not_rejecting, [2]);
        assert_eq!(line.status, lax.0.issues[0].status);
        let all = parked_line(&domains[..2]).expect("a parked line");
        assert_eq!((all.not_rejecting.len(), all.status), (0, Status::Neutral));
        assert_eq!(parked_line(&domains[..1]), None);
        // A parked domain that passes mail gets a line, with that note.
        let sending = domain("reject", 40, 0, true);
        let line = report_line(&facts(&sending, true)).expect("a line");
        assert_eq!(line.notes, [IssueKind::ParkedPassingMail]);
    }
}
