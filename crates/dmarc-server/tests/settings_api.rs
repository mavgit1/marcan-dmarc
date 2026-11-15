//! Integration tests for the settings REST API.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_dns::TableResolver;
use serde_json::{json, Value};
use tower::ServiceExt;
mod common;
use common::{get, get_ok, post_bytes, post_json, put_json, rec, report, send, TestApp};

/// Answered when a stored password would go to another server or username.
const PASSWORD_AGAIN: &str = "Enter the password again: the stored one is only used for the servers and username it was saved for.";

/// A receiving account on `host:port` (TLS) logging in as `username`, with
/// no SMTP server, in the shape `/api/accounts` takes.
fn imap_account(host: &str, port: u16, username: &str, password: Option<&str>) -> Value {
    let mut login = json!({ "kind": "password", "username": username });
    if let Some(password) = password {
        login["password"] = json!(password);
    }
    json!({
        "name": "Reports",
        "provider": "generic",
        "address": "reports@corp.internal",
        "login": login,
        "imap": { "host": host, "port": port, "security": "tls" },
        "smtp": { "host": "", "port": null, "security": "tls" },
        "receiving": { "enabled": true, "folder": "", "forwarded_from": [] },
    })
}

/// `POST /api/accounts/test` of the IMAP login of `account` (`id`: the
/// stored account it edits); its `imap` step.
async fn test_imap(app: &axum::Router, id: Option<i64>, account: Value) -> Value {
    let (status, res) = post_json(
        app,
        "/api/accounts/test",
        json!({ "id": id, "account": account, "smtp": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert!(res["smtp"].is_null(), "{res}");
    res["mailbox"].clone()
}

#[tokio::test]
async fn test_get_settings_unconfigured() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (status, body) = get(&app, "/api/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("mailbox").is_none(), "the 0.3 group is gone");
    assert_eq!(body["accounts"], json!([]));
    assert_eq!(body["receiving"]["poll_interval_secs"], 300);
    assert_eq!(
        body["account_defaults"],
        json!({
            "imap": { "tls": 993, "starttls": 143 },
            "smtp": { "tls": 465, "starttls": 587 },
            "folder": "INBOX",
        })
    );
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 24);
    assert_eq!(body["monitoring"]["domain_reports_stopped_days"], 14);
    assert_eq!(body["retention"]["report_retention_months"], 12);
    assert!(body["monitoring"].get("report_retention_months").is_none());
}

/// An account is answered without its password, by its `POST`, its `PUT`
/// and `GET /api/settings`; saved again for the same server (the host's
/// case aside) without a password, the stored one stays; a blank password
/// is none, so another server needs it typed.
#[tokio::test]
async fn an_account_round_trips_without_its_password() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let mut account = imap_account(
        "mail.corp.internal",
        993,
        "reports@corp.internal",
        Some("supersecretpassword"),
    );
    account["receiving"]["folder"] = json!("DMARC");
    let (status, saved) = post_json(&app, "/api/accounts", account).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(!saved.to_string().contains("supersecretpassword"));
    let stored = &saved["accounts"][0];
    assert_eq!(stored["imap"]["host"], "mail.corp.internal");
    assert_eq!(stored["imap"]["port"], 993);
    assert_eq!(stored["login"]["username"], "reports@corp.internal");
    assert!(stored["login"].get("password").is_none());
    assert_eq!(stored["login"]["has_password"], true);
    assert_eq!(stored["receiving"]["folder"], "DMARC");
    assert_eq!(stored["ready_to_receive"], true);

    let got = get_ok(&app, "/api/settings").await;
    assert!(!got.to_string().contains("supersecretpassword"));
    assert_eq!(got["accounts"], saved["accounts"]);

    let mut same = imap_account("MAIL.corp.internal", 993, "reports@corp.internal", Some(""));
    same["receiving"]["folder"] = json!("Reports");
    let (status, res) = put_json(&app, "/api/accounts/1", same).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["accounts"][0]["receiving"]["folder"], "Reports");
    assert_eq!(res["accounts"][0]["login"]["has_password"], true);

    let blank = imap_account("attacker.example", 993, "reports@corp.internal", Some("  "));
    let (status, res) = put_json(&app, "/api/accounts/1", blank).await;
    assert_eq!(status, StatusCode::CONFLICT, "{res}");
    assert_eq!(res["error"], PASSWORD_AGAIN);
    let current = get_ok(&app, "/api/settings").await;
    assert_eq!(current["accounts"][0]["imap"]["host"], "MAIL.corp.internal");

    let typed = imap_account(
        "imap.new.example",
        993,
        "reports@corp.internal",
        Some("new-secret"),
    );
    let (status, res) = put_json(&app, "/api/accounts/1", typed).await;
    assert_eq!(status, StatusCode::OK, "{res}");
    assert_eq!(res["accounts"][0]["imap"]["host"], "imap.new.example");
    assert_eq!(res["accounts"][0]["login"]["has_password"], true);
}

/// Through the real network: the stored account's own server is tried with
/// its stored password (a closed local port, so the step fails, without the
/// password in the answer); another port, host or username is not tried.
#[tokio::test]
async fn account_test_uses_the_stored_password_for_its_own_server_only() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let stored = imap_account(
        "127.0.0.1",
        65534,
        "reports@corp.internal",
        Some("stored-secret"),
    );
    let (status, body) = post_json(&app, "/api/accounts", stored).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let same = imap_account("127.0.0.1", 65534, "reports@corp.internal", None);
    let res = test_imap(&app, Some(1), same).await;
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Could not reach 127.0.0.1:65534.");
    assert!(!res.to_string().contains("stored-secret"));

    for (host, port, username) in [
        ("127.0.0.1", 65533, "reports@corp.internal"),
        ("localhost", 65534, "reports@corp.internal"),
        ("127.0.0.1", 65534, "someone-else"),
    ] {
        let res = test_imap(&app, Some(1), imap_account(host, port, username, None)).await;
        assert_eq!(res["ok"], false);
        assert_eq!(res["message"], PASSWORD_AGAIN, "{host}:{port} {username}");
    }
}

#[tokio::test]
async fn failed_sync_is_reported_in_system_health() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (status, res) = post_json(&app, "/api/settings/receiving/sync", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["started"], false);
    assert_eq!(res["reason"], "not_configured");
    assert!(res["message"]
        .as_str()
        .unwrap()
        .starts_with("No account receives reports yet."));
    let (_, health) = get(&app, "/api/system/health").await;
    assert!(health["receive_last_error"].is_null());
    assert!(health["pass_progress"].is_null());

    let account = imap_account(
        "127.0.0.1",
        65534,
        "reports@corp.internal",
        Some("stored-secret"),
    );
    let (status, body) = post_json(&app, "/api/accounts", account).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The loop, as the server runs it, reads the new account at once: its
    // pass fails to connect and backs off.
    let imap = t.run_imap_loop();
    let passes = || async {
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM receive_passes")
            .fetch_one(&t.pool)
            .await
            .unwrap();
        n
    };
    // Poll health like the UI does until the pass has finished. Health reads the
    // progress before the stored outcome and the pass writes its outcome in one
    // transaction before clearing its progress, so the first idle response already
    // carries the complete error. The bound only guards against a hang (a refused
    // connection can take seconds on Windows).
    let until_idle = |count: i64| {
        let app = app.clone();
        let passes = &passes;
        async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                let (status, health) = get(&app, "/api/system/health").await;
                assert_eq!(status, StatusCode::OK);
                match health["pass_progress"].as_object() {
                    None if passes().await >= count => break health,
                    None => {}
                    Some(progress) => {
                        assert_eq!(progress["running"], true);
                        assert!(progress["started_at"].as_i64().is_some());
                    }
                }
                assert!(std::time::Instant::now() < deadline, "sync never finished");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    };
    until_idle(1).await;

    // Returns at once; the loop runs the pass, and the very next health
    // poll already shows it.
    let (status, res) = post_json(&app, "/api/settings/receiving/sync", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(res["started"], true, "{res}");
    assert!(res["reason"].is_null());
    let (_, health) = get(&app, "/api/system/health").await;
    assert_eq!(health["pass_progress"]["running"], true, "{health}");
    let health = until_idle(2).await;
    imap.stop().await;
    assert_eq!(passes().await, 2);
    assert_eq!(health["receive_configured"], true);
    let error = health["receive_last_error"]
        .as_str()
        .expect("last error recorded");
    assert!(error.contains("127.0.0.1:65534"), "{error}");
    assert!(!error.contains("stored-secret"));
    assert!(health["receive_last_error_at"].as_i64().is_some());
    assert!(health["receive_last_success_at"].is_null());
}

#[tokio::test]
async fn account_test_reports_an_unreachable_server() {
    let t = TestApp::new().await;

    // A closed port on localhost: the step fails, answered with ok: false.
    let account = imap_account("127.0.0.1", 65534, "fake_user", Some("fake_password"));
    let res = test_imap(&t.app, None, account).await;
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Could not reach 127.0.0.1:65534.");
    assert!(res["detail"].as_str().is_some_and(|d| !d.is_empty()));
    assert!(!res.to_string().contains("fake_password"));
    assert_eq!(res["folders"], json!([]));
    assert!(res["selected"].is_null());
}

#[tokio::test]
async fn account_imap_security_round_trip_and_its_port() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let body = get(&app, "/api/settings").await.1;
    assert_eq!(body["account_defaults"]["imap"]["tls"], 993);
    assert_eq!(body["account_defaults"]["imap"]["starttls"], 143);

    let mut account = imap_account(
        "mail.corp.internal",
        143,
        "reports@corp.internal",
        Some("secret"),
    );
    account["imap"]["security"] = json!("starttls");
    let (status, saved) = post_json(&app, "/api/accounts", account.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["accounts"][0]["imap"]["security"], "starttls");
    assert_eq!(saved["accounts"][0]["imap"]["port"], 143);
    assert_eq!(
        saved["accounts"][0]["receiving"]["folder"], "INBOX",
        "empty: INBOX"
    );

    // The whole account is sent: without the security nothing is saved.
    let mut partial = account.clone();
    partial["imap"].as_object_mut().unwrap().remove("security");
    let (status, _) = put_json(&app, "/api/accounts/1", partial).await;
    assert!(status.is_client_error(), "{status}");

    let mut plain = account.clone();
    plain["imap"]["security"] = json!("plain");
    let (status, _) = put_json(&app, "/api/accounts/1", plain).await;
    assert!(status.is_client_error(), "{status}");

    // Port 0 is not set: the usual one for the security chosen (STARTTLS,
    // as saved above). One that is not a port number at all is a malformed
    // request, a 400 in JSON.
    let mut zero = account.clone();
    zero["imap"]["port"] = json!(0);
    zero["imap"]["security"] = json!("starttls");
    let (status, body) = put_json(&app, "/api/accounts/1", zero).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accounts"][0]["imap"]["port"], 143);
    let mut huge = account;
    huge["imap"]["port"] = json!(70000);
    let (status, body) = put_json(&app, "/api/accounts/1", huge).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    assert!(body["error"].as_str().unwrap().contains("70000"), "{body}");

    let imap = &get(&app, "/api/settings").await.1["accounts"][0]["imap"];
    assert_eq!(imap["security"], "starttls");
    assert_eq!(imap["port"], 143);
}

#[tokio::test]
async fn account_test_says_when_the_imap_server_does_not_offer_starttls() {
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
    let mut account = imap_account("127.0.0.1", port, "u", Some("fake_password"));
    account["imap"]["security"] = json!("starttls");
    let res = test_imap(&t.app, None, account).await;
    assert_eq!(res["ok"], false);
    assert_eq!(
        res["message"],
        format!("The server does not offer STARTTLS on port {port}. Choose TLS (usually port 993) instead.")
    );
    assert!(!res.to_string().contains("fake_password"));
}

#[tokio::test]
async fn account_test_reports_an_unknown_host_and_missing_fields() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let res = test_imap(
        &app,
        None,
        imap_account("imap.invalid", 993, "u", Some("p")),
    )
    .await;
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Server not found. Check the host name.");

    let res = test_imap(&app, None, imap_account("imap.invalid", 993, "u", None)).await;
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Enter the password.");

    let res = test_imap(
        &app,
        None,
        imap_account("imap.invalid", 993, " ", Some("p")),
    )
    .await;
    assert_eq!(res["ok"], false);
    assert_eq!(res["message"], "Enter the username.");

    // No server at all: nothing to test.
    let (status, res) = post_json(
        &app,
        "/api/accounts/test",
        json!({ "account": imap_account("", 993, "u", Some("p")), "smtp": true }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{res}");
    assert_eq!(res["error"], "Enter a server to test.");
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
            r#"{"account":{"imap":{"host":"internal.example"}},"smtp":false}"#,
        ),
        (
            Some("application/x-www-form-urlencoded"),
            "host=internal.example",
        ),
    ] {
        let mut request = Request::builder().method("POST").uri("/api/accounts/test");
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

/// A receiving account at `address` (on an IMAP server nobody answers), the
/// reports of the `forwarded_from` addresses forwarded to it.
fn report_account(address: &str, forwarded_from: &[&str]) -> Value {
    let mut account = imap_account("127.0.0.1", 65534, address, Some("pw"));
    account["name"] = json!(address);
    account["address"] = json!(address);
    account["receiving"]["forwarded_from"] = json!(forwarded_from);
    account
}

async fn add_domain(app: &axum::Router, domain: &str) -> i64 {
    let (_, organizations) = get(app, "/api/organizations").await;
    let org = organizations["items"][0]["id"].as_i64().unwrap();
    let (status, created) = post_json(
        app,
        "/api/domains",
        json!({ "organization_id": org, "domain": domain }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["id"].as_i64().unwrap()
}

/// The row of `name` in a reporting setup answer.
fn setup_row(body: &Value, name: &str) -> Value {
    body["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} missing: {body}"))
}

/// Each domain's reporting status is the server's: a lookup that fails is
/// "check failed", never "not authorized" or "no record".
#[tokio::test]
async fn reporting_setup_reports_failed_lookups_as_check_failed() {
    // A resolver nobody answers on: every lookup fails, at once.
    let t = TestApp::with_resolver(TableResolver::failing()).await;
    let app = t.app.clone();

    // No receiving account: nothing to check the domains against.
    let body = get_ok(&app, "/api/reporting-setup").await;
    assert_eq!(body["addresses"], json!([]));
    assert_eq!(body["default_address"], "");
    assert_eq!(body["domains"], json!([]));
    assert_eq!(body["authorizations"], json!([]));

    let (status, body) = post_json(
        &app,
        "/api/accounts",
        report_account("dmarc@Reports.test", &[]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    add_domain(&app, "example.com").await;

    let body = get_ok(&app, "/api/reporting-setup?refresh=true").await;
    assert_eq!(body["default_address"], "dmarc@Reports.test");
    assert_eq!(body["authorizations"][0]["domain"], "reports.test");
    assert_eq!(body["authorizations"][0]["state"], "check_failed");
    let row = &body["domains"][0];
    assert_eq!(row["domain"], "example.com");
    assert_eq!(row["state"], "check_failed");
    assert_eq!(row["status"], "neutral");
    assert_eq!(row["set_up"], false);
    assert_eq!(row["missing_authorizations"], json!([]));

    let (status, _) = get(&app, "/api/reporting-setup?domain_id=999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// With the records published, the reporting setup reads them: a domain
/// whose `rua=` names the account's address and whose reports its domain
/// accepts is set up and waits for its first report; one with no record
/// (and no authorization record) says so, and with no wildcard the address
/// domain says that too.
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
    let (status, body) = post_json(
        &app,
        "/api/accounts",
        report_account("dmarc@reports.test", &[]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    for domain in ["example.com", "example.net"] {
        add_domain(&app, domain).await;
    }

    let body = get_ok(&app, "/api/reporting-setup?refresh=true").await;
    assert_eq!(body["authorizations"][0]["state"], "no_wildcard", "{body}");
    let set_up = setup_row(&body, "example.com");
    assert_eq!(set_up["state"], "waiting_for_first_report", "{set_up}");
    assert_eq!(set_up["set_up"], true);
    assert_eq!(set_up["named_addresses"], json!(["dmarc@reports.test"]));
    assert_eq!(set_up["missing_authorizations"], json!([]));
    let missing = setup_row(&body, "example.net");
    assert_eq!(missing["state"], "no_record", "{missing}");
    assert_eq!(missing["set_up"], false);
    assert_eq!(missing["missing_authorizations"], json!(["reports.test"]));
    // Worst first: the domain that needs a DNS change leads.
    assert_eq!(body["domains"][0]["domain"], "example.net");
}

/// One domain's reporting setup (its page's waiting card) reads the DMARC
/// record from the stored DNS check, the one its header and scorecard show,
/// not a lookup of its own; `refresh` re-checks and stores it, so the page's
/// other answers then agree.
#[tokio::test]
async fn one_domains_reporting_setup_reads_the_stored_dns_check() {
    let published = "v=DMARC1; p=reject; rua=mailto:dmarc@reports.test";
    let resolver = TableResolver::new()
        .with_txt("_dmarc.example.com", &[published])
        .with_txt("example.com._report._dmarc.reports.test", &["v=DMARC1"]);
    let t = TestApp::with_resolver(resolver).await;
    let app = t.app.clone();
    let (status, body) = post_json(
        &app,
        "/api/accounts",
        report_account("dmarc@reports.test", &[]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = add_domain(&app, "example.com").await;
    // The stored check, fresh, still has the old record.
    let stored = "v=DMARC1; p=none; rua=mailto:dmarc@reports.test";
    common::store_dns(
        &t.pool,
        "example.com",
        stored,
        "v=spf1 -all",
        dmarc_store::unix_now(),
    )
    .await;

    let one = format!("/api/reporting-setup?domain_id={id}");
    let row = get_ok(&app, &one).await["domains"][0].clone();
    assert_eq!(row["dmarc"]["records"], json!([stored]), "{row}");
    assert_eq!(row["dmarc"]["parsed"]["p"], "none");
    let dns = get_ok(&app, "/api/dns/check?domain=example.com").await;
    assert_eq!(dns["dmarc"]["records"], json!([stored]));

    // Re-check: the live record, stored for everyone.
    let row = get_ok(&app, &format!("{one}&refresh=true")).await["domains"][0].clone();
    assert_eq!(row["dmarc"]["records"], json!([published]), "{row}");
    assert_eq!(row["state"], "waiting_for_first_report", "{row}");
    let dns = get_ok(&app, "/api/dns/check?domain=example.com").await;
    assert_eq!(dns["cached"], true);
    assert_eq!(dns["dmarc"]["records"], json!([published]));
    let overview = get_ok(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(overview["policy"]["p"], "reject", "{overview}");
}

/// Two receiving accounts, the second also receiving the reports forwarded
/// from `dmarc@example.org` and `dmarc@client.test`: a record naming any of
/// the addresses is
/// judged by that address, each address domain's authorization is its own,
/// and new records (in the list and for one domain, as its page asks) name
/// the first receiving account's address in list order.
#[tokio::test]
async fn reporting_setup_with_two_accounts() {
    let resolver = TableResolver::new()
        .with_txt(
            "_dmarc.example.com",
            &["v=DMARC1; p=none; rua=mailto:dmarc@reports.test"],
        )
        .with_txt("example.com._report._dmarc.reports.test", &["v=DMARC1"])
        .with_txt(
            "_dmarc.example.org",
            &["v=DMARC1; p=none; rua=mailto:DMARC@example.org"],
        )
        .with_txt(
            "_dmarc.shop.example",
            &["v=DMARC1; p=none; rua=mailto:reports@other.test,mailto:x@vendor.test"],
        );
    let t = TestApp::with_resolver(resolver).await;
    let app = t.app.clone();
    for account in [
        report_account("dmarc@reports.test", &[]),
        report_account(
            "reports@other.test",
            &["dmarc@example.org", "dmarc@client.test"],
        ),
    ] {
        let (status, body) = post_json(&app, "/api/accounts", account).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let mut ids = std::collections::HashMap::new();
    for domain in ["example.com", "example.org", "shop.example", "example.net"] {
        ids.insert(domain, add_domain(&app, domain).await);
    }

    let body = get_ok(&app, "/api/reporting-setup?refresh=true").await;
    assert_eq!(body["default_address"], "dmarc@reports.test");
    assert_eq!(
        body["addresses"],
        json!([
            { "account_id": 1, "name": "dmarc@reports.test", "address": "dmarc@reports.test", "forwarded_from": [] },
            { "account_id": 2, "name": "reports@other.test", "address": "reports@other.test", "forwarded_from": ["dmarc@example.org", "dmarc@client.test"] }
        ])
    );
    let row = setup_row(&body, "example.com");
    assert_eq!(row["state"], "waiting_for_first_report", "{row}");
    // The forwarded-from address, on the domain itself: nothing to authorize.
    let row = setup_row(&body, "example.org");
    assert_eq!(row["state"], "waiting_for_first_report", "{row}");
    assert_eq!(row["named_addresses"], json!(["DMARC@example.org"]));
    // Account 2's address, which its domain does not accept yet.
    let row = setup_row(&body, "shop.example");
    assert_eq!(row["state"], "not_authorized", "{row}");
    assert_eq!(row["missing_authorizations"], json!(["other.test"]));
    assert_eq!(row["other_addresses"], json!(["x@vendor.test"]));
    assert_eq!(row["refused"][0]["host"], "vendor.test");
    // No record: the first account's address, and its domain's permission.
    let row = setup_row(&body, "example.net");
    assert_eq!(row["state"], "no_record", "{row}");
    assert_eq!(
        row["record_to_publish"],
        "v=DMARC1; p=none; rua=mailto:dmarc@reports.test"
    );
    assert_eq!(row["missing_authorizations"], json!(["reports.test"]));

    let auth: Vec<(String, String, Value)> = body["authorizations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["domain"].as_str().unwrap().to_owned(),
                a["state"].as_str().unwrap().to_owned(),
                a["addresses"].clone(),
            )
        })
        .collect();
    assert_eq!(
        auth,
        [
            (
                "reports.test".to_owned(),
                "no_wildcard".to_owned(),
                json!(["dmarc@reports.test"])
            ),
            (
                "other.test".to_owned(),
                "no_wildcard".to_owned(),
                json!(["reports@other.test"])
            ),
            (
                "example.org".to_owned(),
                "not_needed".to_owned(),
                json!(["dmarc@example.org"])
            ),
            (
                "client.test".to_owned(),
                "not_needed".to_owned(),
                json!(["dmarc@client.test"])
            ),
        ]
    );

    // The domain page asks for its one domain: the same record to publish.
    let one = format!("/api/reporting-setup?domain_id={}", ids["example.net"]);
    let page = get_ok(&app, &one).await;
    assert_eq!(
        page["domains"][0]["record_to_publish"],
        "v=DMARC1; p=none; rua=mailto:dmarc@reports.test"
    );
    assert_eq!(page["authorizations"], json!([]));

    // Account 1 stops receiving: account 2's address is the default.
    let mut first = report_account("dmarc@reports.test", &[]);
    first["login"]["password"] = json!("");
    first["receiving"]["enabled"] = json!(false);
    let (status, answer) = put_json(&app, "/api/accounts/1", first).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let body = get_ok(&app, "/api/reporting-setup").await;
    assert_eq!(body["default_address"], "reports@other.test");
    assert_eq!(
        setup_row(&body, "example.net")["record_to_publish"],
        "v=DMARC1; p=none; rua=mailto:reports@other.test"
    );
    // example.com still names account 1's address, no longer a known one.
    assert_eq!(setup_row(&body, "example.com")["state"], "address_missing");
    let page = get_ok(&app, &one).await;
    assert_eq!(
        page["domains"][0]["record_to_publish"],
        "v=DMARC1; p=none; rua=mailto:reports@other.test"
    );
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

/// Saving the poll interval or the same account again must not interrupt a
/// running import; only a different mailbox pass does. Passwords are saved verbatim.
#[tokio::test]
async fn delivery_settings_do_not_interrupt_the_import() {
    let t = TestApp::new().await;
    let (app, pool) = (t.app.clone(), t.pool.clone());
    let worker = t.state.mailboxes.clone();

    let mut account = imap_account(
        "imap.example.com",
        993,
        "reports@example.com",
        Some("  spaced secret  "),
    );
    account["receiving"]["folder"] = json!("INBOX");
    let (status, body) = post_json(&app, "/api/accounts", account.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let after_first_save = worker.settings_generation();
    assert_eq!(after_first_save, 1);
    assert_eq!(
        stored_password(&pool).await,
        "  spaced secret  ",
        "passwords are not trimmed"
    );

    // The same account again (password omitted: the stored one is kept),
    // then the poll interval.
    let mut same = account.clone();
    same["login"]["password"] = json!("");
    let (status, body) = put_json(&app, "/api/accounts/1", same).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = put_json(
        &app,
        "/api/settings/receiving",
        json!({ "poll_interval_secs": 600 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(worker.settings_generation(), after_first_save);
    assert_eq!(stored_password(&pool).await, "  spaced secret  ");

    // A different folder is a different mailbox pass.
    let mut other = account;
    other["receiving"]["folder"] = json!("DMARC");
    other["login"]["password"] = json!("");
    let (status, _) = put_json(&app, "/api/accounts/1", other).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(worker.settings_generation(), after_first_save + 1);
    assert_eq!(stored_password(&pool).await, "  spaced secret  ");
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

/// `GET /api/settings` reflects each write; every account write and every
/// group's PUT answers it.
#[tokio::test]
async fn get_settings_reflects_each_group_write() {
    let t = TestApp::new().await;
    let app = t.app.clone();
    let mut account = imap_account("imap.example.com", 993, "u@example.com", Some("pw"));
    account["receiving"]["folder"] = json!("DMARC");
    account["smtp"] = json!({ "host": "smtp.example.com", "port": 465, "security": "tls" });
    let (status, answer) = post_json(&app, "/api/accounts", account).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer, get(&app, "/api/settings").await.1, "accounts");
    let writes = [
        ("receiving", json!({ "poll_interval_secs": 120 })),
        (
            "sending",
            json!({ "enabled": true, "account_id": 1, "sender": "", "reply_to": "", "test_recipient": "me@example.org" }),
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
    let stored = &body["accounts"][0];
    assert_eq!(stored["imap"]["host"], "imap.example.com");
    assert_eq!(stored["receiving"]["folder"], "DMARC");
    assert_eq!(stored["login"]["has_password"], true);
    assert_eq!(stored["ready_to_receive"], true);
    assert_eq!(body["receiving"]["poll_interval_secs"], 120);
    assert_eq!(body["sending"]["enabled"], true);
    assert_eq!(body["sending"]["account_id"], 1);
    assert_eq!(
        body["sending"]["effective_test_recipient"],
        "me@example.org"
    );
    assert!(
        body.get("reporting").is_none(),
        "the reporting group is gone"
    );
    assert_eq!(body["monitoring"]["report_freshness_warning_hours"], 48);
    assert_eq!(body["retention"]["report_retention_months"], 6);
    assert_eq!(body["dns"]["nameservers"], json!(["9.9.9.9"]));
    assert_eq!(body["onboarding"]["state"], "completed");
}

/// The clean-up applies the stored retention: with one month, a report that
/// ended 40 days ago goes and yesterday's stays, and so do alert events; the
/// reverse DNS of an IP only the deleted report named goes too.
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
    for (id, days_ago, ip) in [("old", 41, "192.0.2.9"), ("recent", 2, "192.0.2.1")] {
        let r = report(
            id,
            "example.com",
            now - days_ago * 86_400,
            vec![rec(ip, 5, "pass", "pass")],
        );
        dmarc_store::ingest_report(&t.pool, &r).await.unwrap();
        dmarc_store::upsert_identity_snapshot(&t.pool, ip, now, Some("mail.example.com"), None)
            .await
            .unwrap();
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
        dmarc_store::save_alert_evaluations(&t.pool, now - days_ago * 86_400, &run, None)
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
    // The reverse DNS of the old report's sender went with it.
    let snapshots: Vec<String> = sqlx::query_scalar("SELECT ip FROM identity_snapshots")
        .fetch_all(&t.pool)
        .await
        .unwrap();
    assert_eq!(snapshots, vec!["192.0.2.1"]);
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

/// The per-setting endpoints that came before the groups are gone, and so
/// are the 0.3 mailbox endpoints the accounts replaced.
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
        "/api/settings/imap/connect",
    ] {
        let (status, _) = post_json(&t.app, path, json!({})).await;
        assert!(status.is_client_error(), "POST {path}: {status}");
    }
    let mailbox = json!({
        "host": "imap.example.com", "port": 993, "username": "u", "password": "p",
        "security": "tls", "folder": "", "poll_interval_secs": 300,
    });
    let (status, _) = put_json(&t.app, "/api/settings/mailbox", mailbox).await;
    assert!(
        status.is_client_error(),
        "PUT /api/settings/mailbox: {status}"
    );
    assert!(dmarc_store::list_accounts(&t.pool)
        .await
        .unwrap()
        .is_empty());
}

/// The version check is on by default and shows nothing before its first
/// answer; switched on again, it wakes the job and meta names a newer
/// version the job stored; switched off, it forgets that answer.
#[tokio::test]
async fn the_version_check_is_on_by_default_and_forgets_when_off() {
    let t = TestApp::new().await;
    let app = t.app.clone();

    let (_, body) = get(&app, "/api/settings").await;
    assert_eq!(body["updates"], json!({ "check": true }));
    assert_eq!(get_ok(&app, "/api/meta").await["available"], Value::Null);
    let (status, body) = put_json(&app, "/api/settings/updates", json!({ "check": false })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updates"], json!({ "check": false }));

    // An answer stored while off is not shown.
    let newer = dmarc_store::UpdateCheck {
        attempted_at: 1,
        seen: Some(dmarc_store::SeenVersion {
            version: "999.0.0".into(),
            url: "https://marcan.kaitiaki.tech/releases".into(),
            checked_at: 1,
        }),
    };
    dmarc_store::set_update_check(&t.pool, &newer)
        .await
        .unwrap();
    assert_eq!(get_ok(&app, "/api/meta").await["available"], Value::Null);

    let (status, body) = put_json(&app, "/api/settings/updates", json!({ "check": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updates"]["check"], true);
    assert_eq!(
        get_ok(&app, "/api/meta").await["available"],
        json!({ "version": "999.0.0", "url": "https://marcan.kaitiaki.tech/releases" })
    );
    // Switching it on woke the job.
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        t.state.update_check_wake.notified(),
    )
    .await
    .expect("woken");

    // The running version itself is not named.
    let mut current = newer.clone();
    if let Some(seen) = current.seen.as_mut() {
        seen.version = env!("CARGO_PKG_VERSION").into();
    }
    dmarc_store::set_update_check(&t.pool, &current)
        .await
        .unwrap();
    assert_eq!(get_ok(&app, "/api/meta").await["available"], Value::Null);

    let (status, body) = put_json(&app, "/api/settings/updates", json!({ "check": false })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["updates"], json!({ "check": false }));
    assert_eq!(dmarc_store::get_update_check(&t.pool).await.unwrap(), None);

    // The whole group, as every PUT.
    let (status, _) = put_json(&app, "/api/settings/updates", json!({})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The mailbox's (account 1's) stored password, which no key seals here.
async fn stored_password(pool: &sqlx::SqlitePool) -> String {
    let row = dmarc_store::get_account_secret(pool, 1, dmarc_store::SecretKind::Password)
        .await
        .unwrap()
        .expect("a stored password");
    String::from_utf8(row.value).unwrap()
}
