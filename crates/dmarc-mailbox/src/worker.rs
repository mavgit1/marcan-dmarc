//! The polling loop over every mailbox the backend lists.
//!
//! The loop ([`MailboxWorker::run`]) is the one driver: only it takes the pass
//! lock and runs passes, one at a time (the host has one database writer,
//! the UI one progress slot). Everything a person asks for (Sync now, a
//! re-import, one account's sync, a settings change) marks the mailboxes in
//! the schedule and wakes the loop; asked while a pass runs, it runs right
//! after. Without a running loop nothing is read.
//!
//! Each mailbox is due its poll interval after its last pass
//! ended; of the due ones, the one whose turn is longest ago goes first
//! (round-robin, in listing order the first time). An importing pass yields
//! between batches once it has run [`YIELD_AFTER`] while another mailbox is
//! due: its cursor is saved per batch, so it goes to the back of the queue,
//! due again at once, and resumes where it stopped. The list and each
//! mailbox's configuration are re-read before every pass, so settings saved
//! in the UI apply without a restart. A settings change stops that
//! mailbox's running pass after its batch ([`MailboxWorker::settings_changed`]);
//! when the change replaced the mailbox, the host has forgotten its state
//! and moved its epoch on already, so the stopped pass can write nothing
//! back and the worker has nothing to forget.
//!
//! A mailbox whose passes keep failing (it cannot be reached, its login or
//! token is refused, the database fails) backs off: each failure in a row
//! waits longer before the next try ([`RETRY_BACKOFF`]), so a broken
//! mailbox does not hammer its server or the token endpoint. A success, a
//! settings change of that mailbox, Sync now, a re-import and a connection
//! test start the back-off over.

use crate::backend::{Backend, MailboxConfig, MailboxRef, PassRecord};
use crate::batch::Limits;
use crate::imap::config::ImapConfig;
use crate::imap::Transport;
use crate::pass::{default_parser, FetchSummary, Pass, YieldPoint};
use crate::progress::{PassProgress, SharedProgress};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{Mutex, Notify, OwnedMutexGuard};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// How often the loop re-reads the mailbox list while it is empty, and when
/// an unconfigured mailbox is tried again.
const UNCONFIGURED_RECHECK: Duration = Duration::from_secs(30);

/// Retry delay after a pass that failed part-way but moved the cursor: the
/// import is clearly working, so continue soon rather than after the interval.
const RETRY_AFTER_PROGRESS: Duration = Duration::from_secs(30);

/// How long a mailbox waits after each failed pass in a row (no progress:
/// it could not be reached, its login or token was refused, the database
/// failed): the first retry after 30 seconds, then longer; after the last
/// step, the poll interval when that is longer. The one schedule for every
/// mailbox ([`retry_after`]).
pub const RETRY_BACKOFF: [Duration; 6] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_mins(2),
    Duration::from_mins(5),
    Duration::from_mins(15),
    Duration::from_hours(1),
];

/// The wait after `failures` failed passes in a row (at least one) of a
/// mailbox polled every `poll`: [`RETRY_BACKOFF`]'s step, capped at the
/// longer of `poll` and the last step, which every later failure waits.
pub fn retry_after(failures: u32, poll: Duration) -> Duration {
    let last = RETRY_BACKOFF[RETRY_BACKOFF.len() - 1];
    let cap = poll.max(last);
    let step = usize::try_from(failures.saturating_sub(1)).unwrap_or(usize::MAX);
    RETRY_BACKOFF.get(step).copied().unwrap_or(cap).min(cap)
}

/// When a mailbox is due after its pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// After its interval: the pass read it.
    Interval,
    /// After [`UNCONFIGURED_RECHECK`]: nothing to read with yet.
    Recheck,
    /// At once: it gave way to another mailbox or was stopped by a settings
    /// change, so its import is unfinished.
    Resume,
    /// After [`RETRY_AFTER_PROGRESS`]: it failed part-way after moving the
    /// cursor; its import is unfinished.
    Continue,
    /// After its back-off ([`retry_after`]): it failed without progress.
    Failed,
}

/// How long an importing pass runs before it gives way to another mailbox
/// that is due.
pub const YIELD_AFTER: Duration = Duration::from_secs(120);

/// Result of one worker pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The mailbox has no usable configuration; nothing was fetched.
    Unconfigured,
    Fetched(FetchSummary),
}

/// Answer to [`MailboxWorker::start_sync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStart {
    /// The passes were started in the background.
    Started,
    /// A pass is already running; its progress is in [`MailboxWorker::progress`].
    AlreadyRunning,
    /// The backend lists no mailbox.
    Unconfigured,
}

/// One listed mailbox's place in the schedule.
#[derive(Debug)]
struct Slot {
    id: String,
    poll: Duration,
    /// When its last pass ended; `None` before its first (due at once).
    last_end: Option<Instant>,
    /// What the last pass asked for instead of the interval (at most the
    /// interval): a retry, the unconfigured recheck, or zero (due again at
    /// once). `None` with `failures`: the back-off.
    delay: Option<Duration>,
    /// Failed passes in a row (none since a success or a reset).
    failures: u32,
    /// Its last pass left its import unfinished ([`Next::Resume`],
    /// [`Next::Continue`]).
    unfinished: bool,
    /// Asked for now: a settings change, a manual sync, a re-import.
    due_now: bool,
    /// Its pass runs: not due, whatever the clock says.
    running: bool,
    /// When it last took its turn, as a count: the least recent goes first.
    turn: u64,
    /// Its next pass reads the folder from its first message.
    reimport: bool,
    /// It ended a pass since startup.
    ended: bool,
}

impl Slot {
    fn new(mailbox: &MailboxRef) -> Self {
        Self {
            id: mailbox.id.clone(),
            poll: mailbox.poll,
            last_end: None,
            delay: None,
            failures: 0,
            unfinished: false,
            due_now: false,
            running: false,
            turn: 0,
            reimport: false,
            ended: false,
        }
    }

    /// When it is due; `None` while its pass runs.
    fn due_at(&self, now: Instant) -> Option<Instant> {
        if self.running {
            return None;
        }
        match self.last_end {
            Some(end) if !self.due_now => Some(end + self.wait()),
            _ => Some(now),
        }
    }

    /// How long after its last pass it is due.
    fn wait(&self) -> Duration {
        match self.delay {
            Some(delay) => delay.min(self.poll),
            None if self.failures > 0 => retry_after(self.failures, self.poll),
            None => self.poll,
        }
    }

    /// Asked for by a person (a settings change, Sync now, a re-import):
    /// due at once, its back-off forgotten.
    fn ask(&mut self) {
        self.due_now = true;
        self.failures = 0;
    }

    fn is_due(&self, now: Instant) -> bool {
        self.due_at(now).is_some_and(|at| at <= now)
    }
}

#[derive(Debug, Default)]
struct Schedule {
    /// The mailboxes of the last listing, in its order.
    slots: Vec<Slot>,
    /// Turns taken since startup.
    turns: u64,
    /// A re-import of every mailbox was asked for before the next listing
    /// (kept until a listing has one).
    pending_reimport: bool,
    /// The running pass's mailbox and cancellation, so a settings change
    /// stops it between batches (shutdown cancels the loop's own token).
    current: Option<(String, CancellationToken)>,
    /// Every listed mailbox has ended one pass since startup (sticky).
    round_complete: bool,
    /// The loop runs: only then is a manual start's progress announced.
    looping: bool,
    /// A manual start published the progress of the pass the loop runs
    /// next, before the loop got to it ([`MailboxWorker::start_sync`]). The
    /// pass takes it over when it is claimed; the loop clears it when it
    /// runs nothing after all, and when it stops.
    announced: bool,
}

impl Schedule {
    /// Whether a pass runs, or a manual start announced one.
    fn busy(&self) -> bool {
        self.current.is_some() || self.announced || self.slots.iter().any(|s| s.running)
    }

    fn slot_mut(&mut self, id: &str) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    /// Take the backend's listing: known mailboxes keep their place, new
    /// ones are due at once, unlisted ones are forgotten.
    fn update(&mut self, listing: &[MailboxRef]) {
        let mut known: HashMap<String, Slot> =
            self.slots.drain(..).map(|s| (s.id.clone(), s)).collect();
        self.slots = listing
            .iter()
            .map(|m| {
                let mut slot = known.remove(&m.id).unwrap_or_else(|| Slot::new(m));
                slot.poll = m.poll;
                slot
            })
            .collect();
        if self.pending_reimport {
            for slot in &mut self.slots {
                slot.reimport = true;
                slot.ask();
            }
            self.pending_reimport = self.slots.is_empty();
        }
        self.check_round();
    }

    fn check_round(&mut self) {
        if self.slots.iter().all(|s| s.ended) {
            self.round_complete = true;
        }
    }

    /// The due mailbox whose turn is longest ago.
    fn next(&self, now: Instant) -> Option<&str> {
        self.slots
            .iter()
            .filter(|s| s.is_due(now))
            .min_by_key(|s| s.turn)
            .map(|s| s.id.as_str())
    }

    /// [`Self::next`], marked running.
    fn pick(&mut self, now: Instant) -> Option<String> {
        let id = self.next(now)?.to_owned();
        self.claim(&id);
        Some(id)
    }

    /// Mark `id` running; its pass owns the progress from here.
    fn claim(&mut self, id: &str) {
        self.announced = false;
        if let Some(slot) = self.slot_mut(id) {
            slot.running = true;
            slot.due_now = false;
        }
    }

    /// Whether a mailbox other than `id` is due.
    fn others_due(&self, id: &str, now: Instant) -> bool {
        self.slots.iter().any(|s| s.id != id && s.is_due(now))
    }

    /// When the next mailbox is due; `None` when none is listed or all run.
    fn next_due(&self, now: Instant) -> Option<Instant> {
        self.slots.iter().filter_map(|s| s.due_at(now)).min()
    }

    /// `id`'s pass ended: it goes to the back of the queue, due as `next`
    /// says.
    fn finish(&mut self, id: &str, next: Next, now: Instant) {
        self.turns += 1;
        let turn = self.turns;
        if let Some(slot) = self.slot_mut(id) {
            slot.running = false;
            slot.last_end = Some(now);
            slot.delay = match next {
                Next::Interval | Next::Failed => None,
                Next::Recheck => Some(UNCONFIGURED_RECHECK),
                Next::Resume => Some(Duration::ZERO),
                Next::Continue => Some(RETRY_AFTER_PROGRESS),
            };
            slot.failures = match next {
                Next::Failed => slot.failures.saturating_add(1),
                _ => 0,
            };
            slot.unfinished = matches!(next, Next::Resume | Next::Continue);
            slot.turn = turn;
            slot.ended = true;
        }
        self.check_round();
    }
}

/// Makes the scripted server for a pass of a mailbox, from its
/// configuration; `None`: the network (tests only).
#[cfg(any(test, feature = "test-support"))]
type FakeServers =
    Box<dyn FnMut(&ImapConfig) -> Option<crate::imap::fake_server::FakeServer> + Send>;

struct Inner {
    /// Held by the running pass; passes are serialized. Its own `Arc` so a
    /// pass can own its guard (`lock_owned`).
    pass_lock: Arc<Mutex<()>>,
    wake: Notify,
    progress: SharedProgress,
    schedule: StdMutex<Schedule>,
    /// How many times the settings changed; for tests.
    #[cfg(any(test, feature = "test-support"))]
    generation: std::sync::atomic::AtomicU64,
    limits: Limits,
    #[cfg(any(test, feature = "test-support"))]
    fakes: StdMutex<Option<FakeServers>>,
}

/// Runs the fetch passes of the polling loop, the one driver, and takes the
/// person's asks for it.
///
/// Passes are serialized: at most one runs at a time, and a manual sync
/// while one runs reports that instead of waiting.
#[derive(Clone)]
pub struct MailboxWorker {
    inner: Arc<Inner>,
}

impl Default for MailboxWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl MailboxWorker {
    pub fn new() -> Self {
        Self::with_limits(Limits::DEFAULT)
    }

    fn with_limits(limits: Limits) -> Self {
        Self {
            inner: Arc::new(Inner {
                pass_lock: Arc::new(Mutex::new(())),
                wake: Notify::new(),
                progress: SharedProgress::default(),
                schedule: StdMutex::new(Schedule::default()),
                #[cfg(any(test, feature = "test-support"))]
                generation: std::sync::atomic::AtomicU64::new(0),
                limits,
                #[cfg(any(test, feature = "test-support"))]
                fakes: StdMutex::new(None),
            }),
        }
    }

    fn schedule(&self) -> MutexGuard<'_, Schedule> {
        self.inner
            .schedule
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Make the loop re-read the mailbox list now instead of after its sleep:
    /// a changed interval applies at once, a mailbox that became due runs.
    pub fn wake(&self) {
        self.inner.wake.notify_one();
    }

    /// Re-import every mailbox with its next pass, which starts now or right
    /// after the running one: nothing is lost to a pass in progress. Without a
    /// mailbox there is nothing to read, and the first pass reads everything.
    pub fn request_reimport(&self) {
        self.schedule().pending_reimport = true;
        self.wake();
    }

    /// Poll mailbox `id` now (or right after the running pass), without
    /// interrupting anything.
    pub fn poll_now(&self, id: &str) {
        if let Some(slot) = self.schedule().slot_mut(id) {
            slot.ask();
        }
        self.wake();
    }

    /// Mailbox `id`'s login was tested: if its passes were failing, the
    /// next one waits only the first step of the back-off after the last
    /// (most likely due at once), without interrupting anything.
    pub fn reset_backoff(&self, id: &str) {
        if let Some(slot) = self.schedule().slot_mut(id) {
            if slot.failures > 0 {
                slot.failures = 0;
                slot.delay = Some(RETRY_BACKOFF[0]);
            }
        }
        self.wake();
    }

    /// Failed passes in a row of mailbox `id`; for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn failures(&self, id: &str) -> u32 {
        self.schedule().slot_mut(id).map_or(0, |s| s.failures)
    }

    /// Mailbox `id`'s settings were saved: its running pass stops after the
    /// current batch (keeping its cursor, unless the host has moved the
    /// mailbox to another [`MailboxConfig::epoch`] and so drops the pass's
    /// writes) and it is due at once with the new settings. Another
    /// mailbox's pass is left alone.
    pub fn settings_changed(&self, id: &str) {
        #[cfg(any(test, feature = "test-support"))]
        self.inner
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        {
            let mut schedule = self.schedule();
            if let Some(slot) = schedule.slot_mut(id) {
                slot.ask();
            }
            if let Some((running, token)) = &schedule.current {
                if running == id {
                    token.cancel();
                }
            }
        }
        self.wake();
    }

    /// Whether a re-import waits for a pass; for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reimport_requested(&self) -> bool {
        let schedule = self.schedule();
        schedule.pending_reimport || schedule.slots.iter().any(|s| s.reimport)
    }

    /// How many times [`Self::settings_changed`] was called; for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn settings_generation(&self) -> u64 {
        self.inner
            .generation
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Progress of the running pass, or of the one a manual start announced;
    /// `None` while idle.
    pub fn progress(&self) -> Option<PassProgress> {
        self.inner.progress.get()
    }

    /// Whether every mailbox has ended one pass since startup, however it
    /// ended: `false` until the list was read and each listed mailbox had
    /// its first pass (the alert job waits for that), `true` at once when
    /// none is listed. It stays `true` afterwards.
    pub fn round_complete(&self) -> bool {
        self.schedule().round_complete
    }

    /// Whether an import is unfinished: a pass runs, a listed mailbox's
    /// last pass left its import unfinished (it gave way to another
    /// mailbox, was stopped by a settings change, or failed after moving its
    /// cursor: due again at once or soon), or a re-import waits for its
    /// pass. Between the passes of a long import this stays `true`, where
    /// [`Self::progress`] is `None`; the alert job holds its evaluation on it.
    pub fn import_unfinished(&self) -> bool {
        let schedule = self.schedule();
        schedule.current.is_some()
            || schedule
                .slots
                .iter()
                .any(|s| s.running || s.unfinished || s.reimport)
    }

    fn set_current(&self, current: Option<(String, CancellationToken)>) {
        self.schedule().current = current;
    }

    /// Read the backend's mailbox list into the schedule.
    async fn refresh<B: Backend>(&self, backend: &B) -> Result<(), B::Error> {
        let listing = backend.mailboxes().await?;
        self.schedule().update(&listing);
        Ok(())
    }

    /// Ask the loop to sync every mailbox now, one after the other, and
    /// return at once: [`SyncStart::AlreadyRunning`] while a pass runs (the
    /// sync follows right after it). The outcomes are recorded like any
    /// loop pass.
    pub async fn start_sync<B: Backend>(&self, backend: &B) -> Result<SyncStart, B::Error> {
        self.start(backend, false).await
    }

    /// Like [`Self::start_sync`], but read every folder from its first message
    /// again: each cursor and quarantine is reset by the mailbox's next pass,
    /// under the pass lock, so a running pass cannot move the cursor back.
    /// Reports already stored are skipped, so this adds only what is missing,
    /// e.g. after un-ignoring a domain.
    pub async fn start_reimport<B: Backend>(&self, backend: &B) -> Result<SyncStart, B::Error> {
        self.start(backend, true).await
    }

    async fn start<B: Backend>(
        &self,
        backend: &B,
        from_start: bool,
    ) -> Result<SyncStart, B::Error> {
        let listing = backend.mailboxes().await?;
        if listing.is_empty() {
            return Ok(SyncStart::Unconfigured);
        }
        let answer = {
            let mut schedule = self.schedule();
            // Marked whatever the answer: asked while a pass runs, it is
            // not lost but runs right after it (like [`Self::poll_now`]).
            if from_start {
                schedule.pending_reimport = true;
            }
            schedule.update(&listing);
            for slot in &mut schedule.slots {
                slot.ask();
            }
            if schedule.busy() {
                SyncStart::AlreadyRunning
            } else {
                // Visible to the very next health poll, before the loop gets
                // scheduled: the progress of the mailbox it picks next.
                let first = schedule.next(Instant::now()).map(str::to_owned);
                if let (true, Some(id)) = (schedule.looping, first) {
                    schedule.announced = true;
                    self.inner.progress.begin(backend.now(), &id);
                }
                SyncStart::Started
            }
        };
        self.wake();
        Ok(answer)
    }

    /// The loop runs nothing now: a manual start's announced progress goes,
    /// unless a mailbox became due meanwhile (the start woke the loop, which
    /// runs it next). With `stopping`, it goes whatever is due, and no start
    /// announces one any more.
    fn drop_announcement(&self, listed: bool, stopping: bool) {
        let mut schedule = self.schedule();
        if stopping {
            schedule.looping = false;
        }
        let keep = listed && !stopping && schedule.next(Instant::now()).is_some();
        if schedule.announced && !keep {
            schedule.announced = false;
            self.inner.progress.end();
        }
    }

    /// Poll until `shutdown` is cancelled: re-read the mailbox list, run the
    /// next due mailbox's pass, or sleep until one is due (or [`Self::wake`]
    /// is called). On shutdown the running pass stops between batches; its
    /// cursor is kept, and nothing runs afterwards.
    pub async fn run<B: Backend>(self, backend: B, shutdown: CancellationToken) {
        self.schedule().looping = true;
        self.poll(&backend, &shutdown).await;
        self.drop_announcement(false, true);
        debug!("mailbox loop stopped");
    }

    async fn poll<B: Backend>(&self, backend: &B, shutdown: &CancellationToken) {
        loop {
            let listed = self.refresh(backend).await;
            if let Err(e) = &listed {
                error!(error = %e, "failed to list the mailboxes; will retry");
            }
            let due = listed.is_ok() && self.schedule().next(Instant::now()).is_some();
            if due {
                let guard = tokio::select! {
                    guard = Arc::clone(&self.inner.pass_lock).lock_owned() => guard,
                    () = shutdown.cancelled() => return,
                };
                if self
                    .run_next(guard, backend, shutdown.child_token())
                    .await
                    .is_some()
                {
                    if shutdown.is_cancelled() {
                        return;
                    }
                    continue;
                }
            }
            self.drop_announcement(listed.is_ok(), false);
            let now = Instant::now();
            let wait = match (&listed, self.schedule().next_due(now)) {
                (Ok(()), Some(at)) => at.saturating_duration_since(now),
                _ => UNCONFIGURED_RECHECK,
            };
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
                () = self.inner.wake.notified() => debug!("mailbox loop woken early"),
            }
        }
    }

    /// Run the next due mailbox's pass while holding `guard`; `None` when
    /// none is due.
    async fn run_next<B: Backend>(
        &self,
        guard: OwnedMutexGuard<()>,
        backend: &B,
        cancel: CancellationToken,
    ) -> Option<Result<SyncOutcome, String>> {
        let id = self.schedule().pick(Instant::now())?;
        Some(self.spawn_pass(guard, backend, id, cancel).await)
    }

    /// Run mailbox `id`'s pass as a task of its own that holds `guard`, log
    /// it and put the mailbox back in the queue. A pass that ends abnormally
    /// (a panic) only ends that pass: the lock is released, its progress
    /// cleared (see [`ProgressReset`]) and the failure recorded for the UI.
    async fn spawn_pass<B: Backend>(
        &self,
        guard: OwnedMutexGuard<()>,
        backend: &B,
        id: String,
        cancel: CancellationToken,
    ) -> Result<SyncOutcome, String> {
        let worker = self.clone();
        let task_backend = backend.clone();
        let task_id = id.clone();
        let started_at = backend.now();
        let pass = tokio::spawn(async move {
            let _guard = guard;
            worker.pass_locked(&task_backend, &task_id, cancel).await
        });
        let (result, next) = match pass.await {
            Ok(done) => done,
            Err(e) => {
                let error = format!("the mailbox pass ended abnormally: {e}");
                record(
                    backend,
                    PassRecord {
                        mailbox: id.clone(),
                        epoch: None,
                        started_at,
                        finished_at: backend.now(),
                        checked_at: None,
                        summary: FetchSummary::default(),
                        error: Some(error.clone()),
                        error_detail: None,
                    },
                )
                .await;
                (Err(error), Next::Failed)
            }
        };
        self.schedule().finish(&id, next, Instant::now());
        log_pass(&id, &result);
        result
    }

    /// How a pass reaches `cfg`'s mailbox: in tests, the scripted server
    /// the worker was given for it, else the network.
    fn transport(&self, _cfg: &ImapConfig) -> Transport {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(make) = self
            .inner
            .fakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            if let Some(server) = make(_cfg) {
                return Transport::Fake(server);
            }
        }
        Transport::Network
    }

    /// One pass of mailbox `id` (caller holds the pass lock), plus when its
    /// next is due.
    async fn pass_locked<B: Backend>(
        &self,
        backend: &B,
        id: &str,
        cancel: CancellationToken,
    ) -> (Result<SyncOutcome, String>, Next) {
        // However the pass ends, even by a panic, the UI stops showing it.
        let _reset = ProgressReset(self);
        self.set_current(Some((id.to_owned(), cancel.clone())));
        let started = Instant::now();
        let started_at = backend.now();
        let cfg = match backend.prepare(id).await {
            Ok(Some(cfg)) => cfg,
            Ok(None) => return (Ok(SyncOutcome::Unconfigured), Next::Recheck),
            Err(e) => {
                // An expired secret, a refused token, a server that may not
                // have the login: the mailbox was not reached, and says why.
                let error = e.to_string();
                record(
                    backend,
                    PassRecord {
                        mailbox: id.to_owned(),
                        epoch: None,
                        started_at,
                        finished_at: backend.now(),
                        checked_at: None,
                        summary: FetchSummary::default(),
                        error: Some(error.clone()),
                        error_detail: None,
                    },
                )
                .await;
                return (
                    Err(format!("failed to prepare the mailbox: {error}")),
                    Next::Failed,
                );
            }
        };
        // Under the pass lock, so a running pass cannot move the cursor back.
        let reimport = self
            .schedule()
            .slot_mut(id)
            .is_some_and(|s| std::mem::take(&mut s.reimport));
        if reimport {
            if let Err(e) = backend.reset_folder(cfg.id(), cfg.folder()).await {
                if let Some(slot) = self.schedule().slot_mut(id) {
                    slot.reimport = true;
                }
                return (
                    Err(format!("failed to start the re-import: {e}")),
                    Next::Failed,
                );
            }
            info!(mailbox = id, folder = %cfg.folder(), "re-importing the mailbox from its first message");
        }
        if self.inner.progress.get().is_none_or(|p| p.mailbox != id) {
            self.inner.progress.begin(started_at, id);
        }
        let parser = default_parser();
        let others_due: Arc<dyn Fn() -> bool + Send + Sync> = {
            let worker = self.clone();
            let id = id.to_owned();
            Arc::new(move || worker.schedule().others_due(&id, Instant::now()))
        };
        let transport = match &cfg {
            MailboxConfig::Imap(imap) => self.transport(imap),
            // Graph reads its endpoint, a fake one in tests.
            MailboxConfig::Graph(_) => Transport::Network,
        };
        let mut pass = Pass::new(
            backend,
            &cfg,
            &self.inner.progress,
            &cancel,
            &self.inner.limits,
            &parser,
        );
        pass.yield_point = Some(YieldPoint {
            at: started + YIELD_AFTER,
            others_due,
        });
        let outcome = pass.run(transport).await;
        let checked_at = pass.checked_at;
        let (summary, error, error_detail, mut next) = match &outcome {
            Ok(summary) => (*summary, None, None, Next::Interval),
            Err(e) => {
                let next = if e.summary.progressed {
                    Next::Continue
                } else {
                    Next::Failed
                };
                // Errors reach the UI, so mask the secret should a server echo it back.
                let detail = Some(cfg.redact(&e.detail)).filter(|d| !d.is_empty());
                (e.summary, Some(cfg.redact(&e.sentence())), detail, next)
            }
        };
        if summary.interrupted || summary.yielded {
            // Due again at once: with the new settings, or after the others.
            next = Next::Resume;
        }
        // Neither a success nor a failure of the new settings; it reruns now.
        if !summary.interrupted {
            record(
                backend,
                PassRecord {
                    mailbox: cfg.id().to_owned(),
                    epoch: Some(cfg.epoch()),
                    started_at,
                    finished_at: backend.now(),
                    checked_at,
                    summary,
                    error: error.clone(),
                    error_detail,
                },
            )
            .await;
        }
        let result = match error {
            None => Ok(SyncOutcome::Fetched(summary)),
            Some(e) => Err(e),
        };
        (result, next)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl MailboxWorker {
    /// Hand each pass `fakes`' scripted server (`None` restores the network);
    /// what it replaced.
    fn set_fakes(&self, fakes: Option<FakeServers>) -> Option<FakeServers> {
        std::mem::replace(
            &mut *self
                .inner
                .fakes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            fakes,
        )
    }

    /// One pass for a test without the loop: the first listed mailbox is
    /// asked for now and the next due one runs through the loop's own
    /// [`Self::run_next`] (under the pass lock, with its stored
    /// configuration, its outcome recorded and counted for
    /// [`Self::round_complete`]), reading `server` instead of the network.
    /// Not for use beside a running loop.
    pub async fn sync_with_fake_server<B: Backend>(
        &self,
        backend: &B,
        server: crate::imap::fake_server::FakeServer,
    ) -> Result<SyncOutcome, String> {
        self.sync_once(backend, Some(server)).await
    }

    /// [`Self::sync_with_fake_server`] for a mailbox read over Graph, which
    /// reads the endpoint its configuration names (a fake Graph in tests).
    pub async fn sync_graph<B: Backend>(&self, backend: &B) -> Result<SyncOutcome, String> {
        self.sync_once(backend, None).await
    }

    async fn sync_once<B: Backend>(
        &self,
        backend: &B,
        server: Option<crate::imap::fake_server::FakeServer>,
    ) -> Result<SyncOutcome, String> {
        let guard = Arc::clone(&self.inner.pass_lock).lock_owned().await;
        self.refresh(backend)
            .await
            .map_err(|e| format!("failed to list the mailboxes: {e}"))?;
        match self.schedule().slots.first_mut() {
            Some(slot) => slot.ask(),
            None => return Ok(SyncOutcome::Unconfigured),
        }
        let mut server = server;
        let before = self.set_fakes(Some(Box::new(move |_| server.take())));
        let result = self
            .run_next(guard, backend, CancellationToken::new())
            .await;
        self.set_fakes(before);
        result.unwrap_or(Ok(SyncOutcome::Unconfigured))
    }
}

#[cfg(test)]
impl MailboxWorker {
    /// A worker whose passes read the scripted server `make` returns for
    /// each mailbox instead of the network, with `limits`.
    fn with_fake_servers(
        limits: Limits,
        make: Arc<dyn Fn(&ImapConfig) -> crate::imap::fake_server::FakeServer + Send + Sync>,
    ) -> Self {
        let worker = Self::with_limits(limits);
        worker.set_fakes(Some(Box::new(move |cfg| Some(make(cfg)))));
        worker
    }
}

/// Clears the worker's progress and current token when dropped: at the end
/// of a pass, or while unwinding from a panic in it.
struct ProgressReset<'a>(&'a MailboxWorker);

impl Drop for ProgressReset<'_> {
    fn drop(&mut self) {
        self.0.set_current(None);
        self.0.inner.progress.end();
    }
}

fn log_pass(mailbox: &str, result: &Result<SyncOutcome, String>) {
    match result {
        Ok(SyncOutcome::Fetched(summary)) if summary.interrupted => {
            info!(mailbox, %summary, "mailbox pass stopped early");
        }
        Ok(SyncOutcome::Fetched(summary)) if summary.yielded => {
            info!(mailbox, %summary, "mailbox pass gave way to another mailbox");
        }
        Ok(SyncOutcome::Fetched(summary)) => info!(mailbox, %summary, "mailbox pass ok"),
        Ok(SyncOutcome::Unconfigured) => {
            debug!(mailbox, "mailbox not configured; skipping its pass");
        }
        Err(e) => error!(mailbox, error = %e, "mailbox pass failed; will retry"),
    }
}

async fn record<B: Backend>(backend: &B, record: PassRecord) {
    if let Err(e) = backend.record_pass(&record).await {
        warn!(error = %e, "failed to record the mailbox pass");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory::MemoryBackend;
    use crate::backend::Cursor;
    use crate::imap::fake_server::{FakeMessage, FakeServer};

    fn mailbox(id: &str, host: &str, port: u16) -> ImapConfig {
        ImapConfig {
            id: id.into(),
            epoch: 0,
            host: host.into(),
            port,
            security: crate::ImapSecurity::Tls,
            username: "u".into(),
            password: "p".into(),
            folder: "INBOX".into(),
        }
    }

    /// A mailbox nothing listens on: the pass fails to connect at once.
    /// The last UID a stored IMAP cursor names.
    fn last_uid(cursor: &Cursor) -> u32 {
        cursor.position.parse().unwrap()
    }

    fn unreachable() -> MemoryBackend {
        MemoryBackend::with_config(mailbox("m1", "127.0.0.1", 1))
    }

    fn minutes(n: u64) -> Duration {
        Duration::from_secs(60 * n)
    }

    /// Two mailboxes, `a` and `b`, polled every five minutes, each read from
    /// its fake server with `uids` reports (`a` the first, `b` the second).
    fn two_mailboxes(
        a: &'static [u32],
        b: &'static [u32],
        body_fetch_delay: Duration,
    ) -> (MailboxWorker, MemoryBackend) {
        let backend = MemoryBackend::with_mailboxes(vec![
            (mailbox("a", "imap.a.example", 993), minutes(5)),
            (mailbox("b", "imap.b.example", 993), minutes(5)),
        ]);
        let limits = Limits {
            batch_max_messages: 2,
            batch_timeout_base: minutes(5),
            ..Limits::DEFAULT
        };
        let worker = MailboxWorker::with_fake_servers(
            limits,
            Arc::new(move |cfg: &ImapConfig| {
                let (uid_validity, uids) = if cfg.id == "a" { (1, a) } else { (2, b) };
                let messages = uids
                    .iter()
                    .map(|&uid| FakeMessage::report(uid_validity * 1000 + uid))
                    .collect();
                FakeServer::new(uid_validity, messages).delay_body_fetch(body_fetch_delay)
            }),
        );
        (worker, backend)
    }

    /// Run the loop until `done` holds for the backend (in paused time).
    async fn run_until(
        worker: &MailboxWorker,
        backend: &MemoryBackend,
        done: impl Fn(&MemoryBackend) -> bool,
    ) {
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        let deadline = Instant::now() + Duration::from_secs(3600);
        while !done(backend) {
            assert!(Instant::now() < deadline, "{:?}", backend.pass_order());
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        shutdown.cancel();
        task.await.unwrap();
    }

    /// Wait (in real time) until the loop ended `passes` passes and runs none.
    async fn until_idle(worker: &MailboxWorker, backend: &MemoryBackend, passes: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while backend.state().passes.len() < passes || worker.schedule().busy() {
            assert!(Instant::now() < deadline, "the loop did not settle");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn start_sync_reports_unconfigured_and_already_running() {
        let worker = MailboxWorker::new();
        assert_eq!(
            worker.start_sync(&MemoryBackend::default()).await.unwrap(),
            SyncStart::Unconfigured
        );
        assert!(worker.progress().is_none());

        // A pass runs: the answer comes from the schedule.
        let backend = unreachable();
        worker.refresh(&backend).await.unwrap();
        worker.schedule().claim("m1");
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        worker.schedule().finish("m1", Next::Failed, Instant::now());

        // The loop runs that sync, fails to connect and backs off.
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        until_idle(&worker, &backend, 1).await;
        assert!(worker.progress().is_none());

        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::Started
        );
        // Visible immediately, before the loop has done anything.
        assert_eq!(worker.progress().map(|p| p.mailbox).as_deref(), Some("m1"));
        // Asked again before the loop got to it: it is running.
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        // The pass fails to connect, records the error and clears its progress.
        until_idle(&worker, &backend, 2).await;
        assert!(worker.progress().is_none());
        shutdown.cancel();
        task.await.unwrap();
        let state = backend.state();
        assert_eq!(state.passes.len(), 2, "{:?}", state.passes);
        let pass = state.passes.last().expect("pass recorded");
        assert_eq!(pass.mailbox, "m1");
        assert!(pass
            .error
            .as_deref()
            .is_some_and(|e| e.contains("127.0.0.1:1")));
        assert_eq!(pass.checked_at, None);
        assert!(pass.finished_at >= pass.started_at);
    }

    /// The progress a start announced never outlives it: the loop clears it
    /// when it runs nothing after all (the mailbox went), and when it stops
    /// first; after the loop stopped, a start announces nothing.
    #[tokio::test]
    async fn an_announced_sync_is_cleared_when_the_loop_runs_nothing() {
        let backend = unreachable();
        let worker = MailboxWorker::new();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        until_idle(&worker, &backend, 1).await;
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::Started
        );
        assert!(worker.progress().is_some());
        backend.remove("m1");
        let deadline = Instant::now() + Duration::from_secs(10);
        while worker.progress().is_some() {
            assert!(Instant::now() < deadline, "the progress stayed");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!worker.schedule().busy());
        assert_eq!(backend.state().passes.len(), 1);
        shutdown.cancel();
        task.await.unwrap();

        let backend = unreachable();
        let worker = MailboxWorker::new();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        until_idle(&worker, &backend, 1).await;
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::Started
        );
        assert!(worker.progress().is_some());
        shutdown.cancel();
        task.await.unwrap();
        assert!(worker.progress().is_none());
        assert!(!worker.schedule().busy());
        // Nothing runs after shutdown, and nothing claims to.
        let passes = backend.state().passes.len();
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::Started
        );
        assert!(worker.progress().is_none());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(backend.state().passes.len(), passes);
    }

    #[tokio::test]
    async fn a_pass_that_panics_still_clears_its_progress() {
        let worker = MailboxWorker::new();
        worker.inner.progress.begin(0, "m1");
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
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        worker.schedule().claim("m1");
        assert_eq!(
            worker.start_reimport(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        assert!(worker.reimport_requested());
        // The loop is woken, so it runs the re-import once the pass is done.
        tokio::time::timeout(Duration::from_secs(1), worker.inner.wake.notified())
            .await
            .expect("loop woken");
        // The next listing hands it to the mailbox, due at once.
        worker.refresh(&backend).await.unwrap();
        {
            let mut schedule = worker.schedule();
            let slot = schedule.slot_mut("m1").unwrap();
            assert!(slot.reimport && slot.due_now);
        }

        // A plain sync asked meanwhile queues nothing.
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        worker.schedule().claim("m1");
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        assert!(!worker.reimport_requested());
    }

    /// "Sync now" while a pass runs answers that it runs, and every mailbox
    /// is read right after it, not after its interval.
    #[tokio::test]
    async fn a_sync_asked_while_a_pass_runs_makes_every_mailbox_due() {
        let backend = MemoryBackend::with_mailboxes(vec![
            (mailbox("a", "imap.a.example", 993), minutes(5)),
            (mailbox("b", "imap.b.example", 993), minutes(5)),
        ]);
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        let now = Instant::now();
        for id in ["a", "b"] {
            worker.schedule().finish(id, Next::Interval, now);
        }
        assert_eq!(worker.schedule().next(now), None);
        worker.schedule().claim("a");
        assert_eq!(
            worker.start_sync(&backend).await.unwrap(),
            SyncStart::AlreadyRunning
        );
        // `b` at once, `a` as soon as its pass ends.
        assert_eq!(worker.schedule().next(now), Some("b"));
        worker.schedule().finish("a", Next::Interval, now);
        assert!(worker.schedule().slots.iter().all(|s| s.is_due(now)));
        // The loop is woken to run them once the pass is done.
        tokio::time::timeout(Duration::from_secs(1), worker.inner.wake.notified())
            .await
            .expect("loop woken");
    }

    /// A re-import asked while the loop's pass runs answers that a pass
    /// runs, and the next pass, right after it, reads the folder from its
    /// first message: the reset waited for the running pass, whose cursor
    /// saves came before it.
    #[tokio::test(start_paused = true)]
    async fn a_reimport_asked_while_a_pass_runs_follows_it_from_the_start() {
        let (worker, backend) = replaceable();
        let (w, b) = (worker.clone(), backend.clone());
        let asked = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            w.start_reimport(&b).await.unwrap()
        });
        let started = Instant::now();
        run_until(&worker, &backend, |b| b.state().passes.len() >= 2).await;
        assert_eq!(asked.await.unwrap(), SyncStart::AlreadyRunning);
        assert!(started.elapsed() < minutes(5), "not after the interval");
        let state = backend.state();
        let first = &state.passes[0].summary;
        assert_eq!((first.reports_ingested, first.end_position), (4, Some(14)));
        assert!(!first.interrupted, "{first}");
        let again = &state.passes[1].summary;
        assert_eq!(again.start_position, None, "{again}");
        assert_eq!((again.messages_seen, again.reports_ingested), (4, 0));
        assert_eq!(
            state.cursors.get(&("a".into(), "INBOX".into())),
            Some(&Cursor {
                generation: "1".into(),
                position: "14".into()
            })
        );
        drop(state);
        assert!(!worker.reimport_requested());
    }

    /// Shutdown stops the running pass after its batch: the cursor that
    /// batch saved stays, the stopped pass is not recorded, and nothing runs
    /// afterwards.
    #[tokio::test(start_paused = true)]
    async fn shutdown_stops_the_running_pass_between_batches() {
        let backend = MemoryBackend::with_config(mailbox("a", "imap.a.example", 993));
        let limits = Limits {
            batch_max_messages: 2,
            batch_timeout_base: minutes(5),
            ..Limits::DEFAULT
        };
        let worker = MailboxWorker::with_fake_servers(
            limits,
            Arc::new(|_: &ImapConfig| {
                let messages = (1..=10).map(FakeMessage::report).collect();
                FakeServer::new(1, messages).delay_body_fetch(Duration::from_secs(50))
            }),
        );
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        // Five batches of 50 s: stopped in the third.
        tokio::time::sleep(Duration::from_secs(120)).await;
        shutdown.cancel();
        task.await.unwrap();
        let cursor = backend.cursor_in("a", "INBOX").as_ref().map(last_uid);
        assert!(matches!(cursor, Some(4 | 6)), "{cursor:?}");
        assert!(
            backend.state().passes.is_empty(),
            "{:?}",
            backend.state().passes
        );
        assert!(worker.progress().is_none());
        assert!(!worker.schedule().busy());
        tokio::time::sleep(minutes(10)).await;
        assert_eq!(
            backend.cursor_in("a", "INBOX").as_ref().map(last_uid),
            cursor
        );
        assert!(backend.state().passes.is_empty());
    }

    /// Mailbox `a` at `imap.old.example` (UIDs 11 to 14) and `imap.new.example`
    /// (UIDs 1 to 3, the same UIDVALIDITY: an old cursor would skip them
    /// all), read in batches of two with each body fetch taking 20 s.
    fn replaceable() -> (MailboxWorker, MemoryBackend) {
        let backend = MemoryBackend::with_config(mailbox("a", "imap.old.example", 993));
        let limits = Limits {
            batch_max_messages: 2,
            batch_timeout_base: minutes(5),
            ..Limits::DEFAULT
        };
        let worker = MailboxWorker::with_fake_servers(
            limits,
            Arc::new(|cfg: &ImapConfig| {
                let uids: &[u32] = if cfg.host == "imap.old.example" {
                    &[11, 12, 13, 14]
                } else {
                    &[1, 2, 3]
                };
                let messages = uids.iter().map(|&uid| FakeMessage::report(uid)).collect();
                FakeServer::new(1, messages).delay_body_fetch(Duration::from_secs(20))
            }),
        );
        (worker, backend)
    }

    /// At 10 s into `a`'s first pass, the host replaces its mailbox as the
    /// server's save does (another server, the next epoch, its stored state
    /// forgotten) and then runs `after` on the worker.
    fn replace_at_10s(
        worker: &MailboxWorker,
        backend: &MemoryBackend,
        after: impl FnOnce(&MailboxWorker) + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let (w, b) = (worker.clone(), backend.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            b.replace("a", |cfg| cfg.host = "imap.new.example".into());
            after(&w);
        })
    }

    /// The new mailbox's pass read it from its first message, and nothing
    /// the old mailbox's pass wrote after the replacement survived.
    fn read_from_the_start(backend: &MemoryBackend) {
        let state = backend.state();
        assert_eq!(state.passes.len(), 1, "{:?}", state.passes);
        let pass = &state.passes[0];
        assert_eq!(pass.epoch, Some(1));
        assert_eq!(pass.summary.start_position, None);
        assert_eq!(pass.summary.reports_ingested, 3);
        assert_eq!(
            state.cursors.get(&("a".into(), "INBOX".into())),
            Some(&Cursor {
                generation: "1".into(),
                position: "3".into()
            })
        );
    }

    /// A save replaces the mailbox while its pass runs: the pass is
    /// cancelled after its batch, the cursor that batch saves under the old
    /// epoch is dropped, and the next pass, due at once, reads the new
    /// mailbox from its first message. The host's save is all it takes: the
    /// worker forgets nothing itself.
    #[tokio::test(start_paused = true)]
    async fn a_mailbox_replaced_mid_pass_is_read_from_the_start() {
        let (worker, backend) = replaceable();
        let saved = replace_at_10s(&worker, &backend, |w| w.settings_changed("a"));
        run_until(&worker, &backend, |b| !b.state().passes.is_empty()).await;
        saved.await.unwrap();
        read_from_the_start(&backend);
    }

    /// Even a pass that is not stopped (the worker was not told) writes
    /// nothing after the replacement: its cursor saves and its record are
    /// dropped, so the next pass still reads the new mailbox from the start.
    #[tokio::test(start_paused = true)]
    async fn a_pass_not_stopped_by_the_replacement_writes_nothing() {
        let (worker, backend) = replaceable();
        let saved = replace_at_10s(&worker, &backend, |_| {});
        let w = worker.clone();
        let asked = tokio::spawn(async move {
            // After the old pass's four messages (80 s) ended.
            tokio::time::sleep(Duration::from_secs(120)).await;
            w.poll_now("a");
        });
        run_until(&worker, &backend, |b| !b.state().passes.is_empty()).await;
        saved.await.unwrap();
        asked.await.unwrap();
        read_from_the_start(&backend);
        assert!(backend.state().reports.contains("fake-14"));
    }

    /// A re-import asked around the replacement is not lost and does not
    /// bring the old mailbox's cursor back: the next pass reads the new
    /// mailbox from its first message, the re-import done.
    #[tokio::test(start_paused = true)]
    async fn a_reimport_asked_with_the_replacement_reads_the_new_mailbox() {
        let (worker, backend) = replaceable();
        let saved = replace_at_10s(&worker, &backend, |w| {
            w.request_reimport();
            w.settings_changed("a");
        });
        run_until(&worker, &backend, |b| !b.state().passes.is_empty()).await;
        saved.await.unwrap();
        read_from_the_start(&backend);
        assert!(!worker.reimport_requested());
    }

    /// The mailbox removed while its pass runs: the pass stops after its
    /// batch and writes nothing, and it is not read again.
    #[tokio::test(start_paused = true)]
    async fn a_mailbox_removed_mid_pass_writes_nothing() {
        let (worker, backend) = replaceable();
        let (w, b) = (worker.clone(), backend.clone());
        let removed = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            b.remove("a");
            w.settings_changed("a");
        });
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        tokio::time::sleep(minutes(10)).await;
        shutdown.cancel();
        task.await.unwrap();
        removed.await.unwrap();
        let state = backend.state();
        assert!(state.passes.is_empty(), "{:?}", state.passes);
        assert!(state.cursors.is_empty());
        assert!(worker.schedule().slots.is_empty());
    }

    #[tokio::test]
    async fn a_settings_change_cancels_only_that_mailboxs_pass() {
        let worker = MailboxWorker::new();
        let token = CancellationToken::new();
        worker.set_current(Some(("a".into(), token.clone())));
        worker.settings_changed("b");
        assert!(!token.is_cancelled());
        worker.settings_changed("a");
        assert!(token.is_cancelled());
        assert_eq!(worker.settings_generation(), 2);

        // Polling one now cancels nothing and makes only it due.
        let backend = MemoryBackend::with_mailboxes(vec![
            (mailbox("a", "imap.a.example", 993), minutes(5)),
            (mailbox("b", "imap.b.example", 993), minutes(5)),
        ]);
        worker.refresh(&backend).await.unwrap();
        let now = Instant::now();
        for id in ["a", "b"] {
            worker.schedule().finish(id, Next::Interval, now);
        }
        let token = CancellationToken::new();
        worker.set_current(Some(("a".into(), token.clone())));
        worker.poll_now("b");
        assert!(!token.is_cancelled());
        assert_eq!(worker.schedule().next(now), Some("b"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_settings_change_reruns_its_mailbox_and_leaves_the_other_running() {
        // `a` imports slowly; `b` is changed while `a` runs.
        let (worker, backend) = two_mailboxes(&[1, 2, 3, 4], &[1], Duration::from_secs(20));
        let w = worker.clone();
        let changed = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            w.settings_changed("b");
        });
        run_until(&worker, &backend, |b| b.state().passes.len() >= 2).await;
        changed.await.unwrap();
        let state = backend.state();
        // `a` was not interrupted: all four reports in its one pass.
        assert_eq!(state.passes[0].mailbox, "a");
        assert_eq!(state.passes[0].summary.reports_ingested, 4);
        assert!(!state.passes[0].summary.interrupted);
        assert_eq!(state.passes[1].mailbox, "b");
    }

    #[tokio::test(start_paused = true)]
    async fn mailboxes_take_turns_round_robin() {
        let (worker, backend) = two_mailboxes(&[1, 2], &[1], Duration::ZERO);
        assert!(!worker.round_complete());
        run_until(&worker, &backend, |b| b.state().passes.len() >= 4).await;
        assert_eq!(&backend.pass_order()[..4], ["a", "b", "a", "b"]);
        assert!(worker.round_complete());
        assert_eq!(
            backend.cursor_in("a", "INBOX").as_ref().map(last_uid),
            Some(1002)
        );
        assert_eq!(
            backend.cursor_in("b", "INBOX").as_ref().map(last_uid),
            Some(2001)
        );
    }

    /// A long first import of `a` gives way to `b` after two minutes, at a
    /// batch boundary, and then resumes from its cursor.
    #[tokio::test(start_paused = true)]
    async fn an_import_yields_to_a_due_mailbox_after_two_minutes() {
        let a: &'static [u32] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let (worker, backend) = two_mailboxes(a, &[1], Duration::from_secs(50));
        run_until(&worker, &backend, |b| b.state().passes.len() >= 3).await;
        let state = backend.state();
        let order: Vec<_> = state.passes.iter().map(|p| p.mailbox.as_str()).collect();
        assert_eq!(&order[..3], ["a", "b", "a"]);
        // Batches of two take 50 s each: the check after the third batch
        // (150 s) is the first past 120 s.
        let first = &state.passes[0].summary;
        assert!(first.yielded && !first.interrupted, "{first}");
        assert_eq!(first.reports_ingested, 6);
        assert_eq!(first.end_position, Some(1006));
        assert!(state.passes[0].checked_at.is_some());
        // `b` went next, then `a` again at once, from its cursor.
        assert_eq!(state.passes[1].summary.reports_ingested, 1);
        let resumed = &state.passes[2].summary;
        assert_eq!(resumed.start_position, Some(1006));
        assert_eq!(resumed.reports_ingested, 4);
        assert!(!resumed.yielded, "{resumed}");
    }

    /// Without another mailbox due, a long import never yields.
    #[tokio::test(start_paused = true)]
    async fn an_import_alone_does_not_yield() {
        let backend = MemoryBackend::with_config(mailbox("a", "imap.a.example", 993));
        let limits = Limits {
            batch_max_messages: 2,
            batch_timeout_base: minutes(5),
            ..Limits::DEFAULT
        };
        let worker = MailboxWorker::with_fake_servers(
            limits,
            Arc::new(|_: &ImapConfig| {
                let messages = (1..=10).map(FakeMessage::report).collect();
                FakeServer::new(1, messages).delay_body_fetch(Duration::from_secs(50))
            }),
        );
        run_until(&worker, &backend, |b| !b.state().passes.is_empty()).await;
        let summary = backend.state().passes[0].summary;
        assert!(!summary.yielded, "{summary}");
        assert_eq!(summary.reports_ingested, 10);
    }

    #[test]
    fn the_round_is_complete_once_every_listed_mailbox_had_a_pass() {
        let now = Instant::now();
        let listing = |ids: &[&str]| -> Vec<MailboxRef> {
            ids.iter()
                .map(|id| MailboxRef {
                    id: (*id).into(),
                    poll: minutes(5),
                })
                .collect()
        };
        let mut schedule = Schedule::default();
        assert!(!schedule.round_complete);
        schedule.update(&listing(&["a", "b"]));
        assert!(!schedule.round_complete);
        assert_eq!(schedule.pick(now).as_deref(), Some("a"));
        // `a` runs: `b` is due, `a` is not.
        assert!(schedule.others_due("a", now));
        assert_eq!(schedule.next(now), Some("b"));
        schedule.finish("a", Next::Interval, now);
        assert!(!schedule.round_complete);
        assert_eq!(schedule.pick(now).as_deref(), Some("b"));
        schedule.finish("b", Next::Recheck, now);
        assert!(schedule.round_complete);
        // Nothing due until `b`'s recheck, then `a` after its interval.
        assert_eq!(schedule.next(now), None);
        assert_eq!(schedule.next_due(now), Some(now + UNCONFIGURED_RECHECK));
        // A mailbox added later is due at once but does not undo the round.
        schedule.update(&listing(&["a", "b", "c"]));
        assert!(schedule.round_complete);
        assert_eq!(schedule.next(now), Some("c"));

        // No mailbox at all: complete once the list was read.
        let mut empty = Schedule::default();
        empty.update(&[]);
        assert!(empty.round_complete);
    }

    #[test]
    fn a_yielded_mailbox_goes_to_the_back_of_the_queue() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        let refs: Vec<MailboxRef> = ["a", "b", "c"]
            .iter()
            .map(|id| MailboxRef {
                id: (*id).into(),
                poll: minutes(5),
            })
            .collect();
        schedule.update(&refs);
        assert_eq!(schedule.pick(now).as_deref(), Some("a"));
        // `a` yields: due again at once, but after `b` and `c`.
        schedule.finish("a", Next::Resume, now);
        assert_eq!(schedule.pick(now).as_deref(), Some("b"));
        schedule.finish("b", Next::Interval, now);
        assert_eq!(schedule.pick(now).as_deref(), Some("c"));
        schedule.finish("c", Next::Interval, now);
        assert_eq!(schedule.pick(now).as_deref(), Some("a"));
    }

    #[tokio::test]
    async fn the_loop_stops_on_shutdown_and_records_its_passes() {
        let backend = unreachable();
        let worker = MailboxWorker::new();
        assert!(!worker.round_complete());
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        // The first pass fails to connect and is recorded; then the loop sleeps.
        let deadline = Instant::now() + Duration::from_secs(10);
        while backend.state().passes.is_empty() {
            assert!(Instant::now() < deadline, "no pass recorded");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("loop stopped")
            .unwrap();
        assert!(worker.progress().is_none());
        assert_eq!(backend.state().passes.len(), 1);
        assert!(worker.round_complete());
    }

    /// An import that gives way, is stopped, or fails after progress stays
    /// unfinished between its passes, until a pass of that mailbox ends
    /// otherwise; a waiting re-import and a running pass are unfinished too.
    #[tokio::test]
    async fn an_import_is_unfinished_between_its_passes() {
        let backend = MemoryBackend::with_mailboxes(vec![
            (mailbox("a", "imap.a.example", 993), minutes(5)),
            (mailbox("b", "imap.b.example", 993), minutes(5)),
        ]);
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        assert!(!worker.import_unfinished());
        let now = Instant::now();
        for next in [Next::Resume, Next::Continue] {
            worker.schedule().claim("a");
            assert!(worker.import_unfinished(), "running");
            worker.schedule().finish("a", next, now);
            assert!(worker.import_unfinished(), "{next:?}");
            // Another mailbox's pass ending changes nothing.
            worker.schedule().claim("b");
            worker.schedule().finish("b", Next::Interval, now);
            assert!(worker.import_unfinished(), "{next:?}");
            worker.schedule().claim("a");
            worker.schedule().finish("a", Next::Interval, now);
            assert!(!worker.import_unfinished());
        }
        worker.schedule().claim("a");
        worker.schedule().finish("a", Next::Failed, now);
        assert!(!worker.import_unfinished(), "a failure without progress");

        worker.request_reimport();
        worker.refresh(&backend).await.unwrap();
        assert!(worker.import_unfinished(), "a re-import waits");
        worker.set_current(Some(("b".into(), CancellationToken::new())));
        let reimport = ["a", "b"].map(|id| {
            worker
                .schedule()
                .slot_mut(id)
                .is_some_and(|s| std::mem::take(&mut s.reimport))
        });
        assert_eq!(reimport, [true, true]);
        assert!(worker.import_unfinished(), "a pass runs");
        worker.set_current(None);
        assert!(!worker.import_unfinished());
    }

    /// The loop's own long import: once `a` gave way to `b`, no pass may
    /// run, and still the import is unfinished; once both read everything
    /// it is finished.
    #[tokio::test(start_paused = true)]
    async fn a_yielding_import_is_unfinished_until_it_read_everything() {
        let a: &'static [u32] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let (worker, backend) = two_mailboxes(a, &[1], Duration::from_secs(50));
        run_until(&worker, &backend, |b| !b.state().passes.is_empty()).await;
        assert!(backend.state().passes[0].summary.yielded);
        assert!(worker.progress().is_none());
        assert!(worker.import_unfinished());
        let read_all = |b: &MemoryBackend| {
            b.cursor_in("a", "INBOX").as_ref().map(last_uid) == Some(1010)
                && b.cursor_in("b", "INBOX").as_ref().map(last_uid) == Some(2001)
        };
        run_until(&worker, &backend, read_all).await;
        assert!(!worker.import_unfinished());
    }

    #[test]
    fn the_back_off_grows_to_the_longer_of_an_hour_and_the_interval() {
        let five = minutes(5);
        let waits: Vec<u64> = (1..=8).map(|n| retry_after(n, five).as_secs()).collect();
        assert_eq!(waits, [30, 60, 120, 300, 900, 3600, 3600, 3600]);
        let two_hours = minutes(120);
        assert_eq!(retry_after(6, two_hours), minutes(60));
        assert_eq!(retry_after(7, two_hours), two_hours);
        assert_eq!(retry_after(u32::MAX, two_hours), two_hours);
    }

    /// Failed passes in a row wait ever longer; a success, the person's own
    /// asks (a settings change, Sync now, a re-import) and a test start over.
    #[test]
    fn failures_back_off_until_a_success_or_a_reset() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        schedule.update(&[MailboxRef {
            id: "a".into(),
            poll: minutes(5),
        }]);
        let fail = |schedule: &mut Schedule| {
            schedule.claim("a");
            schedule.finish("a", Next::Failed, now);
            schedule.next_due(now).map(|at| at - now)
        };
        assert_eq!(fail(&mut schedule), Some(Duration::from_secs(30)));
        assert_eq!(fail(&mut schedule), Some(minutes(1)));
        assert_eq!(fail(&mut schedule), Some(minutes(2)));
        assert_eq!(fail(&mut schedule), Some(minutes(5)));
        assert_eq!(fail(&mut schedule), Some(minutes(15)));
        assert_eq!(fail(&mut schedule), Some(minutes(60)));
        assert_eq!(fail(&mut schedule), Some(minutes(60)));
        // Progress before the failure is no failure in a row.
        schedule.finish("a", Next::Continue, now);
        assert_eq!(schedule.slots[0].failures, 0);
        assert_eq!(fail(&mut schedule), Some(Duration::from_secs(30)));
        assert_eq!(fail(&mut schedule), Some(minutes(1)));
        // A success: the interval again.
        schedule.finish("a", Next::Interval, now);
        assert_eq!(schedule.next_due(now), Some(now + minutes(5)));
        // Unconfigured: its cheap recheck, whatever came before.
        fail(&mut schedule);
        fail(&mut schedule);
        schedule.finish("a", Next::Recheck, now);
        assert_eq!(schedule.next_due(now), Some(now + UNCONFIGURED_RECHECK));
    }

    #[tokio::test]
    async fn the_persons_asks_and_a_test_start_the_back_off_over() {
        let backend = MemoryBackend::with_mailboxes(vec![
            (mailbox("a", "imap.a.example", 993), minutes(5)),
            (mailbox("b", "imap.b.example", 993), minutes(5)),
        ]);
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        let failing = |worker: &MailboxWorker| {
            let now = Instant::now();
            let mut schedule = worker.schedule();
            for _ in 0..4 {
                schedule.claim("a");
                schedule.finish("a", Next::Failed, now);
            }
            assert_eq!(schedule.slots[0].failures, 4);
        };
        failing(&worker);
        worker.settings_changed("a");
        assert_eq!(worker.failures("a"), 0);
        failing(&worker);
        worker.poll_now("a");
        assert_eq!(worker.failures("a"), 0);
        failing(&worker);
        worker.request_reimport();
        worker.refresh(&backend).await.unwrap();
        assert_eq!(worker.failures("a"), 0);
        failing(&worker);
        {
            // Sync now while a pass runs.
            worker.schedule().claim("b");
            assert_eq!(
                worker.start_sync(&backend).await.unwrap(),
                SyncStart::AlreadyRunning
            );
        }
        assert_eq!(worker.failures("a"), 0);

        // A test: the first step after the last pass, due at once here.
        let worker = MailboxWorker::new();
        worker.refresh(&backend).await.unwrap();
        failing(&worker);
        let now = Instant::now();
        assert!(worker.schedule().slots[0].due_at(now) > Some(now + minutes(4)));
        worker.reset_backoff("a");
        assert_eq!(worker.failures("a"), 0);
        assert_eq!(worker.schedule().slots[0].delay, Some(RETRY_BACKOFF[0]));
        assert!(worker.schedule().slots[0].is_due(now + RETRY_BACKOFF[0]));
        // A mailbox not backing off keeps its interval.
        worker.reset_backoff("b");
        assert_eq!(worker.schedule().slots[1].delay, None);
    }

    /// A mailbox whose token step keeps failing is tried after 30 s, 1 min,
    /// 2 min, 5 min, 15 min and then hourly, not every 30 seconds.
    #[tokio::test(start_paused = true)]
    async fn a_failing_prepare_backs_off() {
        let backend = MemoryBackend::with_config(mailbox("a", "imap.a.example", 993));
        backend.state().prepare_down = Some("the token was refused (simulated)".into());
        let worker = MailboxWorker::with_fake_servers(
            Limits::DEFAULT,
            Arc::new(|_: &ImapConfig| FakeServer::new(1, vec![])),
        );
        let started = Instant::now();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        tokio::time::sleep(minutes(150)).await;
        shutdown.cancel();
        task.await.unwrap();
        {
            let state = backend.state();
            // At 0, 0:30, 1:30, 3:30, 8:30, 23:30, 1:23:30 and 2:23:30.
            assert_eq!(state.passes.len(), 8, "{:?}", started.elapsed());
            assert!(state.passes.iter().all(|p| p.error.is_some()));
        }
        assert_eq!(worker.failures("a"), 8);

        // Once it works again, the next pass resets the count.
        backend.state().prepare_down = None;
        let shutdown = CancellationToken::new();
        worker.poll_now("a");
        let task = tokio::spawn(worker.clone().run(backend.clone(), shutdown.clone()));
        tokio::time::sleep(Duration::from_secs(5)).await;
        shutdown.cancel();
        task.await.unwrap();
        assert_eq!(worker.failures("a"), 0);
        assert!(backend.state().passes.last().unwrap().error.is_none());
    }
}
