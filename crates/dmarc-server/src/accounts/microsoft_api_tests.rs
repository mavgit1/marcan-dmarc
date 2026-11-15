//! Microsoft logins through `/api/accounts`: the test (a fresh token, then
//! the mailbox read over the fake Graph, through the real token source; no
//! sending line, nothing sent), the save with a draft certificate and
//! without servers, an unreachable token endpoint said in words, and what
//! does and does not read the mailbox from the start.

use super::certificate::tests::{other_certificate, test_certificate};
use super::connection::ImapProber;
use super::oauth::fake::{token, FakeTokenEndpoint, Registration};
use super::oauth::tests::{registration, registration_with, CLIENT, TENANT};
use super::oauth::{TokenErrorKind, TokenSource};
use crate::mail::MemorySender;
use crate::wording::microsoft::{token_error_sentence, TOKEN_ISSUED};
use crate::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_mailbox::graph_fake::FakeGraph;
use dmarc_mailbox::{ConnectError, GraphApi, ImapConfig, MailboxProbe};
use dmarc_store::{get_account, get_account_secret, init, LoginKind, SecretKind};
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// Records what an IMAP login was tried with: a Microsoft login tries none.
#[derive(Default)]
struct Prober(Mutex<Vec<ImapConfig>>);

impl ImapProber for Prober {
    fn probe<'a>(
        &'a self,
        cfg: &'a ImapConfig,
    ) -> BoxFuture<'a, Result<MailboxProbe, ConnectError>> {
        self.0.lock().unwrap().push(cfg.clone());
        Box::pin(std::future::ready(Err(ConnectError {
            stage: dmarc_mailbox::ConnectStage::Other,
            message: "no IMAP here".into(),
            detail: String::new(),
            docs: None,
        })))
    }
}

const MAILBOX: &str = "dmarc@example.com";

/// A fake Graph whose mailbox has a folder DMARC with one report.
async fn graph() -> FakeGraph {
    let graph = FakeGraph::start(MAILBOX).await;
    let dmarc = graph.add_folder(None, "DMARC");
    graph.deliver_reports(&dmarc, &[1]);
    graph
}

async fn call(app: &axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The Microsoft 365 account on screen with the certificate `draft_id`
/// (`None`: the stored one), as the form sends it: no servers.
fn account(draft_id: Option<&str>) -> Value {
    json!({
        "name": "Reports",
        "provider": "microsoft365",
        "address": "dmarc@example.com",
        "login": { "kind": "microsoft", "tenant_id": TENANT, "client_id": CLIENT, "draft_id": draft_id },
        "imap": null,
        "smtp": null,
        "receiving": { "enabled": true, "folder": "DMARC", "forwarded_from": [] },
    })
}

#[tokio::test]
async fn a_microsoft_account_is_tested_and_saved_through_the_api() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = graph().await;
    graph.accept_tokens(&[&token(1), &token(2)]);
    let prober = Arc::new(Prober::default());
    let sender = Arc::new(MemorySender::default());
    let state = AppState::new(pool.clone())
        .with_token_source(endpoint.source())
        .with_graph_api(GraphApi::new(graph.base.clone()))
        .with_imap_prober(prober.clone())
        .with_mail_sender(sender.clone());
    let app = crate::router_with_state(state.clone());

    // The test of a new form's certificate: a token, the mailbox over
    // Graph; no sending line even when SMTP is asked for, and nothing sent.
    let draft_id = state.accounts.drafts().insert(test_certificate().clone());
    let (status, answer) = call(
        &app,
        "POST",
        "/api/accounts/test",
        json!({ "account": account(Some(&draft_id)), "smtp": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["token"]["ok"], true, "{answer}");
    assert_eq!(answer["token"]["message"], TOKEN_ISSUED);
    assert_eq!(answer["mailbox"]["ok"], true, "{answer}");
    assert_eq!(
        answer["mailbox"]["message"],
        "Read dmarc@example.com over Microsoft Graph."
    );
    assert_eq!(answer["mailbox"]["selected"]["name"], "DMARC");
    assert_eq!(answer["mailbox"]["selected"]["exists"], true);
    assert_eq!(answer["mailbox"]["selected"]["messages"], 1);
    let folders: Vec<&str> = answer["mailbox"]["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(folders, ["INBOX", "DMARC"]);
    assert!(prober.0.lock().unwrap().is_empty(), "no IMAP login");
    assert_eq!(answer["smtp"], Value::Null, "{answer}");
    assert!(sender.logins().is_empty() && sender.sent().is_empty());
    assert!(graph.sent().is_empty(), "the test sends nothing");
    assert_eq!(endpoint.grants(), 1, "one fresh token");
    assert_eq!(answer["mailbox"]["docs"], Value::Null);

    // Without the read role: the sentence leads to the guide's Exchange
    // Online step.
    graph.forbid(true);
    let (status, answer) = call(
        &app,
        "POST",
        "/api/accounts/test",
        json!({ "account": account(Some(&draft_id)), "smtp": false }),
    )
    .await;
    graph.forbid(false);
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["mailbox"]["ok"], false, "{answer}");
    assert_eq!(
        answer["mailbox"]["docs"], "accounts#exchange-online",
        "{answer}"
    );

    // Not receiving, the token alone is tested (a mailbox that only sends
    // may not be read).
    let mut sends_only = account(Some(&draft_id));
    sends_only["receiving"]["enabled"] = json!(false);
    let (status, answer) = call(
        &app,
        "POST",
        "/api/accounts/test",
        json!({ "account": sends_only, "smtp": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["token"]["ok"], true, "{answer}");
    assert_eq!(
        (answer["mailbox"].clone(), answer["smtp"].clone()),
        (Value::Null, Value::Null)
    );

    // Saved with nothing but the draft certificate (servers sent anyway
    // are not stored): a Microsoft login without servers, its key stored,
    // the draft taken.
    let mut with_servers = account(Some(&draft_id));
    with_servers["imap"] = json!({ "host": "imap.example.com", "port": 993, "security": "tls" });
    with_servers["smtp"] =
        json!({ "host": "smtp.example.com", "port": 587, "security": "starttls" });
    let (status, answer) = call(&app, "POST", "/api/accounts", with_servers).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let view = &answer["accounts"][0];
    assert_eq!(view["login"]["kind"], "microsoft");
    let certificate = &view["login"]["certificate"];
    assert_eq!(certificate["thumbprint"], test_certificate().thumbprint);
    assert_eq!(certificate["expires_at"], test_certificate().expires_at);
    assert_eq!(
        (view["imap"].clone(), view["smtp"].clone()),
        (Value::Null, Value::Null)
    );
    assert_eq!(view["ready_to_receive"], true);
    assert_eq!(view["sends"], true);
    assert_eq!(view["ready_to_send"], true);
    let id = view["id"].as_i64().unwrap();
    let stored = get_account(&pool, id).await.unwrap().unwrap();
    assert_eq!(stored.fields.login_kind, LoginKind::Microsoft);
    assert_eq!(
        (
            stored.fields.imap_host.as_str(),
            stored.fields.smtp_host.as_str()
        ),
        ("", "")
    );
    assert_eq!(stored.fields.smtp_port, None);
    assert!(get_account_secret(&pool, id, SecretKind::PrivateKey)
        .await
        .unwrap()
        .is_some());
    assert!(state.accounts.drafts().get(&draft_id).is_none(), "taken");

    // A certificate generated by the API is held as a draft.
    let (status, draft) = call(
        &app,
        "POST",
        "/api/accounts/certificate",
        json!({ "subject": "dmarc@example.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    assert!(state
        .accounts
        .drafts()
        .get(draft["draft_id"].as_str().unwrap())
        .is_some());

    // Kept as it is on the next save; a gone draft is said in words.
    let (status, _) = call(&app, "PUT", &format!("/api/accounts/{id}"), account(None)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = call(
        &app,
        "PUT",
        &format!("/api/accounts/{id}"),
        account(Some(&draft_id)),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(answer["error"], crate::wording::microsoft::DRAFT_GONE);
    // A test of the stored certificate gets a token with an assertion.
    let (_, answer) = call(
        &app,
        "POST",
        "/api/accounts/test",
        json!({ "id": id, "account": account(None), "smtp": false }),
    )
    .await;
    assert_eq!(answer["token"]["ok"], true, "{answer}");
    assert!(endpoint
        .requests()
        .last()
        .unwrap()
        .contains_key("client_assertion"));
}

/// The token endpoint unreachable: the test says so in words and stops.
#[tokio::test]
async fn an_unreachable_token_endpoint_is_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed =
        reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    drop(listener);
    let prober = Arc::new(Prober::default());
    let state = AppState::new(pool)
        .with_token_source(TokenSource::new(reqwest::Client::new(), closed))
        .with_imap_prober(prober.clone());
    let draft_id = state.accounts.drafts().insert(test_certificate().clone());
    let app = crate::router_with_state(state);
    let (status, answer) = call(
        &app,
        "POST",
        "/api/accounts/test",
        json!({ "account": account(Some(&draft_id)), "smtp": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["token"]["ok"], false);
    assert_eq!(
        answer["token"]["message"],
        token_error_sentence(TokenErrorKind::Unreachable)
    );
    // Nothing to change in Entra: no docs section.
    assert_eq!(answer["token"]["docs"], Value::Null);
    assert_eq!(
        (answer["mailbox"].clone(), answer["smtp"].clone()),
        (Value::Null, Value::Null)
    );
    assert!(
        prober.0.lock().unwrap().is_empty(),
        "nothing tried without a token"
    );
}

/// A Microsoft login reads its address in its tenant: another address is
/// another mailbox, so the save starts its reading over (cursor and
/// skipped messages forgotten before the next pass), while a rename, a new
/// certificate or another app registration keeps where it was.
#[tokio::test]
async fn another_address_of_a_microsoft_login_is_read_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    const OTHER_CLIENT: &str = "99999999-bbbb-cccc-dddd-eeeeeeeeeeee";
    let mut both = registration();
    both.also = registration_with(other_certificate()).certificate;
    let other_app = Registration {
        client: OTHER_CLIENT.into(),
        ..registration_with(other_certificate())
    };
    let endpoint = FakeTokenEndpoint::start(vec![both, other_app]).await;
    let graph = graph().await;
    let state = AppState::new(pool.clone())
        .with_token_source(endpoint.source())
        .with_graph_api(GraphApi::new(graph.base.clone()));
    let app = crate::router_with_state(state.clone());
    let draft_id = state.accounts.drafts().insert(test_certificate().clone());
    let (status, answer) = call(&app, "POST", "/api/accounts", account(Some(&draft_id))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let id = answer["accounts"][0]["id"].as_i64().unwrap();
    let path = format!("/api/accounts/{id}");
    let one_pass = || {
        let state = state.clone();
        async move {
            match state.mailboxes.sync_graph(&state.mailbox_backend()).await {
                Ok(dmarc_mailbox::SyncOutcome::Fetched(summary)) => summary,
                other => panic!("{other:?}"),
            }
        }
    };
    assert_eq!(one_pass().await.reports_ingested, 1);
    let (generation, _) = dmarc_store::get_cursor(&pool, id, "DMARC")
        .await
        .unwrap()
        .expect("a cursor");
    dmarc_store::quarantine_receive_message(
        &pool,
        id,
        "DMARC",
        &generation,
        "AAMk=",
        "broken",
        1,
        0,
    )
    .await
    .unwrap();
    let quarantined = || dmarc_store::count_receive_quarantine(&pool, Some(id), None);

    // Renamed, the address in capitals, a new certificate, another app
    // registration of the tenant: the same mailbox, read on from its cursor.
    let mut renamed = account(None);
    renamed["name"] = json!("DMARC reports");
    renamed["address"] = json!("DMARC@example.com");
    let renewed = state.accounts.drafts().insert(other_certificate().clone());
    let mut new_certificate = account(Some(&renewed));
    new_certificate["address"] = json!("DMARC@example.com");
    let mut other_client = account(None);
    other_client["login"]["client_id"] = json!(OTHER_CLIENT);
    for (step, body) in [
        ("renamed", renamed),
        ("new certificate", new_certificate),
        ("other client", other_client),
    ] {
        let (status, answer) = call(&app, "PUT", &path, body).await;
        assert_eq!(status, StatusCode::OK, "{step}: {answer}");
        assert_eq!(one_pass().await.messages_seen, 0, "{step}");
        assert_eq!(graph.downloads().len(), 1, "{step}: nothing read again");
        assert_eq!(quarantined().await.unwrap(), 1, "{step}");
    }

    // Another address: read from its first message, nothing skipped.
    let mut moved = account(None);
    moved["address"] = json!("reports@example.com");
    let (status, answer) = call(&app, "PUT", &path, moved).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        dmarc_store::get_cursor(&pool, id, "DMARC").await.unwrap(),
        None
    );
    assert_eq!(quarantined().await.unwrap(), 0);
}
