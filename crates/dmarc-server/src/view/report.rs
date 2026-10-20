//! Reports: the list row and one report's records.

use super::domain::{DkimCheck, JudgedChecks, SpfCheck};
use dmarc_assess::{enforcement, Enforcement, MailVerdicts};
use dmarc_identity::SenderIdentity;
use dmarc_store::{OverrideReason, RecordRow, ReportSummaryRow};
use serde::Serialize;
use ts_rs::TS;

/// One `reports` row with its domain name, counts and verdicts.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReportSummary {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: ReportSummaryRow,
    #[serde(flatten)]
    #[ts(flatten)]
    pub verdicts: MailVerdicts,
    /// Verdict on the `p=` published with this report.
    pub enforcement: Enforcement,
    /// Verdict on `sp=`, when published.
    pub subdomain_enforcement: Option<Enforcement>,
}

impl From<ReportSummaryRow> for ReportSummary {
    fn from(row: ReportSummaryRow) -> Self {
        Self {
            verdicts: row.mail.mail().verdicts(),
            enforcement: enforcement(row.policy_p.as_deref()),
            subdomain_enforcement: row.policy_sp.as_deref().map(|sp| enforcement(Some(sp))),
            row,
        }
    }
}

/// A record with its stored SPF / DKIM checks (alignment computed at ingest:
/// PSL, relaxed/strict per the report's `aspf` / `adkim`) and the sender's identity.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ReportRecord {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: RecordRow,
    /// `outcome`'s place worst first (0: a delivered failure, 3: passed),
    /// the order a list of records sorts by (`Outcome::rank`).
    pub outcome_rank: u8,
    /// Every SPF check, most relevant first.
    pub spf_auth: Vec<SpfCheck>,
    /// Every DKIM signature, most relevant first.
    pub dkim_auth: Vec<DkimCheck>,
    /// The receiver's override reasons; a trusted forward makes a delivered failure "Forwarded".
    pub overrides: Vec<OverrideReason>,
    pub identity: SenderIdentity,
}

/// The record with its sender's identity.
pub fn report_record(mut row: RecordRow, identity: SenderIdentity) -> ReportRecord {
    let checks = JudgedChecks::from(std::mem::take(&mut row.checks));
    ReportRecord {
        outcome_rank: row.outcome.rank(),
        spf_auth: checks.spf,
        dkim_auth: checks.dkim,
        overrides: checks.overrides,
        identity,
        row,
    }
}
