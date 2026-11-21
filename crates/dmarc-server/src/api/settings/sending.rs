//! The sending group (`PUT /api/settings/sending`): whether Marcan sends,
//! the account it sends through and from (one that can send, from Settings
//! → Accounts: an SMTP server, or a Microsoft login), Reply-To and the test
//! recipient; and the test send (`POST /api/settings/sending/test`).
//!
//! Anyone who reaches Marcan can make it send mail as the account's address
//! (Marcan has no login of its own; it sits behind the proxy's), as they can
//! change the settings. So the test send takes the group on screen (the
//! account by its id, with its stored login), nothing else: the subject and
//! the text are fixed.

use super::settings_response;
use crate::accounts::save::valid_address;
use crate::accounts::{self, sends_mail};
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::mail::{parse_address, parse_reply_to, Address, Message, SendConfig, SendError};
use crate::view::settings::SettingsResponse;
use crate::wording::accounts::NO_SENDING_ACCOUNT;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{get_account, AppSettings, SendingSettings};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;
use ts_rs::TS;

const TEST_SUBJECT: &str = "Marcan test message";
const TEST_TEXT: &str = "Marcan can send mail from this address.\r\n\r\nThis message was sent from Settings → Sending (“Send a test”).\r\n";

/// `POST /api/settings/sending/test`: to the test recipient of the group sent.
#[derive(Debug, Deserialize, Default, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SendTestRequest {
    /// The group as on screen, in the shape of its `PUT` (its `enabled` is
    /// ignored: a test runs while sending is off). Absent: the stored
    /// group, which must be on.
    pub sending: Option<SendingSettings>,
}

/// A test message was handed to the server. A failure is an error answer
/// instead: `409` while sending is off, incomplete or locked, `422` when the
/// server could not be reached or refused, each with the plain-language
/// `error`, the failed step as `reason` (`resolve`, `connect`, `tls`,
/// `starttls_not_offered`, `auth`, `rejected`, …) and the raw `detail`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SendTestResponse {
    pub ok: bool,
    pub message: String,
}

/// The group `payload` asks for, trimmed and checked: valid addresses, and
/// an account that exists and can send ([`sends_mail`]).
async fn checked(state: &AppState, payload: &SendingSettings) -> AppResult<SendingSettings> {
    let group = SendingSettings {
        enabled: payload.enabled,
        account_id: payload.account_id,
        reply_to: payload.reply_to.trim().to_owned(),
        test_recipient: payload.test_recipient.trim().to_owned(),
    };
    parse_reply_to(&group.reply_to).map_err(AppError::unprocessable)?;
    if !group.test_recipient.is_empty() {
        valid_address(&group.test_recipient, "test recipient")?;
    }
    if let Some(id) = group.account_id {
        let account = get_account(&state.pool, id)
            .await?
            .ok_or_else(|| AppError::unprocessable(format!("There is no account {id}.")))?;
        if !sends_mail(&account.fields) {
            return Err(AppError::unprocessable(format!(
                "{} has no SMTP server. Add one under Accounts.",
                account.fields.name
            )));
        }
    }
    Ok(group)
}

/// Where tests and previews go ([`accounts::test_recipient`]), from the
/// address `config` sends from.
pub(super) fn test_recipient(sending: &SendingSettings, config: &SendConfig) -> AppResult<Address> {
    let sender = config.sender.to_string();
    let raw = accounts::test_recipient(sending, &sender);
    parse_address(raw).ok_or_else(|| {
        AppError::unprocessable(format!(
            "The test recipient {raw} is not a valid email address."
        ))
    })
}

/// `PUT /api/settings/sending`: the whole group; switched on, its account
/// must be ready to send (`409` with what is missing). Answers every setting.
pub async fn put_sending(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SendingSettings>,
) -> AppResult<Json<SettingsResponse>> {
    let group = checked(&state, &payload).await?;
    if group.enabled {
        if group.account_id.is_none() {
            return Err(AppError::conflict(NO_SENDING_ACCOUNT));
        }
        let settings = AppSettings::load(&state.pool).await?;
        state
            .accounts
            .sending_config(&settings, &group)
            .await
            .map_err(|e| AppError::conflict(e.message()))?;
    }
    dmarc_store::save_settings(&state.pool, &group).await?;
    settings_response(&state).await
}

/// The error answer of a failed test send.
pub(super) fn send_failed(error: &SendError) -> AppError {
    warn!(reason = error.reason(), error = %error, "test send failed");
    let base = match error {
        SendError::Disabled | SendError::Incomplete(_) | SendError::Locked { .. } => {
            AppError::conflict(error.message())
        }
        _ => AppError::unprocessable(error.message()),
    };
    base.with_send_failure(error.reason(), error.detail())
        .with_docs(error.docs())
}

/// Send the fixed test message with the group on screen (`sending`), else
/// the stored one, to its test recipient (default: the account's address).
pub async fn send_test(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendTestRequest>,
) -> AppResult<Json<SendTestResponse>> {
    let settings = AppSettings::load(&state.pool).await?;
    let (config, to) = match &req.sending {
        Some(draft) => {
            let group = checked(&state, draft).await?;
            let config = state
                .accounts
                .sending_config(&settings, &group)
                .await
                .map_err(|e| send_failed(&e))?;
            let to = test_recipient(&group, &config)?;
            (config, to)
        }
        None => {
            let config = state
                .accounts
                .send_config(&settings)
                .await
                .map_err(|e| send_failed(&e))?;
            let to = test_recipient(&settings.sending, &config)?;
            (config, to)
        }
    };
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
    fn tests_go_to_the_test_recipient_else_the_account_s_address() {
        let config = crate::accounts::tests::config();
        let mut sending = SendingSettings::default();
        assert_eq!(
            test_recipient(&sending, &config).unwrap().to_string(),
            "dmarc@example.com"
        );
        sending.test_recipient = "me@example.org".into();
        assert_eq!(
            test_recipient(&sending, &config).unwrap().to_string(),
            "me@example.org"
        );
        sending.test_recipient = "me".into();
        let err = test_recipient(&sending, &config).unwrap_err();
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}
