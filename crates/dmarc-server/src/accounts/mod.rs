//! The mail accounts behind receiving and sending: [`Accounts`] on
//! [`AppState`](crate::AppState) turns a stored account and its secret into
//! the mailbox worker's [`MailboxConfig`] ([`Accounts::mailbox_config`]:
//! IMAP, or Graph for a Microsoft login) and the sender's [`SendConfig`]
//! ([`Accounts::send_config`]: SMTP, or Graph for a Microsoft login), and
//! at start seals the plain secrets when a key is set and moves the
//! previous key's to it ([`Accounts::start`]).
//!
//! - `secrets`: AES-256-GCM under `MARCAN_SECRET_KEY`, and the lock of a
//!   value the key cannot open.
//! - `credentials`: which secret a login uses (a stored one only for the
//!   server and username it was saved for).
//! - `presets`: the servers and login each Provider option starts from.
//! - `save`: an account form as a save stores it and a test tries it, what
//!   a save refuses, and what it does to the secrets.
//! - `connection`: the connection test of an account form.
//! - `oauth`: a Microsoft login's access tokens (the client credentials
//!   grant with a certificate, cached in memory).
//! - `certificate`: a Microsoft login's certificate and the client
//!   assertion signed with its key; unsaved forms' certificates in memory.
//! - `microsoft`: what saving a Microsoft login checks and stores, and the
//!   token request of a stored or typed one.
//! - `test`: the connection test's token step.
//!
//! An account whose secret is locked neither receives (the worker lists it,
//! and its pass finds no configuration and logs why) nor sends (a
//! [`SendError::Locked`]); entering the credentials again stores them with
//! the current key, or plain without one. A Microsoft login has no servers:
//! it reads its mailbox address over Microsoft Graph with an access token
//! (fetched when each pass starts, and again when it runs out or Graph
//! refuses it), and sends as it over Graph with the same token
//! ([`sends_mail`]).

pub mod certificate;
pub mod connection;
pub mod credentials;
pub mod microsoft;
pub mod oauth;
pub mod presets;
pub mod save;
pub mod secrets;
pub mod test;

pub use secrets::{SecretChange, SecretKey, SecretKeys};

use crate::mail::{
    check_port, parse_address, parse_reply_to, Letterhead, SendConfig, SendError, SmtpServer,
    Transport,
};
use crate::wording::accounts::{
    invalid_sender_address, locked_sentence, no_sender_address, NO_SENDING_ACCOUNT,
};
use certificate::Drafts;
use dmarc_assess::{AccountReading, LockReason};
use dmarc_mailbox::{
    GraphApi, GraphSendConfig, GraphToken, ImapConfig, ImapSecurity as ClientSecurity,
    MailboxConfig,
};
use dmarc_store::{
    compact_or_warn, get_account, get_account_secret, list_account_secrets, list_accounts,
    receive_pass_status, reseal_secrets, AccountFields, AccountRow, AppSettings, ImapSecurity,
    LoginKind, SecretKind, SecretRow, SecretWrite, SendingSettings, StoreError,
};
use oauth::{TokenError, TokenSource};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{error, info};
use zeroize::Zeroizing;

/// Why an account's configuration for a pass could not be made: the store,
/// or its Microsoft login (no token).
/// What a failed pass records and the health shows.
#[derive(Debug)]
pub enum AccountError {
    Store(StoreError),
    Token(TokenError),
    /// The login cannot be used as stored; the text says why.
    Refused(String),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Token(e) => write!(f, "{e}"),
            Self::Refused(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for AccountError {}

impl From<StoreError> for AccountError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// When an account's credential expires: a Microsoft login's certificate;
/// `None` for a password, or a Microsoft login without a certificate.
pub fn credential_expires_at(account: &AccountFields) -> Option<i64> {
    let ms = &account.microsoft;
    match account.login_kind {
        LoginKind::Microsoft => ms.cert_der.as_ref().and(ms.cert_expires_at),
        LoginKind::Password => None,
    }
}

/// Who an account logs in as: its username, else (a Microsoft login, which
/// logs in as its mailbox) its address.
pub fn login_user(account: &AccountFields) -> &str {
    match account.username.as_str() {
        "" => account.address.as_str(),
        user => user,
    }
}

/// Whether mail can go out through `account` once its login is complete:
/// a password login through its SMTP server, so it needs one; a Microsoft
/// login over Microsoft Graph as its mailbox. Every "can this account send"
/// decision starts here.
pub fn sends_mail(account: &AccountFields) -> bool {
    match account.login_kind {
        LoginKind::Password => !account.smtp_host.is_empty(),
        LoginKind::Microsoft => true,
    }
}

/// Whether `account` has a way to read its mailbox once its login is
/// complete: a password login its IMAP server, a Microsoft login always
/// (over Microsoft Graph). Receiving needs it, beside `receive_enabled`.
pub fn reads_mailbox(account: &AccountFields) -> bool {
    match account.login_kind {
        LoginKind::Password => !account.imap_host.is_empty(),
        LoginKind::Microsoft => true,
    }
}

/// Which mailbox and folder an account reads ([`receive_identity`]). Its
/// cursor, skipped messages, retry counts and passes belong to this: a save
/// that changes it starts the account's reading over
/// ([`dmarc_store::ReceiveChange::Replaced`], which moves its
/// `receive_epoch` on), and two accounts with the same one read the same
/// mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailboxIdentity<'a> {
    /// A mailbox on an IMAP server: the host (case folded), the port and
    /// the user it logs in as ([`login_user`]).
    Imap {
        host: String,
        port: u16,
        user: &'a str,
        folder: &'a str,
    },
    /// A Microsoft 365 mailbox read over Graph: its tenant and address
    /// (case folded). Not the app registration (client ID) or its
    /// certificate: another one reading the same mailbox reads the same
    /// folder, and the stored position (a delta link naming the mailbox
    /// and folder) still holds for it; one Graph no longer accepts starts
    /// the listing over by itself.
    Graph {
        tenant: String,
        address: String,
        folder: &'a str,
    },
}

/// The mailbox and folder `account` names, whether it receives or not.
pub fn receive_identity(account: &AccountFields) -> MailboxIdentity<'_> {
    let folder = account.folder.as_str();
    match account.login_kind {
        LoginKind::Password => MailboxIdentity::Imap {
            host: account.imap_host.to_ascii_lowercase(),
            port: account.imap_port,
            user: login_user(account),
            folder,
        },
        LoginKind::Microsoft => MailboxIdentity::Graph {
            tenant: account
                .microsoft
                .tenant_id
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            address: login_user(account).to_ascii_lowercase(),
            folder,
        },
    }
}

/// The stored client setting as the IMAP client's.
pub fn imap_security(security: ImapSecurity) -> ClientSecurity {
    match security {
        ImapSecurity::Tls => ClientSecurity::Tls,
        ImapSecurity::StartTls => ClientSecurity::StartTls,
    }
}

/// One stored secret, opened.
#[derive(Clone, PartialEq, Eq)]
pub enum SecretState {
    Missing,
    Readable(String),
    Locked(LockReason),
}

impl std::fmt::Debug for SecretState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("Missing"),
            Self::Readable(_) => f.write_str("Readable(***)"),
            Self::Locked(reason) => write!(f, "Locked({reason:?})"),
        }
    }
}

impl SecretState {
    /// A value is stored, readable or not.
    pub fn is_stored(&self) -> bool {
        !matches!(self, Self::Missing)
    }
}

/// Every account with what is known of its secrets without opening them
/// (which are stored, which are locked): what the settings and the health
/// answers are made from.
#[derive(Debug, Clone, Default)]
pub struct AccountsSnapshot {
    pub accounts: Vec<AccountRow>,
    stored: HashSet<(i64, SecretKind)>,
    locks: HashMap<i64, LockReason>,
    /// How many stored secrets are not encrypted.
    plain: usize,
}

impl AccountsSnapshot {
    pub fn get(&self, id: i64) -> Option<&AccountRow> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn has_secret(&self, id: i64, kind: SecretKind) -> bool {
        self.stored.contains(&(id, kind))
    }

    /// Why account `id` is locked, if it is: one of its secrets cannot be read.
    pub fn lock(&self, id: i64) -> Option<LockReason> {
        self.locks.get(&id).copied()
    }

    /// Account `account`'s login is complete, its secret readable or not: a
    /// username and password, or a Microsoft login's tenant, client ID,
    /// mailbox address, certificate and key.
    pub fn has_login(&self, account: &AccountRow) -> bool {
        let f = &account.fields;
        let ms = &f.microsoft;
        let set = |v: &Option<String>| v.as_deref().is_some_and(|v| !v.is_empty());
        match f.login_kind {
            LoginKind::Password => {
                !f.username.is_empty() && self.has_secret(account.id, SecretKind::Password)
            }
            LoginKind::Microsoft => {
                set(&ms.tenant_id)
                    && set(&ms.client_id)
                    && !login_user(f).is_empty()
                    && ms.cert_der.is_some()
                    && self.has_secret(account.id, SecretKind::PrivateKey)
            }
        }
    }

    /// Account `account` has everything a pass needs, its secret readable
    /// or not: the worker lists it. Receiving switched on and its login
    /// complete ([`reads_mailbox`]: a password login's IMAP server too).
    pub fn can_receive(&self, account: &AccountRow) -> bool {
        let f = &account.fields;
        f.receive_enabled && reads_mailbox(f) && self.has_login(account)
    }

    /// The account's passes read its mailbox: everything set, nothing
    /// locked.
    pub fn ready_to_receive(&self, account: &AccountRow) -> bool {
        self.can_receive(account) && self.lock(account.id).is_none()
    }

    /// Mail can go out through the account ([`sends_mail`]): its login
    /// set, nothing locked.
    pub fn ready_to_send(&self, account: &AccountRow) -> bool {
        sends_mail(&account.fields) && self.has_login(account) && self.lock(account.id).is_none()
    }

    /// The accounts that receive reports and have a complete login, in id order.
    pub fn receiving(&self) -> impl Iterator<Item = &AccountRow> {
        self.accounts.iter().filter(|a| self.can_receive(a))
    }

    /// The mailbox account `account` reads; `None` when it reads none
    /// (the worker does not list it: [`Self::can_receive`]).
    pub fn mailbox_of<'a>(&self, account: &'a AccountRow) -> Option<MailboxIdentity<'a>> {
        self.can_receive(account)
            .then(|| receive_identity(&account.fields))
    }
}

/// The accounts' secrets and the key they are sealed with, the Microsoft
/// logins' tokens and the Graph endpoint they read from, and the
/// certificates of forms not saved yet.
pub struct Accounts {
    pool: SqlitePool,
    key: Option<Arc<SecretKey>>,
    /// The key `key` replaces: its values are sealed again at start.
    previous: Option<Arc<SecretKey>>,
    /// When each locked account's pass last logged its lock.
    lock_logged: Mutex<HashMap<i64, Instant>>,
    tokens: Arc<TokenSource>,
    graph: GraphApi,
    drafts: Drafts,
}

impl Accounts {
    /// The accounts in `pool`, sealed with `key`; Microsoft logins get their
    /// tokens from Microsoft's identity platform and read Microsoft Graph.
    pub fn new(pool: SqlitePool, key: Option<SecretKey>) -> Self {
        Self {
            pool,
            key: key.map(Arc::new),
            previous: None,
            lock_logged: Mutex::new(HashMap::new()),
            tokens: Arc::new(TokenSource::microsoft()),
            graph: GraphApi::microsoft(),
            drafts: Drafts::default(),
        }
    }

    /// These accounts with `previous`, the key the current one replaces
    /// (`MARCAN_SECRET_KEY_PREVIOUS`): [`start`](Self::start) seals its
    /// values again with the current key.
    #[must_use]
    pub fn with_previous_key(self, previous: Option<SecretKey>) -> Self {
        Self {
            previous: previous.map(Arc::new),
            ..self
        }
    }

    /// The same accounts and key with Microsoft logins' tokens from
    /// `tokens` (a fake token endpoint in tests, or the source of the
    /// accounts replaced; nothing cached, no draft kept).
    #[must_use]
    pub fn with_token_source(&self, tokens: Arc<TokenSource>) -> Self {
        Self {
            pool: self.pool.clone(),
            key: self.key.clone(),
            previous: self.previous.clone(),
            lock_logged: Mutex::new(HashMap::new()),
            tokens,
            graph: self.graph.clone(),
            drafts: Drafts::default(),
        }
    }

    /// The same accounts, key and tokens with Microsoft logins reading
    /// Graph at `graph` (a fake one in tests, or the endpoint of the
    /// accounts replaced; nothing cached, no draft kept).
    #[must_use]
    pub fn with_graph_api(&self, graph: GraphApi) -> Self {
        Self {
            graph,
            ..self.with_token_source(Arc::clone(&self.tokens))
        }
    }

    /// Where Microsoft logins' tokens come from.
    pub fn tokens(&self) -> &Arc<TokenSource> {
        &self.tokens
    }

    /// Where Microsoft logins read their mailbox.
    pub fn graph(&self) -> &GraphApi {
        &self.graph
    }

    /// The certificates generated for forms not saved yet.
    pub fn drafts(&self) -> &Drafts {
        &self.drafts
    }

    /// The key secrets are sealed with; `None`: they are stored plain.
    pub fn key(&self) -> Option<&SecretKey> {
        self.key.as_deref()
    }

    /// `value` as account `id`'s secret of `kind` is stored: sealed with the
    /// key, plain without one.
    pub fn store(&self, id: i64, kind: SecretKind, value: &str) -> SecretRow {
        secrets::store(self.key(), id, kind, value)
    }

    /// `changes` as the writes of account `id`: each value set sealed with
    /// the key (its additional data names the account).
    pub fn seal(
        &self,
        id: i64,
        changes: &[(SecretKind, SecretChange)],
    ) -> Vec<(SecretKind, SecretWrite)> {
        changes
            .iter()
            .map(|(kind, change)| {
                let write = match change {
                    SecretChange::Keep => SecretWrite::Keep,
                    SecretChange::Set(value) => SecretWrite::Set(self.store(id, *kind, value)),
                    SecretChange::Clear => SecretWrite::Clear,
                };
                (*kind, write)
            })
            .collect()
    }

    /// At start: with a key, every plain secret is sealed and every one the
    /// previous key sealed is sealed again with the key, in one transaction
    /// (the counts logged), and then the database is rewritten so the
    /// replaced values are not left in it ([`dmarc_store::compact`]; a
    /// failure there is logged, not fatal).
    /// Without a key nothing changes, and that secrets are stored
    /// unencrypted is logged while one is. Then each locked account is
    /// logged once with the ways out (its first pass does not repeat it).
    pub async fn start(&self) -> Result<(), StoreError> {
        if let Some(key) = self.key() {
            let previous = self.previous.as_deref().filter(|p| p.id() != key.id());
            let (mut sealed, mut moved) = (0usize, 0usize);
            reseal_secrets(&self.pool, |id, stored| {
                if stored.is_encrypted() {
                    let row = previous?.reseal(key, id, stored)?;
                    moved += 1;
                    return Some(row);
                }
                let text = Zeroizing::new(String::from_utf8(stored.value.clone()).ok()?);
                sealed += 1;
                Some(key.seal(id, stored.kind, &text))
            })
            .await?;
            info!(
                sealed,
                key_id = %key.id_hex(),
                "secrets at rest: encrypted with MARCAN_SECRET_KEY ({sealed} stored in plain text until now)"
            );
            if let Some(previous) = &self.previous {
                match moved {
                    0 => info!(
                        previous_key_id = %previous.id_hex(),
                        "secrets at rest: no stored secret is encrypted with MARCAN_SECRET_KEY_PREVIOUS; it can be removed"
                    ),
                    n => info!(
                        moved = n,
                        previous_key_id = %previous.id_hex(),
                        "secrets at rest: {n} encrypted again from MARCAN_SECRET_KEY_PREVIOUS to MARCAN_SECRET_KEY; MARCAN_SECRET_KEY_PREVIOUS can be removed"
                    ),
                }
            }
            if sealed + moved > 0 {
                // The old values are still in the pages the update freed
                // (and, after an upgrade from 0.3, in the ones the
                // passwords' settings rows left behind).
                compact_or_warn(&self.pool).await;
            }
        }
        let snapshot = self.snapshot().await?;
        if self.key().is_none() && snapshot.plain > 0 {
            info!(
                plain = snapshot.plain,
                "MARCAN_SECRET_KEY is not set: account secrets are stored unencrypted"
            );
        }
        let now = Instant::now();
        let mut logged = self
            .lock_logged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for account in &snapshot.accounts {
            if let Some(reason) = snapshot.lock(account.id) {
                log_locked(&account.fields.name, account.id, reason);
                logged.insert(account.id, now);
            }
        }
        Ok(())
    }

    /// Every account and which of its secrets are stored and locked.
    pub async fn snapshot(&self) -> Result<AccountsSnapshot, StoreError> {
        snapshot_of(&self.pool, self.key()).await
    }

    /// Account `id`'s secret of `kind`, opened.
    pub async fn secret(&self, id: i64, kind: SecretKind) -> Result<SecretState, StoreError> {
        Ok(match get_account_secret(&self.pool, id, kind).await? {
            None => SecretState::Missing,
            Some(row) => match secrets::open(self.key(), id, &row) {
                Ok(text) => SecretState::Readable(text),
                Err(reason) => SecretState::Locked(reason),
            },
        })
    }

    /// The configuration of account `id` for the pass about to start: IMAP
    /// with its password, or Graph for a Microsoft login; `None` when it
    /// cannot be read now: not receiving, its login incomplete, or locked
    /// (logged at most once per `poll`). A Microsoft login gets its token
    /// here; no token is an [`AccountError`] the pass records.
    pub async fn mailbox_config(
        &self,
        id: i64,
        poll: Duration,
    ) -> Result<Option<MailboxConfig>, AccountError> {
        let Some(account) = get_account(&self.pool, id).await? else {
            return Ok(None);
        };
        let f = &account.fields;
        if !f.receive_enabled || !reads_mailbox(f) || login_user(f).is_empty() {
            return Ok(None);
        }
        if f.login_kind == LoginKind::Microsoft {
            return Ok(self
                .microsoft_mailbox_config(&account, poll)
                .await?
                .map(MailboxConfig::Graph));
        }
        let password = match self.secret(id, SecretKind::Password).await? {
            SecretState::Readable(password) => password,
            SecretState::Missing => return Ok(None),
            SecretState::Locked(reason) => {
                self.log_locked_pass(&account, reason, poll);
                return Ok(None);
            }
        };
        Ok(Some(MailboxConfig::Imap(ImapConfig {
            id: id.to_string(),
            epoch: account.receive_epoch,
            host: f.imap_host.clone(),
            port: f.imap_port,
            security: imap_security(f.imap_security),
            username: f.username.clone(),
            password,
            folder: f.folder.clone(),
        })))
    }

    /// A locked account's pass: logged, but at most once per `every`.
    fn log_locked_pass(&self, account: &AccountRow, reason: LockReason, every: Duration) {
        let now = Instant::now();
        let mut logged = self
            .lock_logged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = logged
            .get(&account.id)
            .is_none_or(|at| now.duration_since(*at) >= every);
        if due {
            logged.insert(account.id, now);
            log_locked(&account.fields.name, account.id, reason);
        }
    }

    /// The stored sending settings, ready to send: [`SendError::Disabled`]
    /// while sending is off, [`SendError::Incomplete`] (saying what is
    /// missing) while it has no account or the account lacks something,
    /// [`SendError::Locked`] (and logged) while its credential cannot be
    /// read. A Microsoft login sends over Graph with its cached token.
    pub async fn send_config(&self, settings: &AppSettings) -> Result<SendConfig, SendError> {
        let sending = &settings.sending;
        if !sending.enabled {
            return Err(SendError::Disabled);
        }
        let config = self.sending_config(settings, sending).await;
        if let (Err(SendError::Locked { account, reason }), Some(id)) =
            (&config, sending.account_id)
        {
            log_locked(account, id, *reason);
        }
        config
    }

    /// The sending group `sending` (stored, or on screen in Settings →
    /// Sending, on or off) ready to send through its account, or why not:
    /// [`SendError::Incomplete`] without an account or while it lacks
    /// something, [`SendError::Locked`] while its credential cannot be read.
    pub async fn sending_config(
        &self,
        settings: &AppSettings,
        sending: &SendingSettings,
    ) -> Result<SendConfig, SendError> {
        let incomplete = || SendError::Incomplete(NO_SENDING_ACCOUNT.to_owned());
        let id = sending.account_id.ok_or_else(incomplete)?;
        let unreadable = |e: StoreError| SendError::Other {
            message: "The sending account could not be read.".to_owned(),
            detail: e.to_string(),
        };
        let account = get_account(&self.pool, id)
            .await
            .map_err(unreadable)?
            .ok_or_else(incomplete)?;
        let login = self.send_login(&account).await.map_err(unreadable)?;
        send_config_for(&account.fields, login, sending, settings)
    }

    /// What stored account `account` sends with: its password (to its SMTP
    /// server), or its Microsoft login's Graph token at the accounts' Graph
    /// endpoint.
    pub async fn send_login(&self, account: &AccountRow) -> Result<SendLogin, StoreError> {
        Ok(match account.fields.login_kind {
            LoginKind::Password => {
                SendLogin::Password(self.secret(account.id, SecretKind::Password).await?)
            }
            LoginKind::Microsoft => SendLogin::Microsoft {
                api: self.graph.clone(),
                token: self.microsoft_send_token(account).await?,
            },
        })
    }
}

/// Every account and which of its secrets are stored, and which `key`
/// cannot read.
async fn snapshot_of(
    pool: &SqlitePool,
    key: Option<&SecretKey>,
) -> Result<AccountsSnapshot, StoreError> {
    let accounts = list_accounts(pool).await?;
    let mut snapshot = AccountsSnapshot {
        accounts,
        ..AccountsSnapshot::default()
    };
    for (id, row) in list_account_secrets(pool).await? {
        snapshot.stored.insert((id, row.kind));
        snapshot.plain += usize::from(!row.is_encrypted());
        if let Some(reason) = secrets::lock_of(key, &row) {
            snapshot.locks.entry(id).or_insert(reason);
        }
    }
    Ok(snapshot)
}

/// The stalest of the accounts' times: the oldest, or `None` while one has
/// none (or there is no account).
pub fn stalest(times: &[Option<i64>]) -> Option<i64> {
    times
        .iter()
        .copied()
        .try_fold(None, |oldest: Option<i64>, t| {
            let t = t?;
            Some(Some(oldest.map_or(t, |o| o.min(t))))
        })?
}

/// How each receiving account is read, in id order: what the client-report
/// hold ([`dmarc_assess::client_report_hold`]) and the "Account not read"
/// alert ([`dmarc_assess::unread_events`]) judge. Its lock is the one `key`
/// finds (the hold does not look at it).
pub async fn receiving_reads(
    pool: &SqlitePool,
    key: Option<&SecretKey>,
) -> Result<Vec<AccountReading>, StoreError> {
    let snapshot = snapshot_of(pool, key).await?;
    let mut reads = Vec::new();
    for account in snapshot.receiving() {
        let status = receive_pass_status(pool, account.id).await?;
        reads.push(AccountReading {
            account_id: account.id,
            account: account.fields.name.clone(),
            last_read_at: status.last_success_at,
            receiving_since: account.receiving_since,
            locked: snapshot.lock(account.id),
            last_error: status.last_error.map(|(error, _)| error),
        });
    }
    Ok(reads)
}

/// Account `name` (`id`) is locked: logged with the ways out.
fn log_locked(name: &str, id: i64, reason: LockReason) {
    error!(
        account = %name,
        id,
        "account locked: {}",
        locked_sentence(name, reason)
    );
}

/// What a sending account proves itself with, as far as it is known.
pub enum SendLogin {
    /// A password login's stored password.
    Password(SecretState),
    /// A Microsoft login's Graph token, for the Graph endpoint `api`.
    Microsoft {
        api: GraphApi,
        token: MicrosoftToken,
    },
}

/// Where a Microsoft login's Graph token comes from, if it can.
pub enum MicrosoftToken {
    /// The account's cached token, fetched when needed.
    Ready(Arc<dyn GraphToken>),
    /// The login is incomplete; the text says what is missing.
    Missing(&'static str),
    /// Its credential cannot be read with the server's key.
    Locked(LockReason),
}

/// An account's way out (its SMTP server and `login`, or its mailbox over
/// Graph) from its address, with the sending settings' Reply-To, ready to
/// send; [`SendError::Incomplete`] names the first thing missing,
/// [`SendError::Locked`] a secret that cannot be read. The password goes
/// to the account's saved SMTP server only, a Microsoft login's token only
/// to the Graph endpoint.
pub fn send_config_for(
    account: &AccountFields,
    login: SendLogin,
    sending: &SendingSettings,
    settings: &AppSettings,
) -> Result<SendConfig, SendError> {
    let incomplete = |why: &str| SendError::Incomplete(why.to_owned());
    let locked = |reason| SendError::Locked {
        account: account.name.clone(),
        reason,
    };
    let transport = match login {
        SendLogin::Password(password) => {
            if account.smtp_host.is_empty() {
                return Err(incomplete("Enter the SMTP server's host name."));
            }
            // An unset port is the provider's, as when the account was saved.
            let port =
                presets::preset_of(account).smtp_port(account.smtp_port, account.smtp_security);
            check_port(port)?;
            if account.username.is_empty() {
                return Err(incomplete("Enter the SMTP username."));
            }
            let password = match password {
                SecretState::Readable(password) => password,
                SecretState::Missing => return Err(incomplete("Enter the SMTP password.")),
                SecretState::Locked(reason) => return Err(locked(reason)),
            };
            Transport::Smtp(SmtpServer {
                host: account.smtp_host.clone(),
                port,
                security: account.smtp_security,
                username: account.username.clone(),
                password,
            })
        }
        SendLogin::Microsoft { api, token } => {
            let user = login_user(account);
            if user.is_empty() {
                return Err(incomplete(crate::wording::microsoft::ENTER_MAILBOX));
            }
            let token = match token {
                MicrosoftToken::Ready(token) => token,
                MicrosoftToken::Missing(why) => return Err(incomplete(why)),
                MicrosoftToken::Locked(reason) => return Err(locked(reason)),
            };
            Transport::Graph(GraphSendConfig {
                address: user.to_owned(),
                api,
                token,
            })
        }
    };
    let sender = match account.address.as_str() {
        "" => return Err(incomplete(&no_sender_address(&account.name))),
        from => parse_address(from)
            .ok_or_else(|| SendError::Incomplete(invalid_sender_address(&account.name, from)))?,
    };
    let reply_to = parse_reply_to(&sending.reply_to).map_err(SendError::Incomplete)?;
    Ok(SendConfig {
        transport,
        sender,
        sender_name: Letterhead::new(settings).from_name(),
        reply_to,
    })
}

/// Where test emails and previews go: the stored test recipient, else
/// `sender`, the sending account's address (not validated here).
pub fn test_recipient<'a>(sending: &'a SendingSettings, sender: &'a str) -> &'a str {
    match sending.test_recipient.as_str() {
        "" => sender,
        to => to,
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) mod microsoft_tests;

#[cfg(test)]
mod microsoft_api_tests;
