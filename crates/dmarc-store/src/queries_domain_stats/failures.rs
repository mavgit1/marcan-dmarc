//! The Failures tab: which records it shows (shared with the auth-issues
//! export) and its aggregate rows.

use crate::outcome::{outcome_rank_expr, OUTCOME_EXPR, VERDICT_EXPR};
use crate::queries::Scope;
use crate::record_checks::{RecordChecks, RECORD_CHECKS_JSON};
use crate::sort::{sort_keys, Sort, SortDir};
use crate::StoreError;
use dmarc_assess::{DmarcVerdict, Outcome};
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use ts_rs::TS;

/// Failures tab / auth-issues export filter, as the query parameter names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TS)]
#[ts(export, rename_all = "snake_case")]
pub enum FailureOutcome {
    /// Every DMARC failure: delivered, forwarded and blocked. Forwarded mail
    /// is in it, unlike a bare "failed" count, hence the name.
    AllFailures,
    /// DMARC failed and the receiver delivered it, without a trusted forward.
    Delivered,
    /// DMARC failed, delivered because the receiver trusted the forwarder.
    Forwarded,
    /// DMARC failed and the receiver quarantined or rejected it.
    Blocked,
    /// DMARC passed via DKIM only (SPF has no aligned pass).
    DkimOnly,
    /// DMARC passed via SPF only (DKIM has no aligned pass).
    SpfOnly,
}

impl FailureOutcome {
    pub const ALL: [Self; 6] = [
        Self::AllFailures,
        Self::Delivered,
        Self::Forwarded,
        Self::Blocked,
        Self::DkimOnly,
        Self::SpfOnly,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllFailures => "all_failures",
            Self::Delivered => "delivered",
            Self::Forwarded => "forwarded",
            Self::Blocked => "blocked",
            Self::DkimOnly => "dkim_only",
            Self::SpfOnly => "spf_only",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == raw)
    }
}

/// Which failures to show; shared by the Failures tab and its CSV export so the
/// two always contain the same records. The time window is part of the scope.
#[derive(Debug, Clone, Default)]
pub struct DomainFailureFilter<'a> {
    /// The kinds to include, a record matching any of them. Empty: every record
    /// where SPF or DKIM lacks an aligned pass.
    pub outcomes: Vec<FailureOutcome>,
    pub source_ip: Option<&'a str>,
}

impl DomainFailureFilter<'_> {
    /// `AND …` clauses over `records rec`: at least one of SPF/DKIM lacks an
    /// aligned pass, plus the optional filters. The kinds become `IN (…)` over
    /// [`OUTCOME_EXPR`] and [`VERDICT_EXPR`], joined by `OR`.
    pub(crate) fn push_predicate(&self, qb: &mut QueryBuilder<'_, Sqlite>) {
        qb.push(format!(" AND {VERDICT_EXPR} != 'pass_both'"));
        let mut outcomes: Vec<&str> = Vec::new();
        let mut verdicts: Vec<&str> = Vec::new();
        for kind in &self.outcomes {
            let (list, values): (&mut Vec<&str>, &[&str]) = match kind {
                FailureOutcome::AllFailures => {
                    (&mut outcomes, &["delivered", "forwarded", "blocked"])
                }
                FailureOutcome::Delivered => (&mut outcomes, &["delivered"]),
                FailureOutcome::Forwarded => (&mut outcomes, &["forwarded"]),
                FailureOutcome::Blocked => (&mut outcomes, &["blocked"]),
                FailureOutcome::DkimOnly => (&mut verdicts, &["pass_dkim"]),
                FailureOutcome::SpfOnly => (&mut verdicts, &["pass_spf"]),
            };
            for value in values {
                if !list.contains(value) {
                    list.push(value);
                }
            }
        }
        let in_list = |expr: &str, values: &[&str]| {
            let quoted: Vec<String> = values.iter().map(|v| format!("'{v}'")).collect();
            format!("{expr} IN ({})", quoted.join(", "))
        };
        let mut conditions = Vec::new();
        if !outcomes.is_empty() {
            conditions.push(in_list(OUTCOME_EXPR, &outcomes));
        }
        if !verdicts.is_empty() {
            conditions.push(in_list(VERDICT_EXPR, &verdicts));
        }
        if !conditions.is_empty() {
            qb.push(format!(" AND ({})", conditions.join(" OR ")));
        }
        self.push_source_ip(qb);
    }

    fn push_source_ip(&self, qb: &mut QueryBuilder<'_, Sqlite>) {
        if let Some(ip) = self.source_ip {
            qb.push(" AND rec.source_ip = ")
                .push_bind(ip.trim().to_owned());
        }
    }
}

/// One failures-tab aggregate row: records with the same source, verdict,
/// disposition, reporter, checks and overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct FailureRow {
    pub source_ip: String,
    pub verdict: DmarcVerdict,
    pub outcome: Outcome,
    pub disposition: Option<String>,
    /// The reporter.
    pub org_name: String,
    /// Not sent as stored: the server sends them judged.
    #[serde(skip)]
    #[ts(skip)]
    pub checks: RecordChecks,
    pub count: i64,
}

#[derive(Debug, FromRow)]
struct DomainFailureSql {
    source_ip: String,
    #[sqlx(try_from = "String")]
    verdict: DmarcVerdict,
    #[sqlx(try_from = "String")]
    outcome_kind: Outcome,
    disposition: Option<String>,
    org_name: String,
    spf_auth: Option<String>,
    dkim_auth: Option<String>,
    overrides: Option<String>,
    count: i64,
    total: i64,
}

sort_keys!(
    /// Sortable columns of the Failures tab. `outcome` ascending is worst first
    /// (delivered, forwarded, blocked, passed). `spf_domain` / `dkim_selector` order
    /// by the row's first listed check.
    FailureSort {
        Count => "count",
        SourceIp => "source_ip",
        Verdict => "verdict",
        Outcome => "outcome",
        Disposition => "disposition",
        SpfDomain => "spf_domain",
        DkimSelector => "dkim_selector",
        OrgName => "org_name",
    }
);

impl FailureSort {
    /// Worst first.
    pub const DEFAULT: Sort<Self> = Sort::new(Self::Count, SortDir::Desc);
}

/// Domain, time window, filter, order and page of [`list_domain_failures`].
#[derive(Debug, Clone)]
pub struct DomainFailureQuery<'a> {
    pub domain_id: i64,
    /// Records of reports from this time on; `None`: all time.
    pub since: Option<i64>,
    pub filter: DomainFailureFilter<'a>,
    pub sort: Sort<FailureSort>,
    pub limit: i64,
    pub offset: i64,
}

/// Failures tab: records of `q.domain_id` from `q.since` on where at least one
/// of SPF/DKIM is not an aligned pass (`verdict` is `fail`, `pass_spf` or
/// `pass_dkim`).
///
/// Records are first reduced to one row each (child rows are JSON arrays), so
/// message volume is not inflated. Returns the page and the total number of rows.
pub async fn list_domain_failures(
    pool: &SqlitePool,
    q: &DomainFailureQuery<'_>,
) -> Result<(Vec<FailureRow>, i64), StoreError> {
    let outcome_rank = outcome_rank_expr("outcome_kind");
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT
            source_ip, verdict, outcome_kind, disposition, org_name, spf_auth, dkim_auth,
            overrides,
            -- Sort key only: worst first.
            {outcome_rank} AS outcome,
            json_extract(spf_auth, '$[0].domain') AS spf_domain,
            json_extract(dkim_auth, '$[0].selector') AS dkim_selector,
            COALESCE(SUM(count), 0) AS count,
            COUNT(*) OVER() AS total
        FROM (
            SELECT
                rec.source_ip AS source_ip,
                {VERDICT_EXPR} AS verdict,
                {OUTCOME_EXPR} AS outcome_kind,
                rec.disposition AS disposition,
                r.org_name AS org_name,
                rec.count AS count,
                {RECORD_CHECKS_JSON}
            FROM records rec
            INNER JOIN reports r ON r.id = rec.report_id
            WHERE 1=1
        "#
    ));
    Scope::domain(q.domain_id, q.since).push_reports(&mut qb, "r");
    q.filter.push_predicate(&mut qb);
    qb.push(format!(
        r#"
        )
        GROUP BY source_ip, verdict, outcome_kind, disposition, org_name, spf_auth, dkim_auth,
                 overrides
        ORDER BY {}, count DESC, source_ip
        LIMIT "#,
        q.sort.order_by()
    ));
    qb.push_bind(q.limit);
    qb.push(" OFFSET ");
    qb.push_bind(q.offset);

    let rows = qb
        .build_query_as::<DomainFailureSql>()
        .fetch_all(pool)
        .await?;
    let total = rows.first().map_or(0, |r| r.total);
    let items = rows
        .into_iter()
        .map(|r| FailureRow {
            source_ip: r.source_ip,
            verdict: r.verdict,
            outcome: r.outcome_kind,
            disposition: r.disposition,
            org_name: r.org_name,
            checks: RecordChecks::from_json(
                r.spf_auth.as_deref(),
                r.dkim_auth.as_deref(),
                r.overrides.as_deref(),
            ),
            count: r.count,
        })
        .collect();
    Ok((items, total))
}
