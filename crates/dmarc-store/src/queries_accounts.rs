//! The mail accounts (`accounts`) and their secrets (`account_secrets`): the
//! rows as stored, judging nothing. Whether an account is ready to receive or
//! send, and what a secret's key means, is the server's to say.
//!
//! A secret is a [`SecretRow`]: its bytes plain, or sealed with the nonce and
//! the fingerprint of the key that sealed them; the server seals and opens
//! them. An account's receive state (its cursor, quarantine, retry counts
//! and passes in `queries_receive`) belongs to one mailbox and folder: when the
//! server finds that an update changed which mailbox the account reads, the
//! update forgets it and moves the account to its next `receive_epoch` in
//! its own transaction ([`ReceiveChange::Replaced`]). A pass writes under
//! the epoch it was prepared with, so one still running writes nothing more.

use crate::queries::begin_write;
use crate::{ImapSecurity, SmtpSecurity, StoreError};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use sqlx::{Sqlite, Transaction};
use std::fmt;
use ts_rs::TS;

/// Which provider an account was set up for: its servers' presets and its
/// login choices.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountProvider {
    Microsoft365,
    #[default]
    Generic,
}

impl AccountProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microsoft365 => "microsoft365",
            Self::Generic => "generic",
        }
    }

    fn from_stored(raw: &str) -> Self {
        match raw {
            "microsoft365" => Self::Microsoft365,
            _ => Self::Generic,
        }
    }
}

/// How an account logs in: a username and password, or Microsoft's OAuth 2.0
/// client credentials (an app registration in the tenant).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum LoginKind {
    #[default]
    Password,
    Microsoft,
}

impl LoginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::Microsoft => "microsoft",
        }
    }

    fn from_stored(raw: &str) -> Self {
        match raw {
            "microsoft" => Self::Microsoft,
            _ => Self::Password,
        }
    }
}

/// Which secret of an account a [`SecretRow`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretKind {
    /// The password (or app password) of a password login.
    Password,
    /// The private key of a Microsoft login's certificate.
    PrivateKey,
}

impl SecretKind {
    pub const ALL: [Self; 2] = [Self::Password, Self::PrivateKey];

    /// The stored name (`account_secrets.kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
        }
    }

    fn from_stored(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == raw)
    }
}

/// A Microsoft login's settings; every field `None` for a password login.
/// It proves itself with its certificate (`None` until one is generated).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicrosoftLogin {
    pub tenant_id: Option<String>,
    pub client_id: Option<String>,
    /// The certificate (DER) whose private key is the account's `PrivateKey`.
    pub cert_der: Option<Vec<u8>>,
    pub cert_thumbprint: Option<String>,
    pub cert_expires_at: Option<i64>,
}

/// An account's columns, as written and read; no secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountFields {
    pub name: String,
    pub provider: AccountProvider,
    /// The report address and default From; empty: none.
    pub address: String,
    pub login_kind: LoginKind,
    /// The login of a password login; empty: not set.
    pub username: String,
    pub microsoft: MicrosoftLogin,
    /// Empty: no IMAP server (always for a Microsoft login, which reads
    /// and sends over Microsoft Graph and stores no servers).
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ImapSecurity,
    /// Empty: the account cannot send.
    pub smtp_host: String,
    /// `None`: not set.
    pub smtp_port: Option<u16>,
    pub smtp_security: SmtpSecurity,
    /// Reports are read from it (a password login needs an IMAP host too).
    pub receive_enabled: bool,
    /// The folder reports are read from.
    pub folder: String,
    /// The other addresses whose reports are forwarded to this account, in
    /// the order entered; each belongs to this account alone.
    pub forwarded_from: Vec<String>,
}

impl Default for AccountFields {
    fn default() -> Self {
        Self {
            name: String::new(),
            provider: AccountProvider::Generic,
            address: String::new(),
            login_kind: LoginKind::Password,
            username: String::new(),
            microsoft: MicrosoftLogin::default(),
            imap_host: String::new(),
            imap_port: 993,
            imap_security: ImapSecurity::Tls,
            smtp_host: String::new(),
            smtp_port: None,
            smtp_security: SmtpSecurity::Tls,
            receive_enabled: true,
            folder: "INBOX".to_owned(),
            forwarded_from: Vec::new(),
        }
    }
}

/// One stored account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: i64,
    pub created_at: i64,
    /// When it started reading the mailbox it reads now: when it was
    /// added, its receiving switched on, or a save made it read another
    /// mailbox ([`ReceiveChange`]).
    pub receiving_since: i64,
    /// Which mailbox's receive state it holds: one more each time a save
    /// made it read another mailbox ([`ReceiveChange::Replaced`]). A pass
    /// writes only while the epoch it was prepared under is still this one.
    pub receive_epoch: i64,
    pub fields: AccountFields,
}

/// One stored secret: plain (`nonce` and `key_id` `None`) or sealed. Its
/// value is never printed by [`Debug`](fmt::Debug).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretRow {
    pub kind: SecretKind,
    /// The UTF-8 text when plain; ciphertext and tag when sealed.
    pub value: Vec<u8>,
    /// 12 bytes when sealed.
    pub nonce: Option<Vec<u8>>,
    /// The fingerprint of the key it was sealed with.
    pub key_id: Option<Vec<u8>>,
}

impl SecretRow {
    /// A secret stored as it is.
    pub fn plain(kind: SecretKind, value: &str) -> Self {
        Self {
            kind,
            value: value.as_bytes().to_vec(),
            nonce: None,
            key_id: None,
        }
    }

    pub fn is_encrypted(&self) -> bool {
        self.nonce.is_some()
    }
}

impl fmt::Debug for SecretRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretRow")
            .field("kind", &self.kind)
            .field("value", &"***")
            .field("encrypted", &self.is_encrypted())
            .field("key_id", &self.key_id)
            .finish()
    }
}

/// What a write does to one secret of the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretWrite {
    Keep,
    Set(SecretRow),
    Clear,
}

#[derive(sqlx::FromRow)]
struct RawAccount {
    id: i64,
    name: String,
    provider: String,
    address: String,
    login_kind: String,
    username: String,
    ms_tenant_id: Option<String>,
    ms_client_id: Option<String>,
    ms_cert_der: Option<Vec<u8>>,
    ms_cert_thumbprint: Option<String>,
    ms_cert_expires_at: Option<i64>,
    imap_host: String,
    imap_port: i64,
    imap_security: String,
    smtp_host: String,
    smtp_port: Option<i64>,
    smtp_security: String,
    receive_enabled: bool,
    folder: String,
    /// A JSON array of the forwarded addresses.
    forwarded_from: String,
    created_at: i64,
    receiving_since: i64,
    receive_epoch: i64,
}

impl From<RawAccount> for AccountRow {
    fn from(r: RawAccount) -> Self {
        Self {
            id: r.id,
            created_at: r.created_at,
            receiving_since: r.receiving_since,
            receive_epoch: r.receive_epoch,
            fields: AccountFields {
                name: r.name,
                provider: AccountProvider::from_stored(&r.provider),
                address: r.address,
                login_kind: LoginKind::from_stored(&r.login_kind),
                username: r.username,
                microsoft: MicrosoftLogin {
                    tenant_id: r.ms_tenant_id,
                    client_id: r.ms_client_id,
                    cert_der: r.ms_cert_der,
                    cert_thumbprint: r.ms_cert_thumbprint,
                    cert_expires_at: r.ms_cert_expires_at,
                },
                imap_host: r.imap_host,
                imap_port: u16::try_from(r.imap_port).unwrap_or(993),
                imap_security: ImapSecurity::from_stored(&r.imap_security),
                smtp_host: r.smtp_host,
                smtp_port: r.smtp_port.and_then(|p| u16::try_from(p).ok()),
                smtp_security: SmtpSecurity::from_stored(&r.smtp_security),
                receive_enabled: r.receive_enabled,
                folder: r.folder,
                forwarded_from: serde_json::from_str(&r.forwarded_from).unwrap_or_default(),
            },
        }
    }
}

const ACCOUNT_COLUMNS: &str = "id, name, provider, address, login_kind, username, \
    ms_tenant_id, ms_client_id, ms_cert_der, ms_cert_thumbprint, ms_cert_expires_at, \
    imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, receive_enabled, folder, created_at, receiving_since, \
    receive_epoch, (SELECT json_group_array(address) FROM (SELECT address FROM account_forwarded_addresses AS f \
    WHERE f.account_id = accounts.id ORDER BY f.rowid)) AS forwarded_from";

/// Every account, in id order (the order they were added).
pub async fn list_accounts(pool: &SqlitePool) -> Result<Vec<AccountRow>, StoreError> {
    let rows: Vec<RawAccount> = sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts ORDER BY id"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(AccountRow::from).collect())
}

/// Account `id`, if it exists.
pub async fn get_account(pool: &SqlitePool, id: i64) -> Result<Option<AccountRow>, StoreError> {
    let row: Option<RawAccount> = sqlx::query_as(&format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(AccountRow::from))
}

/// The account whose address, or one of whose forwarded addresses, is
/// `address` (case folded), if any.
pub async fn account_with_address(
    pool: &SqlitePool,
    address: &str,
) -> Result<Option<i64>, StoreError> {
    if address.is_empty() {
        return Ok(None);
    }
    Ok(sqlx::query_scalar(
        "SELECT id FROM accounts WHERE address <> '' AND lower(address) = lower(?1) \
         UNION ALL SELECT account_id FROM account_forwarded_addresses \
         WHERE lower(address) = lower(?1) LIMIT 1",
    )
    .bind(address)
    .fetch_optional(pool)
    .await?)
}

fn bind_fields<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    f: &'q AccountFields,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    let ms = &f.microsoft;
    query
        .bind(&f.name)
        .bind(f.provider.as_str())
        .bind(&f.address)
        .bind(f.login_kind.as_str())
        .bind(&f.username)
        .bind(&ms.tenant_id)
        .bind(&ms.client_id)
        .bind(&ms.cert_der)
        .bind(&ms.cert_thumbprint)
        .bind(ms.cert_expires_at)
        .bind(&f.imap_host)
        .bind(i64::from(f.imap_port))
        .bind(f.imap_security.as_str())
        .bind(&f.smtp_host)
        .bind(f.smtp_port.map(i64::from))
        .bind(f.smtp_security.as_str())
        .bind(f.receive_enabled)
        .bind(&f.folder)
}

/// Add an account (as `id` when given, else the next id) with its secrets,
/// in one transaction; its id.
#[cfg(any(test, feature = "test-support"))]
pub async fn insert_account(
    pool: &SqlitePool,
    id: Option<i64>,
    fields: &AccountFields,
    secrets: &[SecretRow],
    now: i64,
) -> Result<i64, StoreError> {
    let mut tx = begin_write(pool).await?;
    let id = insert_row(&mut tx, id, fields, now).await?;
    for secret in secrets {
        put_secret(&mut tx, id, secret).await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// Add an account with the secrets `secrets` makes for its new id (a sealed
/// secret's id is in its associated data), in one transaction; its id.
pub async fn create_account(
    pool: &SqlitePool,
    fields: &AccountFields,
    now: i64,
    secrets: impl FnOnce(i64) -> Vec<(SecretKind, SecretWrite)>,
) -> Result<i64, StoreError> {
    let mut tx = begin_write(pool).await?;
    let id = insert_row(&mut tx, None, fields, now).await?;
    write_secrets(&mut tx, id, &secrets(id)).await?;
    tx.commit().await?;
    Ok(id)
}

async fn insert_row(
    tx: &mut Transaction<'static, Sqlite>,
    id: Option<i64>,
    fields: &AccountFields,
    now: i64,
) -> Result<i64, StoreError> {
    let query = sqlx::query(
        "INSERT INTO accounts (name, provider, address, login_kind, username, \
         ms_tenant_id, ms_client_id, ms_cert_der, ms_cert_thumbprint, ms_cert_expires_at, \
         imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, \
         receive_enabled, folder, created_at, receiving_since, id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );
    let id = bind_fields(query, fields)
        .bind(now)
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();
    write_forwarded(tx, id, &fields.forwarded_from).await?;
    Ok(id)
}

/// Replace account `id`'s forwarded addresses with `addresses`, in order.
async fn write_forwarded(
    tx: &mut Transaction<'static, Sqlite>,
    id: i64,
    addresses: &[String],
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM account_forwarded_addresses WHERE account_id = ?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    for address in addresses {
        sqlx::query("INSERT INTO account_forwarded_addresses (account_id, address) VALUES (?, ?)")
            .bind(id)
            .bind(address)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// What a save does to the account's reading, as the server decided it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveChange {
    /// It reads the same mailbox: its receive state stays.
    Keep,
    /// Its receiving was switched on at `at` (`receiving_since`); its
    /// receive state stays.
    Started { at: i64 },
    /// It reads another mailbox from `at` (`receiving_since`): its cursor,
    /// quarantine, retry counts and recorded passes are forgotten and its
    /// `receive_epoch` moves on in the save's transaction, so a pass prepared
    /// before writes nothing more.
    Replaced { at: i64 },
}

/// Write account `id`'s columns and its secret changes in one transaction;
/// whether there is such an account. Its receive state stays (the server
/// decides whether the account now reads another mailbox).
pub async fn update_account(
    pool: &SqlitePool,
    id: i64,
    fields: &AccountFields,
    secrets: &[(SecretKind, SecretWrite)],
) -> Result<bool, StoreError> {
    update_account_receiving(pool, id, fields, secrets, ReceiveChange::Keep).await
}

/// [`update_account`], doing `receive` to its receive state in the same
/// transaction: a restart right after the save cannot leave the old
/// mailbox's cursor behind.
pub async fn update_account_receiving(
    pool: &SqlitePool,
    id: i64,
    fields: &AccountFields,
    secrets: &[(SecretKind, SecretWrite)],
    receive: ReceiveChange,
) -> Result<bool, StoreError> {
    let mut tx = begin_write(pool).await?;
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Ok(false);
    }
    let query = sqlx::query(
        "UPDATE accounts SET name = ?, provider = ?, address = ?, login_kind = ?, \
         username = ?, ms_tenant_id = ?, ms_client_id = ?, ms_cert_der = ?, \
         ms_cert_thumbprint = ?, ms_cert_expires_at = ?, imap_host = ?, imap_port = ?, imap_security = ?, \
         smtp_host = ?, smtp_port = ?, smtp_security = ?, receive_enabled = ?, folder = ? \
         WHERE id = ?",
    );
    bind_fields(query, fields)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    write_forwarded(&mut tx, id, &fields.forwarded_from).await?;
    write_secrets(&mut tx, id, secrets).await?;
    if let ReceiveChange::Started { at } | ReceiveChange::Replaced { at } = receive {
        sqlx::query("UPDATE accounts SET receiving_since = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let ReceiveChange::Replaced { .. } = receive {
        sqlx::query("UPDATE accounts SET receive_epoch = receive_epoch + 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for table in [
            "receive_cursor",
            "receive_quarantine",
            "receive_attempts",
            "receive_passes",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE account_id = ?"))
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    Ok(true)
}

async fn write_secrets(
    tx: &mut Transaction<'static, Sqlite>,
    id: i64,
    secrets: &[(SecretKind, SecretWrite)],
) -> Result<(), StoreError> {
    for (kind, write) in secrets {
        match write {
            SecretWrite::Keep => {}
            SecretWrite::Set(secret) => put_secret(tx, id, secret).await?,
            SecretWrite::Clear => {
                sqlx::query("DELETE FROM account_secrets WHERE account_id = ? AND kind = ?")
                    .bind(id)
                    .bind(kind.as_str())
                    .execute(&mut **tx)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Set account `id`'s address alone (empty: none).
#[cfg(any(test, feature = "test-support"))]
pub async fn set_account_address(
    pool: &SqlitePool,
    id: i64,
    address: &str,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE accounts SET address = ? WHERE id = ?")
        .bind(address)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete account `id` with its secrets, receive state and passes; whether
/// it existed.
pub async fn delete_account(pool: &SqlitePool, id: i64) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

async fn put_secret(
    tx: &mut Transaction<'static, Sqlite>,
    id: i64,
    secret: &SecretRow,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO account_secrets (account_id, kind, value, encrypted, nonce, key_id) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(account_id, kind) DO UPDATE SET value = excluded.value, \
         encrypted = excluded.encrypted, nonce = excluded.nonce, key_id = excluded.key_id",
    )
    .bind(id)
    .bind(secret.kind.as_str())
    .bind(&secret.value)
    .bind(secret.is_encrypted())
    .bind(&secret.nonce)
    .bind(&secret.key_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RawSecret {
    account_id: i64,
    kind: String,
    value: Vec<u8>,
    nonce: Option<Vec<u8>>,
    key_id: Option<Vec<u8>>,
}

impl RawSecret {
    fn into_row(self) -> Option<(i64, SecretRow)> {
        Some((
            self.account_id,
            SecretRow {
                kind: SecretKind::from_stored(&self.kind)?,
                value: self.value,
                nonce: self.nonce,
                key_id: self.key_id,
            },
        ))
    }
}

/// Every stored secret with its account id, by account and kind.
pub async fn list_account_secrets(pool: &SqlitePool) -> Result<Vec<(i64, SecretRow)>, StoreError> {
    let rows: Vec<RawSecret> = sqlx::query_as(
        "SELECT account_id, kind, value, nonce, key_id FROM account_secrets \
         ORDER BY account_id, kind",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(RawSecret::into_row).collect())
}

/// Account `id`'s secret of `kind`, if one is stored.
pub async fn get_account_secret(
    pool: &SqlitePool,
    id: i64,
    kind: SecretKind,
) -> Result<Option<SecretRow>, StoreError> {
    let row: Option<RawSecret> = sqlx::query_as(
        "SELECT account_id, kind, value, nonce, key_id FROM account_secrets \
         WHERE account_id = ? AND kind = ?",
    )
    .bind(id)
    .bind(kind.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(RawSecret::into_row).map(|(_, s)| s))
}

/// Replace every stored secret with what `reseal` makes of it (`None`
/// keeps it as it is), in one transaction: the plain ones sealed when a key
/// is added, the ones sealed with the previous key sealed again with the
/// current one. How many were replaced.
pub async fn reseal_secrets(
    pool: &SqlitePool,
    mut reseal: impl FnMut(i64, &SecretRow) -> Option<SecretRow>,
) -> Result<usize, StoreError> {
    let mut tx = begin_write(pool).await?;
    let rows: Vec<RawSecret> = sqlx::query_as(
        "SELECT account_id, kind, value, nonce, key_id FROM account_secrets \
         ORDER BY account_id, kind",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut replaced = 0;
    for (id, stored) in rows.into_iter().filter_map(RawSecret::into_row) {
        let Some(secret) = reseal(id, &stored).filter(|s| s.kind == stored.kind) else {
            continue;
        };
        put_secret(&mut tx, id, &secret).await?;
        replaced += 1;
    }
    tx.commit().await?;
    Ok(replaced)
}
