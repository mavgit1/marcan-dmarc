//! A domain's open issues in words, once for the emails and the UI: each
//! issue's name ([`issue_name`], "delivered failures"), what it means
//! ([`issue_meaning`], the attention list's tooltip and the docs' table of
//! issues), and, from the domain's own figures, its badge ([`issue_label`],
//! "3 delivered failures") and the scorecard's finding ([`issue_finding`]).
//! Which issues are open, how severe and in which colour is `dmarc_assess`'s
//! call; the domain views send the words beside it (`WordedIssue`), the
//! catalogue goes out with `GET /api/meta`.

use super::{count_noun, format_pct, one_or_many};
use dmarc_assess::{CoverageState, DkimCoverage, EnforcementState, IssueKind, Rate};
use dmarc_store::{PolicySource, DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS};

/// An open issue named without its figures: the alert sentences, the digest
/// and the client report say it so.
pub fn issue_name(issue: IssueKind) -> &'static str {
    match issue {
        IssueKind::DeliveredFailures => "delivered failures",
        IssueKind::DnsGrade => "failing DNS grade",
        IssueKind::NotEnforcing => "not enforcing (p=none)",
        IssueKind::ParkedNotRejecting => "parked domain without p=reject",
        IssueKind::ParkedPassingMail => "mail passing DMARC for a parked domain",
        IssueKind::DkimCoverage => "passing mail not signed with DKIM",
        IssueKind::ReportsStopped => "reports stopped",
    }
}

/// When the issue is raised, in one sentence.
pub fn issue_meaning(issue: IssueKind) -> String {
    match issue {
        IssueKind::DeliveredFailures => {
            "Mail failed DMARC and was still delivered, at a warning or failing rate.".to_owned()
        }
        IssueKind::DnsGrade => "The DNS grade fails, or there is a critical DNS finding.".to_owned(),
        IssueKind::NotEnforcing => {
            "The policy is p=none: spoofed mail would be delivered.".to_owned()
        }
        IssueKind::ParkedNotRejecting => "A parked domain does not publish p=reject.".to_owned(),
        IssueKind::ParkedPassingMail => {
            "Mail passed DMARC for a parked domain: something sends as it.".to_owned()
        }
        IssueKind::DkimCoverage => {
            "Too little of the passing mail is signed by the domain itself.".to_owned()
        }
        IssueKind::ReportsStopped => format!(
            "Reports arrived before and then stopped: none for the days of the domain warning (Settings → Mailbox, {DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS} by default)."
        ),
    }
}

/// The figures a domain's issues are worded from (a domain list row or a
/// domain overview).
#[derive(Debug, Clone, Copy)]
pub struct IssueFigures<'a> {
    pub domain: &'a str,
    pub delivered_failures: i64,
    pub compliant_messages: i64,
    pub no_aligned_dkim_messages: i64,
    pub delivered_compliance: Rate,
    /// The published `p=`, as found in DNS.
    pub policy: Option<&'a str>,
    /// Why `policy` is `None`.
    pub policy_source: PolicySource,
    pub enforcement: EnforcementState,
    pub dns_grade: Option<&'a str>,
    pub dkim_coverage: DkimCoverage,
    pub reports_stopped_days: Option<i64>,
    pub reports_stopped_after_days: i64,
}

/// The policy as a badge: `p=none`, the published value Marcan does not
/// know, or, without a record, the state shown in its place everywhere
/// ("No record", "Not checked yet", "Check failed").
fn policy_label(f: &IssueFigures<'_>) -> String {
    let Some(p) = f.policy else {
        return match f.policy_source {
            PolicySource::NotChecked => "Not checked yet",
            PolicySource::CheckFailed => "Check failed",
            PolicySource::Dns | PolicySource::NotPublished => "No record",
        }
        .to_owned();
    };
    match f.enforcement {
        EnforcementState::Reject => "p=reject".to_owned(),
        EnforcementState::Quarantine => "p=quarantine".to_owned(),
        EnforcementState::Monitor => "p=none".to_owned(),
        EnforcementState::Unknown if p.trim().is_empty() => "Unknown".to_owned(),
        EnforcementState::Unknown => p.to_owned(),
    }
}

/// `—` for a rate nothing was measured for.
fn rate(r: Rate) -> String {
    r.pct.map_or_else(|| "—".to_owned(), format_pct)
}

/// The badge of an open issue: `3 delivered failures`, `DNS grade C`,
/// `p=none`, `5.0% without DKIM`, `No reports for 9 days`.
pub fn issue_label(issue: IssueKind, f: &IssueFigures<'_>) -> String {
    match issue {
        IssueKind::DeliveredFailures => count_noun(f.delivered_failures, "delivered failure"),
        IssueKind::DnsGrade => format!("DNS grade {}", f.dns_grade.unwrap_or("?")),
        IssueKind::NotEnforcing | IssueKind::ParkedNotRejecting => policy_label(f),
        IssueKind::ParkedPassingMail => count_noun(f.compliant_messages, "passing message"),
        IssueKind::DkimCoverage => match (f.dkim_coverage.state, f.dkim_coverage.pct) {
            (CoverageState::None, _) | (_, None) => "No DKIM".to_owned(),
            (_, Some(pct)) => format!("{} without DKIM", format_pct(100.0 - pct)),
        },
        IssueKind::ReportsStopped => format!(
            "No reports for {}",
            count_noun(f.reports_stopped_days.unwrap_or(0), "day")
        ),
    }
}

/// The issue as a finding of the scorecard's report data, with its figures;
/// `None` for the DNS and enforcement issues, which are the DNS cards' own
/// findings.
pub fn issue_finding(issue: IssueKind, f: &IssueFigures<'_>) -> Option<String> {
    match issue {
        IssueKind::ParkedPassingMail => Some(format!(
            "{} passed DMARC for a parked domain: something is authorized to send as it.",
            count_noun(f.compliant_messages, "message")
        )),
        IssueKind::DeliveredFailures => Some(format!(
            "Delivered compliance is {}: {} failed DMARC and {} still delivered.",
            rate(f.delivered_compliance),
            count_noun(f.delivered_failures, "message"),
            one_or_many(f.delivered_failures, "was", "were")
        )),
        IssueKind::DkimCoverage => Some(format!(
            "{} of the mail that passed DMARC is signed by {}; {} passed on SPF alone and fail when forwarded.",
            rate(Rate {
                pct: f.dkim_coverage.pct,
                status: f.dkim_coverage.status
            }),
            f.domain,
            count_noun(f.no_aligned_dkim_messages, "message")
        )),
        IssueKind::ReportsStopped => Some(format!(
            "No reports for {} (threshold {}).",
            count_noun(f.reports_stopped_days.unwrap_or(0), "day"),
            count_noun(f.reports_stopped_after_days, "day")
        )),
        IssueKind::DnsGrade | IssueKind::NotEnforcing | IssueKind::ParkedNotRejecting => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_assess::Status;

    fn figures() -> IssueFigures<'static> {
        IssueFigures {
            domain: "example.com",
            delivered_failures: 3,
            compliant_messages: 1_000,
            no_aligned_dkim_messages: 50,
            delivered_compliance: Rate {
                pct: Some(99.7),
                status: Status::Pass,
            },
            policy: Some("none"),
            policy_source: PolicySource::Dns,
            enforcement: EnforcementState::Monitor,
            dns_grade: Some("C"),
            dkim_coverage: DkimCoverage {
                pct: Some(95.0),
                status: Status::Warn,
                state: CoverageState::Partial,
            },
            reports_stopped_days: Some(9),
            reports_stopped_after_days: 7,
        }
    }

    #[test]
    fn badges_carry_the_domains_figures() {
        let f = figures();
        let label = |kind| issue_label(kind, &f);
        assert_eq!(label(IssueKind::DeliveredFailures), "3 delivered failures");
        assert_eq!(label(IssueKind::DnsGrade), "DNS grade C");
        assert_eq!(label(IssueKind::NotEnforcing), "p=none");
        assert_eq!(
            label(IssueKind::ParkedPassingMail),
            "1,000 passing messages"
        );
        assert_eq!(label(IssueKind::DkimCoverage), "5.0% without DKIM");
        assert_eq!(label(IssueKind::ReportsStopped), "No reports for 9 days");
        let unsigned = IssueFigures {
            dkim_coverage: DkimCoverage {
                pct: Some(0.0),
                status: Status::Fail,
                state: CoverageState::None,
            },
            ..f
        };
        assert_eq!(issue_label(IssueKind::DkimCoverage, &unsigned), "No DKIM");
    }

    #[test]
    fn a_missing_policy_reads_as_its_state() {
        let missing = |source| IssueFigures {
            policy: None,
            policy_source: source,
            enforcement: EnforcementState::Unknown,
            ..figures()
        };
        let label = |source| issue_label(IssueKind::ParkedNotRejecting, &missing(source));
        assert_eq!(label(PolicySource::NotPublished), "No record");
        assert_eq!(label(PolicySource::NotChecked), "Not checked yet");
        assert_eq!(label(PolicySource::CheckFailed), "Check failed");
        let odd = IssueFigures {
            policy: Some("monitor"),
            enforcement: EnforcementState::Unknown,
            ..figures()
        };
        assert_eq!(issue_label(IssueKind::ParkedNotRejecting, &odd), "monitor");
        let quarantine = IssueFigures {
            policy: Some("quarantine"),
            enforcement: EnforcementState::Quarantine,
            ..figures()
        };
        assert_eq!(
            issue_label(IssueKind::ParkedNotRejecting, &quarantine),
            "p=quarantine"
        );
    }

    #[test]
    fn report_issues_are_findings_and_dns_ones_are_not() {
        let f = figures();
        assert_eq!(
            issue_finding(IssueKind::DeliveredFailures, &f).unwrap(),
            "Delivered compliance is 99.7%: 3 messages failed DMARC and were still delivered."
        );
        assert_eq!(
            issue_finding(IssueKind::DkimCoverage, &f).unwrap(),
            "95.0% of the mail that passed DMARC is signed by example.com; 50 messages passed on SPF alone and fail when forwarded."
        );
        assert_eq!(
            issue_finding(IssueKind::ReportsStopped, &f).unwrap(),
            "No reports for 9 days (threshold 7 days)."
        );
        let one = IssueFigures {
            compliant_messages: 1,
            ..f
        };
        assert_eq!(
            issue_finding(IssueKind::ParkedPassingMail, &one).unwrap(),
            "1 message passed DMARC for a parked domain: something is authorized to send as it."
        );
        for kind in [
            IssueKind::DnsGrade,
            IssueKind::NotEnforcing,
            IssueKind::ParkedNotRejecting,
        ] {
            assert_eq!(issue_finding(kind, &f), None);
        }
    }

    /// The docs' table of issues (`docs/dashboard.md`, "The attention list")
    /// is these words: a changed meaning fails here until the docs follow.
    #[test]
    fn the_docs_list_every_issue_as_worded_here() {
        let docs = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/dashboard.md"
        ))
        .expect("docs/dashboard.md");
        let rows: Vec<(String, String)> = docs
            .lines()
            .skip_while(|l| !l.starts_with("| Issue | When it is raised |"))
            .skip(2)
            .take_while(|l| l.starts_with('|'))
            .map(|l| {
                let cells: Vec<&str> = l.trim_matches('|').split(" | ").collect();
                let plain = |s: &str| s.trim().replace(['`'], "").replace("**", "");
                (plain(cells[0]), plain(cells[1]))
            })
            .collect();
        let expected: Vec<(String, String)> = IssueKind::ALL
            .into_iter()
            .map(|kind| {
                let id = serde_json::to_value(kind).unwrap();
                (id.as_str().unwrap().to_owned(), issue_meaning(kind))
            })
            .collect();
        assert_eq!(rows, expected, "docs/dashboard.md's issue table");
    }
}
