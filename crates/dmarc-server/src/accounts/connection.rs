//! The account test (`POST /api/accounts/test`): with the values on screen,
//! normalised as a save would store them ([`save::normalise`]), log in to
//! the IMAP server and list its folders, and log in to the SMTP server
//! (`EHLO` and `AUTH`, no message). A stored secret is used only for the
//! server and username it was saved for, the rule a save follows
//! ([`credentials::reusable`]); a test of another target needs the secret
//! typed. A Microsoft login gets a fresh token first, then reads the
//! mailbox over Microsoft Graph (its folders, and the folder's message
//! count). It sends over Graph, which has no step that proves sending
//! without sending a message, so its test has no sending line: Settings →
//! Sending's test message proves it.
//!
//! The IMAP login of a password login goes through the [`ImapProber`] on
//! [`AppState`](crate::AppState) (the network, or a test's fake server), a
//! Microsoft login's reading to the accounts' Graph endpoint, the SMTP
//! login through its [`MailSender`](crate::mail::MailSender).

use super::credentials::{self, resolve, Resolved, Server};
use super::microsoft::{graph_config, MicrosoftLoginRequest};
use super::save::{self, non_empty, Login, SaveAccountRequest};
use super::{imap_security, login_user, AccountError, SecretState};
use crate::error::{AppError, AppResult};
use crate::mail::{check_port, SmtpServer};
use crate::view::accounts::{
    AccountTestResponse, FolderResponse, MailboxTestResult, SelectedFolderResponse, StepResult,
};
use crate::wording::accounts::{
    locked_sentence, logged_in_as, read_over_graph, ENTER_PASSWORD, ENTER_SERVER_TO_TEST,
    ENTER_SMTP_PORT, ENTER_USERNAME, PASSWORD_AGAIN, UNNAMED,
};
use crate::wording::microsoft::ENTER_MAILBOX;
use crate::AppState;
use dmarc_mailbox::{probe_graph, ConnectError, ImapConfig, MailboxProbe};
use dmarc_store::{AccountFields, AccountRow, SecretKind};
use futures::future::BoxFuture;

/// Logs in to an IMAP server, lists its folders and examines one: the
/// network ([`NetworkProber`]) or, in tests, a fake server.
pub trait ImapProber: Send + Sync {
    fn probe<'a>(
        &'a self,
        cfg: &'a ImapConfig,
    ) -> BoxFuture<'a, Result<MailboxProbe, ConnectError>>;
}

/// [`dmarc_mailbox::connect_mailbox`]: the real server.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkProber;

impl ImapProber for NetworkProber {
    fn probe<'a>(
        &'a self,
        cfg: &'a ImapConfig,
    ) -> BoxFuture<'a, Result<MailboxProbe, ConnectError>> {
        Box::pin(dmarc_mailbox::connect_mailbox(cfg))
    }
}

/// Test the account on screen (`account`; `saved` the stored one it edits,
/// whose secrets apply to its own servers only), examining `folder`
/// (default: the account's) and, a password login, logging in to the SMTP
/// server too when `smtp`. One line per step; `422` for a password login
/// without a server to test.
pub async fn test_connection(
    state: &AppState,
    saved: Option<&AccountRow>,
    account: &SaveAccountRequest,
    folder: Option<&str>,
    smtp: bool,
) -> AppResult<AccountTestResponse> {
    let (mut fields, login) = save::normalise(account);
    // What a save would refuse is tried anyway, each step saying what it
    // misses; a form without a name is still named in its sentences.
    if fields.name.is_empty() {
        UNNAMED.clone_into(&mut fields.name);
    }
    if let Some(folder) = folder.and_then(non_empty) {
        folder.clone_into(&mut fields.folder);
    }
    let typed = match &login {
        Login::Microsoft(login) => {
            let id = saved.map(|row| row.id);
            return test_microsoft(state, id, &fields, login).await;
        }
        Login::Password(typed) => typed.as_deref(),
    };
    let test_smtp = smtp && !fields.smtp_host.is_empty();
    if fields.imap_host.is_empty() && !test_smtp {
        return Err(AppError::unprocessable(ENTER_SERVER_TO_TEST));
    }
    let stored = match saved {
        Some(row) => state.accounts.secret(row.id, SecretKind::Password).await?,
        None => SecretState::Missing,
    };
    let saved = saved.map(|row| &row.fields);
    let mailbox = if fields.imap_host.is_empty() {
        None
    } else {
        Some(test_imap(state.imap_prober.as_ref(), &fields, typed, saved, &stored).await)
    };
    let smtp = if test_smtp {
        Some(match smtp_server(&fields, typed, saved, &stored) {
            Err(why) => StepResult::failed(why, None),
            Ok(server) => smtp_step(state, &server).await,
        })
    } else {
        None
    };
    Ok(AccountTestResponse {
        token: None,
        mailbox,
        smtp,
    })
}

/// The password a test of `server` uses: typed, else the stored one when
/// it may go there ([`credentials::reusable`]).
fn test_password(
    account: &AccountFields,
    server: Server,
    typed: Option<&str>,
    saved: Option<&AccountFields>,
    stored: &SecretState,
) -> Result<String, String> {
    let same = credentials::reusable(saved, account, server);
    match resolve(typed, stored.is_stored(), same) {
        Resolved::Typed(password) => Ok(password),
        Resolved::Required => Err(PASSWORD_AGAIN.to_owned()),
        Resolved::Stored => match stored {
            SecretState::Readable(password) => Ok(password.clone()),
            SecretState::Missing => Err(ENTER_PASSWORD.to_owned()),
            SecretState::Locked(reason) => Err(locked_sentence(&account.name, *reason)),
        },
    }
}

/// The IMAP step of a password login: `account` (the values on screen)
/// examining its folder; `saved` is the stored account, if any, whose
/// `stored` password applies only to its own server and username.
async fn test_imap(
    prober: &dyn ImapProber,
    account: &AccountFields,
    typed: Option<&str>,
    saved: Option<&AccountFields>,
    stored: &SecretState,
) -> MailboxTestResult {
    if account.username.is_empty() {
        return MailboxTestResult::failed(ENTER_USERNAME, None);
    }
    let password = match test_password(account, Server::Imap, typed, saved, stored) {
        Ok(password) => password,
        Err(why) => return MailboxTestResult::failed(why, None),
    };
    let cfg = ImapConfig {
        id: "test".to_owned(),
        epoch: 0,
        host: account.imap_host.clone(),
        port: account.imap_port,
        security: imap_security(account.imap_security),
        username: account.username.clone(),
        password,
        folder: account.folder.clone(),
    };
    probe_result(logged_in_as(&cfg.username), prober.probe(&cfg).await)
}

/// The mailbox step's line: `reached` and the folders and the examined
/// one, or the failure.
fn probe_result(reached: String, probe: Result<MailboxProbe, ConnectError>) -> MailboxTestResult {
    match probe {
        Ok(probe) => MailboxTestResult {
            ok: true,
            message: reached,
            detail: None,
            docs: None,
            folders: probe
                .folders
                .into_iter()
                .map(|f| FolderResponse {
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
        Err(e) => MailboxTestResult {
            docs: e.docs.map(str::to_owned),
            ..MailboxTestResult::failed(e.message, Some(e.detail))
        },
    }
}

/// The SMTP step: log in to `server` (`EHLO` and `AUTH`, no message).
async fn smtp_step(state: &AppState, server: &SmtpServer) -> StepResult {
    match state.mail.login(server).await {
        Ok(()) => StepResult::passed(logged_in_as(&server.username)),
        Err(e) => StepResult::failed(e.message(), Some(e.detail().to_owned())),
    }
}

/// The SMTP port of `account`, or why it cannot be tried.
fn smtp_port(account: &AccountFields) -> Result<u16, String> {
    let port = account
        .smtp_port
        .ok_or_else(|| ENTER_SMTP_PORT.to_owned())?;
    check_port(port).map_err(|e| e.message())?;
    Ok(port)
}

/// The SMTP server of a password login `account` (the values on screen) as
/// the sender would log in to it, or why it cannot be tried.
fn smtp_server(
    account: &AccountFields,
    typed: Option<&str>,
    saved: Option<&AccountFields>,
    stored: &SecretState,
) -> Result<SmtpServer, String> {
    let port = smtp_port(account)?;
    if account.username.is_empty() {
        return Err(ENTER_USERNAME.to_owned());
    }
    let password = test_password(account, Server::Smtp, typed, saved, stored)?;
    Ok(SmtpServer {
        host: account.smtp_host.clone(),
        port,
        security: account.smtp_security,
        username: account.username.clone(),
        password,
    })
}

/// The test of the Microsoft login on screen (`fields`, the stored account
/// `id` whose certificate applies): a fresh token, then, when the account
/// receives, the mailbox read over Graph (its folders, and the fields'
/// folder with its message count; a mailbox that only sends may lack the
/// role to be read). No sending line: sending over Graph cannot be tried
/// without sending.
async fn test_microsoft(
    state: &AppState,
    id: Option<i64>,
    fields: &AccountFields,
    login: &MicrosoftLoginRequest,
) -> AppResult<AccountTestResponse> {
    let token_only = |token: StepResult| AccountTestResponse {
        token: Some(token),
        mailbox: None,
        smtp: None,
    };
    if login_user(fields).is_empty() {
        return Ok(token_only(StepResult::failed(ENTER_MAILBOX, None)));
    }
    let request = match state.accounts.microsoft_test_request(id, login).await {
        Ok(request) => request,
        Err(AccountError::Store(e)) => return Err(e.into()),
        Err(e) => return Ok(token_only(StepResult::failed(e.to_string(), None))),
    };
    let (token_step, issued) = state.accounts.test_token(&request).await;
    if !issued || !fields.receive_enabled {
        return Ok(token_only(token_step));
    }
    // The token source has the test's fresh token cached.
    let cfg = graph_config(
        "test",
        fields,
        state.accounts.graph(),
        state.accounts.tokens(),
        request,
    );
    Ok(AccountTestResponse {
        token: Some(token_step),
        mailbox: Some(probe_result(
            read_over_graph(&cfg.address),
            probe_graph(&cfg).await,
        )),
        smtp: None,
    })
}
