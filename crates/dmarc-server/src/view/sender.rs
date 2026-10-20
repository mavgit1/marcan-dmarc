//! Senders: the list row and one IP's cross-domain detail, with their verdicts.
//! Each wire type embeds the store row (flattened) and adds what is judged.

use super::stats::TimeseriesPoint;
use dmarc_assess::{
    dkim_coverage, pass_rate, sender_check, sender_dkim_check, sender_domain_verdict,
    spoofing_status, DkimCoverage, SenderCheck, SenderDomainVerdict, Status,
};
use dmarc_store::{SenderDetailRow, SenderDomainStatsRow, SenderSummaryRow};
use serde::Serialize;
use ts_rs::TS;

/// One source IP within the scope, with its compliance judged.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SenderSummary {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: SenderSummaryRow,
    /// The raw pass rate, `compliant / total` as 0–100 (`Mail::pass_rate`);
    /// 0 without messages.
    pub compliance_pct: f64,
    /// `compliance_pct` judged by the 99 / 95 rule.
    pub compliance_status: Status,
}

impl From<SenderSummaryRow> for SenderSummary {
    fn from(row: SenderSummaryRow) -> Self {
        let rate = pass_rate(row.compliant_messages, row.total_messages);
        Self {
            compliance_pct: rate.pct.unwrap_or(0.0),
            compliance_status: rate.status,
            row,
        }
    }
}

/// One sending domain of a source IP, judged.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SenderDomainRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: SenderDomainStatsRow,
    pub compliance_pct: f64,
    /// Failures quarantined or rejected.
    pub blocked_messages: i64,
    /// The row's one verdict: its outcome, unless DKIM coverage is worse.
    pub verdict: SenderDomainVerdict,
    /// Share of this sender's passing mail signed by the domain.
    pub dkim_coverage: DkimCoverage,
    /// Passed DMARC without an aligned DKIM pass (`compliant - dkim_pass`).
    pub no_aligned_dkim_messages: i64,
    /// SPF across the row's mail.
    pub spf: SenderCheck,
    /// DKIM across the row's mail, coloured by coverage when that is worse.
    pub dkim: SenderCheck,
    /// The row's failures read as spoofing (see `dmarc_assess::spoofing_status`).
    pub spoofing_status: Status,
}

impl From<SenderDomainStatsRow> for SenderDomainRow {
    fn from(row: SenderDomainStatsRow) -> Self {
        let mail = row.mail.mail();
        let no_aligned_dkim_messages = (mail.compliant - row.dkim_pass_messages).max(0);
        let coverage = dkim_coverage(mail.compliant, no_aligned_dkim_messages);
        Self {
            compliance_pct: mail.pass_rate().pct.unwrap_or(0.0),
            blocked_messages: mail.blocked(),
            verdict: sender_domain_verdict(&mail, row.dkim_pass_messages),
            dkim_coverage: coverage,
            no_aligned_dkim_messages,
            spf: sender_check(mail.total, row.spf_pass_messages),
            dkim: sender_dkim_check(mail.total, row.dkim_pass_messages, coverage.status),
            spoofing_status: spoofing_status(&mail),
            row,
        }
    }
}

/// Cross-domain view of one source IP.
#[derive(Debug, Clone, Serialize, TS)]
pub struct SenderDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: SenderDetailRow,
    pub compliance_pct: f64,
    /// Failures quarantined or rejected.
    pub blocked_messages: i64,
    /// Per-domain breakdown: pass rates here tell a shared ESP from spoofing.
    pub items: Vec<SenderDomainRow>,
    pub timeseries: Vec<TimeseriesPoint>,
}

impl From<SenderDetailRow> for SenderDetail {
    fn from(mut row: SenderDetailRow) -> Self {
        Self {
            compliance_pct: row.mail.mail().pass_rate().pct.unwrap_or(0.0),
            blocked_messages: row.mail.blocked(),
            items: std::mem::take(&mut row.items)
                .into_iter()
                .map(Into::into)
                .collect(),
            timeseries: std::mem::take(&mut row.timeseries)
                .into_iter()
                .map(Into::into)
                .collect(),
            row,
        }
    }
}
