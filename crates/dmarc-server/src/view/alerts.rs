//! Alert events as `GET /api/alerts` lists them: the stored event (what
//! changed, its colour) with its domain and organization, and its words
//! (`crate::wording::alerts`), the same the alert email uses.

use crate::wording::alerts::{alert_line, status_word};
use dmarc_assess::catalogue_id;
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
}

impl From<AlertEventRow> for AlertItem {
    fn from(row: AlertEventRow) -> Self {
        Self {
            catalogue_id: catalogue_id(&row.event.kind),
            text: alert_line(&row.event.kind),
            severity_label: status_word(row.event.severity),
            row,
        }
    }
}
