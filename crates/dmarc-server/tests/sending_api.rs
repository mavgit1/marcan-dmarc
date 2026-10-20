//! The sending settings group (`PUT /api/settings/sending`, the `sending`
//! part of `GET /api/settings`) and the test send.

use axum::http::StatusCode;
use dmarc_server::mail::MemorySender;
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{init, set_setting};
use serde_json::{json, Value};
use std::sync::Arc;
mod common;
use common::{get_ok, post_json, put_json, TestApp};

/// An enabled group with its own login.
fn own_login(host: &str, password: Option<&str>) -> Value {
    let mut body = json!({
        "enabled": true,
        "host": host,
        "port": 587,
        "security": "starttls",
        "same_login_as_mailbox": false,
        "username": "alerts@example.com",
        "sender": "dmarc@example.com",
        "reply_to": "help@example.com",
        "test_recipient": "",
    });
    if let Some(password) = password {
        body["password"] = json!(password);
    }
    body
}

async fn save_mailbox(t: &TestApp) {
    let (status, body) = put_json(
        &t.app,
        "/api/settings/mailbox",
        json!({
            "host": "imap.example.com",
            "port": 993,
            "username": "reports@example.com",
            "password": "mailbox-secret",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn sending_is_off_by_default_with_nothing_filled_in() {
    let t = TestApp::new().await;
    let sending = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(
        sending,
        json!({
            "enabled": false,
            "host": "",
            "port": null,
            "security": "tls",
            "same_login_as_mailbox": true,
            "username": "",
            "has_password": false,
            "sender": "",
            "reply_to": "",
            "effective_sender": "",
            "test_recipient": "",
            "effective_test_recipient": "",
            "defaults": { "port": 465, "starttls_port": 587 },
        })
    );

    // With the same login, the mailbox address is the sender until one is saved.
    save_mailbox(&t).await;
    let sending = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(sending["effective_sender"], "reports@example.com");
    assert_eq!(sending["effective_test_recipient"], "reports@example.com");
    assert_eq!(sending["has_password"], false);
}

#[tokio::test]
async fn sending_round_trip_keeps_the_password_for_the_same_account_only() {
    let t = TestApp::new().await;
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        own_login("smtp.example.com", Some("smtp-secret")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.to_string().contains("smtp-secret"));
    let sending = &body["sending"];
    assert_eq!(sending["enabled"], true);
    assert_eq!(sending["host"], "smtp.example.com");
    assert_eq!(sending["port"], 587);
    assert_eq!(sending["security"], "starttls");
    assert_eq!(sending["same_login_as_mailbox"], false);
    assert_eq!(sending["username"], "alerts@example.com");
    assert_eq!(sending["has_password"], true);
    assert_eq!(sending["reply_to"], "help@example.com");
    assert_eq!(sending["effective_sender"], "dmarc@example.com");
    assert!(sending.get("password").is_none());

    let got = get_ok(&t.app, "/api/settings").await;
    assert_eq!(got["sending"], *sending);
    assert!(!got.to_string().contains("smtp-secret"));

    // The same account without a password keeps it (the host's case aside).
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        own_login("SMTP.example.com", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sending"]["has_password"], true);

    // Another server needs the password typed again: the stored one never
    // follows a changed host.
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        own_login("smtp.elsewhere.net", None),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("Enter the SMTP password"),
        "{body}"
    );

    // Off, the change is saved and the stored password, which belongs to the
    // old server, is dropped.
    let mut off = own_login("smtp.elsewhere.net", None);
    off["enabled"] = json!(false);
    let (status, body) = put_json(&t.app, "/api/settings/sending", off).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sending"]["host"], "smtp.elsewhere.net");
    assert_eq!(body["sending"]["has_password"], false);
}

#[tokio::test]
async fn sending_validation_says_what_is_missing() {
    let t = TestApp::new().await;
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
    let with = |changes: Value| {
        let mut body = own_login("smtp.example.com", Some("smtp-secret"));
        for (key, value) in changes.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };

    assert_eq!(
        put(NOT_READY, with(json!({ "host": " " }))).await,
        "Enter a server first."
    );
    assert_eq!(
        put(NOT_READY, with(json!({ "port": null }))).await,
        "Enter the SMTP server's port."
    );
    assert_eq!(
        put(INVALID, with(json!({ "port": 0, "enabled": false }))).await,
        "The SMTP port must be between 1 and 65535."
    );
    // Not a port number at all: the request is malformed, a 400 in JSON.
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        with(json!({ "port": 70000, "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    assert!(body["error"].as_str().unwrap().contains("70000"), "{body}");
    assert!(put(INVALID, with(json!({ "port": 25 })))
        .await
        .starts_with("Port 25 is for delivery between mail servers."));
    assert_eq!(
        put(
            INVALID,
            with(json!({ "sender": "dmarc", "enabled": false }))
        )
        .await,
        "The sender address dmarc is not a valid email address."
    );
    assert_eq!(
        put(
            INVALID,
            with(json!({ "reply_to": "help@", "enabled": false }))
        )
        .await,
        "The Reply-To address help@ is not a valid email address."
    );
    assert_eq!(
        put(NOT_READY, with(json!({ "sender": "" }))).await,
        "Enter the sender address."
    );
    assert_eq!(
        put(NOT_READY, with(json!({ "username": "" }))).await,
        "Enter the SMTP username."
    );

    // The same login needs a mailbox, and only goes to the mailbox's provider.
    assert!(
        put(NOT_READY, with(json!({ "same_login_as_mailbox": true })))
            .await
            .starts_with("The mailbox has no login yet.")
    );
    save_mailbox(&t).await;
    assert!(put(
        NOT_READY,
        with(json!({
            "same_login_as_mailbox": true,
            "host": "smtp.elsewhere.net",
        })),
    )
    .await
    .starts_with("The mailbox login is only sent to the mailbox's provider"));
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        with(json!({ "same_login_as_mailbox": true, "sender": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sending"]["effective_sender"], "reports@example.com");

    // Nothing invalid was stored along the way.
    assert_eq!(
        get_ok(&t.app, "/api/settings").await["sending"]["host"],
        "smtp.example.com"
    );
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

    // Switched on behind the API's back, with nothing configured.
    set_setting(&t.pool, "sending", r#"{"enabled":true}"#)
        .await
        .unwrap();
    let (status, body) = post_json(&t.app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "incomplete");
    assert_eq!(body["error"], "Enter the SMTP server's host name.");
}

#[tokio::test]
async fn the_test_send_names_the_failed_step() {
    let t = TestApp::new().await;
    let (status, body) = put_json(
        &t.app,
        "/api/settings/sending",
        own_login("smtp.invalid", Some("smtp-secret")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut draft = own_login("smtp.invalid", None);
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

#[tokio::test]
async fn the_test_send_uses_the_account_on_screen_with_the_stored_password_for_its_own_account_only(
) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("test.db")).await.unwrap();
    let sender = Arc::new(MemorySender::default());
    let app = router_with_state(AppState::new(pool.clone()).with_mail_sender(sender.clone()));

    // Saved while off, with its password: the test runs before sending is turned on.
    let mut saved = own_login("smtp.example.com", Some("smtp-secret"));
    saved["enabled"] = json!(false);
    let (status, body) = put_json(&app, "/api/settings/sending", saved.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The draft as on screen: the stored password never comes back, so none is typed.
    let mut on_screen = saved.clone();
    on_screen.as_object_mut().unwrap().remove("password");
    let saved = on_screen;

    // The same account with another sender: the stored password is used.
    let mut draft = saved.clone();
    draft["sender"] = json!("alerts@example.com");
    let (status, body) = post_json(
        &app,
        "/api/settings/sending/test",
        json!({ "sending": draft.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // No test recipient: to the sender on screen; the one on screen once typed.
    assert_eq!(body["message"], "Test message sent to alerts@example.com.");
    assert_eq!(sender.sent()[0].to.to_string(), "alerts@example.com");
    draft["test_recipient"] = json!(" owner@example.org ");
    let (status, body) = post_json(
        &app,
        "/api/settings/sending/test",
        json!({ "sending": draft }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Test message sent to owner@example.org.");
    let config = &sender.configs()[0];
    assert_eq!(config.host, "smtp.example.com");
    assert_eq!(config.password, "smtp-secret");
    assert_eq!(config.sender.to_string(), "alerts@example.com");

    // Another server: the stored password is not sent there.
    let mut other = saved.clone();
    other["host"] = json!("smtp.elsewhere.example");
    let (status, body) = post_json(
        &app,
        "/api/settings/sending/test",
        json!({ "sending": other.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .starts_with("Enter the SMTP password"));
    other["password"] = json!("other-secret");
    let (status, body) = post_json(
        &app,
        "/api/settings/sending/test",
        json!({ "sending": other }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(sender.configs()[2].host, "smtp.elsewhere.example");
    assert_eq!(sender.configs()[2].password, "other-secret");

    // Nothing of the drafts was stored.
    let stored = get_ok(&app, "/api/settings").await["sending"].clone();
    assert_eq!(stored["host"], "smtp.example.com");
    assert_eq!(stored["sender"], "dmarc@example.com");
    assert_eq!(stored["enabled"], false);
    assert_eq!(stored["test_recipient"], "");
}

#[tokio::test]
async fn the_test_recipient_is_saved_and_defaults_to_the_sender() {
    let t = TestApp::new().await;
    let sending = get_ok(&t.app, "/api/settings").await["sending"].clone();
    assert_eq!(sending["test_recipient"], "");
    assert_eq!(sending["effective_test_recipient"], "");

    let mut body = own_login("smtp.example.com", Some("smtp-secret"));
    let (status, saved) = put_json(&t.app, "/api/settings/sending", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["sending"]["test_recipient"], "");
    assert_eq!(
        saved["sending"]["effective_test_recipient"],
        saved["sending"]["effective_sender"]
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
