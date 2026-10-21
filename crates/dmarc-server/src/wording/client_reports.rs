//! Client reports in words: a domain's line in the report (its figures,
//! the step it can take now, the notes beside them), and for Settings →
//! Client reports the reports of the latest report time that did not reach
//! a recipient, each with its cause and what becomes of it.

use super::readiness::verdict;
use super::{count_noun, format_count, format_pct, one_or_many};
use crate::client_reports::job::Undelivered;
use chrono::{DateTime, Utc};
use dmarc_assess::{ClientReportRetry, PolicyReadiness, ReportLine, CLIENT_REPORT_RETRY_DAYS};

/// One figure of a domain's line, for the renderer to colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Figure {
    Messages,
    Passed,
    DeliveredFailures,
    Blocked,
    Forwarded,
}

/// A domain line's figures, in the order they are read: `9,973 messages`,
/// `97.7% passed DMARC`, then `224 failed but were delivered`, `30
/// blocked` and `5 forwarded` when there are any; `no mail reported`
/// without messages.
pub fn line_figures(line: &ReportLine) -> Vec<(Figure, String)> {
    let Some(passed) = line.passed_pct else {
        return vec![(Figure::Messages, "no mail reported".to_owned())];
    };
    let mut figures = vec![
        (Figure::Messages, count_noun(line.messages, "message")),
        (
            Figure::Passed,
            format!("{} passed DMARC", format_pct(passed)),
        ),
    ];
    let rest = [
        (
            Figure::DeliveredFailures,
            line.delivered_failures,
            format!(
                "failed but {} delivered",
                one_or_many(line.delivered_failures, "was", "were")
            ),
        ),
        (Figure::Blocked, line.blocked, "blocked".to_owned()),
        (Figure::Forwarded, line.forwarded, "forwarded".to_owned()),
    ];
    figures.extend(
        rest.into_iter()
            .filter(|(_, n, _)| *n > 0)
            .map(|(figure, n, words)| (figure, format!("{} {words}", format_count(n)))),
    );
    figures
}

/// The step a domain can take now, in the words of the domain page's
/// verdict: `ready for p=quarantine`, `raise pct to 100 first`.
pub fn step_words(r: &PolicyReadiness) -> String {
    lower_first(&verdict(r).lead)
}

/// The policy beside a domain's name, its badge mid-sentence: `p=none`,
/// `no record`, `not checked yet`.
pub fn policy_words(label: &str) -> String {
    lower_first(label)
}

/// The notes beside a domain's figures, the issues' badges as one
/// sentence: `DNS grade D, 12.0% without DKIM`, `No reports for 9 days`.
pub fn note_words(labels: &[&str]) -> String {
    let mut words: Vec<String> = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        words.push(if i == 0 {
            super::capitalized(label)
        } else {
            lower_first(label)
        });
    }
    words.join(", ")
}

/// `text` with its first letter in lower case, unless its first word is an
/// abbreviation (`DNS grade D`, `DKIM`).
fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && !second.is_uppercase() => {
            first.to_lowercase().chain(text.chars().skip(1)).collect()
        }
        _ => text.to_owned(),
    }
}

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

    fn line(messages: i64, compliant: i64, delivered: i64, blocked: i64) -> ReportLine {
        ReportLine {
            messages,
            #[allow(clippy::cast_precision_loss)]
            passed_pct: (messages > 0).then(|| compliant as f64 * 100.0 / messages as f64),
            delivered_failures: delivered,
            delivered_failures_status: dmarc_assess::Status::Neutral,
            blocked,
            forwarded: messages - compliant - delivered - blocked,
            step: None,
            notes: Vec::new(),
        }
    }

    fn words(line: &ReportLine) -> Vec<String> {
        line_figures(line).into_iter().map(|(_, w)| w).collect()
    }

    #[test]
    fn a_line_says_its_figures_and_leaves_out_the_zeros() {
        assert_eq!(
            words(&line(9_973, 9_744, 224, 0)),
            [
                "9,973 messages",
                "97.7% passed DMARC",
                "224 failed but were delivered",
                "5 forwarded"
            ]
        );
        assert_eq!(
            words(&line(410, 400, 1, 9)),
            [
                "410 messages",
                "97.6% passed DMARC",
                "1 failed but was delivered",
                "9 blocked"
            ]
        );
        assert_eq!(
            words(&line(1, 1, 0, 0)),
            ["1 message", "100.0% passed DMARC"]
        );
        assert_eq!(words(&line(0, 0, 0, 0)), ["no mail reported"]);
    }

    #[test]
    fn notes_read_as_one_sentence() {
        assert_eq!(
            note_words(&["DNS grade D", "12.0% without DKIM"]),
            "DNS grade D, 12.0% without DKIM"
        );
        assert_eq!(
            note_words(&["12.0% without DKIM", "No reports for 9 days", "No DKIM"]),
            "12.0% without DKIM, no reports for 9 days, no DKIM"
        );
        assert_eq!(
            lower_first("Ready for p=quarantine"),
            "ready for p=quarantine"
        );
        assert_eq!(lower_first("DNS grade D"), "DNS grade D");
        assert_eq!(policy_words("No record"), "no record");
        assert_eq!(policy_words("p=none"), "p=none");
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
