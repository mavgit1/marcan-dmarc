//! The mailbox worker's state, per account: the cursor per folder, the
//! quarantine of messages that could not be imported, the retry counts of
//! messages that keep failing, and the record of finished passes. Deleting
//! the account deletes its state (`ON DELETE CASCADE`).
//!
//! The cursor, quarantine and retry counts are keyed by a folder
//! generation, a position and message ids in text, which the source that
//! reads the mailbox writes and reads back; the store only compares them
//! for equality. IMAP spells its numbers in decimal (the UIDVALIDITY, the
//! last handled UID, a UID), and orders them itself, never by this text.
//!
//! A pass writes under the account's `receive_epoch` it was prepared with:
//! each of its writes lands only while the account exists and still has
//! that epoch. A save that made the account read another mailbox forgets
//! the receive state and moves the epoch on in one transaction, so a pass
//! still running then, or after the account was deleted, writes nothing and
//! says so (`false`, or no attempt counted). Reads and deletes take no epoch.

use crate::queries::begin_write;
use crate::StoreError;
use sqlx::sqlite::SqlitePool;
use sqlx::SqliteConnection;

/// Whether account `account_id` exists and, given an `epoch`, still has it.
async fn current(
    conn: &mut SqliteConnection,
    account_id: i64,
    epoch: Option<i64>,
) -> Result<bool, StoreError> {
    let current: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM accounts \
         WHERE id = ?1 AND (?2 IS NULL OR receive_epoch = ?2))",
    )
    .bind(account_id)
    .bind(epoch)
    .fetch_one(conn)
    .await?;
    Ok(current)
}

/// The cursor `(generation, position)` for `folder` of account
/// `account_id`, if one was saved.
pub async fn get_cursor(
    pool: &SqlitePool,
    account_id: i64,
    folder: &str,
) -> Result<Option<(String, String)>, StoreError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT generation, position FROM receive_cursor WHERE account_id = ? AND folder = ?",
    )
    .bind(account_id)
    .bind(folder)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Forget the cursor for `folder` of account `account_id`, its quarantined messages and
/// its retry counts, so the next pass reads the folder from its first message
/// again (already stored reports are skipped).
pub async fn reset_cursor(
    pool: &SqlitePool,
    account_id: i64,
    folder: &str,
) -> Result<(), StoreError> {
    let mut tx = begin_write(pool).await?;
    for table in ["receive_cursor", "receive_quarantine", "receive_attempts"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE account_id = ? AND folder = ?"
        ))
        .bind(account_id)
        .bind(folder)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Upsert the cursor for `folder` of account `account_id`. Quarantine rows
/// and retry counts of another `generation` are stale (their message ids no
/// longer name the same messages) and are dropped in the same transaction.
/// Whether it was written: not when the account is gone or no longer has
/// `epoch`.
pub async fn set_cursor(
    pool: &SqlitePool,
    account_id: i64,
    folder: &str,
    generation: &str,
    position: &str,
    epoch: i64,
) -> Result<bool, StoreError> {
    let mut tx = begin_write(pool).await?;
    if !current(&mut tx, account_id, Some(epoch)).await? {
        return Ok(false);
    }
    sqlx::query(
        r#"
        INSERT INTO receive_cursor (account_id, folder, generation, position)
        VALUES (?, ?, ?, ?)
        ON CONFLICT(account_id, folder) DO UPDATE SET
            generation = excluded.generation,
            position = excluded.position
        "#,
    )
    .bind(account_id)
    .bind(folder)
    .bind(generation)
    .bind(position)
    .execute(&mut *tx)
    .await?;
    for table in ["receive_quarantine", "receive_attempts"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE account_id = ? AND folder = ? AND generation <> ?"
        ))
        .bind(account_id)
        .bind(folder)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Put a message in `receive_quarantine`: it could not be turned into a report.
///
/// Called for a message the cursor is about to move past, so the row is the
/// only lasting evidence the message arrived. Re-recording the same message
/// of the same `generation` (possible after a re-import rewinds the cursor)
/// keeps the one row, with the latest error. Whether it was written: not
/// when the account is gone or no longer has `epoch`.
#[allow(clippy::too_many_arguments)]
pub async fn quarantine_receive_message(
    pool: &SqlitePool,
    account_id: i64,
    folder: &str,
    generation: &str,
    message_id: &str,
    error: &str,
    now: i64,
    epoch: i64,
) -> Result<bool, StoreError> {
    let mut tx = begin_write(pool).await?;
    if !current(&mut tx, account_id, Some(epoch)).await? {
        return Ok(false);
    }
    sqlx::query(
        r#"
        INSERT INTO receive_quarantine
            (account_id, folder, generation, message_id, last_failed_at, error)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(account_id, folder, generation, message_id) DO UPDATE SET
            last_failed_at = excluded.last_failed_at,
            error = excluded.error
        "#,
    )
    .bind(account_id)
    .bind(folder)
    .bind(generation)
    .bind(message_id)
    .bind(now)
    .bind(error)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// How many messages have been quarantined, across all folder
/// generations: of account `account_id` (every account when `None`), in
/// `folder` (every folder when `None`).
pub async fn count_receive_quarantine(
    pool: &SqlitePool,
    account_id: Option<i64>,
    folder: Option<&str>,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM receive_quarantine \
         WHERE (?1 IS NULL OR account_id = ?1) AND (?2 IS NULL OR folder = ?2)",
    )
    .bind(account_id)
    .bind(folder)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Identity of one retry count: the folder generation, what the count is
/// about (`"message"` or `"batch"`) and the message id that names it (a
/// batch's last message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveAttemptKey<'a> {
    pub account_id: i64,
    pub folder: &'a str,
    pub generation: &'a str,
    pub kind: &'a str,
    pub message_id: &'a str,
}

/// Consecutive failed attempts recorded for `key`; `0` when none.
pub async fn get_receive_attempts(
    pool: &SqlitePool,
    key: &ReceiveAttemptKey<'_>,
) -> Result<u32, StoreError> {
    let attempts: Option<i64> = sqlx::query_scalar(
        "SELECT attempts FROM receive_attempts \
         WHERE account_id = ? AND folder = ? AND generation = ? AND kind = ? AND message_id = ?",
    )
    .bind(key.account_id)
    .bind(key.folder)
    .bind(key.generation)
    .bind(key.kind)
    .bind(key.message_id)
    .fetch_optional(pool)
    .await?;
    Ok(attempts.map_or(0, |n| n.max(0) as u32))
}

/// Count one more failed attempt for `key` and return the new total; `0`,
/// counting nothing, when the account is gone or no longer has `epoch`.
pub async fn record_receive_attempt(
    pool: &SqlitePool,
    key: &ReceiveAttemptKey<'_>,
    now: i64,
    epoch: i64,
) -> Result<u32, StoreError> {
    let mut tx = begin_write(pool).await?;
    if !current(&mut tx, key.account_id, Some(epoch)).await? {
        return Ok(0);
    }
    let attempts: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO receive_attempts
            (account_id, folder, generation, kind, message_id, attempts, last_failed_at)
        VALUES (?, ?, ?, ?, ?, 1, ?)
        ON CONFLICT(account_id, folder, generation, kind, message_id) DO UPDATE SET
            attempts = attempts + 1,
            last_failed_at = excluded.last_failed_at
        RETURNING attempts
        "#,
    )
    .bind(key.account_id)
    .bind(key.folder)
    .bind(key.generation)
    .bind(key.kind)
    .bind(key.message_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(attempts.max(0) as u32)
}

/// Forget the attempts of `key` (it was imported or quarantined).
pub async fn clear_receive_attempts(
    pool: &SqlitePool,
    key: &ReceiveAttemptKey<'_>,
) -> Result<(), StoreError> {
    sqlx::query(
        "DELETE FROM receive_attempts \
         WHERE account_id = ? AND folder = ? AND generation = ? AND kind = ? AND message_id = ?",
    )
    .bind(key.account_id)
    .bind(key.folder)
    .bind(key.generation)
    .bind(key.kind)
    .bind(key.message_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// One finished mailbox pass, as recorded in `receive_passes`: what the status
/// display reads. The pass's counts are logged, not stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceivePassRow {
    /// The account the pass read.
    pub account_id: i64,
    pub finished_at: i64,
    /// When the pass reached the mailbox search; `None` when it never got there.
    pub checked_at: Option<i64>,
    /// Why the pass failed; `None` on success.
    pub error: Option<String>,
    /// The server's own answer beside `error`, when it gave one.
    pub error_detail: Option<String>,
    /// It gave way to another mailbox before reading everything: a check of
    /// the mailbox, but no success (the mailbox is not read up to date).
    pub yielded: bool,
}

/// Passes kept in `receive_passes` per account (plus its newest successful one,
/// whatever its age); older ones are pruned on every insert. A
/// five-minute poll fills this in about three days, which is plenty for the
/// status display and for seeing what happened overnight.
pub const RECEIVE_PASSES_KEPT: i64 = 1_000;

/// Append one finished pass and drop the account's oldest beyond
/// [`RECEIVE_PASSES_KEPT`], in one transaction. Whether it was recorded: not
/// when the account is gone or, given the `epoch` its pass was prepared
/// under, no longer has it.
pub async fn record_receive_pass(
    pool: &SqlitePool,
    row: &ReceivePassRow,
    epoch: Option<i64>,
) -> Result<bool, StoreError> {
    let mut tx = begin_write(pool).await?;
    if !current(&mut tx, row.account_id, epoch).await? {
        return Ok(false);
    }
    sqlx::query(
        r#"
        INSERT INTO receive_passes (account_id, finished_at, checked_at, error, error_detail, yielded)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(row.account_id)
    .bind(row.finished_at)
    .bind(row.checked_at)
    .bind(row.error.as_deref())
    .bind(row.error_detail.as_deref())
    .bind(row.yielded)
    .execute(&mut *tx)
    .await?;
    // The newest successful pass is kept whatever its age: it is where
    // "last success" comes from, and a long run of failures must not erase it.
    sqlx::query(
        "DELETE FROM receive_passes WHERE account_id = ?1 AND id <= \
         (SELECT id FROM receive_passes WHERE account_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2) \
         AND id IS NOT (SELECT MAX(id) FROM receive_passes \
         WHERE account_id = ?1 AND error IS NULL AND NOT yielded)",
    )
    .bind(row.account_id)
    .bind(RECEIVE_PASSES_KEPT)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// What the recorded passes say about one account, for the status display.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReceivePassStatus {
    /// The newest time a pass reached the mailbox search.
    pub last_check_at: Option<i64>,
    /// When a pass last read the whole mailbox without error (one that gave
    /// way to another mailbox did not).
    pub last_success_at: Option<i64>,
    /// The error of the newest pass and when it finished, if that pass failed
    /// and no later pass succeeded.
    pub last_error: Option<(String, i64)>,
    /// The server's answer beside [`Self::last_error`], when it gave one.
    pub last_error_detail: Option<String>,
}

/// The status account `account_id`'s recorded passes add up to; everything
/// unset while none was recorded.
pub async fn receive_pass_status(
    pool: &SqlitePool,
    account_id: i64,
) -> Result<ReceivePassStatus, StoreError> {
    let last_check_at: Option<i64> =
        sqlx::query_scalar("SELECT MAX(checked_at) FROM receive_passes WHERE account_id = ?")
            .bind(account_id)
            .fetch_one(pool)
            .await?;
    let last_success_at: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(finished_at) FROM receive_passes \
         WHERE account_id = ? AND error IS NULL AND NOT yielded",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    // The newest row by insertion: finished_at alone cannot order two passes
    // recorded in the same second.
    let newest: Option<(Option<String>, Option<String>, i64)> = sqlx::query_as(
        "SELECT error, error_detail, finished_at FROM receive_passes \
         WHERE account_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let (last_error, last_error_detail) = match newest {
        Some((Some(error), detail, finished_at)) => (Some((error, finished_at)), detail),
        _ => (None, None),
    };
    Ok(ReceivePassStatus {
        last_check_at,
        last_success_at,
        last_error,
        last_error_detail,
    })
}
