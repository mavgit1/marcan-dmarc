//! The client reports' send in words, for Settings → Client reports: the
//! reports of the latest report time that did not reach a recipient, each
//! with its cause and what becomes of it.

use super::one_or_many;
use crate::client_reports::job::Undelivered;
use chrono::{DateTime, Utc};
use dmarc_assess::{ClientReportRetry, CLIENT_REPORT_RETRY_DAYS};

/// `4 November` (UTC).
fn day_month(unix: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix, 0)
        .map(|t| t.format("%-d %B").to_string())
        .unwrap_or_default()
}

/// One recipient: `x@y (mailbox does not exist)`, `a@b (server
/// unreachable, retrying until 4 November)`.
fn recipient(u: &Undelivered) -> String {
    let cause = &u.failure.cause;
    let email = &u.failure.email;
    match u.retry {
        ClientReportRetry::Refused => format!("{email} ({cause})"),
        ClientReportRetry::Retrying { until } => {
            format!("{email} ({cause}, retrying until {})", day_month(until))
        }
        ClientReportRetry::GivenUp => {
            format!("{email} ({cause}, given up after {CLIENT_REPORT_RETRY_DAYS} days)")
        }
    }
}

/// "2 of 4 reports not delivered: x@y (mailbox does not exist), a@b
/// (server unreachable, retrying until 4 November)." of the `attempted`
/// emails of the latest report time; `None` when all went out.
pub fn undelivered_line(attempted: i64, undelivered: &[Undelivered]) -> Option<String> {
    if undelivered.is_empty() {
        return None;
    }
    let names: Vec<String> = undelivered.iter().map(recipient).collect();
    Some(format!(
        "{} of {attempted} {} not delivered: {}.",
        undelivered.len(),
        one_or_many(attempted, "report", "reports"),
        names.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_assess::client_report_retry;
    use dmarc_store::ClientReportFailure;

    /// 2026-11-01 07:00 UTC.
    const NOV_1: i64 = 1_793_516_400;

    fn undelivered(email: &str, permanent: bool, cause: &str, now: i64) -> Undelivered {
        Undelivered {
            failure: ClientReportFailure {
                organization_id: 1,
                organization: "Acme".into(),
                email: email.into(),
                month: "2026-11".into(),
                permanent,
                cause: cause.into(),
                detail: String::new(),
                first_failed_at: NOV_1,
                last_failed_at: now,
            },
            retry: client_report_retry(permanent, NOV_1, now),
        }
    }

    #[test]
    fn each_undelivered_report_says_why_and_what_follows() {
        let now = NOV_1 + 3600;
        let line = undelivered_line(
            4,
            &[
                undelivered("x@y.example", true, "mailbox does not exist", now),
                undelivered("a@b.example", false, "server unreachable", now),
            ],
        );
        assert_eq!(
            line.as_deref(),
            Some("2 of 4 reports not delivered: x@y.example (mailbox does not exist), a@b.example (server unreachable, retrying until 4 November).")
        );
        let later = NOV_1 + 4 * 86_400;
        assert_eq!(
            undelivered_line(1, &[undelivered("a@b.example", false, "server unreachable", later)])
                .as_deref(),
            Some("1 of 1 report not delivered: a@b.example (server unreachable, given up after 3 days).")
        );
        assert_eq!(undelivered_line(3, &[]), None);
    }
}
