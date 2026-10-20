//! Row sources for the CSV exports under `/api/export/*.csv`.
//!
//! Exports have no row limit, so rows are handed to a callback one at a time
//! while the query is still running instead of being collected into a `Vec`.
//! Every query uses the same scope and aggregate expressions as the screen it
//! mirrors, so an export holds exactly what the UI shows. Lists travel as JSON
//! arrays (`json_group_array`), never as delimited strings.

use crate::outcome::{
    MailCounts, DELIVERED_EXPR, DKIM_PASS_MESSAGES_EXPR, MAIL_COUNTS_SELECT,
    NO_ALIGNED_DKIM_MESSAGES_EXPR, OUTCOME_EXPR, SPF_PASS_MESSAGES_EXPR,
};
use crate::queries::Scope;
use crate::queries_domain_stats::DomainFailureFilter;
use crate::record_checks::parse_json_list;
use crate::StoreError;
use dmarc_assess::Outcome;
use futures::TryStreamExt;
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use std::future::Future;

/// Run `qb` and hand each row to `on_row` as it arrives. `on_row` returns
/// `false` to stop early (the client went away).
async fn for_each_row<R, F, Fut>(
    pool: &SqlitePool,
    mut qb: QueryBuilder<'_, Sqlite>,
    mut on_row: F,
) -> Result<(), StoreError>
where
    R: for<'r> FromRow<'r, SqliteRow> + Send + Unpin,
    F: FnMut(R) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut rows = qb.build_query_as::<R>().fetch(pool);
    while let Some(row) = rows.try_next().await? {
        if !on_row(row).await {
            break;
        }
    }
    Ok(())
}

/// A `json_group_array` of strings as a list: trimmed, without nulls, empties
/// and duplicates, sorted so list cells are stable across runs.
fn sorted_list(json: Option<&str>) -> Vec<String> {
    let mut items = ordered_list(json);
    items.sort_unstable();
    items.dedup();
    items
}

/// Like [`sorted_list`] but keeps first-seen order (auth results in report order).
fn ordered_list(json: Option<&str>) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    for part in parse_json_list::<Option<String>>(json)
        .into_iter()
        .flatten()
    {
        let part = part.trim();
        if !part.is_empty() && !items.iter().any(|i| i == part) {
            items.push(part.to_owned());
        }
    }
    items
}

/// One sender IP × domain row of `senders.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderExportRow {
    pub organization: String,
    pub domain: String,
    pub source_ip: String,
    pub mail: MailCounts,
    pub spf_aligned_pass_messages: i64,
    pub dkim_aligned_pass_messages: i64,
    /// Quarantined or rejected failures ([`MailCounts::blocked`]).
    pub blocked_messages: i64,
    /// Passing without an aligned DKIM pass (same definition as the domain rows).
    pub no_aligned_dkim_messages: i64,
    pub envelope_from: Vec<String>,
    pub dkim_selectors: Vec<String>,
    pub reporters: Vec<String>,
    pub report_count: i64,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, FromRow)]
struct SenderExportSql {
    organization: String,
    domain: String,
    source_ip: String,
    #[sqlx(flatten)]
    mail: MailCounts,
    spf_aligned_pass_messages: i64,
    dkim_aligned_pass_messages: i64,
    no_aligned_dkim_messages: i64,
    envelope_from: Option<String>,
    dkim_selectors: Option<String>,
    reporters: Option<String>,
    report_count: i64,
    first_seen: i64,
    last_seen: i64,
}

impl From<SenderExportSql> for SenderExportRow {
    fn from(r: SenderExportSql) -> Self {
        Self {
            organization: r.organization,
            domain: r.domain,
            source_ip: r.source_ip,
            blocked_messages: r.mail.blocked(),
            mail: r.mail,
            spf_aligned_pass_messages: r.spf_aligned_pass_messages,
            dkim_aligned_pass_messages: r.dkim_aligned_pass_messages,
            no_aligned_dkim_messages: r.no_aligned_dkim_messages,
            envelope_from: sorted_list(r.envelope_from.as_deref()),
            dkim_selectors: sorted_list(r.dkim_selectors.as_deref()),
            reporters: sorted_list(r.reporters.as_deref()),
            report_count: r.report_count,
            first_seen: r.first_seen,
            last_seen: r.last_seen,
        }
    }
}

/// Distinct source IPs of [`stream_sender_export`] for the same scope, so the
/// caller can name them before streaming.
pub async fn sender_export_ips(pool: &SqlitePool, scope: Scope) -> Result<Vec<String>, StoreError> {
    let mut qb = QueryBuilder::new(
        r#"
        SELECT DISTINCT rec.source_ip
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE 1=1
        "#,
    );
    scope.push_reports(&mut qb, "r");
    Ok(qb.build_query_scalar::<String>().fetch_all(pool).await?)
}

/// Every sender IP × domain in scope (all columns scoped the same way), by
/// organization, domain, then busiest sender first.
pub async fn stream_sender_export<F, Fut>(
    pool: &SqlitePool,
    scope: Scope,
    mut on_row: F,
) -> Result<(), StoreError>
where
    F: FnMut(SenderExportRow) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT
            g.organization, g.domain, g.source_ip,
            g.total_messages, g.compliant_messages, g.delivered_failures, g.forwarded_failures,
            g.spf_aligned_pass_messages, g.dkim_aligned_pass_messages,
            g.no_aligned_dkim_messages, g.envelope_from,
            sel.selectors AS dkim_selectors,
            g.reporters, g.report_count, g.first_seen, g.last_seen
        FROM (
            SELECT
                r.domain_id AS domain_id,
                org.name AS organization,
                d.domain AS domain,
                rec.source_ip AS source_ip,
                {MAIL_COUNTS_SELECT},
                {SPF_PASS_MESSAGES_EXPR} AS spf_aligned_pass_messages,
                {DKIM_PASS_MESSAGES_EXPR} AS dkim_aligned_pass_messages,
                {NO_ALIGNED_DKIM_MESSAGES_EXPR} AS no_aligned_dkim_messages,
                json_group_array(DISTINCT rec.envelope_from) AS envelope_from,
                json_group_array(DISTINCT r.org_name) AS reporters,
                COUNT(DISTINCT r.id) AS report_count,
                MIN(r.date_begin) AS first_seen,
                MAX(r.date_end) AS last_seen
            FROM records rec
            INNER JOIN reports r ON r.id = rec.report_id
            INNER JOIN domains d ON d.id = r.domain_id
            INNER JOIN organizations org ON org.id = d.organization_id
            WHERE 1=1
        "#
    ));
    scope.push_reports(&mut qb, "r");
    qb.push(
        r#"
            GROUP BY r.domain_id, rec.source_ip
        ) g
        LEFT JOIN (
            SELECT
                r.domain_id AS domain_id,
                rec.source_ip AS source_ip,
                json_group_array(DISTINCT da.selector) AS selectors
            FROM record_dkim_auth da
            INNER JOIN records rec ON rec.id = da.record_id
            INNER JOIN reports r ON r.id = rec.report_id
            WHERE da.selector IS NOT NULL AND da.selector != ''
        "#,
    );
    scope.push_reports(&mut qb, "r");
    qb.push(
        r#"
            GROUP BY r.domain_id, rec.source_ip
        ) sel ON sel.domain_id = g.domain_id AND sel.source_ip = g.source_ip
        ORDER BY g.organization COLLATE NOCASE, g.domain COLLATE NOCASE,
                 g.total_messages DESC, g.source_ip
        "#,
    );
    for_each_row(pool, qb, |row: SenderExportSql| {
        on_row(SenderExportRow::from(row))
    })
    .await
}

/// One record of `auth-issues.csv`: SPF and/or DKIM lacks an aligned pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthIssueExportRow {
    /// The reporter's `report_id` (not the internal primary key).
    pub report_id: String,
    /// Internal `records.id`, stable across exports.
    pub record_id: i64,
    pub date_begin: i64,
    pub date_end: i64,
    pub reporter: String,
    pub organization: String,
    pub domain: String,
    pub source_ip: String,
    pub messages: i64,
    pub dmarc_pass: bool,
    pub spf_aligned_pass: bool,
    pub dkim_aligned_pass: bool,
    /// The Failures tab's Outcome column.
    pub outcome: Outcome,
    /// The receiver delivered it (no disposition counts as delivered), as the SQL says.
    pub delivered: bool,
    pub disposition: Option<String>,
    pub header_from: Option<String>,
    pub envelope_from: Option<String>,
    pub spf_domains: Vec<String>,
    pub dkim_domains: Vec<String>,
    pub dkim_selectors: Vec<String>,
    /// `policy_evaluated` SPF verdict as the reporter stated it.
    pub reporter_spf_verdict: Option<String>,
    /// `policy_evaluated` DKIM verdict as the reporter stated it.
    pub reporter_dkim_verdict: Option<String>,
    /// `reason_type: comment` per override reason.
    pub policy_overrides: Vec<String>,
}

#[derive(Debug, FromRow)]
struct AuthIssueExportSql {
    report_id: String,
    record_id: i64,
    date_begin: i64,
    date_end: i64,
    reporter: String,
    organization: String,
    domain: String,
    source_ip: String,
    messages: i64,
    dmarc_pass: bool,
    spf_aligned_pass: bool,
    dkim_aligned_pass: bool,
    #[sqlx(try_from = "String")]
    outcome: Outcome,
    delivered: bool,
    disposition: Option<String>,
    header_from: Option<String>,
    envelope_from: Option<String>,
    spf_domains: Option<String>,
    dkim_domains: Option<String>,
    dkim_selectors: Option<String>,
    reporter_spf_verdict: Option<String>,
    reporter_dkim_verdict: Option<String>,
    policy_overrides: Option<String>,
}

impl From<AuthIssueExportSql> for AuthIssueExportRow {
    fn from(r: AuthIssueExportSql) -> Self {
        Self {
            report_id: r.report_id,
            record_id: r.record_id,
            date_begin: r.date_begin,
            date_end: r.date_end,
            reporter: r.reporter,
            organization: r.organization,
            domain: r.domain,
            source_ip: r.source_ip,
            messages: r.messages,
            dmarc_pass: r.dmarc_pass,
            spf_aligned_pass: r.spf_aligned_pass,
            dkim_aligned_pass: r.dkim_aligned_pass,
            outcome: r.outcome,
            delivered: r.delivered,
            disposition: r.disposition,
            header_from: r.header_from,
            envelope_from: r.envelope_from,
            spf_domains: ordered_list(r.spf_domains.as_deref()),
            dkim_domains: ordered_list(r.dkim_domains.as_deref()),
            dkim_selectors: ordered_list(r.dkim_selectors.as_deref()),
            reporter_spf_verdict: r.reporter_spf_verdict,
            reporter_dkim_verdict: r.reporter_dkim_verdict,
            policy_overrides: ordered_list(r.policy_overrides.as_deref()),
        }
    }
}

/// The records behind the Failures tab (same predicate and filters), one row
/// per record, newest first. Per-record child lists are correlated subqueries,
/// so they cannot fan out `messages`.
pub async fn stream_auth_issue_export<F, Fut>(
    pool: &SqlitePool,
    scope: Scope,
    filter: &DomainFailureFilter<'_>,
    mut on_row: F,
) -> Result<(), StoreError>
where
    F: FnMut(AuthIssueExportRow) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT
            r.report_id AS report_id,
            rec.id AS record_id,
            r.date_begin AS date_begin,
            r.date_end AS date_end,
            r.org_name AS reporter,
            org.name AS organization,
            d.domain AS domain,
            rec.source_ip AS source_ip,
            rec.count AS messages,
            rec.dmarc_pass AS dmarc_pass,
            rec.spf_aligned_pass AS spf_aligned_pass,
            rec.dkim_aligned_pass AS dkim_aligned_pass,
            {OUTCOME_EXPR} AS outcome,
            {DELIVERED_EXPR} AS delivered,
            rec.disposition AS disposition,
            rec.header_from AS header_from,
            rec.envelope_from AS envelope_from,
            (SELECT json_group_array(a.domain) FROM record_spf_auth a
             WHERE a.record_id = rec.id) AS spf_domains,
            (SELECT json_group_array(a.domain) FROM record_dkim_auth a
             WHERE a.record_id = rec.id) AS dkim_domains,
            (SELECT json_group_array(a.selector) FROM record_dkim_auth a
             WHERE a.record_id = rec.id) AS dkim_selectors,
            rec.spf_result AS reporter_spf_verdict,
            rec.dkim_result AS reporter_dkim_verdict,
            (SELECT json_group_array(
                        COALESCE(o.reason_type, '') ||
                        CASE WHEN o.comment IS NOT NULL AND o.comment != ''
                             THEN ': ' || o.comment ELSE '' END)
             FROM record_overrides o
             WHERE o.record_id = rec.id) AS policy_overrides
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        INNER JOIN domains d ON d.id = r.domain_id
        INNER JOIN organizations org ON org.id = d.organization_id
        WHERE 1=1
        "#
    ));
    scope.push_reports(&mut qb, "r");
    filter.push_predicate(&mut qb);
    qb.push(" ORDER BY r.date_begin DESC, rec.id DESC");
    for_each_row(pool, qb, |row: AuthIssueExportSql| {
        on_row(AuthIssueExportRow::from(row))
    })
    .await
}

/// A stored DNS snapshot with its domain's organization, for `dns-findings.csv`.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct DnsSnapshotExportRow {
    pub organization: String,
    pub domain: String,
    pub checked_at: i64,
    pub grade: String,
    pub score: i64,
    /// The serialized `DnsReport`; the server extracts its findings.
    pub report_json: String,
}

/// Stored DNS snapshots of the monitored domains in scope (no live DNS; the
/// time window does not apply). Domains that were never graded have no rows.
pub async fn stream_dns_snapshot_export<F, Fut>(
    pool: &SqlitePool,
    scope: Scope,
    on_row: F,
) -> Result<(), StoreError>
where
    F: FnMut(DnsSnapshotExportRow) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut qb = QueryBuilder::new(
        r#"
        SELECT org.name AS organization, d.domain AS domain, s.checked_at AS checked_at,
               s.grade AS grade, s.score AS score, s.report_json AS report_json
        FROM domains d
        INNER JOIN organizations org ON org.id = d.organization_id
        INNER JOIN dns_snapshots s ON s.domain_id = d.id
        WHERE 1=1
        "#,
    );
    scope.push_domains(&mut qb, "d");
    qb.push(" ORDER BY org.name COLLATE NOCASE, d.domain COLLATE NOCASE");
    for_each_row(pool, qb, on_row).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_are_trimmed_deduped_and_null_free() {
        assert_eq!(
            sorted_list(Some(r#"["b.example"," a.example","b.example","",null]"#)),
            vec!["a.example", "b.example"]
        );
        assert_eq!(
            sorted_list(Some(r#"["Acme, Inc","Google"]"#)),
            vec!["Acme, Inc", "Google"]
        );
        assert!(sorted_list(None).is_empty());
        assert!(sorted_list(Some("not json")).is_empty());
        assert_eq!(
            ordered_list(Some(r#"["z.example","a.example","z.example"]"#)),
            vec!["z.example", "a.example"]
        );
    }
}
