//! The mail accounts as the API answers them: each account's servers, login
//! and receiving with what follows from them (ready to receive, ready to
//! send, whether its credentials can be used), the providers' presets, each
//! receiving account's health, and what a connection test answered
//! ([`AccountTestResponse`]). Secrets never leave: a password is
//! `has_password`, a certificate's key stays with the server.

use crate::accounts::presets::{Preset, ProviderOption, PRESETS};
use crate::accounts::{credential_expires_at, sends_mail, AccountsSnapshot};
use crate::wording::accounts::{expiry_sentence, locked_sentence, same_mailbox_as};
use dmarc_assess::{credential_condition, CredentialCondition, LockReason, MailboxState, Status};
use dmarc_store::{AccountRow, ImapSecurity, LoginKind, SecretKind, SmtpSecurity};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One account.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct AccountView {
    pub id: i64,
    pub name: String,
    /// The Provider option it is shown as ([`ProviderOption::of`]).
    pub provider: ProviderOption,
    /// The report address and default From; empty: none.
    pub address: String,
    pub login: LoginView,
    /// A password login's servers; `null` for a Microsoft login, which
    /// reads and sends over Microsoft Graph.
    pub imap: Option<ImapServerView>,
    pub smtp: Option<SmtpServerView>,
    pub receiving: AccountReceivingView,
    /// Its passes read its mailbox: receiving, its login set (a password
    /// login's IMAP server too), its credentials readable.
    pub ready_to_receive: bool,
    /// Its address is where reports reach Marcan (the reporting setup's
    /// known addresses): receiving, with an address, its login set (a
    /// password login's IMAP server too). Unlike `ready_to_receive`, locked
    /// credentials keep it: the reports still arrive in its mailbox.
    pub report_address: bool,
    /// It can send once its login is complete: a password login has an
    /// SMTP server, a Microsoft login always can (over Microsoft Graph).
    /// The accounts Settings → Sending offers.
    pub sends: bool,
    /// Mail can go out through it: `sends`, its login set, its credentials
    /// readable.
    pub ready_to_send: bool,
    pub credential: CredentialState,
    /// Another receiving account reads the same mailbox and folder
    /// ([`receive_identity`](crate::accounts::receive_identity)): the
    /// sentence naming it; `null`
    /// when none does. Allowed (each report is stored once), but one of the
    /// two is enough.
    pub shared_mailbox: Option<String>,
}

/// How an account logs in, without the secret.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginView {
    Password {
        /// Empty: not set.
        username: String,
        has_password: bool,
    },
    Microsoft {
        tenant_id: String,
        client_id: String,
        /// The stored certificate it proves itself with; `null`: none yet.
        certificate: Option<CertificateView>,
    },
}

/// A Microsoft login's stored certificate (its key stays with the server).
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct CertificateView {
    /// SHA-1, upper-case hex, as the Entra portal lists it once uploaded.
    pub thumbprint: String,
    /// Unix seconds.
    pub expires_at: Option<i64>,
}

/// An account's IMAP server. An empty host: none.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ImapServerView {
    pub host: String,
    pub port: u16,
    pub security: ImapSecurity,
}

/// An account's SMTP server. An empty host: none (the account then cannot
/// send).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SmtpServerView {
    pub host: String,
    /// `null`: not set.
    pub port: Option<u16>,
    pub security: SmtpSecurity,
}

/// Whether and where an account receives reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct AccountReceivingView {
    pub enabled: bool,
    pub folder: String,
    /// The other addresses whose reports are forwarded to this account, in
    /// the order entered.
    pub forwarded_from: Vec<String>,
}

/// Whether an account's credentials can be used now, with the verdict.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct CredentialState {
    pub condition: CredentialCondition,
    pub status: Status,
    /// Why they cannot be read; `null` unless `locked`.
    pub lock: Option<LockReason>,
    /// What to do about the lock, naming the account; `null` unless `locked`.
    pub lock_text: Option<String>,
    /// When the credential expires (a Microsoft login's certificate);
    /// `null`: it does not.
    pub expires_at: Option<i64>,
    /// When it expires or expired, as a sentence; `null` unless `expiring`
    /// or `expired`.
    pub expiry_text: Option<String>,
}

/// The servers and login one Provider option's accounts start from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ProviderPreset {
    pub provider: ProviderOption,
    pub label: String,
    /// The line under the Provider select, before its docs link.
    pub hint: String,
    /// Its section of the docs, relative to their root (`accounts#google`).
    pub docs: String,
    /// The servers it starts from; `null` for Microsoft 365, which has none.
    pub imap: Option<ImapServerView>,
    pub smtp: Option<SmtpServerView>,
    /// The one login its accounts use.
    pub login: LoginKind,
}

impl From<&Preset> for ProviderPreset {
    fn from(p: &Preset) -> Self {
        Self {
            provider: p.option,
            label: p.label.to_owned(),
            hint: p.hint.to_owned(),
            docs: p.docs.to_owned(),
            imap: p.servers.map(|s| ImapServerView {
                host: s.imap_host.to_owned(),
                port: s.imap_port,
                security: s.imap_security,
            }),
            smtp: p.servers.map(|s| SmtpServerView {
                host: s.smtp_host.to_owned(),
                port: s.smtp_port,
                security: s.smtp_security,
            }),
            login: p.login,
        }
    }
}

/// Every option's preset, in the order the form offers them.
pub fn presets() -> Vec<ProviderPreset> {
    PRESETS.iter().map(ProviderPreset::from).collect()
}

/// The usual ports of one protocol: the port field starts at the one for
/// the security chosen, and follows it while untouched.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct ServerPorts {
    /// With TLS from the start.
    pub tls: u16,
    /// With `STARTTLS`.
    pub starttls: u16,
}

/// What an account's unset fields start from in the account form.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct AccountDefaults {
    pub imap: ServerPorts,
    pub smtp: ServerPorts,
    /// The folder reports are read from when none is chosen.
    pub folder: String,
}

impl Default for AccountDefaults {
    fn default() -> Self {
        Self {
            imap: ServerPorts {
                tls: dmarc_mailbox::DEFAULT_PORT,
                starttls: dmarc_mailbox::STARTTLS_PORT,
            },
            smtp: ServerPorts {
                tls: crate::mail::TLS_PORT,
                starttls: crate::mail::STARTTLS_PORT,
            },
            folder: dmarc_mailbox::DEFAULT_FOLDER.to_owned(),
        }
    }
}

/// One step of an account test that passed or failed: `message` in plain
/// language, `detail` the raw error (secrets masked), `docs` the section of
/// the docs that shows the fix, relative to their root
/// (`accounts#exchange-online`), when the message asks for a setup step there.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct StepResult {
    pub ok: bool,
    pub message: String,
    pub detail: Option<String>,
    pub docs: Option<String>,
}

impl StepResult {
    pub fn passed(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
            detail: None,
            docs: None,
        }
    }

    pub fn failed(message: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            detail: detail.filter(|d| !d.is_empty()),
            docs: None,
        }
    }

    /// The docs section that shows the fix ([`StepResult::docs`]).
    pub fn with_docs(self, docs: Option<&str>) -> Self {
        Self {
            docs: docs.map(str::to_owned),
            ..self
        }
    }
}

/// A selectable folder the mailbox step listed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct FolderResponse {
    pub name: String,
    pub delimiter: Option<String>,
}

/// The folder the mailbox step examined.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct SelectedFolderResponse {
    pub name: String,
    /// The folder could be opened (IMAP: read-only `EXAMINE`; Graph: found
    /// by its path).
    pub exists: bool,
    /// Message count; `null` when the folder does not exist.
    pub messages: Option<u32>,
}

/// The mailbox step (over IMAP for a password login, over Microsoft Graph
/// for a Microsoft login): the mailbox reached, its folders listed and one
/// examined; on failure `message` says why, `detail` is the raw error and
/// `docs` the docs section on the fix, as on a [`StepResult`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct MailboxTestResult {
    pub ok: bool,
    pub message: String,
    pub detail: Option<String>,
    pub docs: Option<String>,
    pub folders: Vec<FolderResponse>,
    pub selected: Option<SelectedFolderResponse>,
}

impl MailboxTestResult {
    pub fn failed(message: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            detail: detail.filter(|d| !d.is_empty()),
            docs: None,
            folders: Vec::new(),
            selected: None,
        }
    }
}

/// `POST /api/accounts/test`: one line per step that ran, `null` for one
/// that did not (no server for it, not asked for, or a step before it
/// failed).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct AccountTestResponse {
    /// The access token (a Microsoft login only).
    pub token: Option<StepResult>,
    /// The mailbox and its folders.
    pub mailbox: Option<MailboxTestResult>,
    /// `EHLO` and the login; no message is sent.
    pub smtp: Option<StepResult>,
}

/// One receiving account's health, as `GET /api/system/health` lists it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct AccountHealth {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub state: MailboxState,
    pub status: Status,
    /// The newest time one of its passes reached the mailbox search.
    pub last_check_at: Option<i64>,
    /// When one of its passes last read the whole mailbox without error.
    pub last_success_at: Option<i64>,
    /// Why its last pass failed, or why it cannot be read (locked); `null`
    /// once a later pass succeeded. The sentence alone: the server's answer
    /// is `last_error_detail`.
    pub last_error: Option<String>,
    /// The server's own answer beside `last_error` (Microsoft Graph's code
    /// and message), for the account page's Details; `null` when it gave
    /// none or the account is locked.
    pub last_error_detail: Option<String>,
    pub quarantine_count: i64,
}

impl AccountView {
    /// `account` with what `snapshot` knows of its secrets, at `now`.
    pub fn new(account: &AccountRow, snapshot: &AccountsSnapshot, now: i64) -> Self {
        let f = &account.fields;
        let ms = &f.microsoft;
        let login = match f.login_kind {
            LoginKind::Password => LoginView::Password {
                username: f.username.clone(),
                has_password: snapshot.has_secret(account.id, SecretKind::Password),
            },
            LoginKind::Microsoft => LoginView::Microsoft {
                tenant_id: ms.tenant_id.clone().unwrap_or_default(),
                client_id: ms.client_id.clone().unwrap_or_default(),
                certificate: ms.cert_der.as_ref().map(|_| CertificateView {
                    thumbprint: ms.cert_thumbprint.clone().unwrap_or_default(),
                    expires_at: ms.cert_expires_at,
                }),
            },
        };
        let expires_at = credential_expires_at(f);
        let servers = f.login_kind == LoginKind::Password;
        let lock = snapshot.lock(account.id);
        let condition = credential_condition(lock, expires_at, now);
        Self {
            id: account.id,
            name: f.name.clone(),
            provider: ProviderOption::of(f.provider, &f.imap_host),
            address: f.address.clone(),
            login,
            imap: servers.then(|| ImapServerView {
                host: f.imap_host.clone(),
                port: f.imap_port,
                security: f.imap_security,
            }),
            smtp: servers.then(|| SmtpServerView {
                host: f.smtp_host.clone(),
                port: f.smtp_port,
                security: f.smtp_security,
            }),
            receiving: AccountReceivingView {
                enabled: f.receive_enabled,
                folder: f.folder.clone(),
                forwarded_from: f.forwarded_from.clone(),
            },
            ready_to_receive: snapshot.ready_to_receive(account),
            report_address: snapshot.can_receive(account) && !f.address.trim().is_empty(),
            sends: sends_mail(f),
            ready_to_send: snapshot.ready_to_send(account),
            credential: CredentialState {
                condition,
                status: condition.status(),
                lock,
                lock_text: lock.map(|reason| locked_sentence(&f.name, reason)),
                expires_at,
                expiry_text: expiry_sentence(condition, expires_at),
            },
            shared_mailbox: shares_mailbox_with(account, snapshot)
                .map(|other| same_mailbox_as(&other.fields.name)),
        }
    }
}

/// The first other account of `snapshot` that reads the mailbox and folder
/// `account` reads ([`AccountsSnapshot::mailbox_of`]); `None` when
/// `account` reads none or none does.
pub fn shares_mailbox_with<'a>(
    account: &AccountRow,
    snapshot: &'a AccountsSnapshot,
) -> Option<&'a AccountRow> {
    let own = snapshot.mailbox_of(account)?;
    snapshot
        .accounts
        .iter()
        .find(|other| other.id != account.id && snapshot.mailbox_of(other).as_ref() == Some(&own))
}

/// Every account of `snapshot`, in id order.
pub fn account_views(snapshot: &AccountsSnapshot, now: i64) -> Vec<AccountView> {
    snapshot
        .accounts
        .iter()
        .map(|a| AccountView::new(a, snapshot, now))
        .collect()
}
