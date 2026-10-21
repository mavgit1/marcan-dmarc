//! The check for new versions (on by default), and its last answer.

use super::{get_setting, set_setting, SettingsGroup};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use ts_rs::TS;

/// Whether the server looks for a new version once a day. On until the
/// operator switches it off (a database without the row reads as on). The
/// API carries it unchanged, in both directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdateSettings {
    /// Fetch the newest version from the docs site once a day.
    pub check: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { check: true }
    }
}

impl SettingsGroup for UpdateSettings {
    const KEY: &'static str = "updates";
}

/// The job record of the version check: when it last asked, and the last
/// answer it got. Kept beside the settings as one JSON row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheck {
    /// When the check last asked (unix seconds), answered or not: the next
    /// one waits a day from here.
    pub attempted_at: i64,
    /// The last answer; `None` before the first. A failed check keeps it.
    pub seen: Option<SeenVersion>,
}

/// The newest version as the docs site last named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenVersion {
    /// "0.2.1".
    pub version: String,
    /// Where its release notes are.
    pub url: String,
    /// When this answer came (unix seconds).
    pub checked_at: i64,
}

const UPDATE_CHECK_KEY: &str = "update_check";

/// The stored record; `None` without one, or when it does not read (the
/// next check then asks again).
pub async fn get_update_check(pool: &SqlitePool) -> Result<Option<UpdateCheck>, StoreError> {
    Ok(get_setting(pool, UPDATE_CHECK_KEY)
        .await?
        .and_then(|raw| serde_json::from_str(&raw).ok()))
}

/// Store the record, replacing the last one.
pub async fn set_update_check(pool: &SqlitePool, check: &UpdateCheck) -> Result<(), StoreError> {
    set_setting(pool, UPDATE_CHECK_KEY, &serde_json::to_string(check)?).await
}

/// Forget the record (the check was switched off).
pub async fn clear_update_check(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM app_settings WHERE key = ?")
        .bind(UPDATE_CHECK_KEY)
        .execute(pool)
        .await?;
    Ok(())
}
