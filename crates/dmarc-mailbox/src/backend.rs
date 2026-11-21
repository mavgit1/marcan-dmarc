//! What a pass needs from its host: the mailboxes and their configuration,
//! where the cursor and attempt counts live, where skipped messages and pass
//! outcomes are recorded, and where the reports go. The server implements
//! this over its database; tests use an in-memory one.
//!
//! Every write of a pass carries the [`MailboxConfig::epoch`] the host handed
//! out with its configuration, so a host can drop the writes of a pass that
//! was prepared before the mailbox's stored state was replaced.

use crate::graph::GraphConfig;
use crate::imap::config::ImapConfig;
use crate::pass::FetchSummary;
use dmarc_parser::Report;
use std::future::Future;
use std::time::Duration;

/// A mailbox the worker polls, as [`Backend::mailboxes`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxRef {
    /// Its cursor, quarantine, retry counts and passes are stored under it.
    pub id: String,
    /// From the end of one pass to the start of the next.
    pub poll: Duration,
}

/// The stored position of one folder: where the next pass continues, and
/// the folder generation it belongs to. Both are opaque text to the host;
/// the source that wrote them reads them back (IMAP: the UIDVALIDITY and
/// the last handled UID, in decimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// The folder generation: when the folder answers with another one, the
    /// stored position, skipped messages and retry counts are void (IMAP:
    /// its UIDVALIDITY).
    pub generation: String,
    /// Where the next pass continues (IMAP: every UID up to this one of the
    /// generation has been handled).
    pub position: String,
}

/// One message of one folder generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef<'a> {
    pub mailbox: &'a str,
    /// The pass's [`MailboxConfig::epoch`].
    pub epoch: i64,
    pub folder: &'a str,
    /// The folder generation ([`Cursor::generation`]).
    pub generation: &'a str,
    /// The message's id in that generation (IMAP: its UID in decimal).
    pub message_id: &'a str,
}

/// What an attempt count is about: one message, or one download batch (named
/// by its last message), which is split into single messages once it failed.
/// Each holds a message id as [`MessageRef::message_id`] spells it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttemptKind {
    Message(String),
    Batch(String),
}

impl AttemptKind {
    /// The stored name of the kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::Batch(_) => "batch",
        }
    }

    /// The id of the message the count is keyed by.
    pub fn message_id(&self) -> &str {
        match self {
            Self::Message(id) | Self::Batch(id) => id,
        }
    }
}

/// Identity of an attempt count: folder generation plus what it is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttemptKey {
    pub mailbox: String,
    /// The pass's [`MailboxConfig::epoch`].
    pub epoch: i64,
    pub folder: String,
    /// The folder generation ([`Cursor::generation`]).
    pub generation: String,
    pub kind: AttemptKind,
}

/// What a pass reads, as [`Backend::prepare`] hands it out: one variant per
/// way of reading a mailbox, each with its own connection settings and the
/// id, epoch and folder every pass needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailboxConfig {
    /// A folder read over IMAP.
    Imap(ImapConfig),
    /// A Microsoft 365 mailbox's folder read over Microsoft Graph.
    Graph(GraphConfig),
}

impl From<ImapConfig> for MailboxConfig {
    fn from(cfg: ImapConfig) -> Self {
        Self::Imap(cfg)
    }
}

impl From<GraphConfig> for MailboxConfig {
    fn from(cfg: GraphConfig) -> Self {
        Self::Graph(cfg)
    }
}

impl MailboxConfig {
    /// The mailbox's id ([`MailboxRef::id`]).
    pub fn id(&self) -> &str {
        match self {
            Self::Imap(cfg) => &cfg.id,
            Self::Graph(cfg) => &cfg.id,
        }
    }

    /// Which generation of the mailbox's stored state the host prepared
    /// this pass for (see [`ImapConfig::epoch`]).
    pub fn epoch(&self) -> i64 {
        match self {
            Self::Imap(cfg) => cfg.epoch,
            Self::Graph(cfg) => cfg.epoch,
        }
    }

    /// The folder the pass reads.
    pub fn folder(&self) -> &str {
        match self {
            Self::Imap(cfg) => &cfg.folder,
            Self::Graph(cfg) => &cfg.folder,
        }
    }

    /// The server the pass talks to, for the log.
    pub fn host(&self) -> &str {
        match self {
            Self::Imap(cfg) => &cfg.host,
            Self::Graph(cfg) => cfg.api.host(),
        }
    }

    /// `text` with the credentials masked, for errors that reach the UI.
    pub fn redact(&self, text: &str) -> String {
        match self {
            Self::Imap(cfg) => cfg.redact(text),
            Self::Graph(cfg) => cfg.redact(text),
        }
    }

    /// The IMAP settings, when the pass reads over IMAP (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn as_imap(&self) -> Option<&ImapConfig> {
        match self {
            Self::Imap(cfg) => Some(cfg),
            Self::Graph(_) => None,
        }
    }

    /// The Graph settings, when the pass reads over Graph (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn as_graph(&self) -> Option<&GraphConfig> {
        match self {
            Self::Graph(cfg) => Some(cfg),
            Self::Imap(_) => None,
        }
    }
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
    /// The id of the mailbox the pass read.
    pub mailbox: String,
    /// The [`MailboxConfig::epoch`] the pass was prepared under; `None` when
    /// it ended before it had a configuration.
    pub epoch: Option<i64>,
    pub started_at: i64,
    pub finished_at: i64,
    /// When the pass reached the mailbox search; `None` when it never got there.
    pub checked_at: Option<i64>,
    pub summary: FetchSummary,
    /// Why the pass failed, password or token redacted; `None` on success.
    pub error: Option<String>,
    /// The server's own answer beside `error` (redacted too), when it gave
    /// one: shown behind Details, never in the sentence.
    pub error_detail: Option<String>,
}

/// The host side of a pass. Every method is a small, idempotent operation;
/// an `Err` is an environment failure (the database is unavailable, say)
/// that makes the pass fail without moving the cursor, so it is retried.
///
/// A write that carries an epoch (the cursor, a skipped message, a retry
/// count, a pass record) may be dropped silently when the mailbox's state
/// has moved on to another epoch, or the mailbox is gone: the pass that made
/// it reads a mailbox that is no longer configured.
pub trait Backend: Clone + Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// The mailboxes to poll, in the order they first take turns: those
    /// with a complete login. Read before every pass, so a mailbox added,
    /// changed or removed in the settings applies at once; an empty list
    /// means nothing is configured.
    fn mailboxes(&self) -> impl Future<Output = Result<Vec<MailboxRef>, Self::Error>> + Send;

    /// The configuration of mailbox `id` (its [`MailboxConfig::id`] is `id`)
    /// for the pass about to start, credentials ready (a host may fetch an
    /// access token here) and its current [`MailboxConfig::epoch`]; `None` when
    /// it cannot be polled after all (its login went missing, it was
    /// removed): the pass counts as unconfigured.
    fn prepare(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<Option<MailboxConfig>, Self::Error>> + Send;

    /// The stored cursor of `folder` in `mailbox`.
    fn cursor(
        &self,
        mailbox: &str,
        folder: &str,
    ) -> impl Future<Output = Result<Option<Cursor>, Self::Error>> + Send;

    /// Store the cursor, written under `epoch`. Skipped messages and retry
    /// counts of another [`Cursor::generation`] are stale and may be
    /// forgotten.
    fn save_cursor(
        &self,
        mailbox: &str,
        epoch: i64,
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
        generation: &str,
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
    fn ingest_batch(
        &self,
        reports: &[&Report],
    ) -> impl Future<Output = Result<Vec<Ingested>, Self::Error>> + Send;

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
        pub generation: String,
        pub message_id: String,
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
        /// Every mailbox with its interval, complete or not, in listing order.
        /// Without any (a pass test hands its configuration in directly),
        /// every write lands.
        pub mailboxes: Vec<(ImapConfig, Duration)>,
        /// Mailboxes removed ([`MemoryBackend::remove`]): no write lands.
        pub removed: HashSet<String>,
        pub cursors: HashMap<(String, String), Cursor>,
        pub quarantine: Vec<Quarantined>,
        pub attempts: HashMap<AttemptKey, u32>,
        pub reports: HashSet<String>,
        pub passes: Vec<PassRecord>,
        pub ingest_rules: HashMap<String, IngestRule>,
        /// Every ingest fails (a database that is down).
        pub ingest_down: Option<String>,
        /// Every prepare fails (a token endpoint that refuses).
        pub prepare_down: Option<String>,
        /// Calls of `ingest` (one report each) and of `ingest_batch`.
        pub ingest_calls: usize,
        pub batch_calls: usize,
    }

    #[derive(Clone, Default)]
    pub struct MemoryBackend(pub Arc<Mutex<State>>);

    impl MemoryBackend {
        /// One mailbox, polled every five minutes.
        pub fn with_config(cfg: ImapConfig) -> Self {
            Self::with_mailboxes(vec![(cfg, Duration::from_secs(300))])
        }

        pub fn with_mailboxes(mailboxes: Vec<(ImapConfig, Duration)>) -> Self {
            let b = Self::default();
            b.state().mailboxes = mailboxes;
            b
        }

        /// The ids of the recorded passes' mailboxes, in order.
        pub fn pass_order(&self) -> Vec<String> {
            self.state()
                .passes
                .iter()
                .map(|p| p.mailbox.clone())
                .collect()
        }

        pub fn cursor_in(&self, mailbox: &str, folder: &str) -> Option<Cursor> {
            self.state()
                .cursors
                .get(&(mailbox.to_owned(), folder.to_owned()))
                .cloned()
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
                .map(|(_, c)| c.clone())
        }

        pub fn report_count(&self) -> usize {
            self.state().reports.len()
        }

        pub fn quarantine_reason(&self, uid: u32) -> Option<String> {
            let id = uid.to_string();
            self.state()
                .quarantine
                .iter()
                .find(|q| q.message_id == id)
                .map(|q| q.error.clone())
        }

        /// What the server's save does when mailbox `id` reads another
        /// mailbox now: `change` its configuration, move it to the next
        /// epoch and forget its cursor, skipped messages, retry counts and
        /// passes, all at once.
        pub fn replace(&self, id: &str, change: impl FnOnce(&mut ImapConfig)) {
            let mut s = self.state();
            let (cfg, _) = s
                .mailboxes
                .iter_mut()
                .find(|(cfg, _)| cfg.id == id)
                .expect("listed mailbox");
            change(cfg);
            cfg.epoch += 1;
            s.cursors.retain(|(mailbox, _), _| mailbox != id);
            s.quarantine.clear();
            s.attempts.retain(|k, _| k.mailbox != id);
            s.passes.retain(|p| p.mailbox != id);
        }

        /// What the server's delete does: mailbox `id` is gone.
        pub fn remove(&self, id: &str) {
            let mut s = self.state();
            s.mailboxes.retain(|(cfg, _)| cfg.id != id);
            s.removed.insert(id.to_owned());
        }

        pub fn attempts_of(&self, uid: u32) -> Option<u32> {
            self.state()
                .attempts
                .iter()
                .find(|(k, _)| k.kind == AttemptKind::Message(uid.to_string()))
                .map(|(_, n)| *n)
        }
    }

    impl State {
        /// Whether a write under `epoch` lands: mailbox `id` is listed with
        /// that epoch (or nothing is listed and it was not removed).
        fn current(&self, id: &str, epoch: Option<i64>) -> bool {
            (self.mailboxes.is_empty() && !self.removed.contains(id))
                || self
                    .mailboxes
                    .iter()
                    .any(|(cfg, _)| cfg.id == id && epoch.is_none_or(|e| e == cfg.epoch))
        }
    }

    impl Backend for MemoryBackend {
        type Error = Failure;

        async fn mailboxes(&self) -> Result<Vec<MailboxRef>, Failure> {
            Ok(self
                .state()
                .mailboxes
                .iter()
                .filter(|(cfg, _)| cfg.is_complete())
                .map(|(cfg, poll)| MailboxRef {
                    id: cfg.id.clone(),
                    poll: *poll,
                })
                .collect())
        }

        async fn prepare(&self, id: &str) -> Result<Option<MailboxConfig>, Failure> {
            if let Some(reason) = &self.state().prepare_down {
                return Err(Failure(reason.clone()));
            }
            Ok(self
                .state()
                .mailboxes
                .iter()
                .map(|(cfg, _)| cfg)
                .find(|cfg| cfg.id == id && cfg.is_complete())
                .cloned()
                .map(MailboxConfig::Imap))
        }

        async fn cursor(&self, mailbox: &str, folder: &str) -> Result<Option<Cursor>, Failure> {
            Ok(self
                .state()
                .cursors
                .get(&(mailbox.to_owned(), folder.to_owned()))
                .cloned())
        }

        async fn save_cursor(
            &self,
            mailbox: &str,
            epoch: i64,
            folder: &str,
            cursor: Cursor,
        ) -> Result<(), Failure> {
            let mut s = self.state();
            if !s.current(mailbox, Some(epoch)) {
                return Ok(());
            }
            s.quarantine.retain(|q| q.generation == cursor.generation);
            s.attempts.retain(|k, _| k.generation == cursor.generation);
            s.cursors
                .insert((mailbox.to_owned(), folder.to_owned()), cursor);
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
            let mut s = self.state();
            if !s.current(message.mailbox, Some(message.epoch)) {
                return Ok(());
            }
            s.quarantine.push(Quarantined {
                generation: message.generation.to_owned(),
                message_id: message.message_id.to_owned(),
                error: error.to_owned(),
            });
            Ok(())
        }

        async fn quarantined_count(
            &self,
            _mailbox: &str,
            _folder: &str,
            generation: &str,
        ) -> Result<i64, Failure> {
            Ok(self
                .state()
                .quarantine
                .iter()
                .filter(|q| q.generation == generation)
                .count() as i64)
        }

        async fn attempts(&self, key: &AttemptKey) -> Result<u32, Failure> {
            Ok(self.state().attempts.get(key).copied().unwrap_or(0))
        }

        async fn record_attempt(&self, key: &AttemptKey, _now: i64) -> Result<u32, Failure> {
            let mut s = self.state();
            if !s.current(&key.mailbox, Some(key.epoch)) {
                return Ok(0);
            }
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
            let mut s = self.state();
            if s.current(&record.mailbox, record.epoch) {
                s.passes.push(record.clone());
            }
            Ok(())
        }
    }
}
