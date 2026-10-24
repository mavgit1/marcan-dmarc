//! One fetch pass: connect, find the messages above the cursor, download and
//! ingest them batch by batch, and move the cursor after every batch.
//!
//! Progress is tracked via a per-folder UID cursor the [`Backend`] keeps, not
//! via the IMAP `\Seen` flag. The mailbox is treated as read-only: we only use
//! `BODY.PEEK[]` and never `STORE` flags, so other clients reading the same
//! mailbox cannot starve us of mail by marking messages seen.
//!
//! # Large mailboxes
//!
//! A pass imports in bounded, resumable steps so a mailbox with tens of
//! thousands of reports imports as reliably as one with ten:
//!
//! 1. `UID SEARCH` for UIDs above the cursor, then `UID FETCH (UID RFC822.SIZE)`
//!    in chunks to learn each message's size without downloading it.
//! 2. [`plan_batches`] splits the UIDs into batches (by count and bytes);
//!    messages over the size cap are quarantined, never downloaded.
//! 3. Each batch's bodies are streamed and parsed one message at a time, then
//!    stored together ([`Backend::ingest_batch`]: one database transaction in
//!    the server, not one per report), and the cursor is persisted after every
//!    batch. A crash, timeout or restart resumes after the last completed
//!    batch instead of starting over.
//!
//! # Memory
//!
//! Attachments are small, but what they unpack to need not be: a 200 KB gzip
//! can hold a 50 MB report with 700,000 records, about 250 MB once parsed.
//! So the decoded XML is bounded at three levels: one report at
//! [`dmarc_parser::MAX_REPORT_BYTES`], one message's reports together at the
//! same [`MAX_MESSAGE_XML_BYTES`] (the rest of its attachments are refused),
//! and a batch is stored early, before its next message is parsed, once its
//! reports hold [`Limits::max_pending_xml_bytes`].
//!
//! Timeouts apply per command and per batch, never to the whole pass, so a long
//! import is fine as long as it keeps moving.
//!
//! # Cursor advancement vs failures
//!
//! The cursor advances past every message of a completed batch **regardless of
//! individual parse/ingest failures**. A message that hard-fails will **not** be
//! retried automatically: one permanently bad message must not block all
//! subsequent mail forever. Because nothing retries it, the skip is recorded
//! through [`Backend::quarantine`] as well as logged.
//!
//! A batch that fails as a whole (timeout, dropped connection, backend error)
//! does **not** move the cursor; the next pass fetches it again. A message
//! whose error repeats on [`MAX_ATTEMPTS`] consecutive passes is quarantined,
//! so a misclassified error cannot block newer mail. The counts live in the
//! backend, so a restart does not reset them.

use crate::attach::extract_report_attachments;
use crate::backend::{AttemptKey, AttemptKind, Backend, Cursor, Ingested, MessageRef};
use crate::batch::{compact_uid_set, format_bytes, plan_batches, too_large_reason, Batch, Limits};
use crate::config::ImapConfig;
use crate::connect::{connect_and_login, ConnectStage};
use crate::progress::{ImapPhase, SharedProgress};
use dmarc_parser::{parse_report_within, ParseError, Report, MAX_REPORT_BYTES};
use futures::{Future, TryStreamExt};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// How many passes in a row may fail on the same message (a backend error,
/// no answer from the server, a download that times out) before it is
/// quarantined and skipped, so one message can never block newer mail.
pub const MAX_ATTEMPTS: u32 = 5;

/// The most XML the reports of one message may unpack to together: as much
/// as one report may. A message with more report attachments than that is
/// quarantined; the reports before the limit are imported.
pub const MAX_MESSAGE_XML_BYTES: u64 = MAX_REPORT_BYTES;

/// Counts from a single fetch pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FetchSummary {
    /// Messages handled in this pass (UID > stored cursor): processed,
    /// quarantined, or found deleted.
    pub messages_seen: u32,
    /// Newly inserted reports.
    pub reports_ingested: u32,
    /// Reports skipped because `report_id` already existed.
    pub reports_duplicate: u32,
    /// Report attachments that could not be imported: refused, unparsable,
    /// rejected by the backend, or whose message was given up on.
    pub errors: u32,
    /// Messages quarantined because the cursor moved past a failure.
    pub messages_quarantined: u32,
    /// Cursor `last_uid` at the start of this pass (`None` if starting from 0 / reset).
    pub starting_uid: Option<u32>,
    /// Cursor `last_uid` persisted by this pass (`None` if still 0).
    pub ending_uid: Option<u32>,
    /// The pass stopped early between batches because it was cancelled (the
    /// settings changed, or the process is shutting down).
    pub interrupted: bool,
}

impl FetchSummary {
    /// Whether this pass moved the cursor forward.
    pub fn advanced(&self) -> bool {
        self.ending_uid.unwrap_or(0) > self.starting_uid.unwrap_or(0)
    }
}

impl fmt::Display for FetchSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "messages_seen={}, reports_ingested={}, reports_duplicate={}, errors={}, messages_quarantined={}, starting_uid={:?}, ending_uid={:?}, interrupted={}",
            self.messages_seen,
            self.reports_ingested,
            self.reports_duplicate,
            self.errors,
            self.messages_quarantined,
            self.starting_uid,
            self.ending_uid,
            self.interrupted
        )
    }
}

/// Why a pass failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Reaching or logging in to the server failed at this stage.
    Connect(ConnectStage),
    /// A command or a batch did not finish in time.
    Timeout,
    /// The server answered with an error, or without the data it should have.
    Server,
    /// The backend (the database) failed.
    Storage,
}

/// A pass that failed, with what it achieved before failing. Batches completed
/// before the error are kept: the cursor was persisted after each one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub summary: FetchSummary,
    pub kind: FailureKind,
    /// What went wrong, for the log and the UI (password redacted by the caller).
    pub message: String,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if self.summary.advanced() {
            write!(
                f,
                " (progress saved: {} messages processed, {} reports imported; the next pass continues from there)",
                self.summary.messages_seen, self.summary.reports_ingested
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for FetchError {}

/// A failure inside a pass, before the summary is attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub kind: FailureKind,
    pub message: String,
}

impl Failure {
    fn timeout(what: &str, limit: Duration) -> Self {
        Self {
            kind: FailureKind::Timeout,
            message: format!("{what} timed out after {} seconds", limit.as_secs()),
        }
    }

    fn server(what: &str, detail: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Server,
            message: format!("{what} failed: {detail}"),
        }
    }

    fn storage(what: &str, detail: impl fmt::Display) -> Self {
        Self {
            kind: FailureKind::Storage,
            message: format!("{what}: {detail}"),
        }
    }

    fn message(kind: FailureKind, message: String) -> Self {
        Self { kind, message }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

type PassResult<T> = Result<T, Failure>;

/// The CPU-bound half of a message (MIME, decompression, XML): the reports
/// found, and the reasons any report-looking attachment was bad.
pub(crate) type Parser = Arc<dyn Fn(&[u8]) -> (Vec<Report>, Vec<String>) + Send + Sync>;

pub(crate) fn default_parser() -> Parser {
    Arc::new(parse_message)
}

/// Where one batch's time went, for the per-batch log line. The rest of the
/// batch's time is spent reading the `UID FETCH` answer: the mail server and
/// the network.
#[derive(Debug, Default, Clone, Copy)]
struct BatchTimings {
    /// MIME, decompression and XML on the blocking pool.
    parse: Duration,
    /// Every backend call: reports, retry counts, quarantine, the cursor.
    store: Duration,
}

/// Run `fut` (one IMAP command) with a timeout and a readable error.
async fn timed<T, E>(
    limit: Duration,
    what: &str,
    fut: impl Future<Output = Result<T, E>>,
) -> PassResult<T>
where
    E: fmt::Display,
{
    match tokio::time::timeout(limit, fut).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Failure::server(what, e)),
        Err(_) => Err(Failure::timeout(what, limit)),
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// How a pass reaches the mailbox.
pub(crate) enum Transport {
    /// TCP and TLS (or STARTTLS) to the configured server.
    Network,
    /// The scripted in-memory server, for other crates' tests.
    #[cfg(feature = "test-support")]
    Fake(crate::fake_server::FakeServer),
}

/// Everything one pass works with. Generic over the backend (the server's
/// database, or memory in tests) and, inside, over the IMAP transport.
pub(crate) struct Pass<'a, B: Backend> {
    pub backend: &'a B,
    pub cfg: &'a ImapConfig,
    pub progress: &'a SharedProgress,
    /// Cancelled when the pass should stop between batches (the settings
    /// changed, or the process is shutting down); the cursor is kept.
    pub cancel: &'a CancellationToken,
    pub limits: &'a Limits,
    pub parser: &'a Parser,
    pub summary: FetchSummary,
    /// When the pass reached the mailbox search.
    pub checked_at: Option<i64>,
    /// The folder's UIDVALIDITY once EXAMINE answered.
    uid_validity: Option<u32>,
    /// The running batch's parse and store time.
    timings: BatchTimings,
    /// The running batch's messages that have failed attempts recorded, so
    /// only theirs are cleared once they are stored.
    retried: HashSet<u32>,
}

/// One downloaded message, parsed, waiting to be stored with its batch.
struct ParsedMessage {
    uid: u32,
    /// Its size, for the progress.
    bytes: u64,
    reports: Vec<Report>,
    /// Why report-looking attachments were bad.
    failures: Vec<String>,
}

impl<'a, B: Backend> Pass<'a, B> {
    pub(crate) fn new(
        backend: &'a B,
        cfg: &'a ImapConfig,
        progress: &'a SharedProgress,
        cancel: &'a CancellationToken,
        limits: &'a Limits,
        parser: &'a Parser,
    ) -> Self {
        Self {
            backend,
            cfg,
            progress,
            cancel,
            limits,
            parser,
            summary: FetchSummary::default(),
            checked_at: None,
            uid_validity: None,
            timings: BatchTimings::default(),
            retried: HashSet::new(),
        }
    }

    fn update(&self, f: impl FnOnce(&mut crate::progress::ImapProgress)) {
        self.progress.update(self.backend.now(), f);
    }

    fn mailbox_key(&self) -> String {
        self.cfg.mailbox_key()
    }

    fn message_key(&self, uid_validity: u32, uid: u32) -> AttemptKey {
        AttemptKey {
            mailbox: self.mailbox_key(),
            folder: self.cfg.folder.clone(),
            uid_validity,
            kind: AttemptKind::Message(uid),
        }
    }

    fn batch_key(&self, uid_validity: u32, last_uid: u32) -> AttemptKey {
        AttemptKey {
            mailbox: self.mailbox_key(),
            folder: self.cfg.folder.clone(),
            uid_validity,
            kind: AttemptKind::Batch(last_uid),
        }
    }

    async fn attempts(&self, key: &AttemptKey) -> PassResult<u32> {
        self.backend
            .attempts(key)
            .await
            .map_err(|e| Failure::storage("failed to read the retry count", e))
    }

    async fn record_attempt(&self, key: &AttemptKey) -> PassResult<u32> {
        self.backend
            .record_attempt(key, self.backend.now())
            .await
            .map_err(|e| Failure::storage("failed to record the retry count", e))
    }

    async fn clear_attempts(&self, key: &AttemptKey) -> PassResult<()> {
        self.backend
            .clear_attempts(key)
            .await
            .map_err(|e| Failure::storage("failed to clear the retry count", e))
    }

    /// Connect, log in, run [`Self::import`], log out.
    pub(crate) async fn run(&mut self, transport: Transport) -> Result<FetchSummary, FetchError> {
        self.update(|p| p.phase = ImapPhase::Connecting);
        match transport {
            Transport::Network => {}
            #[cfg(feature = "test-support")]
            Transport::Fake(server) => {
                let (session, _log) = crate::fake_server::session(server).await;
                return self.run_on(session).await;
            }
        }
        let cfg = self.cfg;
        info!(host = %cfg.host, port = cfg.port, folder = %cfg.folder, "connecting to IMAP");
        let connected =
            tokio::time::timeout(self.limits.command_timeout, connect_and_login(cfg)).await;
        let session = match connected {
            Ok(Ok(session)) => session,
            Ok(Err(e)) => {
                return Err(FetchError {
                    summary: self.summary,
                    kind: FailureKind::Connect(e.stage),
                    message: e.to_string(),
                })
            }
            Err(_) => {
                return Err(FetchError {
                    summary: self.summary,
                    kind: FailureKind::Timeout,
                    message: format!(
                        "connecting to {}:{} timed out after {} seconds",
                        cfg.host,
                        cfg.port,
                        self.limits.command_timeout.as_secs()
                    ),
                })
            }
        };
        self.run_on(session).await
    }

    /// [`Self::import`] on a logged-in session, then log out.
    async fn run_on<T>(
        &mut self,
        mut session: async_imap::Session<T>,
    ) -> Result<FetchSummary, FetchError>
    where
        T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
    {
        let cfg = self.cfg;
        let result = self.import(&mut session).await;
        if result.is_ok() {
            match tokio::time::timeout(self.limits.command_timeout, session.logout()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn!(error = %e, "IMAP LOGOUT failed"),
                Err(_) => warn!("IMAP LOGOUT timed out"),
            }
        }
        // After a failure the connection may be mid-response: simply dropped.
        self.report_quarantine().await;

        let summary = self.summary;
        match result {
            Ok(()) => {
                info!(%summary, "fetch cycle complete");
                Ok(summary)
            }
            Err(failure) => {
                error!(
                    host = %cfg.host,
                    folder = %cfg.folder,
                    error = %failure,
                    %summary,
                    "IMAP fetch pass failed"
                );
                Err(FetchError {
                    summary,
                    kind: failure.kind,
                    message: failure.message,
                })
            }
        }
    }

    /// Report the standing total, not just this pass: a single skipped report
    /// is easy to miss in one log line, and the point of the record is that
    /// the count persists.
    async fn report_quarantine(&self) {
        if self.summary.messages_quarantined == 0 {
            return;
        }
        let Some(uid_validity) = self.uid_validity else {
            return;
        };
        match self
            .backend
            .quarantined_count(&self.mailbox_key(), &self.cfg.folder, uid_validity)
            .await
        {
            Ok(total) => warn!(
                folder = %self.cfg.folder,
                this_pass = self.summary.messages_quarantined,
                total,
                "messages quarantined: these reports were skipped and will not be retried"
            ),
            Err(e) => warn!(error = %e, "failed to count quarantined messages"),
        }
    }

    /// The body of a pass on an authenticated session. Generic over the
    /// transport so tests can drive it against an in-memory fake server.
    pub(crate) async fn import<T>(&mut self, session: &mut async_imap::Session<T>) -> PassResult<()>
    where
        T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
    {
        self.update(|p| p.phase = ImapPhase::Scanning);
        let cfg = self.cfg;
        let limits = self.limits;

        // EXAMINE opens the folder read-only: Marcan never changes the mailbox.
        let examine = format!("EXAMINE {}", cfg.folder);
        let mailbox = timed(
            limits.command_timeout,
            &examine,
            session.examine(&cfg.folder),
        )
        .await?;
        let uid_validity = mailbox.uid_validity.ok_or_else(|| {
            Failure::message(
                FailureKind::Server,
                format!(
                    "EXAMINE {} did not return UIDVALIDITY; cannot track UID cursor",
                    cfg.folder
                ),
            )
        })?;
        self.uid_validity = Some(uid_validity);

        let mailbox_key = self.mailbox_key();
        let stored = self
            .backend
            .cursor(&mailbox_key, &cfg.folder)
            .await
            .map_err(|e| Failure::storage("failed to load IMAP cursor", e))?;

        let last_uid = match stored {
            Some(c) if c.uid_validity == uid_validity => {
                info!(folder = %cfg.folder, uid_validity, last_uid = c.last_uid, "loaded IMAP cursor");
                c.last_uid
            }
            Some(c) => {
                warn!(
                    folder = %cfg.folder,
                    stored_validity = c.uid_validity,
                    uid_validity,
                    "UIDVALIDITY changed; resetting cursor to 0"
                );
                0
            }
            None => {
                info!(folder = %cfg.folder, uid_validity, "no stored IMAP cursor; starting from UID 0");
                0
            }
        };
        if last_uid > 0 {
            self.summary.starting_uid = Some(last_uid);
            self.summary.ending_uid = Some(last_uid);
        }

        // UID SEARCH UID <from>:* — note IMAP's `n:*` quirk can return the highest
        // existing UID even when it is < n, so we always filter to uid > last_uid.
        let search_from = last_uid.saturating_add(1).max(1);
        let search_query = format!("UID {search_from}:*");
        let found = timed(
            limits.command_timeout,
            &format!("IMAP UID SEARCH {search_query}"),
            session.uid_search(&search_query),
        )
        .await?;
        let mut uids: Vec<u32> = found.into_iter().filter(|uid| *uid > last_uid).collect();
        uids.sort_unstable();
        self.checked_at = Some(self.backend.now());

        if uids.is_empty() {
            info!(last_uid, "no new messages above cursor");
            // Persist validity on first run / after reset even with no mail, so we
            // don't keep treating every empty pass as a cold start after a validity change.
            if stored.map(|c| c.uid_validity) != Some(uid_validity) {
                self.save_cursor(uid_validity, last_uid).await?;
            }
            return Ok(());
        }

        self.update(|p| p.total_messages = count_u32(uids.len()));
        info!(
            count = uids.len(),
            last_uid, "reading sizes of new messages"
        );
        let sizes = fetch_sizes(session, &uids, limits).await?;
        let messages: Vec<(u32, Option<u32>)> = uids
            .iter()
            .map(|uid| (*uid, sizes.get(uid).copied()))
            .collect();
        let batches = plan_batches(&messages, limits);
        let total_bytes: u64 = batches.iter().map(|b| b.bytes).sum();

        self.update(|p| {
            p.phase = ImapPhase::Importing;
            p.total_bytes = total_bytes;
        });
        info!(
            count = uids.len(),
            batches = batches.len(),
            total = %format_bytes(total_bytes),
            last_uid,
            "importing new messages"
        );

        for (index, batch) in batches.iter().enumerate() {
            if self.cancel.is_cancelled() {
                info!(
                    completed = index,
                    batches = batches.len(),
                    "pass cancelled; stopping between batches (cursor saved)"
                );
                self.summary.interrupted = true;
                return Ok(());
            }
            let started = Instant::now();
            let seen = self.summary.messages_seen;
            self.timings = BatchTimings::default();
            self.import_batch(session, batch, uid_validity).await?;
            let saving = Instant::now();
            self.save_cursor(uid_validity, batch.last_uid).await?;
            self.timings.store += saving.elapsed();
            self.summary.ending_uid = Some(batch.last_uid);
            let total = started.elapsed();
            let BatchTimings { parse, store } = self.timings;
            info!(
                batch = index + 1,
                batches = batches.len(),
                messages = self.summary.messages_seen - seen,
                bytes = %format_bytes(batch.bytes),
                total_ms = total.as_millis() as u64,
                fetch_ms = total.saturating_sub(parse + store).as_millis() as u64,
                parse_ms = parse.as_millis() as u64,
                store_ms = store.as_millis() as u64,
                cursor = batch.last_uid,
                "IMAP batch imported"
            );
        }
        Ok(())
    }

    async fn save_cursor(&self, uid_validity: u32, last_uid: u32) -> PassResult<()> {
        self.backend
            .save_cursor(
                &self.mailbox_key(),
                &self.cfg.folder,
                Cursor {
                    uid_validity,
                    last_uid,
                },
            )
            .await
            .map_err(|e| Failure::storage("failed to persist IMAP cursor", e))
    }

    /// One batch: skip what keeps timing out, download the rest (one message
    /// at a time after a batch timeout), then quarantine the oversized ones.
    async fn import_batch<T>(
        &mut self,
        session: &mut async_imap::Session<T>,
        batch: &Batch,
        uid_validity: u32,
    ) -> PassResult<()>
    where
        T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
    {
        // A message whose download timed out on MAX_ATTEMPTS passes is skipped
        // without downloading it again.
        let started = Instant::now();
        let mut download = Vec::with_capacity(batch.download.len());
        self.retried.clear();
        for &(uid, size) in &batch.download {
            let key = self.message_key(uid_validity, uid);
            let attempts = self.attempts(&key).await?;
            if attempts > 0 {
                self.retried.insert(uid);
            }
            if attempts < MAX_ATTEMPTS {
                download.push((uid, size));
                continue;
            }
            error!(
                uid,
                "message download keeps timing out; quarantining it so newer mail can be imported"
            );
            self.summary.messages_seen += 1;
            self.summary.errors += 1;
            let reason =
                format!("gave up after {MAX_ATTEMPTS} attempts: the download kept timing out");
            self.quarantine(uid_validity, uid, &reason).await?;
            self.clear_attempts(&key).await?;
            self.update(|p| p.processed_messages += 1);
        }
        self.timings.store += started.elapsed();

        // A batch that timed out before is downloaded one message at a time, so
        // the message causing it is found and counted on its own.
        let batch_key = self.batch_key(uid_validity, batch.last_uid);
        let batch_retried = self.attempts(&batch_key).await? > 0;
        let parts: Vec<Batch> = if batch_retried {
            download.iter().map(|&m| Batch::single(m)).collect()
        } else if download.is_empty() {
            Vec::new()
        } else {
            vec![Batch {
                download,
                ..batch.clone()
            }]
        };
        for part in &parts {
            let limit = self.limits.batch_timeout(part.bytes);
            let fetched =
                tokio::time::timeout(limit, self.fetch_batch(session, part, uid_validity)).await;
            match fetched {
                Ok(result) => result?,
                Err(_) => {
                    let uids = part.download_uids();
                    let attempt = match uids.as_slice() {
                        [uid] => {
                            let key = self.message_key(uid_validity, *uid);
                            self.record_attempt(&key).await?
                        }
                        _ => self.record_attempt(&batch_key).await?,
                    };
                    // The FETCH is still in flight, so the session ends here either way.
                    return Err(Failure::message(
                        FailureKind::Timeout,
                        format!(
                            "IMAP fetch of {} message(s) (UIDs {}–{}, {}) timed out after {} seconds \
                             (attempt {attempt} of {MAX_ATTEMPTS}); will retry",
                            uids.len(),
                            uids.first().copied().unwrap_or_default(),
                            uids.last().copied().unwrap_or_default(),
                            format_bytes(part.bytes),
                            limit.as_secs()
                        ),
                    ));
                }
            }
        }
        if batch_retried {
            self.clear_attempts(&batch_key).await?;
        }

        // After the download succeeded, so a retried batch does not record these twice.
        for &(uid, size) in &batch.too_large {
            warn!(
                uid,
                size, "message exceeds the size limit; quarantined without downloading"
            );
            self.summary.messages_seen += 1;
            self.quarantine(uid_validity, uid, &too_large_reason(size, self.limits))
                .await?;
            self.update(|p| p.processed_messages += 1);
        }
        Ok(())
    }

    /// Download one batch with `BODY.PEEK[]`, parse each message as it
    /// arrives, and store them together once the answer is complete (or
    /// sooner, once their reports hold [`Limits::max_pending_xml_bytes`] of
    /// XML), so at most one batch is ever in flight.
    async fn fetch_batch<T>(
        &mut self,
        session: &mut async_imap::Session<T>,
        batch: &Batch,
        uid_validity: u32,
    ) -> PassResult<()>
    where
        T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
    {
        let uids = batch.download_uids();
        let set = compact_uid_set(&uids);
        // UID requested explicitly so Fetch.uid is populated; BODY.PEEK[] avoids \Seen.
        let mut pending: BTreeMap<u32, u64> = batch.download.iter().copied().collect();
        let mut answered_without_body: HashSet<u32> = HashSet::new();
        let mut parsed = Vec::with_capacity(pending.len());
        // XML held by the reports in `parsed` (see "Memory" above).
        let mut pending_xml: u64 = 0;
        {
            let what = format!("IMAP UID FETCH {set}");
            let mut stream = session
                .uid_fetch(&set, "(UID BODY.PEEK[])")
                .await
                .map_err(|e| Failure::server(&what, e))?;
            while let Some(fetch) = stream
                .try_next()
                .await
                .map_err(|e| Failure::server("reading UID FETCH response", e))?
            {
                // Unsolicited FETCH responses (flag changes by other clients) carry no UID
                // or no body; they are not failures.
                let Some(uid) = fetch.uid else {
                    debug!(seq = fetch.message, "ignoring FETCH response without UID");
                    continue;
                };
                let Some(&size) = pending.get(&uid) else {
                    debug!(uid, "ignoring FETCH response for a UID not in this batch");
                    continue;
                };
                let Some(raw) = fetch.body() else {
                    answered_without_body.insert(uid);
                    continue;
                };
                pending.remove(&uid);
                self.summary.messages_seen += 1;
                let bytes = if size > 0 { size } else { raw.len() as u64 };
                let (reports, failures) = match self.parse(raw).await {
                    Ok(parsed) => parsed,
                    // Parsing was cancelled: counted against this message
                    // like a backend failure, so the batch is retried.
                    Err(failure) => {
                        self.settle(uid_validity, uid, Err(failure), true).await?;
                        continue;
                    }
                };
                pending_xml += reports.iter().map(|r| r.raw_xml.len() as u64).sum::<u64>();
                parsed.push(ParsedMessage {
                    uid,
                    bytes,
                    reports,
                    failures,
                });
                // Big reports: stored now rather than held until the batch ends.
                if pending_xml >= self.limits.max_pending_xml_bytes {
                    debug!(
                        pending_xml,
                        "storing the batch's reports so far before parsing more"
                    );
                    self.store_parsed(std::mem::take(&mut parsed), uid_validity)
                        .await?;
                    pending_xml = 0;
                }
            }
        }
        // A backend failure aborts the batch before its cursor is saved, so
        // the next pass retries these messages (up to MAX_ATTEMPTS).
        self.store_parsed(parsed, uid_validity).await?;

        if pending.is_empty() {
            return Ok(());
        }

        // The FETCH stream ends at the tagged response whatever its status, so a
        // missing message is either deleted meanwhile or a server-side failure.
        // Ask again: only UIDs that no longer exist may be skipped.
        let missing: Vec<u32> = pending.keys().copied().collect();
        let still_there = timed(
            self.limits.command_timeout,
            "IMAP UID SEARCH (missing messages)",
            session.uid_search(format!("UID {}", compact_uid_set(&missing))),
        )
        .await?;
        let lost: Vec<u32> = missing
            .iter()
            .copied()
            .filter(|uid| still_there.contains(uid) && !answered_without_body.contains(uid))
            .collect();
        // Some servers answer a damaged message with nothing at all: retried on the
        // next passes, then quarantined so it cannot hold back newer mail.
        let mut attempts = Vec::with_capacity(lost.len());
        for &uid in &lost {
            let key = self.message_key(uid_validity, uid);
            attempts.push(self.record_attempt(&key).await?);
        }
        if let Some(&fewest) = attempts.iter().min() {
            if fewest < MAX_ATTEMPTS {
                return Err(Failure::message(
                    FailureKind::Server,
                    format!(
                        "the mail server returned nothing for {} message(s) that still exist (UIDs {}; \
                         attempt {fewest} of {MAX_ATTEMPTS}); will retry",
                        lost.len(),
                        compact_uid_set(&lost)
                    ),
                ));
            }
        }
        for &uid in &lost {
            error!(
                uid,
                "mail server keeps returning nothing for this message; quarantining it"
            );
            self.summary.messages_seen += 1;
            self.summary.errors += 1;
            let reason =
                format!("gave up after {MAX_ATTEMPTS} attempts: the mail server returned nothing");
            self.quarantine(uid_validity, uid, &reason).await?;
            self.clear_attempts(&self.message_key(uid_validity, uid))
                .await?;
            if let Some(size) = pending.remove(&uid) {
                self.update(|p| {
                    p.processed_messages += 1;
                    p.processed_bytes += size.min(p.total_bytes.saturating_sub(p.processed_bytes));
                });
            }
        }

        for (uid, size) in pending {
            self.summary.messages_seen += 1;
            if answered_without_body.contains(&uid) {
                error!(
                    uid,
                    "UID FETCH response missing body; cursor will still advance past this message"
                );
                self.summary.errors += 1;
                self.quarantine(uid_validity, uid, "UID FETCH response missing body")
                    .await?;
            } else {
                warn!(
                    uid,
                    "message was deleted from the mailbox during the import; skipping"
                );
            }
            self.update(|p| {
                p.processed_messages += 1;
                p.processed_bytes += size.min(p.total_bytes.saturating_sub(p.processed_bytes));
            });
        }
        Ok(())
    }

    /// Store a batch's parsed messages with one [`Backend::ingest_batch`],
    /// then settle each message: its retry count cleared, quarantined when
    /// any of its reports was bad. When the batch call fails, nothing of it
    /// is stored, and the messages are stored one by one instead
    /// ([`Self::ingest_each`]), so a failure counts against its own message
    /// and the messages before it are kept, as they would be on their own.
    async fn store_parsed(
        &mut self,
        parsed: Vec<ParsedMessage>,
        uid_validity: u32,
    ) -> PassResult<()> {
        let started = Instant::now();
        let reports: Vec<&Report> = parsed.iter().flat_map(|m| m.reports.iter()).collect();
        let answers = if reports.is_empty() {
            Ok(Vec::new())
        } else {
            match self.backend.ingest_batch(&reports).await {
                Ok(answers) if answers.len() == reports.len() => Ok(answers),
                Ok(answers) => Err(format!(
                    "{} answers for {} reports",
                    answers.len(),
                    reports.len()
                )),
                Err(e) => Err(e.to_string()),
            }
        };
        drop(reports);
        match answers {
            Ok(answers) => {
                let mut answers = answers.into_iter();
                for message in parsed {
                    let mut failures = message.failures;
                    for (report, answer) in message.reports.iter().zip(answers.by_ref()) {
                        self.tally(report, answer, &mut failures);
                    }
                    let retried = self.retried.contains(&message.uid);
                    self.settle(uid_validity, message.uid, Ok(failures), retried)
                        .await?;
                    self.processed(message.bytes);
                }
            }
            Err(error) => {
                warn!(%error, "storing the batch at once failed; storing its messages one by one");
                for message in parsed {
                    let outcome = self.ingest_each(&message.reports, message.failures).await;
                    self.settle(uid_validity, message.uid, outcome, true)
                        .await?;
                    self.processed(message.bytes);
                }
            }
        }
        self.timings.store += started.elapsed();
        Ok(())
    }

    /// Count one stored message in the progress.
    fn processed(&self, bytes: u64) {
        let ingested = self.summary.reports_ingested;
        self.update(|p| {
            p.processed_messages += 1;
            p.processed_bytes += bytes.min(p.total_bytes.saturating_sub(p.processed_bytes));
            p.reports_ingested = ingested;
        });
    }

    /// Parse and ingest one downloaded message on its own (the batch path is
    /// [`Self::store_parsed`]); see [`Self::settle`].
    #[cfg(test)]
    pub(crate) async fn handle_message(
        &mut self,
        raw: &[u8],
        uid_validity: u32,
        uid: u32,
    ) -> PassResult<()> {
        let outcome = match self.parse(raw).await {
            Ok((reports, failures)) => self.ingest_each(&reports, failures).await,
            Err(failure) => Err(failure),
        };
        self.settle(uid_validity, uid, outcome, true).await
    }

    /// Settle one message after its reports were stored. Bad content
    /// (unparsable, undecompressable or invalid report: `Ok` with failures)
    /// is quarantined and skipped; a backend failure is returned so the batch
    /// fails without moving the cursor, unless the same message has now
    /// failed [`MAX_ATTEMPTS`] passes in a row: then the error is most likely
    /// about the message after all (misclassified), so it is quarantined and
    /// the batch carries on. `clear` forgets the message's earlier attempts
    /// once it went through.
    async fn settle(
        &mut self,
        uid_validity: u32,
        uid: u32,
        outcome: PassResult<Vec<String>>,
        clear: bool,
    ) -> PassResult<()> {
        let key = self.message_key(uid_validity, uid);
        match outcome {
            Ok(failures) => {
                if clear {
                    self.clear_attempts(&key).await?;
                }
                if !failures.is_empty() {
                    self.quarantine(uid_validity, uid, &failures.join("; "))
                        .await?;
                }
                Ok(())
            }
            Err(failure) => {
                let attempts = self.record_attempt(&key).await?;
                if attempts < MAX_ATTEMPTS {
                    return Err(Failure::message(
                        failure.kind,
                        format!(
                            "storing reports from message UID {uid} failed \
                             (attempt {attempts} of {MAX_ATTEMPTS}); will retry: {failure}"
                        ),
                    ));
                }
                error!(
                    uid,
                    attempts,
                    error = %failure,
                    "message keeps failing; quarantining it so newer mail can be imported"
                );
                let reason = format!("gave up after {attempts} attempts: {failure}");
                // If even this write fails the count stays, and the next pass tries again.
                self.quarantine(uid_validity, uid, &reason).await?;
                self.clear_attempts(&key).await?;
                Ok(())
            }
        }
    }

    /// Extract and parse one RFC822 message's report attachments: the reports
    /// found, and the reasons any attachment's content was bad. These
    /// failures are kept, not returned as errors, so one bad attachment does
    /// not abandon the others in the same message; the message is
    /// quarantined when there are any.
    ///
    /// Parsing runs on the blocking pool, so a large report does not stall the
    /// runtime and a parser panic costs only this message (quarantined as bad
    /// content), not the import.
    async fn parse(&mut self, raw: &[u8]) -> PassResult<(Vec<Report>, Vec<String>)> {
        let owned = raw.to_vec();
        let parser = Arc::clone(self.parser);
        let started = Instant::now();
        let parsed = tokio::task::spawn_blocking(move || parser(&owned)).await;
        self.timings.parse += started.elapsed();
        let (reports, failures) = match parsed {
            Ok(parsed) => parsed,
            Err(e) if e.is_panic() => {
                let reason = format!("the report parser crashed on this message: {e}");
                error!(%reason, "quarantining the message (cursor advances)");
                (Vec::new(), vec![reason])
            }
            Err(e) => return Err(Failure::storage("parsing the message was cancelled", e)),
        };
        self.summary.errors += count_u32(failures.len());
        Ok((reports, failures))
    }

    /// Ingest one message's reports one at a time. Returns its failures with
    /// the reports the backend rejected added; a backend failure is returned
    /// as `Err` instead: nothing is wrong with the message, so it must be
    /// retried, not skipped.
    async fn ingest_each(
        &mut self,
        reports: &[Report],
        mut failures: Vec<String>,
    ) -> PassResult<Vec<String>> {
        for report in reports {
            match self.backend.ingest(report).await {
                Ok(answer) => self.tally(report, answer, &mut failures),
                Err(e) => {
                    self.summary.errors += 1;
                    return Err(Failure::storage(
                        &format!("database error while storing report {}", report.report_id),
                        e,
                    ));
                }
            }
        }
        Ok(failures)
    }

    /// Count what became of one report; a rejected one adds its reason to
    /// its message's `failures`.
    fn tally(&mut self, report: &Report, answer: Ingested, failures: &mut Vec<String>) {
        match answer {
            Ingested::Inserted => {
                debug!(report_id = %report.report_id, "ingested report");
                self.summary.reports_ingested += 1;
            }
            Ingested::Duplicate => {
                debug!(report_id = %report.report_id, "duplicate report skipped");
                self.summary.reports_duplicate += 1;
            }
            Ingested::Ignored => {
                debug!(domain = %report.policy.domain, "report for an ignored domain dropped");
            }
            Ingested::Rejected(reason) => {
                error!(
                    report_id = %report.report_id,
                    reason = %reason,
                    "report was rejected; will not be retried (cursor advances)"
                );
                self.summary.errors += 1;
                failures.push(format!("ingest failed for {}: {reason}", report.report_id));
            }
        }
    }

    /// Record a message the cursor is about to skip. A backend that cannot
    /// take the row fails the batch like any other backend error, so no skip
    /// goes unrecorded.
    async fn quarantine(&mut self, uid_validity: u32, uid: u32, error: &str) -> PassResult<()> {
        let mailbox = self.mailbox_key();
        self.backend
            .quarantine(
                MessageRef {
                    mailbox: &mailbox,
                    folder: &self.cfg.folder,
                    uid_validity,
                    uid,
                },
                error,
                self.backend.now(),
            )
            .await
            .map_err(|e| {
                Failure::storage(
                    &format!("failed to record quarantined message UID {uid}"),
                    e,
                )
            })?;
        self.summary.messages_quarantined += 1;
        Ok(())
    }
}

/// `RFC822.SIZE` of every UID in `uids`, in chunks of [`Limits::size_chunk`].
/// UIDs the server does not answer for (deleted meanwhile) are simply absent.
async fn fetch_sizes<T>(
    session: &mut async_imap::Session<T>,
    uids: &[u32],
    limits: &Limits,
) -> PassResult<HashMap<u32, u32>>
where
    T: AsyncRead + AsyncWrite + Unpin + fmt::Debug + Send,
{
    let mut sizes = HashMap::with_capacity(uids.len());
    for chunk in uids.chunks(limits.size_chunk.max(1)) {
        let set = compact_uid_set(chunk);
        let fetches: Vec<_> = timed(
            limits.command_timeout,
            "IMAP UID FETCH (RFC822.SIZE)",
            async {
                session
                    .uid_fetch(&set, "(UID RFC822.SIZE)")
                    .await?
                    .try_collect::<Vec<_>>()
                    .await
            },
        )
        .await?;
        for fetch in &fetches {
            if let (Some(uid), Some(size)) = (fetch.uid, fetch.size) {
                sizes.insert(uid, size);
            }
        }
    }
    Ok(sizes)
}

/// The CPU-bound half of a message (MIME, decompression, XML): the reports
/// found, and the reasons any report-looking attachment was bad.
fn parse_message(raw: &[u8]) -> (Vec<Report>, Vec<String>) {
    parse_message_within(raw, MAX_MESSAGE_XML_BYTES)
}

/// [`parse_message`] with `budget` bytes of XML for all of the message's
/// reports together: each attachment gets what the ones before it left.
fn parse_message_within(raw: &[u8], budget: u64) -> (Vec<Report>, Vec<String>) {
    let mut left = budget;
    let mut reports = Vec::new();
    let mut failures = Vec::new();
    let candidates = extract_report_attachments(raw);
    if candidates.is_empty() {
        // Not a failure: mailboxes receive ordinary mail, and quarantining every
        // human message would bury the reports that genuinely failed to parse.
        debug!("message has no DMARC-looking attachments; skipping (cursor still advances)");
    }
    for candidate in candidates {
        let att = match candidate {
            Ok(att) => att,
            Err(reason) => {
                error!(%reason, "DMARC attachment refused; will not be retried (cursor advances)");
                failures.push(reason);
                continue;
            }
        };
        let hint = att.parse_hint();
        let name = att.filename.as_deref().unwrap_or("unnamed attachment");
        match parse_report_within(&att.data, hint.as_deref(), left) {
            Ok(report) => {
                left = left.saturating_sub(report.raw_xml.len() as u64);
                reports.push(report);
            }
            // Within the per-report limit, but not within what is left.
            Err(ParseError::TooLarge(_)) if left < budget => {
                error!(
                    filename = ?att.filename,
                    "the message's reports unpack to more than its limit together; \
                     will not be retried (cursor advances)"
                );
                failures.push(format!(
                    "{name} was not imported: the reports in this message unpack to more \
                     than {} together",
                    format_bytes(budget)
                ));
            }
            // An office file or a TLS-RPT report in the same mailbox: ordinary mail.
            Err(ParseError::NotDmarc) => {
                debug!(filename = ?att.filename, "attachment is not a DMARC report; skipping");
            }
            Err(e) => {
                error!(
                    filename = ?att.filename,
                    error = %e,
                    "failed to parse DMARC attachment; will not be retried (cursor advances)"
                );
                failures.push(format!("parse failed for {name}: {e}"));
            }
        }
    }
    (reports, failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::{IngestRule, MemoryBackend};
    use crate::fake_server::{self, FakeMessage, FakeServer};

    #[test]
    fn summary_display_and_default() {
        let s = FetchSummary::default();
        assert_eq!(s.messages_seen, 0);
        assert_eq!(s.starting_uid, None);
        assert_eq!(s.ending_uid, None);
        assert!(!s.advanced());
        assert_eq!(
            s.to_string(),
            "messages_seen=0, reports_ingested=0, reports_duplicate=0, errors=0, messages_quarantined=0, starting_uid=None, ending_uid=None, interrupted=false"
        );

        let s = FetchSummary {
            messages_seen: 2,
            reports_ingested: 1,
            reports_duplicate: 1,
            starting_uid: Some(10),
            ending_uid: Some(20),
            ..FetchSummary::default()
        };
        assert!(s.advanced());
        assert!(s.to_string().contains("reports_ingested=1"));
        assert!(s.to_string().contains("starting_uid=Some(10)"));
        assert!(s.to_string().contains("ending_uid=Some(20)"));
    }

    #[test]
    fn fetch_error_mentions_saved_progress() {
        let e = FetchError {
            summary: FetchSummary {
                messages_seen: 100,
                reports_ingested: 90,
                ending_uid: Some(100),
                ..FetchSummary::default()
            },
            kind: FailureKind::Timeout,
            message: "batch timed out".into(),
        };
        let text = e.to_string();
        assert!(text.starts_with("batch timed out"), "{text}");
        assert!(
            text.contains("100 messages processed, 90 reports imported"),
            "{text}"
        );

        let e = FetchError {
            summary: FetchSummary::default(),
            kind: FailureKind::Connect(ConnectStage::Auth),
            message: "login failed".into(),
        };
        assert_eq!(e.to_string(), "login failed");
    }

    pub(crate) fn cfg() -> ImapConfig {
        ImapConfig {
            host: "fake".into(),
            port: 993,
            security: crate::ImapSecurity::Tls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        }
    }

    fn small_limits() -> Limits {
        Limits {
            batch_max_messages: 2,
            batch_max_bytes: 1_000_000,
            max_message_bytes: 1_000_000,
            max_pending_xml_bytes: Limits::DEFAULT.max_pending_xml_bytes,
            size_chunk: 3,
            command_timeout: Duration::from_secs(5),
            batch_timeout_base: Duration::from_secs(2),
            batch_bytes_per_extra_sec: u64::MAX,
        }
    }

    /// Run one `import` against `server`; the progress of the pass is returned
    /// with the result so tests can look at it.
    struct Run {
        result: PassResult<()>,
        summary: FetchSummary,
        progress: SharedProgress,
        log: fake_server::Log,
    }

    async fn run_with(
        server: FakeServer,
        backend: &MemoryBackend,
        cancel: &CancellationToken,
        limits: &Limits,
        parser: Parser,
    ) -> Run {
        run_in(&cfg(), server, backend, cancel, limits, parser).await
    }

    /// [`run_with`] under the settings `cfg` (another folder, say).
    async fn run_in(
        cfg: &ImapConfig,
        server: FakeServer,
        backend: &MemoryBackend,
        cancel: &CancellationToken,
        limits: &Limits,
        parser: Parser,
    ) -> Run {
        let (mut session, log) = fake_server::session(server).await;
        let progress = SharedProgress::default();
        let mut pass = Pass::new(backend, cfg, &progress, cancel, limits, &parser);
        let result = pass.import(&mut session).await;
        Run {
            result,
            summary: pass.summary,
            progress,
            log,
        }
    }

    async fn run_import(server: FakeServer, backend: &MemoryBackend, limits: &Limits) -> Run {
        run_with(
            server,
            backend,
            &CancellationToken::new(),
            limits,
            default_parser(),
        )
        .await
    }

    fn mailbox(uids: &[u32]) -> Vec<FakeMessage> {
        uids.iter().map(|&uid| FakeMessage::report(uid)).collect()
    }

    fn cursor(backend: &MemoryBackend) -> Option<(u32, u32)> {
        backend
            .cursor_of("INBOX")
            .map(|c| (c.uid_validity, c.last_uid))
    }

    #[tokio::test]
    async fn imports_in_batches_and_persists_cursor() {
        let backend = MemoryBackend::default();
        let mut messages = mailbox(&[3, 4, 5, 9, 10]);
        messages.push(FakeMessage::plain(11));
        let run = run_import(FakeServer::new(7, messages), &backend, &small_limits()).await;
        run.result.unwrap();

        assert_eq!(run.summary.messages_seen, 6);
        assert_eq!(run.summary.reports_ingested, 5);
        assert_eq!(run.summary.ending_uid, Some(11));
        assert_eq!(backend.report_count(), 5);
        assert_eq!(cursor(&backend), Some((7, 11)));
        // Each batch's reports were stored with one call, none on their own.
        assert_eq!(backend.state().batch_calls, 3);
        assert_eq!(backend.state().ingest_calls, 0);

        let log = run.log.lock().unwrap().clone();
        let sizes: Vec<_> = log.iter().filter(|c| c.contains("RFC822.SIZE")).collect();
        assert_eq!(sizes.len(), 2, "{log:?}");
        assert!(sizes[0].contains("UID FETCH 3:5 "), "{log:?}");
        assert!(sizes[1].contains("UID FETCH 9:11 "), "{log:?}");
        let bodies: Vec<_> = log.iter().filter(|c| c.contains("BODY.PEEK[]")).collect();
        assert_eq!(bodies.len(), 3, "{log:?}");
        assert!(bodies[0].contains("UID FETCH 3:4 "));
        assert!(bodies[1].contains("UID FETCH 5,9 "));
        assert!(bodies[2].contains("UID FETCH 10:11 "));
        assert!(!log
            .iter()
            .any(|c| c.contains("STORE") || c.contains("BODY[]")));

        let progress = run.progress.get().unwrap();
        assert_eq!(progress.phase, ImapPhase::Importing);
        assert_eq!(progress.total_messages, 6);
        assert_eq!(progress.processed_messages, 6);
        assert_eq!(progress.reports_ingested, 5);
        assert!(progress.total_bytes > 0);
        assert_eq!(progress.processed_bytes, progress.total_bytes);

        // Nothing new: the next pass only searches.
        let run = run_import(
            FakeServer::new(7, mailbox(&[3, 4, 5, 9, 10])),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.messages_seen, 0);
        assert_eq!(run.summary.ending_uid, Some(11));
        assert!(!run.log.lock().unwrap().iter().any(|c| c.contains("FETCH")));
    }

    #[tokio::test]
    async fn oversized_messages_are_quarantined_without_download() {
        let backend = MemoryBackend::default();
        let mut messages = mailbox(&[1, 3]);
        messages.insert(1, FakeMessage::report(2).announced_size(5_000_000));
        let run = run_import(FakeServer::new(1, messages), &backend, &small_limits()).await;
        run.result.unwrap();

        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(cursor(&backend), Some((1, 3)));
        let log = run.log.lock().unwrap().clone();
        let bodies: Vec<_> = log.iter().filter(|c| c.contains("BODY.PEEK[]")).collect();
        assert_eq!(bodies.len(), 1, "{log:?}");
        assert!(bodies[0].contains("UID FETCH 1,3 "), "{log:?}");
        let reason = backend.quarantine_reason(2).unwrap();
        assert!(reason.contains("import limit"), "{reason}");
    }

    #[tokio::test]
    async fn timed_out_batch_keeps_earlier_batches_and_next_pass_resumes() {
        let backend = MemoryBackend::default();
        let uids = [1, 2, 3, 4, 5];
        // The second body FETCH never answers.
        let server = FakeServer::new(1, mailbox(&uids)).hang_on_body_fetch(2);
        let run = run_import(server, &backend, &small_limits()).await;
        let err = run.result.unwrap_err();
        assert_eq!(err.kind, FailureKind::Timeout);
        assert!(err.message.contains("timed out"), "{err}");
        assert!(err.message.contains("UIDs 3–4"), "{err}");
        assert_eq!(run.summary.ending_uid, Some(2));
        assert!(run.summary.advanced());
        assert_eq!(cursor(&backend), Some((1, 2)));
        assert_eq!(backend.report_count(), 2);

        let run = run_import(
            FakeServer::new(1, mailbox(&uids)),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.starting_uid, Some(2));
        assert_eq!(run.summary.messages_seen, 3);
        assert_eq!(run.summary.ending_uid, Some(5));
        assert_eq!(backend.report_count(), 5);
        let log = run.log.lock().unwrap().clone();
        assert!(
            log.iter().any(|c| c.contains("UID SEARCH UID 3:*")),
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn cancellation_stops_between_batches() {
        let backend = MemoryBackend::default();
        let cancel = CancellationToken::new();
        // Cancelled before the first batch starts.
        cancel.cancel();
        let run = run_with(
            FakeServer::new(1, mailbox(&[1, 2, 3])),
            &backend,
            &cancel,
            &small_limits(),
            default_parser(),
        )
        .await;
        run.result.unwrap();
        assert!(run.summary.interrupted);
        assert_eq!(run.summary.messages_seen, 0);
        assert_eq!(cursor(&backend), None);
        assert!(!run
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("BODY.PEEK[]")));
    }

    /// The folder changes in the settings while a pass runs: the worker's
    /// `settings_changed` cancels the pass's token, the batch in hand
    /// finishes and saves its cursor under the old folder, and the pass
    /// stops there. The next pass, under the new settings, reads the new
    /// folder from its first message; the old folder keeps its cursor.
    #[tokio::test]
    async fn a_folder_change_mid_pass_keeps_the_old_cursor_and_reads_the_new_folder_from_zero() {
        let backend = MemoryBackend::default();
        let cancel = CancellationToken::new();
        // The settings are saved while the first batch is being parsed.
        let changed: Parser = {
            let cancel = cancel.clone();
            Arc::new(move |raw: &[u8]| {
                cancel.cancel();
                parse_message(raw)
            })
        };
        let run = run_with(
            FakeServer::new(7, mailbox(&[1, 2, 3, 4, 5])),
            &backend,
            &cancel,
            &small_limits(),
            changed,
        )
        .await;
        run.result.unwrap();
        assert!(run.summary.interrupted);
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(cursor(&backend), Some((7, 2)));

        let reports = ImapConfig {
            folder: "Reports".into(),
            ..cfg()
        };
        let run = run_in(
            &reports,
            FakeServer::new(9, mailbox(&[11, 12, 13])),
            &backend,
            &CancellationToken::new(),
            &small_limits(),
            default_parser(),
        )
        .await;
        run.result.unwrap();
        assert!(!run.summary.interrupted);
        assert_eq!(run.summary.starting_uid, None);
        assert_eq!(run.summary.reports_ingested, 3);
        assert!(run
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("EXAMINE") && c.contains("Reports")));
        let new = backend
            .cursor_of("Reports")
            .map(|c| (c.uid_validity, c.last_uid));
        assert_eq!(new, Some((9, 13)));
        assert_eq!(cursor(&backend), Some((7, 2)));
    }

    #[tokio::test]
    async fn uidvalidity_change_restarts_from_zero_and_forgets_the_old_generation() {
        let backend = MemoryBackend::default();
        backend
            .save_cursor(
                &cfg().mailbox_key(),
                "INBOX",
                Cursor {
                    uid_validity: 1,
                    last_uid: 50,
                },
            )
            .await
            .unwrap();
        backend
            .state()
            .quarantine
            .push(crate::backend::memory::Quarantined {
                uid_validity: 1,
                uid: 7,
                error: "old".into(),
            });
        let run = run_import(
            FakeServer::new(2, mailbox(&[1, 2])),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.starting_uid, None);
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(cursor(&backend), Some((2, 2)));
        assert!(backend.state().quarantine.is_empty());
    }

    #[tokio::test]
    async fn backend_errors_fail_the_batch_without_skipping_mail() {
        let backend = MemoryBackend::default();
        // A database that cannot write (busy / disk full) for reports.
        backend.state().ingest_down = Some("disk I/O error (simulated)".into());
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3])),
            &backend,
            &small_limits(),
        )
        .await;
        let err = run.result.unwrap_err();
        assert_eq!(err.kind, FailureKind::Storage);
        assert!(err.message.contains("simulated"), "{err}");
        assert_eq!(run.summary.messages_quarantined, 0);
        assert_eq!(cursor(&backend), None);
        assert!(backend.state().quarantine.is_empty());

        // The database recovers: the next pass imports everything.
        backend.state().ingest_down = None;
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3])),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 3);
        assert_eq!(backend.report_count(), 3);
        assert_eq!(cursor(&backend), Some((1, 3)));
    }

    /// A batch whose store call fails is stored message by message: the
    /// messages before the failing one are kept, the failure counts against
    /// its own message, and the cursor stays.
    #[tokio::test]
    async fn a_failed_batch_store_falls_back_to_one_message_at_a_time() {
        let backend = MemoryBackend::default();
        backend.state().ingest_rules.insert(
            "fake-3".into(),
            IngestRule::Fail("locked (simulated)".into()),
        );
        let limits = Limits {
            batch_max_messages: 4,
            ..small_limits()
        };
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3, 4])),
            &backend,
            &limits,
        )
        .await;
        let err = run.result.unwrap_err();
        assert!(err.message.contains("message UID 3"), "{err}");
        assert_eq!(backend.state().batch_calls, 1);
        assert_eq!(backend.state().ingest_calls, 3);
        assert_eq!(backend.report_count(), 2);
        assert_eq!(backend.attempts_of(3), Some(1));
        assert_eq!(cursor(&backend), None);

        // Recovered: one batch call stores the rest; the earlier two are
        // duplicates, and UID 3's count is cleared.
        backend.state().ingest_rules.clear();
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3, 4])),
            &backend,
            &limits,
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.reports_duplicate, 2);
        assert_eq!(backend.state().batch_calls, 2);
        assert_eq!(backend.state().ingest_calls, 3);
        assert!(backend.state().attempts.is_empty());
        assert_eq!(cursor(&backend), Some((1, 4)));
    }

    #[tokio::test]
    async fn persistently_failing_message_is_quarantined_on_the_last_attempt() {
        let backend = MemoryBackend::default();
        // Only UID 2's report fails, with an error classified as environmental.
        backend.state().ingest_rules.insert(
            "fake-2".into(),
            IngestRule::Fail("stuck (simulated)".into()),
        );
        let uids = [1, 2, 3, 4];
        for attempt in 1..MAX_ATTEMPTS {
            let run = run_import(
                FakeServer::new(1, mailbox(&uids)),
                &backend,
                &small_limits(),
            )
            .await;
            let err = run.result.unwrap_err();
            assert!(
                err.message
                    .contains(&format!("attempt {attempt} of {MAX_ATTEMPTS}")),
                "{err}"
            );
            assert_eq!(run.summary.messages_quarantined, 0);
            assert_eq!(cursor(&backend), None);
            assert!(backend.state().quarantine.is_empty());
        }

        let run = run_import(
            FakeServer::new(1, mailbox(&uids)),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(cursor(&backend), Some((1, 4)));
        assert_eq!(backend.report_count(), 3);
        let reason = backend.quarantine_reason(2).unwrap();
        assert!(
            reason.starts_with(&format!("gave up after {MAX_ATTEMPTS} attempts: ")),
            "{reason}"
        );
        assert!(reason.contains("stuck (simulated)"), "{reason}");
        assert!(backend.state().attempts.is_empty());
    }

    #[tokio::test]
    async fn success_resets_the_attempt_count() {
        let backend = MemoryBackend::default();
        backend.state().ingest_rules.insert(
            "fake-1".into(),
            IngestRule::Fail("flaky (simulated)".into()),
        );
        for _ in 0..2 {
            let run =
                run_import(FakeServer::new(1, mailbox(&[1])), &backend, &small_limits()).await;
            assert!(run.result.is_err());
        }
        assert_eq!(backend.attempts_of(1), Some(2));

        backend.state().ingest_rules.clear();
        let run = run_import(FakeServer::new(1, mailbox(&[1])), &backend, &small_limits()).await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 1);
        assert_eq!(run.summary.messages_quarantined, 0);
        assert!(backend.state().attempts.is_empty());
    }

    #[tokio::test]
    async fn rejected_reports_are_quarantined_not_retried() {
        let backend = MemoryBackend::default();
        backend.state().ingest_rules.insert(
            "fake-2".into(),
            IngestRule::Reject("domain is not a hostname".into()),
        );
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3])),
            &backend,
            &small_limits(),
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(run.summary.errors, 1);
        assert_eq!(cursor(&backend), Some((1, 3)));
        let reason = backend.quarantine_reason(2).unwrap();
        assert!(reason.contains("not a hostname"), "{reason}");
        // Rejected within its batch's call: the batch's other report is kept.
        assert_eq!(backend.state().ingest_calls, 0);
        assert_eq!(backend.report_count(), 2);
    }

    #[tokio::test]
    async fn unparsable_reports_are_quarantined_and_skipped() {
        let backend = MemoryBackend::default();
        let mut messages = mailbox(&[1, 3]);
        messages.insert(1, FakeMessage::broken_report(2));
        let run = run_import(FakeServer::new(1, messages), &backend, &small_limits()).await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(cursor(&backend), Some((1, 3)));
    }

    #[tokio::test]
    async fn deleted_messages_are_skipped_but_silent_failures_are_retried() {
        let backend = MemoryBackend::default();
        // UID 2 disappears between the size scan and the body fetch.
        let server = FakeServer::new(1, mailbox(&[1, 2, 3])).expunge_after_sizes(2);
        let run = run_import(server, &backend, &small_limits()).await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.messages_quarantined, 0);
        assert_eq!(cursor(&backend), Some((1, 3)));

        // UID 2 still exists but the server answers the FETCH with nothing.
        let backend = MemoryBackend::default();
        let server = FakeServer::new(1, mailbox(&[1, 2, 3])).omit_body(2);
        let run = run_import(server, &backend, &small_limits()).await;
        let err = run.result.unwrap_err();
        assert_eq!(err.kind, FailureKind::Server);
        assert!(err.message.contains("still exist"), "{err}");
        assert_eq!(cursor(&backend), None);
    }

    #[tokio::test]
    async fn message_the_server_never_returns_is_quarantined_after_max_attempts() {
        let backend = MemoryBackend::default();
        for attempt in 1..MAX_ATTEMPTS {
            let server = FakeServer::new(1, mailbox(&[1, 2, 3])).omit_body(2);
            let run = run_import(server, &backend, &small_limits()).await;
            let err = run.result.unwrap_err();
            assert!(
                err.message
                    .contains(&format!("attempt {attempt} of {MAX_ATTEMPTS}")),
                "{err}"
            );
        }
        let server = FakeServer::new(1, mailbox(&[1, 2, 3])).omit_body(2);
        let run = run_import(server, &backend, &small_limits()).await;
        run.result.unwrap();
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(backend.report_count(), 2);
        assert_eq!(cursor(&backend), Some((1, 3)));
        assert!(backend.state().attempts.is_empty());
    }

    #[tokio::test]
    async fn download_that_keeps_timing_out_is_isolated_then_quarantined() {
        let backend = MemoryBackend::default();
        let limits = Limits {
            batch_timeout_base: Duration::from_millis(200),
            ..small_limits()
        };
        let pass = || async {
            let server = FakeServer::new(1, mailbox(&[1, 2, 3, 4])).hang_on_uid(3);
            run_import(server, &backend, &limits).await
        };
        // The batch [3, 4] times out, then UID 3 alone, MAX_ATTEMPTS times.
        let run = pass().await;
        assert!(run.result.unwrap_err().message.contains("UIDs 3–4"));
        for attempt in 1..=MAX_ATTEMPTS {
            let err = pass().await.result.unwrap_err();
            assert!(err.message.contains("UIDs 3–3"), "{err}");
            assert!(
                err.message
                    .contains(&format!("attempt {attempt} of {MAX_ATTEMPTS}")),
                "{err}"
            );
        }
        // Then UID 3 is skipped without downloading it, and UID 4 is imported.
        let run = pass().await;
        run.result.unwrap();
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(backend.report_count(), 3);
        assert_eq!(cursor(&backend), Some((1, 4)));
        assert!(backend.state().attempts.is_empty());
    }

    #[tokio::test]
    async fn a_parser_panic_quarantines_the_message_and_the_import_goes_on() {
        let backend = MemoryBackend::default();
        // The parser crashes on UID 2's message (its attachment name is in the raw text).
        let marker = b"!2.xml".to_vec();
        let parser: Parser = Arc::new(move |raw: &[u8]| {
            assert!(
                !raw.windows(marker.len()).any(|w| w == marker.as_slice()),
                "test hook: parser panic"
            );
            parse_message(raw)
        });
        let run = run_with(
            FakeServer::new(1, mailbox(&[1, 2, 3])),
            &backend,
            &CancellationToken::new(),
            &small_limits(),
            parser,
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 2);
        assert_eq!(run.summary.messages_quarantined, 1);
        assert_eq!(run.summary.errors, 1);
        assert_eq!(cursor(&backend), Some((1, 3)));
        let reason = backend.quarantine_reason(2).unwrap();
        assert!(reason.contains("parser crashed"), "{reason}");
    }

    #[tokio::test]
    async fn oversized_report_attachment_is_quarantined() {
        use base64::Engine;
        let backend = MemoryBackend::default();
        let huge = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 26 * 1024 * 1024]);
        let raw = format!(
            "From: r@example.com\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: application/gzip\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"big.xml.gz\"\r\n\r\n\
             {huge}\r\n--b--\r\n"
        );
        let cfg = cfg();
        let progress = SharedProgress::default();
        let cancel = CancellationToken::new();
        let limits = small_limits();
        let parser = default_parser();
        let mut pass = Pass::new(&backend, &cfg, &progress, &cancel, &limits, &parser);
        pass.handle_message(raw.as_bytes(), 1, 7).await.unwrap();
        assert_eq!(pass.summary.messages_quarantined, 1);
        assert_eq!(pass.summary.errors, 1);
        let reason = backend.quarantine_reason(7).unwrap();
        assert!(reason.contains("big.xml.gz"), "{reason}");
        assert!(reason.contains("exceeds the 25.0 MB limit"), "{reason}");
    }

    #[tokio::test]
    async fn attachments_that_are_not_dmarc_reports_are_skipped_not_quarantined() {
        let backend = MemoryBackend::default();
        let messages = vec![FakeMessage::report(1), FakeMessage::other_xml(2)];
        let run = run_import(FakeServer::new(1, messages), &backend, &small_limits()).await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 1);
        assert_eq!(run.summary.messages_quarantined, 0);
        assert_eq!(run.summary.errors, 0);
        assert_eq!(cursor(&backend), Some((1, 2)));
    }

    #[tokio::test]
    async fn a_batch_holding_much_xml_is_stored_before_it_ends() {
        let backend = MemoryBackend::default();
        // Every report reaches the limit, so each message is stored on its own
        // as soon as it is parsed, inside batches of three.
        let limits = Limits {
            batch_max_messages: 3,
            max_pending_xml_bytes: 1,
            ..small_limits()
        };
        let run = run_import(
            FakeServer::new(1, mailbox(&[1, 2, 3, 4])),
            &backend,
            &limits,
        )
        .await;
        run.result.unwrap();
        assert_eq!(run.summary.reports_ingested, 4);
        assert_eq!(run.summary.messages_quarantined, 0);
        assert_eq!(cursor(&backend), Some((1, 4)));
        assert_eq!(backend.state().batch_calls, 4);
        assert_eq!(run.progress.get().unwrap().processed_messages, 4);
    }

    /// A raw message with one gzip attachment per XML document.
    fn message_with_reports(xmls: &[String]) -> Vec<u8> {
        use base64::Engine;
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut raw = String::from(
            "From: r@example.com\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n",
        );
        for (i, xml) in xmls.iter().enumerate() {
            let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::fast());
            gz.write_all(xml.as_bytes()).unwrap();
            let b64 = base64::engine::general_purpose::STANDARD.encode(gz.finish().unwrap());
            raw.push_str(&format!(
                "--b\r\nContent-Type: application/gzip\r\n\
                 Content-Transfer-Encoding: base64\r\n\
                 Content-Disposition: attachment; filename=\"r{i}.xml.gz\"\r\n\r\n\
                 {b64}\r\n"
            ));
        }
        raw.push_str("--b--\r\n");
        raw.into_bytes()
    }

    fn report_xml(report_id: &str) -> String {
        format!(
            "<feedback><report_metadata><org_name>o</org_name><email>a@b.c</email>\
             <report_id>{report_id}</report_id><date_range><begin>1704067200</begin>\
             <end>1704153599</end></date_range></report_metadata>\
             <policy_published><domain>example.com</domain></policy_published>\
             <record><row><source_ip>192.0.2.1</source_ip><count>1</count></row></record>\
             </feedback>"
        )
    }

    #[test]
    fn a_message_whose_reports_unpack_past_its_limit_is_refused_from_there() {
        let xmls: Vec<String> = ["a", "b", "c"].into_iter().map(report_xml).collect();
        let raw = message_with_reports(&xmls);
        let len = xmls[0].len() as u64;
        // Room for two reports and a half: the third does not fit.
        let (reports, failures) = parse_message_within(&raw, len * 5 / 2);
        let ids: Vec<&str> = reports.iter().map(|r| r.report_id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].starts_with("r2.xml.gz was not imported"),
            "{failures:?}"
        );
        assert!(failures[0].contains("together"), "{failures:?}");
        // The whole message within the limit: all three.
        assert_eq!(parse_message_within(&raw, len * 3).0.len(), 3);
        // A report over the per-report limit says so, not "together".
        let (reports, failures) = parse_message_within(&raw, len - 1);
        assert!(reports.is_empty());
        assert_eq!(failures.len(), 3, "{failures:?}");
        assert!(failures[0].contains("limit"), "{failures:?}");
        assert!(!failures[0].contains("together"), "{failures:?}");
        assert_eq!(MAX_MESSAGE_XML_BYTES, dmarc_parser::MAX_REPORT_BYTES);
    }
}
