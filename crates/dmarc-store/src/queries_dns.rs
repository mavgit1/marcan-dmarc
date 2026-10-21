//! Persisted live-DNS results. Pages read these; the checker writes them.

use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;

/// One domain's stored DNS check. Keyed by the domain's id in the table; the
/// name is joined from `domains`.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow)]
pub struct DnsSnapshot {
    pub domain: String,
    pub checked_at: i64,
    /// The check's grade, score and critical findings, cached with it (valid
    /// for its `checker_version`).
    pub grade: String,
    pub score: i64,
    pub critical_count: i64,
    /// The DMARC record the check found, as columns the queries read.
    #[sqlx(flatten)]
    pub dmarc: SnapshotDmarc,
    /// The serialized check; only the server reads it, as a whole.
    pub report_json: String,
    /// Checking/scoring logic version that produced this row.
    pub checker_version: i64,
}

/// The DMARC record a stored check found: what the domain list, its sort, the
/// overview and policy readiness read the policy from, written with the
/// snapshot. The tags are the parsed record's, `None` without a record.
#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow)]
pub struct SnapshotDmarc {
    /// DMARC records published at `_dmarc` (0: none, more than 1: invalid).
    pub dmarc_record_count: i64,
    /// The record as published when there is exactly one.
    pub dmarc_record: Option<String>,
    pub policy_p: Option<String>,
    pub policy_sp: Option<String>,
    pub policy_pct: Option<i64>,
    pub policy_adkim: Option<String>,
    pub policy_aspf: Option<String>,
    /// `t=y` (RFC 9989): `Some(true)` when testing, `Some(false)` for any
    /// other record, `None` without one.
    pub policy_testing: Option<bool>,
    /// `np=` (RFC 9989), as parsed.
    pub policy_np: Option<String>,
}

/// The columns of [`DnsSnapshot`] over `dns_snapshots s` joined to `domains d`.
const SNAPSHOT_COLUMNS: &str = "d.domain, s.checked_at, s.grade, s.score, s.critical_count, \
     s.dmarc_record_count, s.dmarc_record, s.policy_p, s.policy_sp, s.policy_pct, \
     s.policy_adkim, s.policy_aspf, s.policy_testing, s.policy_np, s.report_json, \
     s.checker_version";

/// Whether a snapshot is still fresh enough to skip a live re-check.
///
/// A version mismatch is stale even when `checked_at` is recent: after a deploy
/// that changes scoring rules, every domain must be re-graded rather than
/// serving a mix of old and new scores for up to an hour.
pub fn dns_snapshot_is_fresh(
    snap: &DnsSnapshot,
    now: i64,
    fresh_secs: i64,
    current_version: u32,
) -> bool {
    if snap.checker_version != i64::from(current_version) {
        return false;
    }
    now.saturating_sub(snap.checked_at) < fresh_secs
}

/// Insert or replace the snapshot of the domain named `snap.domain`, only
/// while the domain is monitored (known and not ignored): checked and written
/// in one statement, so a domain ignored meanwhile keeps no DNS result.
/// Returns whether a row was written.
pub async fn upsert_dns_snapshot(
    pool: &SqlitePool,
    snap: &DnsSnapshot,
) -> Result<bool, StoreError> {
    let dmarc = &snap.dmarc;
    let done = sqlx::query(
        r#"
        INSERT INTO dns_snapshots
            (domain_id, checked_at, grade, score, critical_count, dmarc_record_count,
             dmarc_record, policy_p, policy_sp, policy_pct, policy_adkim, policy_aspf,
             policy_testing, policy_np, report_json, checker_version)
        SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
        FROM domains WHERE domain = ?1 AND ignored = 0
        ON CONFLICT(domain_id) DO UPDATE SET
            checked_at = excluded.checked_at,
            grade = excluded.grade,
            score = excluded.score,
            critical_count = excluded.critical_count,
            dmarc_record_count = excluded.dmarc_record_count,
            dmarc_record = excluded.dmarc_record,
            policy_p = excluded.policy_p,
            policy_sp = excluded.policy_sp,
            policy_pct = excluded.policy_pct,
            policy_adkim = excluded.policy_adkim,
            policy_aspf = excluded.policy_aspf,
            policy_testing = excluded.policy_testing,
            policy_np = excluded.policy_np,
            report_json = excluded.report_json,
            checker_version = excluded.checker_version
        "#,
    )
    .bind(&snap.domain)
    .bind(snap.checked_at)
    .bind(&snap.grade)
    .bind(snap.score)
    .bind(snap.critical_count)
    .bind(dmarc.dmarc_record_count)
    .bind(&dmarc.dmarc_record)
    .bind(&dmarc.policy_p)
    .bind(&dmarc.policy_sp)
    .bind(dmarc.policy_pct)
    .bind(&dmarc.policy_adkim)
    .bind(&dmarc.policy_aspf)
    .bind(dmarc.policy_testing)
    .bind(&dmarc.policy_np)
    .bind(&snap.report_json)
    .bind(snap.checker_version)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// The stored snapshot of the domain named `domain`.
pub async fn get_dns_snapshot(
    pool: &SqlitePool,
    domain: &str,
) -> Result<Option<DnsSnapshot>, StoreError> {
    let row = sqlx::query_as::<_, DnsSnapshot>(&format!(
        "SELECT {SNAPSHOT_COLUMNS} FROM dns_snapshots s \
         INNER JOIN domains d ON d.id = s.domain_id WHERE d.domain = ?"
    ))
    .bind(domain)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Snapshots for `domains`, keyed by name. Domains without one are absent.
pub async fn get_dns_snapshots(
    pool: &SqlitePool,
    domains: &[String],
) -> Result<std::collections::HashMap<String, DnsSnapshot>, StoreError> {
    let mut out = std::collections::HashMap::with_capacity(domains.len());
    // Well below SQLite's bound-parameter limit.
    for chunk in domains.chunks(500) {
        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(format!(
            "SELECT {SNAPSHOT_COLUMNS} FROM dns_snapshots s \
             INNER JOIN domains d ON d.id = s.domain_id WHERE d.domain IN ("
        ));
        let mut sep = qb.separated(", ");
        for domain in chunk {
            sep.push_bind(domain);
        }
        qb.push(")");
        for snap in qb.build_query_as::<DnsSnapshot>().fetch_all(pool).await? {
            out.insert(snap.domain.clone(), snap);
        }
    }
    Ok(out)
}

/// Names of the visible (not ignored) domains, for the DNS warm loop.
pub async fn list_monitored_domains(pool: &SqlitePool) -> Result<Vec<String>, StoreError> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT domain FROM domains WHERE ignored = 0 ORDER BY domain COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Ids and names of the visible (not ignored) domains, for the alert evaluation.
pub async fn list_monitored_domain_ids(
    pool: &SqlitePool,
) -> Result<Vec<(i64, String)>, StoreError> {
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, domain FROM domains WHERE ignored = 0 ORDER BY domain COLLATE NOCASE",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// How many domains are monitored, and how many of them have a stored DNS
/// result ([`dns_coverage`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, FromRow)]
pub struct DnsCoverage {
    pub domains: i64,
    pub checked: i64,
}

/// Monitored (not ignored) domains and those with a stored DNS result. Cheap:
/// the system health poll reads it every few seconds while work runs.
pub async fn dns_coverage(pool: &SqlitePool) -> Result<DnsCoverage, StoreError> {
    Ok(sqlx::query_as::<_, DnsCoverage>(
        "SELECT COUNT(*) AS domains, COUNT(s.domain_id) AS checked \
         FROM domains d LEFT JOIN dns_snapshots s ON s.domain_id = d.id \
         WHERE d.ignored = 0",
    )
    .fetch_one(pool)
    .await?)
}
