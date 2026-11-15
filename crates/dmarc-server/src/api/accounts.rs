//! The mail accounts: `POST /api/accounts` adds one, `PUT` and `DELETE
//! /api/accounts/{id}` change or remove it (each answering every setting),
//! `POST /api/accounts/test` tries the values on screen, and `POST
//! /api/accounts/{id}/sync` reads one account's mailbox now.
//!
//! A save takes the whole account ([`SaveAccountRequest`], the shape of
//! [`AccountView`](crate::view::accounts::AccountView) with the secret
//! beside it), normalised and checked by [`crate::accounts::save`] (the
//! connection test tries it normalised the same way, without the checks of
//! a complete form: [`test_connection`]). A stored password is kept only
//! while the servers and the username it was saved for are unchanged: a
//! changed host, port or username asks for it again, so whoever reaches the
//! UI cannot send it to a server of their choosing. When a save changes
//! which mailbox an account reads (its IMAP server and port and the user it
//! logs in as, a Microsoft login's tenant and address, and its folder:
//! [`receive_identity`]), its cursor, skipped messages, retry
//! counts and passes are forgotten and its receive epoch moves on in the
//! save's transaction: its running pass, which the worker stops, writes
//! nothing more, and the next one reads the mailbox from the start.
//!
//! Sending goes through one account (`sending.account_id`): while sending
//! is on, that account cannot be deleted or stop being able to send (a
//! password login losing its SMTP server); while it is off, deleting the
//! account clears the choice.
//!
//! A Microsoft login is checked and stored by `prepare_microsoft_save`
//! (GUIDs, a draft certificate from `POST /api/accounts/certificate` taken
//! once written, else the stored one) and tested with a fresh token, then
//! (while it receives) the mailbox read over Microsoft Graph; its sending
//! is tested by Settings → Sending's test message only.
//!
//! A receiving account's address (and the addresses forwarded to it) is
//! where reports are sent: the reporting setup reads them from the accounts.
//! Each of these addresses belongs to one account.

use crate::accounts::connection::test_connection;
use crate::accounts::save::{address_free, normalise, validate, SaveAccountRequest};
use crate::accounts::{receive_identity, sends_mail, SecretChange};
use crate::api::settings::{settings_response, SyncResponse, SyncSkipReason};
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path};
use crate::view::accounts::AccountTestResponse;
use crate::view::settings::SettingsResponse;
use crate::wording::accounts::{sending_needs_smtp, sending_uses};
use crate::AppState;
use axum::extract::State;
use dmarc_store::{
    create_account as insert_stored, delete_account as delete_stored, get_account, save_settings,
    unix_now, update_account_receiving as update_stored, AccountFields, AccountRow, AppSettings,
    ReceiveChange,
};
use serde::Deserialize;
use std::sync::Arc;
use ts_rs::TS;

/// `POST /api/accounts/test`: the account on screen (`account`), the stored
/// one it edits (`id`, whose secrets apply to its own servers only), the
/// folder to examine (default: the account's), and whether to log in to the
/// SMTP server too (a password login's; a Microsoft login has none to test).
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct AccountTestRequest {
    #[ts(optional)]
    #[serde(default)]
    pub id: Option<i64>,
    pub account: SaveAccountRequest,
    #[ts(optional)]
    #[serde(default)]
    pub folder: Option<String>,
    pub smtp: bool,
}

/// Whether the pass of an account must start over: the mailbox it reads
/// ([`receive_identity`]) or how it reaches it and logs in changed.
fn receive_changed(before: &AccountFields, after: &AccountFields, secrets_changed: bool) -> bool {
    receive_identity(before) != receive_identity(after)
        || before.imap_security != after.imap_security
        || before.login_kind != after.login_kind
        || before.microsoft != after.microsoft
        || before.receive_enabled != after.receive_enabled
        || secrets_changed
}

/// `POST /api/accounts`: add the account; answers every setting.
pub async fn create_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SaveAccountRequest>,
) -> AppResult<Json<SettingsResponse>> {
    let (mut fields, login) = normalise(&req);
    validate(&fields)?;
    address_free(&state.pool, &fields, None).await?;
    let (changes, draft) = state
        .accounts
        .secret_changes(None, &mut fields, &login, None)
        .await?;
    // Sealed with the new account id (it is in their AAD), in the insert's
    // transaction.
    let id = insert_stored(&state.pool, &fields, unix_now(), |id| {
        state.accounts.seal(id, &changes)
    })
    .await?;
    if let Some(draft) = draft {
        state.accounts.drafts().take(&draft);
    }
    state.mailboxes.settings_changed(&id.to_string());
    settings_response(&state).await
}

/// Account `id`, or a `404`.
async fn stored_account(state: &AppState, id: i64) -> AppResult<AccountRow> {
    get_account(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("account {id} not found")))
}

/// `PUT /api/accounts/{id}`: replace the account; answers every setting.
pub async fn update_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<SaveAccountRequest>,
) -> AppResult<Json<SettingsResponse>> {
    let saved = stored_account(&state, id).await?;
    let (mut fields, login) = normalise(&req);
    validate(&fields)?;
    address_free(&state.pool, &fields, Some(id)).await?;
    let sending = AppSettings::load(&state.pool).await?.sending;
    if sending.enabled && sending.account_id == Some(id) && !sends_mail(&fields) {
        return Err(AppError::conflict(sending_needs_smtp(&saved.fields.name)));
    }
    let (changes, draft) = state
        .accounts
        .secret_changes(Some(id), &mut fields, &login, Some(&saved.fields))
        .await?;
    let secrets_changed = changes
        .iter()
        .any(|(_, change)| matches!(change, SecretChange::Set(_)));
    let restart = receive_changed(&saved.fields, &fields, secrets_changed);
    let replaced = receive_identity(&saved.fields) != receive_identity(&fields);
    // Its token is fetched anew with what is saved now.
    state.accounts.forget_token(&saved).await;
    // Another mailbox: its receive state goes with the save, so a restart
    // right after it cannot leave the old cursor behind. Either way, or
    // with receiving switched on, the days it may go unread count anew.
    let now = unix_now();
    let receive = if replaced {
        ReceiveChange::Replaced { at: now }
    } else if fields.receive_enabled && !saved.fields.receive_enabled {
        ReceiveChange::Started { at: now }
    } else {
        ReceiveChange::Keep
    };
    let sealed = state.accounts.seal(id, &changes);
    update_stored(&state.pool, id, &fields, &sealed, receive).await?;
    if let Some(draft) = draft {
        state.accounts.drafts().take(&draft);
    }
    if restart {
        // Stop its running pass after the current batch and read it now
        // (another mailbox from its first message: the save forgot the old
        // one's state, and the stopped pass can write none back).
        state.mailboxes.settings_changed(&id.to_string());
    }
    settings_response(&state).await
}

/// `DELETE /api/accounts/{id}`: remove the account with its secrets and
/// receive state (its imported reports stay). Refused (`409`) while sending
/// is on through it; while sending is off, its choice is cleared.
pub async fn delete_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<SettingsResponse>> {
    let saved = stored_account(&state, id).await?;
    let mut sending = AppSettings::load(&state.pool).await?.sending;
    if sending.account_id == Some(id) {
        if sending.enabled {
            return Err(AppError::conflict(sending_uses(&saved.fields.name)));
        }
        sending.account_id = None;
        save_settings(&state.pool, &sending).await?;
    }
    state.accounts.forget_token(&saved).await;
    delete_stored(&state.pool, id).await?;
    // Its running pass stops; the list no longer has it.
    state.mailboxes.settings_changed(&id.to_string());
    settings_response(&state).await
}

/// `POST /api/accounts/test`: log in with the values on screen
/// ([`test_connection`]); one line per step. Nothing is stored. A test of a
/// stored account starts its passes' back-off over: someone is fixing it.
pub async fn test_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AccountTestRequest>,
) -> AppResult<Json<AccountTestResponse>> {
    let saved = match req.id {
        Some(id) => Some(stored_account(&state, id).await?),
        None => None,
    };
    if let Some(id) = req.id {
        state.mailboxes.reset_backoff(&id.to_string());
    }
    let answer = test_connection(
        &state,
        saved.as_ref(),
        &req.account,
        req.folder.as_deref(),
        req.smtp,
    )
    .await?;
    Ok(Json(answer))
}

/// `POST /api/accounts/{id}/sync`: read the account's mailbox now, or right
/// after the pass that is running; answers at once, like the sync of every
/// account. Progress and outcome show in `GET /api/system/health`.
pub async fn sync_account(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<SyncResponse>> {
    let account = stored_account(&state, id).await?;
    let snapshot = state.accounts.snapshot().await?;
    let name = &account.fields.name;
    if !snapshot.ready_to_receive(&account) {
        return Ok(Json(SyncResponse {
            started: false,
            reason: Some(SyncSkipReason::NotConfigured),
            message: format!(
                "{name} does not receive reports yet: switch on Receive reports and complete its login."
            ),
        }));
    }
    let running = state.mailboxes.progress().is_some();
    state.mailboxes.poll_now(&id.to_string());
    Ok(Json(if running {
        SyncResponse {
            started: false,
            reason: Some(SyncSkipReason::AlreadyRunning),
            message: format!("A sync is running; {name} is read right after it."),
        }
    } else {
        SyncResponse {
            started: true,
            reason: None,
            message: format!("Reading {name} now."),
        }
    }))
}
