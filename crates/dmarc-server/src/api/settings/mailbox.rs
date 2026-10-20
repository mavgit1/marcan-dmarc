//! The mailbox group (`PUT /api/settings/mailbox`) and the mailbox actions:
//! `POST /api/settings/imap/connect` (log in and list folders), `/imap/sync`
//! and `/imap/reimport`.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::imap_backend::{imap_security, stored_imap_config};
use crate::view::settings::{MailboxGroup, SettingsResponse};
use crate::AppState;
use axum::extract::State;
use dmarc_imap::{connect_mailbox, ImapConfig, SyncStart, DEFAULT_FOLDER};
use dmarc_store::{save_settings, AppSettings, ImapSecurity, MailboxSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// Shortest accepted poll interval, so a typo cannot hammer the mail server.
const MIN_POLL_INTERVAL_SECS: u64 = 10;

/// Returned when a request would send the stored password to a different mailbox.
const PASSWORD_REQUIRED: &str =
    "password is required when changing host, port or username: the stored password is only used for the mailbox it was saved for";

/// `PUT /api/settings/mailbox`: the whole mailbox ([`MailboxGroup`], as the
/// settings answer it) and the password, the one value it does not answer.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct SaveMailboxRequest {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: MailboxGroup,
    /// The password, as typed. Absent or blank: keep the stored one (a
    /// secret is never answered, so it is not sent back), which is only
    /// allowed for the mailbox it was saved for.
    #[ts(optional)]
    pub password: Option<String>,
}

#[derive(Debug, Deserialize, Default, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct ConnectImapRequest {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Defaults to the stored security.
    pub security: Option<ImapSecurity>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Folder to examine for its message count; defaults to `INBOX`.
    pub folder: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ImapFolderResponse {
    pub name: String,
    pub delimiter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SelectedFolderResponse {
    pub name: String,
    /// The folder could be opened (read-only `EXAMINE`).
    pub exists: bool,
    /// Message count; `null` when the folder does not exist.
    pub messages: Option<u32>,
}

/// Result of `POST /api/settings/imap/connect`. On failure `message` is a
/// plain-language explanation and `detail` the raw error (password masked).
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ConnectImapResponse {
    pub ok: bool,
    pub message: String,
    pub detail: Option<String>,
    pub folders: Vec<ImapFolderResponse>,
    pub selected: Option<SelectedFolderResponse>,
}

impl ConnectImapResponse {
    fn failed(message: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            detail,
            folders: Vec::new(),
            selected: None,
        }
    }
}

/// Trimmed, non-empty request value.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// A supplied password, exactly as typed: leading or trailing spaces can be part
/// of a password. Only an all-blank value counts as "not supplied".
fn supplied_password(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

/// The password to use for `host`/`port`/`username`: the supplied one, else
/// the stored one, but only for the mailbox it was saved for.
///
/// Reusing it for any other target would let whoever reaches the UI send the
/// stored password to a server of their choosing.
fn resolve_password(
    stored: &ImapConfig,
    supplied: Option<&str>,
    host: &str,
    port: u16,
    username: &str,
) -> AppResult<String> {
    if let Some(password) = supplied {
        return Ok(password.to_string());
    }
    let same_mailbox = stored.host.eq_ignore_ascii_case(host)
        && stored.port == port
        && stored.username == username;
    if same_mailbox || stored.password.is_empty() {
        Ok(stored.password.clone())
    } else {
        Err(AppError::conflict(PASSWORD_REQUIRED))
    }
}

/// Store the mailbox (one transaction: the fetch loop never reads a
/// half-saved mailbox) and wake the fetch loop.
pub(super) async fn save_mailbox(state: &AppState, payload: SaveMailboxRequest) -> AppResult<()> {
    let group = &payload.group;
    let host = group.host.trim();
    if host.is_empty() {
        return Err(AppError::unprocessable("host cannot be empty"));
    }
    if group.port == 0 {
        return Err(AppError::unprocessable("port must be between 1 and 65535"));
    }
    let username = group.username.trim();
    if username.is_empty() {
        return Err(AppError::unprocessable("username cannot be empty"));
    }
    let poll_interval = group.poll_interval_secs;
    if poll_interval < MIN_POLL_INTERVAL_SECS {
        return Err(AppError::unprocessable(format!(
            "poll interval must be at least {MIN_POLL_INTERVAL_SECS} seconds"
        )));
    }
    let folder = non_empty(Some(&group.folder)).unwrap_or(DEFAULT_FOLDER);

    let saved = AppSettings::load(&state.pool).await?.mailbox;
    let stored = stored_imap_config(&saved);
    let stored_security = saved.security.unwrap_or_default();
    let security = group.security;
    let port = group.port;
    let password = resolve_password(
        &stored,
        supplied_password(payload.password.as_deref()),
        host,
        port,
        username,
    )?;

    let mailbox_changed = !stored.host.eq_ignore_ascii_case(host)
        || stored.port != port
        || stored_security != security
        || stored.username != username
        || stored.password != password
        || stored.folder != folder;
    let interval_changed = stored.poll_interval_secs != poll_interval;
    save_settings(
        &state.pool,
        &MailboxSettings {
            host: host.to_owned(),
            port: Some(port),
            security: Some(security),
            username: username.to_owned(),
            password,
            folder: Some(folder.to_owned()),
            poll_interval_secs: Some(poll_interval),
        },
    )
    .await?;

    if mailbox_changed {
        // Stop a running pass after its current batch and poll the new mailbox
        // now rather than after the current sleep.
        state.imap.settings_changed();
    } else if interval_changed {
        // Apply the new interval without interrupting a running import.
        state.imap.wake();
    }
    Ok(())
}

/// `PUT /api/settings/mailbox`; answers every setting.
pub async fn put_mailbox(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SaveMailboxRequest>,
) -> AppResult<Json<SettingsResponse>> {
    save_mailbox(&state, payload).await?;
    settings_response(&state).await
}

/// Log in with the request's parameters (falling back to the stored ones), list
/// the selectable folders and examine `folder` (default `INBOX`) read-only.
/// The stored password is only used against the stored mailbox.
pub async fn connect_imap(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ConnectImapRequest>,
) -> AppResult<Json<ConnectImapResponse>> {
    let failed = |message: &str| Ok(Json(ConnectImapResponse::failed(message, None)));

    let stored = stored_imap_config(&AppSettings::load(&state.pool).await?.mailbox);
    let host = non_empty(req.host.as_deref()).unwrap_or(&stored.host);
    let port = req.port.unwrap_or(stored.port);
    let security = req.security.map_or(stored.security, imap_security);
    let username = non_empty(req.username.as_deref()).unwrap_or(&stored.username);
    if host.is_empty() {
        return failed("Enter the IMAP server host name.");
    }
    if username.is_empty() {
        return failed("Enter the username.");
    }
    let password = resolve_password(
        &stored,
        supplied_password(req.password.as_deref()),
        host,
        port,
        username,
    )?;
    if password.is_empty() {
        return failed("Enter the password.");
    }

    let cfg = ImapConfig {
        host: host.to_string(),
        port,
        security,
        username: username.to_string(),
        password,
        folder: non_empty(req.folder.as_deref())
            .unwrap_or(DEFAULT_FOLDER)
            .to_string(),
        poll_interval_secs: stored.poll_interval_secs,
    };

    Ok(Json(match connect_mailbox(&cfg).await {
        Ok(probe) => ConnectImapResponse {
            ok: true,
            message: format!("Connected as {}.", cfg.username),
            detail: None,
            folders: probe
                .folders
                .into_iter()
                .map(|f| ImapFolderResponse {
                    name: f.name,
                    delimiter: f.delimiter,
                })
                .collect(),
            selected: Some(SelectedFolderResponse {
                name: probe.selected.name,
                exists: probe.selected.exists,
                messages: probe.selected.messages,
            }),
        },
        Err(e) => ConnectImapResponse::failed(e.message, Some(e.detail).filter(|d| !d.is_empty())),
    }))
}

/// Result of `POST /api/settings/imap/sync`. The pass runs in the background;
/// its progress and outcome show in `GET /api/system/health`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SyncImapResponse {
    pub started: bool,
    /// Why nothing was started.
    pub reason: Option<SyncSkipReason>,
    /// Plain-language summary for the UI.
    pub message: String,
}

/// Why `POST /api/settings/imap/sync` did not start a pass.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SyncSkipReason {
    AlreadyRunning,
    NotConfigured,
}

/// Start an IMAP pass with the stored settings and return immediately. A pass
/// that is already running is left alone (and reported), never waited for.
pub async fn sync_imap(State(state): State<Arc<AppState>>) -> AppResult<Json<SyncImapResponse>> {
    Ok(sync_answer(
        state.imap.start_sync(&state.imap_backend()).await?,
        "Mailbox sync started.",
        "A mailbox sync is already running.",
    ))
}

/// `POST /api/settings/imap/reimport`: read the whole folder again and add the
/// reports that are missing (already stored ones are skipped). Answers like a
/// sync; while a pass runs, the re-import is queued and starts right after it.
pub async fn reimport_imap(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<SyncImapResponse>> {
    Ok(sync_answer(
        state.imap.start_reimport(&state.imap_backend()).await?,
        "Re-import started.",
        "A mailbox sync is running; the re-import starts as soon as it finishes.",
    ))
}

fn sync_answer(start: SyncStart, started: &str, running: &str) -> Json<SyncImapResponse> {
    let (started, reason, message) = match start {
        SyncStart::Started => (true, None, started),
        SyncStart::AlreadyRunning => (false, Some(SyncSkipReason::AlreadyRunning), running),
        SyncStart::Unconfigured => (
            false,
            Some(SyncSkipReason::NotConfigured),
            "The mailbox is not set up. Enter host, username and password under Settings → Mailbox first.",
        ),
    };
    Json(SyncImapResponse {
        started,
        reason,
        message: message.to_string(),
    })
}
