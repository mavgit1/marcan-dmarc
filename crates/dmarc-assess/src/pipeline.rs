//! Is the ingestion pipeline working: is the mailbox polled, do reports arrive?

use crate::receiving::{stalest_unread, AccountReading};
use crate::status::Status;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the pipeline's state is computed from.
#[derive(Debug, Clone, Copy, Default)]
pub struct PipelineFacts {
    /// A mailbox pass is running right now.
    pub syncing: bool,
    /// The running pass is downloading reports (not still connecting or scanning).
    pub importing: bool,
    /// An import is unfinished (the mailbox worker's `import_unfinished`): a
    /// pass runs, or a long import waits between its passes (it gave way to
    /// another mailbox and resumes), or a re-import waits for its pass.
    pub import_unfinished: bool,
    pub receive_configured: bool,
    /// The last pass ended with an error.
    pub receive_error: bool,
    pub seconds_since_check: Option<i64>,
    pub poll_interval_secs: u64,
    pub hours_since_ingest: Option<f64>,
    pub report_freshness_warning_hours: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum MailboxState {
    Importing,
    Syncing,
    NotConfigured,
    Error,
    AwaitingFirstPoll,
    Active,
    /// One poll missed (plus a minute of slack).
    Overdue,
    /// Six polls missed, and at least an hour.
    Stalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ReportsState {
    NoReports,
    /// An import is unfinished: the reports stored so far are not all there
    /// are, however recent the last one.
    Importing,
    /// Within the freshness threshold.
    UpToDate,
    /// Up to twice the threshold.
    Late,
    Overdue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Pipeline {
    pub mailbox: MailboxState,
    pub mailbox_status: Status,
    pub reports: ReportsState,
    pub reports_status: Status,
    /// The worse of the two.
    pub status: Status,
    /// Which of the two the header's one label names.
    pub headline: PipelineHeadline,
}

/// Which line the header's one label names: a failing mailbox, else failing
/// reports, else whichever warns (the mailbox first), else an unfinished
/// import, else the mailbox while neither is judged yet (not configured,
/// awaiting the first poll); both well: neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PipelineHeadline {
    Mailbox,
    Reports,
    /// An import is unfinished and nothing fails or warns: not all well yet.
    Importing,
    AllWell,
}

fn headline(mailbox: Status, reports: Status, importing: bool) -> PipelineHeadline {
    let status = mailbox.worst(reports);
    if mailbox == Status::Fail {
        PipelineHeadline::Mailbox
    } else if reports == Status::Fail {
        PipelineHeadline::Reports
    } else if status == Status::Warn {
        if mailbox == Status::Warn {
            PipelineHeadline::Mailbox
        } else {
            PipelineHeadline::Reports
        }
    } else if importing {
        PipelineHeadline::Importing
    } else if status == Status::Neutral {
        PipelineHeadline::Mailbox
    } else {
        PipelineHeadline::AllWell
    }
}

/// An import is under way: a pass downloads reports, or a long import waits
/// between its passes. A running pass still connecting or scanning is not
/// one by itself: every routine poll does that.
fn importing(f: &PipelineFacts) -> bool {
    f.importing || (f.import_unfinished && !f.syncing)
}

pub fn pipeline_status(f: &PipelineFacts) -> Pipeline {
    let (mailbox, mailbox_status) = mailbox(f);
    let (reports, reports_status) = reports(f);
    Pipeline {
        mailbox,
        mailbox_status,
        reports,
        reports_status,
        status: mailbox_status.worst(reports_status),
        headline: headline(mailbox_status, reports_status, importing(f)),
    }
}

fn mailbox(f: &PipelineFacts) -> (MailboxState, Status) {
    // Working as intended: green, not blue (blue means blocked by policy).
    if f.syncing {
        let state = if f.importing {
            MailboxState::Importing
        } else {
            MailboxState::Syncing
        };
        return (state, Status::Pass);
    }
    if !f.receive_configured {
        return (MailboxState::NotConfigured, Status::Neutral);
    }
    if f.receive_error {
        return (MailboxState::Error, Status::Fail);
    }
    // Between the passes of a long import: no pass runs, and it is not done.
    if importing(f) {
        return (MailboxState::Importing, Status::Pass);
    }
    let Some(since) = f.seconds_since_check else {
        return (MailboxState::AwaitingFirstPoll, Status::Neutral);
    };
    let interval = f.poll_interval_secs.max(1) as i64;
    if since <= interval * 2 + 60 {
        (MailboxState::Active, Status::Pass)
    } else if since <= (interval * 6).max(3600) {
        (MailboxState::Overdue, Status::Warn)
    } else {
        (MailboxState::Stalled, Status::Fail)
    }
}

fn reports(f: &PipelineFacts) -> (ReportsState, Status) {
    // Fresh reports say nothing while more wait in the mailbox, and an old
    // last report is not late while the import still brings them in.
    if importing(f) {
        return (ReportsState::Importing, Status::Pass);
    }
    let Some(hours) = f.hours_since_ingest else {
        return (ReportsState::NoReports, Status::Neutral);
    };
    let threshold = f64::from(f.report_freshness_warning_hours);
    if hours <= threshold {
        (ReportsState::UpToDate, Status::Pass)
    } else if hours <= threshold * 2.0 {
        (ReportsState::Late, Status::Warn)
    } else {
        (ReportsState::Overdue, Status::Fail)
    }
}

/// From this many new messages on, a running import is a bulk import (a
/// first sync of a full mailbox, say) rather than routine polling.
pub const BULK_IMPORT_MIN_MESSAGES: u32 = 50;

/// Whether a running pass is a bulk import worth keeping in view.
pub fn is_bulk_import(importing: bool, total_messages: u32) -> bool {
    importing && total_messages >= BULK_IMPORT_MIN_MESSAGES
}

/// The most recent mailbox poll, successful or not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LastPoll {
    /// Unix seconds; `null` before the first poll.
    pub at: Option<i64>,
    /// That poll is the one that failed. Never while a pass is running (the
    /// error is being retried).
    pub failed: bool,
}

/// `last_check_at` is only written by a pass that reached the mailbox, so a
/// failed connection shows up in `error_at` alone (`None` once a later pass
/// succeeded).
pub fn last_poll(syncing: bool, last_check_at: Option<i64>, error_at: Option<i64>) -> LastPoll {
    let at = last_check_at.max(error_at);
    let failed = !syncing && error_at.is_some_and(|e| e >= last_check_at.unwrap_or(0));
    LastPoll { at, failed }
}

/// Why the client reports are held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientReportHold {
    /// A receiving account was not read without error within
    /// [`ACCOUNT_NOT_READ_DAYS`](crate::ACCOUNT_NOT_READ_DAYS): `account`
    /// (its name) is the one unread longest, `last_read_at` its last
    /// successful read (`null`: none since it reads this mailbox) and
    /// `receiving_since` when it started reading this mailbox (unix
    /// seconds). All `null`: no account receives reports.
    MailboxNotRead {
        account: Option<String>,
        last_read_at: Option<i64>,
        #[serde(default)]
        receiving_since: Option<i64>,
    },
}

/// Whether client reports going out at `now` are held: while any of the
/// receiving `accounts` is not read ([`AccountReading::not_read`], the rule
/// of the "Account not read" alert), naming the one unread longest; an
/// account never read counts from when it started reading its mailbox.
/// With no receiving account, they are held too: no report could arrive.
pub fn client_report_hold(accounts: &[AccountReading], now: i64) -> Option<ClientReportHold> {
    if accounts.is_empty() {
        return Some(ClientReportHold::MailboxNotRead {
            account: None,
            last_read_at: None,
            receiving_since: None,
        });
    }
    stalest_unread(accounts, now).map(|a| ClientReportHold::MailboxNotRead {
        account: Some(a.account.clone()),
        last_read_at: a.last_read_at,
        receiving_since: Some(a.receiving_since),
    })
}

/// A client report that did not reach a recipient for a passing reason
/// (the server unreachable, a 4xx answer, a timeout, a login to fix) is
/// tried again on each hourly run for this many days from its first
/// failure, then given up for the month.
pub const CLIENT_REPORT_RETRY_DAYS: i64 = 3;

/// What becomes of a recipient's client report that did not go out this
/// month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientReportRetry {
    /// Tried again on each run until `until` (unix seconds).
    Retrying { until: i64 },
    /// A passing failure that lasted [`CLIENT_REPORT_RETRY_DAYS`]: given up
    /// for the month.
    GivenUp,
    /// The server refused for good (a 5xx answer, an address that does not
    /// exist): not tried again this month.
    Refused,
}

impl ClientReportRetry {
    /// Whether the next run tries again.
    pub fn retries(self) -> bool {
        matches!(self, Self::Retrying { .. })
    }

    /// Still being tried: worth watching (`warn`); given up or refused: the
    /// client did not get the month's report (`fail`).
    pub fn status(self) -> Status {
        match self {
            Self::Retrying { .. } => Status::Warn,
            Self::GivenUp | Self::Refused => Status::Fail,
        }
    }
}

/// What becomes of a report that failed `permanent`ly or not, first at
/// `first_failed_at`, as of `now`.
pub fn client_report_retry(permanent: bool, first_failed_at: i64, now: i64) -> ClientReportRetry {
    let until = first_failed_at + CLIENT_REPORT_RETRY_DAYS * 86_400;
    match (permanent, now < until) {
        (true, _) => ClientReportRetry::Refused,
        (false, true) => ClientReportRetry::Retrying { until },
        (false, false) => ClientReportRetry::GivenUp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_names_the_worse_line_the_mailbox_first() {
        use Status::*;
        let cases = [
            ((Fail, Fail), PipelineHeadline::Mailbox),
            ((Pass, Fail), PipelineHeadline::Reports),
            ((Warn, Warn), PipelineHeadline::Mailbox),
            ((Pass, Warn), PipelineHeadline::Reports),
            ((Warn, Pass), PipelineHeadline::Mailbox),
            ((Neutral, Neutral), PipelineHeadline::Mailbox),
            ((Pass, Neutral), PipelineHeadline::AllWell),
            ((Pass, Pass), PipelineHeadline::AllWell),
        ];
        for ((mailbox, reports), expected) in cases {
            assert_eq!(
                headline(mailbox, reports, false),
                expected,
                "{mailbox:?} {reports:?}"
            );
        }
        // An unfinished import is named unless something fails or warns.
        assert_eq!(headline(Pass, Pass, true), PipelineHeadline::Importing);
        assert_eq!(headline(Pass, Neutral, true), PipelineHeadline::Importing);
        assert_eq!(headline(Fail, Pass, true), PipelineHeadline::Mailbox);
        assert_eq!(headline(Pass, Warn, true), PipelineHeadline::Reports);
    }

    /// While an import is unfinished nothing reads "all well": not between
    /// its passes (no pass runs) nor while a pass downloads, whatever the
    /// last report's age. A routine poll that only scans says nothing of it.
    #[test]
    fn an_unfinished_import_is_not_all_well() {
        let between = PipelineFacts {
            import_unfinished: true,
            ..polled(10)
        };
        let p = pipeline_status(&between);
        assert_eq!(p.mailbox, MailboxState::Importing);
        assert_eq!(p.reports, ReportsState::Importing);
        assert_eq!(p.headline, PipelineHeadline::Importing);
        assert_eq!(p.status, Status::Pass);

        let downloading = PipelineFacts {
            syncing: true,
            importing: true,
            import_unfinished: true,
            hours_since_ingest: Some(500.0),
            ..polled(10)
        };
        let p = pipeline_status(&downloading);
        assert_eq!(p.reports, ReportsState::Importing);
        assert_eq!(p.headline, PipelineHeadline::Importing);

        let scanning = PipelineFacts {
            syncing: true,
            import_unfinished: true,
            ..polled(10)
        };
        let p = pipeline_status(&scanning);
        assert_eq!(p.mailbox, MailboxState::Syncing);
        assert_eq!(p.reports, ReportsState::UpToDate);
        assert_eq!(p.headline, PipelineHeadline::AllWell);

        // A failing account is named first, even part-way through.
        let failed = PipelineFacts {
            receive_error: true,
            ..between
        };
        let p = pipeline_status(&failed);
        assert_eq!(p.mailbox, MailboxState::Error);
        assert_eq!(p.headline, PipelineHeadline::Mailbox);

        let done = pipeline_status(&polled(10));
        assert_eq!(done.reports, ReportsState::UpToDate);
        assert_eq!(done.headline, PipelineHeadline::AllWell);
    }

    #[test]
    fn a_passing_failure_is_retried_for_three_days_a_refusal_never() {
        let first = 1_790_838_000;
        let day = 86_400;
        let until = first + 3 * day;
        assert_eq!(
            client_report_retry(false, first, first),
            ClientReportRetry::Retrying { until }
        );
        assert_eq!(
            client_report_retry(false, first, until - 1),
            ClientReportRetry::Retrying { until }
        );
        assert_eq!(
            client_report_retry(false, first, until),
            ClientReportRetry::GivenUp
        );
        assert_eq!(
            client_report_retry(true, first, first),
            ClientReportRetry::Refused
        );
        assert!(client_report_retry(false, first, first).retries());
        assert!(!client_report_retry(true, first, first).retries());
        assert_eq!(
            client_report_retry(false, first, first).status(),
            Status::Warn
        );
        assert_eq!(ClientReportRetry::GivenUp.status(), Status::Fail);
        assert_eq!(ClientReportRetry::Refused.status(), Status::Fail);
    }

    #[test]
    fn client_reports_are_held_while_an_account_is_not_read() {
        let now = 1_790_838_000;
        let day = 86_400;
        let reading = |id: i64, last_read_at: Option<i64>, since: i64| AccountReading {
            account_id: id,
            account: format!("Account {id}"),
            last_read_at,
            receiving_since: since,
            locked: None,
            last_error: None,
        };
        let read = reading(1, Some(now - 3 * day), 0);
        assert_eq!(client_report_hold(std::slice::from_ref(&read), now), None);
        let behind = reading(2, Some(now - 3 * day - 1), 0);
        let held = |a: &AccountReading| {
            Some(ClientReportHold::MailboxNotRead {
                account: Some(a.account.clone()),
                last_read_at: a.last_read_at,
                receiving_since: Some(a.receiving_since),
            })
        };
        assert_eq!(
            client_report_hold(&[read.clone(), behind.clone()], now),
            held(&behind)
        );
        // Never read: from when it was set up; a new one holds nothing yet.
        let new = reading(3, None, now - 2 * day);
        assert_eq!(client_report_hold(&[read.clone(), new.clone()], now), None);
        let stuck = reading(4, None, now - 4 * day);
        assert_eq!(
            client_report_hold(&[read, new, stuck.clone(), behind], now),
            held(&stuck)
        );
        assert_eq!(
            client_report_hold(&[], now),
            Some(ClientReportHold::MailboxNotRead {
                account: None,
                last_read_at: None,
                receiving_since: None,
            })
        );
    }

    fn polled(since: i64) -> PipelineFacts {
        PipelineFacts {
            receive_configured: true,
            seconds_since_check: Some(since),
            poll_interval_secs: 300,
            hours_since_ingest: Some(1.0),
            report_freshness_warning_hours: 48,
            ..Default::default()
        }
    }

    #[test]
    fn mailbox_states() {
        assert_eq!(pipeline_status(&polled(660)).mailbox, MailboxState::Active);
        assert_eq!(pipeline_status(&polled(661)).mailbox, MailboxState::Overdue);
        assert_eq!(
            pipeline_status(&polled(3600)).mailbox,
            MailboxState::Overdue
        );
        assert_eq!(
            pipeline_status(&polled(3601)).mailbox,
            MailboxState::Stalled
        );
        let error = PipelineFacts {
            receive_error: true,
            ..polled(10)
        };
        assert_eq!(pipeline_status(&error).mailbox, MailboxState::Error);
        let syncing = PipelineFacts {
            syncing: true,
            ..error
        };
        assert_eq!(pipeline_status(&syncing).mailbox_status, Status::Pass);
        assert_eq!(
            pipeline_status(&PipelineFacts::default()).mailbox,
            MailboxState::NotConfigured
        );
    }

    #[test]
    fn report_freshness_and_overall_status() {
        let late = PipelineFacts {
            hours_since_ingest: Some(60.0),
            ..polled(10)
        };
        let p = pipeline_status(&late);
        assert_eq!(p.reports, ReportsState::Late);
        assert_eq!(p.status, Status::Warn);
        let overdue = PipelineFacts {
            hours_since_ingest: Some(97.0),
            ..polled(10)
        };
        assert_eq!(pipeline_status(&overdue).reports, ReportsState::Overdue);
    }

    #[test]
    fn bulk_import_from_fifty_messages() {
        assert!(!is_bulk_import(true, 49));
        assert!(is_bulk_import(true, 50));
        assert!(!is_bulk_import(false, 500));
    }

    #[test]
    fn last_poll_failed_only_when_the_error_is_newest() {
        assert_eq!(last_poll(false, None, None), LastPoll::default());
        let ok = last_poll(false, Some(100), Some(50));
        assert_eq!((ok.at, ok.failed), (Some(100), false));
        let failed = last_poll(false, Some(100), Some(120));
        assert_eq!((failed.at, failed.failed), (Some(120), true));
        assert!(last_poll(false, None, Some(5)).failed);
        assert!(!last_poll(true, Some(100), Some(120)).failed);
    }
}
