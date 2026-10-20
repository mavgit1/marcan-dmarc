//! The monthly client reports, and the job's own record beside them.

use super::{get_time, set_time, SettingsGroup};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use ts_rs::TS;

/// When the client reports were last held (unix seconds): the job's own
/// record, which keeps a held month's report due after its day.
const CLIENT_REPORTS_HELD_AT_KEY: &str = "client_reports_held_at";

/// [`ClientReportSettings::day_of_month`] when nothing is stored.
pub const DEFAULT_CLIENT_REPORT_DAY: u8 = 1;
/// [`ClientReportSettings::hour_utc`] when nothing is stored (07:00 UTC).
pub const DEFAULT_CLIENT_REPORT_HOUR: u8 = 7;
/// The days of the month a report can go out on: every month has them.
pub const CLIENT_REPORT_DAY_RANGE: (u8, u8) = (1, 28);

/// The monthly client reports, as stored and as `PUT
/// /api/settings/client-reports` takes them. Off by default; who gets them
/// is the client report recipients list (`client_report_recipients`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ClientReportSettings {
    /// Each listed organization gets its report once a month (sending must be
    /// on too). Only with at least one recipient in the list.
    pub enabled: bool,
    /// 1–28.
    pub day_of_month: u8,
    /// 0–23, UTC.
    pub hour_utc: u8,
    /// Where replies to the reports go. Empty: the sending group's Reply-To.
    pub reply_to: String,
}

impl SettingsGroup for ClientReportSettings {
    const KEY: &'static str = "client_reports";
}

impl Default for ClientReportSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            day_of_month: DEFAULT_CLIENT_REPORT_DAY,
            hour_utc: DEFAULT_CLIENT_REPORT_HOUR,
            reply_to: String::new(),
        }
    }
}

/// When the client reports were last held; `None` if never.
pub async fn get_client_reports_held_at(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    get_time(pool, CLIENT_REPORTS_HELD_AT_KEY).await
}

/// Record that the client reports were held at `at`.
pub async fn set_client_reports_held_at(pool: &SqlitePool, at: i64) -> Result<(), StoreError> {
    set_time(pool, CLIENT_REPORTS_HELD_AT_KEY, at).await
}
