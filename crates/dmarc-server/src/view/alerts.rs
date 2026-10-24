//! Alert events as `GET /api/alerts` lists them: the stored event (what
//! changed, its colour) with its domain and organization (none for an event
//! about the installation), and its words (`crate::wording::alerts`), the
//! same the alert email uses.

use crate::wording::alerts::{alert_line, status_word};
use dmarc_assess::{catalogue_id, AlertKind};
use dmarc_store::AlertEventRow;
use serde::Serialize;
use ts_rs::TS;

/// One event of the alert list.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AlertItem {
    #[serde(flatten)]
    #[ts(flatten)]
    pub row: AlertEventRow,
    /// The alert catalogue entry the event belongs to (`GET /api/meta`'s
    /// `alert_catalogue`): its group and trigger line, and what the alert
    /// settings mute.
    pub catalogue_id: &'static str,
    /// What changed in one sentence, naming the numbers the event carries:
    /// the sentence of the alert email.
    pub text: String,
    /// The event's colour (`event.severity`) in a word: "Problem",
    /// "Warning", "Good" or "Note".
    pub severity_label: &'static str,
    /// The source IP of the sender the event is about, whose sender page
    /// the line links to (a new failing sender); `null` for the others.
    pub sender_ip: Option<String>,
    /// The release notes the line links to (a newer release: the `url` of
    /// the version check's `latest.json`); `null` for the others.
    pub release_notes: Option<String>,
}

impl AlertItem {
    /// Whether the event is about the installation (a newer release), not a
    /// domain.
    pub fn is_installation(&self) -> bool {
        self.row.domain_id.is_none()
    }
}

impl From<AlertEventRow> for AlertItem {
    fn from(row: AlertEventRow) -> Self {
        Self {
            catalogue_id: catalogue_id(&row.event.kind),
            text: alert_line(&row.event.kind),
            severity_label: status_word(row.event.severity),
            sender_ip: match &row.event.kind {
                AlertKind::NewFailingSender { sender, .. } => Some(sender.source_ip.clone()),
                _ => None,
            },
            release_notes: match &row.event.kind {
                AlertKind::UpdateAvailable { url, .. } => Some(url.clone()),
                _ => None,
            },
            row,
        }
    }
}
