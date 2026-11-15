//! Receiving: how often every receiving account is read (`PUT
//! /api/settings/receiving`), and the actions on every receiving account:
//! `/receiving/sync` and `/receiving/reimport`. One account's sync is
//! `POST /api/accounts/{id}/sync`.

use super::settings_response;
use crate::error::AppResult;
use crate::extract::Json;
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_mailbox::SyncStart;
use dmarc_store::{save_settings, ReceivingSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// `PUT /api/settings/receiving`: how often every receiving account is
/// read, stored clamped to ten seconds – one day. The worker re-reads its
/// list at once, so the new interval applies without interrupting a pass.
pub async fn put_receiving(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ReceivingSettings>,
) -> AppResult<Json<SettingsResponse>> {
    save_settings(&state.pool, &payload.clamped()).await?;
    state.mailboxes.wake();
    settings_response(&state).await
}

/// Result of `POST /api/settings/receiving/sync`. The pass runs in the background;
/// its progress and outcome show in `GET /api/system/health`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SyncResponse {
    pub started: bool,
    /// Why nothing was started.
    pub reason: Option<SyncSkipReason>,
    /// Plain-language summary for the UI.
    pub message: String,
}

/// Why `POST /api/settings/receiving/sync` did not start a pass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SyncSkipReason {
    AlreadyRunning,
    NotConfigured,
}

/// Ask the worker's loop for a pass of every receiving account, one after
/// the other, and return immediately. A pass that is already running is
/// left alone (and reported), never waited for; the sync follows it.
pub async fn sync_all(State(state): State<Arc<AppState>>) -> AppResult<Json<SyncResponse>> {
    Ok(sync_answer(
        state.mailboxes.start_sync(&state.mailbox_backend()).await?,
        "Sync started.",
        "A sync is already running.",
    ))
}

/// `POST /api/settings/receiving/reimport`: read every receiving account's folder
/// again and add the reports that are missing (already stored ones are skipped). Answers like a
/// sync; while a pass runs, the re-import is queued and starts right after it.
pub async fn reimport_all(State(state): State<Arc<AppState>>) -> AppResult<Json<SyncResponse>> {
    Ok(sync_answer(
        state
            .mailboxes
            .start_reimport(&state.mailbox_backend())
            .await?,
        "Re-import started.",
        "A sync is running; the re-import starts as soon as it finishes.",
    ))
}

fn sync_answer(start: SyncStart, started: &str, running: &str) -> Json<SyncResponse> {
    let (started, reason, message) = match start {
        SyncStart::Started => (true, None, started),
        SyncStart::AlreadyRunning => (false, Some(SyncSkipReason::AlreadyRunning), running),
        SyncStart::Unconfigured => (
            false,
            Some(SyncSkipReason::NotConfigured),
            "No account receives reports yet. Add one under Settings → Accounts first.",
        ),
    };
    Json(SyncResponse {
        started,
        reason,
        message: message.to_string(),
    })
}
