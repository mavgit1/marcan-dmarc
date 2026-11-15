//! An account as the form sends it ([`SaveAccountRequest`]), turned into
//! what a save stores and a connection test tries: [`normalise`] (trimmed,
//! the name, ports and folder defaulted) for both, then [`validate`] for a
//! save only. A test tolerates an incomplete form (no name, no address
//! while receiving, a missing server or username: its steps say what is
//! missing) and nothing else differs, so it tries exactly what a save of
//! the same form would store.
//!
//! Then what the save does to the secrets ([`Accounts::secret_changes`]):
//! a password typed, kept (only for the servers and username it was saved
//! for, [`credentials::reusable_for_all`]) or asked for again, and a
//! Microsoft login's certificate ([`Accounts::prepare_microsoft_save`]).
//!
//! A Microsoft login has no servers: whatever the request holds for them,
//! it is stored without any (it reads and sends over Microsoft Graph).

use super::credentials::{self, resolve, Resolved};
use super::microsoft::MicrosoftLoginRequest;
use super::presets::{preset, preset_of, ProviderOption};
use super::{Accounts, SecretChange, SecretState};
use crate::error::{AppError, AppResult};
use crate::mail::{check_port, parse_address};
use crate::view::accounts::{AccountReceivingView, ImapServerView, SmtpServerView};
use crate::wording::accounts::{
    ENTER_ADDRESS, ENTER_IMAP_HOST, ENTER_NAME, ENTER_PASSWORD, ENTER_USERNAME,
    MICROSOFT_LOGIN_ONLY, NO_SERVER, PASSWORD_AGAIN,
};
use dmarc_mailbox::DEFAULT_FOLDER;
use dmarc_store::{account_with_address, AccountFields, LoginKind, MicrosoftLogin, SecretKind};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::fmt;
use ts_rs::TS;

/// `POST /api/accounts`, `PUT /api/accounts/{id}`: the whole account as the
/// settings answer it, with its secret beside the login.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
pub struct SaveAccountRequest {
    /// Empty: the address.
    pub name: String,
    /// The Provider chosen; stored as its preset's provider (Google as
    /// `generic`).
    pub provider: ProviderOption,
    /// The report address and default From. Needed while it receives.
    pub address: String,
    pub login: SaveLoginRequest,
    /// A password login's servers; `null` (and ignored) for a Microsoft
    /// login, which has none.
    #[serde(default)]
    pub imap: Option<ImapServerView>,
    #[serde(default)]
    pub smtp: Option<SmtpServerView>,
    pub receiving: AccountReceivingView,
}

/// How the account logs in, with the secret as typed.
#[derive(Clone, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SaveLoginRequest {
    Password {
        username: String,
        /// As typed. Absent or blank: keep the stored one, which is only
        /// allowed for the servers and username it was saved for.
        #[ts(optional)]
        #[serde(default)]
        password: Option<String>,
    },
    Microsoft {
        tenant_id: String,
        client_id: String,
        /// The certificate generated for this form (`POST
        /// /api/accounts/certificate`). Absent: keep the stored one.
        #[serde(default)]
        #[ts(optional)]
        draft_id: Option<String>,
    },
}

/// The password masked: a request may be logged.
impl fmt::Debug for SaveLoginRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password { username, password } => f
                .debug_struct("Password")
                .field("username", username)
                .field("password", &password.as_ref().map(|_| "***"))
                .finish(),
            Self::Microsoft {
                tenant_id,
                client_id,
                draft_id,
            } => f
                .debug_struct("Microsoft")
                .field("tenant_id", tenant_id)
                .field("client_id", client_id)
                .field("draft_id", draft_id)
                .finish(),
        }
    }
}

/// `raw` as an email address a request gave, or a `422` that names `what`.
pub fn valid_address(raw: &str, what: &str) -> AppResult<()> {
    if parse_address(raw).is_none() {
        return Err(AppError::unprocessable(format!(
            "The {what} {raw} is not a valid email address."
        )));
    }
    Ok(())
}

/// How a request logs in: a password as typed (blank as `None`), or a
/// Microsoft login (its columns and secrets are settled by
/// [`Accounts::prepare_microsoft_save`]).
pub enum Login {
    Password(Option<String>),
    Microsoft(MicrosoftLoginRequest),
}

/// Trimmed, empty as `None`.
pub fn non_empty(value: &str) -> Option<&str> {
    Some(value.trim()).filter(|v| !v.is_empty())
}

/// The account `req` describes, the one way a save stores it and a test
/// tries it: every text trimmed, a blank name the address, unset ports the
/// provider's ([`ports`]), a blank folder [`DEFAULT_FOLDER`], blank
/// forwarding rows dropped, a Microsoft login without servers (its own
/// columns still empty); and its login. Refuses nothing: [`validate`]
/// does, for a save.
pub fn normalise(req: &SaveAccountRequest) -> (AccountFields, Login) {
    let address = req.address.trim().to_owned();
    let (login_kind, username, login) = match &req.login {
        SaveLoginRequest::Microsoft {
            tenant_id,
            client_id,
            draft_id,
        } => {
            // Logs in as the mailbox: its address.
            let login = MicrosoftLoginRequest {
                tenant_id: tenant_id.clone(),
                client_id: client_id.clone(),
                draft_id: draft_id.clone(),
            };
            (LoginKind::Microsoft, String::new(), Login::Microsoft(login))
        }
        SaveLoginRequest::Password { username, password } => (
            LoginKind::Password,
            username.trim().to_owned(),
            Login::Password(credentials::typed(password.as_deref()).map(str::to_owned)),
        ),
    };
    let defaults = AccountFields::default();
    let (imap, smtp) = match login_kind {
        LoginKind::Password => (req.imap.as_ref(), req.smtp.as_ref()),
        LoginKind::Microsoft => (None, None),
    };
    let (imap_port, smtp_port) = match (imap, smtp) {
        (None, None) => (defaults.imap_port, defaults.smtp_port),
        _ => ports(imap, smtp, req.provider),
    };
    let fields = AccountFields {
        name: non_empty(&req.name).unwrap_or(&address).to_owned(),
        provider: preset(req.provider).provider,
        login_kind,
        username,
        // A password login has none; a Microsoft one is filled in by its save.
        microsoft: MicrosoftLogin::default(),
        imap_host: imap.map(|s| s.host.trim().to_owned()).unwrap_or_default(),
        imap_port,
        imap_security: imap.map_or(defaults.imap_security, |s| s.security),
        smtp_host: smtp.map(|s| s.host.trim().to_owned()).unwrap_or_default(),
        smtp_port,
        smtp_security: smtp.map_or(defaults.smtp_security, |s| s.security),
        receive_enabled: req.receiving.enabled,
        folder: non_empty(&req.receiving.folder)
            .unwrap_or(DEFAULT_FOLDER)
            .to_owned(),
        forwarded_from: req
            .receiving
            .forwarded_from
            .iter()
            .filter_map(|a| non_empty(a))
            .map(str::to_owned)
            .collect(),
        address,
    };
    (fields, login)
}

/// The ports of the servers on screen, an unset one (0, or no SMTP port)
/// the provider's ([`Preset::imap_port`](super::presets::Preset::imap_port));
/// a server not sent is unset. No SMTP server: its port as given, 0 as none.
fn ports(
    imap: Option<&ImapServerView>,
    smtp: Option<&SmtpServerView>,
    provider: ProviderOption,
) -> (u16, Option<u16>) {
    let preset = preset(provider);
    let smtp_port = match smtp {
        None => None,
        Some(smtp) if smtp.host.trim().is_empty() => smtp.port.filter(|&p| p != 0),
        Some(smtp) => Some(preset.smtp_port(smtp.port, smtp.security)),
    };
    let imap_port = match imap {
        None => preset.imap_port(0, AccountFields::default().imap_security),
        Some(imap) => preset.imap_port(imap.port, imap.security),
    };
    (imap_port, smtp_port)
}

/// What a save refuses of the [`normalise`]d `fields` and a test does not
/// (a `422`, the first in this order): an invalid address, none while
/// receiving, no name, an invalid, own or repeated forwarding address, a
/// login other than the provider's, a password login receiving without an
/// IMAP server or without any server, an invalid SMTP port, a password
/// login with servers and no username.
pub fn validate(fields: &AccountFields) -> AppResult<()> {
    let address = &fields.address;
    if !address.is_empty() {
        valid_address(address, "email address")?;
    } else if fields.receive_enabled {
        return Err(AppError::unprocessable(ENTER_ADDRESS));
    }
    if fields.name.is_empty() {
        return Err(AppError::unprocessable(ENTER_NAME));
    }
    forwarded_addresses(&fields.forwarded_from, address)?;

    let preset = preset_of(fields);
    if fields.login_kind != preset.login {
        return Err(AppError::unprocessable(match preset.login {
            LoginKind::Microsoft => MICROSOFT_LOGIN_ONLY.to_owned(),
            LoginKind::Password => format!("{} accounts log in with a password.", preset.label),
        }));
    }
    // A Microsoft login reads and sends over Graph: it has no servers.
    if fields.login_kind == LoginKind::Microsoft {
        return Ok(());
    }
    if fields.receive_enabled && fields.imap_host.is_empty() {
        return Err(AppError::unprocessable(ENTER_IMAP_HOST));
    }
    if fields.imap_host.is_empty() && fields.smtp_host.is_empty() {
        return Err(AppError::unprocessable(NO_SERVER));
    }
    if let Some(port) = fields.smtp_port.filter(|_| !fields.smtp_host.is_empty()) {
        check_port(port).map_err(|e| AppError::unprocessable(e.message()))?;
    }
    if fields.username.is_empty() {
        return Err(AppError::unprocessable(ENTER_USERNAME));
    }
    Ok(())
}

/// The forwarded addresses (trimmed, no blank row) each a valid address
/// that is neither `address` (the account's own) nor entered twice (case
/// folded); `422` otherwise.
fn forwarded_addresses(forwarded: &[String], address: &str) -> AppResult<()> {
    for (i, other) in forwarded.iter().enumerate() {
        valid_address(other, "forwarding address")?;
        if other.eq_ignore_ascii_case(address) {
            return Err(AppError::unprocessable(format!(
                "The forwarding address {other} is the account's own address."
            )));
        }
        if forwarded[..i].iter().any(|a| a.eq_ignore_ascii_case(other)) {
            return Err(AppError::unprocessable(format!(
                "The forwarding address {other} is entered twice."
            )));
        }
    }
    Ok(())
}

/// The address, or a forwarded address, already belongs to another account
/// than `id` (as its own address or a forwarded one): `409`.
pub async fn address_free(
    pool: &SqlitePool,
    fields: &AccountFields,
    id: Option<i64>,
) -> AppResult<()> {
    for address in std::iter::once(&fields.address).chain(&fields.forwarded_from) {
        if let Some(other) = account_with_address(pool, address).await? {
            if Some(other) != id {
                return Err(AppError::conflict(format!(
                    "Another account already uses {address}."
                )));
            }
        }
    }
    Ok(())
}

/// The password saving `fields` stores: `Some` new one (typed and not the
/// stored one), `None` to keep the stored one where it still applies;
/// `409` when it would go to another target, `422` when a server needs one
/// and there is none.
fn new_password(
    fields: &AccountFields,
    typed: Option<&str>,
    saved: Option<&AccountFields>,
    stored: &SecretState,
) -> AppResult<Option<String>> {
    let same = credentials::reusable_for_all(saved, fields);
    let needs = !(fields.imap_host.is_empty() && fields.smtp_host.is_empty());
    match resolve(typed, stored.is_stored(), same) {
        Resolved::Typed(password) if *stored == SecretState::Readable(password.clone()) => Ok(None),
        Resolved::Typed(password) => Ok(Some(password)),
        Resolved::Stored if needs && !stored.is_stored() => {
            Err(AppError::unprocessable(ENTER_PASSWORD))
        }
        Resolved::Stored => Ok(None),
        Resolved::Required => Err(AppError::conflict(PASSWORD_AGAIN)),
    }
}

impl Accounts {
    /// The secret changes saving `fields` (with `login`) as account `id`
    /// (`None`: a new one) makes, and the draft certificate to take once
    /// written; `fields` gets a Microsoft login's columns. `saved` is the
    /// stored account. Sealed with [`Accounts::seal`] once the id is known.
    pub async fn secret_changes(
        &self,
        id: Option<i64>,
        fields: &mut AccountFields,
        login: &Login,
        saved: Option<&AccountFields>,
    ) -> AppResult<(Vec<(SecretKind, SecretChange)>, Option<String>)> {
        match login {
            Login::Password(typed) => {
                let stored = match id {
                    Some(id) => self.secret(id, SecretKind::Password).await?,
                    None => SecretState::Missing,
                };
                let password = match new_password(fields, typed.as_deref(), saved, &stored)? {
                    Some(password) => SecretChange::Set(password),
                    None => SecretChange::Keep,
                };
                // A password login keeps no Microsoft secrets.
                Ok((
                    vec![
                        (SecretKind::Password, password),
                        (SecretKind::PrivateKey, SecretChange::Clear),
                    ],
                    None,
                ))
            }
            Login::Microsoft(request) => {
                let save = self.prepare_microsoft_save(id, fields, request).await?;
                fields.microsoft = save.microsoft;
                Ok((save.secrets, save.draft_id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_request_prints_no_secret() {
        let password: SaveLoginRequest = serde_json::from_value(serde_json::json!({
            "kind": "password", "username": "u@example.com", "password": "hunter2"
        }))
        .unwrap();
        let printed = format!("{password:?}");
        assert!(printed.contains("u@example.com"), "{printed}");
        assert!(!printed.contains("hunter2"), "{printed}");
    }
}
