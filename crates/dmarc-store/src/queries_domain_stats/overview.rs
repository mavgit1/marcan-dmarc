//! One domain's overview: its identity, the policy it shows and its figures
//! within a time window.

use super::policy::{applied_policy_columns, PolicySource, PublishedPolicy};
use super::{DomainOrganization, StoredDnsColumns, StoredDnsGrade, STORED_DNS_COLUMNS};
use crate::outcome::{
    MailCounts, DKIM_ONLY_MESSAGES_EXPR, MAIL_COUNTS_SELECT, NO_ALIGNED_DKIM_MESSAGES_EXPR,
    SPF_ONLY_MESSAGES_EXPR,
};
use crate::queries::Scope;
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder};
use ts_rs::TS;

/// One domain's identity, published policy and figures within `since`.
#[derive(Debug, Clone, Serialize, TS)]
pub struct DomainOverviewRow {
    pub id: i64,
    pub domain: String,
    pub organization: DomainOrganization,
    /// The stored DNS check's DMARC policy, as on the domain list. `None`
    /// without a record (`policy_source`).
    pub policy: Option<PublishedPolicy>,
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    /// See [`NO_ALIGNED_DKIM_MESSAGES_EXPR`].
    pub no_aligned_dkim_messages: i64,
    /// Passed via DKIM only ([`DKIM_ONLY_MESSAGES_EXPR`]).
    pub dkim_only_messages: i64,
    /// Passed via SPF only ([`SPF_ONLY_MESSAGES_EXPR`]).
    pub spf_only_messages: i64,
    pub parked: bool,
    /// Distinct reporting organizations in scope.
    pub reporter_count: i64,
    /// Newest `date_end` across all reports (ignores `since`); `None` when none were received.
    pub last_report_at: Option<i64>,
    /// The stored DNS check, as on the domain list; `None` before the first one.
    pub dns: Option<StoredDnsGrade>,
    /// Where `policy` comes from (the domain list's rule, [`PolicySource`]).
    pub policy_source: PolicySource,
    /// The DMARC record of the stored DNS check, as published; `None`
    /// without a stored check, or with no record or several. The next policy
    /// step is written into it.
    pub dmarc_record: Option<String>,
}

#[derive(Debug, FromRow)]
struct DomainHeadRow {
    id: i64,
    domain: String,
    #[sqlx(flatten)]
    organization: DomainOrganization,
    last_report_at: Option<i64>,
    parked: bool,
    #[sqlx(flatten)]
    dns: StoredDnsColumns,
    dmarc_record: Option<String>,
    #[sqlx(flatten)]
    policy: PublishedPolicy,
    #[sqlx(try_from = "String")]
    policy_source: PolicySource,
}

#[derive(Debug, FromRow)]
struct OverviewStatsRow {
    #[sqlx(flatten)]
    mail: MailCounts,
    no_aligned_dkim_messages: i64,
    dkim_only_messages: i64,
    spf_only_messages: i64,
    reporter_count: i64,
}

/// Return `true` when `domains.id` exists and is not ignored (removed).
pub async fn domain_exists(pool: &SqlitePool, domain_id: i64) -> Result<bool, StoreError> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM domains WHERE id = ? AND ignored = 0")
            .bind(domain_id)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

/// Domain overview, or `None` when `{id}` is unknown or ignored.
///
/// Totals are `SUM(records.count)` (not `reports.total_messages`) so they
/// match every other screen.
pub async fn get_domain_overview(
    pool: &SqlitePool,
    domain_id: i64,
    since: Option<i64>,
) -> Result<Option<DomainOverviewRow>, StoreError> {
    let policy_columns =
        applied_policy_columns(&["p", "sp", "pct", "adkim", "aspf", "testing", "np"]);
    let head = sqlx::query_as::<_, DomainHeadRow>(&format!(
        r#"
        SELECT d.id, d.domain, d.organization_id, org.name AS organization_name, d.parked,
               (SELECT MAX(r.date_end) FROM reports r WHERE r.domain_id = d.id) AS last_report_at,
               {STORED_DNS_COLUMNS},
               s.dmarc_record AS dmarc_record,
               {policy_columns}
        FROM domains d
        INNER JOIN organizations org ON org.id = d.organization_id
        LEFT JOIN dns_snapshots s ON s.domain_id = d.id
        WHERE d.id = ? AND d.ignored = 0
        "#
    ))
    .bind(domain_id)
    .fetch_optional(pool)
    .await?;

    let Some(head) = head else {
        return Ok(None);
    };
    let mut stats_qb = QueryBuilder::new(format!(
        r#"
        SELECT {MAIL_COUNTS_SELECT},
            {NO_ALIGNED_DKIM_MESSAGES_EXPR} AS no_aligned_dkim_messages,
            {DKIM_ONLY_MESSAGES_EXPR} AS dkim_only_messages,
            {SPF_ONLY_MESSAGES_EXPR} AS spf_only_messages,
            COUNT(DISTINCT r.org_name) AS reporter_count
        FROM reports r
        LEFT JOIN records rec ON rec.report_id = r.id
        WHERE 1=1
        "#
    ));
    Scope::domain(domain_id, since).push_reports(&mut stats_qb, "r");
    let stats = stats_qb
        .build_query_as::<OverviewStatsRow>()
        .fetch_one(pool)
        .await?;

    Ok(Some(DomainOverviewRow {
        id: head.id,
        domain: head.domain,
        organization: head.organization,
        policy: head.policy.published(),
        mail: stats.mail,
        no_aligned_dkim_messages: stats.no_aligned_dkim_messages,
        dkim_only_messages: stats.dkim_only_messages,
        spf_only_messages: stats.spf_only_messages,
        parked: head.parked,
        reporter_count: stats.reporter_count,
        last_report_at: head.last_report_at,
        dns: head.dns.stored(),
        policy_source: head.policy_source,
        dmarc_record: head.dmarc_record,
    }))
}
