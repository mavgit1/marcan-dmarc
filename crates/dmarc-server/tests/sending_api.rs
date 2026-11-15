//! The sending settings group (`PUT /api/settings/sending`, the `sending`
//! part of `GET /api/settings`) and the test send. The SMTP server and its
//! login are the chosen account's (`/api/accounts`); the group holds the
//! choice and the addresses.

use axum::http::StatusCode;
use dmarc_server::mail::MemorySender;
use dmarc_server::wording::accounts::NO_SENDING_ACCOUNT as NO_ACCOUNT;
use dmarc_server::AppState;
use dmarc_store::set_setting;
use serde_json::{json, Value};
use std::sync::Arc;
mod common;
use common::{get_ok, post_json, put_json, TestApp};

/// An account that sends through `host:587` (STARTTLS) as
/// `alerts@example.com` with `password`, from `address`, receiving nothing.
fn smtp_account(address: &str, host: &str, password: &str) -> Value {
    json!({
        "name": "",
        "provider": "generic",
        "address": address,
        "login": { "kind": "password", "username": "alerts@example.com", "password": password },
        "imap": { "host": "", "port": 993, "security": "tls" },
        "smtp": { "host": host, "port": 587, "security": "starttls" },
        "receiving": { "enabled": false, "folder": "", "forwarded_from": [] },
    })
}

/// A receiving account without an SMTP server.
fn mailbox_account() -> Value {
    json!({
        "name": "Reports",
        "provider": "generic",
        "address": "reports@example.com",
        "login": { "kind": "password", "username": "reports@example.com", "password": "mailbox-secret" },
        "imap": { "host": "imap.example.com", "port": 993, "security": "tls" },
        "smtp": { "host": "", "port": null, "security": "tls" },
        "receiving": { "enabled": true, "folder": "", "forwarded_from": [] },
    })
}

/// Create the account `body`; its id.
async fn create(t: &TestApp, body: Value) -> i64 {
    let (status, answer) = post_json(&t.app, "/api/accounts", body).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["accounts"].as_array().unwrap().last().unwrap()["id"]
        .as_i64()
        .unwrap()
}

/// The group switched on through `account_id`, with a Reply-To.
fn group(account_id: i64) -> Value {
    json!({
        "enabled": true,
        "account_id": account_id,
        "reply_to": "help@example.com",
        "test_recipient": "",
    })
}

#[tokio::test]
async fn sending_is_off_by_default_with_nothing_filled_in() {
    let t = TestApp::new().await;
    let sending = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(
        sending,
        json!({
            "enabled": false,
            "account_id": null,
            "reply_to": "",
            "test_recipient": "",
            "effective_test_recipient": "",
        })
    );

    // With an account chosen, its address is the sender.
    let id = create(
        &t,
        smtp_account("reports@example.com", "smtp.example.com", "s"),
    )
    .await;
    let mut chosen = group(id);
    chosen["enabled"] = json!(false);
    let (status, body) = put_json(&t.app, "/api/settings/sending", chosen).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sending = &body["sending"];
    assert_eq!(sending["account_id"], id);
    assert_eq!(sending["effective_test_recipient"], "reports@example.com");
}

/// The group is stored as sent (trimmed) and answered the same by `GET`;
/// the account's password never shows in either.
#[tokio::test]
async fn sending_round_trip_stores_the_group_as_sent() {
    let t = TestApp::new().await;
    let id = create(
        &t,
        smtp_account("alerts@example.com", "smtp.example.com", "smtp-secret"),
    )
    .await;
    let mut sent = group(id);
    sent["reply_to"] = json!(" help@example.com ");
    let (status, body) = put_json(&t.app, "/api/settings/sending", sent).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("smtp-secret"));
    assert_eq!(
        body["sending"],
        json!({
            "enabled": true,
            "account_id": id,
            "reply_to": "help@example.com",
            "test_recipient": "",
            "effective_test_recipient": "alerts@example.com",
        })
    );
    let got = get_ok(&t.app, "/api/settings").await;
    assert_eq!(got["sending"], body["sending"]);
    assert!(!got.to_string().contains("smtp-secret"));

    // Switched off, the choice and the addresses stay.
    let mut off = group(id);
    off["enabled"] = json!(false);
    let (status, body) = put_json(&t.app, "/api/settings/sending", off).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sending"]["enabled"], false);
    assert_eq!(body["sending"]["account_id"], id);
    assert_eq!(body["sending"]["reply_to"], "help@example.com");
    assert_eq!(
        body["sending"]["effective_test_recipient"],
        "alerts@example.com"
    );
}

#[tokio::test]
async fn sending_validation_says_what_is_missing() {
    let t = TestApp::new().await;
    let before = get_ok(&t.app, "/api/settings").await["sending"].clone();
    // 422: a value that is not valid; 409: turned on without what sending needs.
    let put = |expected: StatusCode, body: Value| {
        let app = t.app.clone();
        async move {
            let (status, body) = put_json(&app, "/api/settings/sending", body).await;
            assert_eq!(status, expected, "{body}");
            body["error"].as_str().unwrap().to_owned()
        }
    };
    const INVALID: StatusCode = StatusCode::UNPROCESSABLE_ENTITY;
    const NOT_READY: StatusCode = StatusCode::CONFLICT;
    let mut no_address = smtp_account("", "smtp.example.com", "smtp-secret");
    no_address["name"] = json!("Alerts");
    let smtp = create(&t, no_address).await;
    let mailbox = create(&t, mailbox_account()).await;
    let with = |changes: Value| {
        let mut body = group(smtp);
        for (key, value) in changes.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };

    assert_eq!(
        put(NOT_READY, with(json!({ "account_id": null }))).await,
        NO_ACCOUNT
    );
    assert_eq!(
        put(INVALID, with(json!({ "account_id": 99, "enabled": false }))).await,
        "There is no account 99."
    );
    assert_eq!(
        put(
            INVALID,
            with(json!({ "account_id": mailbox, "enabled": false }))
        )
        .await,
        "Reports has no SMTP server. Add one under Accounts."
    );
    assert_eq!(
        put(
            INVALID,
            with(json!({ "reply_to": "help@", "enabled": false }))
        )
        .await,
        "The Reply-To address help@ is not a valid email address."
    );
    // The account has no address of its own: on, there is none to send from.
    assert_eq!(
        put(NOT_READY, with(json!({}))).await,
        "Alerts has no email address to send from. Enter one under Accounts."
    );
    // Off, the group is saved through it.
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        with(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sending"]["effective_test_recipient"], "");

    // An account stored without a password (behind the API's back) is not
    // ready to send.
    let fields = dmarc_store::AccountFields {
        name: "No password".into(),
        username: "u@example.com".into(),
        smtp_host: "smtp.example.com".into(),
        smtp_port: Some(465),
        receive_enabled: false,
        ..Default::default()
    };
    let bare = dmarc_store::insert_account(&t.pool, None, &fields, &[], 0)
        .await
        .unwrap();
    assert_eq!(
        put(NOT_READY, with(json!({ "account_id": bare }))).await,
        "Enter the SMTP password."
    );

    // Nothing refused was stored along the way.
    let after = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(after["account_id"], smtp);
    assert_eq!(after["enabled"], false);
    assert_ne!(after, before);
}

#[tokio::test]
async fn the_test_send_needs_sending_on_and_complete() {
    let t = TestApp::new().await;
    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "disabled");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .starts_with("Sending is off."));

    // Switched on behind the API's back, with no account to send through.
    set_setting(&t.pool, "sending", r#"{"enabled":true}"#)
        .await
        .unwrap();
    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "incomplete");
    assert_eq!(body["error"], NO_ACCOUNT);
    // The group on screen without an account says the same as its Save
    // and as the stored group's send.
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": { "enabled": false, "account_id": null, "reply_to": "", "test_recipient": "" } }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "incomplete");
    assert_eq!(body["error"], NO_ACCOUNT);

    // Through a mailbox account, which has no SMTP server.
    let mailbox = create(&t, mailbox_account()).await;
    set_setting(
        &t.pool,
        "sending",
        &format!(r#"{{"enabled":true,"account_id":{mailbox}}}"#),
    )
    .await
    .unwrap();
    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "Enter the SMTP server's host name.");
}

#[tokio::test]
async fn the_test_send_names_the_failed_step() {
    let t = TestApp::new().await;
    let id = create(
        &t,
        smtp_account("dmarc@example.com", "smtp.invalid", "smtp-secret"),
    )
    .await;
    let (status, body) = put_json(&t.app, "/api/settings/sending", group(id)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut draft = group(id);
    draft["test_recipient"] = json!("not an address");
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": draft }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body["error"],
        "The test recipient not an address is not a valid email address."
    );

    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["reason"], "resolve");
    assert_eq!(
        body["error"],
        "Server not found: smtp.invalid does not resolve. Check the SMTP host name."
    );
    assert!(!body.to_string().contains("smtp-secret"));
}

/// The test takes the group on screen, which names an account by its id:
/// that account's server and stored password are used, whichever account
/// is chosen, and nothing of the draft is stored.
#[tokio::test]
async fn the_test_send_uses_the_group_on_screen_with_the_stored_accounts_password() {
    let sender = Arc::new(MemorySender::default());
    let mail = sender.clone();
    let t = TestApp::with_state(move |pool| AppState::new(pool).with_mail_sender(mail)).await;
    let first = create(
        &t,
        smtp_account("dmarc@example.com", "smtp.example.com", "smtp-secret"),
    )
    .await;
    let second = create(
        &t,
        smtp_account(
            "other@example.com",
            "smtp.elsewhere.example",
            "other-secret",
        ),
    )
    .await;

    // Saved while off: the test runs before sending is turned on.
    let mut saved = group(first);
    saved["enabled"] = json!(false);
    let (status, body) = put_json(&t.app, "/api/settings/sending", saved.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The same account on screen: its stored password is used.
    let mut draft = saved.clone();
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": draft.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // No test recipient: to the account's address; the one on screen once typed.
    assert_eq!(body["message"], "Test message sent to dmarc@example.com.");
    assert_eq!(sender.sent()[0].to.to_string(), "dmarc@example.com");
    draft["test_recipient"] = json!(" owner@example.org ");
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": draft }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Test message sent to owner@example.org.");
    let config = &sender.configs()[0];
    let server = config.smtp().expect("sent over SMTP");
    assert_eq!(server.host, "smtp.example.com");
    assert_eq!(server.port, 587);
    assert_eq!(server.username, "alerts@example.com");
    assert_eq!(server.password, "smtp-secret");
    assert_eq!(config.sender.to_string(), "dmarc@example.com");
    assert_eq!(
        config.reply_to.as_ref().unwrap().to_string(),
        "help@example.com"
    );

    // Another account: its own server and password, from its address.
    let mut other = saved.clone();
    other["account_id"] = json!(second);
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": other }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Test message sent to other@example.com.");
    let server = sender.configs()[2].smtp().cloned().expect("sent over SMTP");
    assert_eq!(server.host, "smtp.elsewhere.example");
    assert_eq!(server.password, "other-secret");

    // An account that is not there: nothing is sent.
    let mut missing = saved.clone();
    missing["account_id"] = json!(99);
    let (status, body) = post_json(
        &t.app,
        "/api/settings/sending/test",
        json!({ "sending": missing }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "There is no account 99.");
    assert_eq!(sender.sent().len(), 3);

    // Nothing of the drafts was stored.
    let stored = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(stored["account_id"], first);
    assert_eq!(stored["reply_to"], "help@example.com");
    assert_eq!(stored["enabled"], false);
    assert_eq!(stored["test_recipient"], "");
}

#[tokio::test]
async fn the_test_recipient_is_saved_and_defaults_to_the_sender() {
    let t = TestApp::new().await;
    let sending = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(sending["test_recipient"], "");
    assert_eq!(sending["effective_test_recipient"], "");

    let id = create(
        &t,
        smtp_account("alerts@example.com", "smtp.example.com", "smtp-secret"),
    )
    .await;
    let mut body = group(id);
    let (status, saved) = put_json(&t.app, "/api/settings/sending", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["sending"]["test_recipient"], "");
    assert_eq!(
        saved["sending"]["effective_test_recipient"],
        "alerts@example.com"
    );

    body["test_recipient"] = json!(" me@example.org ");
    let (status, saved) = put_json(&t.app, "/api/settings/sending", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["sending"]["test_recipient"], "me@example.org");
    assert_eq!(
        saved["sending"]["effective_test_recipient"],
        "me@example.org"
    );

    body["test_recipient"] = json!("me");
    let (status, err) = put_json(&t.app, "/api/settings/sending", body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(
        err["error"],
        "The test recipient me is not a valid email address."
    );
}
