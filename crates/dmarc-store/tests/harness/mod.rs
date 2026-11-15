//! Migration tests from known settings: a fresh database migrated up to a
//! version, `app_settings` rows written as that version stored them, then
//! opened by the current code ([`dmarc_store::init`]), which applies the
//! remaining migrations exactly as it does at start. The fixtures show what
//! a release really stored; these cases cover the shapes no fixture holds.

use serde_json::{json, Value};
use sqlx::{migrate::Migrator, sqlite::SqliteConnectOptions, SqlitePool};
use std::path::{Path, PathBuf};

/// The last migration 0.3.x applied: the cases' rows are 0.3.2's shapes.
pub const V0_3_2: i64 = 3;

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn version_of(file_name: &str) -> Option<i64> {
    file_name
        .strip_suffix(".sql")?
        .split('_')
        .next()?
        .parse()
        .ok()
}

/// The migrations in `migrations/`, by version.
pub fn migration_versions() -> Vec<i64> {
    let mut versions: Vec<i64> = std::fs::read_dir(migrations_dir())
        .expect("migrations dir")
        .filter_map(|e| version_of(&e.ok()?.file_name().into_string().ok()?))
        .collect();
    versions.sort_unstable();
    versions
}

/// The migrations a database has applied, by version.
pub async fn applied_versions(pool: &SqlitePool) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success ORDER BY version")
        .fetch_all(pool)
        .await
        .expect("applied migrations")
}

/// A database's schema (not sqlx's own table): every table, index and
/// their SQL, by type and name. An upgraded database ran the same
/// migrations as a fresh one, so the two match to the text.
pub async fn schema(pool: &SqlitePool) -> Vec<(String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT type, name, sql FROM sqlite_master \
         WHERE name NOT LIKE '_sqlx_migrations%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("schema")
}

/// A database at migration `version` with `rows` (key, JSON value) as its
/// `app_settings`, then opened by the current code: every later migration
/// ran over those rows. The temp dir holds the database.
pub async fn upgrade_from(version: i64, rows: &[(&str, Value)]) -> (tempfile::TempDir, SqlitePool) {
    upgrade_with(version, rows, &[]).await
}

/// [`upgrade_from`], with the statements `sql` run on the old database after
/// its settings rows (rows of other tables, in that version's schema).
pub async fn upgrade_with(
    version: i64,
    rows: &[(&str, Value)],
    sql: &[&str],
) -> (tempfile::TempDir, SqlitePool) {
    // The migrations up to `version`, byte for byte: the checksums the
    // embedded migrator expects, so `init` takes them as applied.
    let dir = tempfile::tempdir().expect("tempdir");
    let older = dir.path().join("migrations");
    std::fs::create_dir(&older).expect("migrations copy");
    for entry in std::fs::read_dir(migrations_dir())
        .expect("migrations dir")
        .flatten()
    {
        let name = entry.file_name().into_string().expect("utf-8 name");
        if version_of(&name).is_some_and(|v| v <= version) {
            std::fs::copy(entry.path(), older.join(&name)).expect("copy migration");
        }
    }

    let path = dir.path().join("marcan.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let old = SqlitePool::connect_with(options).await.expect("old pool");
    Migrator::new(older.as_path())
        .await
        .expect("older migrations")
        .run(&old)
        .await
        .unwrap_or_else(|e| panic!("migrations up to {version}: {e}"));
    for (key, value) in rows {
        sqlx::query("INSERT INTO app_settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value.to_string())
            .execute(&old)
            .await
            .unwrap_or_else(|e| panic!("settings row {key}: {e}"));
    }
    for statement in sql {
        sqlx::query(statement)
            .execute(&old)
            .await
            .unwrap_or_else(|e| panic!("{statement}: {e}"));
    }
    old.close().await;

    let pool = dmarc_store::init(&path)
        .await
        .unwrap_or_else(|e| panic!("the migrations after {version}: {e}"));
    (dir, pool)
}

/// The accounts conversion's settings edge cases, as 0.3.2 stored them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// Nothing saved: no mailbox, no sending, no report address.
    NoMailbox,
    /// A mailbox and its report address; sending never saved.
    MailboxOnly,
    /// Sending on, logging in as the mailbox.
    SameLoginSending,
    /// Sending off, with its own login and a sender that is not the username.
    OwnLoginSendingOff,
    /// Sending on the mailbox login with no port saved (0.3 sent through the
    /// usual one for the security), and an own-login sender without one.
    SendingWithoutPort,
    /// A mailbox whose domains forward their reports to it.
    ForwardedMode,
    /// A report address saved with no mailbox.
    ReportAddressWithoutMailbox,
}

impl Case {
    /// The case's `app_settings` rows.
    pub fn rows(self) -> Vec<(&'static str, Value)> {
        let mailbox = json!({
            "host": "imap.example.com", "port": 993, "security": "tls",
            "username": "reports@example.com", "password": "case-imap",
            "folder": "INBOX", "poll_interval_secs": 300
        });
        let reporting =
            |mode: &str| json!({ "report_address": "dmarc@example.com", "reporting_mode": mode });
        let sending = |enabled: bool, same: bool| {
            let own = |value: &'static str| if same { "" } else { value };
            json!({
                "enabled": enabled, "host": "smtp.example.com", "port": 587,
                "security": "starttls", "same_login_as_mailbox": same,
                "username": own("mailer@example.net"), "sender": own("alerts@example.com"),
                "reply_to": "", "test_recipient": "", "password": own("case-smtp")
            })
        };
        let (m, direct) = (("mailbox", mailbox), ("reporting", reporting("direct")));
        match self {
            Self::NoMailbox => vec![],
            Self::MailboxOnly => vec![m, direct],
            Self::SameLoginSending => vec![m, direct, ("sending", sending(true, true))],
            Self::OwnLoginSendingOff => vec![m, direct, ("sending", sending(false, false))],
            Self::SendingWithoutPort => {
                let mut no_port = sending(true, true);
                no_port["port"] = Value::Null;
                vec![m, direct, ("sending", no_port)]
            }
            Self::ForwardedMode => vec![m, ("reporting", reporting("forwarded"))],
            Self::ReportAddressWithoutMailbox => vec![direct],
        }
    }
}
