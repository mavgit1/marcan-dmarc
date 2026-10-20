//! Alert emails and the digest, and the alert job's own records beside them.

use super::{get_time, set_time, SettingsGroup};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use ts_rs::TS;

/// When the last digest went out (unix seconds). Not a setting: the digest
/// job's own record, kept beside the settings it follows.
const LAST_DIGEST_AT_KEY: &str = "alerts_last_digest_at";
/// When the alert job last held its evaluation back for a mailbox import
/// (unix seconds): the job's own record, which the next evaluation reads to
/// re-baseline the domains the import touched, also after a restart.
const ALERTS_HELD_AT_KEY: &str = "alerts_held_at";

/// [`AlertSettings::digest_hour`] when nothing is stored (07:00 UTC).
pub const DEFAULT_DIGEST_HOUR: u8 = 7;

/// How often the digest email goes out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum DigestFrequency {
    #[default]
    Off,
    Daily,
    Weekly,
}

/// A day of the week, Monday first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    #[default]
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// Days after Monday (Monday 0, Sunday 6).
    pub fn days_from_monday(self) -> u32 {
        self as u32
    }
}

/// Alert emails and the digest, as stored and as the API carries them (`PUT
/// /api/settings/alerts` and the settings' `alerts`). Off by default. The
/// server checks and cleans a group before it is saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AlertSettings {
    /// Alert emails go out: one when something changes, and the digest when
    /// it is set (sending must be on too). Off, neither does.
    pub enabled: bool,
    /// Email addresses; blanks and repeats are dropped when saved. At least
    /// one while alert emails are on.
    pub recipients: Vec<String>,
    pub digest: DigestFrequency,
    /// 0–23, UTC.
    pub digest_hour: u8,
    /// The weekly digest's day.
    pub digest_weekday: Weekday,
    /// Where Marcan is reached, for links in the emails: `http://` or
    /// `https://` and a host, saved without a trailing slash. Empty: not
    /// set, no links.
    pub base_url: String,
    /// Ids of `GET /api/meta`'s `alert_catalogue` the emails leave out (the
    /// list in the UI shows every alert), saved sorted, each once. Empty: all
    /// are mailed, so a kind added later arrives on.
    pub muted_kinds: Vec<String>,
}

impl SettingsGroup for AlertSettings {
    const KEY: &'static str = "alerts";
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            recipients: Vec::new(),
            digest: DigestFrequency::Off,
            digest_hour: DEFAULT_DIGEST_HOUR,
            digest_weekday: Weekday::Monday,
            base_url: String::new(),
            muted_kinds: Vec::new(),
        }
    }
}

impl AlertSettings {
    /// The base URL for links; `None` when not set.
    pub fn base_url(&self) -> Option<&str> {
        Some(self.base_url.as_str()).filter(|u| !u.is_empty())
    }
}

/// When the last digest went out; `None` before the first.
pub async fn get_last_digest_at(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    get_time(pool, LAST_DIGEST_AT_KEY).await
}

/// Record that a digest went out at `at`.
pub async fn set_last_digest_at(pool: &SqlitePool, at: i64) -> Result<(), StoreError> {
    set_time(pool, LAST_DIGEST_AT_KEY, at).await
}

/// When the alert evaluation was last held back for an import; `None` if never.
pub async fn get_alerts_held_at(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    get_time(pool, ALERTS_HELD_AT_KEY).await
}

/// Record that the alert evaluation was held back at `at`.
pub async fn set_alerts_held_at(pool: &SqlitePool, at: i64) -> Result<(), StoreError> {
    set_time(pool, ALERTS_HELD_AT_KEY, at).await
}
