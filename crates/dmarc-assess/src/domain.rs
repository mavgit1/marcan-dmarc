//! A domain's verdicts and open issues; a sender's verdict per domain; an
//! organization's attention summary.

use crate::mail::{
    dkim_coverage, enforcement, grade_status, DkimCoverage, Enforcement, EnforcementState, Mail,
};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const DAY_SECS: i64 = 86_400;

/// Whole days since the last report when that is at least `after_days`
/// (the monitoring setting `domain_reports_stopped_days`: a domain that had
/// reports and then none for this long has probably stopped reporting);
/// `None` before the first report or while fresh.
pub fn reports_stopped_days(last_report_at: Option<i64>, now: i64, after_days: i64) -> Option<i64> {
    let days = (now - last_report_at?).div_euclid(DAY_SECS);
    (days >= after_days).then_some(days)
}

/// Whether a domain's reports arrive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ReportingState {
    /// No report has arrived yet.
    #[default]
    NoReports,
    /// The last report is recent.
    Reporting,
    /// Reports arrived and then stopped (see [`reports_stopped_days`]).
    Stopped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Reporting {
    pub state: ReportingState,
    pub status: Status,
}

/// Whether reports arrive at `now`, stopped after `after_days` without one
/// ([`reports_stopped_days`]). No report yet is worth fixing (amber);
/// stopped reports warn too, except for a parked domain, which may well
/// receive no mail to report on.
pub fn reporting(
    last_report_at: Option<i64>,
    now: i64,
    parked: bool,
    after_days: i64,
) -> Reporting {
    let (state, status) = if last_report_at.is_none() {
        (ReportingState::NoReports, Status::Warn)
    } else if reports_stopped_days(last_report_at, now, after_days).is_some() {
        let status = if parked {
            Status::Neutral
        } else {
            Status::Warn
        };
        (ReportingState::Stopped, status)
    } else {
        (ReportingState::Reporting, Status::Pass)
    };
    Reporting { state, status }
}

/// What a domain's verdicts are computed from.
#[derive(Debug, Clone, Copy, Default)]
pub struct DomainFacts<'a> {
    pub mail: Mail,
    /// Passed DMARC without an aligned DKIM pass.
    pub no_aligned_dkim: i64,
    pub parked: bool,
    /// Published `p=`.
    pub policy: Option<&'a str>,
    /// Stored DNS grade; `None` when not checked (no DNS issue then).
    pub dns_grade: Option<&'a str>,
    pub dns_critical_count: i64,
    pub last_report_at: Option<i64>,
}

/// Everything the UI shows about a domain's posture, decided here, beside
/// the [`MailVerdicts`](crate::MailVerdicts) of its counts. The default is
/// "not assessed": no data, neutral, no issues.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DomainAssessment {
    pub enforcement: Enforcement,
    /// Share of DMARC-passing mail signed by the domain itself.
    pub dkim_coverage: DkimCoverage,
    /// Verdict on the stored DNS grade; `neutral` without one.
    pub dns: Status,
    /// Days without reports, once that counts as stopped.
    pub reports_stopped_days: Option<i64>,
    /// The days without reports after which they count as stopped: the
    /// monitoring setting the domain was judged by.
    pub reports_stopped_after_days: i64,
    /// Whether reports arrive.
    pub reporting: Reporting,
    /// A parked domain's passing mail: `fail` when any mail passed DMARC,
    /// `pass` when none did; `neutral` for a domain that is not parked.
    pub parked_passing_mail: Status,
    /// Open issues, worst first; empty when nothing needs attention. Not
    /// serialized here: the server sends them worded (`WordedIssue`).
    #[serde(skip)]
    #[ts(skip)]
    pub issues: Vec<Issue>,
}

/// One reason a domain needs attention. The server words it from the
/// domain's own figures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    /// Failed DMARC and was still delivered, at a failing or warning rate.
    DeliveredFailures,
    /// The DNS grade fails (or a critical DNS finding).
    DnsGrade,
    /// `p=none`: spoofed mail would be delivered.
    NotEnforcing,
    /// A parked domain that does not publish `p=reject`.
    ParkedNotRejecting,
    /// Mail passed DMARC for a parked domain: something sends as it.
    ParkedPassingMail,
    /// Too little of the passing mail is signed by the domain.
    DkimCoverage,
    /// Reports arrived before and then stopped.
    ReportsStopped,
}

impl IssueKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::DeliveredFailures,
        Self::DnsGrade,
        Self::NotEnforcing,
        Self::ParkedNotRejecting,
        Self::ParkedPassingMail,
        Self::DkimCoverage,
        Self::ReportsStopped,
    ];
}

/// Ranks issues only; the colour is the issue's `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Critical,
    High,
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Issue {
    pub kind: IssueKind,
    pub severity: IssueSeverity,
    pub status: Status,
}

impl Issue {
    fn new(kind: IssueKind, severity: IssueSeverity, status: Status) -> Self {
        Self {
            kind,
            severity,
            status,
        }
    }

    /// A rated issue: critical when the rate fails, high when it warns.
    fn rated(kind: IssueKind, status: Status) -> Self {
        let severity = if status == Status::Fail {
            IssueSeverity::Critical
        } else {
            IssueSeverity::High
        };
        Self::new(kind, severity, status)
    }
}

/// A domain's verdicts at `now`.
///
/// Issues are size-blind: the same problem earns the same severity at any
/// volume (a `p=none` domain with three messages a year is still unprotected).
/// Parked domains are judged in reverse: anything short of `p=reject`, and any
/// mail that passes DMARC, is critical; DKIM coverage and stopped reports do
/// not apply to them. Reports count as stopped after `reports_stopped_after_days`
/// without one (the monitoring setting; this crate keeps none).
pub fn assess_domain(
    facts: &DomainFacts<'_>,
    now: i64,
    reports_stopped_after_days: i64,
) -> DomainAssessment {
    let mail = facts.mail;
    let delivered_compliance = mail.delivered_compliance();
    let enforcement = enforcement(facts.policy);
    let dkim_coverage = dkim_coverage(mail.compliant, facts.no_aligned_dkim);
    let dns = grade_status(facts.dns_grade, facts.dns_critical_count);
    let reports_stopped_days =
        reports_stopped_days(facts.last_report_at, now, reports_stopped_after_days);
    let parked_passing_mail = match (facts.parked, mail.compliant > 0) {
        (false, _) => Status::Neutral,
        (true, true) => Status::Fail,
        (true, false) => Status::Pass,
    };

    let mut issues = Vec::new();
    if mail.delivered_failures > 0
        && matches!(delivered_compliance.status, Status::Fail | Status::Warn)
    {
        issues.push(Issue::rated(
            IssueKind::DeliveredFailures,
            delivered_compliance.status,
        ));
    }
    if dns == Status::Fail {
        let f_grade = facts
            .dns_grade
            .is_some_and(|g| g.trim().eq_ignore_ascii_case("F"));
        let severity = if f_grade || facts.dns_critical_count > 0 {
            IssueSeverity::Critical
        } else {
            IssueSeverity::High
        };
        issues.push(Issue::new(IssueKind::DnsGrade, severity, Status::Fail));
    }
    if facts.parked {
        if enforcement.state != EnforcementState::Reject {
            issues.push(Issue::new(
                IssueKind::ParkedNotRejecting,
                IssueSeverity::Critical,
                Status::Fail,
            ));
        }
        if mail.compliant > 0 {
            issues.push(Issue::new(
                IssueKind::ParkedPassingMail,
                IssueSeverity::Critical,
                Status::Fail,
            ));
        }
    } else {
        if enforcement.state == EnforcementState::Monitor {
            issues.push(Issue::new(
                IssueKind::NotEnforcing,
                IssueSeverity::High,
                enforcement.status,
            ));
        }
        if matches!(dkim_coverage.status, Status::Fail | Status::Warn) {
            issues.push(Issue::rated(IssueKind::DkimCoverage, dkim_coverage.status));
        }
        if reports_stopped_days.is_some() {
            issues.push(Issue::new(
                IssueKind::ReportsStopped,
                IssueSeverity::High,
                Status::Warn,
            ));
        }
    }
    issues.sort_by_key(|i| i.severity);

    DomainAssessment {
        enforcement,
        dkim_coverage,
        dns,
        reports_stopped_days,
        reports_stopped_after_days,
        reporting: reporting(
            facts.last_report_at,
            now,
            facts.parked,
            reports_stopped_after_days,
        ),
        parked_passing_mail,
        issues,
    }
}

/// A domain as the "needs attention" list orders it.
#[derive(Debug, Clone, Copy)]
pub struct AttentionCandidate<'a> {
    pub assessment: &'a DomainAssessment,
    pub delivered_failures: i64,
    pub domain: &'a str,
}

/// Each domain's place in the "needs attention" list (1 = first), in input
/// order; `None` for a domain without open issues. Worst issue first, then
/// most delivered failures, then name: never by volume alone.
pub fn attention_ranks(domains: &[AttentionCandidate<'_>]) -> Vec<Option<i64>> {
    let mut open: Vec<(usize, &AttentionCandidate<'_>)> = domains
        .iter()
        .enumerate()
        .filter(|(_, d)| !d.assessment.issues.is_empty())
        .collect();
    open.sort_by(|(_, a), (_, b)| {
        a.assessment.issues[0]
            .severity
            .cmp(&b.assessment.issues[0].severity)
            .then(b.delivered_failures.cmp(&a.delivered_failures))
            .then_with(|| a.domain.to_lowercase().cmp(&b.domain.to_lowercase()))
    });
    let mut ranks = vec![None; domains.len()];
    for (rank, (i, _)) in open.into_iter().enumerate() {
        ranks[i] = Some(rank as i64 + 1);
    }
    ranks
}

/// What a sender's row for one domain says, worst first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SenderVerdictKind {
    /// Delivered failures at a failing or warning rate.
    DeliveredFailures,
    Forwarded,
    Blocked,
    /// Passed, with a few delivered failures below the line.
    MostlyPassed,
    AllPassed,
    /// Too little of the passing mail is signed by the domain.
    DkimCoverage,
    #[default]
    NoMessages,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderDomainVerdict {
    pub kind: SenderVerdictKind,
    pub status: Status,
}

/// The one verdict of a sender's mail for one domain: its outcome, unless
/// DKIM coverage is worse. Delivered failures are judged by the rate, like
/// every compliance colour: one stray failure among a thousand is not red.
pub fn sender_domain_verdict(mail: &Mail, dkim_pass: i64) -> SenderDomainVerdict {
    let delivered = mail.delivered_compliance().status;
    let rated_failures =
        mail.delivered_failures > 0 && matches!(delivered, Status::Fail | Status::Warn);
    let outcome = if rated_failures {
        SenderDomainVerdict {
            kind: SenderVerdictKind::DeliveredFailures,
            status: delivered,
        }
    } else {
        let below_line = Mail {
            delivered_failures: 0,
            total: mail.total - mail.delivered_failures,
            ..*mail
        };
        let status = below_line.outcome_status();
        let kind = match status {
            Status::Forward => SenderVerdictKind::Forwarded,
            Status::Info => SenderVerdictKind::Blocked,
            Status::Pass if mail.delivered_failures > 0 => SenderVerdictKind::MostlyPassed,
            Status::Pass => SenderVerdictKind::AllPassed,
            _ => SenderVerdictKind::NoMessages,
        };
        SenderDomainVerdict { kind, status }
    };
    let no_aligned = (mail.compliant - dkim_pass).max(0);
    let coverage = dkim_coverage(mail.compliant, no_aligned).status;
    if matches!(coverage, Status::Fail | Status::Warn)
        && outcome.status.worst(coverage) != outcome.status
    {
        SenderDomainVerdict {
            kind: SenderVerdictKind::DkimCoverage,
            status: coverage,
        }
    } else {
        outcome
    }
}

/// What one check (SPF or DKIM) says across a sender's mail for a domain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SenderCheckState {
    /// Every message passed for the domain itself.
    AllPassed,
    /// Some did not: the counts explain why.
    Mixed,
    #[default]
    NoMessages,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderCheck {
    pub state: SenderCheckState,
    pub status: Status,
}

/// SPF (or DKIM, before coverage) across `total` messages of which
/// `aligned_pass` passed for the domain. Only "all passed" is a verdict
/// (green); a failing check on failed or forwarded mail is the reason for
/// the outcome and is coloured through it, not here.
pub fn sender_check(total: i64, aligned_pass: i64) -> SenderCheck {
    let (state, status) = if total <= 0 {
        (SenderCheckState::NoMessages, Status::Neutral)
    } else if aligned_pass >= total {
        (SenderCheckState::AllPassed, Status::Pass)
    } else {
        (SenderCheckState::Mixed, Status::Neutral)
    };
    SenderCheck { state, status }
}

/// DKIM across a sender's mail: [`sender_check`], coloured by DKIM coverage
/// of the passing mail when that warns or fails.
pub fn sender_dkim_check(total: i64, aligned_pass: i64, coverage: Status) -> SenderCheck {
    let check = sender_check(total, aligned_pass);
    if check.state != SenderCheckState::AllPassed && matches!(coverage, Status::Warn | Status::Fail)
    {
        SenderCheck {
            status: coverage,
            ..check
        }
    } else {
        check
    }
}

/// The failures of a sender's mail read as spoofing: red when any was
/// delivered, blue when the policy stopped them all; neutral without failures.
/// Forwarded mail is not counted: it is never spoofing.
pub fn spoofing_status(mail: &Mail) -> Status {
    if mail.delivered_failures > 0 {
        Status::Fail
    } else if mail.blocked() > 0 {
        Status::Info
    } else {
        Status::Neutral
    }
}

/// The worst DNS grade among an organization's domains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct WorstDns {
    pub grade: String,
    pub score: i64,
    pub status: Status,
}

/// An organization's domains at a glance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct OrganizationAttention {
    /// Domains with at least one open issue.
    pub domains_needing_attention: i64,
    /// Worst status of every open issue; `neutral` when none.
    pub status: Status,
    /// Worst grade first by status, then by lowest score.
    pub worst_dns: Option<WorstDns>,
}

/// Summarize `domains` (each with its assessment, grade and score).
pub fn organization_attention<'a>(
    domains: impl IntoIterator<Item = (&'a DomainAssessment, Option<(&'a str, i64)>)>,
) -> OrganizationAttention {
    let mut out = OrganizationAttention {
        domains_needing_attention: 0,
        status: Status::Neutral,
        worst_dns: None,
    };
    for (assessment, grade) in domains {
        if !assessment.issues.is_empty() {
            out.domains_needing_attention += 1;
            out.status = Status::worst_of(
                std::iter::once(out.status).chain(assessment.issues.iter().map(|i| i.status)),
            );
        }
        let Some((grade, score)) = grade else {
            continue;
        };
        let candidate = WorstDns {
            grade: grade.to_owned(),
            score,
            status: assessment.dns,
        };
        let worse = match &out.worst_dns {
            None => true,
            Some(w) if w.status != candidate.status => {
                candidate.status.worst(w.status) == candidate.status
            }
            Some(w) => candidate.score < w.score,
        };
        if worse {
            out.worst_dns = Some(candidate);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::HealthState;

    const NOW: i64 = 1_800_000_000;
    /// The default of the monitoring setting.
    const STOPPED: i64 = 7;

    fn facts(total: i64, compliant: i64, delivered: i64) -> DomainFacts<'static> {
        DomainFacts {
            mail: Mail {
                total,
                compliant,
                delivered_failures: delivered,
                forwarded_failures: 0,
            },
            policy: Some("reject"),
            last_report_at: Some(NOW),
            ..Default::default()
        }
    }

    fn kinds(a: &DomainAssessment) -> Vec<IssueKind> {
        a.issues.iter().map(|i| i.kind).collect()
    }

    #[test]
    fn a_healthy_enforcing_domain_has_no_issues() {
        let healthy = facts(1000, 1000, 0);
        let a = assess_domain(&healthy, NOW, STOPPED);
        assert!(a.issues.is_empty());
        assert_eq!(healthy.mail.health().state, HealthState::Protected);
        assert_eq!(a.enforcement.status, Status::Pass);
    }

    #[test]
    fn delivered_failures_are_judged_by_the_rate() {
        // 99.2% delivered compliance: stray failures, not an issue.
        assert!(assess_domain(&facts(1000, 992, 8), NOW, STOPPED)
            .issues
            .is_empty());
        let warn = assess_domain(&facts(100, 97, 3), NOW, STOPPED);
        assert_eq!(warn.issues[0].kind, IssueKind::DeliveredFailures);
        assert_eq!(warn.issues[0].severity, IssueSeverity::High);
        let fail = assess_domain(&facts(3, 2, 1), NOW, STOPPED);
        assert_eq!(fail.issues[0].severity, IssueSeverity::Critical);
        assert_eq!(fail.issues[0].status, Status::Fail);
    }

    #[test]
    fn issues_are_size_blind_and_worst_first() {
        let f = DomainFacts {
            policy: Some("none"),
            no_aligned_dkim: 3,
            dns_grade: Some("F"),
            last_report_at: Some(NOW - 10 * 86_400),
            ..facts(3, 3, 0)
        };
        let a = assess_domain(&f, NOW, STOPPED);
        assert_eq!(
            kinds(&a),
            [
                IssueKind::DnsGrade,
                IssueKind::DkimCoverage,
                IssueKind::NotEnforcing,
                IssueKind::ReportsStopped
            ]
        );
        assert_eq!(a.reports_stopped_days, Some(10));
    }

    #[test]
    fn parked_domains_are_judged_in_reverse() {
        let f = DomainFacts {
            parked: true,
            policy: Some("quarantine"),
            no_aligned_dkim: 5,
            last_report_at: Some(NOW - 30 * 86_400),
            ..facts(5, 5, 0)
        };
        assert_eq!(
            kinds(&assess_domain(&f, NOW, STOPPED)),
            [IssueKind::ParkedNotRejecting, IssueKind::ParkedPassingMail]
        );
    }

    #[test]
    fn a_c_grade_warns_without_an_issue() {
        let f = DomainFacts {
            dns_grade: Some("C"),
            ..facts(10, 10, 0)
        };
        let a = assess_domain(&f, NOW, STOPPED);
        assert_eq!(a.dns, Status::Warn);
        assert!(a.issues.is_empty());
        let d = DomainFacts {
            dns_grade: Some("D"),
            ..facts(10, 10, 0)
        };
        assert_eq!(
            assess_domain(&d, NOW, STOPPED).issues[0].severity,
            IssueSeverity::High
        );
    }

    #[test]
    fn reports_stopped_after_the_set_whole_days() {
        assert_eq!(reports_stopped_days(None, NOW, 7), None);
        assert_eq!(
            reports_stopped_days(Some(NOW - 7 * 86_400 + 1), NOW, 7),
            None
        );
        assert_eq!(
            reports_stopped_days(Some(NOW - 7 * 86_400), NOW, 7),
            Some(7)
        );
        // The threshold is the setting's.
        assert_eq!(
            reports_stopped_days(Some(NOW - 2 * 86_400), NOW, 2),
            Some(2)
        );
        assert_eq!(reports_stopped_days(Some(NOW - 9 * 86_400), NOW, 10), None);
        let a = assess_domain(
            &DomainFacts {
                last_report_at: Some(NOW - 3 * 86_400),
                ..facts(10, 10, 0)
            },
            NOW,
            3,
        );
        assert_eq!(a.issues[0].kind, IssueKind::ReportsStopped);
        assert_eq!(
            (a.reports_stopped_days, a.reports_stopped_after_days),
            (Some(3), 3)
        );
    }

    fn sender(total: i64, compliant: i64, delivered: i64, forwarded: i64) -> Mail {
        Mail {
            total,
            compliant,
            delivered_failures: delivered,
            forwarded_failures: forwarded,
        }
    }

    #[test]
    fn sender_verdicts() {
        let v = |m: Mail, dkim| sender_domain_verdict(&m, dkim).kind;
        assert_eq!(
            v(sender(10, 5, 5, 0), 5),
            SenderVerdictKind::DeliveredFailures
        );
        assert_eq!(
            v(sender(1000, 999, 1, 0), 999),
            SenderVerdictKind::MostlyPassed
        );
        assert_eq!(v(sender(10, 10, 0, 0), 10), SenderVerdictKind::AllPassed);
        assert_eq!(v(sender(10, 8, 0, 2), 8), SenderVerdictKind::Forwarded);
        assert_eq!(v(sender(10, 8, 0, 0), 8), SenderVerdictKind::Blocked);
        assert_eq!(v(sender(0, 0, 0, 0), 0), SenderVerdictKind::NoMessages);
        // All passed, but on SPF alone: coverage is the worse verdict.
        let dkim = sender_domain_verdict(&sender(10, 10, 0, 0), 0);
        assert_eq!(dkim.kind, SenderVerdictKind::DkimCoverage);
        assert_eq!(dkim.status, Status::Fail);
    }

    #[test]
    fn attention_ranks_by_worst_issue_then_failures_then_name() {
        let clean = assess_domain(&facts(10, 10, 0), NOW, STOPPED);
        let high = assess_domain(
            &DomainFacts {
                policy: Some("none"),
                ..facts(10, 10, 0)
            },
            NOW,
            STOPPED,
        );
        let critical = assess_domain(&facts(3, 2, 1), NOW, STOPPED);
        let c = |assessment, delivered_failures, domain| AttentionCandidate {
            assessment,
            delivered_failures,
            domain,
        };
        let ranks = attention_ranks(&[
            c(&clean, 0, "a.example"),
            c(&high, 0, "B.example"),
            c(&high, 0, "a.example"),
            c(&high, 5, "z.example"),
            c(&critical, 1, "y.example"),
        ]);
        assert_eq!(ranks, [None, Some(4), Some(3), Some(2), Some(1)]);
    }

    #[test]
    fn reporting_states() {
        assert_eq!(
            reporting(None, NOW, false, STOPPED).state,
            ReportingState::NoReports
        );
        assert_eq!(reporting(None, NOW, true, STOPPED).status, Status::Warn);
        let fresh = reporting(Some(NOW - 86_400), NOW, false, STOPPED);
        assert_eq!(
            (fresh.state, fresh.status),
            (ReportingState::Reporting, Status::Pass)
        );
        let stopped = Some(NOW - 7 * 86_400);
        assert_eq!(
            reporting(stopped, NOW, false, STOPPED).state,
            ReportingState::Stopped
        );
        assert_eq!(reporting(stopped, NOW, false, STOPPED).status, Status::Warn);
        assert_eq!(
            reporting(stopped, NOW, true, STOPPED).status,
            Status::Neutral
        );
        let a = assess_domain(&facts(1, 1, 0), NOW, STOPPED);
        assert_eq!(a.reporting.state, ReportingState::Reporting);
    }

    #[test]
    fn parked_passing_mail_status() {
        assert_eq!(
            assess_domain(&facts(5, 5, 0), NOW, STOPPED).parked_passing_mail,
            Status::Neutral
        );
        let parked = |compliant| DomainFacts {
            parked: true,
            ..facts(5, compliant, 0)
        };
        assert_eq!(
            assess_domain(&parked(5), NOW, STOPPED).parked_passing_mail,
            Status::Fail
        );
        assert_eq!(
            assess_domain(&parked(0), NOW, STOPPED).parked_passing_mail,
            Status::Pass
        );
    }

    #[test]
    fn sender_checks() {
        assert_eq!(sender_check(0, 0).state, SenderCheckState::NoMessages);
        assert_eq!(sender_check(10, 10).status, Status::Pass);
        assert_eq!(sender_check(10, 9).state, SenderCheckState::Mixed);
        assert_eq!(sender_check(10, 9).status, Status::Neutral);
        assert_eq!(sender_dkim_check(10, 2, Status::Fail).status, Status::Fail);
        assert_eq!(
            sender_dkim_check(10, 9, Status::Pass).status,
            Status::Neutral
        );
        assert_eq!(sender_dkim_check(10, 10, Status::Fail).status, Status::Pass);
    }

    #[test]
    fn spoofing_reads_red_only_when_it_was_delivered() {
        assert_eq!(spoofing_status(&sender(10, 5, 1, 0)), Status::Fail);
        assert_eq!(spoofing_status(&sender(10, 5, 0, 0)), Status::Info);
        assert_eq!(spoofing_status(&sender(10, 8, 0, 2)), Status::Neutral);
    }

    #[test]
    fn organization_attention_counts_domains_and_finds_the_worst_grade() {
        let ok = assess_domain(&facts(10, 10, 0), NOW, STOPPED);
        let bad = assess_domain(
            &DomainFacts {
                policy: Some("none"),
                dns_grade: Some("B"),
                ..facts(10, 10, 0)
            },
            NOW,
            STOPPED,
        );
        let c = organization_attention([(&ok, Some(("A", 95))), (&bad, Some(("B", 85)))]);
        assert_eq!(c.domains_needing_attention, 1);
        assert_eq!(c.status, Status::Fail);
        let worst = c.worst_dns.unwrap();
        assert_eq!((worst.grade.as_str(), worst.score), ("B", 85));
    }
}
