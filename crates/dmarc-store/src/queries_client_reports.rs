//! Client reports: who gets each organization's monthly summary email.
//!
//! The recipient list is edited as text under Settings → Client reports
//! (Recipients) and is the whole truth: saving replaces every stored
//! recipient, so a line taken out stops that recipient's report. The server parses and validates the text
//! (organizations by name, addresses); this module previews and applies the
//! validated list in one transaction.
//!
//! `client_report_sends` is the monthly job's record: one row per recipient
//! and month, written as each email goes out; `client_report_failures` one
//! row per recipient and month whose email failed, as the server classified
//! it, until it goes out. The month is the caller's key (the server's
//! `YYYY-MM` of the report time); nothing here decides when a report is due
//! or whether a failure is tried again.

use crate::queries::begin_write;
use crate::StoreError;
use serde::Serialize;
use sqlx::sqlite::{SqliteConnection, SqlitePool};
use sqlx::FromRow;
use std::collections::HashSet;
use ts_rs::TS;

/// One recipient of an organization's client report.
#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, TS)]
#[ts(export)]
pub struct ClientReportRecipient {
    pub organization_id: i64,
    /// The organization's stored name.
    pub organization: String,
    pub email: String,
}

impl ClientReportRecipient {
    /// Addresses compare without case, like the table's `COLLATE NOCASE`.
    fn key(&self) -> (i64, String) {
        (self.organization_id, self.email.trim().to_ascii_lowercase())
    }
}

/// What saving a recipient list changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ClientReportChanges {
    /// Listed now and not before: they start getting the organization's report.
    pub start: Vec<ClientReportRecipient>,
    /// Stored and no longer listed: their reports stop.
    pub stop: Vec<ClientReportRecipient>,
    /// Listed before and after.
    pub unchanged_count: usize,
}

impl ClientReportChanges {
    pub fn is_empty(&self) -> bool {
        self.start.is_empty() && self.stop.is_empty()
    }
}

const RECIPIENTS_SELECT: &str = "SELECT r.organization_id, org.name AS organization, r.email \
     FROM client_report_recipients r \
     INNER JOIN organizations org ON org.id = r.organization_id \
     ORDER BY org.name COLLATE NOCASE, r.email COLLATE NOCASE";

async fn recipients_in(
    conn: &mut SqliteConnection,
) -> Result<Vec<ClientReportRecipient>, StoreError> {
    Ok(
        sqlx::query_as::<_, ClientReportRecipient>(RECIPIENTS_SELECT)
            .fetch_all(conn)
            .await?,
    )
}

/// Every stored recipient, by organization name, then address.
pub async fn list_client_report_recipients(
    pool: &SqlitePool,
) -> Result<Vec<ClientReportRecipient>, StoreError> {
    let mut conn = pool.acquire().await?;
    recipients_in(&mut conn).await
}

/// The stored recipients who have not been sent `month`'s report, in
/// [`list_client_report_recipients`]' order.
pub async fn list_unsent_client_report_recipients(
    pool: &SqlitePool,
    month: &str,
) -> Result<Vec<ClientReportRecipient>, StoreError> {
    Ok(sqlx::query_as::<_, ClientReportRecipient>(
        "SELECT r.organization_id, org.name AS organization, r.email \
         FROM client_report_recipients r \
         INNER JOIN organizations org ON org.id = r.organization_id \
         WHERE NOT EXISTS (SELECT 1 FROM client_report_sends s \
             WHERE s.organization_id = r.organization_id AND s.email = r.email AND s.month = ?) \
         ORDER BY org.name COLLATE NOCASE, r.email COLLATE NOCASE",
    )
    .bind(month)
    .fetch_all(pool)
    .await?)
}

/// How many emails of `month`'s reports went out (to anyone, listed now or not).
pub async fn count_client_report_sends(pool: &SqlitePool, month: &str) -> Result<i64, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM client_report_sends WHERE month = ?")
            .bind(month)
            .fetch_one(pool)
            .await?,
    )
}

/// Record that `month`'s report went to `email` of `organization_id` at
/// `sent_at`. A second record of the same send keeps the first.
pub async fn record_client_report_send(
    pool: &SqlitePool,
    organization_id: i64,
    email: &str,
    month: &str,
    sent_at: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT OR IGNORE INTO client_report_sends (organization_id, email, month, sent_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(organization_id)
    .bind(email.trim())
    .bind(month)
    .bind(sent_at)
    .execute(pool)
    .await?;
    sqlx::query(
        "DELETE FROM client_report_failures WHERE organization_id = ? AND email = ? AND month = ?",
    )
    .bind(organization_id)
    .bind(email.trim())
    .bind(month)
    .execute(pool)
    .await?;
    Ok(())
}

/// A client report that did not reach a recipient: who, which month, how
/// the server classified the failure and when.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ClientReportFailure {
    pub organization_id: i64,
    /// The organization's stored name (filled in when listed).
    pub organization: String,
    pub email: String,
    pub month: String,
    /// Not to be tried again this month.
    pub permanent: bool,
    /// The failure in a few words.
    pub cause: String,
    /// The server's whole answer.
    pub detail: String,
    pub first_failed_at: i64,
    pub last_failed_at: i64,
}

/// A failed client report to record ([`record_client_report_failure`]).
#[derive(Debug, Clone, Copy)]
pub struct FailedClientReport<'a> {
    pub organization_id: i64,
    pub email: &'a str,
    pub month: &'a str,
    pub permanent: bool,
    pub cause: &'a str,
    pub detail: &'a str,
    pub failed_at: i64,
}

/// Record that a report failed: a first failure as it is; a later one keeps
/// the first failure's time and takes the latest classification, cause and
/// detail.
pub async fn record_client_report_failure(
    pool: &SqlitePool,
    failure: &FailedClientReport<'_>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO client_report_failures \
             (organization_id, email, month, permanent, cause, detail, first_failed_at, last_failed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (organization_id, email, month) DO UPDATE SET \
             permanent = excluded.permanent, cause = excluded.cause, detail = excluded.detail, \
             last_failed_at = excluded.last_failed_at",
    )
    .bind(failure.organization_id)
    .bind(failure.email.trim())
    .bind(failure.month)
    .bind(failure.permanent)
    .bind(failure.cause)
    .bind(failure.detail)
    .bind(failure.failed_at)
    .bind(failure.failed_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// `month`'s failures of the recipients listed now, in
/// [`list_client_report_recipients`]' order.
pub async fn list_client_report_failures(
    pool: &SqlitePool,
    month: &str,
) -> Result<Vec<ClientReportFailure>, StoreError> {
    Ok(sqlx::query_as::<_, ClientReportFailure>(
        "SELECT f.organization_id, org.name AS organization, f.email, f.month, f.permanent, \
                f.cause, f.detail, f.first_failed_at, f.last_failed_at \
         FROM client_report_failures f \
         INNER JOIN client_report_recipients r \
             ON r.organization_id = f.organization_id AND r.email = f.email \
         INNER JOIN organizations org ON org.id = f.organization_id \
         WHERE f.month = ? \
         ORDER BY org.name COLLATE NOCASE, f.email COLLATE NOCASE",
    )
    .bind(month)
    .fetch_all(pool)
    .await?)
}

/// When a client report last went out; `None` before the first.
pub async fn client_reports_last_sent_at(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT MAX(sent_at) FROM client_report_sends")
            .fetch_one(pool)
            .await?,
    )
}

/// `next` against `current`: who starts (in `next`'s order, once each), who
/// stops (in `current`'s order), how many stay.
fn diff(current: &[ClientReportRecipient], next: &[ClientReportRecipient]) -> ClientReportChanges {
    let before: HashSet<(i64, String)> = current.iter().map(ClientReportRecipient::key).collect();
    let after: HashSet<(i64, String)> = next.iter().map(ClientReportRecipient::key).collect();
    let mut seen = HashSet::new();
    ClientReportChanges {
        start: next
            .iter()
            .filter(|r| !before.contains(&r.key()) && seen.insert(r.key()))
            .cloned()
            .collect(),
        stop: current
            .iter()
            .filter(|r| !after.contains(&r.key()))
            .cloned()
            .collect(),
        unchanged_count: current.iter().filter(|r| after.contains(&r.key())).count(),
    }
}

/// What replacing the stored recipients with `next` would change; writes
/// nothing. `next` is validated (every organization exists).
pub async fn preview_client_report_recipients(
    pool: &SqlitePool,
    next: &[ClientReportRecipient],
) -> Result<ClientReportChanges, StoreError> {
    Ok(diff(&list_client_report_recipients(pool).await?, next))
}

/// Replace every stored recipient with `next` in one transaction, and say
/// what changed. A repeated `(organization, address)` is stored once.
pub async fn replace_client_report_recipients(
    pool: &SqlitePool,
    next: &[ClientReportRecipient],
) -> Result<ClientReportChanges, StoreError> {
    let mut tx = begin_write(pool).await?;
    let changes = diff(&recipients_in(&mut tx).await?, next);
    sqlx::query("DELETE FROM client_report_recipients")
        .execute(&mut *tx)
        .await?;
    for recipient in next {
        sqlx::query(
            "INSERT OR IGNORE INTO client_report_recipients (organization_id, email) VALUES (?, ?)",
        )
        .bind(recipient.organization_id)
        .bind(recipient.email.trim())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: i64, org: &str, email: &str) -> ClientReportRecipient {
        ClientReportRecipient {
            organization_id: id,
            organization: org.into(),
            email: email.into(),
        }
    }

    #[test]
    fn diff_names_who_starts_and_who_stops() {
        let current = [
            r(2, "Acme", "a@acme.example"),
            r(3, "Beta", "b@beta.example"),
        ];
        let next = [
            r(2, "Acme", "A@ACME.example"),
            r(3, "Beta", "new@beta.example"),
            r(3, "Beta", "new@beta.example"),
        ];
        let changes = diff(&current, &next);
        assert_eq!(changes.start, [r(3, "Beta", "new@beta.example")]);
        assert_eq!(changes.stop, [r(3, "Beta", "b@beta.example")]);
        assert_eq!(changes.unchanged_count, 1);
        assert!(diff(&current, &current).is_empty());
    }
}
