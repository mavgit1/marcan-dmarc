//! `GET /api/senders`: one row per source IP in an organization / domain / time scope.
//! Used by the Senders page, the dashboard's failing senders and a domain's
//! Senders tab.

use crate::outcome::{
    pass_rate_sort, COMPLIANT_MESSAGES_EXPR, DKIM_PASS_MESSAGES_EXPR, FAILED_MESSAGES_EXPR,
    FORWARDED_FAILURES_EXPR, SPF_PASS_MESSAGES_EXPR,
};
use crate::queries::{Scope, VISIBLE_DOMAIN_IDS};
use crate::sort::{sort_keys, Sort, SortDir};
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder};
use ts_rs::TS;

/// One source IP within the scope.
#[derive(Debug, Clone, FromRow, Serialize, TS)]
pub struct SenderSummaryRow {
    pub source_ip: String,
    pub total_messages: i64,
    /// `dmarc_pass = 1`, the product-wide definition of compliant.
    pub compliant_messages: i64,
    /// Failed DMARC and not a trusted forward: delivered failures plus
    /// blocked by policy (see [`FAILED_MESSAGES_EXPR`]).
    pub failed_messages: i64,
    /// Failed DMARC, delivered because the receiver trusted the forwarder.
    /// `total = compliant + failed + forwarded`.
    pub forwarded_failures: i64,
    /// Messages with an aligned SPF pass.
    pub spf_pass_messages: i64,
    /// Messages with an aligned DKIM pass.
    pub dkim_pass_messages: i64,
    pub report_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
    /// Distinct domains this IP sent as, over all time and all organizations.
    /// Unscoped on purpose: an IP on the page for one domain that also sends as
    /// many others is a shared service (or a spoofer), whatever the filter.
    pub domain_count: i64,
    /// Rows in scope (the same on every row of a page); not part of the row the API sends.
    #[serde(skip)]
    #[ts(skip)]
    total: i64,
}

sort_keys!(
    /// Sortable columns of the senders list.
    SenderSort {
        TotalMessages => "total_messages",
        FailedMessages => "failed_messages",
        CompliancePct => "compliance_pct",
        SpfPassMessages => "spf_pass_messages",
        DkimPassMessages => "dkim_pass_messages",
        ReportCount => "report_count",
        LastSeen => "last_seen",
        SourceIp => "source_ip",
    }
);

impl SenderSort {
    /// Busiest first.
    pub const DEFAULT: Sort<Self> = Sort::new(Self::TotalMessages, SortDir::Desc);
}

/// Scope, order and page of [`list_senders`].
#[derive(Debug, Clone, Copy)]
pub struct SenderListQuery {
    pub scope: Scope,
    /// Only the senders with mail that failed DMARC and was not forwarded
    /// (`failed_messages > 0`).
    pub failing: bool,
    pub sort: Sort<SenderSort>,
    pub limit: i64,
    pub offset: i64,
}

/// A page of senders plus the number of source IPs in scope.
///
/// Totals are `SUM(records.count)`. `domain_count` is computed for the page rows only.
pub async fn list_senders(
    pool: &SqlitePool,
    q: &SenderListQuery,
) -> Result<(Vec<SenderSummaryRow>, i64), StoreError> {
    let order_by = format!("{}, source_ip", q.sort.order_by());
    // The pass rate is selected only to sort by it ([`pass_rate_sort`]).
    let pass_rate = pass_rate_sort(COMPLIANT_MESSAGES_EXPR, "SUM(rec.count)");
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT page.*,
            (
                SELECT COUNT(DISTINCT r2.domain_id)
                FROM records rec2
                INNER JOIN reports r2 ON r2.id = rec2.report_id
                WHERE rec2.source_ip = page.source_ip
                  AND r2.domain_id IN {VISIBLE_DOMAIN_IDS}
            ) AS domain_count
        FROM (
            SELECT
                rec.source_ip AS source_ip,
                COALESCE(SUM(rec.count), 0) AS total_messages,
                {COMPLIANT_MESSAGES_EXPR} AS compliant_messages,
                {FAILED_MESSAGES_EXPR} AS failed_messages,
                {FORWARDED_FAILURES_EXPR} AS forwarded_failures,
                {pass_rate} AS compliance_pct,
                {SPF_PASS_MESSAGES_EXPR} AS spf_pass_messages,
                {DKIM_PASS_MESSAGES_EXPR} AS dkim_pass_messages,
                COUNT(DISTINCT r.id) AS report_count,
                MIN(r.date_begin) AS first_seen,
                MAX(r.date_end) AS last_seen,
                COUNT(*) OVER() AS total
            FROM records rec
            INNER JOIN reports r ON r.id = rec.report_id
            WHERE 1=1
        "#
    ));
    q.scope.push_reports(&mut qb, "r");
    qb.push(" GROUP BY rec.source_ip");
    if q.failing {
        qb.push(format!(" HAVING {FAILED_MESSAGES_EXPR} > 0"));
    }
    qb.push(format!(" ORDER BY {order_by} LIMIT "));
    qb.push_bind(q.limit);
    qb.push(" OFFSET ");
    qb.push_bind(q.offset);
    qb.push(format!(") page ORDER BY {order_by}"));

    let rows = qb
        .build_query_as::<SenderSummaryRow>()
        .fetch_all(pool)
        .await?;
    let total = rows.first().map_or(0, |r| r.total);
    Ok((rows, total))
}
