//! The catalogue: what creates an alert, in the groups the settings show
//! it in ([`ALERT_CATALOGUE`]), and the id of each [`AlertKind`]. Its words
//! are the server's.

use super::events::{AlertKind, AlertedIssue};
use crate::dns::CheckKind;
use crate::status::Status;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The groups the alert catalogue is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AlertGroup {
    Issues,
    Health,
    DnsRecords,
    Policy,
    Readiness,
    Parked,
    Senders,
}

/// One kind of alert as the user can choose it: an [`AlertKind`], split by
/// issue for issues opening and clearing and by check for DNS record changes.
/// The server words each (`wording::alerts::catalogue_title` and
/// `catalogue_trigger`); this crate keeps no sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueKind {
    IssueOpened(AlertedIssue),
    IssueResolved(AlertedIssue),
    HealthChanged,
    DnsRecordChanged(CheckKind),
    PolicyChanged,
    ReadinessReached,
    ParkedChanged { parked: bool },
    NewFailingSender,
}

impl CatalogueKind {
    /// The catalogue kind `kind` belongs to.
    pub fn of(kind: &AlertKind) -> Self {
        match kind {
            AlertKind::IssueOpened { issue, .. } => Self::IssueOpened(*issue),
            AlertKind::IssueResolved { issue, .. } => Self::IssueResolved(*issue),
            AlertKind::HealthChanged { .. } => Self::HealthChanged,
            AlertKind::DnsRecordChanged { check, .. } => Self::DnsRecordChanged(*check),
            AlertKind::PolicyChanged { .. } => Self::PolicyChanged,
            AlertKind::ReadinessReached { .. } => Self::ReadinessReached,
            AlertKind::ParkedChanged { parked } => Self::ParkedChanged { parked: *parked },
            AlertKind::NewFailingSender { .. } => Self::NewFailingSender,
        }
    }

    /// Its stable id, stored in the alert settings' muted kinds.
    pub fn id(self) -> &'static str {
        match self {
            Self::IssueOpened(AlertedIssue::DeliveredFailures) => "issue_opened.delivered_failures",
            Self::IssueResolved(AlertedIssue::DeliveredFailures) => {
                "issue_resolved.delivered_failures"
            }
            Self::IssueOpened(AlertedIssue::ReportsStopped) => "issue_opened.reports_stopped",
            Self::IssueResolved(AlertedIssue::ReportsStopped) => "issue_resolved.reports_stopped",
            Self::HealthChanged => "health_changed",
            Self::DnsRecordChanged(check) => match check {
                CheckKind::Dmarc => "dns_record_changed.dmarc",
                CheckKind::Spf => "dns_record_changed.spf",
                CheckKind::Dkim => "dns_record_changed.dkim",
                CheckKind::MtaSts => "dns_record_changed.mta_sts",
                CheckKind::TlsRpt => "dns_record_changed.tls_rpt",
                CheckKind::Bimi => "dns_record_changed.bimi",
            },
            Self::PolicyChanged => "policy_changed",
            Self::ReadinessReached => "readiness_reached",
            Self::ParkedChanged { parked: true } => "parked_changed.parked",
            Self::ParkedChanged { parked: false } => "parked_changed.unparked",
            Self::NewFailingSender => "new_failing_sender",
        }
    }

    /// The group the settings show it in.
    pub fn group(self) -> AlertGroup {
        match self {
            Self::IssueOpened(_) | Self::IssueResolved(_) => AlertGroup::Issues,
            Self::HealthChanged => AlertGroup::Health,
            Self::DnsRecordChanged(_) => AlertGroup::DnsRecords,
            Self::PolicyChanged => AlertGroup::Policy,
            Self::ReadinessReached => AlertGroup::Readiness,
            Self::ParkedChanged { .. } => AlertGroup::Parked,
            Self::NewFailingSender => AlertGroup::Senders,
        }
    }
}

/// Every alert kind [`alert_events`](super::alert_events) raises, in the order the settings and
/// the docs list them. A test checks that every raised kind maps to exactly
/// one entry ([`catalogue_id`]) and no entry is unreachable.
pub static ALERT_CATALOGUE: [CatalogueKind; 16] = {
    use CatalogueKind::*;
    [
        IssueOpened(AlertedIssue::DeliveredFailures),
        IssueResolved(AlertedIssue::DeliveredFailures),
        IssueOpened(AlertedIssue::ReportsStopped),
        IssueResolved(AlertedIssue::ReportsStopped),
        HealthChanged,
        DnsRecordChanged(CheckKind::Dmarc),
        DnsRecordChanged(CheckKind::Spf),
        DnsRecordChanged(CheckKind::Dkim),
        DnsRecordChanged(CheckKind::MtaSts),
        DnsRecordChanged(CheckKind::TlsRpt),
        DnsRecordChanged(CheckKind::Bimi),
        PolicyChanged,
        ReadinessReached,
        ParkedChanged { parked: true },
        ParkedChanged { parked: false },
        NewFailingSender,
    ]
};

/// The catalogue id of `kind`: every kind has an entry in [`ALERT_CATALOGUE`].
pub fn catalogue_id(kind: &AlertKind) -> &'static str {
    CatalogueKind::of(kind).id()
}

/// The catalogue kind named `id`, if there is one.
pub fn catalogue_entry(id: &str) -> Option<CatalogueKind> {
    ALERT_CATALOGUE.iter().copied().find(|e| e.id() == id)
}

/// `events` ordered worst first (stable among equals), as the alert email
/// and the digest list them.
pub fn worst_first<T>(items: &mut [T], severity: impl Fn(&T) -> Status) {
    items.sort_by_key(|item| std::cmp::Reverse(severity(item).rank()));
}
