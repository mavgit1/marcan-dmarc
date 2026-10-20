//! The background work and its supervisor: the IMAP polling loop, the DNS
//! snapshot warm loop, the reverse-DNS identity warm loop, the alert job, the
//! monthly client reports and the daily report retention clean-up. One
//! cancellation token stops every loop between units of work (a snapshot, a
//! mailbox batch, an evaluation run); shutdown waits a grace period for
//! them, then aborts what is left.
//!
//! Every scheduled loop is [`every`]: run now, then every interval, until
//! cancelled.

use crate::alerts::job::{self as alert_job, ALERT_INTERVAL};
use crate::client_reports::job::{self as client_report_job, CLIENT_REPORT_INTERVAL};
use crate::snapshots;
use crate::AppState;
use dmarc_store::{
    delete_alert_events_older_than, delete_reports_older_than, load_settings, retention_cutoff,
    unix_now, RetentionSettings, StoreError, REPORT_RETENTION_MONTHS_RANGE,
};
use futures::FutureExt;
use sqlx::SqlitePool;
use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Notify};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// How often the warm loops re-examine their candidates.
///
/// Shorter than the snapshots' freshness windows on purpose: the interval
/// bounds how long a newly ingested domain or sender stays unchecked, while
/// the per-item freshness check keeps the actual DNS traffic down.
pub const WARM_INTERVAL: Duration = Duration::from_mins(15);

/// The soonest a woken DNS warm pass follows the previous one (see
/// [`Schedule::OrWhenWoken`]): new domains are checked within about this long.
pub const WARM_WAKE_MIN_GAP: Duration = Duration::from_secs(20);

/// How often reports older than the retention setting are deleted.
pub const RETENTION_INTERVAL: Duration = Duration::from_hours(24);

/// The longest the alert job waits at startup for the first DNS warm pass,
/// so its first evaluation sees fresh DNS results. A pass that takes longer
/// (hundreds of stale domains) does not hold alerts back further.
pub const FIRST_WARM_WAIT: Duration = Duration::from_mins(10);

/// The running background tasks.
pub struct Background {
    token: CancellationToken,
    tasks: JoinSet<()>,
}

impl Background {
    /// Start every background task for `state`.
    pub fn start(state: &AppState) -> Self {
        let token = CancellationToken::new();
        let mut tasks = JoinSet::new();

        // Always running: the worker re-reads the IMAP settings on every pass,
        // so a mailbox configured in the UI starts polling without a restart.
        tasks.spawn(
            state
                .imap
                .clone()
                .run(state.imap_backend(), token.child_token()),
        );

        // The DNS warm loop says when each pass starts and ends; the alert job
        // runs after each one, so every evaluation sees the DNS results of
        // its round. A new domain (added, or found in a report) wakes it
        // early, so its first result does not wait for the interval.
        let (rounds_tx, rounds_rx) = watch::channel(WarmRounds::default());
        let (pool, dns, child) = (state.pool.clone(), state.dns.clone(), token.child_token());
        tasks.spawn(every(
            "dns",
            Schedule::OrWhenWoken {
                interval: WARM_INTERVAL,
                min_gap: WARM_WAKE_MIN_GAP,
                wake: state.dns.wake_signal(),
            },
            child,
            move |token| {
                let (pool, dns, rounds) = (pool.clone(), dns.clone(), rounds_tx.clone());
                async move {
                    let _round = WarmRound::start(&rounds);
                    dns.warm_once(&pool, &token).await;
                }
            },
        ));

        let (pool, dns, child) = (state.pool.clone(), state.dns.clone(), token.child_token());
        tasks.spawn(every("identity", WARM_INTERVAL, child, move |token| {
            let (pool, dns) = (pool.clone(), dns.clone());
            async move { snapshots::identity::warm_once(&pool, &dns, &token).await }
        }));

        // The alert job also holds back while the mailbox imports (it stores
        // the hold, which the next evaluation reads).
        let (alerts_state, child) = (state.clone(), token.child_token());
        tasks.spawn(async move {
            if !after_first_warm(rounds_rx.clone(), &child).await {
                return;
            }
            let schedule = Schedule::AfterRound {
                rounds: rounds_rx,
                fallback: ALERT_INTERVAL,
            };
            every("alerts", schedule, child, move |token| {
                alert_job::run(alerts_state.clone(), token)
            })
            .await;
        });

        let (reports_state, child) = (state.clone(), token.child_token());
        tasks.spawn(every(
            "client-reports",
            CLIENT_REPORT_INTERVAL,
            child,
            move |token| client_report_job::run(reports_state.clone(), token),
        ));

        let (pool, child) = (state.pool.clone(), token.child_token());
        tasks.spawn(every("retention", RETENTION_INTERVAL, child, move |_| {
            let pool = pool.clone();
            async move { apply_retention(&pool).await }
        }));

        Self { token, tasks }
    }

    /// Stop: cancel the loops, give them `grace` to finish the unit of work
    /// in hand (a snapshot being written, a mailbox batch), then abort
    /// whatever is left (every write is a transaction).
    pub async fn shutdown(mut self, grace: Duration) {
        self.token.cancel();
        let deadline = tokio::time::Instant::now() + grace;
        while let Ok(Some(_)) = tokio::time::timeout_at(deadline, self.tasks.join_next()).await {}
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
    }
}

/// The DNS warm passes as the alert job follows them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WarmRounds {
    /// Passes ended since startup.
    pub completed: u64,
    /// A pass runs now.
    pub running: bool,
}

/// One warm pass on the [`WarmRounds`] channel: started when made, counted
/// as ended when dropped, also when the pass panicked.
pub(crate) struct WarmRound<'a>(&'a watch::Sender<WarmRounds>);

impl<'a> WarmRound<'a> {
    pub(crate) fn start(rounds: &'a watch::Sender<WarmRounds>) -> Self {
        rounds.send_modify(|r| r.running = true);
        Self(rounds)
    }
}

impl Drop for WarmRound<'_> {
    fn drop(&mut self) {
        self.0.send_modify(|r| {
            r.running = false;
            r.completed += 1;
        });
    }
}

/// Wait until `rounds` says the first DNS warm pass is done, at most
/// [`FIRST_WARM_WAIT`]. `false` when `token` was cancelled meanwhile.
async fn after_first_warm(
    mut rounds: watch::Receiver<WarmRounds>,
    token: &CancellationToken,
) -> bool {
    tokio::select! {
        () = token.cancelled() => false,
        _ = tokio::time::timeout(FIRST_WARM_WAIT, rounds.wait_for(|r| r.completed > 0)) => true,
    }
}

/// When [`every`] runs its work again.
pub(crate) enum Schedule {
    /// `interval` after the last run ended.
    Every(Duration),
    /// `interval` after the last run ended, or earlier once `wake` is
    /// notified, but never sooner than `min_gap` after the last run started
    /// (wake-ups during a long import would otherwise run it back to back).
    OrWhenWoken {
        interval: Duration,
        min_gap: Duration,
        wake: Arc<Notify>,
    },
    /// As soon as a DNS warm pass ends (`rounds`) after the last run
    /// started, so each run sees the results of the round before it; else
    /// `fallback` after the last run ended, waiting for a pass running then
    /// to end first (at most another `fallback`).
    AfterRound {
        rounds: watch::Receiver<WarmRounds>,
        fallback: Duration,
    },
}

impl From<Duration> for Schedule {
    fn from(interval: Duration) -> Self {
        Self::Every(interval)
    }
}

/// Run `work` now and then on `schedule` until `token` is cancelled.
/// `work` receives the token so it can stop between units. The one schedule
/// of every background loop (warm passes, alerts, client reports, retention).
/// A run that panics is logged with the loop's `name` and ends that run
/// only: the next one follows on schedule.
pub(crate) async fn every<F, Fut>(
    name: &'static str,
    schedule: impl Into<Schedule>,
    token: CancellationToken,
    mut work: F,
) where
    F: FnMut(CancellationToken) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut schedule = schedule.into();
    loop {
        let started = tokio::time::Instant::now();
        // The passes that ended before this run started: it saw their results.
        let seen = match &schedule {
            Schedule::AfterRound { rounds, .. } => rounds.borrow().completed,
            _ => 0,
        };
        if let Err(panic) = AssertUnwindSafe(work(token.clone())).catch_unwind().await {
            tracing::error!(
                loop_name = name,
                panic = panic_message(panic.as_ref()),
                "background job panicked; it runs again on its schedule"
            );
        }
        let next = async {
            match &mut schedule {
                Schedule::Every(interval) => tokio::time::sleep(*interval).await,
                Schedule::OrWhenWoken {
                    interval,
                    min_gap,
                    wake,
                } => {
                    tokio::select! {
                        () = tokio::time::sleep(*interval) => {}
                        () = wake.notified() => {
                            tokio::time::sleep_until(started + *min_gap).await;
                        }
                    }
                }
                Schedule::AfterRound { rounds, fallback } => {
                    let waited =
                        tokio::time::timeout(*fallback, rounds.wait_for(|r| r.completed > seen))
                            .await
                            .map(|ended| ended.map(|_| ()));
                    match waited {
                        Ok(Ok(())) => {}
                        // The warm loop is gone: the fallback alone.
                        Ok(Err(_)) => tokio::time::sleep(*fallback).await,
                        // No pass ended in time: not in the middle of one.
                        Err(_) => {
                            let idle = rounds.wait_for(|r| !r.running);
                            let _ = tokio::time::timeout(*fallback, idle).await;
                        }
                    }
                }
            }
        };
        tokio::select! {
            () = token.cancelled() => {
                tracing::debug!(loop_name = name, "background loop stopped");
                return;
            }
            () = next => {}
        }
    }
}

/// What a panic said, when it said it with a string.
fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)")
}

/// Rows retention deletes per write transaction (reports with what cascades
/// from them, or alert events). A year of reports cleared at once would hold
/// the write lock past the IMAP batches' and API writes' busy timeout.
const RETENTION_CHUNK: i64 = 1_000;

/// Pause between two retention chunks, so a writer waiting on the lock gets
/// it before the next chunk takes it again.
const RETENTION_PAUSE: Duration = Duration::from_millis(100);

/// One retention pass: delete every report that ended, and every alert event
/// created, more than
/// [`RetentionSettings::report_retention_months`] ago, [`RETENTION_CHUNK`]
/// rows per transaction. A settings read that fails, or a stored retention
/// that does not read as months, skips the pass with an error rather than
/// falling back to the default, which could delete what a longer stored
/// setting keeps.
pub async fn apply_retention(pool: &SqlitePool) {
    let months = match load_settings::<RetentionSettings>(pool).await {
        Ok(retention) if retention.report_retention_months >= REPORT_RETENTION_MONTHS_RANGE.0 => {
            retention.report_retention_months
        }
        Ok(retention) => {
            tracing::error!(
                months = retention.report_retention_months,
                "retention: the stored retention is below the minimum; skipping"
            );
            return;
        }
        Err(e) => {
            tracing::error!(error = %e, "retention: could not read the stored retention; skipping");
            return;
        }
    };
    let cutoff = retention_cutoff(unix_now(), months);
    let reports = delete_in_chunks(RETENTION_CHUNK, RETENTION_PAUSE, |limit| {
        delete_reports_older_than(pool, cutoff, limit)
    });
    match reports.await {
        Ok(0) => {}
        Ok(deleted) => tracing::info!(deleted, months, "retention: deleted old reports"),
        Err(e) => tracing::warn!(error = %e, "retention: deleting old reports failed"),
    }
    let events = delete_in_chunks(RETENTION_CHUNK, RETENTION_PAUSE, |limit| {
        delete_alert_events_older_than(pool, cutoff, limit)
    });
    match events.await {
        Ok(0) => {}
        Ok(deleted) => tracing::info!(deleted, months, "retention: deleted old alert events"),
        Err(e) => tracing::warn!(error = %e, "retention: deleting old alert events failed"),
    }
}

/// Call `delete` (one transaction deleting at most `chunk` rows) until it
/// deletes fewer than `chunk`, pausing `pause` between calls; the total.
/// A failed chunk stops the pass and keeps what earlier chunks deleted.
async fn delete_in_chunks<F, Fut>(
    chunk: i64,
    pause: Duration,
    mut delete: F,
) -> Result<u64, StoreError>
where
    F: FnMut(i64) -> Fut,
    Fut: Future<Output = Result<u64, StoreError>>,
{
    let full = u64::try_from(chunk).unwrap_or(0);
    let mut total = 0;
    loop {
        let deleted = delete(chunk).await?;
        total += deleted;
        if deleted < full || full == 0 {
            return Ok(total);
        }
        tokio::time::sleep(pause).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Chunks are taken until one comes back short; an exact multiple costs
    /// one empty call more, and every call asks for the same chunk.
    #[tokio::test(start_paused = true)]
    async fn delete_in_chunks_stops_at_the_first_short_chunk() {
        for (rows, calls) in [(0u64, 1usize), (2, 1), (3, 2), (6, 3), (7, 3)] {
            let left = Arc::new(AtomicUsize::new(rows as usize));
            let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
            let total = delete_in_chunks(3, Duration::from_millis(100), |limit| {
                let (left, asked) = (left.clone(), asked.clone());
                async move {
                    asked.lock().unwrap().push(limit);
                    let take = left.load(Ordering::SeqCst).min(limit as usize);
                    left.fetch_sub(take, Ordering::SeqCst);
                    Ok(take as u64)
                }
            })
            .await
            .unwrap();
            assert_eq!(total, rows, "{rows} rows");
            assert_eq!(*asked.lock().unwrap(), vec![3; calls], "{rows} rows");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn every_runs_now_then_per_interval_until_cancelled() {
        let runs = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let counter = runs.clone();
        let task = tokio::spawn(every(
            "test",
            Duration::from_secs(60),
            token.clone(),
            move |_| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        token.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_run_ends_that_run_only() {
        let runs = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let counter = runs.clone();
        let task = tokio::spawn(every(
            "test",
            Duration::from_secs(60),
            token.clone(),
            move |_| {
                let counter = counter.clone();
                async move {
                    assert!(
                        counter.fetch_add(1, Ordering::SeqCst) != 0,
                        "first run fails"
                    );
                }
            },
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(!task.is_finished());
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        token.cancel();
        task.await.expect("the loop ends normally");
        assert_eq!(panic_message(&"text"), "text");
        assert_eq!(panic_message(&String::from("owned")), "owned");
        assert_eq!(panic_message(&42), "(no message)");
    }

    #[tokio::test(start_paused = true)]
    async fn a_wake_runs_early_but_not_before_the_gap() {
        let runs = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let wake = Arc::new(Notify::new());
        let counter = runs.clone();
        let task = tokio::spawn(every(
            "test",
            Schedule::OrWhenWoken {
                interval: Duration::from_mins(15),
                min_gap: Duration::from_secs(20),
                wake: wake.clone(),
            },
            token.clone(),
            move |_| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // Woken: one more run, held until the gap has passed.
        wake.notify_one();
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_secs(15)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        // Not woken again: the interval.
        tokio::time::sleep(Duration::from_secs(800)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 3);
        token.cancel();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn alerts_wait_for_the_first_warm_pass_but_not_forever() {
        let token = CancellationToken::new();
        let (tx, rx) = watch::channel(WarmRounds::default());
        let waiting = tokio::spawn({
            let token = token.clone();
            async move { after_first_warm(rx, &token).await }
        });
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!waiting.is_finished());
        drop(WarmRound::start(&tx));
        assert!(waiting.await.unwrap());

        let (_tx, rx) = watch::channel(WarmRounds::default());
        let start = tokio::time::Instant::now();
        assert!(after_first_warm(rx, &token).await);
        assert!(start.elapsed() >= FIRST_WARM_WAIT);

        let (_tx, rx) = watch::channel(WarmRounds::default());
        token.cancel();
        assert!(!after_first_warm(rx, &token).await);
    }

    /// A job on [`Schedule::AfterRound`] runs after each DNS warm pass ends,
    /// and after the fallback when none does, but not in the middle of one.
    #[tokio::test(start_paused = true)]
    async fn the_alert_schedule_follows_each_warm_pass() {
        const MINUTE: Duration = Duration::from_secs(60);
        let runs = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let (tx, rx) = watch::channel(WarmRounds::default());
        let counter = runs.clone();
        let task = tokio::spawn(every(
            "test",
            Schedule::AfterRound {
                rounds: rx,
                fallback: 15 * MINUTE,
            },
            token.clone(),
            move |_| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                }
            },
        ));
        let count = || runs.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 1);

        // A pass ends after 5 minutes: the next run follows it at once.
        tokio::time::sleep(4 * MINUTE).await;
        let round = WarmRound::start(&tx);
        tokio::time::sleep(MINUTE).await;
        assert_eq!(count(), 1);
        drop(round);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 2);

        // A woken pass a minute later: another run.
        tokio::time::sleep(MINUTE).await;
        drop(WarmRound::start(&tx));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 3);

        // No pass for 15 minutes: the fallback runs it.
        tokio::time::sleep(14 * MINUTE).await;
        assert_eq!(count(), 3);
        tokio::time::sleep(MINUTE).await;
        assert_eq!(count(), 4);

        // A pass runs when the fallback comes: the run waits for it to end,
        // and that pass counts once.
        tokio::time::sleep(14 * MINUTE).await;
        let round = WarmRound::start(&tx);
        tokio::time::sleep(2 * MINUTE).await;
        assert_eq!(count(), 4);
        drop(round);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(count(), 5);
        tokio::time::sleep(10 * MINUTE).await;
        assert_eq!(count(), 5);

        token.cancel();
        task.await.unwrap();
    }
}
