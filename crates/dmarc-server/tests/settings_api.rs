//! Integration tests for the settings REST API.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_dns::TableResolver;
use dmarc_store::init;
use serde_json::{json, Value};
use tower::ServiceExt;
mod common;
use common::{get, get_ok, post_bytes, post_json, put_json, rec, report, send, TestApp};

#[tokio::test]
async fn test_get_settings_unconfigured() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (status, body) = get(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    let mailbox = &body["mailbox"];
    assert_eq!(mailbox["port"], 993);
    assert_eq!(mailbox["folder"], "INBOX");
    assert_eq!(mailbox["poll_interval_secs"], 300);
    assert_eq!(mailbox["host"], "");
    assert_eq!(mailbox["has_password"], false);
    assert_eq!(mailbox["configured"], false);
    // Never expose password field
    assert!(mailbox.get("password").is_none());
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 24);
    assert_eq!(body["monitoring"]["domain_reports_stopped_days"], 14);
    assert_eq!(body["retention"]["report_retention_months"], 12);
    assert!(body["monitoring"].get("report_retention_months").is_none());
}

#[tokio::test]
async fn test_save_and_retrieve_imap_settings() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    // Save full IMAP settings
    let (status, save_res) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "mail.corp.internal",
            "port": 993,
            "username": "reports@corp.internal",
            "password": "supersecretpassword",
            "folder": "DMARC",
            "poll_interval_secs": 120,
            "security": "tls",
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // Ensure password is NEVER serialized in response
    assert!(!save_res.to_string().contains("supersecretpassword"));
    let save_res = &save_res["mailbox"];
    assert_eq!(save_res["host"], "mail.corp.internal");
    assert_eq!(save_res["port"], 993);
    assert_eq!(save_res["username"], "reports@corp.internal");
    assert_eq!(save_res["folder"], "DMARC");
    assert_eq!(save_res["poll_interval_secs"], 120);
    assert_eq!(save_res["has_password"], true);
    assert_eq!(save_res["configured"], true);
    assert!(save_res.get("password").is_none());

    // GET should return the database settings
    let (get_status, get_res) = get(&app, "/api/settings").await;
    assert_eq!(get_status, StatusCode::OK);
    assert!(!get_res.to_string().contains("supersecretpassword"));
    let get_res = &get_res["mailbox"];
    assert_eq!(get_res["host"], "mail.corp.internal");
    assert_eq!(get_res["username"], "reports@corp.internal");
    assert_eq!(get_res["folder"], "DMARC");
    assert_eq!(get_res["poll_interval_secs"], 120);
    assert_eq!(get_res["has_password"], true);
    assert_eq!(get_res["configured"], true);
    assert!(get_res.get("password").is_none());

    // Update folder/interval without password: same mailbox, so the stored
    // password is kept.
    let (up_status, up_res) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "MAIL.corp.internal",
            "port": 993,
            "username": "reports@corp.internal",
            "password": "",
            "folder": "Reports",
            "poll_interval_secs": 180,
            "security": "tls",
        }),
    )
    .await;

    assert_eq!(up_status, StatusCode::OK);
    assert_eq!(up_res["mailbox"]["folder"], "Reports");
    assert_eq!(up_res["mailbox"]["poll_interval_secs"], 180);
    assert_eq!(up_res["mailbox"]["has_password"], true); // Still has password!
}

async fn save_stored_mailbox(app: &axum::Router) {
    let (status, _) = put_json(
        app,
        "/api/settings/mailbox",
        json!({
            "host": "mail.corp.internal",
            "port": 993,
            "username": "reports@corp.internal",
            "password": "stored-secret",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn save_requires_password_when_mailbox_changes() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    save_stored_mailbox(&app).await;

    // Changing host, port or username without a password would send the
    // stored password to the new target on the next poll.
    for changed in [
        json!({ "host": "attacker.example", "port": 993, "username": "reports@corp.internal", "security": "tls", "folder": "", "poll_interval_secs": 300 }),
        json!({ "host": "mail.corp.internal", "port": 1993, "username": "reports@corp.internal", "security": "tls", "folder": "", "poll_interval_secs": 300 }),
        json!({ "host": "mail.corp.internal", "port": 993, "username": "other@corp.internal", "security": "tls", "folder": "", "poll_interval_secs": 300 }),
        json!({ "host": "attacker.example", "port": 993, "username": "reports@corp.internal", "password": "  ", "security": "tls", "folder": "", "poll_interval_secs": 300 }),
    ] {
        let (status, body) = put_json(&app, "/api/settings/mailbox", changed.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{changed}");
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("password is required"));
    }

    // Nothing was written by the rejected requests.
    let (_, current) = get(&app, "/api/settings").await;
    assert_eq!(current["mailbox"]["host"], "mail.corp.internal");
    assert_eq!(current["mailbox"]["port"], 993);
    assert_eq!(current["mailbox"]["username"], "reports@corp.internal");

    // With a new password the change is accepted.
    let (status, body) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "imap.new.example",
            "port": 993,
            "username": "reports@corp.internal",
            "password": "new-secret",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mailbox"]["host"], "imap.new.example");
    assert_eq!(body["mailbox"]["has_password"], true);
}

#[tokio::test]
async fn save_rejects_too_short_poll_interval() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let (status, body) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "mail.corp.internal",
            "username": "u",
            "password": "p",
            "poll_interval_secs": 0,
            "port": 993,
            "security": "tls",
            "folder": "",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "unprocessable");
    assert_eq!(body["error"], "poll interval must be at least 10 seconds");
}

#[tokio::test]
async fn connect_endpoint_never_sends_stored_password_to_another_mailbox() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    save_stored_mailbox(&app).await;

    for target in [
        json!({ "host": "127.0.0.1", "port": 65534 }),
        json!({ "port": 65534 }),
        json!({ "username": "someone-else" }),
    ] {
        let (status, body) = post_json(&app, "/api/settings/imap/connect", target.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{target}");
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("password is required"));
    }
}

#[tokio::test]
async fn connect_endpoint_reuses_stored_password_for_stored_mailbox() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    // A stored mailbox on a closed local port: the connection fails, but the
    // request is accepted because the target is unchanged.
    let (status, _) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "127.0.0.1",
            "port": 65534,
            "username": "reports@corp.internal",
            "password": "stored-secret",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, res) = post_json(
        &app,
        "/api/settings/imap/connect",
        json!({ "host": "127.0.0.1", "port": 65534, "username": "reports@corp.internal" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["ok"], false);
    assert!(!res["message"].as_str().unwrap().contains("stored-secret"));
    assert_eq!(res["message"], "Could not reach 127.0.0.1:65534.");
}

#[tokio::test]
async fn failed_sync_is_reported_in_system_health() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (status, res) = post_json(&app, "/api/settings/imap/sync", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["started"], false);
    assert_eq!(res["reason"], "not_configured");
    assert!(res["message"].as_str().unwrap().contains("not set up"));
    let (_, health) = get(&app, "/api/system/health").await;
    assert!(health["imap_last_error"].is_null());
    assert!(health["imap_progress"].is_null());

    let (status, _) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "127.0.0.1",
            "port": 65534,
            "username": "reports@corp.internal",
            "password": "stored-secret",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Returns at once; the pass runs in the background.
    let (status, res) = post_json(&app, "/api/settings/imap/sync", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["started"], true, "{res}");
    assert!(res["reason"].is_null());

    // Poll health like the UI does until the pass has finished. Health reads the
    // progress before the stored outcome and the pass writes its outcome in one
    // transaction before clearing its progress, so the first idle response already
    // carries the complete error. The bound only guards against a hang (a refused
    // connection can take seconds on Windows).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let health = loop {
        let (status, health) = get(&app, "/api/system/health").await;
        assert_eq!(status, StatusCode::OK);
        match health["imap_progress"].as_object() {
            None => break health,
            Some(progress) => {
                assert_eq!(progress["running"], true);
                assert!(progress["started_at"].as_i64().is_some());
            }
        }
        assert!(std::time::Instant::now() < deadline, "sync never finished");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(health["imap_configured"], true);
    let error = health["imap_last_error"]
        .as_str()
        .expect("last error recorded");
    assert!(error.contains("127.0.0.1:65534"), "{error}");
    assert!(!error.contains("stored-secret"));
    assert!(health["imap_last_error_at"].as_i64().is_some());
    assert!(health["imap_last_success_at"].is_null());
}

#[tokio::test]
async fn connect_endpoint_reports_unreachable_server() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    // Test with non-existent port on localhost -> should fail gracefully with ok: false
    let (status, res) = post_json(
        &app,
        "/api/settings/imap/connect",
        json!({
            "host": "127.0.0.1",
            "port": 65534,
            "username": "fake_user",
            "password": "fake_password"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Could not reach 127.0.0.1:65534.");
    assert!(res["detail"].as_str().is_some_and(|d| !d.is_empty()));
    assert!(!res.to_string().contains("fake_password"));
    assert_eq!(res["folders"], json!([]));
    assert!(res["selected"].is_null());
}

#[tokio::test]
async fn imap_security_round_trip_and_its_port() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let body = get(&app, "/api/settings").await.1;
    assert_eq!(body["mailbox"]["security"], "tls");
    assert_eq!(body["mailbox"]["defaults"]["port"], 993);
    assert_eq!(body["mailbox"]["defaults"]["starttls_port"], 143);

    let mailbox = json!({
        "host": "mail.corp.internal",
        "port": 143,
        "security": "starttls",
        "username": "reports@corp.internal",
        "password": "secret",
        "folder": "",
        "poll_interval_secs": 300,
    });
    let (status, saved) = put_json(&app, "/api/settings/mailbox", mailbox.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["mailbox"]["security"], "starttls");
    assert_eq!(saved["mailbox"]["port"], 143);
    assert_eq!(saved["mailbox"]["folder"], "INBOX", "empty: INBOX");

    // The whole group is sent: without the security nothing is saved.
    let mut partial = mailbox.clone();
    partial.as_object_mut().unwrap().remove("security");
    let (status, _) = put_json(&app, "/api/settings/mailbox", partial).await;
    assert!(status.is_client_error(), "{status}");

    let mut plain = mailbox;
    plain["security"] = json!("plain");
    let (status, _) = put_json(&app, "/api/settings/mailbox", plain).await;
    assert!(status.is_client_error(), "{status}");
    assert_eq!(
        get(&app, "/api/settings").await.1["mailbox"]["security"],
        "starttls"
    );
}

#[tokio::test]
async fn connect_endpoint_says_when_the_server_does_not_offer_starttls() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        stream.write_all(b"* OK IMAP ready\r\n").await.unwrap();
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        let tag = line.split_whitespace().next().unwrap_or("*").to_owned();
        stream
            .write_all(format!("{tag} BAD Unknown command\r\n").as_bytes())
            .await
            .unwrap();
    });
    let t = TestApp::new().await;
    let (status, res) = post_json(
        &t.app,
        "/api/settings/imap/connect",
        json!({
            "host": "127.0.0.1",
            "port": port,
            "security": "starttls",
            "username": "u",
            "password": "fake_password"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["ok"], false);
    assert_eq!(
        res["message"],
        format!("The server does not offer STARTTLS on port {port}. Choose TLS (usually port 993) instead.")
    );
    assert!(!res.to_string().contains("fake_password"));
}

#[tokio::test]
async fn connect_endpoint_reports_unknown_host_and_missing_fields() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (status, res) = post_json(
        &app,
        "/api/settings/imap/connect",
        json!({ "host": "imap.invalid", "username": "u", "password": "p" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Server not found. Check the host name.");

    let (status, res) = post_json(
        &app,
        "/api/settings/imap/connect",
        json!({ "host": "imap.invalid", "username": "u" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Enter the password.");
}

#[tokio::test]
async fn test_dkim_selectors_api_crud() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    // 1. Initially empty
    let (status, body) = get(&app, "/api/settings/dkim-selectors").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "text": "" }));

    // 2. The preview names what saving adds; the save does it, normalized,
    // deduplicated and sorted.
    let draft = json!({ "text": "mail\n CORP2024 , MAIL\n\n" });
    let (status, preview) =
        post_json(&app, "/api/settings/dkim-selectors/preview", draft.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        preview,
        json!({ "changes": { "add": ["corp2024", "mail"], "remove": [] }, "errors": [] })
    );
    let (status, body) = put_json(&app, "/api/settings/dkim-selectors", draft).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, preview);

    // 3. An invalid selector rejects the whole list, naming its line
    let draft = json!({ "text": "ok\nnot valid, bad/one\n" });
    let (_, preview) = post_json(&app, "/api/settings/dkim-selectors/preview", draft.clone()).await;
    assert_eq!(
        preview["errors"],
        json!([{ "line": 2, "message": "not a DKIM selector: bad/one" }])
    );
    let (status, body) = put_json(&app, "/api/settings/dkim-selectors", draft).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["errors"], preview["errors"]);

    // 4. GET reflects the saved list
    let (status, body) = get(&app, "/api/settings/dkim-selectors").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["text"], "corp2024\nmail\n");

    // 5. An empty list clears it
    let (status, body) =
        put_json(&app, "/api/settings/dkim-selectors", json!({ "text": "" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["changes"]["remove"], json!(["corp2024", "mail"]));
    assert_eq!(
        get(&app, "/api/settings/dkim-selectors").await.1["text"],
        ""
    );
}

#[tokio::test]
async fn posts_must_be_json() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    for (content_type, body) in [
        (None, ""),
        (
            Some("text/plain"),
            r#"{"host":"internal.example","username":"x"}"#,
        ),
        (
            Some("application/x-www-form-urlencoded"),
            "host=internal.example",
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/settings/imap/connect");
        if let Some(ct) = content_type {
            request = request.header(header::CONTENT_TYPE, ct);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{content_type:?}"
        );
    }
}

/// axum's own rejections answer like every other error: a JSON body with
/// `error` and `code`, 400 for a malformed request, 405 for a wrong method.
#[tokio::test]
async fn malformed_requests_are_json_errors() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let malformed = [
        // Not JSON.
        post_bytes(
            &app,
            "/api/settings/sending/test",
            "application/json",
            b"{not json".to_vec(),
        )
        .await,
        // A field missing.
        put_json(&app, "/api/settings/branding", json!({})).await,
        // A field of the wrong type.
        put_json(
            &app,
            "/api/settings/retention",
            json!({ "report_retention_months": "six" }),
        )
        .await,
        // A query parameter missing.
        get(&app, "/api/dns/dmarc").await,
        // A path segment of the wrong type.
        get(&app, "/api/domains/abc").await,
    ];
    for (status, body) in malformed {
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["code"], "bad_request", "{body}");
        assert!(!body["error"].as_str().unwrap().is_empty(), "{body}");
    }
    let (status, body) = send(&app, "DELETE", "/api/settings", None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["code"], "method_not_allowed");
}

#[tokio::test]
async fn test_dns_settings_api() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    // 1. Initial GET: nothing configured, the defaults are named
    let (status, body) = get(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["dns"]["nameservers"], json!([]));
    assert_eq!(body["dns"]["defaults"], json!(["1.1.1.1", "8.8.8.8"]));

    // 3. Reject more than 4 nameservers
    let (status, body) = put_json(
        &app,
        "/api/settings/dns",
        json!({ "nameservers": ["1.1.1.1", "8.8.8.8", "9.9.9.9", "1.0.0.1", "8.8.4.4"] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "unprocessable");
    assert_eq!(body["error"], "at most 4 nameservers can be configured");

    // 4. Reject invalid IP format
    let (status, body) = put_json(
        &app,
        "/api/settings/dns",
        json!({ "nameservers": ["not-an-ip"] }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "unprocessable");
    assert_eq!(body["error"], "invalid IP address format: not-an-ip");

    // 5. Successfully update to custom nameservers
    let (status, body) = put_json(
        &app,
        "/api/settings/dns",
        json!({ "nameservers": ["9.9.9.9", "149.112.112.112"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The saved settings come back, like every other settings PUT.
    assert_eq!(
        body["dns"]["nameservers"],
        json!(["9.9.9.9", "149.112.112.112"])
    );

    // 6. GET now returns the stored nameservers
    let (status, body) = get(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["dns"]["nameservers"],
        json!(["9.9.9.9", "149.112.112.112"])
    );

    // 7. Saving no resolvers (blank fields) goes back to the defaults
    let (status, body) = put_json(
        &app,
        "/api/settings/dns",
        json!({ "nameservers": ["", " "] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["dns"]["nameservers"], json!([]));
}

#[tokio::test]
async fn report_address_round_trip_and_validation() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["report_address"], "");
    assert_eq!(body["reporting"]["report_org_domain"], "");

    let with = |addr: &str| json!({ "report_address": addr });

    let (status, body) = put_json(
        &app,
        "/api/settings/reporting",
        with(" dmarc@Reports.example "),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["report_address"], "dmarc@Reports.example");
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["report_address"], "dmarc@Reports.example");

    // Omitting the field keeps the stored value, and so does saving the mailbox.
    let (status, body) = put_json(&app, "/api/settings/reporting", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["report_address"], "dmarc@Reports.example");
    let (status, body) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "mail.corp.internal",
            "port": 993,
            "username": "imap-user",
            "password": "pw",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["report_address"], "dmarc@Reports.example");

    for bad in ["not-an-email", "a@b", "a b@example.com", "x@example.com;y"] {
        let (status, body) = put_json(&app, "/api/settings/reporting", with(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
        assert_eq!(body["code"], "unprocessable");
        assert_eq!(
            body["error"],
            format!("report address is not a valid email address: {bad}")
        );
    }
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["report_address"], "dmarc@Reports.example");

    // An empty string clears it.
    let (status, body) = put_json(&app, "/api/settings/reporting", with("")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["report_address"], "");
}

#[tokio::test]
async fn reporting_mode_round_trip_and_validation() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["reporting_mode"], "direct", "default");
    assert_eq!(body["reporting"]["reporting_mode_saved"], false);

    let with = |mode: &str| json!({ "reporting_mode": mode });

    // Saving the connection alone leaves the mode unsaved.
    let (status, body) = put_json(
        &app,
        "/api/settings/mailbox",
        json!({
            "host": "mail.corp.internal",
            "port": 993,
            "username": "imap-user",
            "password": "pw",
            "security": "tls",
            "folder": "",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["reporting_mode_saved"], false);

    let (status, body) = put_json(&app, "/api/settings/reporting", with("forwarded")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");
    assert_eq!(body["reporting"]["reporting_mode_saved"], true);
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");

    // Omitting the field keeps the stored value.
    let (status, body) = put_json(
        &app,
        "/api/settings/reporting",
        json!({ "report_address": "dmarc@example.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");

    // Unknown values are rejected and leave the stored mode alone.
    let (status, _) = put_json(&app, "/api/settings/reporting", with("alias")).await;
    assert!(status.is_client_error(), "got {status}");
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");

    let (status, body) = put_json(&app, "/api/settings/reporting", with("direct")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporting"]["reporting_mode"], "direct");
}

/// Each domain's reporting status is the server's: a lookup that fails is
/// "check failed", never "not authorized" or "no record".
#[tokio::test]
async fn reporting_setup_reports_failed_lookups_as_check_failed() {
    // A resolver nobody answers on: every lookup fails, at once.
    let t = TestApp::with_resolver(TableResolver::failing()).await;
    let app = t.app.clone();

    // No report address: nothing to check the domains against.
    let body = get_ok(&app, "/api/reporting-setup").await;
    assert_eq!(body["report_address"], "");
    assert_eq!(body["domains"], json!([]));
    assert_eq!(body["mailbox_authorization"], Value::Null);

    let (status, _) = put_json(
        &app,
        "/api/settings/reporting",
        json!({ "report_address": "dmarc@Reports.test", "reporting_mode": "direct" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, organizations) = get(&app, "/api/organizations").await;
    let org = organizations["items"][0]["id"].as_i64().unwrap();
    let (status, created) = post_json(
        &app,
        "/api/domains",
        json!({ "organization_id": org, "domain": "example.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");

    let body = get_ok(&app, "/api/reporting-setup?refresh=true").await;
    assert_eq!(body["mailbox_domain"], "reports.test");
    assert_eq!(body["mailbox_authorization"]["state"], "check_failed");
    let row = &body["domains"][0];
    assert_eq!(row["domain"], "example.com");
    assert_eq!(row["state"], "check_failed");
    assert_eq!(row["status"], "neutral");
    assert_eq!(row["set_up"], false);
    assert_eq!(row["mailbox_auth_missing"], false);

    let (status, _) = get(&app, "/api/reporting-setup?domain_id=999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// With the records published, the reporting setup reads them: a domain
/// whose `rua=` names the report address and whose reports the mailbox
/// domain accepts is set up and waits for its first report; one with no
/// record (and no authorization record) says so, and with no wildcard the
/// mailbox domain says that too.
#[tokio::test]
async fn reporting_setup_reads_the_published_records() {
    let resolver = TableResolver::new()
        .with_txt(
            "_dmarc.example.com",
            &["v=DMARC1; p=reject; rua=mailto:dmarc@reports.test"],
        )
        .with_txt("example.com._report._dmarc.reports.test", &["v=DMARC1"]);
    let t = TestApp::with_resolver(resolver).await;
    let app = t.app.clone();
    let (status, _) = put_json(
        &app,
        "/api/settings/reporting",
        json!({ "report_address": "dmarc@reports.test", "reporting_mode": "direct" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, organizations) = get(&app, "/api/organizations").await;
    let org = organizations["items"][0]["id"].as_i64().unwrap();
    for domain in ["example.com", "example.net"] {
        let (status, created) = post_json(
            &app,
            "/api/domains",
            json!({ "organization_id": org, "domain": domain }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
    }

    let body = get_ok(&app, "/api/reporting-setup?refresh=true").await;
    assert_eq!(
        body["mailbox_authorization"]["state"], "no_wildcard",
        "{body}"
    );
    let row = |name: &str| {
        body["domains"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["domain"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("{name} missing: {body}"))
    };
    let set_up = row("example.com");
    assert_eq!(set_up["state"], "waiting_for_first_report", "{set_up}");
    assert_eq!(set_up["set_up"], true);
    assert_eq!(set_up["mailbox_auth_missing"], false);
    let missing = row("example.net");
    assert_eq!(missing["state"], "no_record", "{missing}");
    assert_eq!(missing["set_up"], false);
    assert_eq!(missing["mailbox_auth_missing"], true);
    // Worst first: the domain that needs a DNS change leads.
    assert_eq!(body["domains"][0]["domain"], "example.net");
}

#[tokio::test]
async fn onboarding_state_round_trip() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let (status, body) = get(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["onboarding"]["state"],
        Value::Null,
        "fresh install has not onboarded"
    );

    for value in ["skipped", "completed"] {
        let (status, body) =
            put_json(&app, "/api/settings/onboarding", json!({ "state": value })).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["onboarding"]["state"], value);
        let (_, body) = get(&app, "/api/settings").await;
        assert_eq!(body["onboarding"]["state"], value);
    }

    // Unknown values are rejected and leave the stored state alone.
    let (status, _) = put_json(&app, "/api/settings/onboarding", json!({ "state": "done" })).await;
    assert!(status.is_client_error(), "got {status}");
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["onboarding"]["state"], "completed");

    // `null` clears it so the setup opens again.
    let (status, body) = put_json(&app, "/api/settings/onboarding", json!({ "state": null })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["onboarding"]["state"], Value::Null);
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["onboarding"]["state"], Value::Null);
}

/// Saving the delivery settings must not interrupt a running import; only a
/// different mailbox does. Passwords are saved verbatim.
#[tokio::test]
async fn delivery_settings_do_not_interrupt_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let state = dmarc_server::AppState::new(pool.clone());
    let worker = state.imap.clone();
    let app = dmarc_server::router_with_state(state);

    let mailbox = json!({
        "host": "imap.example.com",
        "port": 993,
        "username": "reports@example.com",
        "password": "  spaced secret  ",
        "folder": "INBOX",
        "security": "tls",
        "poll_interval_secs": 300,
    });
    let (status, body) = put_json(&app, "/api/settings/mailbox", mailbox.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after_first_save = worker.settings_generation();
    assert_eq!(after_first_save, 1);
    let stored = dmarc_store::AppSettings::load(&pool).await.unwrap().mailbox;
    assert_eq!(
        stored.password, "  spaced secret  ",
        "passwords are not trimmed"
    );

    // The same mailbox again (password omitted: the stored one is kept), then
    // the delivery settings.
    let mut same = mailbox.clone();
    same["password"] = json!("");
    let (status, body) = put_json(&app, "/api/settings/mailbox", same).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_json(
        &app,
        "/api/settings/reporting",
        json!({
            "report_address": "dmarc@reports.firma.co.uk",
            "reporting_mode": "forwarded",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(worker.settings_generation(), after_first_save);
    assert_eq!(
        body["reporting"]["report_address"],
        "dmarc@reports.firma.co.uk"
    );
    assert_eq!(body["reporting"]["report_org_domain"], "firma.co.uk");
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");
    let stored = dmarc_store::AppSettings::load(&pool).await.unwrap().mailbox;
    assert_eq!(stored.password, "  spaced secret  ");

    // A different folder is a different mailbox pass.
    let mut other = mailbox;
    other["folder"] = json!("DMARC");
    other["password"] = json!("");
    let (status, _) = put_json(&app, "/api/settings/mailbox", other).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(worker.settings_generation(), after_first_save + 1);

    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["reporting"]["report_org_domain"], "firma.co.uk");
}

/// The report retention defaults to 12 months and is its own group, saved
/// clamped to 1–120; the freshness warning (monitoring, clamped to 1–168
/// hours) is another group, and neither write touches the other.
#[tokio::test]
async fn report_retention_round_trip() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["retention"]["report_retention_months"], 12);

    let (status, body) = put_json(
        &app,
        "/api/settings/monitoring",
        json!({ "report_freshness_warning_hours": 48, "domain_reports_stopped_days": 10 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 48);
    assert_eq!(body["monitoring"]["domain_reports_stopped_days"], 10);

    let retention = |months: u32| json!({ "report_retention_months": months });
    let (status, body) = put_json(&app, "/api/settings/retention", retention(24)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["retention"]["report_retention_months"], 24);
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["retention"]["report_retention_months"], 24);
    assert_eq!(
        body["monitoring"]["report_freshness_warning_hours"], 48,
        "the retention write leaves the arrival warning"
    );

    for (sent, stored) in [(0, 1), (500, 120)] {
        let (status, body) = put_json(&app, "/api/settings/retention", retention(sent)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["retention"]["report_retention_months"], stored);
    }
    for (sent, stored, stored_days) in [(0, 1, 1), (500, 168, 30)] {
        let (status, body) = put_json(
            &app,
            "/api/settings/monitoring",
            json!({ "report_freshness_warning_hours": sent, "domain_reports_stopped_days": sent }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["monitoring"]["report_freshness_warning_hours"], stored);
        assert_eq!(
            body["monitoring"]["domain_reports_stopped_days"],
            stored_days
        );
        assert_eq!(
            body["retention"]["report_retention_months"], 120,
            "the monitoring write leaves the retention"
        );
    }

    // A group without its value is refused and nothing is saved.
    for group in ["retention", "monitoring"] {
        let (status, _) = put_json(&app, &format!("/api/settings/{group}"), json!({})).await;
        assert!(status.is_client_error(), "{group}: {status}");
    }
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["retention"]["report_retention_months"], 120);
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 168);
}

/// `GET /api/settings` reflects each group's write; every PUT answers it.
#[tokio::test]
async fn get_settings_reflects_each_group_write() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let writes = [
        (
            "mailbox",
            json!({ "host": "imap.example.com", "username": "u@example.com", "password": "pw", "folder": "DMARC", "port": 993, "security": "tls", "poll_interval_secs": 300 }),
        ),
        (
            "reporting",
            json!({ "report_address": "dmarc@example.com", "reporting_mode": "forwarded" }),
        ),
        (
            "monitoring",
            json!({ "report_freshness_warning_hours": 48, "domain_reports_stopped_days": 10 }),
        ),
        ("retention", json!({ "report_retention_months": 6 })),
        ("dns", json!({ "nameservers": ["9.9.9.9"] })),
        ("onboarding", json!({ "state": "completed" })),
    ];
    for (group, body) in &writes {
        let (status, answer) =
            put_json(&app, &format!("/api/settings/{group}"), body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{group}: {answer}");
        assert_eq!(answer, get(&app, "/api/settings").await.1, "{group}");
    }
    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["mailbox"]["host"], "imap.example.com");
    assert_eq!(body["mailbox"]["folder"], "DMARC");
    assert_eq!(body["mailbox"]["has_password"], true);
    assert_eq!(body["mailbox"]["configured"], true);
    assert_eq!(body["reporting"]["report_address"], "dmarc@example.com");
    assert_eq!(body["reporting"]["report_org_domain"], "example.com");
    assert_eq!(body["reporting"]["reporting_mode"], "forwarded");
    assert_eq!(body["reporting"]["reporting_mode_saved"], true);
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 48);
    assert_eq!(body["retention"]["report_retention_months"], 6);
    assert_eq!(body["dns"]["nameservers"], json!(["9.9.9.9"]));
    assert_eq!(body["onboarding"]["state"], "completed");
}

/// The clean-up applies the stored retention: with one month, a report that
/// ended 40 days ago goes and yesterday's stays, and so do alert events.
#[tokio::test]
async fn retention_pass_applies_the_stored_months() {
    let t = TestApp::new().await;
    dmarc_store::save_settings(
        &t.pool,
        &dmarc_store::RetentionSettings {
            report_retention_months: 1,
        },
    )
    .await
    .unwrap();
    let now = dmarc_store::unix_now();
    for (id, days_ago) in [("old", 41), ("recent", 2)] {
        let r = report(
            id,
            "example.com",
            now - days_ago * 86_400,
            vec![rec("192.0.2.1", 5, "pass", "pass")],
        );
        dmarc_store::ingest_report(&t.pool, &r).await.unwrap();
    }
    let domain_id: i64 = sqlx::query_scalar("SELECT id FROM domains WHERE domain = 'example.com'")
        .fetch_one(&t.pool)
        .await
        .unwrap();
    for days_ago in [41, 2] {
        let run = [dmarc_store::AlertEvaluation {
            domain_id,
            state: dmarc_assess::AlertState::default(),
            events: vec![dmarc_assess::AlertEvent {
                kind: dmarc_assess::AlertKind::ParkedChanged { parked: true },
                severity: dmarc_assess::Status::Neutral,
            }],
        }];
        dmarc_store::save_alert_evaluations(&t.pool, now - days_ago * 86_400, &run)
            .await
            .unwrap();
    }

    dmarc_server::background::apply_retention(&t.pool).await;

    let left: Vec<String> = sqlx::query_scalar("SELECT report_id FROM reports")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(left, vec!["recent"]);
    let events: Vec<i64> = sqlx::query_scalar("SELECT created_at FROM alert_events")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(events, vec![now - 2 * 86_400]);
}

/// A stored retention that does not read as months skips the clean-up: the
/// default (12 months) in its place could delete what a longer setting keeps.
#[tokio::test]
async fn retention_pass_skips_an_unreadable_setting() {
    let t = TestApp::new().await;
    dmarc_store::set_setting(
        &t.pool,
        "retention",
        r#"{"report_retention_months":"twenty"}"#,
    )
    .await
    .unwrap();
    let now = dmarc_store::unix_now();
    let r = report(
        "two-years-old",
        "example.com",
        now - 730 * 86_400,
        vec![rec("192.0.2.1", 5, "pass", "pass")],
    );
    dmarc_store::ingest_report(&t.pool, &r).await.unwrap();

    dmarc_server::background::apply_retention(&t.pool).await;

    let left: Vec<String> = sqlx::query_scalar("SELECT report_id FROM reports")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(left, vec!["two-years-old"]);
}

/// The per-setting endpoints that came before the groups are gone.
#[tokio::test]
async fn old_per_setting_endpoints_are_gone() {
    let t = TestApp::new().await;
    for path in [
        "/api/settings/imap",
        "/api/settings/dns",
        "/api/settings/onboarding",
    ] {
        let (status, _) = get(&t.app, path).await;
        assert!(status.is_client_error(), "GET {path}: {status}");
    }
    for path in [
        "/api/settings/imap",
        "/api/settings/report-freshness",
        "/api/settings/report-retention",
        "/api/settings/dns",
        "/api/settings/onboarding",
    ] {
        let (status, _) = post_json(&t.app, path, json!({})).await;
        assert!(status.is_client_error(), "POST {path}: {status}");
    }
}
