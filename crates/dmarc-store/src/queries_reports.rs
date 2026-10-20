//! `GET /api/reports` (paged, searchable, sortable), the report header of
//! `GET /api/reports/{id}` and its records, and report lookups for ingest.

use crate::outcome::{
    delivered_compliance_sort, MailCounts, DELIVERED_FAILURES_EXPR, FORWARDED_FAILURES_EXPR,
    OUTCOME_EXPR, VERDICT_EXPR,
};
use crate::queries::{contains_pattern, Scope};
use crate::record_checks::{RecordChecks, RECORD_CHECKS_JSON};
use crate::sort::{sort_keys, Sort, SortDir, SortKey};
use crate::StoreError;
use dmarc_assess::{DmarcVerdict, Outcome};
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder, Sqlite};
use ts_rs::TS;

/// One `reports` row with its domain name and message counts.
#[derive(Debug, Clone, FromRow, Serialize, TS)]
pub struct ReportSummaryRow {
    pub id: i64,
    pub report_id: String,
    pub org_name: String,
    pub email: String,
    pub domain_id: i64,
    pub domain: String,
    pub date_begin: i64,
    pub date_end: i64,
    pub policy_p: Option<String>,
    pub policy_sp: Option<String>,
    pub policy_pct: Option<i64>,
    #[sqlx(flatten)]
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
}

/// `SELECT` for [`ReportSummaryRow`] over `reports r` joined to `domains d`; callers
/// append `AND …`. Also exposes `delivered_pct` and `blocked` for sorting.
fn report_summary_query() -> QueryBuilder<'static, Sqlite> {
    QueryBuilder::new(format!(
        r#"
        SELECT * FROM (
            SELECT
                r.id,
                r.report_id,
                r.org_name,
                r.email,
                r.domain_id,
                d.domain,
                r.date_begin,
                r.date_end,
                r.policy_p,
                r.policy_sp,
                r.policy_pct,
                r.total_messages,
                r.compliant_messages,
                (
                    SELECT {DELIVERED_FAILURES_EXPR}
                    FROM records rec
                    WHERE rec.report_id = r.id
                ) AS delivered_failures,
                (
                    SELECT {FORWARDED_FAILURES_EXPR}
                    FROM records rec
                    WHERE rec.report_id = r.id
                ) AS forwarded_failures
            FROM reports r
            INNER JOIN domains d ON d.id = r.domain_id
            WHERE 1=1
        "#
    ))
}

sort_keys!(
    /// Sortable columns of the reports list.
    ReportSort {
        DateBegin => "date_begin",
        Domain => "domain",
        OrgName => "org_name",
        TotalMessages => "total_messages",
        DeliveredPct => "delivered_pct",
        Blocked => "blocked",
        PolicyP => "policy_p",
    }
);

impl ReportSort {
    /// Newest first.
    pub const DEFAULT: Sort<Self> = Sort::new(Self::DateBegin, SortDir::Desc);
}

/// Scope, search, order and page of [`list_reports`].
#[derive(Debug, Clone)]
pub struct ReportListQuery {
    pub scope: Scope,
    /// Case-insensitive substring of the domain or the reporting organization.
    pub search: Option<String>,
    pub sort: Sort<ReportSort>,
    pub limit: i64,
    pub offset: i64,
}

fn push_filters(qb: &mut QueryBuilder<'_, Sqlite>, q: &ReportListQuery) {
    q.scope.push_reports(qb, "r");
    if let Some(pattern) = contains_pattern(q.search.as_deref()) {
        qb.push(" AND (d.domain LIKE ")
            .push_bind(pattern.clone())
            .push(" ESCAPE '\\' OR r.org_name LIKE ")
            .push_bind(pattern)
            .push(" ESCAPE '\\')");
    }
}

/// A page of reports plus the number of reports matching scope and search.
pub async fn list_reports(
    pool: &SqlitePool,
    q: &ReportListQuery,
) -> Result<(Vec<ReportSummaryRow>, i64), StoreError> {
    let mut count_qb = QueryBuilder::new(
        "SELECT COUNT(*) FROM reports r INNER JOIN domains d ON d.id = r.domain_id WHERE 1=1",
    );
    push_filters(&mut count_qb, q);
    let total: i64 = count_qb.build_query_scalar().fetch_one(pool).await?;

    let mut qb = report_summary_query();
    push_filters(&mut qb, q);
    qb.push(format!(
        ") s ORDER BY {}, s.date_begin DESC, s.id DESC LIMIT ",
        sort_expr(q.sort)
    ));
    qb.push_bind(q.limit);
    qb.push(" OFFSET ");
    qb.push_bind(q.offset);

    let items = qb
        .build_query_as::<ReportSummaryRow>()
        .fetch_all(pool)
        .await?;
    Ok((items, total))
}

/// `ORDER BY` term for `sort` over the aliased subquery `s`.
fn sort_expr(sort: Sort<ReportSort>) -> String {
    let expr = match sort.key {
        // Delivered share and blocked count are derived only to sort by them.
        ReportSort::DeliveredPct => {
            delivered_compliance_sort("s.compliant_messages", "s.delivered_failures")
        }
        ReportSort::Blocked => {
            "MAX(0, s.total_messages - s.compliant_messages - s.delivered_failures - s.forwarded_failures)".to_owned()
        }
        key => format!("s.{}", key.column()),
    };
    format!("{expr} {}", sort.dir.sql())
}

/// One report's header, or `None` when `id` is unknown or its domain is ignored.
pub async fn get_report_summary(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<ReportSummaryRow>, StoreError> {
    let mut qb = report_summary_query();
    Scope::ALL.push_reports(&mut qb, "r");
    qb.push(" AND r.id = ").push_bind(id).push(") s");
    Ok(qb
        .build_query_as::<ReportSummaryRow>()
        .fetch_optional(pool)
        .await?)
}

/// A row from the `reports` table.
#[derive(Debug, Clone, FromRow)]
pub struct StoredReport {
    pub id: i64,
    pub report_id: String,
    pub org_name: String,
    pub email: String,
    pub domain_id: i64,
    pub date_begin: i64,
    pub date_end: i64,
    pub policy_p: Option<String>,
    pub policy_sp: Option<String>,
    pub policy_pct: Option<i64>,
    pub total_messages: i64,
    pub compliant_messages: i64,
}

/// A `records` row with its [`DmarcVerdict`], [`Outcome`] and stored checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct RecordRow {
    pub id: i64,
    /// Not sent: the report is the one asked for.
    #[serde(skip)]
    #[ts(skip)]
    pub report_id: i64,
    pub source_ip: String,
    pub count: i64,
    pub disposition: Option<String>,
    pub dkim_result: Option<String>,
    pub spf_result: Option<String>,
    pub header_from: Option<String>,
    pub envelope_from: Option<String>,
    pub verdict: DmarcVerdict,
    pub outcome: Outcome,
    /// Not sent as stored: the server sends them judged.
    #[serde(skip)]
    #[ts(skip)]
    pub checks: RecordChecks,
}

#[derive(Debug, FromRow)]
struct RecordSql {
    id: i64,
    report_id: i64,
    source_ip: String,
    count: i64,
    disposition: Option<String>,
    dkim_result: Option<String>,
    spf_result: Option<String>,
    header_from: Option<String>,
    envelope_from: Option<String>,
    #[sqlx(try_from = "String")]
    verdict: DmarcVerdict,
    #[sqlx(try_from = "String")]
    outcome: Outcome,
    spf_auth: Option<String>,
    dkim_auth: Option<String>,
    overrides: Option<String>,
}

impl From<RecordSql> for RecordRow {
    fn from(r: RecordSql) -> Self {
        Self {
            id: r.id,
            report_id: r.report_id,
            source_ip: r.source_ip,
            count: r.count,
            disposition: r.disposition,
            dkim_result: r.dkim_result,
            spf_result: r.spf_result,
            header_from: r.header_from,
            envelope_from: r.envelope_from,
            verdict: r.verdict,
            outcome: r.outcome,
            checks: RecordChecks::from_json(
                r.spf_auth.as_deref(),
                r.dkim_auth.as_deref(),
                r.overrides.as_deref(),
            ),
        }
    }
}

/// Look up a report by its reporting organization and external `report_id`.
/// The id is only unique per reporter, so both are needed.
pub async fn find_report_by_report_id(
    pool: &SqlitePool,
    org_name: &str,
    report_id: &str,
) -> Result<Option<StoredReport>, StoreError> {
    let row = sqlx::query_as::<_, StoredReport>(
        r#"
        SELECT id, report_id, org_name, email, domain_id, date_begin, date_end,
               policy_p, policy_sp, policy_pct, total_messages, compliant_messages
        FROM reports
        WHERE org_name = ? AND report_id = ?
        "#,
    )
    .bind(org_name)
    .bind(report_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Every record of the report with internal id `report_pk`, with its checks,
/// in stored order.
pub async fn list_records_for_report(
    pool: &SqlitePool,
    report_pk: i64,
) -> Result<Vec<RecordRow>, StoreError> {
    let rows = sqlx::query_as::<_, RecordSql>(&format!(
        r#"
        SELECT rec.id, rec.report_id, rec.source_ip, rec.count, rec.disposition,
               rec.dkim_result, rec.spf_result, rec.header_from, rec.envelope_from,
               {VERDICT_EXPR} AS verdict, {OUTCOME_EXPR} AS outcome,
               {RECORD_CHECKS_JSON}
        FROM records rec
        WHERE rec.report_id = ?
        ORDER BY rec.id
        "#
    ))
    .bind(report_pk)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(RecordRow::from).collect())
}
