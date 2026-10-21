//! The domain list: every visible domain with its figures, and the page of it
//! the UI shows (search, sort, paging).

use super::policy::{applied_policy_columns, PolicySource, PublishedPolicy};
use super::{DomainOrganization, StoredDnsColumns, StoredDnsGrade, STORED_DNS_COLUMNS};
use crate::outcome::{
    delivered_compliance_sort, MailCounts, MAIL_COUNTS_SELECT, NO_ALIGNED_DKIM_MESSAGES_EXPR,
};
use crate::queries::{contains_pattern, Scope};
use crate::sort::{sort_keys, Sort, SortDir, SortKey};
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use ts_rs::TS;

/// One monitored domain with its stored figures: the domain list's row before
/// the verdicts are added.
#[derive(Debug, Clone, FromRow, Serialize, TS)]
pub struct DomainSummaryRow {
    pub id: i64,
    #[sqlx(flatten)]
    pub organization: DomainOrganization,
    pub domain: String,
    /// All-time report count, independent of `since`: "no reports yet" must not
    /// flip on when the time range merely excludes older reports.
    pub report_count: i64,
    /// Newest `date_end` across all reports; `None` when none were received.
    pub last_report_at: Option<i64>,
    /// The stored DNS check's DMARC policy; `None` without a record
    /// (`policy_source` says why).
    #[sqlx(skip)]
    pub policy: Option<PublishedPolicy>,
    /// Whether `policy` comes from DNS, or why there is none.
    #[sqlx(try_from = "String")]
    pub policy_source: PolicySource,
    /// Distinct reporting organizations within `since` (same definition as the
    /// domain overview).
    pub reporter_count: i64,
    /// Message counts within `since`.
    #[sqlx(flatten)]
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    /// Passed DMARC without an aligned DKIM pass: SPF alone carried them
    /// (see [`NO_ALIGNED_DKIM_MESSAGES_EXPR`]).
    pub no_aligned_dkim_messages: i64,
    /// Sends no mail of its own: judged by `p=reject` and unexpected passing mail.
    pub parked: bool,
    /// The stored DNS check; `None` before the first one.
    #[sqlx(skip)]
    pub dns: Option<StoredDnsGrade>,
}

/// A [`DomainSummaryRow`] as read: its policy and DNS columns beside it.
#[derive(FromRow)]
struct DomainSummaryFetch {
    #[sqlx(flatten)]
    row: DomainSummaryRow,
    #[sqlx(flatten)]
    policy: PublishedPolicy,
    #[sqlx(flatten)]
    dns: StoredDnsColumns,
}

impl From<DomainSummaryFetch> for DomainSummaryRow {
    fn from(f: DomainSummaryFetch) -> Self {
        Self {
            policy: f.policy.published(),
            dns: f.dns.stored(),
            ..f.row
        }
    }
}

/// `SELECT * FROM (SELECT …` of [`DomainSummaryRow`] over `domains d` (with
/// `organizations org`): the visible domains in `scope`, volumes within its
/// time window. Callers append `AND …` clauses over `d` / `org`, then `) s`
/// and an order over `s`.
fn domain_summary_query(scope: Scope) -> QueryBuilder<'static, Sqlite> {
    let policy_columns =
        applied_policy_columns(&["p", "sp", "pct", "adkim", "aspf", "testing", "np"]);
    let mut qb: QueryBuilder<'_, Sqlite> = QueryBuilder::new(format!(
        r#"
    SELECT * FROM (
    SELECT
        d.id,
        d.organization_id,
        org.name AS organization_name,
        d.domain,
        COALESCE(a.report_count, 0) AS report_count,
        a.last_report_at AS last_report_at,
        {policy_columns},
        COALESCE(v.reporter_count, 0) AS reporter_count,
        COALESCE(v.total_messages, 0) AS total_messages,
        COALESCE(v.compliant_messages, 0) AS compliant_messages,
        COALESCE(v.delivered_failures, 0) AS delivered_failures,
        COALESCE(v.forwarded_failures, 0) AS forwarded_failures,
        COALESCE(v.no_aligned_dkim_messages, 0) AS no_aligned_dkim_messages,
        d.parked AS parked,
        {STORED_DNS_COLUMNS}
    FROM domains d
    INNER JOIN organizations org ON org.id = d.organization_id
    LEFT JOIN (
        SELECT domain_id, COUNT(*) AS report_count, MAX(date_end) AS last_report_at
        FROM reports GROUP BY domain_id
    ) a ON a.domain_id = d.id
    LEFT JOIN (
        SELECT
            r.domain_id,
            COUNT(DISTINCT r.org_name) AS reporter_count,
            {MAIL_COUNTS_SELECT},
            {NO_ALIGNED_DKIM_MESSAGES_EXPR} AS no_aligned_dkim_messages
        FROM reports r
        LEFT JOIN records rec ON rec.report_id = r.id
        WHERE 1=1
"#
    ));
    scope.push_reports(&mut qb, "r");
    qb.push(
        r#"
        GROUP BY r.domain_id
    ) v ON v.domain_id = d.id
    LEFT JOIN dns_snapshots s ON s.domain_id = d.id
    WHERE 1=1
"#,
    );
    scope.push_domains(&mut qb, "d");
    qb
}

/// Visible (not ignored) domains in `scope`; volumes within its time window,
/// so an organization's rows add up to its `/api/statistics` totals. Every
/// domain, by name (the alerts, the client reports and the CSV export); the
/// domain list pages them with [`page_domain_summaries`].
pub async fn list_domain_summaries(
    pool: &SqlitePool,
    scope: Scope,
) -> Result<Vec<DomainSummaryRow>, StoreError> {
    let mut qb = domain_summary_query(scope);
    qb.push(") s ORDER BY s.domain COLLATE NOCASE");
    Ok(qb
        .build_query_as::<DomainSummaryFetch>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

sort_keys!(
    /// Sortable columns of the domain list.
    DomainSort {
        Domain => "domain",
        Organization => "organization_name",
        DnsScore => "dns_score",
        Policy => "policy_p",
        TotalMessages => "total_messages",
        DeliveredPct => "delivered_pct",
        DeliveredFailures => "delivered_failures",
        LastReport => "last_report_at",
    }
);

impl DomainSort {
    /// By name.
    pub const DEFAULT: Sort<Self> = Sort::new(Self::Domain, SortDir::Asc);
}

/// Scope, filter, order and page of [`page_domain_summaries`].
#[derive(Debug, Clone)]
pub struct DomainListQuery {
    pub scope: Scope,
    /// Case-insensitive substring of the domain or its organization's name.
    pub search: Option<String>,
    pub sort: Sort<DomainSort>,
    pub limit: i64,
    pub offset: i64,
}

/// `AND …` over `domains d` and `organizations org`: the name of the domain or of
/// its organization contains `search`.
fn push_domain_search(qb: &mut QueryBuilder<'_, Sqlite>, search: Option<&str>) {
    if let Some(pattern) = contains_pattern(search) {
        qb.push(" AND (d.domain LIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR org.name LIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
}

/// `ORDER BY` term for `sort` over the aliased subquery `s`. Each column orders
/// by its stored value; a domain without one (no DNS check, no report) sorts first.
fn domain_sort_expr(sort: Sort<DomainSort>) -> String {
    let expr = match sort.key {
        DomainSort::Domain | DomainSort::Organization => {
            format!("s.{} COLLATE NOCASE", sort.key.column())
        }
        DomainSort::DeliveredPct => {
            delivered_compliance_sort("s.compliant_messages", "s.delivered_failures")
        }
        key => format!("s.{}", key.column()),
    };
    format!("{expr} {}", sort.dir.sql())
}

/// A page of [`list_domain_summaries`] narrowed by `search`, plus the number of
/// domains matching scope and search.
pub async fn page_domain_summaries(
    pool: &SqlitePool,
    q: &DomainListQuery,
) -> Result<(Vec<DomainSummaryRow>, i64), StoreError> {
    let mut count_qb = QueryBuilder::new(
        "SELECT COUNT(*) FROM domains d \
         INNER JOIN organizations org ON org.id = d.organization_id WHERE 1=1",
    );
    q.scope.push_domains(&mut count_qb, "d");
    push_domain_search(&mut count_qb, q.search.as_deref());
    let total: i64 = count_qb.build_query_scalar().fetch_one(pool).await?;

    let mut qb = domain_summary_query(q.scope);
    push_domain_search(&mut qb, q.search.as_deref());
    qb.push(format!(
        ") s ORDER BY {}, s.domain COLLATE NOCASE, s.id LIMIT ",
        domain_sort_expr(q.sort)
    ));
    qb.push_bind(q.limit);
    qb.push(" OFFSET ");
    qb.push_bind(q.offset);
    let items = qb
        .build_query_as::<DomainSummaryFetch>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok((items, total))
}

/// Visible domains in `scope`, with or without reports (its time window does
/// not apply).
pub async fn count_domains(pool: &SqlitePool, scope: Scope) -> Result<i64, StoreError> {
    let mut qb = QueryBuilder::new("SELECT COUNT(*) FROM domains d WHERE 1=1");
    scope.push_domains(&mut qb, "d");
    Ok(qb.build_query_scalar().fetch_one(pool).await?)
}

/// One domain's [`DomainSummaryRow`] over all time, or `None` when `id` is
/// unknown or ignored.
pub async fn get_domain_summary(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<DomainSummaryRow>, StoreError> {
    Ok(list_domain_summaries(pool, Scope::domain(id, None))
        .await?
        .pop())
}
