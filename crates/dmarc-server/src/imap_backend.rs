//! The IMAP worker's [`Backend`] over the store: the mailbox settings, the
//! cursor, quarantine and retry counts, the reports, and the pass records.

use dmarc_imap::{
    AttemptKey, Backend, Cursor, ImapConfig, ImapSecurity, Ingested, MessageRef, PassRecord,
    DEFAULT_FOLDER, DEFAULT_POLL_INTERVAL_SECS,
};
use dmarc_parser::Report;
use dmarc_store::{
    AppSettings, ImapAttemptKey, ImapPassRow, ImapSecurity as StoredSecurity, IngestResult,
    MailboxSettings, StoreError,
};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;

use crate::snapshots::dns::DnsService;

/// The stored setting as the IMAP client's.
pub fn imap_security(security: StoredSecurity) -> ImapSecurity {
    match security {
        StoredSecurity::Tls => ImapSecurity::Tls,
        StoredSecurity::StartTls => ImapSecurity::StartTls,
    }
}

/// The stored mailbox settings with the worker's defaults applied (TLS, and
/// the usual port for the chosen security). Host, username or password may
/// be empty; see [`ImapConfig::is_complete`].
pub fn stored_imap_config(imap: &MailboxSettings) -> ImapConfig {
    let security = imap_security(imap.security.unwrap_or_default());
    ImapConfig {
        host: imap.host.clone(),
        port: imap.port.unwrap_or_else(|| security.default_port()),
        security,
        username: imap.username.clone(),
        password: imap.password.clone(),
        folder: imap
            .folder
            .clone()
            .unwrap_or_else(|| DEFAULT_FOLDER.to_owned()),
        poll_interval_secs: imap
            .poll_interval_secs
            .unwrap_or(DEFAULT_POLL_INTERVAL_SECS),
    }
}

/// Whether an ingest failure is caused by the report itself (quarantine it and
/// move on) rather than by the database: busy, I/O, disk full, a closed pool,
/// and also a violated constraint, which a parsed report cannot cause on its
/// own (ingest normalizes and derives every constrained value) and so means a
/// schema or store bug. Those fail the batch so the next pass retries the
/// message (up to the IMAP pass's `MAX_ATTEMPTS`, then it is quarantined as a
/// message that keeps failing).
fn is_bad_content(err: &StoreError) -> bool {
    matches!(err, StoreError::Json(_))
}

fn attempt_key(key: &AttemptKey) -> ImapAttemptKey<'_> {
    ImapAttemptKey {
        mailbox: &key.mailbox,
        folder: &key.folder,
        uid_validity: key.uid_validity,
        kind: key.kind.as_str(),
        uid: key.kind.uid(),
    }
}

/// The server's [`Backend`]: every call is one store query, a batch's
/// reports one transaction. A stored report also wakes the DNS warm loop, so
/// a domain it brings in is checked soon.
#[derive(Clone)]
pub struct StoreBackend {
    pool: SqlitePool,
    dns: Arc<DnsService>,
}

impl StoreBackend {
    pub fn new(pool: SqlitePool, dns: Arc<DnsService>) -> Self {
        Self { pool, dns }
    }
}

impl Backend for StoreBackend {
    type Error = StoreError;

    async fn config(&self) -> Result<Option<ImapConfig>, StoreError> {
        let cfg = stored_imap_config(&AppSettings::load(&self.pool).await?.mailbox);
        Ok(cfg.is_complete().then_some(cfg))
    }

    async fn cursor(&self, mailbox: &str, folder: &str) -> Result<Option<Cursor>, StoreError> {
        Ok(dmarc_store::get_cursor(&self.pool, mailbox, folder)
            .await?
            .map(|(uid_validity, last_uid)| Cursor {
                uid_validity,
                last_uid,
            }))
    }

    async fn save_cursor(
        &self,
        mailbox: &str,
        folder: &str,
        cursor: Cursor,
    ) -> Result<(), StoreError> {
        dmarc_store::set_cursor(
            &self.pool,
            mailbox,
            folder,
            cursor.uid_validity,
            cursor.last_uid,
        )
        .await
    }

    async fn reset_folder(&self, mailbox: &str, folder: &str) -> Result<(), StoreError> {
        dmarc_store::reset_cursor(&self.pool, mailbox, folder).await
    }

    async fn quarantine(
        &self,
        message: MessageRef<'_>,
        error: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        dmarc_store::quarantine_imap_message(
            &self.pool,
            message.mailbox,
            message.folder,
            message.uid_validity,
            message.uid,
            error,
            now,
        )
        .await
    }

    async fn quarantined_count(
        &self,
        mailbox: &str,
        folder: &str,
        _uid_validity: u32,
    ) -> Result<i64, StoreError> {
        // Other generations' rows are dropped when the cursor is saved, so the
        // folder's count is the generation's.
        dmarc_store::count_imap_quarantine(&self.pool, Some((mailbox, folder))).await
    }

    async fn attempts(&self, key: &AttemptKey) -> Result<u32, StoreError> {
        dmarc_store::get_imap_attempts(&self.pool, &attempt_key(key)).await
    }

    async fn record_attempt(&self, key: &AttemptKey, now: i64) -> Result<u32, StoreError> {
        dmarc_store::record_imap_attempt(&self.pool, &attempt_key(key), now).await
    }

    async fn clear_attempts(&self, key: &AttemptKey) -> Result<(), StoreError> {
        dmarc_store::clear_imap_attempts(&self.pool, &attempt_key(key)).await
    }

    async fn ingest(&self, report: &Report) -> Result<Ingested, StoreError> {
        match dmarc_store::ingest_report(&self.pool, report).await {
            Ok(IngestResult::Inserted { .. }) => {
                // The report may name a domain not checked yet; a pass that
                // finds every domain fresh costs one query.
                self.dns.wake();
                Ok(Ingested::Inserted)
            }
            Ok(IngestResult::Duplicate { .. }) => Ok(Ingested::Duplicate),
            Ok(IngestResult::Ignored) => Ok(Ingested::Ignored),
            Err(e) if is_bad_content(&e) => Ok(Ingested::Rejected(e.to_string())),
            Err(e) => Err(e),
        }
    }

    /// One write transaction for the batch ([`dmarc_store::ingest_reports`]):
    /// on a filesystem where every commit is slow (a Docker bind mount, a
    /// NAS), a first import waits for one commit per batch instead of one per
    /// report. A report whose content is bad is left out and the rest stored
    /// again, so it is rejected alone; any other failure stores nothing.
    async fn ingest_batch(&self, reports: &[&Report]) -> Result<Vec<Ingested>, StoreError> {
        let mut rejected: HashMap<usize, String> = HashMap::new();
        loop {
            let kept: Vec<usize> = (0..reports.len())
                .filter(|i| !rejected.contains_key(i))
                .collect();
            let batch: Vec<&Report> = kept.iter().map(|&i| reports[i]).collect();
            match dmarc_store::ingest_reports(&self.pool, &batch).await {
                Ok(results) => {
                    let mut results = kept.into_iter().zip(results).collect::<HashMap<_, _>>();
                    let answers: Vec<Ingested> = (0..reports.len())
                        .map(|i| match (rejected.remove(&i), results.remove(&i)) {
                            (Some(reason), _) => Ingested::Rejected(reason),
                            (None, Some(IngestResult::Inserted { .. })) => Ingested::Inserted,
                            (None, Some(IngestResult::Duplicate { .. })) => Ingested::Duplicate,
                            (None, Some(IngestResult::Ignored) | None) => Ingested::Ignored,
                        })
                        .collect();
                    if answers.contains(&Ingested::Inserted) {
                        self.dns.wake();
                    }
                    return Ok(answers);
                }
                Err(e) => match e.index {
                    Some(index) if is_bad_content(&e.error) => {
                        rejected.insert(kept[index], e.error.to_string());
                    }
                    _ => return Err(e.error),
                },
            }
        }
    }

    async fn record_pass(&self, record: &PassRecord) -> Result<(), StoreError> {
        dmarc_store::record_imap_pass(
            &self.pool,
            &ImapPassRow {
                finished_at: record.finished_at,
                checked_at: record.checked_at,
                error: record.error.clone(),
            },
        )
        .await
    }

    fn now(&self) -> i64 {
        dmarc_store::unix_now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bad_content_is_a_json_error_not_a_database_failure() {
        let dir = tempfile::tempdir().unwrap();
        let pool = dmarc_store::init(&dir.path().join("t.db")).await.unwrap();
        let json = serde_json::from_str::<i32>("x").unwrap_err();
        assert!(is_bad_content(&StoreError::Json(json)));
        assert!(!is_bad_content(&StoreError::Database(
            sqlx::Error::PoolClosed
        )));
        for _ in 0..2 {
            let result = sqlx::query("INSERT INTO app_settings (key, value) VALUES ('k', 'v')")
                .execute(&pool)
                .await;
            if let Err(e) = result {
                assert!(!is_bad_content(&StoreError::Database(e)));
                return;
            }
        }
        panic!("the duplicate key was accepted");
    }
}
