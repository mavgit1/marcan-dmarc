//! The mailbox worker's [`Backend`] over the store: the receiving accounts,
//! the cursor, quarantine and retry counts, the reports, and the pass
//! records.
//!
//! Every account that receives reports and has a complete login is a mailbox
//! of the worker, its id the account id as a string; [`Backend::prepare`]
//! reads the account and opens its password, or fetches its Microsoft
//! login's Graph token, for the pass about to start (a locked one is
//! skipped, and says why in the log).
//!
//! The configuration [`Backend::prepare`] hands out carries the account's
//! `receive_epoch` ([`MailboxConfig::epoch`]), and the pass's writes (cursor,
//! skipped messages, retry counts, its record) land only while the account
//! still has it. A save that made the account read another mailbox (which
//! moves the epoch on) or deleted it while the pass ran turns the rest of
//! that pass's writes into no-ops instead of the old mailbox's cursor or a
//! foreign-key error.

use crate::accounts::{AccountError, Accounts};
use dmarc_mailbox::{
    AttemptKey, Backend, Cursor, Ingested, MailboxConfig, MailboxRef, MessageRef, PassRecord,
};
use dmarc_parser::Report;
use dmarc_store::{AppSettings, IngestResult, ReceiveAttemptKey, ReceivePassRow, StoreError};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::snapshots::dns::DnsService;

/// The account id a mailbox id names; ids the server did not hand out name
/// no account (never one with id 0).
fn account_id(mailbox: &str) -> i64 {
    mailbox.parse().unwrap_or(0)
}

/// Whether an ingest failure is caused by the report itself (quarantine it and
/// move on) rather than by the database: busy, I/O, disk full, a closed pool,
/// and also a violated constraint, which a parsed report cannot cause on its
/// own (ingest normalizes and derives every constrained value) and so means a
/// schema or store bug. Those fail the batch so the next pass retries the
/// message (up to the pass's `MAX_ATTEMPTS`, then it is quarantined as a
/// message that keeps failing).
fn is_bad_content(err: &StoreError) -> bool {
    matches!(err, StoreError::Json(_))
}

fn attempt_key(key: &AttemptKey) -> ReceiveAttemptKey<'_> {
    ReceiveAttemptKey {
        account_id: account_id(&key.mailbox),
        folder: &key.folder,
        generation: &key.generation,
        kind: key.kind.as_str(),
        message_id: key.kind.message_id(),
    }
}

/// The server's [`Backend`]: every call is one store query, a batch's
/// reports one transaction. A stored report also wakes the DNS warm loop, so
/// a domain it brings in is checked soon.
#[derive(Clone)]
pub struct StoreBackend {
    pool: SqlitePool,
    dns: Arc<DnsService>,
    accounts: Arc<Accounts>,
}

impl StoreBackend {
    pub fn new(pool: SqlitePool, dns: Arc<DnsService>, accounts: Arc<Accounts>) -> Self {
        Self {
            pool,
            dns,
            accounts,
        }
    }

    async fn poll(&self) -> Result<Duration, StoreError> {
        let receiving = AppSettings::load(&self.pool).await?.receiving;
        Ok(Duration::from_secs(receiving.poll_interval_secs))
    }
}

impl Backend for StoreBackend {
    type Error = AccountError;

    async fn mailboxes(&self) -> Result<Vec<MailboxRef>, AccountError> {
        let poll = self.poll().await?;
        let snapshot = self.accounts.snapshot().await?;
        Ok(snapshot
            .receiving()
            .map(|account| MailboxRef {
                id: account.id.to_string(),
                poll,
            })
            .collect())
    }

    async fn prepare(&self, id: &str) -> Result<Option<MailboxConfig>, AccountError> {
        let poll = self.poll().await?;
        self.accounts.mailbox_config(account_id(id), poll).await
    }

    async fn cursor(&self, mailbox: &str, folder: &str) -> Result<Option<Cursor>, AccountError> {
        Ok(
            dmarc_store::get_cursor(&self.pool, account_id(mailbox), folder)
                .await?
                .map(|(generation, position)| Cursor {
                    generation,
                    position,
                }),
        )
    }

    async fn save_cursor(
        &self,
        mailbox: &str,
        epoch: i64,
        folder: &str,
        cursor: Cursor,
    ) -> Result<(), AccountError> {
        let written = dmarc_store::set_cursor(
            &self.pool,
            account_id(mailbox),
            folder,
            &cursor.generation,
            &cursor.position,
            epoch,
        )
        .await?;
        if !written {
            tracing::debug!(
                mailbox,
                "cursor not saved: the account reads another mailbox now"
            );
        }
        Ok(())
    }

    async fn reset_folder(&self, mailbox: &str, folder: &str) -> Result<(), AccountError> {
        dmarc_store::reset_cursor(&self.pool, account_id(mailbox), folder)
            .await
            .map_err(AccountError::from)
    }

    async fn quarantine(
        &self,
        message: MessageRef<'_>,
        error: &str,
        now: i64,
    ) -> Result<(), AccountError> {
        dmarc_store::quarantine_receive_message(
            &self.pool,
            account_id(message.mailbox),
            message.folder,
            message.generation,
            message.message_id,
            error,
            now,
            message.epoch,
        )
        .await?;
        Ok(())
    }

    async fn quarantined_count(
        &self,
        mailbox: &str,
        folder: &str,
        _generation: &str,
    ) -> Result<i64, AccountError> {
        // Other generations' rows are dropped when the cursor is saved, so the
        // folder's count is the generation's.
        dmarc_store::count_receive_quarantine(&self.pool, Some(account_id(mailbox)), Some(folder))
            .await
            .map_err(AccountError::from)
    }

    async fn attempts(&self, key: &AttemptKey) -> Result<u32, AccountError> {
        dmarc_store::get_receive_attempts(&self.pool, &attempt_key(key))
            .await
            .map_err(AccountError::from)
    }

    async fn record_attempt(&self, key: &AttemptKey, now: i64) -> Result<u32, AccountError> {
        dmarc_store::record_receive_attempt(&self.pool, &attempt_key(key), now, key.epoch)
            .await
            .map_err(AccountError::from)
    }

    async fn clear_attempts(&self, key: &AttemptKey) -> Result<(), AccountError> {
        dmarc_store::clear_receive_attempts(&self.pool, &attempt_key(key))
            .await
            .map_err(AccountError::from)
    }

    async fn ingest(&self, report: &Report) -> Result<Ingested, AccountError> {
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
            Err(e) => Err(e.into()),
        }
    }

    /// One write transaction for the batch ([`dmarc_store::ingest_reports`]):
    /// on a filesystem where every commit is slow (a Docker bind mount, a
    /// NAS), a first import waits for one commit per batch instead of one per
    /// report. A report whose content is bad is left out and the rest stored
    /// again, so it is rejected alone; any other failure stores nothing.
    async fn ingest_batch(&self, reports: &[&Report]) -> Result<Vec<Ingested>, AccountError> {
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
                    _ => return Err(e.error.into()),
                },
            }
        }
    }

    async fn record_pass(&self, record: &PassRecord) -> Result<(), AccountError> {
        if record.error.is_some() && record.checked_at.is_none() {
            // It failed before reading the mailbox, perhaps at the login: a
            // Microsoft login's next pass fetches a new token rather than
            // offer Exchange the one it may have refused.
            self.accounts
                .forget_token_of(account_id(&record.mailbox))
                .await;
        }
        dmarc_store::record_receive_pass(
            &self.pool,
            &ReceivePassRow {
                account_id: account_id(&record.mailbox),
                finished_at: record.finished_at,
                checked_at: record.checked_at,
                error: record.error.clone(),
                error_detail: record.error_detail.clone(),
                yielded: record.summary.yielded,
            },
            record.epoch,
        )
        .await?;
        Ok(())
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
