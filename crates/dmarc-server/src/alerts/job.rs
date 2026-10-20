//! The alert job: evaluate every monitored domain, mail what changed, and
//! send the digest when it is due.

use super::evaluate::evaluate_all;
use crate::mail::{self, digest::DigestInput, Address, Letterhead, SendError, SmtpConfig};
use crate::view::alerts::AlertItem;
use crate::view::domain::{domain_summaries, Judging};
use crate::AppState;
use chrono::{DateTime, Datelike, Duration as Days, TimeZone, Utc};
use dmarc_assess::readiness_window_start;
use dmarc_imap::ImapWorker;
use dmarc_store::{
    alert_events_in, get_last_digest_at, list_domain_summaries, mark_alert_events_notified,
    set_alerts_held_at, set_last_digest_at, unix_now, unnotified_alert_events, AlertSettings,
    AppSettings, DigestFrequency, Scope,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The longest between two evaluations. The job runs right after each DNS
/// warm pass ends (every 15 minutes, sooner when a new domain wakes it), so
/// each run sees the results that pass stored; this is the fallback when no
/// pass ends (`background::Schedule::AfterRound`).
pub const ALERT_INTERVAL: Duration = Duration::from_mins(15);

/// Events older than this are not mailed any more (they stay in the list):
/// switching alert emails on, or sending coming back after an outage, must
/// not send a backlog of old news.
pub const ALERT_MAIL_MAX_AGE_SECS: i64 = 24 * 60 * 60;

/// The most events one run reads to mail, oldest first; the rest wait for
/// the next run. Bounds the work of a run, not what is mailed in the end.
/// Muted events count against it until they are older than
/// [`ALERT_MAIL_MAX_AGE_SECS`].
pub const ALERT_MAIL_BATCH: i64 = 500;

const DAY_SECS: i64 = 86_400;

/// What a delivery did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Alert emails are off, or nobody is set to receive them.
    Off,
    /// Nothing new to send.
    Nothing,
    /// Sending is off or incomplete, or every recipient failed: the events
    /// wait for the next run.
    Failed(SendError),
    /// One email to each of `recipients` (those that took it) with `events` events.
    Sent { events: usize, recipients: usize },
}

/// The recipients that are addresses mail can go to.
fn recipients(alerts: &AlertSettings) -> Vec<Address> {
    alerts
        .recipients
        .iter()
        .filter_map(|r| mail::parse_address(r))
        .collect()
}

/// Whether `item` goes into the emails: its kind is not muted in the alert
/// settings. The alert list shows every event either way. A muted event is
/// never marked as mailed, so it ages out of the queue like any other.
fn mailed(alerts: &AlertSettings, item: &AlertItem) -> bool {
    !alerts.muted_kinds.iter().any(|id| id == item.catalogue_id)
}

/// Send `content` to each recipient; the number that took it, or the last
/// error when none did.
async fn send_to_all(
    state: &AppState,
    config: &SmtpConfig,
    to: &[Address],
    content: &mail::Content,
) -> Result<usize, SendError> {
    let mut sent = 0;
    let mut last_error = None;
    for address in to {
        match state
            .mail
            .send(config, &content.message(address.clone()))
            .await
        {
            Ok(()) => sent += 1,
            Err(e) => {
                tracing::warn!(to = %address, reason = e.reason(), error = %e, "alert email failed");
                last_error = Some(e);
            }
        }
    }
    match (sent, last_error) {
        (0, Some(e)) => Err(e),
        (n, _) => Ok(n),
    }
}

/// Mail every event not mailed yet (and at most
/// [`ALERT_MAIL_MAX_AGE_SECS`] old) in one email per recipient, and mark
/// them notified once at least one recipient took it.
pub async fn deliver(state: &AppState, settings: &AppSettings, now: i64) -> Delivery {
    let to = recipients(&settings.alerts);
    if !settings.alerts.enabled || to.is_empty() {
        return Delivery::Off;
    }
    let since = now - ALERT_MAIL_MAX_AGE_SECS;
    let pending = match unnotified_alert_events(&state.pool, since, ALERT_MAIL_BATCH).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "alerts: could not read the events to send");
            return Delivery::Nothing;
        }
    };
    let items: Vec<AlertItem> = pending
        .into_iter()
        .map(AlertItem::from)
        .filter(|item| mailed(&settings.alerts, item))
        .collect();
    if items.is_empty() {
        return Delivery::Nothing;
    }
    let config = match SmtpConfig::from_settings(settings) {
        Ok(config) => config,
        Err(e) => return Delivery::Failed(e),
    };
    let content = mail::alerts::render(
        &items,
        settings.alerts.base_url(),
        &Letterhead::load(&state.pool, settings).await,
    );
    match send_to_all(state, &config, &to, &content).await {
        Err(e) => Delivery::Failed(e),
        Ok(recipients) => {
            let ids: Vec<i64> = items.iter().map(|i| i.row.id).collect();
            if let Err(e) = mark_alert_events_notified(&state.pool, &ids, now).await {
                tracing::warn!(error = %e, "alerts: sent, but could not mark the events sent");
            }
            Delivery::Sent {
                events: items.len(),
                recipients,
            }
        }
    }
}

/// The latest digest time at or before `now`: today's (or the configured
/// weekday's) `hour`:00 UTC. `None` while the digest is off.
pub fn digest_slot(alerts: &AlertSettings, now: i64) -> Option<i64> {
    let now_utc: DateTime<Utc> = DateTime::from_timestamp(now, 0)?;
    let date = now_utc.date_naive();
    let at_hour = |d: chrono::NaiveDate| -> Option<i64> {
        let time = d.and_hms_opt(u32::from(alerts.digest_hour), 0, 0)?;
        Some(Utc.from_utc_datetime(&time).timestamp())
    };
    // The slot on `day`, or one `period` earlier when that is still ahead.
    let latest = |day, period: i64| at_hour(day).map(|t| if t <= now { t } else { t - period });
    match alerts.digest {
        DigestFrequency::Off => None,
        DigestFrequency::Daily => latest(date, DAY_SECS),
        DigestFrequency::Weekly => {
            let back = (date.weekday().num_days_from_monday() + 7
                - alerts.digest_weekday.days_from_monday())
                % 7;
            latest(date - Days::days(i64::from(back)), 7 * DAY_SECS)
        }
    }
}

fn period_secs(digest: DigestFrequency) -> i64 {
    match digest {
        DigestFrequency::Weekly => 7 * DAY_SECS,
        _ => DAY_SECS,
    }
}

/// Send the digest when its time has come and it has not gone out since:
/// what changed since the last digest (at most one period back) and every
/// domain's open issues. The first time the job sees the digest switched on
/// it only remembers the time, so switching it on never sends one at once.
/// The digest is an alert email: with alert emails off it never goes out.
/// Returns whether a digest was sent.
pub async fn send_digest_if_due(state: &AppState, settings: &AppSettings, now: i64) -> bool {
    let alerts = &settings.alerts;
    if !alerts.enabled {
        return false;
    }
    let to = recipients(alerts);
    let Some(slot) = digest_slot(alerts, now) else {
        return false;
    };
    if to.is_empty() {
        return false;
    }
    let last = match get_last_digest_at(&state.pool).await {
        Ok(Some(last)) => last,
        Ok(None) => {
            if let Err(e) = set_last_digest_at(&state.pool, now).await {
                tracing::warn!(error = %e, "digest: could not record the start");
            }
            return false;
        }
        Err(e) => {
            tracing::warn!(error = %e, "digest: could not read when it last went out");
            return false;
        }
    };
    if last >= slot {
        return false;
    }
    let since = last.max(slot - period_secs(alerts.digest));
    let config = match SmtpConfig::from_settings(settings) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(reason = e.reason(), error = %e, "digest: sending is not ready");
            return false;
        }
    };
    let window = readiness_window_start(now);
    let (rows, events) = match (
        list_domain_summaries(&state.pool, Scope::ALL.window(Some(window), None)).await,
        alert_events_in(&state.pool, Scope::ALL.window(Some(since), None)).await,
    ) {
        (Ok(rows), Ok(events)) => (rows, events),
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!(error = %e, "digest: could not read the domains or events");
            return false;
        }
    };
    let domains = domain_summaries(rows, Judging::new(now, &settings.monitoring));
    let events: Vec<AlertItem> = events
        .into_iter()
        .map(AlertItem::from)
        .filter(|item| mailed(alerts, item))
        .collect();
    let content = mail::digest::render(&DigestInput {
        frequency: alerts.digest,
        since,
        now,
        domains: &domains,
        events: &events,
        base_url: alerts.base_url(),
        letterhead: &Letterhead::load(&state.pool, settings).await,
    });
    match send_to_all(state, &config, &to, &content).await {
        Ok(recipients) => {
            if let Err(e) = set_last_digest_at(&state.pool, now).await {
                tracing::warn!(error = %e, "digest: sent, but could not record it");
            }
            tracing::info!(recipients, events = events.len(), "digest sent");
            true
        }
        Err(e) => {
            tracing::warn!(reason = e.reason(), error = %e, "digest: not sent; retrying next run");
            false
        }
    }
}

/// One run at `now`: evaluate and store, then mail what changed and the
/// digest when due.
pub async fn run_once(state: &AppState, now: i64, token: &CancellationToken) {
    match evaluate_all(&state.pool, now, token).await {
        Ok(summary) => {
            if !summary.skipped.is_empty() {
                tracing::warn!(
                    skipped = summary.skipped.len(),
                    evaluated = summary.domains,
                    domains = %summary.skipped.join(", "),
                    "alerts: some domains could not be evaluated and keep their last state; the others were"
                );
            }
            if summary.rebaselined > 0 {
                tracing::info!(
                    domains = summary.rebaselined,
                    "alerts: first evaluation after a mailbox import; its domains' reports are a new baseline"
                );
            }
            if summary.events > 0 {
                tracing::info!(
                    domains = summary.domains,
                    events = summary.events,
                    "alerts: changes found"
                );
            } else {
                tracing::debug!(domains = summary.domains, "alerts: nothing changed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "alerts: evaluation failed"),
    }
    if token.is_cancelled() {
        return;
    }
    let settings = match AppSettings::load(&state.pool).await {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!(error = %e, "alerts: could not read the settings; nothing sent");
            return;
        }
    };
    match deliver(state, &settings, now).await {
        Delivery::Sent { events, recipients } => {
            tracing::info!(events, recipients, "alert email sent");
        }
        Delivery::Failed(e) => {
            tracing::warn!(reason = e.reason(), error = %e, "alert email not sent; retrying next run");
        }
        Delivery::Off | Delivery::Nothing => {}
    }
    send_digest_if_due(state, &settings, now).await;
}

/// How long a run waits for a running mailbox pass to end before it holds
/// the evaluation back: a routine poll ends within it, an import does not.
pub const SHORT_PASS_WAIT: Duration = Duration::from_secs(60);

/// The mailbox import as the alert job sees it: the IMAP worker's, the same
/// source as the health endpoint's progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportStatus {
    /// When the running pass started; `None` while no pass runs.
    pub running_since: Option<i64>,
    /// Passes ended since startup.
    pub passes_done: u64,
}

impl ImportStatus {
    pub fn of(imap: &ImapWorker) -> Self {
        Self {
            running_since: imap.progress().map(|p| p.started_at),
            passes_done: imap.passes_done(),
        }
    }
}

impl ImportStatus {
    /// Whether the evaluation waits: a pass still runs (after
    /// [`SHORT_PASS_WAIT`]), or none has ended since startup.
    ///
    /// Reports land in any order during an import, so a domain's window
    /// figures in between are no state it was ever in: evaluating them raised
    /// issues that "opened" and "resolved" as older and newer reports
    /// arrived. A held run records when it held (`alerts_held_at`), and the
    /// next evaluation takes the reports of every domain that got some since
    /// its last evaluation as a new baseline
    /// ([`evaluate_all`](super::evaluate::evaluate_all)). Both are stored, so
    /// a restart in the middle of an import or a failed evaluation does not
    /// bring the import's noise back.
    pub fn holds(&self) -> bool {
        self.running_since.is_some() || self.passes_done == 0
    }
}

/// Wait up to [`SHORT_PASS_WAIT`] for a running mailbox pass to end.
async fn wait_for_short_pass(imap: &ImapWorker, token: &CancellationToken) {
    let deadline = tokio::time::Instant::now() + SHORT_PASS_WAIT;
    while imap.progress().is_some() && tokio::time::Instant::now() < deadline {
        tokio::select! {
            () = token.cancelled() => return,
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

/// The scheduled run: [`run_once`] at the current time, unless the mailbox
/// import holds it back ([`ImportStatus::holds`]; then nothing runs, the
/// emails included: they go out with the next run).
pub async fn run(state: AppState, token: CancellationToken) {
    wait_for_short_pass(&state.imap, &token).await;
    if token.is_cancelled() {
        return;
    }
    let now = unix_now();
    if ImportStatus::of(&state.imap).holds() {
        tracing::info!("alerts: the mailbox is importing; evaluation held back");
        if let Err(e) = set_alerts_held_at(&state.pool, now).await {
            tracing::warn!(error = %e, "alerts: held back, but could not record it");
        }
        return;
    }
    run_once(&state, now, &token).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_store::Weekday;

    const DAY: i64 = 86_400;
    // 2026-10-14 (a Wednesday) 07:00 UTC.
    const WED_7: i64 = 1_791_961_200;

    fn alerts(digest: DigestFrequency, hour: u8, weekday: Weekday) -> AlertSettings {
        AlertSettings {
            digest,
            digest_hour: hour,
            digest_weekday: weekday,
            ..AlertSettings::default()
        }
    }

    fn import(running_since: Option<i64>, passes_done: u64) -> ImportStatus {
        ImportStatus {
            running_since,
            passes_done,
        }
    }

    /// No evaluation before the first pass since startup ends or while one
    /// runs.
    #[test]
    fn the_gate_holds_during_an_import() {
        // Startup: no pass has ended yet.
        assert!(import(None, 0).holds());
        // The first pass is an import.
        assert!(import(Some(1_100), 0).holds());
        // It ended.
        assert!(!import(None, 1).holds());
        // A later long pass (a re-import).
        assert!(import(Some(5_000), 1).holds());
    }

    #[test]
    fn daily_slots_are_today_or_yesterday_at_the_hour() {
        let daily = alerts(DigestFrequency::Daily, 7, Weekday::Monday);
        assert_eq!(digest_slot(&daily, WED_7), Some(WED_7));
        assert_eq!(digest_slot(&daily, WED_7 + 3600), Some(WED_7));
        assert_eq!(digest_slot(&daily, WED_7 - 1), Some(WED_7 - DAY));
        assert_eq!(
            digest_slot(&alerts(DigestFrequency::Off, 7, Weekday::Monday), WED_7),
            None
        );
    }

    #[test]
    fn weekly_slots_fall_on_the_weekday() {
        let monday = alerts(DigestFrequency::Weekly, 7, Weekday::Monday);
        assert_eq!(digest_slot(&monday, WED_7), Some(WED_7 - 2 * DAY));
        let wednesday = alerts(DigestFrequency::Weekly, 7, Weekday::Wednesday);
        assert_eq!(digest_slot(&wednesday, WED_7), Some(WED_7));
        assert_eq!(digest_slot(&wednesday, WED_7 - 1), Some(WED_7 - 7 * DAY));
        let thursday = alerts(DigestFrequency::Weekly, 18, Weekday::Thursday);
        assert_eq!(
            digest_slot(&thursday, WED_7),
            Some(WED_7 - 6 * DAY + 11 * 3600)
        );
    }
}
