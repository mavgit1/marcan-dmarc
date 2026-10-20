//! What a pass needs from its host: the configuration, where the cursor and
//! attempt counts live, where skipped messages and pass outcomes are recorded,
//! and where the reports go. The server implements this over its database;
//! tests use an in-memory one.

use crate::config::ImapConfig;
use crate::pass::FetchSummary;
use dmarc_parser::Report;
use std::future::Future;

/// The UID cursor of one folder: every UID up to `last_uid` of generation
/// `uid_validity` has been handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub uid_validity: u32,
    pub last_uid: u32,
}

/// One message of one folder generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef<'a> {
    pub mailbox: &'a str,
    pub folder: &'a str,
    pub uid_validity: u32,
    pub uid: u32,
}

/// What an attempt count is about: one message, or one download batch (named by
/// its last UID), which is split into single messages once it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttemptKind {
    Message(u32),
    Batch(u32),
}

impl AttemptKind {
    /// The stored name of the kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Batch(_) => "batch",
        }
    }

    /// The UID the count is keyed by.
    pub fn uid(self) -> u32 {
        match self {
            Self::Message(uid) | Self::Batch(uid) => uid,
        }
    }
}

/// Identity of an attempt count: folder generation plus what it is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptKey {
    pub mailbox: String,
    pub folder: String,
    pub uid_validity: u32,
    pub kind: AttemptKind,
}

/// What became of one report handed to the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingested {
    Inserted,
    /// A report with the same reporter and `report_id` already existed.
    Duplicate,
    /// The report's domain is ignored: dropped on purpose.
    Ignored,
    /// The backend refused the report because of its content (a value it
    /// cannot store): quarantined, never retried. A failure of the store
    /// itself, a violated constraint included, is an `Err` instead.
    Rejected(String),
}

/// One finished pass, for the host's status display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassRecord {
    pub started_at: i64,
    pub finished_at: i64,
    /// When the pass reached the mailbox search; `None` when it never got there.
    pub checked_at: Option<i64>,
    pub summary: FetchSummary,
    /// Why the pass failed, password redacted; `None` on success.
    pub error: Option<String>,
}

/// The host side of a pass. Every method is a small, idempotent operation;
/// an `Err` is an environment failure (the database is unavailable, say)
/// that makes the pass fail without moving the cursor, so it is retried.
pub trait Backend: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// The mailbox configuration, or `None` while host, username or password
    /// is missing. Read before every pass so saved settings apply at once.
    fn config(&self) -> impl Future<Output = Result<Option<ImapConfig>, Self::Error>> + Send;

    /// The stored cursor of `folder` in `mailbox`.
    fn cursor(
        &self,
        mailbox: &str,
        folder: &str,
    ) -> impl Future<Output = Result<Option<Cursor>, Self::Error>> + Send;

    /// Store the cursor. Records of another UIDVALIDITY generation are stale
    /// and may be forgotten.
    fn save_cursor(
        &self,
        mailbox: &str,
        folder: &str,
        cursor: Cursor,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Forget the cursor, the skipped messages and the retry counts of
    /// `folder`, so the next pass reads it from its first message again.
    fn reset_folder(
        &self,
        mailbox: &str,
        folder: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Record a message the cursor is about to skip, with the reason.
    fn quarantine(
        &self,
        message: MessageRef<'_>,
        error: &str,
        now: i64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// How many messages of this folder generation were skipped so far.
    fn quarantined_count(
        &self,
        mailbox: &str,
        folder: &str,
        uid_validity: u32,
    ) -> impl Future<Output = Result<i64, Self::Error>> + Send;

    /// Failed attempts on `key` so far (the retry count, not the quarantine).
    fn attempts(&self, key: &AttemptKey) -> impl Future<Output = Result<u32, Self::Error>> + Send;

    /// Count one more failed attempt on `key`; returns the new total.
    fn record_attempt(
        &self,
        key: &AttemptKey,
        now: i64,
    ) -> impl Future<Output = Result<u32, Self::Error>> + Send;

    /// Forget the attempts on `key` (it was imported or quarantined).
    fn clear_attempts(
        &self,
        key: &AttemptKey,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Store one parsed report.
    fn ingest(&self, report: &Report)
        -> impl Future<Output = Result<Ingested, Self::Error>> + Send;

    /// Store the reports of one batch, in order, answering for each what
    /// [`Self::ingest`] would. A backend that can should write them at once
    /// (the server: one database transaction), and store nothing of them on
    /// an `Err`; the pass then stores the batch's messages one by one through
    /// [`Self::ingest`], so each failure is counted against its own message.
    /// The default stores them one by one.
    fn ingest_batch(
        &self,
        reports: &[&Report],
    ) -> impl Future<Output = Result<Vec<Ingested>, Self::Error>> + Send {
        async move {
            let mut answers = Vec::with_capacity(reports.len());
            for report in reports {
                answers.push(self.ingest(report).await?);
            }
            Ok(answers)
        }
    }

    /// Record how a pass ended.
    fn record_pass(
        &self,
        record: &PassRecord,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// The current time in unix seconds.
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

#[cfg(test)]
pub(crate) mod memory {
    //! An in-memory [`Backend`] for tests, with failure injection.

    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Quarantined {
        pub uid_validity: u32,
        pub uid: u32,
        pub error: String,
    }

    /// How the memory backend answers `ingest` for one `report_id`.
    #[derive(Debug, Clone)]
    pub enum IngestRule {
        /// An environment failure: the pass fails and retries.
        Fail(String),
        /// Bad content: quarantined.
        Reject(String),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Failure(pub String);

    impl std::fmt::Display for Failure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for Failure {}

    #[derive(Default)]
    pub struct State {
        pub config: Option<ImapConfig>,
        pub cursors: HashMap<(String, String), Cursor>,
        pub quarantine: Vec<Quarantined>,
        pub attempts: HashMap<AttemptKey, u32>,
        pub reports: HashSet<String>,
        pub passes: Vec<PassRecord>,
        pub ingest_rules: HashMap<String, IngestRule>,
        /// Every ingest fails (a database that is down).
        pub ingest_down: Option<String>,
        /// Calls of `ingest` (one report each) and of `ingest_batch`.
        pub ingest_calls: usize,
        pub batch_calls: usize,
    }

    #[derive(Clone, Default)]
    pub struct MemoryBackend(pub Arc<Mutex<State>>);

    impl MemoryBackend {
        pub fn with_config(cfg: ImapConfig) -> Self {
            let b = Self::default();
            b.state().config = Some(cfg);
            b
        }

        pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        pub fn cursor_of(&self, folder: &str) -> Option<Cursor> {
            self.state()
                .cursors
                .iter()
                .find(|((_, f), _)| f == folder)
                .map(|(_, c)| *c)
        }

        pub fn report_count(&self) -> usize {
            self.state().reports.len()
        }

        pub fn quarantine_reason(&self, uid: u32) -> Option<String> {
            self.state()
                .quarantine
                .iter()
                .find(|q| q.uid == uid)
                .map(|q| q.error.clone())
        }

        pub fn attempts_of(&self, uid: u32) -> Option<u32> {
            self.state()
                .attempts
                .iter()
                .find(|(k, _)| k.kind == AttemptKind::Message(uid))
                .map(|(_, n)| *n)
        }
    }

    impl Backend for MemoryBackend {
        type Error = Failure;

        async fn config(&self) -> Result<Option<ImapConfig>, Failure> {
            Ok(self.state().config.clone().filter(ImapConfig::is_complete))
        }

        async fn cursor(&self, mailbox: &str, folder: &str) -> Result<Option<Cursor>, Failure> {
            Ok(self
                .state()
                .cursors
                .get(&(mailbox.to_owned(), folder.to_owned()))
                .copied())
        }

        async fn save_cursor(
            &self,
            mailbox: &str,
            folder: &str,
            cursor: Cursor,
        ) -> Result<(), Failure> {
            let mut s = self.state();
            s.cursors
                .insert((mailbox.to_owned(), folder.to_owned()), cursor);
            s.quarantine
                .retain(|q| q.uid_validity == cursor.uid_validity);
            s.attempts
                .retain(|k, _| k.uid_validity == cursor.uid_validity);
            Ok(())
        }

        async fn reset_folder(&self, mailbox: &str, folder: &str) -> Result<(), Failure> {
            let mut s = self.state();
            s.cursors.remove(&(mailbox.to_owned(), folder.to_owned()));
            s.quarantine.clear();
            s.attempts.retain(|k, _| k.folder != folder);
            Ok(())
        }

        async fn quarantine(
            &self,
            message: MessageRef<'_>,
            error: &str,
            _now: i64,
        ) -> Result<(), Failure> {
            self.state().quarantine.push(Quarantined {
                uid_validity: message.uid_validity,
                uid: message.uid,
                error: error.to_owned(),
            });
            Ok(())
        }

        async fn quarantined_count(
            &self,
            _mailbox: &str,
            _folder: &str,
            uid_validity: u32,
        ) -> Result<i64, Failure> {
            Ok(self
                .state()
                .quarantine
                .iter()
                .filter(|q| q.uid_validity == uid_validity)
                .count() as i64)
        }

        async fn attempts(&self, key: &AttemptKey) -> Result<u32, Failure> {
            Ok(self.state().attempts.get(key).copied().unwrap_or(0))
        }

        async fn record_attempt(&self, key: &AttemptKey, _now: i64) -> Result<u32, Failure> {
            let mut s = self.state();
            let n = s.attempts.entry(key.clone()).or_insert(0);
            *n += 1;
            Ok(*n)
        }

        async fn clear_attempts(&self, key: &AttemptKey) -> Result<(), Failure> {
            self.state().attempts.remove(key);
            Ok(())
        }

        async fn ingest(&self, report: &Report) -> Result<Ingested, Failure> {
            let mut s = self.state();
            s.ingest_calls += 1;
            if let Some(reason) = &s.ingest_down {
                return Err(Failure(reason.clone()));
            }
            match s.ingest_rules.get(&report.report_id).cloned() {
                Some(IngestRule::Fail(reason)) => return Err(Failure(reason)),
                Some(IngestRule::Reject(reason)) => return Ok(Ingested::Rejected(reason)),
                None => {}
            }
            Ok(if s.reports.insert(report.report_id.clone()) {
                Ingested::Inserted
            } else {
                Ingested::Duplicate
            })
        }

        /// All or nothing, like the server's transaction.
        async fn ingest_batch(&self, reports: &[&Report]) -> Result<Vec<Ingested>, Failure> {
            let mut s = self.state();
            s.batch_calls += 1;
            if let Some(reason) = &s.ingest_down {
                return Err(Failure(reason.clone()));
            }
            let mut stored = s.reports.clone();
            let mut answers = Vec::with_capacity(reports.len());
            for report in reports {
                answers.push(match s.ingest_rules.get(&report.report_id).cloned() {
                    Some(IngestRule::Fail(reason)) => return Err(Failure(reason)),
                    Some(IngestRule::Reject(reason)) => Ingested::Rejected(reason),
                    None if stored.insert(report.report_id.clone()) => Ingested::Inserted,
                    None => Ingested::Duplicate,
                });
            }
            s.reports = stored;
            Ok(answers)
        }

        async fn record_pass(&self, record: &PassRecord) -> Result<(), Failure> {
            self.state().passes.push(record.clone());
            Ok(())
        }
    }
}
