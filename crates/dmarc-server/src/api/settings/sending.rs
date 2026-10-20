//! The sending group (`PUT /api/settings/sending`) and the test send
//! (`POST /api/settings/sending/test`).
//!
//! Anyone who reaches Marcan can make it send mail as the sender address
//! (Marcan has no login of its own; it sits behind the proxy's), as they can
//! change the settings. So the test send takes the account on screen (the
//! test recipient with it), nothing else: the subject and the text are fixed, and a stored
//! password is only used for the server and username it was saved for (the
//! mailbox's only for the mailbox's provider), as when saving.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::mail::{self, parse_address, Address, Message, SendError, SmtpConfig};
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{save_settings, AppSettings, SendingGroup, SendingSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;
use ts_rs::TS;

/// Returned when sending is switched on without a server; the Save bar says
/// the same.
const NO_SERVER: &str = "Enter a server first.";

/// Returned when a test or preview has nowhere to go: no test recipient and
/// no sender address. The buttons say the same.
const NO_TEST_RECIPIENT: &str = "Set a test recipient under Sending first.";

/// Returned when a request would send the stored password to another account.
const PASSWORD_REQUIRED: &str =
    "Enter the SMTP password: the stored one is only used for the server and username it was saved for.";

const TEST_SUBJECT: &str = "Marcan test message";
const TEST_TEXT: &str = "Marcan can send mail from this address.\r\n\r\nThis message was sent from Settings → Sending (“Send a test”).\r\n";

/// `PUT /api/settings/sending`: the whole group ([`SendingGroup`], as the
/// settings answer it) and the password, the one value it does not answer.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SaveSendingRequest {
    #[serde(flatten)]
    #[ts(flatten)]
    pub group: SendingGroup,
    /// The SMTP account's own password, as typed. Absent or blank: keep the
    /// stored one (a secret is never answered, so it is not sent back),
    /// which is only allowed for the server and username it was saved for.
    #[ts(optional)]
    pub password: Option<String>,
}

/// `POST /api/settings/sending/test`: to the test recipient of the group sent.
#[derive(Debug, Deserialize, Default, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SendTestRequest {
    /// The group as on screen, in the shape of its `PUT` (its `enabled` is
    /// ignored: a test runs while sending is off). An absent password means
    /// the stored one, for the server and username it was saved for only.
    /// Absent: the stored group, which must be on.
    pub sending: Option<SaveSendingRequest>,
}

/// A test message was handed to the server. A failure is an error answer
/// instead: `409` while sending is off or incomplete, `422` when the server
/// could not be reached or refused, each with the plain-language `error`, the
/// failed step as `reason` (`resolve`, `connect`, `tls`,
/// `starttls_not_offered`, `auth`, `rejected`, …) and the raw `detail`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SendTestResponse {
    pub ok: bool,
    pub message: String,
}

/// Trimmed, non-empty request value.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// An address the request gave, or a `422` that names `what`.
fn valid_address(raw: &str, what: &str) -> AppResult<()> {
    if parse_address(raw).is_none() {
        return Err(AppError::unprocessable(format!(
            "The {what} {raw} is not a valid email address."
        )));
    }
    Ok(())
}

/// The group `payload` asks for over the stored `settings`, validated, with
/// the password resolved: typed, else the stored one for the same account.
fn requested_sending(
    settings: &AppSettings,
    payload: &SaveSendingRequest,
) -> AppResult<SendingSettings> {
    let group = &payload.group;
    let host = group.host.trim();
    let port = match group.port {
        None => None,
        Some(0) => {
            return Err(AppError::unprocessable(
                "The SMTP port must be between 1 and 65535.",
            ))
        }
        Some(port) => {
            mail::check_port(port).map_err(|e| AppError::unprocessable(e.message()))?;
            Some(port)
        }
    };
    let username = group.username.trim();
    let sender = group.sender.trim();
    if !sender.is_empty() {
        valid_address(sender, "sender address")?;
    }
    let reply_to = non_empty(Some(&group.reply_to));
    if let Some(reply_to) = reply_to {
        valid_address(reply_to, "Reply-To address")?;
    }
    let test_recipient = group.test_recipient.trim();
    if !test_recipient.is_empty() {
        valid_address(test_recipient, "test recipient")?;
    }

    let stored = &settings.sending;
    let same_account = stored.group.host.eq_ignore_ascii_case(host)
        && stored.group.port == port
        && stored.group.username == username;
    let password = match payload.password.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(password) => password.to_owned(),
        None if same_account || stored.password.is_empty() => stored.password.clone(),
        // The stored password belongs to another account: needed, it must be
        // typed again; not needed (the same login), it is dropped.
        None if group.enabled && !group.same_login_as_mailbox => {
            return Err(AppError::conflict(PASSWORD_REQUIRED))
        }
        None => String::new(),
    };

    Ok(SendingSettings {
        group: SendingGroup {
            host: host.to_owned(),
            port,
            username: username.to_owned(),
            sender: sender.to_owned(),
            reply_to: reply_to.unwrap_or_default().to_owned(),
            test_recipient: test_recipient.to_owned(),
            ..group.clone()
        },
        password,
    })
}

/// Where tests and previews go ([`AppSettings::test_recipient`]): `409`
/// when there is no test recipient and no sender address.
pub(super) fn test_recipient(settings: &AppSettings) -> AppResult<Address> {
    let raw = settings.test_recipient();
    if raw.is_empty() {
        return Err(AppError::conflict(NO_TEST_RECIPIENT));
    }
    parse_address(&raw).ok_or_else(|| {
        AppError::unprocessable(format!(
            "The test recipient {raw} is not a valid email address."
        ))
    })
}

/// Store the group; an enabled one must be ready to send.
pub(super) async fn save_sending(state: &AppState, payload: SaveSendingRequest) -> AppResult<()> {
    let settings = AppSettings::load(&state.pool).await?;
    let sending = requested_sending(&settings, &payload)?;
    if sending.group.enabled && sending.group.host.is_empty() {
        return Err(AppError::conflict(NO_SERVER));
    }
    if sending.group.enabled {
        let candidate = AppSettings {
            sending: sending.clone(),
            ..settings
        };
        SmtpConfig::from_settings(&candidate).map_err(|e| AppError::conflict(e.message()))?;
    }
    save_settings(&state.pool, &sending).await?;
    Ok(())
}

/// `PUT /api/settings/sending`; answers every setting.
pub async fn put_sending(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SaveSendingRequest>,
) -> AppResult<Json<SettingsResponse>> {
    save_sending(&state, payload).await?;
    settings_response(&state).await
}

/// The error answer of a failed test send.
pub(super) fn send_failed(error: &SendError) -> AppError {
    warn!(reason = error.reason(), error = %error, "test send failed");
    let base = match error {
        SendError::Disabled | SendError::Incomplete(_) => AppError::conflict(error.message()),
        _ => AppError::unprocessable(error.message()),
    };
    base.with_send_failure(error.reason(), error.detail())
}

/// Send the fixed test message with the account on screen (`sending`), else
/// the stored one, to its test recipient (default: the sender itself).
pub async fn send_test(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendTestRequest>,
) -> AppResult<Json<SendTestResponse>> {
    let mut settings = AppSettings::load(&state.pool).await?;
    if let Some(draft) = &req.sending {
        let mut draft = draft.clone();
        draft.group.enabled = true;
        settings.sending = requested_sending(&settings, &draft)?;
    }
    let config = SmtpConfig::from_settings(&settings).map_err(|e| send_failed(&e))?;
    let to = test_recipient(&settings)?;
    let message = Message {
        to: to.clone(),
        subject: TEST_SUBJECT.to_owned(),
        text: TEST_TEXT.to_owned(),
        html: None,
        images: Vec::new(),
    };
    state
        .mail
        .send(&config, &message)
        .await
        .map_err(|e| send_failed(&e))?;
    Ok(Json(SendTestResponse {
        ok: true,
        message: format!("Test message sent to {to}."),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn tests_go_to_the_test_recipient_else_the_sender_else_nowhere() {
        let mut settings = AppSettings::default();
        settings.sending.group.same_login_as_mailbox = false;
        let err = test_recipient(&settings).unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.message, "Set a test recipient under Sending first.");

        settings.sending.group.sender = "dmarc@example.com".into();
        assert_eq!(
            test_recipient(&settings).unwrap().to_string(),
            "dmarc@example.com"
        );
        settings.sending.group.test_recipient = "me@example.org".into();
        assert_eq!(
            test_recipient(&settings).unwrap().to_string(),
            "me@example.org"
        );
    }
}
