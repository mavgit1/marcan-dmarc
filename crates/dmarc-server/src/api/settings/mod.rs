//! Settings endpoints under `/api/settings`.
//!
//! Marcan's own settings are groups: `GET /api/settings` answers all of them
//! ([`SettingsResponse`]) and `PUT /api/settings/{group}` saves one (mailbox,
//! reporting, monitoring, retention, dns, sending, alerts, client-reports,
//! license, branding, onboarding),
//! plus the branding logo (`POST` / `DELETE /api/settings/branding/logo`),
//! answering all of them again, and the branding preview (`POST
//! /api/settings/branding/preview`, how the emails would look with a draft,
//! which stores nothing). The mailbox actions (connect, sync, re-import) live
//! with the mailbox, the test sends with sending and alerts, the rendered and
//! sent client report with the client reports; the DKIM selector list is data
//! with its own validation, not a group, edited like the other lists
//! (`crate::view::ListText`).
//!
//! Each group is written by exactly one Settings form (a form may write
//! several groups), so two forms never overwrite each other's values.
//!
//! Values live in the `app_settings` table, one row per group, read and
//! written through `dmarc_store::AppSettings` and its groups. The groups
//! without checks of their own (monitoring, retention, onboarding) are saved
//! here; each other group has its module.

mod alerts;
mod branding;
mod client_reports;
mod dns;
mod license;
mod mailbox;
mod reporting;
mod selectors;
mod sending;

pub use alerts::{put_alerts, send_test as send_test_alert};
pub use branding::{
    delete_logo, preview_branding, put_branding, upload_logo, LOGO_MAX_BYTES,
    OPERATOR_NAME_MAX_CHARS,
};
pub(crate) use client_reports::NO_RECIPIENTS as NO_CLIENT_REPORT_RECIPIENTS;
pub use client_reports::{
    put_client_reports, send_email as send_client_report_email,
    show_email as show_client_report_email,
};
pub use dns::put_dns;
pub use license::put_license;
pub use mailbox::{connect_imap, put_mailbox, reimport_imap, sync_imap};
pub use reporting::put_reporting;
pub use selectors::{get_dkim_selectors, preview_dkim_selectors, save_dkim_selectors};
pub use sending::{put_sending, send_test};

use crate::client_reports::job::schedule;
use crate::error::AppResult;
use crate::extract::Json;
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{
    save_settings, unix_now, AppSettings, MonitoringSettings, OnboardingSettings, RetentionSettings,
};
use std::sync::Arc;

/// Every setting, as stored now, and when the client reports go out.
async fn settings_response(state: &AppState) -> AppResult<Json<SettingsResponse>> {
    let settings = AppSettings::load(&state.pool).await?;
    let schedule = schedule(&state.pool, &settings, unix_now()).await?;
    let logo = dmarc_store::get_logo_info(&state.pool).await?;
    Ok(Json(SettingsResponse::new(&settings, &schedule, logo)))
}

/// `GET /api/settings`.
pub async fn get_settings(State(state): State<Arc<AppState>>) -> AppResult<Json<SettingsResponse>> {
    settings_response(&state).await
}

/// `PUT /api/settings/monitoring`: the report freshness warnings, stored
/// clamped to their ranges. They are also part of `GET /api/system/health`,
/// next to the freshness they judge.
pub async fn put_monitoring(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<MonitoringSettings>,
) -> AppResult<Json<SettingsResponse>> {
    save_settings(&state.pool, &payload.clamped()).await?;
    settings_response(&state).await
}

/// `PUT /api/settings/retention`: the months reports are kept, stored clamped
/// to one month – ten years. The next daily clean-up applies them.
pub async fn put_retention(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RetentionSettings>,
) -> AppResult<Json<SettingsResponse>> {
    save_settings(&state.pool, &payload.clamped()).await?;
    settings_response(&state).await
}

/// `PUT /api/settings/onboarding`: how the setup ended; `null` reopens it.
pub async fn put_onboarding(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<OnboardingSettings>,
) -> AppResult<Json<SettingsResponse>> {
    save_settings(&state.pool, &payload).await?;
    settings_response(&state).await
}
