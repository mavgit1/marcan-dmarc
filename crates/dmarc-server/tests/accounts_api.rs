//! The mail accounts: `/api/accounts` creates, changes, deletes, tests and
//! syncs them, `GET /api/settings` lists them with the providers' presets,
//! secrets are sealed with `MARCAN_SECRET_KEY`, an account whose secret the
//! key cannot open is marked locked and neither receives nor sends, and
//! `GET /api/system/health` adds up every receiving account.

mod common;

use axum::http::StatusCode;
use common::{get_ok, post_json, put_json, send, TestApp};
use dmarc_mailbox::fake_server::{self, FakeMessage, FakeServer};
use dmarc_server::accounts::connection::ImapProber;
use dmarc_server::mail::MemorySender;
use dmarc_server::wording::accounts::MICROSOFT_LOGIN_ONLY;
use dmarc_server::wording::microsoft::TENANT_NOT_GUID;
use dmarc_server::{AppState, SecretKey, SecretKeys};
use dmarc_store::{
    get_account, get_account_secret, get_cursor, insert_account, list_accounts,
    receive_pass_status, record_receive_pass, set_cursor, AccountFields, AccountProvider,
    AppSettings, ReceivePassRow, SecretKind, SecretRow,
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Arc;

async fn put_ok(t: &TestApp, path: &str, body: Value) -> Value {
    let (status, answer) = put_json(&t.app, path, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {answer}");
    answer
}

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes(&[byte; 32])
}

async fn password_row(pool: &SqlitePool, id: i64) -> SecretRow {
    get_account_secret(pool, id, SecretKind::Password)
        .await
        .unwrap()
        .expect("a stored password")
}

/// A sending-only account (no IMAP server, receiving nothing) with its own
/// login, in the shape of the settings answer.
fn sending_account(password: &str) -> Value {
    json!({
        "name": "Alerts",
        "provider": "generic",
        "address": "alerts@example.com",
        "login": { "kind": "password", "username": "alerts@example.com", "password": password },
        "imap": { "host": "", "port": 993, "security": "tls" },
        "smtp": { "host": "smtp.example.com", "port": 465, "security": "tls" },
        "receiving": { "enabled": false, "folder": "", "forwarded_from": [] },
    })
}

/// Sending on through account `id`, from its own address.
fn sending_through(id: i64) -> Value {
    json!({ "enabled": true, "account_id": id, "sender": "", "reply_to": "", "test_recipient": "" })
}

/// Before any account: none listed, each provider's servers to start from;
/// a created account is listed without its password.
#[tokio::test]
async fn the_settings_list_the_accounts_with_the_providers_presets() {
    let t = TestApp::new().await;
    let settings = get_ok(&t.app, "/api/settings").await;
    assert_eq!(settings["accounts"], json!([]));
    let presets = settings["presets"].as_array().unwrap();
    assert_eq!(presets[0]["provider"], "generic");
    assert_eq!(presets[0]["label"], "IMAP/SMTP");
    assert_eq!(presets[0]["imap"]["host"], "");
    assert_eq!(presets[0]["login"], "password");
    assert_eq!(presets[1]["provider"], "google");
    assert_eq!(presets[1]["imap"]["host"], "imap.gmail.com");
    assert_eq!(presets[1]["smtp"]["host"], "smtp.gmail.com");
    assert_eq!(presets[1]["docs"], "accounts#google");
    assert_eq!(presets[2]["provider"], "microsoft365");
    // Microsoft 365 has no servers: it reads and sends over Graph.
    assert_eq!(presets[2]["imap"], Value::Null);
    assert_eq!(presets[2]["smtp"], Value::Null);
    assert_eq!(presets[2]["login"], "microsoft");

    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    assert_eq!(
        body["accounts"],
        json!([{
            "id": 1,
            "name": "Reports",
            "provider": "generic",
            "address": "reports@example.com",
            "login": { "kind": "password", "username": "reports@example.com", "has_password": true },
            "imap": { "host": "imap.example.com", "port": 993, "security": "tls" },
            "smtp": { "host": "smtp.example.com", "port": 465, "security": "tls" },
            "receiving": { "enabled": true, "folder": "INBOX", "forwarded_from": [] },
            "ready_to_receive": true,
            "report_address": true,
            "sends": true,
            "ready_to_send": true,
            "credential": {
                "condition": "ok", "status": "pass", "lock": null, "lock_text": null,
                "expires_at": null, "expiry_text": null
            },
            "shared_mailbox": null
        }])
    );
    assert_eq!(get_ok(&t.app, "/api/settings").await, body);

    // Google is stored as generic, and shown as Google again by its IMAP host.
    let mut google = account(Some("app-password"));
    google["provider"] = json!("google");
    google["address"] = json!("dmarc@gmail.example");
    google["imap"]["host"] = json!("imap.gmail.com");
    let body = post_ok(&t, "/api/accounts", google).await;
    assert_eq!(body["accounts"][1]["provider"], "google");
    let stored = list_accounts(&t.pool).await.unwrap();
    assert_eq!(stored[1].fields.provider, AccountProvider::Generic);
}

/// A report address alone (as an upgrade from 0.2 has it) is account 1,
/// receiving, without a server or login: the worker does not read it, so
/// it is no report address yet, and saving it asks for a server.
#[tokio::test]
async fn a_report_address_alone_is_an_account_without_a_login() {
    let t = TestApp::new().await;
    let fields = AccountFields {
        name: "dmarc@example.com".into(),
        address: "dmarc@example.com".into(),
        ..AccountFields::default()
    };
    insert_account(&t.pool, Some(1), &fields, &[], 0)
        .await
        .unwrap();
    let body = get_ok(&t.app, "/api/settings").await;
    let account = &body["accounts"][0];
    assert_eq!(account["id"], 1);
    assert_eq!(account["address"], "dmarc@example.com");
    assert_eq!(account["name"], "dmarc@example.com");
    assert_eq!(account["ready_to_receive"], false);
    assert_eq!(account["login"]["has_password"], false);
    // The worker has nothing to read.
    let (_, sync) = post_json(&t.app, "/api/settings/receiving/sync", json!({})).await;
    assert_eq!(sync["reason"], "not_configured");
    let (_, sync) = post_json(&t.app, "/api/accounts/1/sync", json!({})).await;
    assert_eq!(sync["reason"], "not_configured");
    // Its address is not where reports reach Marcan until it can be read.
    assert_eq!(account["report_address"], false);
    let setup = get_ok(&t.app, "/api/reporting-setup").await;
    assert_eq!(setup["default_address"], "");
    assert_eq!(setup["addresses"], json!([]));
    // A save without a server is refused: there is nothing to receive or send with.
    let mut save = crate::account(Some("secret"));
    save["imap"]["host"] = json!("");
    save["smtp"]["host"] = json!("");
    save["receiving"]["enabled"] = json!(false);
    let (status, refused) = put_json(&t.app, "/api/accounts/1", save).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        refused["error"],
        "Enter an IMAP or SMTP server: without one the account neither receives reports nor sends."
    );
}

#[tokio::test]
async fn with_a_key_every_secret_is_stored_sealed_and_used_plain() {
    let (t, _, sender) = with_fakes(Some(key(1)), "secret").await;
    let mut reports = account(Some(" mailbox secret "));
    reports["smtp"] = json!({ "host": "", "port": null, "security": "tls" });
    post_ok(&t, "/api/accounts", reports).await;
    let row = password_row(&t.pool, 1).await;
    assert!(row.is_encrypted());
    assert_eq!(row.key_id.as_deref(), Some(key(1).id()));
    post_ok(&t, "/api/accounts", sending_account("smtp-secret")).await;
    assert!(password_row(&t.pool, 2).await.is_encrypted());
    put_ok(&t, "/api/settings/sending", sending_through(2)).await;

    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sender.configs()[0].smtp().unwrap().password, "smtp-secret");
    let worker = t.state.mailbox_backend();
    let cfg = dmarc_mailbox::Backend::prepare(&worker, "1")
        .await
        .unwrap()
        .expect("account 1 is read");
    assert_eq!(
        cfg.as_imap().unwrap().password.as_str(),
        " mailbox secret ",
        "not trimmed"
    );

    // Without a key the same save is stored plain.
    let (plain, _, _) = with_fakes(None, "secret").await;
    post_ok(&plain, "/api/accounts", account(Some("p"))).await;
    assert!(!password_row(&plain.pool, 1).await.is_encrypted());
}

/// Sealed with key 1, opened with key 2 (or none): the account is locked in
/// the settings and the health, neither read nor sent through nor tested,
/// until its password is entered again.
#[tokio::test]
async fn an_account_whose_key_is_gone_is_locked_until_its_password_is_entered_again() {
    let (sealed, _, _) = with_fakes(Some(key(1)), "secret").await;
    post_ok(&sealed, "/api/accounts", account(Some("secret"))).await;
    put_ok(&sealed, "/api/settings/sending", sending_through(1)).await;
    let row = password_row(&sealed.pool, 1).await;

    for (other, lock) in [(Some(key(2)), "key_changed"), (None, "key_missing")] {
        let (t, prober, sender) = with_fakes(other, "secret").await;
        let fields = get_account(&sealed.pool, 1).await.unwrap().unwrap().fields;
        insert_account(&t.pool, Some(1), &fields, std::slice::from_ref(&row), 0)
            .await
            .unwrap();
        dmarc_store::save_settings(
            &t.pool,
            &AppSettings::load(&sealed.pool).await.unwrap().sending,
        )
        .await
        .unwrap();

        let settings = get_ok(&t.app, "/api/settings").await;
        let credential = &settings["accounts"][0]["credential"];
        assert_eq!(credential["condition"], "locked", "{lock}");
        assert_eq!(credential["status"], "fail");
        assert_eq!(credential["lock"], lock);
        let text = credential["lock_text"].as_str().unwrap();
        assert!(text.contains("Reports"), "{text}");
        assert!(text.contains("enter the credentials again"), "{text}");
        assert_eq!(settings["accounts"][0]["ready_to_receive"], false);
        assert_eq!(settings["accounts"][0]["ready_to_send"], false);

        let health = get_ok(&t.app, "/api/system/health").await;
        assert_eq!(health["receive_configured"], true);
        assert_eq!(health["accounts"][0]["state"], "error");
        assert_eq!(health["accounts"][0]["last_error"], text);
        assert_eq!(health["receive_last_error"], text);
        assert_eq!(health["pipeline"]["mailbox"], "error");

        // The worker lists it but finds nothing it can read.
        let worker = t.state.mailbox_backend();
        assert_eq!(
            dmarc_mailbox::Backend::mailboxes(&worker)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            dmarc_mailbox::Backend::prepare(&worker, "1").await.unwrap(),
            None
        );
        // Sending refuses, naming the lock.
        let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["reason"], "locked");
        assert_eq!(body["error"], text);
        assert!(sender.sent().is_empty());
        // The account test says so too, and logs in nowhere.
        let test = post_ok(
            &t,
            "/api/accounts/test",
            json!({ "id": 1, "account": account(None), "smtp": true }),
        )
        .await;
        assert_eq!(test["mailbox"]["ok"], false);
        assert_eq!(test["mailbox"]["message"], text);
        assert_eq!(test["smtp"]["message"], text);
        assert!(prober.passwords().is_empty());
        assert!(sender.logins().is_empty());

        // Entered again: stored with the current key (or plain), unlocked.
        let body = put_ok(&t, "/api/accounts/1", account(Some("secret"))).await;
        assert_eq!(body["accounts"][0]["credential"]["condition"], "ok");
        assert_eq!(body["accounts"][0]["ready_to_receive"], true);
        assert_eq!(body["accounts"][0]["ready_to_send"], true);
        assert_eq!(
            password_row(&t.pool, 1).await.is_encrypted(),
            lock == "key_changed"
        );
        let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let cfg = dmarc_mailbox::Backend::prepare(&worker, "1").await.unwrap();
        assert_eq!(
            cfg.expect("read again")
                .as_imap()
                .unwrap()
                .password
                .as_str(),
            "secret"
        );
    }
}

#[tokio::test]
async fn health_adds_up_every_receiving_account() {
    let t = TestApp::new().await;
    post_ok(&t, "/api/accounts", account(Some("p"))).await;
    let second = AccountFields {
        name: "Second".into(),
        address: "second@example.net".into(),
        username: "second@example.net".into(),
        imap_host: "imap.example.net".into(),
        ..AccountFields::default()
    };
    insert_account(
        &t.pool,
        Some(3),
        &second,
        &[SecretRow::plain(SecretKind::Password, "q")],
        0,
    )
    .await
    .unwrap();
    // A sending-only account is no mailbox.
    post_ok(&t, "/api/accounts", sending_account("s")).await;
    assert_eq!(list_accounts(&t.pool).await.unwrap().len(), 3);
    let now = dmarc_store::unix_now();
    let pass = |account_id: i64, at: i64, error: Option<&str>| ReceivePassRow {
        account_id,
        finished_at: at,
        checked_at: Some(at),
        error: error.map(str::to_owned),
        error_detail: None,
        yielded: false,
    };
    record_receive_pass(&t.pool, &pass(1, now - 100, None), None)
        .await
        .unwrap();
    record_receive_pass(&t.pool, &pass(3, now - 50, None), None)
        .await
        .unwrap();
    record_receive_pass(&t.pool, &pass(3, now - 10, Some("login failed")), None)
        .await
        .unwrap();

    let health = get_ok(&t.app, "/api/system/health").await;
    let accounts = health["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2, "{health}");
    assert_eq!(accounts[0]["id"], 1);
    assert_eq!(accounts[0]["state"], "active");
    assert_eq!(accounts[0]["status"], "pass");
    assert_eq!(accounts[0]["last_success_at"], now - 100);
    assert_eq!(accounts[1]["name"], "Second");
    assert_eq!(accounts[1]["state"], "error");
    assert_eq!(accounts[1]["last_error"], "login failed");
    assert_eq!(accounts[1]["last_check_at"], now - 10);
    // The top: the stalest check and success, the first error.
    assert_eq!(health["last_check_at"], now - 100);
    assert_eq!(health["receive_last_success_at"], now - 100);
    assert_eq!(health["receive_last_error"], "login failed");
    assert_eq!(health["pipeline"]["mailbox"], "error");
}

// ---- The accounts API: create, update, delete, test, sync, receiving ----

/// Logs in to a fake IMAP server per test call: it accepts only `secret`
/// and has `folders`; the login each call tried is kept.
struct FakeProber {
    secret: &'static str,
    folders: &'static [&'static str],
    tried: std::sync::Mutex<Vec<dmarc_mailbox::ImapConfig>>,
}

impl FakeProber {
    fn new(secret: &'static str) -> Arc<Self> {
        Arc::new(Self {
            secret,
            folders: &["INBOX", "DMARC"],
            tried: std::sync::Mutex::default(),
        })
    }

    fn passwords(&self) -> Vec<String> {
        self.tried
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.password.as_str().to_owned())
            .collect()
    }
}

impl ImapProber for FakeProber {
    fn probe<'a>(
        &'a self,
        cfg: &'a dmarc_mailbox::ImapConfig,
    ) -> BoxFuture<'a, Result<dmarc_mailbox::MailboxProbe, dmarc_mailbox::ConnectError>> {
        self.tried.lock().unwrap().push(cfg.clone());
        let server = FakeServer::new(7, vec![FakeMessage::report(1), FakeMessage::report(2)])
            .with_folders(self.folders)
            .expect_secret(self.secret);
        Box::pin(fake_server::probe(server, cfg))
    }
}

/// An app with a fake IMAP server accepting `secret`, sending into memory.
async fn with_fakes(
    key: Option<SecretKey>,
    secret: &'static str,
) -> (TestApp, Arc<FakeProber>, Arc<MemorySender>) {
    let prober = FakeProber::new(secret);
    let sender = Arc::new(MemorySender::default());
    let (p, mail) = (prober.clone(), sender.clone());
    let t = TestApp::with_state(move |pool| {
        AppState::new(pool)
            .with_secret_keys(SecretKeys {
                current: key,
                previous: None,
            })
            .with_mail_sender(mail)
            .with_imap_prober(p)
    })
    .await;
    (t, prober, sender)
}

/// A receiving and sending account in the shape of the settings answer.
fn account(password: Option<&str>) -> Value {
    let mut login = json!({ "kind": "password", "username": "reports@example.com" });
    if let Some(password) = password {
        login["password"] = json!(password);
    }
    json!({
        "name": "Reports",
        "provider": "generic",
        "address": "reports@example.com",
        "login": login,
        "imap": { "host": "imap.example.com", "port": 993, "security": "tls" },
        "smtp": { "host": "smtp.example.com", "port": 465, "security": "tls" },
        "receiving": { "enabled": true, "folder": "", "forwarded_from": [] },
    })
}

async fn post_ok(t: &TestApp, path: &str, body: Value) -> Value {
    let (status, answer) = post_json(&t.app, path, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {answer}");
    answer
}

#[tokio::test]
async fn an_account_is_created_with_its_password_sealed() {
    let (t, _, _) = with_fakes(Some(key(1)), "secret").await;
    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    assert!(!body.to_string().contains("\"secret\""));
    let created = &body["accounts"][0];
    assert_eq!(created["name"], "Reports");
    assert_eq!(created["receiving"]["folder"], "INBOX", "empty: INBOX");
    assert_eq!(created["login"]["has_password"], true);
    assert_eq!(created["ready_to_receive"], true);
    assert_eq!(created["ready_to_send"], true);
    let id = created["id"].as_i64().unwrap();
    // Sealed under its own id (the AAD), so the worker can open it.
    assert!(password_row(&t.pool, id).await.is_encrypted());
    let cfg = dmarc_mailbox::Backend::prepare(&t.state.mailbox_backend(), &id.to_string())
        .await
        .unwrap()
        .expect("read by the worker");
    assert_eq!(cfg.as_imap().unwrap().password.as_str(), "secret");
    assert_eq!(body["receiving"]["poll_interval_secs"], 300);
    assert_eq!(body["account_defaults"]["imap"]["starttls"], 143);

    // A name left empty is the address.
    let mut other = account(Some("p"));
    other["name"] = json!(" ");
    other["address"] = json!("second@example.com");
    let body = post_ok(&t, "/api/accounts", other).await;
    assert_eq!(body["accounts"][1]["name"], "second@example.com");
}

#[tokio::test]
async fn a_save_is_checked_before_anything_is_stored() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let refused = |change: fn(&mut Value), status: StatusCode, error: &'static str| {
        let mut body = account(Some("p"));
        change(&mut body);
        let app = t.app.clone();
        async move {
            let (got, answer) = post_json(&app, "/api/accounts", body).await;
            assert_eq!(got, status, "{answer}");
            assert_eq!(answer["error"], error);
        }
    };
    refused(
        |b| b["address"] = json!(""),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Enter the email address reports are sent to.",
    )
    .await;
    refused(
        |b| b["address"] = json!("reports"),
        StatusCode::UNPROCESSABLE_ENTITY,
        "The email address reports is not a valid email address.",
    )
    .await;
    refused(
        |b| b["imap"]["host"] = json!(" "),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Enter the IMAP server's host name.",
    )
    .await;
    refused(
        |b| b["smtp"]["port"] = json!(25),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Port 25 is for delivery between mail servers. Use your provider's submission port: 465 with TLS or 587 with STARTTLS.",
    )
    .await;
    refused(
        |b| b["login"] = json!({ "kind": "password", "username": "u" }),
        StatusCode::UNPROCESSABLE_ENTITY,
        "Enter the password.",
    )
    .await;
    refused(
        |b| {
            b["receiving"]["forwarded_from"] =
                json!(["dmarc@client.example", "REPORTS@example.com"]);
        },
        StatusCode::UNPROCESSABLE_ENTITY,
        "The forwarding address REPORTS@example.com is the account's own address.",
    )
    .await;
    refused(
        |b| {
            b["receiving"]["forwarded_from"] =
                json!(["dmarc@client.example", "DMARC@client.example"]);
        },
        StatusCode::UNPROCESSABLE_ENTITY,
        "The forwarding address DMARC@client.example is entered twice.",
    )
    .await;
    refused(
        |b| b["receiving"]["forwarded_from"] = json!(["client.example"]),
        StatusCode::UNPROCESSABLE_ENTITY,
        "The forwarding address client.example is not a valid email address.",
    )
    .await;
    refused(
        |b| {
            b["provider"] = json!("generic");
            b["login"] = json!({
                "kind": "microsoft", "tenant_id": "t", "client_id": "c"
            });
        },
        StatusCode::UNPROCESSABLE_ENTITY,
        "IMAP/SMTP accounts log in with a password.",
    )
    .await;
    // Microsoft 365 has the Microsoft login only (Exchange refuses passwords).
    refused(
        |b| b["provider"] = json!("microsoft365"),
        StatusCode::UNPROCESSABLE_ENTITY,
        MICROSOFT_LOGIN_ONLY,
    )
    .await;
    // A Microsoft login needs GUIDs (the rest: accounts::microsoft_tests).
    refused(
        |b| {
            b["provider"] = json!("microsoft365");
            b["login"] = json!({
                "kind": "microsoft", "tenant_id": "t", "client_id": "c"
            });
        },
        StatusCode::UNPROCESSABLE_ENTITY,
        TENANT_NOT_GUID,
    )
    .await;
    assert!(list_accounts(&t.pool).await.unwrap().is_empty());

    // One address, one account.
    post_ok(&t, "/api/accounts", account(Some("p"))).await;
    let mut twin = account(Some("p"));
    twin["address"] = json!("Reports@Example.com");
    let (status, answer) = post_json(&t.app, "/api/accounts", twin).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(
        answer["error"],
        "Another account already uses Reports@Example.com."
    );

    // A forwarded address belongs to one account too: blank rows are
    // dropped, and another account can take neither it nor the address.
    let mut first = account(Some("p"));
    first["address"] = json!("first@example.com");
    first["receiving"]["forwarded_from"] =
        json!([" dmarc@client.example ", "", "dmarc@client2.example"]);
    let body = post_ok(&t, "/api/accounts", first).await;
    let saved = body["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["address"] == "first@example.com")
        .unwrap();
    assert_eq!(
        saved["receiving"]["forwarded_from"],
        json!(["dmarc@client.example", "dmarc@client2.example"])
    );
    for (address, forwarded, taken) in [
        (
            "other@example.com",
            json!(["DMARC@client.example"]),
            "DMARC@client.example",
        ),
        ("Dmarc@Client2.example", json!([]), "Dmarc@Client2.example"),
        (
            "other@example.com",
            json!(["reports@example.com"]),
            "reports@example.com",
        ),
    ] {
        let mut other = account(Some("p"));
        other["address"] = json!(address);
        other["receiving"]["forwarded_from"] = forwarded;
        let (status, answer) = post_json(&t.app, "/api/accounts", other).await;
        assert_eq!(status, StatusCode::CONFLICT, "{answer}");
        assert_eq!(
            answer["error"],
            format!("Another account already uses {taken}.")
        );
    }
}

#[tokio::test]
async fn an_update_keeps_the_password_only_for_its_own_servers() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let path = format!("/api/accounts/{id}");

    // Same servers and username: the stored password stays.
    let mut renamed = account(None);
    renamed["name"] = json!("DMARC reports");
    renamed["receiving"]["folder"] = json!("DMARC");
    let body = put_ok(&t, &path, renamed).await;
    assert_eq!(body["accounts"][0]["name"], "DMARC reports");
    assert_eq!(body["accounts"][0]["receiving"]["folder"], "DMARC");
    assert_eq!(body["accounts"][0]["login"]["has_password"], true);

    // Another host, port or username without the password: asked again.
    for change in [
        json!({ "imap": { "host": "imap.evil.example", "port": 993, "security": "tls" } }),
        json!({ "imap": { "host": "imap.example.com", "port": 143, "security": "starttls" } }),
        json!({ "smtp": { "host": "smtp.evil.example", "port": 465, "security": "tls" } }),
        json!({ "login": { "kind": "password", "username": "other@example.com" } }),
    ] {
        let mut body = account(None);
        for (k, v) in change.as_object().unwrap() {
            body[k] = v.clone();
        }
        let (status, answer) = put_json(&t.app, &path, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{change}: {answer}");
        assert_eq!(
            answer["error"],
            "Enter the password again: the stored one is only used for the servers and username it was saved for."
        );
    }
    let stored = get_account(&t.pool, id).await.unwrap().unwrap().fields;
    assert_eq!(stored.imap_host, "imap.example.com");

    // A server removed is no leak: kept.
    let mut no_smtp = account(None);
    no_smtp["smtp"] = json!({ "host": "", "port": null, "security": "tls" });
    let body = put_ok(&t, &path, no_smtp).await;
    assert_eq!(body["accounts"][0]["login"]["has_password"], true);
    assert_eq!(body["accounts"][0]["ready_to_send"], false);
    // Added back, it needs the password again; typed, it is stored.
    let (status, _) = put_json(&t.app, &path, account(None)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let body = put_ok(&t, &path, account(Some("new secret"))).await;
    assert_eq!(body["accounts"][0]["ready_to_send"], true);
    let cfg = dmarc_mailbox::Backend::prepare(&t.state.mailbox_backend(), &id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cfg.as_imap().unwrap().password.as_str(), "new secret");

    let (status, _) = put_json(&t.app, "/api/accounts/99", account(Some("p"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// An account stored without an SMTP port (as converted from a 0.3 sender
/// that had none) sends through the provider's port: saving it with that
/// port, or with none, keeps the stored password, and its test uses it.
#[tokio::test]
async fn a_stored_unset_smtp_port_is_the_providers_port() {
    let (t, prober, sender) = with_fakes(None, "secret").await;
    let fields = AccountFields {
        name: "Reports".into(),
        address: "reports@example.com".into(),
        username: "reports@example.com".into(),
        imap_host: "imap.example.com".into(),
        smtp_host: "smtp.example.com".into(),
        smtp_port: None,
        smtp_security: dmarc_store::SmtpSecurity::StartTls,
        ..AccountFields::default()
    };
    let id = insert_account(
        &t.pool,
        None,
        &fields,
        &[SecretRow::plain(SecretKind::Password, "secret")],
        0,
    )
    .await
    .unwrap();
    let path = format!("/api/accounts/{id}");
    for port in [json!(587), Value::Null] {
        let mut body = account(None);
        body["smtp"] = json!({ "host": "smtp.example.com", "port": port, "security": "starttls" });
        let saved = put_ok(&t, &path, body.clone()).await;
        assert_eq!(saved["accounts"][0]["smtp"]["port"], 587, "{port}");
        assert_eq!(saved["accounts"][0]["login"]["has_password"], true);

        let answer = post_ok(
            &t,
            "/api/accounts/test",
            json!({ "id": id, "account": body, "smtp": true }),
        )
        .await;
        assert_eq!(answer["smtp"]["ok"], true, "{answer}");
    }
    assert_eq!(prober.passwords().last().unwrap(), "secret");
    assert_eq!(sender.logins().last().unwrap().password, "secret");
}

#[tokio::test]
async fn the_sending_account_is_kept_while_sending_is_on() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let path = format!("/api/accounts/{id}");
    let mut sending = dmarc_store::SendingSettings {
        enabled: true,
        account_id: Some(id),
        ..Default::default()
    };
    dmarc_store::save_settings(&t.pool, &sending).await.unwrap();

    // On: neither deleted nor without its SMTP server.
    let (status, answer) = send(&t.app, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(
        answer["error"],
        "Sending goes through Reports. Choose another account under Sending, or turn sending off, first."
    );
    let mut no_smtp = account(None);
    no_smtp["smtp"]["host"] = json!("");
    let (status, answer) = put_json(&t.app, &path, no_smtp).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert!(answer["error"]
        .as_str()
        .unwrap()
        .starts_with("Sending goes through Reports, so it needs its SMTP server."));

    // Off: deleted, and the choice cleared.
    sending.enabled = false;
    dmarc_store::save_settings(&t.pool, &sending).await.unwrap();
    let (status, body) = send(&t.app, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accounts"], json!([]));
    assert_eq!(
        AppSettings::load(&t.pool).await.unwrap().sending.account_id,
        None
    );
    let (status, _) = send(&t.app, "DELETE", &path, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_test_logs_in_with_the_values_on_screen() {
    let (t, prober, sender) = with_fakes(None, "secret").await;
    let test = |body: Value| post_ok(&t, "/api/accounts/test", body);

    // A new account: the typed password, the folders listed, one examined.
    let answer =
        test(json!({ "account": account(Some("secret")), "folder": "DMARC", "smtp": true })).await;
    assert_eq!(answer["token"], Value::Null);
    let imap = &answer["mailbox"];
    assert_eq!(imap["ok"], true, "{answer}");
    assert_eq!(imap["message"], "Logged in as reports@example.com.");
    assert_eq!(imap["folders"][1]["name"], "DMARC");
    assert_eq!(imap["selected"]["name"], "DMARC");
    assert_eq!(imap["selected"]["messages"], 2);
    assert_eq!(answer["smtp"]["ok"], true, "{answer}");
    assert_eq!(
        answer["smtp"]["message"],
        "Logged in as reports@example.com."
    );
    // The SMTP step logs in and sends nothing.
    assert!(sender.sent().is_empty());
    assert_eq!(sender.logins().len(), 1);
    assert_eq!(sender.logins()[0].host, "smtp.example.com");
    assert_eq!(sender.logins()[0].password, "secret");

    // A wrong password, a folder that is not there.
    let answer = test(json!({ "account": account(Some("wrong")), "smtp": false })).await;
    assert_eq!(answer["mailbox"]["ok"], false);
    assert!(answer["mailbox"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Login rejected."));
    assert_eq!(answer["smtp"], Value::Null, "not asked for");
    let answer =
        test(json!({ "account": account(Some("secret")), "folder": "Gone", "smtp": false })).await;
    assert_eq!(answer["mailbox"]["selected"]["exists"], false);

    // A stored account: its password, for its own servers only.
    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let answer = test(json!({ "id": id, "account": account(None), "smtp": true })).await;
    assert_eq!(answer["mailbox"]["ok"], true, "{answer}");
    assert_eq!(answer["smtp"]["ok"], true, "{answer}");
    assert_eq!(prober.passwords().last().unwrap(), "secret");
    let mut elsewhere = account(None);
    elsewhere["imap"]["host"] = json!("imap.evil.example");
    elsewhere["smtp"]["host"] = json!("smtp.evil.example");
    let tried = prober.passwords().len();
    let answer = test(json!({ "id": id, "account": elsewhere, "smtp": true })).await;
    let again = "Enter the password again: the stored one is only used for the servers and username it was saved for.";
    assert_eq!(answer["mailbox"]["message"], again);
    assert_eq!(answer["smtp"]["message"], again);
    assert_eq!(prober.passwords().len(), tried, "nothing was sent");
    assert_eq!(sender.logins().len(), 2);

    // A Microsoft login has no SMTP line: it sends over Graph, which
    // cannot be tried without sending; its IDs are checked first.
    let mut microsoft = account(None);
    microsoft["provider"] = json!("microsoft365");
    microsoft["login"] = json!({
        "kind": "microsoft", "tenant_id": "t", "client_id": "c"
    });
    let logins = sender.logins().len();
    let answer = test(json!({ "account": microsoft, "smtp": true })).await;
    assert_eq!(answer["token"]["ok"], false);
    assert_eq!(answer["token"]["message"], TENANT_NOT_GUID);
    assert_eq!(answer["mailbox"], Value::Null);
    assert_eq!(answer["smtp"], Value::Null);
    assert_eq!(sender.logins().len(), logins, "no SMTP login");

    // Nothing to test.
    let mut none = account(Some("p"));
    none["imap"]["host"] = json!("");
    let (status, _) = post_json(
        &t.app,
        "/api/accounts/test",
        json!({ "account": none, "smtp": false }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        list_accounts(&t.pool).await.unwrap().len() == 1,
        "the test stores nothing"
    );
}

/// A form tested and then saved is one account: the test logs in to the
/// host, ports, folder and as the user the save stores (trimmed, unset
/// ports the provider's, a blank folder INBOX), with nothing defaulted its
/// own way.
#[tokio::test]
async fn a_tested_form_is_the_account_its_save_stores() {
    let (t, prober, sender) = with_fakes(None, "secret").await;
    let form = json!({
        "name": "",
        "provider": "generic",
        "address": " reports@example.com ",
        "login": { "kind": "password", "username": " reports@example.com ", "password": "secret" },
        "imap": { "host": " IMAP.example.com ", "port": 0, "security": "starttls" },
        "smtp": { "host": " smtp.example.com ", "port": null, "security": "starttls" },
        "receiving": { "enabled": true, "folder": "  ", "forwarded_from": [] },
    });
    let answer = post_ok(
        &t,
        "/api/accounts/test",
        json!({ "account": form, "smtp": true }),
    )
    .await;
    assert_eq!(answer["mailbox"]["ok"], true, "{answer}");
    assert_eq!(answer["smtp"]["ok"], true, "{answer}");
    let imap = prober.tried.lock().unwrap().last().cloned().unwrap();
    let smtp = sender.logins().last().cloned().unwrap();

    post_ok(&t, "/api/accounts", form).await;
    let saved = list_accounts(&t.pool).await.unwrap().remove(0).fields;
    assert_eq!(saved.name, "reports@example.com");
    assert_eq!(
        (imap.host.as_str(), imap.port, imap.folder.as_str()),
        (
            saved.imap_host.as_str(),
            saved.imap_port,
            saved.folder.as_str()
        )
    );
    assert_eq!((imap.port, imap.folder.as_str()), (143, "INBOX"));
    assert_eq!(imap.username, saved.username);
    assert_eq!(
        (smtp.host.as_str(), Some(smtp.port)),
        (saved.smtp_host.as_str(), saved.smtp_port)
    );
    assert_eq!(smtp.port, 587);
    assert_eq!(smtp.username, saved.username);
    assert_eq!(saved.username, "reports@example.com");
}

/// The SMTP step through the real client: a server that does not offer
/// STARTTLS is left before the login, and the step says why.
#[tokio::test]
async fn the_smtp_test_fails_honestly_without_starttls() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        stream.write_all(b"220 fake ESMTP\r\n").await.unwrap();
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            lines.push(line.trim_end().to_owned());
            let _ = stream
                .write_all(b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n")
                .await;
        }
        lines
    });
    let t = TestApp::new().await;
    let mut body = account(Some("secret"));
    body["smtp"] = json!({ "host": "127.0.0.1", "port": port, "security": "starttls" });
    body["imap"]["host"] = json!("");
    body["receiving"]["enabled"] = json!(false);
    let answer = post_ok(
        &t,
        "/api/accounts/test",
        json!({ "account": body, "smtp": true }),
    )
    .await;
    assert_eq!(answer["smtp"]["ok"], false, "{answer}");
    assert!(answer["smtp"]["message"]
        .as_str()
        .unwrap()
        .contains("STARTTLS"));
    let lines = server.await.unwrap();
    assert!(lines.iter().any(|l| l.starts_with("EHLO")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.starts_with("AUTH")), "{lines:?}");
}

#[tokio::test]
async fn a_locked_account_is_tested_and_saved_only_with_its_password_again() {
    let (sealed, _, _) = with_fakes(Some(key(1)), "secret").await;
    let body = post_ok(&sealed, "/api/accounts", account(Some("secret"))).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let row = password_row(&sealed.pool, id).await;
    let fields = get_account(&sealed.pool, id).await.unwrap().unwrap().fields;

    let (t, prober, _) = with_fakes(Some(key(2)), "secret").await;
    insert_account(&t.pool, Some(id), &fields, &[row], 0)
        .await
        .unwrap();
    let path = format!("/api/accounts/{id}");
    let answer = post_ok(
        &t,
        "/api/accounts/test",
        json!({ "id": id, "account": account(None), "smtp": true }),
    )
    .await;
    let message = answer["mailbox"]["message"].as_str().unwrap();
    assert!(message.contains("another MARCAN_SECRET_KEY"), "{message}");
    assert_eq!(answer["smtp"]["message"], message);
    assert!(prober.passwords().is_empty(), "nothing was sent");

    // Saved unchanged it stays locked; entered again it opens.
    let body = put_ok(&t, &path, account(None)).await;
    assert_eq!(body["accounts"][0]["credential"]["condition"], "locked");
    let body = put_ok(&t, &path, account(Some("secret"))).await;
    assert_eq!(body["accounts"][0]["credential"]["condition"], "ok");
    assert_eq!(body["accounts"][0]["ready_to_receive"], true);
}

#[tokio::test]
async fn one_account_is_synced_and_the_interval_is_clamped() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let mut idle = account(Some("p"));
    idle["receiving"]["enabled"] = json!(false);
    let body = post_ok(&t, "/api/accounts", idle).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let sync = post_ok(&t, &format!("/api/accounts/{id}/sync"), json!({})).await;
    assert_eq!(sync["started"], false);
    assert_eq!(sync["reason"], "not_configured");
    put_ok(&t, &format!("/api/accounts/{id}"), account(None)).await;
    let sync = post_ok(&t, &format!("/api/accounts/{id}/sync"), json!({})).await;
    assert_eq!(sync["started"], true, "{sync}");
    let (status, _) = post_json(&t.app, "/api/accounts/99/sync", json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let body = put_ok(
        &t,
        "/api/settings/receiving",
        json!({ "poll_interval_secs": 1 }),
    )
    .await;
    assert_eq!(body["receiving"]["poll_interval_secs"], 10);
    assert_eq!(body["receiving"]["defaults"]["min_poll_interval_secs"], 10);
    let body = put_ok(
        &t,
        "/api/settings/receiving",
        json!({ "poll_interval_secs": 600 }),
    )
    .await;
    assert_eq!(body["receiving"]["poll_interval_secs"], 600);
}

/// Two receiving accounts on the same IMAP host, port, user and folder are saved
/// (each report is stored once), and each says it shares the mailbox with
/// the other, in the save's answer as in the settings.
#[tokio::test]
async fn two_accounts_reading_one_mailbox_are_saved_with_a_warning() {
    let (t, _, _) = with_fakes(None, "secret").await;
    post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    let mut twin = account(Some("secret"));
    twin["name"] = json!("Twin");
    twin["address"] = json!("twin@example.com");
    twin["imap"]["host"] = json!("IMAP.example.com");
    let body = post_ok(&t, "/api/accounts", twin.clone()).await;
    let accounts = &body["accounts"];
    assert_eq!(
        accounts[0]["shared_mailbox"],
        "Twin reads the same mailbox and folder. Each report is stored once, so one of the two accounts is enough."
    );
    assert_eq!(
        accounts[1]["shared_mailbox"],
        "Reports reads the same mailbox and folder. Each report is stored once, so one of the two accounts is enough."
    );
    let settings = get_ok(&t.app, "/api/settings").await;
    assert_eq!(settings["accounts"], *accounts);

    // Another folder, another port: no warning.
    let id = accounts[1]["id"].as_i64().unwrap();
    let mut other_folder = twin.clone();
    other_folder["receiving"]["folder"] = json!("DMARC");
    let body = put_ok(&t, &format!("/api/accounts/{id}"), other_folder).await;
    assert_eq!(body["accounts"][0]["shared_mailbox"], Value::Null);
    assert_eq!(body["accounts"][1]["shared_mailbox"], Value::Null);
    let mut other_port = twin;
    other_port["imap"]["port"] = json!(1993);
    let body = put_ok(&t, &format!("/api/accounts/{id}"), other_port).await;
    assert_eq!(body["accounts"][1]["imap"]["port"], 1993);
    assert_eq!(body["accounts"][0]["shared_mailbox"], Value::Null);
    assert_eq!(body["accounts"][1]["shared_mailbox"], Value::Null);
}

/// A save that makes the account read another folder forgets its cursor,
/// skipped messages and passes and moves its epoch on with the save itself,
/// before any pass: a restart right after it cannot leave the old cursor
/// behind, and a pass prepared before it writes nothing more. A rename
/// keeps them, and the epoch.
#[tokio::test]
async fn another_folder_forgets_the_receive_state_with_the_save() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let body = post_ok(&t, "/api/accounts", account(Some("secret"))).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let path = format!("/api/accounts/{id}");
    let state = |pool: SqlitePool| async move {
        let cursor = get_cursor(&pool, id, "INBOX").await.unwrap();
        let passes = receive_pass_status(&pool, id)
            .await
            .unwrap()
            .last_success_at;
        let epoch = get_account(&pool, id).await.unwrap().unwrap().receive_epoch;
        (cursor, passes, epoch)
    };
    set_cursor(&t.pool, id, "INBOX", "7", "42", 0)
        .await
        .unwrap();
    let pass = ReceivePassRow {
        account_id: id,
        finished_at: 10,
        checked_at: Some(10),
        error: None,
        error_detail: None,
        yielded: false,
    };
    record_receive_pass(&t.pool, &pass, None).await.unwrap();

    let mut renamed = account(None);
    renamed["name"] = json!("DMARC reports");
    put_ok(&t, &path, renamed).await;
    assert_eq!(
        state(t.pool.clone()).await,
        (Some(("7".into(), "42".into())), Some(10), 0)
    );

    let mut moved = account(None);
    moved["receiving"]["folder"] = json!("DMARC");
    put_ok(&t, &path, moved).await;
    assert_eq!(state(t.pool.clone()).await, (None, None, 1));
    // What the pass prepared before the save writes back is dropped.
    assert!(!set_cursor(&t.pool, id, "INBOX", "7", "50", 0)
        .await
        .unwrap());
    assert!(!record_receive_pass(&t.pool, &pass, Some(0)).await.unwrap());
    assert_eq!(state(t.pool.clone()).await, (None, None, 1));
}

/// An account reads its mailbox from when it was added; switching its
/// receiving on, or another folder, starts that over (the days it may go
/// unread count from then), a rename does not.
#[tokio::test]
async fn receiving_since_starts_over_with_receiving_or_another_mailbox() {
    let (t, _, _) = with_fakes(None, "secret").await;
    let mut off = account(Some("secret"));
    off["receiving"]["enabled"] = json!(false);
    let before = dmarc_store::unix_now();
    let body = post_ok(&t, "/api/accounts", off.clone()).await;
    let id = body["accounts"][0]["id"].as_i64().unwrap();
    let path = format!("/api/accounts/{id}");
    let since = |pool: SqlitePool| async move {
        get_account(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .receiving_since
    };
    let added = since(t.pool.clone()).await;
    assert!(added >= before);
    let reset = |pool: SqlitePool| async move {
        sqlx::query("UPDATE accounts SET receiving_since = 1 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    };

    reset(t.pool.clone()).await;
    let mut renamed = off.clone();
    renamed["name"] = json!("Renamed");
    put_ok(&t, &path, renamed).await;
    assert_eq!(since(t.pool.clone()).await, 1, "a rename keeps it");

    put_ok(&t, &path, account(None)).await;
    assert!(since(t.pool.clone()).await >= before, "receiving on");

    reset(t.pool.clone()).await;
    let mut moved = account(None);
    moved["receiving"]["folder"] = json!("DMARC");
    put_ok(&t, &path, moved).await;
    assert!(since(t.pool.clone()).await >= before, "another folder");
}
