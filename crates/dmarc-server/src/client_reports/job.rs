//! The monthly send: on the set day and hour (UTC), one email per listed
//! organization and recipient, once a month. Each email that goes out is
//! recorded on its own (`client_report_sends`, by recipient and month), so a
//! run that stops part way (a restart, a refusing server) leaves only the
//! rest to the next hourly run, and nobody gets a month's report twice.
//! Held while the mailbox has not been read lately
//! ([`dmarc_assess::client_report_hold`]): a held month stays due, and goes
//! out on the first run that finds the mailbox read again.
//!
//! An email that fails is recorded with its cause (`client_report_failures`)
//! as the mail module classifies it ([`SendError::is_permanent`]): a passing
//! failure (the server unreachable, a `4xx` answer, a timeout, a login to
//! fix) is tried again on each run for [`CLIENT_REPORT_RETRY_DAYS`] from the
//! first, then given up for the month; a refusal for good (a `5xx` answer,
//! an address that is not valid) is not tried again
//! ([`dmarc_assess::client_report_retry`]). Settings → Client reports lists
//! both under the send line until the next report time.

use super::report::{organization_report, report_config};
use crate::mail::Letterhead;
#[cfg(doc)]
use crate::mail::SendError;
use crate::AppState;
use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
#[cfg(doc)]
use dmarc_assess::CLIENT_REPORT_RETRY_DAYS;
use dmarc_assess::{client_report_hold, client_report_retry, ClientReportHold, ClientReportRetry};
use dmarc_store::{
    client_reports_last_sent_at, count_client_report_sends, get_client_reports_held_at,
    imap_pass_status, list_client_report_failures, list_client_report_recipients,
    list_unsent_client_report_recipients, record_client_report_failure, record_client_report_send,
    set_client_reports_held_at, unix_now, AppSettings, ClientReportFailure, ClientReportRecipient,
    ClientReportSettings, FailedClientReport, StoreError,
};
use sqlx::SqlitePool;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// How often the job looks whether the reports are due.
pub const CLIENT_REPORT_INTERVAL: Duration = Duration::from_hours(1);

/// How long after its time a month's reports may still go out when none of
/// them has: a server that was down (or a submission server that refused)
/// at the hour sends them later that day; after that, the month is skipped.
/// Once one has gone out, the rest stay due until the next report time.
pub const LATE_SEND_SECS: i64 = 24 * 60 * 60;

/// From this many days before a send, the organization page says that its
/// report goes out (served as `client_report_notice_days` by `GET /api/meta`).
pub const CLIENT_REPORT_NOTICE_DAYS: i64 = 3;

/// The first day of the month (UTC) `now` is in.
fn month_of(now: i64) -> Option<NaiveDate> {
    let today = DateTime::<Utc>::from_timestamp(now, 0)?.date_naive();
    NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
}

/// The key a report time's sends are recorded under: its month, `YYYY-MM`.
fn month_key(slot: i64) -> Option<String> {
    Some(month_of(slot)?.format("%Y-%m").to_string())
}

/// The report time in `month` (its first day): the set day at the set hour, UTC.
fn slot_in(reports: &ClientReportSettings, month: NaiveDate) -> Option<i64> {
    let day = month.with_day(u32::from(reports.day_of_month))?;
    let at = day.and_hms_opt(u32::from(reports.hour_utc), 0, 0)?;
    Some(Utc.from_utc_datetime(&at).timestamp())
}

/// The latest report time at or before `now`: this month's once it has
/// come, else last month's.
pub fn last_slot(reports: &ClientReportSettings, now: i64) -> Option<i64> {
    let this_month = month_of(now)?;
    [
        Some(this_month),
        this_month.checked_sub_months(Months::new(1)),
    ]
    .into_iter()
    .flatten()
    .filter_map(|month| slot_in(reports, month))
    .find(|&slot| slot <= now)
}

/// The first report time after `now`: this month's until it comes, then
/// next month's.
pub fn next_slot(reports: &ClientReportSettings, now: i64) -> Option<i64> {
    let this_month = month_of(now)?;
    [
        Some(this_month),
        this_month.checked_add_months(Months::new(1)),
    ]
    .into_iter()
    .flatten()
    .filter_map(|month| slot_in(reports, month))
    .find(|&slot| slot > now)
}

/// The report time (the set day at the set hour, UTC) that `now` is at or
/// less than [`LATE_SEND_SECS`] after; `None` between them. Last month's
/// counts too, for a slot late on the 28th that runs into the 1st.
pub fn report_slot(reports: &ClientReportSettings, now: i64) -> Option<i64> {
    last_slot(reports, now).filter(|&slot| now < slot + LATE_SEND_SECS)
}

/// The report time due at `now`: [`report_slot`], or the latest one after
/// its day while its month is under way, until the next report time comes:
/// the reports were held since it came (`held_at`), so a held month goes out
/// once the hold ends, or some of them went out (`started`), so the rest
/// follow once their servers accept.
pub fn due_slot(
    reports: &ClientReportSettings,
    now: i64,
    held_at: Option<i64>,
    started: bool,
) -> Option<i64> {
    last_slot(reports, now).filter(|&slot| {
        now < slot + LATE_SEND_SECS || started || held_at.is_some_and(|held| held >= slot)
    })
}

/// A listed recipient's report of the month that did not go out, and what
/// becomes of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undelivered {
    pub failure: ClientReportFailure,
    pub retry: ClientReportRetry,
}

/// The latest report time at `now`, its month's key, how many of its emails
/// went out and the listed recipients it failed for.
struct Month {
    slot: i64,
    key: String,
    sent: i64,
    undelivered: Vec<Undelivered>,
}

impl Month {
    /// Some of its emails were tried: the rest stay due until the next
    /// report time.
    fn started(&self) -> bool {
        self.sent > 0 || !self.undelivered.is_empty()
    }

    /// Of `unsent` (the recipients without the month's report), those the
    /// next run tries: never failed, or failed and tried again.
    fn waiting(&self, unsent: Vec<ClientReportRecipient>) -> Vec<ClientReportRecipient> {
        unsent
            .into_iter()
            .filter(|r| {
                !self.undelivered.iter().any(|u| {
                    u.failure.organization_id == r.organization_id
                        && u.failure.email.eq_ignore_ascii_case(&r.email)
                        && !u.retry.retries()
                })
            })
            .collect()
    }
}

async fn latest_month(
    pool: &SqlitePool,
    reports: &ClientReportSettings,
    now: i64,
) -> Result<Option<Month>, StoreError> {
    let Some((slot, key)) = last_slot(reports, now).and_then(|s| Some((s, month_key(s)?))) else {
        return Ok(None);
    };
    let sent = count_client_report_sends(pool, &key).await?;
    let undelivered = list_client_report_failures(pool, &key)
        .await?
        .into_iter()
        .map(|failure| Undelivered {
            retry: client_report_retry(failure.permanent, failure.first_failed_at, now),
            failure,
        })
        .collect();
    Ok(Some(Month {
        slot,
        key,
        sent,
        undelivered,
    }))
}

/// The monthly send as the Client reports page and the organization page
/// show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientReportSchedule {
    /// When the reports go out next: the report time that is due while some
    /// recipient does not have it yet, else the next one; `None` while the
    /// reports are off.
    pub next_send_at: Option<i64>,
    /// The organizations that get their report then: those with a recipient
    /// still waiting for the due one, else every listed organization.
    pub organizations_due: i64,
    /// When a report last went out; `None` before the first.
    pub last_sent_at: Option<i64>,
    /// Why the due reports wait; `None` unless a report time has come, some
    /// have not gone out, and the next run would hold them.
    pub held: Option<ClientReportHold>,
    /// The latest report time's emails that did not reach a listed
    /// recipient, shown until the next report time.
    pub undelivered: Vec<Undelivered>,
    /// How many of the latest report time's emails were tried: sent, or
    /// failed for a listed recipient.
    pub attempted: i64,
    /// The due report time and the organizations still waiting for it.
    pending: Option<(i64, Vec<i64>)>,
    /// The next report time; `None` while the reports are off.
    upcoming: Option<i64>,
}

impl ClientReportSchedule {
    /// Whether `organization_id` has a recipient still waiting for the due report.
    fn waiting(&self, organization_id: i64) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, ids)| ids.contains(&organization_id))
    }

    /// When `organization_id`'s report goes out next: the due report time
    /// while one of its recipients does not have it yet, else the next one.
    pub fn next_send_for(&self, organization_id: i64) -> Option<i64> {
        match &self.pending {
            Some((slot, _)) if self.waiting(organization_id) => Some(*slot),
            _ => self.upcoming,
        }
    }

    /// Whether `organization_id`'s due report is held.
    pub fn held_for(&self, organization_id: i64) -> bool {
        self.held.is_some() && self.waiting(organization_id)
    }
}

/// The schedule of the stored `settings` at `now`.
pub async fn schedule(
    pool: &SqlitePool,
    settings: &AppSettings,
    now: i64,
) -> Result<ClientReportSchedule, StoreError> {
    let reports = &settings.client_reports;
    let last_sent_at = client_reports_last_sent_at(pool).await?;
    let listed = by_organization(list_client_report_recipients(pool).await?).len() as i64;
    if !reports.enabled {
        return Ok(ClientReportSchedule {
            organizations_due: listed,
            last_sent_at,
            ..ClientReportSchedule::default()
        });
    }
    let mut pending = None;
    let (mut undelivered, mut attempted) = (Vec::new(), 0);
    if let Some(month) = latest_month(pool, reports, now).await? {
        let held_at = get_client_reports_held_at(pool).await?;
        if due_slot(reports, now, held_at, month.started()).is_some() {
            let unsent = list_unsent_client_report_recipients(pool, &month.key).await?;
            let waiting = month.waiting(unsent);
            if !waiting.is_empty() {
                let ids = by_organization(waiting).into_iter().map(|(id, ..)| id);
                pending = Some((month.slot, ids.collect::<Vec<_>>()));
            }
        }
        attempted = month.sent + month.undelivered.len() as i64;
        undelivered = month.undelivered;
    }
    let held = match pending {
        Some(_) => client_report_hold(imap_pass_status(pool).await?.last_success_at, now),
        None => None,
    };
    let upcoming = next_slot(reports, now);
    Ok(ClientReportSchedule {
        next_send_at: pending.as_ref().map(|(slot, _)| *slot).or(upcoming),
        organizations_due: pending.as_ref().map_or(listed, |(_, ids)| ids.len() as i64),
        last_sent_at,
        held,
        undelivered,
        attempted,
        pending,
        upcoming,
    })
}

/// The cause recorded for a listed address mail cannot be sent to (the list
/// is checked when saved, so only an address stored before that could be).
const NOT_AN_ADDRESS: &str = "not a valid address";

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// The reports are off, or it is not their time.
    NotDue,
    /// Every listed recipient has this month's report.
    AlreadySent,
    /// The listed recipients without this month's report refused it or
    /// were given up for it: nothing is tried until the next report time.
    Undeliverable,
    /// Nobody is listed.
    NoRecipients,
    /// Due, but held: nothing was sent, and the month stays due.
    Held(ClientReportHold),
    /// Sending is off or incomplete, or every email failed: retried on the
    /// next run while the month is due.
    Failed,
    /// `sent` emails went out (and `failed` did not) for `organizations`,
    /// the organizations that were waiting.
    Sent {
        organizations: usize,
        sent: usize,
        failed: usize,
    },
    /// Cancelled between two emails after `sent` went out: the rest go on
    /// the next run.
    Stopped { sent: usize },
}

/// `recipients` by organization, in their order.
fn by_organization(recipients: Vec<ClientReportRecipient>) -> Vec<(i64, String, Vec<String>)> {
    let mut groups: Vec<(i64, String, Vec<String>)> = Vec::new();
    for r in recipients {
        match groups
            .iter_mut()
            .find(|(id, _, _)| *id == r.organization_id)
        {
            Some((_, _, emails)) => emails.push(r.email),
            None => groups.push((r.organization_id, r.organization, vec![r.email])),
        }
    }
    groups
}

/// Send the due report to every listed recipient who does not have it yet,
/// recording each email as it goes out; stop between two emails once
/// `token` is cancelled.
pub async fn send_if_due(
    state: &AppState,
    settings: &AppSettings,
    now: i64,
    token: &CancellationToken,
) -> Delivery {
    let reports = &settings.client_reports;
    if !reports.enabled {
        return Delivery::NotDue;
    }
    let held_at = match get_client_reports_held_at(&state.pool).await {
        Ok(held_at) => held_at,
        Err(e) => {
            tracing::warn!(error = %e, "client reports: could not read when they were held");
            return Delivery::Failed;
        }
    };
    let month = match latest_month(&state.pool, reports, now).await {
        Ok(Some(month)) => month,
        Ok(None) => return Delivery::NotDue,
        Err(e) => {
            tracing::warn!(error = %e, "client reports: could not read what went out");
            return Delivery::Failed;
        }
    };
    if due_slot(reports, now, held_at, month.started()).is_none() {
        return Delivery::NotDue;
    }
    let unsent = match list_unsent_client_report_recipients(&state.pool, &month.key).await {
        Ok(unsent) if unsent.is_empty() && month.started() => return Delivery::AlreadySent,
        // Nothing went out this month and nobody is waiting: nobody is listed.
        Ok(unsent) if unsent.is_empty() => return Delivery::NoRecipients,
        Ok(unsent) => month.waiting(unsent),
        Err(e) => {
            tracing::warn!(error = %e, "client reports: could not read the recipients");
            return Delivery::Failed;
        }
    };
    if unsent.is_empty() {
        return Delivery::Undeliverable;
    }
    let last_read_at = match imap_pass_status(&state.pool).await {
        Ok(status) => status.last_success_at,
        Err(e) => {
            tracing::warn!(error = %e, "client reports: could not read when the mailbox was read");
            return Delivery::Failed;
        }
    };
    if let Some(hold) = client_report_hold(last_read_at, now) {
        tracing::warn!(
            last_read_at,
            "client reports held: the mailbox has not been read lately; they go out once it is"
        );
        if let Err(e) = set_client_reports_held_at(&state.pool, now).await {
            tracing::warn!(error = %e, "client reports: held, but could not record it");
        }
        return Delivery::Held(hold);
    }
    let config = match report_config(settings) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(reason = e.reason(), error = %e, "client reports: sending is not ready");
            return Delivery::Failed;
        }
    };
    let groups = by_organization(unsent);
    let letterhead = Letterhead::load(&state.pool, settings).await;
    let (mut sent, mut failed) = (0, 0);
    for (organization_id, organization, emails) in &groups {
        if token.is_cancelled() {
            return Delivery::Stopped { sent };
        }
        let content = match organization_report(
            &state.pool,
            *organization_id,
            organization,
            now,
            &letterhead,
        )
        .await
        {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(organization, error = %e, "client report: could not be rendered");
                failed += emails.len();
                continue;
            }
        };
        for email in emails {
            if token.is_cancelled() {
                return Delivery::Stopped { sent };
            }
            let failure = |permanent: bool, cause: &'static str, detail: String| {
                let month = &month.key;
                let pool = &state.pool;
                async move {
                    let record = FailedClientReport {
                        organization_id: *organization_id,
                        email,
                        month,
                        permanent,
                        cause,
                        detail: &detail,
                        failed_at: now,
                    };
                    if let Err(e) = record_client_report_failure(pool, &record).await {
                        tracing::warn!(organization, to = email, error = %e, "client report: not sent, and could not record it");
                    }
                }
            };
            let Some(to) = crate::mail::parse_address(email) else {
                tracing::warn!(
                    organization,
                    to = email,
                    "client report: not a valid address"
                );
                failure(true, NOT_AN_ADDRESS, String::new()).await;
                failed += 1;
                continue;
            };
            match state.mail.send(&config, &content.message(to)).await {
                Ok(()) => {
                    sent += 1;
                    if let Err(e) = record_client_report_send(
                        &state.pool,
                        *organization_id,
                        email,
                        &month.key,
                        now,
                    )
                    .await
                    {
                        tracing::warn!(organization, to = email, error = %e, "client report: sent, but could not record it");
                    }
                }
                Err(e) => {
                    tracing::warn!(organization, to = email, reason = e.reason(), permanent = e.is_permanent(), error = %e, "client report not sent");
                    failure(e.is_permanent(), e.cause(), e.to_string()).await;
                    failed += 1;
                }
            }
        }
    }
    if sent == 0 {
        return Delivery::Failed;
    }
    Delivery::Sent {
        organizations: groups.len(),
        sent,
        failed,
    }
}

/// One run at `now`.
pub async fn run_once(state: &AppState, now: i64, token: &CancellationToken) -> Delivery {
    let settings = match AppSettings::load(&state.pool).await {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!(error = %e, "client reports: could not read the settings");
            return Delivery::Failed;
        }
    };
    let delivery = send_if_due(state, &settings, now, token).await;
    match &delivery {
        Delivery::Sent {
            organizations,
            sent,
            failed,
        } => tracing::info!(organizations, sent, failed, "client reports sent"),
        Delivery::Stopped { sent } => {
            tracing::info!(
                sent,
                "client reports stopped; the rest go out on the next run"
            );
        }
        _ => {}
    }
    delivery
}

/// [`run_once`] at the current time (the supervisor's loop body).
pub async fn run(state: AppState, token: CancellationToken) {
    run_once(&state, unix_now(), &token).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;
    /// 2026-10-01 07:00 UTC.
    const OCT_1_7: i64 = 1_790_838_000;

    fn reports(day: u8, hour: u8) -> ClientReportSettings {
        ClientReportSettings {
            enabled: true,
            day_of_month: day,
            hour_utc: hour,
            reply_to: String::new(),
        }
    }

    #[test]
    fn the_slot_is_the_day_and_hour_and_the_day_after() {
        let first = reports(1, 7);
        assert_eq!(report_slot(&first, OCT_1_7), Some(OCT_1_7));
        assert_eq!(report_slot(&first, OCT_1_7 + 23 * HOUR), Some(OCT_1_7));
        assert_eq!(report_slot(&first, OCT_1_7 - 1), None);
        assert_eq!(report_slot(&first, OCT_1_7 + 24 * HOUR), None);
        let late = reports(28, 23);
        // 2026-10-28 23:00 UTC, and still due at 22:59 the next day.
        let slot = OCT_1_7 + 27 * 24 * HOUR + 16 * HOUR;
        assert_eq!(report_slot(&late, slot + 23 * HOUR + 59 * 60), Some(slot));
        assert_eq!(report_slot(&late, slot - HOUR), None);
        // 2027-02-28 23:00 UTC is still due at 00:30 on 1 March, and its
        // sends are February's.
        let at = |m, d, h, min| {
            Utc.with_ymd_and_hms(2027, m, d, h, min, 0)
                .unwrap()
                .timestamp()
        };
        assert_eq!(report_slot(&late, at(3, 1, 0, 30)), Some(at(2, 28, 23, 0)));
        assert_eq!(report_slot(&late, at(3, 1, 23, 0)), None);
        assert_eq!(month_key(at(2, 28, 23, 0)).as_deref(), Some("2027-02"));
    }

    #[test]
    fn the_next_slot_follows_the_last() {
        let first = reports(1, 7);
        // 2026-11-01 07:00 UTC.
        let nov_1_7 = OCT_1_7 + 31 * 24 * HOUR;
        assert_eq!(next_slot(&first, OCT_1_7 - 1), Some(OCT_1_7));
        assert_eq!(next_slot(&first, OCT_1_7), Some(nov_1_7));
        assert_eq!(last_slot(&first, OCT_1_7 + 10 * 24 * HOUR), Some(OCT_1_7));
        assert_eq!(
            last_slot(&first, OCT_1_7 - 1),
            Some(OCT_1_7 - 30 * 24 * HOUR)
        );
        // December runs into January.
        let dec_2 = Utc
            .with_ymd_and_hms(2026, 12, 2, 12, 0, 0)
            .unwrap()
            .timestamp();
        let jan_1 = Utc
            .with_ymd_and_hms(2027, 1, 1, 7, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(next_slot(&first, dec_2), Some(jan_1));
    }

    #[test]
    fn a_held_or_started_month_stays_due_until_the_next_report_time() {
        let first = reports(1, 7);
        let nov_1_7 = OCT_1_7 + 31 * 24 * HOUR;
        let oct_5 = OCT_1_7 + 4 * 24 * HOUR;
        // Neither: the day only.
        assert_eq!(due_slot(&first, OCT_1_7 + HOUR, None, false), Some(OCT_1_7));
        assert_eq!(due_slot(&first, oct_5, None, false), None);
        // Held on the day: still due days later, until next month's comes.
        let held = Some(OCT_1_7 + HOUR);
        assert_eq!(due_slot(&first, oct_5, held, false), Some(OCT_1_7));
        assert_eq!(due_slot(&first, nov_1_7 - 1, held, false), Some(OCT_1_7));
        assert_eq!(due_slot(&first, nov_1_7 + 2 * 24 * HOUR, held, false), None);
        // Some went out: the rest stay due the same way.
        assert_eq!(due_slot(&first, oct_5, None, true), Some(OCT_1_7));
        assert_eq!(due_slot(&first, nov_1_7 - 1, None, true), Some(OCT_1_7));
        // A hold of an earlier month keeps nothing due.
        let september = Some(OCT_1_7 - 25 * 24 * HOUR);
        assert_eq!(due_slot(&first, oct_5, september, false), None);
    }

    #[test]
    fn recipients_are_grouped_by_organization() {
        let r = |id: i64, org: &str, email: &str| ClientReportRecipient {
            organization_id: id,
            organization: org.into(),
            email: email.into(),
        };
        let groups = by_organization(vec![
            r(2, "Acme", "a@acme.example"),
            r(2, "Acme", "b@acme.example"),
            r(3, "Beta", "c@beta.example"),
        ]);
        assert_eq!(
            groups,
            [
                (
                    2,
                    "Acme".into(),
                    vec!["a@acme.example".into(), "b@acme.example".into()]
                ),
                (3, "Beta".into(), vec!["c@beta.example".into()]),
            ]
        );
    }
}
