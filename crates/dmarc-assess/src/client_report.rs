//! What a client report says, decided before it is worded: the
//! organization's verdict and its colour ([`report_verdict`]), which domains
//! get a block of their own and what the block holds ([`report_block`]), and
//! the line over the parked domains ([`parked_line`]). The server's renderer
//! (`mail::client_report`) words and lays out what is decided here and
//! decides nothing itself.

use crate::domain::{organization_attention, DomainAssessment, IssueKind};
use crate::readiness::{PolicyReadiness, PolicyStep};
use crate::status::Status;

/// One domain as its client report judges it.
#[derive(Debug, Clone, Copy)]
pub struct ReportDomainFacts<'a> {
    /// Its figures and issues over the readiness window before sending.
    pub assessment: &'a DomainAssessment,
    pub readiness: &'a PolicyReadiness,
    pub parked: bool,
    /// A stored DNS grade from the current checker: an outdated one is
    /// being redone and is left out, as is a domain not checked yet.
    pub dns_current: bool,
    /// Its changes (alert events) in the report's month.
    pub changes: usize,
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

/// A domain's block: the lines it holds. Every field is a line or `None`
/// (`false`) when the line is left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportBlock {
    /// The open issues it names, as the assessment orders them (worst
    /// first); a parked domain without `p=reject` is said once, on the
    /// parked line, not here.
    pub issues: Vec<IssueKind>,
    /// The colour of the issues' line: the worst of them.
    pub issues_status: Status,
    /// Delivered compliance, the figure behind delivered failures: only
    /// with that issue.
    pub compliance: bool,
    /// The next policy step: where the window's rules decide it (ready, not
    /// ready, raise pct first).
    pub next_step: bool,
    /// What moving to this step would act on: the window's failing mail and
    /// its senders, ready or not; never while `pct` is to be raised first.
    pub impact: Option<PolicyStep>,
    /// The DNS grade's line and colour: a current grade that does not pass.
    pub dns: Option<Status>,
    /// The month's changes.
    pub changes: bool,
}

/// Where a domain is said in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportPlace {
    /// A block of its own: an open issue, a step to take now (ready, or
    /// raise pct) or a change in the month.
    Block(ReportBlock),
    /// Named on the line of protected and unchanged domains.
    Quiet,
    /// A quiet parked domain: counted on the parked line only.
    ParkedOnly,
}

/// Where `d` is said, and with a block, what its block holds.
pub fn report_place(d: &ReportDomainFacts<'_>) -> ReportPlace {
    let issues: Vec<IssueKind> = d
        .assessment
        .issues
        .iter()
        .map(|i| i.kind)
        .filter(|&kind| kind != IssueKind::ParkedNotRejecting)
        .collect();
    if issues.is_empty() && !d.readiness.step_pending() && d.changes == 0 {
        return if d.parked {
            ReportPlace::ParkedOnly
        } else {
            ReportPlace::Quiet
        };
    }
    let issues_status = Status::worst_of(
        d.assessment
            .issues
            .iter()
            .filter(|i| issues.contains(&i.kind))
            .map(|i| i.status),
    );
    ReportPlace::Block(ReportBlock {
        compliance: issues.contains(&IssueKind::DeliveredFailures),
        issues,
        issues_status,
        next_step: d.readiness.rules_decide(),
        impact: d
            .readiness
            .next_step()
            .filter(|_| d.readiness.window.messages > 0),
        dns: (d.dns_current && d.assessment.dns != Status::Pass).then_some(d.assessment.dns),
        changes: d.changes > 0,
    })
}

/// The block of `d`; `None` when it has none.
pub fn report_block(d: &ReportDomainFacts<'_>) -> Option<ReportBlock> {
    match report_place(d) {
        ReportPlace::Block(block) => Some(block),
        ReportPlace::Quiet | ReportPlace::ParkedOnly => None,
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
        let mail = Mail {
            total,
            compliant: total - failures,
            delivered_failures: failures,
            forwarded_failures: 0,
        };
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
                parked,
                policy_messages: total,
                policy_first_report_at: Some(NOW - 60 * DAY),
                other_policy: None,
            },
            NOW,
        );
        (assessment, readiness)
    }

    fn facts(
        d: &(DomainAssessment, PolicyReadiness),
        parked: bool,
        changes: usize,
    ) -> ReportDomainFacts<'_> {
        ReportDomainFacts {
            assessment: &d.0,
            readiness: &d.1,
            parked,
            dns_current: true,
            changes,
        }
    }

    #[test]
    fn the_verdict_counts_domains_needing_attention_and_an_all_clear_is_green() {
        let good = domain("reject", 1000, 0, false);
        let bad = domain("none", 1000, 60, false);
        assert_eq!(
            report_verdict(&[facts(&good, false, 0)]),
            ReportVerdict {
                domains: 1,
                needing_attention: 0,
                status: Status::Pass
            }
        );
        let verdict = report_verdict(&[facts(&good, false, 0), facts(&bad, false, 0)]);
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

    #[test]
    fn a_domain_gets_a_block_for_an_issue_a_step_to_take_or_a_change() {
        let enforced = domain("reject", 1000, 0, false);
        assert_eq!(
            report_place(&facts(&enforced, false, 0)),
            ReportPlace::Quiet
        );
        // A change in the month gives it a block with only that line.
        let changed = report_block(&facts(&enforced, false, 2)).expect("a block");
        assert_eq!(
            changed,
            ReportBlock {
                issues: vec![],
                issues_status: Status::Neutral,
                compliance: false,
                next_step: false,
                impact: None,
                dns: None,
                changes: true,
            }
        );
        // p=none, all mail passing: ready for quarantine, a step to take now.
        let ready = domain("none", 1000, 0, false);
        let block = report_block(&facts(&ready, false, 0)).expect("a block");
        assert!(block.next_step);
        assert_eq!(block.impact, Some(PolicyStep::Quarantine));
        assert_eq!(block.issues, [IssueKind::NotEnforcing]);
        // Delivered failures: their figure, and a step not ready yet.
        let failing = domain("quarantine", 1000, 60, false);
        let block = report_block(&facts(&failing, false, 0)).expect("a block");
        assert!(block.compliance);
        assert!(block.next_step);
        assert_eq!(block.impact, Some(PolicyStep::Reject));
        assert_eq!(block.issues_status, Status::Fail);
    }

    #[test]
    fn a_grade_shows_only_when_current_and_not_passing() {
        let mut failing = domain("reject", 1000, 0, false);
        failing.0.dns = Status::Fail;
        let mut f = facts(&failing, false, 1);
        assert_eq!(report_block(&f).unwrap().dns, Some(Status::Fail));
        f.dns_current = false;
        assert_eq!(report_block(&f).unwrap().dns, None);
        let passing = domain("reject", 1000, 0, false);
        assert_eq!(report_block(&facts(&passing, false, 1)).unwrap().dns, None);
    }

    #[test]
    fn parked_domains_are_said_on_their_own_line() {
        let rejecting = domain("reject", 0, 0, true);
        let lax = domain("none", 0, 0, true);
        let other = domain("reject", 1000, 0, false);
        let domains = [
            facts(&other, false, 0),
            facts(&rejecting, true, 0),
            facts(&lax, true, 0),
        ];
        // Not rejecting is said on the parked line only; quiet parked
        // domains have no block and are not on the protected line.
        assert_eq!(report_place(&domains[1]), ReportPlace::ParkedOnly);
        assert_eq!(report_place(&domains[2]), ReportPlace::ParkedOnly);
        let line = parked_line(&domains).expect("a parked line");
        assert_eq!(line.parked, 2);
        assert_eq!(line.not_rejecting, [2]);
        assert_eq!(line.status, lax.0.issues[0].status);
        let all = parked_line(&domains[..2]).expect("a parked line");
        assert_eq!((all.not_rejecting.len(), all.status), (0, Status::Neutral));
        assert_eq!(parked_line(&domains[..1]), None);
    }
}
