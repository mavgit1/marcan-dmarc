//! Domains: the list row, the overview and the Failures tab, with their verdicts.
//! Each wire type embeds the store row (flattened) and adds what is judged.

use super::readiness::Readiness;
use crate::snapshots::dns::{dns_outdated, DnsService};
use crate::wording::issues::{issue_finding, issue_label, policy_label, IssueFigures};
use dmarc_assess::{
    assess_domain, attention_ranks, policy_readiness, AttentionCandidate, CheckState,
    DomainAssessment, DomainFacts, Issue, MailVerdicts, PolicyReadiness, ReadinessFacts, Status,
};
use dmarc_dns::dmarc_publish::new_record;
use dmarc_domain::organizational_domain;
use dmarc_store::{
    DkimCheckRow, DomainOverviewRow, DomainSummaryRow, FailureRow, MailCounts, MonitoringSettings,
    OverrideReason, PolicySource, PublishedPolicy, ReadinessMailRow, RecordChecks, SpfCheckRow,
    StoredDnsGrade, DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS,
};
use serde::Serialize;
use std::ops::Deref;
use ts_rs::TS;

/// When and by which setting domains are judged: at `now`, their reports
/// stopped after `reports_stopped_after_days` without one (the monitoring
/// setting `domain_reports_stopped_days`, passed to `dmarc_assess`, which
/// keeps no setting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judging {
    pub now: i64,
    pub reports_stopped_after_days: i64,
}

impl Judging {
    /// At `now`, by the stored monitoring settings.
    pub fn new(now: i64, monitoring: &MonitoringSettings) -> Self {
        Self {
            now,
            reports_stopped_after_days: i64::from(monitoring.domain_reports_stopped_days),
        }
    }

    /// At `now`, by the settings stored in `pool`.
    pub async fn load(pool: &sqlx::SqlitePool, now: i64) -> Result<Self, dmarc_store::StoreError> {
        let settings = dmarc_store::AppSettings::load(pool).await?;
        Ok(Self::new(now, &settings.monitoring))
    }

    /// At `now`, by the settings' defaults.
    pub fn defaults_at(now: i64) -> Self {
        Self {
            now,
            reports_stopped_after_days: i64::from(DEFAULT_DOMAIN_REPORTS_STOPPED_DAYS),
        }
    }
}

/// [`organizational_domain`], falling back to the name itself.
fn org_domain_of(domain: &str) -> String {
    organizational_domain(domain).unwrap_or_else(|| domain.to_owned())
}

/// One open issue as served: `dmarc_assess`'s verdict with its words from
/// the domain's figures (`wording::issues`). Its name and meaning are the
/// same for every domain: `GET /api/meta`'s `issue_catalogue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct WordedIssue {
    #[serde(flatten)]
    #[ts(flatten)]
    pub issue: Issue,
    /// The badge: `3 delivered failures`, `DNS grade C`, `p=none`.
    pub label: String,
    /// The scorecard's report-data finding; `null` for the DNS and
    /// enforcement issues, which are the DNS cards' own findings.
    pub finding: Option<String>,
}

/// A domain's assessment as served: `dmarc_assess`'s verdicts, and its open
/// issues worded. Reads as the [`DomainAssessment`] it wraps.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AssessmentView {
    #[serde(flatten)]
    #[ts(flatten)]
    pub verdicts: DomainAssessment,
    /// Open issues, worst first, as `verdicts.issues` lists them.
    #[serde(rename = "issues")]
    #[ts(rename = "issues")]
    pub worded_issues: Vec<WordedIssue>,
}

impl Deref for AssessmentView {
    type Target = DomainAssessment;

    fn deref(&self) -> &DomainAssessment {
        &self.verdicts
    }
}

/// `assessment`'s open issues worded from `figures`.
fn worded_issues(assessment: &DomainAssessment, figures: &IssueFigures<'_>) -> Vec<WordedIssue> {
    assessment
        .issues
        .iter()
        .map(|&issue| WordedIssue {
            issue,
            label: issue_label(issue.kind, figures),
            finding: issue_finding(issue.kind, figures),
        })
        .collect()
}

/// What a domain's issues are worded from: the figures a list row and an
/// overview both have.
#[allow(clippy::too_many_arguments)]
fn issue_figures<'a>(
    domain: &'a str,
    mail: &MailCounts,
    no_aligned_dkim_messages: i64,
    policy: Option<&'a PublishedPolicy>,
    policy_source: PolicySource,
    dns: Option<&'a StoredDnsGrade>,
    verdicts: &MailVerdicts,
    assessment: &DomainAssessment,
) -> IssueFigures<'a> {
    IssueFigures {
        domain,
        delivered_failures: mail.delivered_failures,
        compliant_messages: mail.compliant_messages,
        no_aligned_dkim_messages,
        delivered_compliance: verdicts.delivered_compliance,
        policy: policy_p(policy),
        policy_source,
        enforcement: assessment.enforcement.state,
        dns_grade: dns.map(|d| d.grade.as_str()),
        dkim_coverage: assessment.dkim_coverage,
        reports_stopped_days: assessment.reports_stopped_days,
        reports_stopped_after_days: assessment.reports_stopped_after_days,
    }
}

fn summary_figures<'a>(
    row: &'a DomainSummaryRow,
    verdicts: &MailVerdicts,
    assessment: &DomainAssessment,
) -> IssueFigures<'a> {
    issue_figures(
        &row.domain,
        &row.mail,
        row.no_aligned_dkim_messages,
        row.policy.as_ref(),
        row.policy_source,
        row.dns.as_ref(),
        verdicts,
        assessment,
    )
}

fn overview_figures<'a>(
    row: &'a DomainOverviewRow,
    verdicts: &MailVerdicts,
    assessment: &DomainAssessment,
) -> IssueFigures<'a> {
    issue_figures(
        &row.domain,
        &row.mail,
        row.no_aligned_dkim_messages,
        row.policy.as_ref(),
        row.policy_source,
        row.dns.as_ref(),
        verdicts,
        assessment,
    )
}

/// One row of `GET /api/domains`: the stored figures plus the verdicts.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DomainSummary {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: DomainSummaryRow,
    /// Blocked count, delivered compliance and health of the mail within
    /// `since`, beside its counts as on every response.
    #[serde(flatten)]
    #[ts(flatten)]
    pub verdicts: MailVerdicts,
    /// Organizational domain (PSL registrable domain) of `domain`, or `domain`
    /// itself when none can be derived. RFC 7489 §7.1 authorization is only
    /// needed for a `rua` address under a different organizational domain.
    pub org_domain: String,
    /// The DNS grade comes from an older checker: shown until the re-check
    /// (already queued first) replaces it.
    pub dns_updating: bool,
    /// No DNS result is stored and this process's latest check of the domain
    /// failed: "Check failed", not "Not checked yet". Set by
    /// [`note_failed_checks`]; `false` from [`domain_summaries`] alone.
    pub dns_check_failed: bool,
    /// Verdicts and open issues (see `dmarc_assess::assess_domain`), the
    /// issues worded.
    pub assessment: AssessmentView,
    /// Place in the "needs attention" list among the listed domains (1 =
    /// first); `null` without open issues (see `dmarc_assess::attention_ranks`),
    /// and on every row of a page of `GET /api/domains`: a rank is a place in
    /// the whole scope (`GET /api/domains/attention`), not in one page.
    pub attention_rank: Option<i64>,
}

impl DomainSummary {
    /// The policy as its badge reads: `p=none`, or the state shown in its
    /// place without a record ("No record", "Not checked yet").
    pub fn policy_label(&self) -> String {
        policy_label(&summary_figures(
            &self.row,
            &self.verdicts,
            &self.assessment,
        ))
    }
}

/// The `p=` of a stored policy.
fn policy_p(policy: Option<&PublishedPolicy>) -> Option<&str> {
    policy.and_then(|p| p.p.as_deref())
}

fn summary_facts(row: &DomainSummaryRow) -> DomainFacts<'_> {
    DomainFacts {
        mail: row.mail.mail(),
        no_aligned_dkim: row.no_aligned_dkim_messages,
        parked: row.parked,
        policy: policy_p(row.policy.as_ref()),
        dns_grade: row.dns.as_ref().map(|d| d.grade.as_str()),
        dns_critical_count: row.dns.as_ref().map_or(0, |d| d.critical_count),
        last_report_at: row.last_report_at,
    }
}

/// One domain judged `at`, without an attention rank (a rank is a place
/// among other domains). Its policy is the one the store picked (see
/// [`PolicySource`]): the DMARC record of its stored DNS check, none without
/// one. It is shown and judged alike.
pub fn judged_summary(row: DomainSummaryRow, at: Judging) -> DomainSummary {
    let verdicts = row.mail.mail().verdicts();
    let assessment = assess_domain(&summary_facts(&row), at.now, at.reports_stopped_after_days);
    let worded_issues = worded_issues(&assessment, &summary_figures(&row, &verdicts, &assessment));
    DomainSummary {
        assessment: AssessmentView {
            verdicts: assessment,
            worded_issues,
        },
        verdicts,
        org_domain: org_domain_of(&row.domain),
        dns_updating: row
            .dns
            .as_ref()
            .is_some_and(|d| dns_outdated(d.checker_version)),
        dns_check_failed: false,
        attention_rank: None,
        row,
    }
}

/// The listed domains judged `at` ([`judged_summary`]), ranked for the
/// "needs attention" list among themselves.
pub fn domain_summaries(rows: Vec<DomainSummaryRow>, at: Judging) -> Vec<DomainSummary> {
    let mut items: Vec<DomainSummary> = rows
        .into_iter()
        .map(|row| judged_summary(row, at))
        .collect();
    let candidates: Vec<AttentionCandidate<'_>> = items
        .iter()
        .map(|d| AttentionCandidate {
            assessment: &d.assessment,
            delivered_failures: d.row.mail.delivered_failures,
            domain: &d.row.domain,
        })
        .collect();
    let ranks = attention_ranks(&candidates);
    for (item, rank) in items.iter_mut().zip(ranks) {
        item.attention_rank = rank;
    }
    items
}

/// The domains with open issues, in the attention list's order (the ranks of
/// [`domain_summaries`]): the dashboard's list and the digest's.
pub fn needing_attention(domains: &[DomainSummary]) -> Vec<&DomainSummary> {
    let mut open: Vec<&DomainSummary> = domains
        .iter()
        .filter(|d| !d.assessment.issues.is_empty())
        .collect();
    open.sort_by_key(|d| d.attention_rank.unwrap_or(i64::MAX));
    open
}

/// A policy not checked yet whose latest DNS check failed reads "Check failed".
pub fn note_failed_policy_check(source: &mut PolicySource, check_failed: bool) {
    if check_failed && *source == PolicySource::NotChecked {
        *source = PolicySource::CheckFailed;
    }
}

/// Mark the domains without a stored DNS result whose latest check failed
/// (see [`DomainSummary::dns_check_failed`]).
pub fn note_failed_checks(items: &mut [DomainSummary], dns: &DnsService) {
    for item in items {
        item.dns_check_failed = item.row.dns.is_none() && dns.failing(&item.row.domain);
        if item.dns_check_failed && item.row.policy_source == PolicySource::NotChecked {
            note_failed_policy_check(&mut item.row.policy_source, true);
            let a = &item.assessment.verdicts;
            item.assessment.worded_issues =
                worded_issues(a, &summary_figures(&item.row, &item.verdicts, a));
        }
    }
}

/// Policy readiness at `now` of a domain whose DNS policy is `policy` (its
/// `p=` and `pct=`), from `mail`, its [`readiness_mail`](dmarc_store::readiness_mail)
/// over the readiness window (`READINESS_WINDOW_DAYS` before `now`) split by
/// that policy.
fn judged_readiness(
    mail: &ReadinessMailRow,
    policy: Option<&PublishedPolicy>,
    parked: bool,
    now: i64,
) -> PolicyReadiness {
    policy_readiness(
        &ReadinessFacts {
            window_mail: mail.mail.mail(),
            policy: policy_p(policy),
            pct: policy.and_then(|p| p.pct),
            testing: policy.and_then(|p| p.testing).unwrap_or(false),
            parked,
            policy_messages: mail.policy_messages,
            policy_first_report_at: mail.policy_first_report_at,
            other_policy: mail.other_policy.as_deref(),
        },
        now,
    )
}

/// Policy readiness of a domain list row, judged like the overview's, from
/// its `mail` over the readiness window split by `row.policy`'s `p=`.
pub fn summary_readiness(
    row: &DomainSummaryRow,
    mail: &ReadinessMailRow,
    now: i64,
) -> PolicyReadiness {
    judged_readiness(mail, row.policy.as_ref(), row.parked, now)
}

/// One domain judged on its own (its attention rank is 1 or none).
pub fn domain_summary(row: DomainSummaryRow, at: Judging) -> DomainSummary {
    domain_summaries(vec![row], at)
        .pop()
        .expect("one row in, one out")
}

/// `GET /api/domains/{id}`: identity, published policy and figures within `since`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DomainOverview {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: DomainOverviewRow,
    /// As on the domain list ([`DomainSummary::verdicts`]).
    #[serde(flatten)]
    #[ts(flatten)]
    pub verdicts: MailVerdicts,
    /// Organizational domain of `domain` (see [`DomainSummary::org_domain`]).
    pub org_domain: String,
    /// Verdicts and open issues, as on the domain list (the stored DNS grade included).
    pub assessment: AssessmentView,
    /// Whether the domain is ready for its next policy step, judged on the
    /// last `READINESS_WINDOW_DAYS` days whatever `since` is (see
    /// `dmarc_assess::policy_readiness`), with its words. Not on the domain
    /// list: it costs a query per domain.
    pub policy_readiness: Readiness,
    /// Messages that failed DMARC within `since`, every outcome: delivered
    /// failures, forwarded and blocked (the Failures tab's opening line).
    /// Forwarded mail is in it, unlike a bare "failed" count, hence the name.
    pub dmarc_failed_incl_forwarded: i64,
}

impl DomainOverview {
    /// A policy not checked yet whose latest DNS check failed reads "Check
    /// failed" ([`note_failed_policy_check`]), in its issues too.
    pub fn note_failed_policy_check(&mut self, check_failed: bool) {
        if !check_failed || self.row.policy_source != PolicySource::NotChecked {
            return;
        }
        note_failed_policy_check(&mut self.row.policy_source, true);
        let a = &self.assessment.verdicts;
        self.assessment.worded_issues =
            worded_issues(a, &overview_figures(&self.row, &self.verdicts, a));
    }
}

/// The overview judged `at`; `readiness_mail` is the domain's mail over
/// the readiness window (`READINESS_WINDOW_DAYS` before `now`), split by its
/// policy `row.policy.p` ([`readiness_mail`](dmarc_store::readiness_mail)).
/// Its policy steps are written into the published record, else into
/// `v=DMARC1; p=none` ([`domain_page`] names the report address in it).
pub fn domain_overview(
    row: DomainOverviewRow,
    readiness_mail: &ReadinessMailRow,
    at: Judging,
) -> DomainOverview {
    domain_page(row, readiness_mail, at, "")
}

/// [`domain_overview`] as the domain page shows it: a domain without a
/// single published record gets its policy steps written into a new record
/// naming `report_address` (when one is set).
pub fn domain_page(
    row: DomainOverviewRow,
    readiness_mail: &ReadinessMailRow,
    at: Judging,
    report_address: &str,
) -> DomainOverview {
    let record = match (&row.dmarc_record, report_address.trim()) {
        (Some(published), _) => published.clone(),
        (None, "") => "v=DMARC1; p=none".to_owned(),
        (None, address) => new_record(address),
    };
    let now = at.now;
    let policy = row.policy.as_ref();
    let policy_readiness = judged_readiness(readiness_mail, policy, row.parked, now);
    let facts = DomainFacts {
        mail: row.mail.mail(),
        no_aligned_dkim: row.no_aligned_dkim_messages,
        parked: row.parked,
        policy: policy_p(policy),
        dns_grade: row.dns.as_ref().map(|d| d.grade.as_str()),
        dns_critical_count: row.dns.as_ref().map_or(0, |d| d.critical_count),
        last_report_at: row.last_report_at,
    };
    let assessment = assess_domain(&facts, now, at.reports_stopped_after_days);
    let verdicts = facts.mail.verdicts();
    let worded_issues = worded_issues(&assessment, &overview_figures(&row, &verdicts, &assessment));
    DomainOverview {
        dmarc_failed_incl_forwarded: facts.mail.dmarc_failed_incl_forwarded(),
        verdicts,
        assessment: AssessmentView {
            verdicts: assessment,
            worded_issues,
        },
        org_domain: org_domain_of(&row.domain),
        policy_readiness: Readiness::new(policy_readiness, row.parked, &record),
        row,
    }
}

/// One SPF check of a record, as stored at ingest, with what it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SpfCheck {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: SpfCheckRow,
    pub state: CheckState,
    pub status: Status,
}

impl From<SpfCheckRow> for SpfCheck {
    fn from(row: SpfCheckRow) -> Self {
        let state = CheckState::of(row.result.as_deref(), row.aligned);
        Self {
            row,
            state,
            status: state.status(),
        }
    }
}

/// One DKIM signature check of a record, as stored at ingest, with what it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DkimCheck {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: DkimCheckRow,
    pub state: CheckState,
    pub status: Status,
}

impl From<DkimCheckRow> for DkimCheck {
    fn from(row: DkimCheckRow) -> Self {
        let state = CheckState::of(row.result.as_deref(), row.aligned);
        Self {
            row,
            state,
            status: state.status(),
        }
    }
}

/// A record's checks judged, each list most relevant first (see
/// [`CheckState`]), stored order among equals.
pub struct JudgedChecks {
    pub spf: Vec<SpfCheck>,
    pub dkim: Vec<DkimCheck>,
    pub overrides: Vec<OverrideReason>,
}

impl From<RecordChecks> for JudgedChecks {
    fn from(c: RecordChecks) -> Self {
        let mut spf: Vec<SpfCheck> = c.spf.into_iter().map(Into::into).collect();
        let mut dkim: Vec<DkimCheck> = c.dkim.into_iter().map(Into::into).collect();
        spf.sort_by_key(|c| c.state);
        dkim.sort_by_key(|c| c.state);
        Self {
            spf,
            dkim,
            overrides: c.overrides,
        }
    }
}

/// One failures-tab aggregate row: records with the same source, verdict,
/// disposition, reporter, checks and overrides.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DomainFailureRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: FailureRow,
    /// Every SPF check of the records, with the stored result and alignment.
    pub spf_auth: Vec<SpfCheck>,
    /// Every DKIM signature of the records, with the stored result and alignment.
    pub dkim_auth: Vec<DkimCheck>,
    pub overrides: Vec<OverrideReason>,
}

impl From<FailureRow> for DomainFailureRow {
    fn from(mut row: FailureRow) -> Self {
        let checks = JudgedChecks::from(std::mem::take(&mut row.checks));
        Self {
            row,
            spf_auth: checks.spf,
            dkim_auth: checks.dkim,
            overrides: checks.overrides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_are_ranked_most_relevant_first() {
        let checks = RecordChecks {
            spf: vec![
                SpfCheckRow {
                    domain: Some("other.example".into()),
                    scope: None,
                    result: Some("pass".into()),
                    aligned: false,
                },
                SpfCheckRow {
                    domain: Some("example.com".into()),
                    scope: Some("mfrom".into()),
                    result: Some("pass".into()),
                    aligned: true,
                },
            ],
            dkim: vec![],
            overrides: vec![],
        };
        let judged = JudgedChecks::from(checks);
        assert_eq!(judged.spf[0].state, CheckState::Pass);
        assert_eq!(judged.spf[1].state, CheckState::PassOther);
        assert_eq!(judged.spf[1].status, Status::Neutral);
    }
}
