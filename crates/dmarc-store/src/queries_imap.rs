//! The IMAP worker's state: the UID cursor per mailbox and folder, the
//! quarantine of messages that could not be imported, the retry counts of
//! messages that keep failing, and the record of finished passes.

use crate::queries::begin_write;
use crate::StoreError;
use sqlx::sqlite::SqlitePool;

/// The IMAP UID cursor `(uid_validity, last_uid)` for `folder` of `mailbox`
/// (see `ImapConfig::mailbox_key`), if one was saved.
pub async fn get_cursor(
    pool: &SqlitePool,
    mailbox: &str,
    folder: &str,
) -> Result<Option<(u32, u32)>, StoreError> {
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT uid_validity, last_uid FROM imap_cursor WHERE mailbox = ? AND folder = ?",
    )
    .bind(mailbox)
    .bind(folder)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(uid_validity, last_uid)| (uid_validity as u32, last_uid as u32)))
}

/// Forget the cursor for `folder` of `mailbox`, its quarantined messages and
/// its retry counts, so the next pass reads the folder from its first message
/// again (already stored reports are skipped).
pub async fn reset_cursor(
    pool: &SqlitePool,
    mailbox: &str,
    folder: &str,
) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    for table in ["imap_cursor", "imap_quarantine", "imap_attempts"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE mailbox = ? AND folder = ?"
        ))
        .bind(mailbox)
        .bind(folder)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Upsert the IMAP UID cursor for `folder` of `mailbox`. Quarantine rows and
/// retry counts of another UIDVALIDITY generation are stale (their UIDs no
/// longer name the same messages) and are dropped in the same transaction.
pub async fn set_cursor(
    pool: &SqlitePool,
    mailbox: &str,
    folder: &str,
    uid_validity: u32,
    last_uid: u32,
) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    sqlx::query(
        r#"
        INSERT INTO imap_cursor (mailbox, folder, uid_validity, last_uid)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(mailbox, folder) DO UPDATE SET
            uid_validity = excluded.uid_validity,
            last_uid = excluded.last_uid
        "#,
    )
    .bind(mailbox)
    .bind(folder)
    .bind(i64::from(uid_validity))
    .bind(i64::from(last_uid))
    .execute(&mut *tx)
    .await?;
    for table in ["imap_quarantine", "imap_attempts"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE mailbox = ? AND folder = ? AND uid_validity <> ?"
        ))
        .bind(mailbox)
        .bind(folder)
        .bind(i64::from(uid_validity))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Put a message in `imap_quarantine`: it could not be turned into a report.
///
/// Called for a UID the cursor is about to move past, so the row is the only
/// lasting evidence the message arrived. Re-recording the same UID (possible
/// after a UIDVALIDITY reset rewinds the cursor) keeps the one row, with the
/// latest error.
pub async fn quarantine_imap_message(
    pool: &SqlitePool,
    mailbox: &str,
    folder: &str,
    uid_validity: u32,
    uid: u32,
    error: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        r#"
        INSERT INTO imap_quarantine
            (mailbox, folder, uid_validity, uid, last_failed_at, error)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(mailbox, folder, uid_validity, uid) DO UPDATE SET
            last_failed_at = excluded.last_failed_at,
            error = excluded.error
        "#,
    )
    .bind(mailbox)
    .bind(folder)
    .bind(i64::from(uid_validity))
    .bind(i64::from(uid))
    .bind(now)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// How many messages have been quarantined for `(mailbox, folder)` (everything
/// when `None`), across all UIDVALIDITY generations.
pub async fn count_imap_quarantine(
    pool: &SqlitePool,
    scope: Option<(&str, &str)>,
) -> Result<i64, StoreError> {
    let (mailbox, folder) = scope.unzip();
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imap_quarantine \
         WHERE ?1 IS NULL OR (mailbox = ?1 AND folder = ?2)",
    )
    .bind(mailbox)
    .bind(folder)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Identity of one retry count: the folder generation, what the count is
/// about (`"message"` or `"batch"`) and the UID that names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapAttemptKey<'a> {
    pub mailbox: &'a str,
    pub folder: &'a str,
    pub uid_validity: u32,
    pub kind: &'a str,
    pub uid: u32,
}

/// Consecutive failed attempts recorded for `key`; `0` when none.
pub async fn get_imap_attempts(
    pool: &SqlitePool,
    key: &ImapAttemptKey<'_>,
) -> Result<u32, StoreError> {
    let attempts: Option<i64> = sqlx::query_scalar(
        "SELECT attempts FROM imap_attempts \
         WHERE mailbox = ? AND folder = ? AND uid_validity = ? AND kind = ? AND uid = ?",
    )
    .bind(key.mailbox)
    .bind(key.folder)
    .bind(i64::from(key.uid_validity))
    .bind(key.kind)
    .bind(i64::from(key.uid))
    .fetch_optional(pool)
    .await?;
    Ok(attempts.map_or(0, |n| n.max(0) as u32))
}

/// Count one more failed attempt for `key` and return the new total.
pub async fn record_imap_attempt(
    pool: &SqlitePool,
    key: &ImapAttemptKey<'_>,
    now: i64,
) -> Result<u32, StoreError> {
    let attempts: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO imap_attempts (mailbox, folder, uid_validity, kind, uid, attempts, last_failed_at)
        VALUES (?, ?, ?, ?, ?, 1, ?)
        ON CONFLICT(mailbox, folder, uid_validity, kind, uid) DO UPDATE SET
            attempts = attempts + 1,
            last_failed_at = excluded.last_failed_at
        RETURNING attempts
        "#,
    )
    .bind(key.mailbox)
    .bind(key.folder)
    .bind(i64::from(key.uid_validity))
    .bind(key.kind)
    .bind(i64::from(key.uid))
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(attempts.max(0) as u32)
}

/// Forget the attempts of `key` (it was imported or quarantined).
pub async fn clear_imap_attempts(
    pool: &SqlitePool,
    key: &ImapAttemptKey<'_>,
) -> Result<(), StoreError> {
    sqlx::query(
        "DELETE FROM imap_attempts \
         WHERE mailbox = ? AND folder = ? AND uid_validity = ? AND kind = ? AND uid = ?",
    )
    .bind(key.mailbox)
    .bind(key.folder)
    .bind(i64::from(key.uid_validity))
    .bind(key.kind)
    .bind(i64::from(key.uid))
    .execute(pool)
    .await?;
    Ok(())
}

/// One finished mailbox pass, as recorded in `imap_passes`: what the status
/// display reads. The pass's counts are logged, not stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImapPassRow {
    pub finished_at: i64,
    /// When the pass reached the mailbox search; `None` when it never got there.
    pub checked_at: Option<i64>,
    /// Why the pass failed; `None` on success.
    pub error: Option<String>,
}

/// Passes kept in `imap_passes` (plus the newest successful one, whatever its
/// age); older ones are pruned on every insert. A
/// five-minute poll fills this in about three days, which is plenty for the
/// status display and for seeing what happened overnight.
pub const IMAP_PASSES_KEPT: i64 = 1_000;

/// Append one finished pass and drop the oldest beyond [`IMAP_PASSES_KEPT`],
/// in one transaction.
pub async fn record_imap_pass(pool: &SqlitePool, row: &ImapPassRow) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    sqlx::query(
        r#"
        INSERT INTO imap_passes (finished_at, checked_at, error)
        VALUES (?, ?, ?)
        "#,
    )
    .bind(row.finished_at)
    .bind(row.checked_at)
    .bind(row.error.as_deref())
    .execute(&mut *tx)
    .await?;
    // The newest successful pass is kept whatever its age: it is where
    // "last success" comes from, and a long run of failures must not erase it.
    sqlx::query(
        "DELETE FROM imap_passes WHERE id <= \
         (SELECT id FROM imap_passes ORDER BY id DESC LIMIT 1 OFFSET ?) \
         AND id IS NOT (SELECT MAX(id) FROM imap_passes WHERE error IS NULL)",
    )
    .bind(IMAP_PASSES_KEPT)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// What the recorded passes say about the mailbox, for the status display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImapPassStatus {
    /// The newest time a pass reached the mailbox search.
    pub last_check_at: Option<i64>,
    /// When a pass last finished without error.
    pub last_success_at: Option<i64>,
    /// The error of the newest pass and when it finished, if that pass failed
    /// and no later pass succeeded.
    pub last_error: Option<(String, i64)>,
}

/// The status the recorded passes add up to; everything unset while none
/// was recorded.
pub async fn imap_pass_status(pool: &SqlitePool) -> Result<ImapPassStatus, StoreError> {
    let last_check_at: Option<i64> = sqlx::query_scalar("SELECT MAX(checked_at) FROM imap_passes")
        .fetch_one(pool)
        .await?;
    let last_success_at: Option<i64> =
        sqlx::query_scalar("SELECT MAX(finished_at) FROM imap_passes WHERE error IS NULL")
            .fetch_one(pool)
            .await?;
    // The newest row by insertion: finished_at alone cannot order two passes
    // recorded in the same second.
    let newest: Option<(Option<String>, i64)> =
        sqlx::query_as("SELECT error, finished_at FROM imap_passes ORDER BY id DESC LIMIT 1")
            .fetch_optional(pool)
            .await?;
    let last_error = match newest {
        Some((Some(error), finished_at)) => Some((error, finished_at)),
        _ => None,
    };
    Ok(ImapPassStatus {
        last_check_at,
        last_success_at,
        last_error,
    })
}
