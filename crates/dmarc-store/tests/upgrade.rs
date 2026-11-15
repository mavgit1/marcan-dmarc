//! Schema upgrades on real old databases: a database made by a released
//! version, opened with the current code, which runs the migrations it lacks.
//! A migration that cannot run on existing rows (a `NOT NULL` column without
//! a default, a table rebuilt without its data, an index the old rows
//! violate) fails here before it reaches anyone's database.
//!
//! The fixtures in `tests/fixtures/` are never edited: one is added per
//! release whose schema or stored settings changed (`vX.Y.Z.db`), made the
//! same way:
//!
//! 1. `cargo run -p dmarc-store --example demo-data -- <dir>` at the release
//!    tag (30 days of reports for four domains through the real ingest).
//! 2. The released image run once against it until `/healthz` answers, so the
//!    released binary itself opens it, stores its DNS and reverse-DNS
//!    snapshots and saves settings through its own API:
//!    `docker run --rm -p 127.0.0.1:18321:8080 -v <dir>:/data ghcr.io/mavgit1/marcan:X.Y.Z`,
//!    then `PUT /api/settings/retention` `{"report_retention_months":24}` and
//!    `PUT /api/settings/branding` `{"operator_name":"Fixture IT"}` (and the
//!    release's own groups, below), then stop it.
//! 3. `sqlite3 <dir>/marcan.db "PRAGMA wal_checkpoint(TRUNCATE); VACUUM;"`,
//!    and only `marcan.db` is kept (no `-wal` or `-shm` file).
//!
//! `v0.2.1.db` was made that way (the demo data at a commit whose code was
//! the tag's), the first release with a frozen schema.
//!
//! `v0.3.2.db` was made that way at the tag, the last release with one
//! mailbox, so the accounts conversion starts from what 0.3.2 really stored.
//! Besides retention and branding, through 0.3.2's API (hosts under
//! `.invalid`, so its one IMAP pass failed and was recorded):
//! `PUT /api/settings/mailbox` `{"host":"imap.fixture.invalid","port":993,
//! "security":"tls","username":"reports@example.com","password":"fixture-imap",
//! "folder":"DMARC","poll_interval_secs":300}`, `PUT /api/settings/sending`
//! `{"enabled":false,"host":"smtp.fixture.invalid","port":587,
//! "security":"starttls","same_login_as_mailbox":false,
//! "username":"mailer@example.net","sender":"alerts@example.com",
//! "reply_to":"","test_recipient":"","password":"fixture-smtp"}` (sending off
//! with its own login) and `PUT /api/settings/reporting`
//! `{"report_address":"dmarc@example.com","reporting_mode":"forwarded"}`.

mod harness;

use dmarc_parser::{PolicyPublished, Record, Report, SpfAuthResult};
use dmarc_store::{
    count_receive_quarantine, delete_account, get_cursor, get_dns_snapshot, get_receive_attempts,
    ingest_report, init, list_account_secrets, list_accounts, list_domain_summaries, list_senders,
    quarantine_receive_message, receive_pass_status, record_receive_attempt, save_settings,
    scope_stats, set_cursor, AccountFields, AccountProvider, AccountRow, AppSettings, ImapSecurity,
    IngestResult, LoginKind, OnboardingState, ReceiveAttemptKey, ReceivingSettings,
    RetentionSettings, Scope, SecretKind, SenderListQuery, SenderSort, SendingSettings,
    SmtpSecurity,
};
use harness::{
    applied_versions, migration_versions, schema, upgrade_from, upgrade_with, Case, V0_3_2,
};
use sqlx::SqlitePool;
use std::path::Path;

/// The fixture `name` copied into a temp dir (the original is never opened
/// for writing) and opened with the current code: every pending migration ran.
async fn open_fixture(name: &str) -> (tempfile::TempDir, SqlitePool) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("marcan.db");
    std::fs::copy(&fixture, &path).expect("copy the fixture");
    let pool = init(&path)
        .await
        .unwrap_or_else(|e| panic!("{name} does not open with the current migrations: {e}"));
    (dir, pool)
}

/// A new report for one of the fixture's domains, written after the upgrade.
fn new_report() -> Report {
    Report {
        report_id: "upgrade-test-1".into(),
        org_name: "upgrade.test".into(),
        email: "dmarc@upgrade.test".into(),
        date_begin: 1_790_000_000,
        date_end: 1_790_086_399,
        policy: PolicyPublished {
            domain: "example.com".into(),
            adkim: Some("s".into()),
            aspf: Some("s".into()),
            p: Some("reject".into()),
            sp: None,
            pct: Some(100),
            fo: None,
        },
        records: vec![Record {
            source_ip: "192.0.2.77".into(),
            count: 5,
            disposition: Some("reject".into()),
            dkim_result: Some("fail".into()),
            spf_result: Some("fail".into()),
            header_from: Some("example.com".into()),
            envelope_from: Some("spoof.example".into()),
            envelope_to: None,
            dkim_auth: vec![],
            spf_auth: vec![SpfAuthResult {
                domain: "spoof.example".into(),
                result: "fail".into(),
                scope: Some("mfrom".into()),
            }],
            policy_overrides: vec![],
        }],
        raw_xml: "<feedback/>".into(),
    }
}

/// What a fixture's demo data counts to, read by today's queries.
struct Expected {
    total_messages: i64,
    compliant_messages: i64,
}

/// The checks every fixture passes once upgraded: every migration applied,
/// the old rows and settings read as they were, and new rows and settings
/// written over them.
async fn check_upgraded(pool: &SqlitePool, release: &str, expected: Expected) {
    assert_eq!(applied_versions(pool).await, migration_versions());
    let fresh_dir = tempfile::tempdir().expect("tempdir");
    let fresh = init(&fresh_dir.path().join("fresh.db"))
        .await
        .expect("fresh");
    assert_eq!(schema(pool).await, schema(&fresh).await, "{release}");
    fresh.close().await;
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .expect("foreign key check");
    assert!(broken.is_empty(), "{release}: {broken:?}");

    // The reports as the release stored them, counted by today's queries.
    let stats = scope_stats(pool, Scope::ALL).await.expect("stats");
    assert_eq!(stats.report_count, 270);
    assert_eq!(stats.mail.total_messages, expected.total_messages);
    assert_eq!(stats.mail.compliant_messages, expected.compliant_messages);
    assert_eq!(stats.active_domains, 3);
    assert_eq!(stats.unique_senders, 11);

    let domains = list_domain_summaries(pool, Scope::ALL)
        .await
        .expect("domains");
    let names: Vec<&str> = domains.iter().map(|d| d.domain.as_str()).collect();
    assert_eq!(
        names,
        [
            "example.com",
            "example.net",
            "example.org",
            "legacy.example.net"
        ]
    );

    let senders = SenderListQuery {
        scope: Scope::ALL,
        failing: false,
        only_ips: None,
        sort: SenderSort::DEFAULT,
        limit: 50,
        offset: 0,
    };
    let (_, senders_total) = list_senders(pool, &senders).await.expect("senders");
    assert_eq!(senders_total, 11);

    let snapshot = get_dns_snapshot(pool, "example.com")
        .await
        .expect("dns snapshot")
        .unwrap_or_else(|| panic!("stored by {release}"));
    assert_eq!(snapshot.grade, "A");

    // The settings every fixture saved.
    let settings = AppSettings::load(pool).await.expect("settings");
    assert_eq!(settings.retention.report_retention_months, 24);
    assert_eq!(settings.branding.operator_name, "Fixture IT");
    assert_eq!(settings.onboarding.state, Some(OnboardingState::Completed));

    // Written after the upgrade: an old report again is a duplicate, a new one
    // counts with the rest, and a settings group saves over the old row.
    let (org_name, report_id): (String, String) =
        sqlx::query_as("SELECT org_name, report_id FROM reports ORDER BY id LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("an old report");
    let mut again = new_report();
    again.org_name = org_name;
    again.report_id = report_id;
    assert!(matches!(
        ingest_report(pool, &again).await.expect("ingest"),
        IngestResult::Duplicate { .. }
    ));
    assert!(matches!(
        ingest_report(pool, &new_report()).await.expect("ingest"),
        IngestResult::Inserted { .. }
    ));
    let stats = scope_stats(pool, Scope::ALL).await.expect("stats");
    assert_eq!(stats.report_count, 271);
    assert_eq!(stats.mail.total_messages, expected.total_messages + 5);

    save_settings(
        pool,
        &RetentionSettings {
            report_retention_months: 36,
        },
    )
    .await
    .expect("save");
    let settings = AppSettings::load(pool).await.expect("settings");
    assert_eq!(settings.retention.report_retention_months, 36);
    assert_eq!(settings.branding.operator_name, "Fixture IT");
}

/// The stored JSON of settings group `key`, as the release wrote it.
async fn settings_row(pool: &SqlitePool, key: &str) -> serde_json::Value {
    let value: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("settings row {key}: {e}"));
    serde_json::from_str(&value).expect("settings JSON")
}

/// The v0.2.1 database: its report address, now account 1's, and the groups
/// added since at their defaults.
#[tokio::test]
async fn a_v0_2_1_database_opens_and_keeps_its_data() {
    let (_dir, pool) = open_fixture("v0.2.1.db").await;
    let settings = AppSettings::load(&pool).await.expect("settings");
    assert_eq!(settings_value(&pool, "reporting").await, None);
    // No mailbox, a report address: account 1 with that address and no login.
    let accounts = list_accounts(&pool).await.expect("accounts");
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(accounts[0].fields, report_address_only());
    assert!(list_account_secrets(&pool).await.unwrap().is_empty());
    assert_eq!(settings.receiving, ReceivingSettings::default());
    assert_eq!(settings.sending, SendingSettings::default());
    // 0.2.1 had no version check, so no `updates` row: it reads as on.
    assert!(settings.updates.check);

    check_upgraded(
        &pool,
        "v0.2.1",
        Expected {
            total_messages: 33_855,
            compliant_messages: 31_461,
        },
    )
    .await;
}

/// The v0.3.2 database: its mailbox, sending (off, with its own login) and
/// forwarded reporting rows as 0.3.2 saved them, converted: account 1 with
/// the mailbox and the report address (no forwarded addresses: every domain's
/// stored DNS check names the report address itself), account 2 "Sending"
/// with the own login, sending off through account 2, and no reporting row.
#[tokio::test]
async fn a_v0_3_2_database_opens_and_keeps_its_data() {
    let (_dir, pool) = open_fixture("v0.3.2.db").await;
    let accounts = list_accounts(&pool).await.expect("accounts");
    assert_eq!(ids(&accounts), [1, 2], "{accounts:?}");
    assert_eq!(
        accounts[0].fields,
        AccountFields {
            imap_host: "imap.fixture.invalid".into(),
            folder: "DMARC".into(),
            ..mailbox_account()
        }
    );
    assert_eq!(
        accounts[1].fields,
        AccountFields {
            smtp_host: "smtp.fixture.invalid".into(),
            ..own_login_account()
        }
    );
    assert_eq!(
        secrets(&pool).await,
        [
            (1, SecretKind::Password, "fixture-imap".to_owned()),
            (2, SecretKind::Password, "fixture-smtp".to_owned())
        ]
    );
    let settings = AppSettings::load(&pool).await.expect("settings");
    assert_eq!(
        settings.sending,
        SendingSettings {
            enabled: false,
            account_id: Some(2),
            ..SendingSettings::default()
        }
    );
    assert_eq!(
        settings_row(&pool, "receiving").await,
        serde_json::json!({ "poll_interval_secs": 300 })
    );
    assert_eq!(settings.receiving.poll_interval_secs, 300);
    assert_eq!(accounts[0].fields.address, "dmarc@example.com");
    assert!(accounts[0].fields.forwarded_from.is_empty());
    assert_eq!(settings_value(&pool, "reporting").await, None);
    assert_eq!(settings_value(&pool, "mailbox").await, None);
    // Its one recorded pass (it failed) is account 1's.
    let passes = receive_pass_status(&pool, 1).await.expect("passes");
    assert!(passes.last_error.is_some());

    check_upgraded(
        &pool,
        "v0.3.2",
        Expected {
            total_messages: 32_730,
            compliant_messages: 30_401,
        },
    )
    .await;
}

/// Account 1 as converted from the cases' mailbox (and report address).
fn mailbox_account() -> AccountFields {
    AccountFields {
        name: "dmarc@example.com".into(),
        provider: AccountProvider::Generic,
        address: "dmarc@example.com".into(),
        login_kind: LoginKind::Password,
        username: "reports@example.com".into(),
        imap_host: "imap.example.com".into(),
        imap_port: 993,
        imap_security: ImapSecurity::Tls,
        receive_enabled: true,
        folder: "INBOX".into(),
        forwarded_from: Vec::new(),
        ..AccountFields::default()
    }
}

/// Account 2 as converted from the cases' own-login sending.
fn own_login_account() -> AccountFields {
    AccountFields {
        name: "Sending".into(),
        address: "alerts@example.com".into(),
        username: "mailer@example.net".into(),
        smtp_host: "smtp.example.com".into(),
        smtp_port: Some(587),
        smtp_security: SmtpSecurity::StartTls,
        receive_enabled: false,
        ..AccountFields::default()
    }
}

/// Account 1 with a report address alone: no login, no server.
fn report_address_only() -> AccountFields {
    AccountFields {
        name: "dmarc@example.com".into(),
        address: "dmarc@example.com".into(),
        ..AccountFields::default()
    }
}

/// Every stored secret, which the conversion stores plain.
async fn secrets(pool: &SqlitePool) -> Vec<(i64, SecretKind, String)> {
    list_account_secrets(pool)
        .await
        .expect("secrets")
        .into_iter()
        .map(|(id, s)| {
            assert!(!s.is_encrypted());
            (id, s.kind, String::from_utf8(s.value).expect("utf-8"))
        })
        .collect()
}

/// The stored JSON of settings group `key`, if there is a row.
async fn settings_value(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .expect("settings row")
}

fn ids(accounts: &[AccountRow]) -> Vec<i64> {
    accounts.iter().map(|a| a.id).collect()
}

/// One settings edge case from 0.3.2: every later migration runs over its
/// rows and leaves the schema a fresh database has. The pool, with the temp
/// dir that holds it, is the case's upgraded database to assert on.
async fn upgrades(case: Case) -> (tempfile::TempDir, SqlitePool) {
    let (dir, pool) = upgrade_from(V0_3_2, &case.rows()).await;
    assert_eq!(
        applied_versions(&pool).await,
        migration_versions(),
        "{case:?}"
    );
    let fresh = init(&dir.path().join("fresh.db")).await.expect("fresh");
    assert_eq!(schema(&pool).await, schema(&fresh).await, "{case:?}");
    fresh.close().await;
    // The report address lives on the accounts: no reporting row is left.
    assert_eq!(settings_value(&pool, "reporting").await, None, "{case:?}");
    (dir, pool)
}

#[tokio::test]
async fn upgrades_with_no_mailbox() {
    let (_dir, pool) = upgrades(Case::NoMailbox).await;
    assert!(list_accounts(&pool).await.unwrap().is_empty());
    let settings = AppSettings::load(&pool).await.unwrap();
    assert_eq!(settings.sending, SendingSettings::default());
    assert_eq!(settings.receiving, ReceivingSettings::default());
    assert_eq!(settings_value(&pool, "receiving").await, None);
}

#[tokio::test]
async fn upgrades_with_a_mailbox_only() {
    let (_dir, pool) = upgrades(Case::MailboxOnly).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(accounts[0].fields, mailbox_account());
    assert_eq!(
        secrets(&pool).await,
        [(1, SecretKind::Password, "case-imap".to_owned())]
    );
    let settings = AppSettings::load(&pool).await.unwrap();
    assert_eq!(settings.receiving.poll_interval_secs, 300);
    assert_eq!(settings.sending, SendingSettings::default());
    assert_eq!(settings_value(&pool, "mailbox").await, None);
    assert_eq!(settings_value(&pool, "sending").await, None);
}

/// The same login: its server onto account 1, sent through account 1. 0.3's
/// From (the mailbox username, which is not the report address) does not
/// survive: mail comes from account 1's address.
#[tokio::test]
async fn upgrades_with_sending_on_the_mailbox_login() {
    let (_dir, pool) = upgrades(Case::SameLoginSending).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(
        accounts[0].fields,
        AccountFields {
            smtp_host: "smtp.example.com".into(),
            smtp_port: Some(587),
            smtp_security: SmtpSecurity::StartTls,
            ..mailbox_account()
        }
    );
    assert_eq!(
        secrets(&pool).await,
        [(1, SecretKind::Password, "case-imap".to_owned())]
    );
    assert_eq!(
        AppSettings::load(&pool).await.unwrap().sending,
        SendingSettings {
            enabled: true,
            account_id: Some(1),
            ..SendingSettings::default()
        }
    );
    assert_eq!(settings_row(&pool, "sending").await.get("sender"), None);
}

/// No SMTP port saved: the port 0.3 sent through (587 with STARTTLS) is
/// written, so the converted server is the one the stored password was
/// saved for and every later save compares ports that are set.
#[tokio::test]
async fn upgrades_sending_without_a_port_to_the_usual_one() {
    let (_dir, pool) = upgrades(Case::SendingWithoutPort).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(accounts[0].fields.smtp_port, Some(587));
    assert_eq!(accounts[0].fields.smtp_security, SmtpSecurity::StartTls);

    // An own login without a port: account 2 gets it too.
    let mut own = Case::OwnLoginSendingOff.rows();
    own[2].1["port"] = serde_json::Value::Null;
    own[2].1["security"] = serde_json::json!("tls");
    let (_dir, pool) = harness::upgrade_from(V0_3_2, &own).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(accounts[1].fields.smtp_port, Some(465));
}

/// Account ids are never reused: a deleted account's id does not pass to
/// the next one (what is kept per account id cannot carry over).
#[tokio::test]
async fn account_ids_are_not_reused_after_the_upgrade() {
    let (_dir, pool) = upgrades(Case::OwnLoginSendingOff).await;
    dmarc_store::delete_account(&pool, 2).await.unwrap();
    let id = dmarc_store::insert_account(&pool, None, &AccountFields::default(), &[], 0)
        .await
        .unwrap();
    assert_eq!(id, 3);
}

/// Its own login: account 2 "Sending" (receiving nothing, its address the
/// sender, so no override), sending still off through it.
#[tokio::test]
async fn upgrades_with_sending_off_on_its_own_login() {
    let (_dir, pool) = upgrades(Case::OwnLoginSendingOff).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1, 2]);
    assert_eq!(accounts[0].fields, mailbox_account());
    assert_eq!(accounts[1].fields, own_login_account());
    assert_eq!(
        secrets(&pool).await,
        [
            (1, SecretKind::Password, "case-imap".to_owned()),
            (2, SecretKind::Password, "case-smtp".to_owned())
        ]
    );
    assert_eq!(
        AppSettings::load(&pool).await.unwrap().sending,
        SendingSettings {
            enabled: false,
            account_id: Some(2),
            ..SendingSettings::default()
        }
    );
}

/// Forwarded mode with no stored DNS check: account 1 keeps the report
/// address and has no forwarded addresses, and the mode goes with the
/// reporting row.
#[tokio::test]
async fn upgrades_in_forwarded_mode() {
    let (_dir, pool) = upgrades(Case::ForwardedMode).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(accounts[0].fields, mailbox_account());
    assert_eq!(accounts[0].fields.address, "dmarc@example.com");
    assert!(accounts[0].fields.forwarded_from.is_empty());
    assert_eq!(settings_value(&pool, "reporting").await, None);
}

/// 0.3.2 rows for domains with their latest DNS check, in 0.3.2's schema:
/// each `(domain, ignored, report_json)`; the check's DMARC result is shaped
/// as 0.3.2 stored it (`dmarc.parsed.rua`, one object per URI).
fn domains_with_checks(domains: &[(&str, bool, &str)]) -> Vec<String> {
    let mut sql = Vec::new();
    for (i, (domain, ignored, report_json)) in domains.iter().enumerate() {
        let id = i + 1;
        sql.push(format!(
            "INSERT INTO domains (id, organization_id, domain, ignored) \
             VALUES ({id}, 1, '{domain}', {})",
            i32::from(*ignored)
        ));
        sql.push(format!(
            "INSERT INTO dns_snapshots (domain_id, checked_at, grade, score, critical_count, \
             dmarc_record_count, report_json, checker_version) \
             VALUES ({id}, 1, 'C', 50, 0, 1, '{}', 1)",
            report_json.replace('\'', "''")
        ));
    }
    sql
}

/// A stored DNS check whose DMARC record's `rua=` names `uris`.
fn check_naming(domain: &str, uris: &[&str]) -> String {
    let rua: Vec<serde_json::Value> = uris
        .iter()
        .map(|uri| {
            let (scheme, address) = uri.split_once(':').unwrap();
            let mailto = scheme == "mailto";
            let host = address.split_once('@').map(|(_, h)| h);
            serde_json::json!({
                "raw": uri, "scheme": scheme,
                "address": mailto.then_some(address), "host": host,
                "size_limit": null, "valid": mailto
            })
        })
        .collect();
    let record = format!("v=DMARC1; p=none; rua={}", uris.join(","));
    serde_json::json!({
        "domain": domain,
        "dmarc": {
            "domain": domain, "name": format!("_dmarc.{domain}"), "lookup": "ok",
            "records": [record], "record_count": 1,
            "parsed": { "v": "DMARC1", "p": "none", "rua": rua, "ruf": [] }
        }
    })
    .to_string()
}

/// 0.3's forwarded mode: each client domain's record names an address of
/// its own that forwards to the mailbox. Upgrading puts every address the
/// monitored domains' latest DNS checks name in `rua=` onto account 1, once
/// each (case folded), leaving out the accounts' own addresses, ignored
/// domains, other schemes and checks that cannot be read: nothing to
/// re-enter. Central mode, or no mailbox, adds nothing.
#[tokio::test]
async fn upgrades_in_forwarded_mode_keep_each_domains_forwarder() {
    let checks = [
        check_naming(
            "client1.example",
            &["mailto:dmarc@client1.example", "mailto:DMARC@example.com"],
        ),
        check_naming("client2.example", &["mailto:Dmarc@Client2.example"]),
        // Another domain of the same client, through the same forwarder.
        check_naming(
            "shop.client2.example",
            &["mailto:dmarc@client2.example", "https://reports.example/x"],
        ),
        // The sending account's own address is no forwarder.
        check_naming("client3.example", &["mailto:alerts@example.com"]),
        check_naming("removed.example", &["mailto:dmarc@removed.example"]),
        // No DMARC record found: nothing parsed.
        serde_json::json!({ "domain": "bare.example", "dmarc": { "parsed": null } }).to_string(),
        "not json".to_owned(),
    ];
    let domains = [
        ("client1.example", false, checks[0].as_str()),
        ("client2.example", false, checks[1].as_str()),
        ("shop.client2.example", false, checks[2].as_str()),
        ("client3.example", false, checks[3].as_str()),
        ("removed.example", true, checks[4].as_str()),
        ("bare.example", false, checks[5].as_str()),
        ("broken.example", false, checks[6].as_str()),
    ];
    let sql = domains_with_checks(&domains);
    let sql: Vec<&str> = sql.iter().map(String::as_str).collect();
    let forwarded = |mut rows: Vec<(&'static str, serde_json::Value)>| {
        for row in &mut rows {
            if row.0 == "reporting" {
                row.1["reporting_mode"] = serde_json::json!("forwarded");
            }
        }
        rows
    };

    let rows = forwarded(Case::OwnLoginSendingOff.rows());
    let (_dir, pool) = upgrade_with(V0_3_2, &rows, &sql).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1, 2]);
    assert_eq!(
        accounts[0].fields.forwarded_from,
        ["dmarc@client1.example", "Dmarc@Client2.example"]
    );
    assert!(accounts[1].fields.forwarded_from.is_empty());
    assert_eq!(settings_value(&pool, "reporting").await, None);
    let fresh_dir = tempfile::tempdir().unwrap();
    let fresh = init(&fresh_dir.path().join("fresh.db")).await.unwrap();
    assert_eq!(schema(&pool).await, schema(&fresh).await);

    // Central mode: the domains name the report address; nothing is added.
    let (_dir, pool) = upgrade_with(V0_3_2, &Case::OwnLoginSendingOff.rows(), &sql).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert!(accounts.iter().all(|a| a.fields.forwarded_from.is_empty()));

    // Forwarded mode without a mailbox or report address: no account 1.
    let rows = [(
        "reporting",
        serde_json::json!({ "report_address": null, "reporting_mode": "forwarded" }),
    )];
    let (_dir, pool) = upgrade_with(V0_3_2, &rows, &sql).await;
    assert!(list_accounts(&pool).await.unwrap().is_empty());
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM account_forwarded_addresses")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn upgrades_with_a_report_address_and_no_mailbox() {
    let (_dir, pool) = upgrades(Case::ReportAddressWithoutMailbox).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(ids(&accounts), [1]);
    assert_eq!(accounts[0].fields, report_address_only());
    assert!(secrets(&pool).await.is_empty());
}

/// The receive state of the converted mailbox (its key: the host case
/// folded, the port, the username) goes to account 1, keyed in text in
/// IMAP's decimal spelling (UIDs and UIDVALIDITY up to 2^32 - 1); the rows of
/// a mailbox configured before it are dropped, and so are passes with no
/// account. The carried rows are the ones a pass writes to and keep their
/// rules: another generation's go when the cursor moves to it, the
/// account's go with it.
#[tokio::test]
async fn the_receive_state_follows_the_mailbox_to_account_1() {
    let key = "imap.example.com|993|reports@example.com";
    let orphan = "old.example.net|143|someone";
    let mut sql = Vec::new();
    for mailbox in [key, orphan] {
        sql.push(format!(
            "INSERT INTO imap_cursor (mailbox, folder, uid_validity, last_uid) VALUES \
             ('{mailbox}', 'INBOX', 7, 42), ('{mailbox}', 'DMARC', 4294967295, 4294967290)"
        ));
        sql.push(format!(
            "INSERT INTO imap_quarantine (mailbox, folder, uid_validity, uid, last_failed_at, error) \
             VALUES ('{mailbox}', 'INBOX', 7, 3, 1, 'bad'), \
             ('{mailbox}', 'INBOX', 7, 4000000000, 2, 'too big')"
        ));
        sql.push(format!(
            "INSERT INTO imap_attempts (mailbox, folder, uid_validity, kind, uid, attempts, last_failed_at) \
             VALUES ('{mailbox}', 'INBOX', 7, 'message', 43, 2, 1), \
             ('{mailbox}', 'INBOX', 7, 'batch', 50, 1, 1)"
        ));
    }
    sql.push(
        "INSERT INTO imap_passes (finished_at, checked_at, error) VALUES \
         (100, 99, NULL), (120, NULL, 'refused')"
            .into(),
    );
    let sql: Vec<&str> = sql.iter().map(String::as_str).collect();
    let mut rows = Case::MailboxOnly.rows();
    // As typed in 0.3: the host's case, the port left to the default.
    rows[0].1["host"] = serde_json::json!("IMAP.Example.com");
    rows[0].1["port"] = serde_json::Value::Null;
    let (_dir, pool) = upgrade_with(V0_3_2, &rows, &sql).await;

    let cursor = |generation: &str, position: &str| Some((generation.into(), position.into()));
    assert_eq!(
        get_cursor(&pool, 1, "INBOX").await.unwrap(),
        cursor("7", "42")
    );
    assert_eq!(
        get_cursor(&pool, 1, "DMARC").await.unwrap(),
        cursor("4294967295", "4294967290")
    );
    let quarantine: Vec<(i64, String, String, String, i64, String)> = sqlx::query_as(
        "SELECT account_id, folder, generation, message_id, last_failed_at, error \
         FROM receive_quarantine ORDER BY last_failed_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let row = |id: &str, at: i64, error: &str| {
        (
            1,
            "INBOX".to_owned(),
            "7".to_owned(),
            id.to_owned(),
            at,
            error.to_owned(),
        )
    };
    assert_eq!(
        quarantine,
        [row("3", 1, "bad"), row("4000000000", 2, "too big")]
    );
    let key = |kind, message_id| ReceiveAttemptKey {
        account_id: 1,
        folder: "INBOX",
        generation: "7",
        kind,
        message_id,
    };
    let (message, batch) = (key("message", "43"), key("batch", "50"));
    assert_eq!(get_receive_attempts(&pool, &message).await.unwrap(), 2);
    assert_eq!(get_receive_attempts(&pool, &batch).await.unwrap(), 1);
    let passes = receive_pass_status(&pool, 1).await.unwrap();
    assert_eq!(passes.last_success_at, Some(100));
    assert_eq!(passes.last_error, Some(("refused".into(), 120)));
    assert_eq!(passes.last_error_detail, None);
    let account = &list_accounts(&pool).await.unwrap()[0];
    assert_eq!(account.fields.imap_host, "IMAP.Example.com");
    assert_eq!(account.fields.imap_port, 993);

    // It starts at epoch 0, and a pass prepared under it writes to the
    // carried rows: the count goes on, the skipped message keeps its one
    // row, the cursor moves.
    assert_eq!(account.receive_epoch, 0);
    assert_eq!(
        record_receive_attempt(&pool, &message, 200, 0)
            .await
            .unwrap(),
        3
    );
    assert!(
        quarantine_receive_message(&pool, 1, "INBOX", "7", "3", "bad again", 200, 0)
            .await
            .unwrap()
    );
    assert_eq!(
        count_receive_quarantine(&pool, Some(1), Some("INBOX"))
            .await
            .unwrap(),
        2
    );
    assert!(set_cursor(&pool, 1, "INBOX", "7", "44", 0).await.unwrap());
    assert_eq!(
        get_cursor(&pool, 1, "INBOX").await.unwrap(),
        cursor("7", "44")
    );
    assert_eq!(get_receive_attempts(&pool, &batch).await.unwrap(), 1);
    // Another generation: this folder's skipped messages and counts go.
    assert!(set_cursor(&pool, 1, "INBOX", "8", "0", 0).await.unwrap());
    assert_eq!(get_receive_attempts(&pool, &message).await.unwrap(), 0);
    assert_eq!(get_receive_attempts(&pool, &batch).await.unwrap(), 0);
    assert_eq!(
        count_receive_quarantine(&pool, Some(1), None)
            .await
            .unwrap(),
        0
    );

    // Foreign keys hold, and the account takes its rows with it.
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
    assert!(delete_account(&pool, 1).await.unwrap());
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM account_secrets) \
         + (SELECT COUNT(*) FROM receive_cursor) + (SELECT COUNT(*) FROM receive_quarantine) \
         + (SELECT COUNT(*) FROM receive_attempts) + (SELECT COUNT(*) FROM receive_passes)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);

    // Without a mailbox there is no account, so no state either.
    let (_dir, pool) = upgrade_with(V0_3_2, &Case::NoMailbox.rows(), &sql).await;
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM receive_cursor) + (SELECT COUNT(*) FROM receive_quarantine) \
         + (SELECT COUNT(*) FROM receive_attempts) + (SELECT COUNT(*) FROM receive_passes)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

/// The From 0.3 used becomes the address of its own login's account, unless
/// another account has it already; no From is kept besides the accounts'.
#[tokio::test]
async fn the_0_3_sender_becomes_an_account_address_where_it_can() {
    // Its own login sending as the report address: account 2 gets no
    // address (account 1 has it), and the sender goes.
    let mut rows = Case::OwnLoginSendingOff.rows();
    rows[2].1["sender"] = serde_json::json!("DMARC@example.com");
    let (_dir, pool) = upgrade_from(V0_3_2, &rows).await;
    let accounts = list_accounts(&pool).await.unwrap();
    assert_eq!(accounts[1].fields.address, "");
    assert_eq!(settings_row(&pool, "sending").await.get("sender"), None);

    // The mailbox's login with a From of its own: the From goes and every
    // other sending setting stays.
    let mut rows = Case::SameLoginSending.rows();
    rows[2].1["sender"] = serde_json::json!("alerts@example.com");
    rows[2].1["reply_to"] = serde_json::json!("help@example.com");
    rows[2].1["test_recipient"] = serde_json::json!("me@example.com");
    let (_dir, pool) = upgrade_from(V0_3_2, &rows).await;
    assert_eq!(
        settings_row(&pool, "sending").await,
        serde_json::json!({
            "enabled": true,
            "account_id": 1,
            "reply_to": "help@example.com",
            "test_recipient": "me@example.com"
        })
    );
    assert_eq!(
        list_accounts(&pool).await.unwrap()[0].fields.address,
        "dmarc@example.com"
    );

    // Its own login without a sender: the username, an address, is account
    // 2's address.
    let mut rows = Case::OwnLoginSendingOff.rows();
    rows[2].1["sender"] = serde_json::json!("");
    let (_dir, pool) = upgrade_from(V0_3_2, &rows).await;
    assert_eq!(
        list_accounts(&pool).await.unwrap()[1].fields.address,
        "mailer@example.net"
    );

    // Sending with no server has no account, and its host goes nowhere.
    let mut rows = Case::SameLoginSending.rows();
    rows[2].1["host"] = serde_json::json!("");
    let (_dir, pool) = upgrade_from(V0_3_2, &rows).await;
    let sending = AppSettings::load(&pool).await.unwrap().sending;
    assert_eq!(sending.account_id, None);
    assert!(sending.enabled);
    assert_eq!(list_accounts(&pool).await.unwrap()[0].fields.smtp_host, "");
}
