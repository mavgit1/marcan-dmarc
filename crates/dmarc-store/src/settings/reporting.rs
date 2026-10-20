//! Where DMARC reports are sent, and how they reach the mailbox.

use super::SettingsGroup;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where DMARC reports are sent, and how they reach the mailbox, as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportingSettings {
    /// Mailbox address DMARC `rua=` tags point at, trimmed; `None` when not set.
    pub report_address: Option<String>,
    /// `None` until a mode is saved explicitly (the setup's Report delivery step).
    pub reporting_mode: Option<ReportingMode>,
}

impl SettingsGroup for ReportingSettings {
    const KEY: &'static str = "reporting";
}

/// How reports reach the mailbox: domains' `rua=` names the report address
/// itself (`direct`), or may name other addresses that forward to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ReportingMode {
    #[default]
    Direct,
    Forwarded,
}
