//! Evidence for naming a source IP: persisted reverse DNS and SPF passes.
//!
//! The identity warm loop writes the reverse-DNS rows; pages read them.
//! `ptr_confirmed` is stored as SQLite INTEGER NULL/0/1 so "no PTR to check"
//! stays distinct from "forward confirmation failed".

use crate::queries::{begin_write, VISIBLE_DOMAIN_IDS};
use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IdentitySnapshot {
    pub ip: String,
    pub checked_at: i64,
    pub ptr_hostname: Option<String>,
    /// Tri-state: `None` = no PTR; `Some(true)` / `Some(false)` = confirm result.
    pub ptr_confirmed: Option<bool>,
}

/// Encode tri-state confirmation for SQLite INTEGER NULL/0/1 storage.
pub fn encode_ptr_confirmed(value: Option<bool>) -> Option<i64> {
    value.map(i64::from)
}

/// Decode SQLite INTEGER NULL/0/1 back to tri-state confirmation.
pub fn decode_ptr_confirmed(value: Option<i64>) -> Option<bool> {
    value.map(|v| v != 0)
}

/// Whether a snapshot is still fresh enough to skip re-resolution.
///
/// Positive results (a PTR hostname was found) use `positive_fresh_secs`.
/// Negatives (no PTR) use `negative_fresh_secs`, which should be longer --
/// unresolved names burn the full DNS timeout and are unlikely to appear soon.
pub fn identity_snapshot_is_fresh(
    snap: &IdentitySnapshot,
    now: i64,
    positive_fresh_secs: i64,
    negative_fresh_secs: i64,
) -> bool {
    let age = now.saturating_sub(snap.checked_at);
    let window = if snap.ptr_hostname.is_some() {
        positive_fresh_secs
    } else {
        negative_fresh_secs
    };
    age < window
}

pub async fn upsert_identity_snapshot(
    pool: &SqlitePool,
    ip: &str,
    checked_at: i64,
    ptr_hostname: Option<&str>,
    ptr_confirmed: Option<bool>,
) -> Result<(), StoreError> {
    sqlx::query(
        r#"
        INSERT INTO identity_snapshots
            (ip, checked_at, ptr_hostname, ptr_confirmed)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(ip) DO UPDATE SET
            checked_at = excluded.checked_at,
            ptr_hostname = excluded.ptr_hostname,
            ptr_confirmed = excluded.ptr_confirmed
        "#,
    )
    .bind(ip)
    .bind(checked_at)
    .bind(ptr_hostname)
    .bind(encode_ptr_confirmed(ptr_confirmed))
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete up to `limit` reverse-DNS snapshots of IPs that no stored record
/// names any more, in one write transaction; returns how many went. Every
/// sender page and export starts from records, so only those snapshots can
/// still be shown; an IP that reports again is looked up again.
pub async fn delete_unused_identity_snapshots(
    pool: &SqlitePool,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut tx = begin_write(pool).await?;
    let done = sqlx::query(
        "DELETE FROM identity_snapshots WHERE ip IN \
         (SELECT s.ip FROM identity_snapshots s WHERE NOT EXISTS \
          (SELECT 1 FROM records rec WHERE rec.source_ip = s.ip) LIMIT ?)",
    )
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(done.rows_affected())
}

/// Most IPs bound in one `IN (...)` list, well under SQLite's variable limit.
const IP_CHUNK: usize = 500;

/// `?,?,…` for `n` bind parameters.
fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Snapshots for `ips`. IPs without one are absent from the map.
pub async fn get_identity_snapshots(
    pool: &SqlitePool,
    ips: &[String],
) -> Result<HashMap<String, IdentitySnapshot>, StoreError> {
    let mut out = HashMap::with_capacity(ips.len());
    for chunk in ips.chunks(IP_CHUNK) {
        let sql = format!(
            "SELECT ip, checked_at, ptr_hostname, ptr_confirmed \
             FROM identity_snapshots WHERE ip IN ({})",
            placeholders(chunk.len())
        );
        let mut query = sqlx::query_as::<_, IdentitySnapshotRow>(&sql);
        for ip in chunk {
            query = query.bind(ip);
        }
        for row in query.fetch_all(pool).await? {
            let snap = IdentitySnapshot::from(row);
            out.insert(snap.ip.clone(), snap);
        }
    }
    Ok(out)
}

/// Per IP, the domain whose SPF passed for it in the most messages without
/// being aligned with the From domain: a provider's bounce domain, which names
/// the operator. Aligned passes are left out, since they name the customer.
/// Ties go to the alphabetically first domain. IPs without one are absent.
pub async fn spf_pass_domains(
    pool: &SqlitePool,
    ips: &[String],
) -> Result<HashMap<String, String>, StoreError> {
    let mut out = HashMap::new();
    for chunk in ips.chunks(IP_CHUNK) {
        let sql = format!(
            r#"
            SELECT source_ip, domain FROM (
                SELECT rec.source_ip, LOWER(spf.domain) AS domain,
                    ROW_NUMBER() OVER (
                        PARTITION BY rec.source_ip
                        ORDER BY SUM(rec.count) DESC, LOWER(spf.domain)
                    ) AS rank
                FROM records rec
                INNER JOIN record_spf_auth spf ON spf.record_id = rec.id
                WHERE spf.result = 'pass' AND spf.aligned = 0
                  AND spf.domain IS NOT NULL AND spf.domain != ''
                  AND rec.source_ip IN ({})
                GROUP BY rec.source_ip, LOWER(spf.domain)
            ) WHERE rank = 1
            "#,
            placeholders(chunk.len())
        );
        let mut query = sqlx::query_as::<_, (String, String)>(&sql);
        for ip in chunk {
            query = query.bind(ip);
        }
        out.extend(query.fetch_all(pool).await?);
    }
    Ok(out)
}

/// Source IPs that appear in reports of visible (not ignored) domains, highest
/// message volume first.
///
/// The identity warm loop resolves these so the senders a user is most likely
/// to open get named first.
pub async fn list_sender_ips_by_volume(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<String>, StoreError> {
    let rows = sqlx::query_scalar::<_, String>(&format!(
        r#"
        SELECT rec.source_ip
        FROM records rec
        INNER JOIN reports r ON r.id = rec.report_id
        WHERE r.domain_id IN {VISIBLE_DOMAIN_IDS}
        GROUP BY rec.source_ip
        ORDER BY SUM(rec.count) DESC
        LIMIT ?
        "#
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The sender name overrides edited in Settings: `(target, name)`, where the
/// target is an operator domain, an IP or a range, and a `None` name hides the
/// name it would get. Sorted by target.
pub async fn list_sender_names(
    pool: &SqlitePool,
) -> Result<Vec<(String, Option<String>)>, StoreError> {
    Ok(
        sqlx::query_as("SELECT target, name FROM sender_names ORDER BY target")
            .fetch_all(pool)
            .await?,
    )
}

/// Replace every sender name override with `names`, in one transaction.
pub async fn replace_sender_names(
    pool: &SqlitePool,
    names: &[(String, Option<String>)],
) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    sqlx::query("DELETE FROM sender_names")
        .execute(&mut *tx)
        .await?;
    for (target, name) in names {
        sqlx::query("INSERT INTO sender_names (target, name) VALUES (?, ?)")
            .bind(target)
            .bind(name)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Raw row with INTEGER `ptr_confirmed`; converted via [`IdentitySnapshot::from`].
#[derive(Debug, FromRow)]
struct IdentitySnapshotRow {
    ip: String,
    checked_at: i64,
    ptr_hostname: Option<String>,
    ptr_confirmed: Option<i64>,
}

impl From<IdentitySnapshotRow> for IdentitySnapshot {
    fn from(row: IdentitySnapshotRow) -> Self {
        Self {
            ip: row.ip,
            checked_at: row.checked_at,
            ptr_hostname: row.ptr_hostname,
            ptr_confirmed: decode_ptr_confirmed(row.ptr_confirmed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptr_confirmed_round_trips_all_three_states() {
        assert_eq!(encode_ptr_confirmed(None), None);
        assert_eq!(encode_ptr_confirmed(Some(true)), Some(1));
        assert_eq!(encode_ptr_confirmed(Some(false)), Some(0));

        assert_eq!(decode_ptr_confirmed(None), None);
        assert_eq!(decode_ptr_confirmed(Some(1)), Some(true));
        assert_eq!(decode_ptr_confirmed(Some(0)), Some(false));
        // Any non-zero integer counts as true (defensive against odd writes).
        assert_eq!(decode_ptr_confirmed(Some(2)), Some(true));
    }

    #[test]
    fn positive_snapshot_uses_shorter_freshness_window() {
        let snap = IdentitySnapshot {
            ip: "203.0.113.1".into(),
            checked_at: 1_000,
            ptr_hostname: Some("mail.example.com".into()),
            ptr_confirmed: Some(true),
        };
        assert!(identity_snapshot_is_fresh(
            &snap,
            1_000 + 3_600,
            86_400,
            604_800
        ));
        assert!(!identity_snapshot_is_fresh(
            &snap,
            1_000 + 86_400,
            86_400,
            604_800
        ));
    }

    #[test]
    fn negative_snapshot_uses_longer_freshness_window() {
        let snap = IdentitySnapshot {
            ip: "203.0.113.2".into(),
            checked_at: 1_000,
            ptr_hostname: None,
            ptr_confirmed: None,
        };
        // Still fresh under the negative window even past the positive window.
        assert!(identity_snapshot_is_fresh(
            &snap,
            1_000 + 86_400 + 1,
            86_400,
            604_800
        ));
        assert!(!identity_snapshot_is_fresh(
            &snap,
            1_000 + 604_800,
            86_400,
            604_800
        ));
    }
}
