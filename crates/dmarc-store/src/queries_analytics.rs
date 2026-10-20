//! Analytics queries: timeseries, scope statistics, cross-domain senders.
//!
//! Totals are always taken from `records.count`. Child auth tables are never
//! joined when summing `count`, so a record with several DKIM results cannot
//! fan out.
//!
//! `records.dmarc_pass` is the single source of truth for "compliant" across the
//! whole product: it is the alignment verdict our own parser derives with the
//! Public Suffix List, not the reporter's `policy_evaluated` opinion. Never define
//! compliance as `dkim_result = 'pass' OR spf_result = 'pass'` — that is
//! alignment-blind and drifts from every other screen.

use crate::outcome::{
    MailCounts, DKIM_PASS_MESSAGES_EXPR, MAIL_COUNTS_SELECT, SPF_PASS_MESSAGES_EXPR,
};
use crate::queries::Scope;
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use sqlx::{FromRow, QueryBuilder};
use std::collections::HashMap;
use ts_rs::TS;

const DAY_SECS: i64 = 86_400;
const WEEK_SECS: i64 = 7 * DAY_SECS;
/// Cap so a missing `since` (or a 1970 timestamp) cannot emit tens of thousands of zeros.
const MAX_BUCKETS: i64 = 400;

/// UTC day or Monday-start UTC week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeseriesBucket {
    Day,
    Week,
}

impl TimeseriesBucket {
    pub fn step_secs(self) -> i64 {
        match self {
            Self::Day => DAY_SECS,
            Self::Week => WEEK_SECS,
        }
    }
}

/// One timeseries bucket. Empty days/weeks are filled with zeros by [`list_timeseries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow, Serialize, TS)]
pub struct TimeseriesRow {
    pub ts: i64,
    #[sqlx(flatten)]
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
}

impl TimeseriesRow {
    fn zero(ts: i64) -> Self {
        Self {
            ts,
            mail: MailCounts::default(),
        }
    }
}

/// Floor `ts` to a UTC day or Monday 00:00 UTC week.
fn bucket_start(ts: i64, bucket: TimeseriesBucket) -> i64 {
    match bucket {
        TimeseriesBucket::Day => ts.div_euclid(DAY_SECS) * DAY_SECS,
        TimeseriesBucket::Week => {
            let day = ts.div_euclid(DAY_SECS);
            // Unix day 0 is Thursday; Monday is 3 days earlier.
            let days_since_monday = (day + 3).rem_euclid(7);
            (day - days_since_monday) * DAY_SECS
        }
    }
}

/// Daily (then optionally weekly) message volume, bucketed on `reports.date_begin`.
///
/// A report that spans midnight lands in exactly one bucket (its `date_begin`).
/// Days/weeks with no reports are present as zero-value points so a reporting
/// outage is visible on the chart.
pub async fn list_timeseries(
    pool: &SqlitePool,
    scope: Scope,
    bucket: TimeseriesBucket,
    now: i64,
) -> Result<Vec<TimeseriesRow>, StoreError> {
    let daily = fetch_daily_timeseries(pool, scope, None).await?;
    let series = match bucket {
        TimeseriesBucket::Day => daily,
        TimeseriesBucket::Week => fold_weeks(daily),
    };

    let (start, end) = gap_window(&series, scope.since, now, bucket);
    Ok(fill_gaps(series, start, end, bucket.step_secs()))
}

async fn fetch_daily_timeseries(
    pool: &SqlitePool,
    scope: Scope,
    source_ip: Option<&str>,
) -> Result<Vec<TimeseriesRow>, StoreError> {
    // Never join auth tables here: see the module docs.
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT (r.date_begin / 86400) * 86400 AS ts, {MAIL_COUNTS_SELECT}
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE 1=1
        "#
    ));
    scope.push_reports(&mut qb, "r");
    if let Some(ip) = source_ip {
        qb.push(" AND rec.source_ip = ").push_bind(ip);
    }
    qb.push(" GROUP BY ts ORDER BY ts");

    let rows = qb.build_query_as::<TimeseriesRow>().fetch_all(pool).await?;
    Ok(rows)
}

fn fold_weeks(daily: Vec<TimeseriesRow>) -> Vec<TimeseriesRow> {
    let mut weeks: HashMap<i64, TimeseriesRow> = HashMap::new();
    for point in daily {
        let ts = bucket_start(point.ts, TimeseriesBucket::Week);
        weeks
            .entry(ts)
            .or_insert_with(|| TimeseriesRow::zero(ts))
            .mail
            .add(&point.mail);
    }
    let mut out: Vec<TimeseriesRow> = weeks.into_values().collect();
    out.sort_by_key(|p| p.ts);
    out
}

fn gap_window(
    series: &[TimeseriesRow],
    since: Option<i64>,
    now: i64,
    bucket: TimeseriesBucket,
) -> (Option<i64>, Option<i64>) {
    let step = bucket.step_secs();
    let end = bucket_start(now, bucket);
    if let Some(since) = since {
        let mut start = bucket_start(since, bucket);
        let max_span = (MAX_BUCKETS - 1) * step;
        if end.saturating_sub(start) > max_span {
            start = end.saturating_sub(max_span);
            start = bucket_start(start, bucket);
        }
        return (Some(start), Some(end));
    }
    if series.is_empty() {
        return (None, None);
    }
    let data_min = series.iter().map(|p| p.ts).min().unwrap_or(end);
    let data_max = series.iter().map(|p| p.ts).max().unwrap_or(end);
    let mut start = data_min;
    let max_span = (MAX_BUCKETS - 1) * step;
    if data_max.saturating_sub(start) > max_span {
        start = data_max.saturating_sub(max_span);
        start = bucket_start(start, bucket);
    }
    (Some(start), Some(data_max))
}

fn fill_gaps(
    series: Vec<TimeseriesRow>,
    start: Option<i64>,
    end: Option<i64>,
    step: i64,
) -> Vec<TimeseriesRow> {
    let (Some(start), Some(end)) = (start, end) else {
        return series;
    };
    if step <= 0 {
        return series;
    }
    let mut by_ts: HashMap<i64, TimeseriesRow> = series.into_iter().map(|p| (p.ts, p)).collect();
    let mut out = Vec::new();
    let mut ts = start;
    while ts <= end {
        out.push(by_ts.remove(&ts).unwrap_or_else(|| TimeseriesRow::zero(ts)));
        let next = ts.saturating_add(step);
        if next <= ts {
            break;
        }
        ts = next;
    }
    out
}

/// Message, report, domain and sender tallies of a scope, in one query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, FromRow, Serialize, TS)]
pub struct ScopeStats {
    #[sqlx(flatten)]
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    pub report_count: i64,
    pub active_domains: i64,
    pub unique_senders: i64,
}

/// Count messages by outcome, reports, active domains and distinct senders
/// for a scope. The one definition of these numbers; callers must not
/// re-derive them. The LEFT JOIN keeps a report with no records visible in
/// `report_count`.
pub async fn scope_stats(pool: &SqlitePool, scope: Scope) -> Result<ScopeStats, StoreError> {
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT {MAIL_COUNTS_SELECT},
            COUNT(DISTINCT r.id) AS report_count,
            COUNT(DISTINCT r.domain_id) AS active_domains,
            COUNT(DISTINCT rec.source_ip) AS unique_senders
        FROM reports r
        LEFT JOIN records rec ON rec.report_id = r.id
        WHERE 1=1
        "#
    ));
    scope.push_reports(&mut qb, "r");
    Ok(qb.build_query_as::<ScopeStats>().fetch_one(pool).await?)
}

/// `date_begin` of the scope's oldest and newest report, whatever its time
/// window; `(None, None)` without reports.
pub async fn report_span(
    pool: &SqlitePool,
    scope: Scope,
) -> Result<(Option<i64>, Option<i64>), StoreError> {
    let mut qb =
        QueryBuilder::new("SELECT MIN(r.date_begin), MAX(r.date_begin) FROM reports r WHERE 1=1");
    scope.window(None, None).push_reports(&mut qb, "r");
    Ok(qb.build_query_as().fetch_one(pool).await?)
}

/// Cross-domain view of one source IP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
pub struct SenderDetailRow {
    pub source_ip: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    pub first_seen: i64,
    pub last_seen: i64,
    pub domain_count: i64,
    /// Per-domain breakdown; not sent as stored: the server sends each row judged.
    #[serde(skip)]
    #[ts(skip)]
    pub items: Vec<SenderDomainStatsRow>,
    /// Not sent as stored: the server adds each bucket's blocked count.
    #[serde(skip)]
    #[ts(skip)]
    pub timeseries: Vec<TimeseriesRow>,
}

/// Messages to one recipient domain (`envelope_to`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderTargetCount {
    pub target: String,
    pub count: i64,
}

/// Messages one reporter saw, by recipient domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SenderReceiverBreakdown {
    pub reporter: String,
    pub total_messages: i64,
    pub targets: Vec<SenderTargetCount>,
}

/// A domain SPF or DKIM passed for, other than the one the row is about, and
/// how many of the row's messages passed only that way.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct OtherDomainPass {
    pub domain_id: i64,
    pub domain: String,
    pub messages: i64,
    /// SPF only: checked against the sending server's name (`helo` scope),
    /// as receivers do for bounces without an envelope sender.
    pub helo: bool,
}

/// One sending domain of a source IP: the stored figures, nothing judged.
/// `#[sqlx(skip)]` fields come from follow-up queries.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow, Serialize, TS)]
pub struct SenderDomainStatsRow {
    pub domain_id: i64,
    pub domain: String,
    #[sqlx(flatten)]
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    /// Messages with an aligned SPF pass (same definition as the senders list).
    pub spf_pass_messages: i64,
    /// Messages with an aligned DKIM pass.
    pub dkim_pass_messages: i64,
    pub quarantined_messages: i64,
    pub rejected_messages: i64,
    /// Distinct envelope senders, comma-separated and sorted.
    pub envelope_from: Option<String>,
    /// Distinct DKIM selectors, comma-separated and sorted.
    pub dkim_selectors: Option<String>,
    /// Messages without an aligned SPF pass whose SPF passed for another domain.
    pub spf_other_only_messages: i64,
    /// Messages without an aligned DKIM pass with a passing signature of another domain.
    pub dkim_other_only_messages: i64,
    /// Messages that carried no DKIM signature at all.
    pub dkim_unsigned_messages: i64,
    /// Where SPF passed instead, busiest first. A message may count under
    /// more than one domain.
    #[sqlx(skip)]
    pub spf_other_passes: Vec<OtherDomainPass>,
    /// Domains whose DKIM signature passed instead, busiest first.
    #[sqlx(skip)]
    pub dkim_other_passes: Vec<OtherDomainPass>,
    #[sqlx(skip)]
    pub targets: Vec<SenderTargetCount>,
    #[sqlx(skip)]
    pub receivers: Vec<SenderReceiverBreakdown>,
    pub first_seen: i64,
    pub last_seen: i64,
}

#[derive(Debug, FromRow)]
struct SenderHeaderSql {
    #[sqlx(flatten)]
    mail: MailCounts,
    first_seen: Option<i64>,
    last_seen: Option<i64>,
    domain_count: i64,
}

/// Resolve a stored `source_ip` from one or more textual forms (raw path + canonical).
pub async fn resolve_source_ip(
    pool: &SqlitePool,
    candidates: &[String],
    scope: Scope,
) -> Result<Option<String>, StoreError> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut qb = QueryBuilder::new(
        r#"
        SELECT rec.source_ip
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE rec.source_ip IN (
        "#,
    );
    let mut sep = qb.separated(", ");
    for ip in candidates {
        sep.push_bind(ip);
    }
    qb.push(")");
    scope.push_reports(&mut qb, "r");
    qb.push(" LIMIT 1");
    let row: Option<(String,)> = qb.build_query_as().fetch_optional(pool).await?;
    Ok(row.map(|r| r.0))
}

/// Cross-domain sender detail. `None` when the IP has no records in scope (caller maps to 404).
pub async fn get_sender_by_ip(
    pool: &SqlitePool,
    source_ip: &str,
    scope: Scope,
    now: i64,
) -> Result<Option<SenderDetailRow>, StoreError> {
    let header = fetch_sender_header(pool, source_ip, scope).await?;
    let (Some(first_seen), Some(last_seen)) = (header.first_seen, header.last_seen) else {
        return Ok(None);
    };

    let items = fetch_sender_domains(pool, source_ip, scope).await?;
    let daily = fetch_daily_timeseries(pool, scope, Some(source_ip)).await?;
    let (start, end) = gap_window(&daily, scope.since, now, TimeseriesBucket::Day);
    // When `since` is absent, still show the IP's own first→last span (already in `daily`).
    let timeseries = fill_gaps(
        daily,
        start.or_else(|| Some(bucket_start(first_seen, TimeseriesBucket::Day))),
        end.or_else(|| Some(bucket_start(last_seen, TimeseriesBucket::Day))),
        DAY_SECS,
    );

    Ok(Some(SenderDetailRow {
        source_ip: source_ip.to_owned(),
        mail: header.mail,
        first_seen,
        last_seen,
        domain_count: header.domain_count,
        items,
        timeseries,
    }))
}

async fn fetch_sender_header(
    pool: &SqlitePool,
    source_ip: &str,
    scope: Scope,
) -> Result<SenderHeaderSql, StoreError> {
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT {MAIL_COUNTS_SELECT},
            MIN(r.date_begin) AS first_seen,
            MAX(r.date_end) AS last_seen,
            COUNT(DISTINCT r.domain_id) AS domain_count
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE rec.source_ip =
        "#
    ));
    qb.push_bind(source_ip);
    scope.push_reports(&mut qb, "r");
    let row = qb
        .build_query_as::<SenderHeaderSql>()
        .fetch_one(pool)
        .await?;
    Ok(row)
}

fn clean_comma_list(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let mut parts: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    parts.sort_unstable();
    parts.dedup();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

#[derive(Debug, FromRow)]
struct SenderReceiverSql {
    domain_id: i64,
    reporter: String,
    target: String,
    count: i64,
}

/// Count-descending, then target-ascending.
fn sorted_targets(counts: HashMap<String, i64>) -> Vec<SenderTargetCount> {
    let mut targets: Vec<SenderTargetCount> = counts
        .into_iter()
        .map(|(target, count)| SenderTargetCount { target, count })
        .collect();
    targets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.target.cmp(&b.target)));
    targets
}

#[derive(Debug, Clone, Copy)]
enum OtherPassKind {
    Spf,
    Dkim,
}

/// Per sending domain of `source_ip`: the other domains SPF (or DKIM) passed
/// for on messages without an aligned pass, with message counts. A domain that
/// passed twice in one record (e.g. `mfrom` and `helo`) counts that record once.
async fn fetch_other_passes(
    pool: &SqlitePool,
    kind: OtherPassKind,
    source_ip: &str,
    scope: Scope,
) -> Result<HashMap<i64, Vec<OtherDomainPass>>, StoreError> {
    let (table, helo, aligned_pass) = match kind {
        OtherPassKind::Spf => (
            "record_spf_auth",
            "MIN(COALESCE(scope, '') = 'helo')",
            "rec.spf_aligned_pass",
        ),
        OtherPassKind::Dkim => ("record_dkim_auth", "0", "rec.dkim_aligned_pass"),
    };
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT r.domain_id AS domain_id, a.domain AS domain,
               COALESCE(SUM(rec.count), 0) AS messages, MIN(a.helo) AS helo
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        INNER JOIN (
            SELECT record_id, LOWER(domain) AS domain, {helo} AS helo
            FROM {table}
            WHERE result = 'pass' AND aligned = 0 AND domain IS NOT NULL AND domain != ''
              AND record_id IN (SELECT id FROM records WHERE source_ip =
        "#
    ));
    // Restricting the derived table to this IP's records keeps it from
    // aggregating the whole auth table (it uses the partial pass index).
    qb.push_bind(source_ip);
    qb.push(format!(
        r#")
            GROUP BY record_id, LOWER(domain)
        ) a ON a.record_id = rec.id
        WHERE {aligned_pass} = 0 AND rec.source_ip = "#
    ));
    qb.push_bind(source_ip);
    scope.push_reports(&mut qb, "r");
    qb.push(" GROUP BY r.domain_id, a.domain ORDER BY messages DESC, a.domain");
    let rows = qb
        .build_query_as::<OtherDomainPass>()
        .fetch_all(pool)
        .await?;
    let mut out: HashMap<i64, Vec<OtherDomainPass>> = HashMap::new();
    for row in rows {
        out.entry(row.domain_id).or_default().push(row);
    }
    Ok(out)
}

async fn fetch_sender_domains(
    pool: &SqlitePool,
    source_ip: &str,
    scope: Scope,
) -> Result<Vec<SenderDomainStatsRow>, StoreError> {
    let mut qb = QueryBuilder::new(format!(
        r#"
        SELECT
            d.id AS domain_id,
            d.domain AS domain,
            {MAIL_COUNTS_SELECT},
            {SPF_PASS_MESSAGES_EXPR} AS spf_pass_messages,
            {DKIM_PASS_MESSAGES_EXPR} AS dkim_pass_messages,
            COALESCE(SUM(
                CASE WHEN rec.disposition = 'quarantine' THEN rec.count ELSE 0 END
            ), 0) AS quarantined_messages,
            COALESCE(SUM(
                CASE WHEN rec.disposition = 'reject' THEN rec.count ELSE 0 END
            ), 0) AS rejected_messages,
            GROUP_CONCAT(DISTINCT rec.envelope_from) AS envelope_from,
            GROUP_CONCAT(DISTINCT (
                SELECT GROUP_CONCAT(DISTINCT x.selector) FROM record_dkim_auth x
                WHERE x.record_id = rec.id AND x.selector IS NOT NULL AND x.selector != ''
            )) AS dkim_selectors,
            COALESCE(SUM(CASE WHEN rec.spf_aligned_pass = 0 AND EXISTS (
                SELECT 1 FROM record_spf_auth a WHERE a.record_id = rec.id
                AND a.result = 'pass' AND a.aligned = 0) THEN rec.count ELSE 0 END), 0)
                AS spf_other_only_messages,
            COALESCE(SUM(CASE WHEN rec.dkim_aligned_pass = 0 AND EXISTS (
                SELECT 1 FROM record_dkim_auth a WHERE a.record_id = rec.id
                AND a.result = 'pass' AND a.aligned = 0) THEN rec.count ELSE 0 END), 0)
                AS dkim_other_only_messages,
            COALESCE(SUM(CASE WHEN NOT EXISTS (
                SELECT 1 FROM record_dkim_auth a WHERE a.record_id = rec.id)
                THEN rec.count ELSE 0 END), 0) AS dkim_unsigned_messages,
            MIN(r.date_begin) AS first_seen,
            MAX(r.date_end) AS last_seen
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        INNER JOIN domains d ON d.id = r.domain_id
        WHERE rec.source_ip = 
        "#
    ));
    qb.push_bind(source_ip);
    scope.push_reports(&mut qb, "r");
    qb.push(" GROUP BY d.id, d.domain ORDER BY total_messages DESC, d.domain COLLATE NOCASE");
    let mut rows = qb
        .build_query_as::<SenderDomainStatsRow>()
        .fetch_all(pool)
        .await?;
    let mut spf_other = fetch_other_passes(pool, OtherPassKind::Spf, source_ip, scope).await?;
    let mut dkim_other = fetch_other_passes(pool, OtherPassKind::Dkim, source_ip, scope).await?;

    // One (domain, reporter, envelope_to) breakdown feeds both the per-reporter
    // receivers and the domain-level target totals.
    let mut receiver_qb = QueryBuilder::new(
        r#"
        SELECT
            r.domain_id,
            COALESCE(r.org_name, 'Unknown') AS reporter,
            COALESCE(rec.envelope_to, '') AS target,
            COALESCE(SUM(rec.count), 0) AS count
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE rec.source_ip = 
        "#,
    );
    receiver_qb.push_bind(source_ip);
    scope.push_reports(&mut receiver_qb, "r");
    receiver_qb.push(" GROUP BY r.domain_id, r.org_name, rec.envelope_to");
    let receiver_rows = receiver_qb
        .build_query_as::<SenderReceiverSql>()
        .fetch_all(pool)
        .await?;

    #[derive(Default)]
    struct ReporterAcc {
        total_messages: i64,
        target_counts: HashMap<String, i64>,
    }

    let mut targets_map: HashMap<i64, HashMap<String, i64>> = HashMap::new();
    let mut receivers_map: HashMap<i64, HashMap<String, ReporterAcc>> = HashMap::new();
    for row in receiver_rows {
        if !row.target.is_empty() {
            *targets_map
                .entry(row.domain_id)
                .or_default()
                .entry(row.target.clone())
                .or_default() += row.count;
        }
        let reporter = if row.reporter.trim().is_empty() {
            "Unknown".to_string()
        } else {
            row.reporter
        };
        let domain_entry = receivers_map.entry(row.domain_id).or_default();
        let rep_entry = domain_entry.entry(reporter).or_default();
        rep_entry.total_messages += row.count;
        let target = row.target.trim();
        if !target.is_empty() {
            *rep_entry
                .target_counts
                .entry(target.to_string())
                .or_default() += row.count;
        }
    }

    for row in &mut rows {
        row.envelope_from = clean_comma_list(row.envelope_from.take());
        row.dkim_selectors = clean_comma_list(row.dkim_selectors.take());
        row.spf_other_passes = spf_other.remove(&row.domain_id).unwrap_or_default();
        row.dkim_other_passes = dkim_other.remove(&row.domain_id).unwrap_or_default();
        row.targets = sorted_targets(targets_map.remove(&row.domain_id).unwrap_or_default());
        let mut receivers: Vec<SenderReceiverBreakdown> = receivers_map
            .remove(&row.domain_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(reporter, acc)| SenderReceiverBreakdown {
                reporter,
                total_messages: acc.total_messages,
                targets: sorted_targets(acc.target_counts),
            })
            .collect();
        receivers.sort_by(|a, b| {
            b.total_messages
                .cmp(&a.total_messages)
                .then_with(|| a.reporter.cmp(&b.reporter))
        });
        row.receivers = receivers;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(ts: i64, total: i64, compliant: i64, delivered: i64) -> TimeseriesRow {
        TimeseriesRow {
            ts,
            mail: MailCounts {
                total_messages: total,
                compliant_messages: compliant,
                delivered_failures: delivered,
                forwarded_failures: 0,
            },
        }
    }

    #[test]
    fn bucket_start_day_and_monday_week() {
        // 2024-01-01 00:00:00 UTC is a Monday.
        let monday = 1_704_067_200;
        assert_eq!(bucket_start(monday + 3_600, TimeseriesBucket::Day), monday);
        assert_eq!(bucket_start(monday + 3_600, TimeseriesBucket::Week), monday);
        // Wednesday still belongs to that Monday week.
        let wed = monday + 2 * DAY_SECS + 1;
        assert_eq!(bucket_start(wed, TimeseriesBucket::Week), monday);
        // Next Monday is its own week.
        assert_eq!(
            bucket_start(monday + WEEK_SECS, TimeseriesBucket::Week),
            monday + WEEK_SECS
        );
    }

    #[test]
    fn fill_gaps_inserts_zero_days() {
        let monday = 1_704_067_200;
        let series = vec![
            point(monday, 10, 8, 1),
            point(monday + 2 * DAY_SECS, 5, 5, 0),
        ];
        let filled = fill_gaps(series, Some(monday), Some(monday + 2 * DAY_SECS), DAY_SECS);
        assert_eq!(filled.len(), 3);
        assert_eq!(filled[1].ts, monday + DAY_SECS);
        assert_eq!(filled[1].mail, MailCounts::default());
        assert_eq!(filled[0].mail.total_messages, 10);
        assert_eq!(filled[0].mail.delivered_failures, 1);
        assert_eq!(filled[2].mail.total_messages, 5);
    }

    #[test]
    fn fold_weeks_sums_days() {
        let monday = 1_704_067_200;
        let weeks = fold_weeks(vec![
            point(monday, 10, 8, 1),
            point(monday + DAY_SECS, 5, 5, 0),
        ]);
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].ts, monday);
        assert_eq!(weeks[0].mail.total_messages, 15);
        assert_eq!(weeks[0].mail.compliant_messages, 13);
    }
}
