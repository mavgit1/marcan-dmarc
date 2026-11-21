//! Microsoft logins end to end: an account against the fake token endpoint
//! and the fake Graph (expecting the token), reading and sending with its
//! certificate; Graph's refusals in the pass record; no servers needed;
//! what a save checks and keeps; the connection test's token step.

use super::certificate::tests::{other_certificate, test_certificate};
use super::microsoft::{MicrosoftLoginRequest, StoredRequest};
use super::oauth::fake::{token, FakeTokenEndpoint, Registration};
use super::oauth::tests::{registration, registration_with, CLIENT, TENANT};
use super::oauth::{TokenErrorKind, GRAPH_SCOPE};
use super::secrets::tests::key;
use super::*;
use crate::mail::smtp::tests::message;
use crate::wording::microsoft::{
    graph_sentence, token_error_sentence, CLIENT_NOT_GUID, DRAFT_GONE, ENTER_CLIENT, ENTER_MAILBOX,
    ENTER_TENANT, GENERATE_CERTIFICATE, TENANT_NOT_GUID, TOKEN_ISSUED,
};
use crate::AppState;
use dmarc_mailbox::graph_fake::FakeGraph;
use dmarc_mailbox::{GraphApi, GraphProblem, SyncOutcome};
use dmarc_store::{init, insert_account, update_account, AccountProvider, MicrosoftLogin};

const POLL: Duration = Duration::from_secs(300);

pub(crate) const MAILBOX: &str = "dmarc@example.com";

/// A fake Graph with one report in the mailbox's inbox, accepting only
/// `tokens`.
async fn graph_with_a_report(tokens: &[&str]) -> FakeGraph {
    let graph = FakeGraph::start(MAILBOX).await;
    graph.deliver_reports(&graph.inbox(), &[1]);
    graph.accept_tokens(tokens);
    graph
}

pub(crate) fn graph_api(graph: &FakeGraph) -> GraphApi {
    GraphApi::new(graph.base.clone())
}

/// One pass of the worker, outcome as it ended.
async fn one_pass(state: &AppState) -> Result<SyncOutcome, String> {
    state.mailboxes.sync_graph(&state.mailbox_backend()).await
}

pub(crate) async fn pool() -> (tempfile::TempDir, SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    (dir, pool)
}

/// A Microsoft 365 account that receives and sends as dmarc@example.com,
/// proving itself with the test certificate; no servers, as a save stores
/// it.
pub(crate) fn microsoft_account() -> AccountFields {
    let cert = test_certificate();
    AccountFields {
        name: "Reports".into(),
        provider: AccountProvider::Microsoft365,
        address: "dmarc@example.com".into(),
        login_kind: LoginKind::Microsoft,
        microsoft: MicrosoftLogin {
            tenant_id: Some(TENANT.into()),
            client_id: Some(CLIENT.into()),
            cert_der: Some(cert.cert_der.clone()),
            cert_thumbprint: Some(cert.thumbprint.clone()),
            cert_expires_at: Some(cert.expires_at),
        },
        ..AccountFields::default()
    }
}

/// The test certificate's private key, stored plain.
pub(crate) fn private_key() -> SecretRow {
    SecretRow::plain(SecretKind::PrivateKey, &test_certificate().private_key_pem)
}

pub(crate) async fn add(pool: &SqlitePool, fields: &AccountFields, secret: SecretRow) -> i64 {
    insert_account(pool, None, fields, &[secret], 0)
        .await
        .unwrap()
}

pub(crate) fn sending_through(id: i64) -> AppSettings {
    let mut settings = AppSettings::default();
    settings.sending.enabled = true;
    settings.sending.account_id = Some(id);
    settings
}

/// A pass of the real worker reads a Microsoft account's mailbox over
/// Graph with the token the endpoint issued for its certificate; the fake
/// Graph accepts that token only.
#[tokio::test]
async fn a_microsoft_account_receives_over_graph_with_its_token() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = graph_with_a_report(&[&token(1)]).await;
    let state = AppState::new(pool.clone())
        .with_token_source(endpoint.source())
        .with_graph_api(graph_api(&graph));
    let id = add(&pool, &microsoft_account(), private_key()).await;
    let snapshot = state.accounts.snapshot().await.unwrap();
    assert!(snapshot.ready_to_receive(snapshot.get(id).unwrap()));
    assert!(snapshot.ready_to_send(snapshot.get(id).unwrap()));

    let outcome = one_pass(&state).await;
    let Ok(SyncOutcome::Fetched(summary)) = outcome else {
        panic!("the pass failed: {outcome:?}");
    };
    assert_eq!(summary.reports_ingested, 1);
    assert_eq!(endpoint.grants(), 1);
    assert_eq!(
        endpoint.requests()[0]["scope"],
        "https://graph.microsoft.com/.default"
    );
    assert!(graph
        .requests()
        .iter()
        .all(|r| r.target.starts_with(&format!("/v1.0/users/{MAILBOX}/"))));
    assert_eq!(graph.downloads().len(), 1);

    // The configuration reads the mailbox address's folder over Graph; the
    // token is cached, so the next pass asks for none.
    let cfg = state
        .accounts
        .mailbox_config(id, POLL)
        .await
        .unwrap()
        .unwrap();
    let graph_cfg = cfg.as_graph().expect("a Graph configuration");
    assert_eq!(graph_cfg.address, MAILBOX);
    assert_eq!(graph_cfg.folder, "INBOX");
    assert_eq!(graph_cfg.api, graph_api(&graph));
    assert_eq!(cfg.host(), "127.0.0.1");
    assert_eq!(endpoint.grants(), 1);
    let again = one_pass(&state).await;
    assert!(matches!(again, Ok(SyncOutcome::Fetched(_))), "{again:?}");
    assert_eq!(endpoint.grants(), 1);
    assert_eq!(graph.downloads().len(), 1, "nothing new, nothing read");
}

/// Graph refusing the mailbox fails the pass with the sentence, recorded
/// for the account with Graph's code apart (the account page's Details);
/// the token is forgotten (the pass never
/// reached the mailbox), so the next pass asks for a new one.
#[tokio::test]
async fn graph_refusing_the_mailbox_fails_the_pass_in_words() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = graph_with_a_report(&[]).await;
    graph.forbid(true);
    let state = AppState::new(pool.clone())
        .with_token_source(endpoint.source())
        .with_graph_api(graph_api(&graph));
    let id = add(&pool, &microsoft_account(), private_key()).await;
    assert!(one_pass(&state).await.is_err());
    let status = receive_pass_status(&pool, id).await.unwrap();
    let (error, _) = status.last_error.expect("the failed pass is recorded");
    let sentence = graph_sentence(&GraphProblem::NoAccess { mailbox: MAILBOX });
    assert_eq!(error, sentence);
    let detail = status.last_error_detail.expect("Graph's answer is kept");
    assert!(detail.starts_with("ErrorAccessDenied"), "{detail}");
    assert!(!detail.contains(&token(1)), "{detail}");
    assert_eq!(status.last_check_at, None);
    assert_eq!(state.mailboxes.failures(&id.to_string()), 1);

    graph.forbid(false);
    let outcome = one_pass(&state).await;
    assert!(
        matches!(outcome, Ok(SyncOutcome::Fetched(_))),
        "{outcome:?}"
    );
    assert_eq!(
        endpoint.grants(),
        2,
        "the refused pass's token was forgotten"
    );
    assert_eq!(state.mailboxes.failures(&id.to_string()), 0);
}

/// A send through a Microsoft account goes to Graph's `sendMail` as the
/// mailbox, with the token its passes read with (fetched at the first
/// send, then cached); a refused certificate stops it in words, nothing
/// sent.
#[tokio::test]
async fn a_microsoft_account_sends_over_graph_with_its_token() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = graph_with_a_report(&[&token(1)]).await;
    let accounts = Accounts::new(pool.clone(), None)
        .with_token_source(Arc::new(endpoint.source()))
        .with_graph_api(graph_api(&graph));
    let id = add(&pool, &microsoft_account(), private_key()).await;
    let config = accounts.send_config(&sending_through(id)).await.unwrap();
    let mailbox = config.graph().expect("sent over Graph");
    assert_eq!(mailbox.address, MAILBOX);
    assert_eq!(mailbox.api, graph_api(&graph));
    assert_eq!(config.sender.to_string(), MAILBOX);
    assert_eq!(endpoint.grants(), 0, "no token before the send");

    for _ in 0..2 {
        crate::mail::send(&config, &message()).await.unwrap();
    }
    let sent = graph.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|m| m.user == MAILBOX));
    let text = String::from_utf8(sent[0].mime.clone()).unwrap();
    assert!(text.contains("To: owner@example.org\r\n"), "{text}");
    assert_eq!(endpoint.grants(), 1, "one token, cached");
    assert_eq!(endpoint.requests()[0]["scope"], GRAPH_SCOPE);
    // A pass reads with the same token.
    assert!(accounts.mailbox_config(id, POLL).await.unwrap().is_some());
    assert_eq!(endpoint.grants(), 1, "the sends' token reads too");

    // A certificate the endpoint refuses stops the send before Graph, in
    // words.
    let endpoint = FakeTokenEndpoint::start(vec![registration_with(other_certificate())]).await;
    let accounts = Accounts::new(pool.clone(), None)
        .with_token_source(Arc::new(endpoint.source()))
        .with_graph_api(graph_api(&graph));
    let config = accounts.send_config(&sending_through(id)).await.unwrap();
    graph.clear_log();
    let err = crate::mail::send(&config, &message()).await.unwrap_err();
    assert_eq!(
        err.message(),
        token_error_sentence(TokenErrorKind::AssertionRejected)
    );
    assert!(err.detail().contains("AADSTS700027"), "{err}");
    assert_eq!(err.reason(), "auth");
    assert!(graph.requests().is_empty(), "nothing reached Graph");
    assert_eq!(graph.sent().len(), 2);
}

/// A certificate account's passes and sends ask with a signed assertion.
#[tokio::test]
async fn a_certificate_account_gets_its_token_with_an_assertion() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let accounts =
        Accounts::new(pool.clone(), Some(key(1))).with_token_source(Arc::new(endpoint.source()));
    let fields = microsoft_account();
    let row = accounts.store(
        1,
        SecretKind::PrivateKey,
        &test_certificate().private_key_pem,
    );
    assert!(row.is_encrypted());
    let id = insert_account(&pool, Some(1), &fields, &[row], 0)
        .await
        .unwrap();
    let cfg = accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    assert!(cfg.as_graph().is_some(), "{cfg:?}");
    assert_eq!(endpoint.grants(), 1);
    assert!(endpoint.requests()[0].contains_key("client_assertion"));

    // Without the key, the certificate's key is locked: no pass, no send.
    let keyless = Accounts::new(pool.clone(), None).with_token_source(Arc::new(endpoint.source()));
    assert!(keyless.mailbox_config(id, POLL).await.unwrap().is_none());
    assert!(matches!(
        keyless.send_config(&sending_through(id)).await,
        Err(SendError::Locked { .. })
    ));
}

/// A Microsoft login has no servers: it receives while receiving is on and
/// sends either way, over Graph.
#[tokio::test]
async fn a_microsoft_login_receives_and_sends_without_servers() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let accounts = Accounts::new(pool.clone(), None).with_token_source(Arc::new(endpoint.source()));
    for receive_enabled in [true, false] {
        let fields = AccountFields {
            address: format!("{receive_enabled}@example.com"),
            receive_enabled,
            ..microsoft_account()
        };
        assert!(fields.imap_host.is_empty() && fields.smtp_host.is_empty());
        let id = add(&pool, &fields, private_key()).await;
        let snapshot = accounts.snapshot().await.unwrap();
        let account = snapshot.get(id).unwrap();
        assert!(sends_mail(&account.fields));
        assert!(reads_mailbox(&account.fields));
        assert_eq!(snapshot.ready_to_receive(account), receive_enabled);
        assert!(snapshot.ready_to_send(account));
        let config = accounts.send_config(&sending_through(id)).await.unwrap();
        let mailbox = config.graph().expect("sent over Graph");
        assert_eq!(mailbox.api.host(), "graph.microsoft.com");
        assert_eq!(mailbox.address, fields.address);
        let cfg = accounts.mailbox_config(id, POLL).await.unwrap();
        assert_eq!(
            cfg.map(|c| c.host().to_owned()),
            receive_enabled.then(|| "graph.microsoft.com".to_owned())
        );
    }
    // Without a certificate its login is incomplete: it neither receives
    // nor sends.
    let bare = AccountFields {
        address: "bare@example.com".into(),
        microsoft: MicrosoftLogin {
            cert_der: None,
            cert_thumbprint: None,
            cert_expires_at: None,
            ..microsoft_account().microsoft
        },
        ..microsoft_account()
    };
    let id = insert_account(&pool, None, &bare, &[], 0).await.unwrap();
    let snapshot = accounts.snapshot().await.unwrap();
    let account = snapshot.get(id).unwrap();
    assert!(!snapshot.has_login(account));
    assert!(!snapshot.ready_to_receive(account) && !snapshot.ready_to_send(account));
    assert!(accounts.mailbox_config(id, POLL).await.unwrap().is_none());
    assert!(matches!(
        accounts.send_config(&sending_through(id)).await,
        Err(SendError::Incomplete(why)) if why == GENERATE_CERTIFICATE
    ));
    // A password login sends only with an SMTP server, receives only with
    // an IMAP server.
    let password = AccountFields {
        login_kind: LoginKind::Password,
        ..AccountFields::default()
    };
    assert!(!sends_mail(&password) && !reads_mailbox(&password));
}

/// A refused token fails the pass with the plain sentence and the code,
/// and the pass is recorded: the account's status and the health say why
/// it was not read (no check, no success), as the card and the Import step
/// show it.
#[tokio::test]
async fn a_refused_token_fails_the_pass_in_words() {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![Registration {
        certificate: None,
        ..registration()
    }])
    .await;
    let state = AppState::new(pool.clone()).with_token_source(endpoint.source());
    let id = add(&pool, &microsoft_account(), private_key()).await;
    let outcome = one_pass(&state).await;
    assert!(outcome.is_err());
    let sentence = token_error_sentence(TokenErrorKind::AssertionRejected);
    let status = receive_pass_status(&pool, id).await.unwrap();
    let (error, _) = status.last_error.expect("the failed pass is recorded");
    assert!(error.contains(sentence), "{error}");
    assert!(error.contains("AADSTS700027"), "{error}");
    assert_eq!((status.last_check_at, status.last_success_at), (None, None));

    let app = crate::router_with_state(state.clone());
    let request = Request::get("/api/system/health")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let health: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let account = &health["accounts"][0];
    assert_eq!(account["id"], id);
    assert_eq!(account["last_error"], error.as_str());
    assert_eq!(account["state"], "error", "{health}");
    assert_eq!(health["receive_last_error"], error.as_str());
}

fn login(draft_id: Option<&str>) -> MicrosoftLoginRequest {
    MicrosoftLoginRequest {
        tenant_id: TENANT.into(),
        client_id: CLIENT.into(),
        draft_id: draft_id.map(str::to_owned),
    }
}

fn refusal(result: Result<super::microsoft::MicrosoftSave, AccountError>) -> String {
    match result {
        Err(AccountError::Refused(why)) => why,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// What a save checks: GUIDs, a mailbox address and a certificate (a
/// draft, or the stored one, whatever IDs it is used with next).
#[tokio::test]
async fn saving_a_microsoft_login_checks_and_keeps_its_certificate() {
    let (_dir, pool) = pool().await;
    let accounts = Accounts::new(pool.clone(), Some(key(1)));
    let fields = microsoft_account();
    let save = |id, fields: AccountFields, login: MicrosoftLoginRequest| {
        let accounts = &accounts;
        async move { accounts.prepare_microsoft_save(id, &fields, &login).await }
    };
    let draft = accounts.drafts().insert(test_certificate().clone());

    let mut bad = login(Some(&draft));
    bad.tenant_id = "contoso.onmicrosoft.com".into();
    assert_eq!(
        refusal(save(None, fields.clone(), bad).await),
        TENANT_NOT_GUID
    );
    let mut bad = login(Some(&draft));
    bad.client_id = "Marcan".into();
    assert_eq!(
        refusal(save(None, fields.clone(), bad).await),
        CLIENT_NOT_GUID
    );
    // An empty ID is asked for; the format is said only for something typed.
    let mut bad = login(Some(&draft));
    bad.tenant_id = " ".into();
    assert_eq!(refusal(save(None, fields.clone(), bad).await), ENTER_TENANT);
    let mut bad = login(Some(&draft));
    bad.client_id = String::new();
    assert_eq!(refusal(save(None, fields.clone(), bad).await), ENTER_CLIENT);
    let no_address = AccountFields {
        address: String::new(),
        ..fields.clone()
    };
    assert_eq!(
        refusal(save(None, no_address, login(Some(&draft))).await),
        ENTER_MAILBOX
    );
    // A new account needs a certificate; one gone from memory is said so.
    assert_eq!(
        refusal(save(None, fields.clone(), login(None)).await),
        GENERATE_CERTIFICATE
    );
    assert_eq!(
        refusal(save(None, fields.clone(), login(Some("gone"))).await),
        DRAFT_GONE
    );

    // A draft's certificate is stored (the draft taken once written), the
    // password cleared.
    let first = save(None, fields.clone(), login(Some(&draft)))
        .await
        .unwrap();
    assert_eq!(first.draft_id.as_deref(), Some(draft.as_str()));
    assert_eq!(first.microsoft, microsoft_account().microsoft);
    assert_eq!(
        first.secrets,
        [
            (SecretKind::Password, SecretChange::Clear),
            (
                SecretKind::PrivateKey,
                SecretChange::Set(test_certificate().private_key_pem.clone())
            ),
        ]
    );
    let stored = AccountFields {
        microsoft: first.microsoft.clone(),
        ..fields.clone()
    };
    let id = insert_account(&pool, Some(1), &stored, &[], 0)
        .await
        .unwrap();
    update_account(&pool, id, &stored, &accounts.seal(id, &first.secrets))
        .await
        .unwrap();
    accounts.drafts().take(&draft);
    let row = get_account_secret(&pool, id, SecretKind::PrivateKey)
        .await
        .unwrap()
        .unwrap();
    assert!(row.is_encrypted(), "sealed with the key");

    // Without a draft the stored certificate stays, with other IDs too.
    let kept = save(Some(id), stored.clone(), login(None)).await.unwrap();
    assert_eq!(kept.microsoft, first.microsoft);
    assert_eq!(
        kept.secrets[1],
        (SecretKind::PrivateKey, SecretChange::Keep)
    );
    let mut other_client = login(None);
    other_client.client_id = "99999999-bbbb-cccc-dddd-eeeeeeeeeeee".into();
    let moved = save(Some(id), stored.clone(), other_client).await.unwrap();
    assert_eq!(moved.microsoft.cert_der, first.microsoft.cert_der);
    assert_eq!(
        moved.microsoft.client_id.as_deref(),
        Some("99999999-bbbb-cccc-dddd-eeeeeeeeeeee")
    );
    assert!(matches!(
        accounts
            .stored_request(&get_account(&pool, id).await.unwrap().unwrap())
            .await,
        Ok(StoredRequest::Ready(_))
    ));

    // A new draft replaces it.
    let next = accounts.drafts().insert(other_certificate().clone());
    let renewed = save(Some(id), stored.clone(), login(Some(&next)))
        .await
        .unwrap();
    assert_eq!(
        renewed.microsoft.cert_thumbprint.as_deref(),
        Some(other_certificate().thumbprint.as_str())
    );
    assert_eq!(
        renewed.secrets[1],
        (
            SecretKind::PrivateKey,
            SecretChange::Set(other_certificate().private_key_pem.clone())
        )
    );
}

/// The test's token step: a fresh token for Graph each time, checked for
/// no permission (the access comes from Exchange role assignments).
#[tokio::test]
async fn the_token_step_asks_for_a_fresh_graph_token() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let accounts = Accounts::new(pool.clone(), None).with_token_source(Arc::new(endpoint.source()));

    // An unsaved form with a draft certificate.
    let draft = accounts.drafts().insert(test_certificate().clone());
    let request = accounts
        .microsoft_test_request(None, &login(Some(&draft)))
        .await
        .unwrap();
    let (step, issued) = accounts.test_token(&request).await;
    assert!(step.ok && issued, "{step:?}");
    assert_eq!(step.message, TOKEN_ISSUED);
    assert_eq!(endpoint.grants(), 1);
    assert_eq!(endpoint.requests()[0]["scope"], GRAPH_SCOPE);
    assert!(accounts.test_token(&request).await.0.ok);
    assert_eq!(endpoint.grants(), 2, "every test asks anew");
    assert!(accounts.drafts().get(&draft).is_some(), "kept for the save");

    // A certificate not uploaded yet is refused in words.
    let unknown = accounts.drafts().insert(other_certificate().clone());
    let wrong = accounts
        .microsoft_test_request(None, &login(Some(&unknown)))
        .await
        .unwrap();
    let (step, issued) = accounts.test_token(&wrong).await;
    assert!(!step.ok && !issued);
    assert_eq!(
        step.message,
        token_error_sentence(TokenErrorKind::AssertionRejected)
    );
    assert!(step.detail.unwrap().contains("AADSTS700027"));
    assert!(matches!(
        accounts.microsoft_test_request(None, &login(None)).await,
        Err(AccountError::Refused(why)) if why == GENERATE_CERTIFICATE
    ));
}

/// An account's token is forgotten when asked (its save or deletion, a
/// refused login): the next pass fetches a new one; a password account
/// forgets nothing.
#[tokio::test]
async fn an_accounts_token_is_forgotten_on_request() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let accounts = Accounts::new(pool.clone(), None).with_token_source(Arc::new(endpoint.source()));
    let id = add(&pool, &microsoft_account(), private_key()).await;
    accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    assert_eq!(endpoint.grants(), 1, "cached");
    accounts.forget_token_of(id).await;
    accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    assert_eq!(endpoint.grants(), 2, "fetched anew");
    accounts.forget_token_of(id + 100).await;
    accounts.mailbox_config(id, POLL).await.unwrap().unwrap();
    assert_eq!(endpoint.grants(), 2, "another id forgets nothing");
}
