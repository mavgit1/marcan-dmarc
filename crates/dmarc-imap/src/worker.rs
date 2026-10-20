//! The polling loop and manual syncs: one pass at a time, with the stored
//! configuration re-read before each, stopping between batches when the
//! settings change or the process shuts down.

use crate::backend::{Backend, PassRecord};
use crate::batch::Limits;
use crate::pass::{default_parser, FetchSummary, Pass, Transport};
use crate::progress::{ImapProgress, SharedProgress};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Mutex, Notify, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// How often the loop re-reads settings while the mailbox is not configured.
const UNCONFIGURED_RECHECK: Duration = Duration::from_secs(30);

/// Retry delay after a pass that failed part-way but moved the cursor: the
/// import is clearly working, so continue soon rather than after the interval.
const RETRY_AFTER_PROGRESS: Duration = Duration::from_secs(30);

/// Result of one worker pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Host, username or password is not set; nothing was fetched.
    Unconfigured,
    Fetched(FetchSummary),
}

/// Answer to [`ImapWorker::start_sync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStart {
    /// A pass was started in the background.
    Started,
    /// A pass is already running; its progress is in [`ImapWorker::progress`].
    AlreadyRunning,
    /// Host, username or password is not set.
    Unconfigured,
}

struct Inner {
    /// Held by the running pass; passes are serialized. Its own `Arc` so a
    /// pass can own its guard (`lock_owned`).
    pass_lock: Arc<Mutex<()>>,
    wake: Notify,
    progress: SharedProgress,
    /// The running pass's cancellation, so a settings change (or shutdown)
    /// stops it between batches.
    current: StdMutex<Option<CancellationToken>>,
    /// How many times the settings changed; diagnostics and tests.
    generation: AtomicU64,
    /// The next pass reads the folder from its first message (see [`ImapWorker::request_reimport`]).
    reimport: AtomicBool,
    /// Passes that ended since startup, however they ended (unconfigured too).
    passes_done: AtomicU64,
}

/// Runs fetch passes for the polling loop and for manual syncs.
///
/// Every pass re-reads the config from the backend, so settings saved in the
/// UI apply without a restart. Passes are serialized: at most one runs at a
/// time, and a manual sync while one runs reports that instead of waiting.
#[derive(Clone)]
pub struct ImapWorker {
    inner: Arc<Inner>,
}

impl Default for ImapWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl ImapWorker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                pass_lock: Arc::new(Mutex::new(())),
                wake: Notify::new(),
                progress: SharedProgress::default(),
                current: StdMutex::new(None),
                generation: AtomicU64::new(0),
                reimport: AtomicBool::new(false),
                passes_done: AtomicU64::new(0),
            }),
        }
    }

    /// Make the loop start its next pass now instead of after its sleep.
    pub fn wake(&self) {
        self.inner.wake.notify_one();
    }

    /// Re-import the mailbox with the next pass, which starts now or right after
    /// the running one: nothing is lost to a pass in progress. Without a mailbox
    /// configured there is nothing to read, and the first pass reads everything.
    pub fn request_reimport(&self) {
        self.inner.reimport.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// The IMAP settings were saved: a running pass stops after its current
    /// batch (keeping its cursor) and the loop starts over with the new settings.
    pub fn settings_changed(&self) {
        self.inner.generation.fetch_add(1, Ordering::SeqCst);
        self.cancel_current();
        self.wake();
    }

    /// Whether a re-import waits for the next pass; diagnostics and tests.
    pub fn reimport_requested(&self) -> bool {
        self.inner.reimport.load(Ordering::SeqCst)
    }

    /// How many times [`Self::settings_changed`] was called; diagnostics and tests.
    pub fn settings_generation(&self) -> u64 {
        self.inner.generation.load(Ordering::SeqCst)
    }

    /// Progress of the running pass; `None` while idle.
    pub fn progress(&self) -> Option<ImapProgress> {
        self.inner.progress.get()
    }

    /// How many passes ended since startup, however they ended: `0` until the
    /// first one is done (the alert job waits for it).
    pub fn passes_done(&self) -> u64 {
        self.inner.passes_done.load(Ordering::SeqCst)
    }

    fn set_current(&self, token: Option<CancellationToken>) {
        *self
            .inner
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = token;
    }

    /// Stop the running pass, if any, after its current batch.
    fn cancel_current(&self) {
        if let Some(token) = self
            .inner
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            token.cancel();
        }
    }

    /// Start a pass in the background with the stored config and return at once.
    /// The outcome is recorded like any loop pass.
    pub async fn start_sync<B: Backend>(&self, backend: &B) -> Result<SyncStart, B::Error> {
        self.start(backend, false).await
    }

    /// Like [`Self::start_sync`], but read the folder from its first message
    /// again: the cursor and quarantine are reset (under the pass lock, so a
    /// running pass cannot move the cursor back). Reports already stored are
    /// skipped, so this adds only what is missing, e.g. after un-ignoring a domain.
    pub async fn start_reimport<B: Backend>(&self, backend: &B) -> Result<SyncStart, B::Error> {
        self.start(backend, true).await
    }

    async fn start<B: Backend>(
        &self,
        backend: &B,
        from_start: bool,
    ) -> Result<SyncStart, B::Error> {
        if backend.config().await?.is_none() {
            return Ok(SyncStart::Unconfigured);
        }
        // Set before taking the lock: a re-import asked for while a pass runs
        // is not lost but runs right after it (like [`Self::request_reimport`]).
        if from_start {
            self.inner.reimport.store(true, Ordering::SeqCst);
        }
        let Ok(guard) = Arc::clone(&self.inner.pass_lock).try_lock_owned() else {
            if from_start {
                self.wake();
            }
            return Ok(SyncStart::AlreadyRunning);
        };
        // Visible to the very next health poll, before the task gets scheduled.
        self.inner.progress.begin(backend.now());
        let worker = self.clone();
        let backend = backend.clone();
        tokio::spawn(async move {
            worker
                .spawn_pass(guard, &backend, CancellationToken::new())
                .await;
        });
        Ok(SyncStart::Started)
    }

    /// Poll until `shutdown` is cancelled, sleeping the configured interval
    /// between passes (or [`UNCONFIGURED_RECHECK`] while unconfigured) unless
    /// [`Self::wake`] is called. On shutdown the running pass (the loop's or a
    /// manual one) stops between batches; its cursor is kept.
    pub async fn run<B: Backend>(self, backend: B, shutdown: CancellationToken) {
        loop {
            let guard = tokio::select! {
                guard = Arc::clone(&self.inner.pass_lock).lock_owned() => guard,
                () = shutdown.cancelled() => {
                    // A manual pass holds the lock; stop it too.
                    self.cancel_current();
                    break;
                }
            };
            let wait = self
                .spawn_pass(guard, &backend, shutdown.child_token())
                .await;
            tokio::select! {
                () = shutdown.cancelled() => {
                    // A manual pass started while the loop slept runs on its
                    // own token; stop it too.
                    self.cancel_current();
                    break;
                }
                () = tokio::time::sleep(wait) => {}
                () = self.inner.wake.notified() => debug!("IMAP loop woken early"),
            }
        }
        debug!("IMAP loop stopped");
    }

    /// Run one pass as a task of its own that holds `guard`, log it, and
    /// return how long to wait before the next. A pass that ends abnormally
    /// (a panic) only ends that pass: the lock is released, its progress
    /// cleared (see [`ProgressReset`]) and the failure recorded for the UI.
    async fn spawn_pass<B: Backend>(
        &self,
        guard: OwnedMutexGuard<()>,
        backend: &B,
        cancel: CancellationToken,
    ) -> Duration {
        let worker = self.clone();
        let task_backend = backend.clone();
        let started_at = backend.now();
        let pass = tokio::spawn(async move {
            let _guard = guard;
            worker
                .pass_locked(&task_backend, cancel, Transport::Network)
                .await
        });
        let (result, wait) = match pass.await {
            Ok(done) => done,
            Err(e) => {
                let error = format!("the mailbox pass ended abnormally: {e}");
                record(
                    backend,
                    PassRecord {
                        started_at,
                        finished_at: backend.now(),
                        checked_at: None,
                        summary: FetchSummary::default(),
                        error: Some(error.clone()),
                    },
                )
                .await;
                (Err(error), RETRY_AFTER_PROGRESS)
            }
        };
        self.inner.passes_done.fetch_add(1, Ordering::SeqCst);
        log_pass(&result);
        wait
    }

    /// One pass (caller holds the pass lock), plus how long the loop should
    /// wait before the next.
    async fn pass_locked<B: Backend>(
        &self,
        backend: &B,
        cancel: CancellationToken,
        transport: Transport,
    ) -> (Result<SyncOutcome, String>, Duration) {
        // However the pass ends, even by a panic, the UI stops showing it.
        let _reset = ProgressReset(self);
        self.set_current(Some(cancel.clone()));
        let started_at = backend.now();
        let cfg = match backend.config().await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return (Ok(SyncOutcome::Unconfigured), UNCONFIGURED_RECHECK),
            Err(e) => {
                return (
                    Err(format!("failed to load IMAP settings: {e}")),
                    UNCONFIGURED_RECHECK,
                )
            }
        };
        // Under the pass lock, so a running pass cannot move the cursor back.
        if self.inner.reimport.swap(false, Ordering::SeqCst) {
            if let Err(e) = backend.reset_folder(&cfg.mailbox_key(), &cfg.folder).await {
                self.inner.reimport.store(true, Ordering::SeqCst);
                return (
                    Err(format!("failed to start the re-import: {e}")),
                    RETRY_AFTER_PROGRESS,
                );
            }
            info!(folder = %cfg.folder, "re-importing the mailbox from its first message");
        }
        let mut wait = Duration::from_secs(cfg.poll_interval_secs);
        if self.inner.progress.get().is_none() {
            self.inner.progress.begin(started_at);
        }
        let limits = Limits::DEFAULT;
        let parser = default_parser();
        let mut pass = Pass::new(
            backend,
            &cfg,
            &self.inner.progress,
            &cancel,
            &limits,
            &parser,
        );
        let outcome = pass.run(transport).await;
        let checked_at = pass.checked_at;
        let (summary, error) = match &outcome {
            Ok(summary) => (*summary, None),
            Err(e) => {
                if e.summary.advanced() {
                    wait = wait.min(RETRY_AFTER_PROGRESS);
                }
                // Errors reach the UI, so mask the password should a server echo it back.
                (e.summary, Some(cfg.redact(&e.to_string())))
            }
        };
        // Neither a success nor a failure of the new settings; the loop reruns now.
        if !summary.interrupted {
            record(
                backend,
                PassRecord {
                    started_at,
                    finished_at: backend.now(),
                    checked_at,
                    summary,
                    error: error.clone(),
                },
            )
            .await;
        }
        let result = match error {
            None => Ok(SyncOutcome::Fetched(summary)),
            Some(e) => Err(e),
        };
        (result, wait)
    }
}

#[cfg(feature = "test-support")]
impl ImapWorker {
    /// One pass as the loop runs it (under the pass lock, with the stored
    /// configuration, its outcome recorded and counted in
    /// [`Self::passes_done`]), reading `server` instead of the network.
    pub async fn sync_with_fake_server<B: Backend>(
        &self,
        backend: &B,
        server: crate::fake_server::FakeServer,
    ) -> Result<SyncOutcome, String> {
        let _guard = self.inner.pass_lock.lock().await;
        let (result, _wait) = self
            .pass_locked(backend, CancellationToken::new(), Transport::Fake(server))
            .await;
        self.inner.passes_done.fetch_add(1, Ordering::SeqCst);
        log_pass(&result);
        result
    }
}

/// Clears the worker's progress and current token when dropped: at the end
/// of a pass, or while unwinding from a panic in it.
struct ProgressReset<'a>(&'a ImapWorker);

impl Drop for ProgressReset<'_> {
    fn drop(&mut self) {
        self.0.set_current(None);
        self.0.inner.progress.end();
    }
}

fn log_pass(result: &Result<SyncOutcome, String>) {
    match result {
        Ok(SyncOutcome::Fetched(summary)) if summary.interrupted => {
            info!(%summary, "IMAP fetch pass stopped early");
        }
        Ok(SyncOutcome::Fetched(summary)) => info!(%summary, "IMAP fetch pass ok"),
        Ok(SyncOutcome::Unconfigured) => debug!("IMAP not configured; skipping fetch pass"),
        Err(e) => error!(error = %e, "IMAP fetch pass failed; will retry"),
    }
}

async fn record<B: Backend>(backend: &B, record: PassRecord) {
    if let Err(e) = backend.record_pass(&record).await {
        warn!(error = %e, "failed to record IMAP fetch outcome");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::MemoryBackend;
    use crate::config::ImapConfig;

    /// A mailbox nothing listens on: the pass fails to connect at once.
    fn unreachable() -> MemoryBackend {
        MemoryBackend::with_config(ImapConfig {
            host: "127.0.0.1".into(),
            port: 1,
            security: crate::ImapSecurity::Tls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
            poll_interval_secs: 300,
        })
    }

    #[tokio::test]
    async fn start_sync_reports_unconfigured_and_already_running() {
        let worker = ImapWorker::new();
        assert_eq!(
            worker.start_sync(&MemoryBackend::default()).await.unwrap(),
            SyncStart::Unconfigured
        );
        assert!(worker.progress().is_none());

        let backend = unreachable();
        {
            let _running = worker.inner.pass_lock.lock().await;
            assert_eq!(
                worker.start_sync(&backend).await.unwrap(),
                SyncStart::AlreadyRunning
            );
        }
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::Started
        );
        // Visible immediately, before the background pass has done anything.
        assert!(worker.progress().is_some());
        // The pass fails to connect, records the error and clears its progress.
        let _done = worker.inner.pass_lock.lock().await;
        assert!(worker.progress().is_none());
        let state = backend.state();
        let pass = state.passes.last().expect("pass recorded");
        assert!(pass
            .error
            .as_deref()
            .is_some_and(|e| e.contains("127.0.0.1:1")));
        assert_eq!(pass.checked_at, None);
        assert!(pass.finished_at >= pass.started_at);
    }

    #[tokio::test]
    async fn a_pass_that_panics_still_clears_its_progress() {
        let worker = ImapWorker::new();
        worker.inner.progress.begin(0);
        let w = worker.clone();
        let task = tokio::spawn(async move {
            let _reset = ProgressReset(&w);
            panic!("simulated pass panic");
        });
        assert!(task.await.unwrap_err().is_panic());
        assert!(worker.progress().is_none());
    }

    #[tokio::test]
    async fn reimport_asked_while_a_pass_runs_is_queued() {
        let backend = unreachable();
        let worker = ImapWorker::new();
        let _running = worker.inner.pass_lock.lock().await;
        assert_eq!(
            worker.start_reimport(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        assert!(worker.reimport_requested());
        // The loop is woken, so it runs the re-import once the pass is done.
        tokio::time::timeout(Duration::from_secs(1), worker.inner.wake.notified())
            .await
            .expect("loop woken");

        // A plain sync asked meanwhile queues nothing.
        let worker = ImapWorker::new();
        let _running = worker.inner.pass_lock.lock().await;
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        assert!(!worker.reimport_requested());
    }

    #[tokio::test]
    async fn settings_change_cancels_the_running_pass() {
        let worker = ImapWorker::new();
        let token = CancellationToken::new();
        worker.set_current(Some(token.clone()));
        worker.settings_changed();
        assert!(token.is_cancelled());
        assert_eq!(worker.settings_generation(), 1);
    }

    #[tokio::test]
    async fn the_loop_stops_on_shutdown_and_records_its_passes() {
        let backend = unreachable();
        let worker = ImapWorker::new();
        assert_eq!(worker.passes_done(), 0);
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        // The first pass fails to connect and is recorded; then the loop sleeps.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while backend.state().passes.is_empty() {
            assert!(tokio::time::Instant::now() < deadline, "no pass recorded");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("loop stopped")
            .unwrap();
        assert!(worker.progress().is_none());
        assert_eq!(backend.state().passes.len(), 1);
        assert_eq!(worker.passes_done(), 1);
    }
}
