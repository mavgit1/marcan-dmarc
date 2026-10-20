//! Scope statistics and the timeseries.

use dmarc_assess::{count_trend, MailVerdicts, Trend, TrendSense};
use dmarc_store::{ScopeStats, TimeseriesRow};
use serde::Serialize;
use ts_rs::TS;

/// One timeseries bucket (a UTC day or week); empty buckets are zero points.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct TimeseriesPoint {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: TimeseriesRow,
    /// Failures quarantined or rejected.
    pub blocked_messages: i64,
}

impl From<TimeseriesRow> for TimeseriesPoint {
    fn from(row: TimeseriesRow) -> Self {
        Self {
            blocked_messages: row.mail.blocked(),
            row,
        }
    }
}

/// `GET /api/statistics`: scope totals, delivered failures and the change
/// against the previous window of equal length.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Statistics {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: ScopeStats,
    /// Monitored domains in the scope, with or without reports in its time range
    /// (`active_domains` counts those with reports).
    pub domain_count: i64,
    #[serde(flatten)]
    #[ts(flatten)]
    pub verdicts: MailVerdicts,
    /// Whether reports cover the whole previous window, so the `prev_*` figures and
    /// deltas compare like with like. False without `since` or when data starts later.
    pub has_previous_window: bool,
    pub prev_total_messages: i64,
    /// Blocked by policy in the previous window.
    pub prev_blocked_messages: i64,
    pub prev_delivered_failures: i64,
    pub prev_forwarded_failures: i64,
    /// Change in total messages vs the previous window (relative, percent);
    /// `None` without a previous window. The same for the counts below.
    pub total_messages_trend: Option<Trend>,
    /// Passed DMARC, against the previous window.
    pub compliant_messages_trend: Option<Trend>,
    pub blocked_messages_trend: Option<Trend>,
    pub forwarded_failures_trend: Option<Trend>,
    pub delivered_failures_trend: Option<Trend>,
    /// Percentage-point change in delivered compliance
    /// (`compliant / (compliant + delivered failures)`) vs the previous window;
    /// `None` without a previous window or when either window delivered nothing.
    pub delivered_compliance_trend: Option<Trend>,
}

/// The statistics of `current`, compared with `previous` when the scope's
/// reports cover a whole previous window (see `dmarc_assess::previous_window`).
pub fn statistics(
    current: ScopeStats,
    previous: Option<ScopeStats>,
    domain_count: i64,
) -> Statistics {
    let mail = current.mail.mail();
    let has_previous = previous.is_some();
    let prev = previous.map(|p| p.mail).unwrap_or_default();
    let prev_mail = prev.mail();
    Statistics {
        verdicts: mail.verdicts(),
        row: current,
        domain_count,
        has_previous_window: has_previous,
        prev_total_messages: prev.total_messages,
        prev_blocked_messages: prev_mail.blocked(),
        prev_delivered_failures: prev.delivered_failures,
        prev_forwarded_failures: prev.forwarded_failures,
        total_messages_trend: has_previous
            .then(|| count_trend(mail.total, prev.total_messages, TrendSense::Neutral)),
        compliant_messages_trend: has_previous
            .then(|| count_trend(mail.compliant, prev_mail.compliant, TrendSense::Neutral)),
        blocked_messages_trend: has_previous
            .then(|| count_trend(mail.blocked(), prev_mail.blocked(), TrendSense::Neutral)),
        forwarded_failures_trend: has_previous.then(|| {
            count_trend(
                mail.forwarded_failures,
                prev.forwarded_failures,
                TrendSense::Neutral,
            )
        }),
        delivered_failures_trend: has_previous.then(|| {
            count_trend(
                mail.delivered_failures,
                prev.delivered_failures,
                TrendSense::HigherIsWorse,
            )
        }),
        delivered_compliance_trend: has_previous
            .then(|| mail.delivered_compliance_trend(&prev_mail))
            .flatten(),
    }
}
