//! Cheap whole-database figures for the health endpoint and Settings → Data,
//! and the report retention clean-up.

use crate::queries::begin_write;
use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::FromRow;

/// Newest report activity: the later of the newest ingest time and the newest
/// report's `date_begin`. `None` before the first report.
/// Two single-column `MAX`es, each one index lookup; the combined form
/// `MAX(MAX(a), MAX(b))` would scan the table.
pub async fn last_ingest_at(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    let created: Option<i64> = sqlx::query_scalar("SELECT MAX(created_at) FROM reports")
        .fetch_one(pool)
        .await?;
    let begun: Option<i64> = sqlx::query_scalar("SELECT MAX(date_begin) FROM reports")
        .fetch_one(pool)
        .await?;
    Ok(created.max(begun))
}

/// Size of the open database from SQLite itself, whatever path it was opened with.
pub async fn database_size_bytes(pool: &SqlitePool) -> Result<i64, StoreError> {
    let size: i64 = sqlx::query_scalar(
        "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
    )
    .fetch_one(pool)
    .await?;
    Ok(size)
}

/// Report count and message total over all reports, for the health endpoint.
///
/// Cheap by design (it is polled): `reports.total_messages` is written at ingest
/// from the record rows, so its sum equals `SUM(records.count)`, and
/// `idx_reports_date` covers it, so no report body is read.
pub async fn report_totals(pool: &SqlitePool) -> Result<ReportTotals, StoreError> {
    let row = sqlx::query_as::<_, ReportTotals>(
        "SELECT COUNT(*) AS report_count, COALESCE(SUM(total_messages), 0) AS total_messages          FROM reports",
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Result of [`report_totals`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct ReportTotals {
    pub report_count: i64,
    pub total_messages: i64,
}

/// The report end date before which reports are deleted, for a retention of
/// `months` (of 30 days each) at `now`.
pub fn retention_cutoff(now: i64, months: u32) -> i64 {
    now.saturating_sub(i64::from(months) * 30 * 86_400)
}

/// Delete up to `limit` of the reports whose `date_end` is before `cutoff`
/// (oldest first), in one write transaction; returns how many were deleted.
/// Retention calls it until fewer than `limit` go, so a large clean-up holds
/// the write lock for one chunk at a time. Their records, auth results,
/// overrides and raw XML go with them (`ON DELETE CASCADE`). Domains,
/// organizations and DNS results stay: a domain without reports is still
/// monitored.
pub async fn delete_reports_older_than(
    pool: &SqlitePool,
    cutoff: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut tx = begin_write(pool).await?;
    let done = sqlx::query(
        "DELETE FROM reports WHERE id IN \
         (SELECT id FROM reports WHERE date_end < ? ORDER BY date_end, id LIMIT ?)",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(done.rows_affected())
}
