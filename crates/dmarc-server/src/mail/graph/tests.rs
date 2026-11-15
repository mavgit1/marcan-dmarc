//! A Microsoft account's mail over the fake Graph, through the real token
//! source and the real (paced) network sender: the MIME Graph gets is the
//! built message byte for byte; the Sending test, a test alert and a client
//! report preview with its inline image go out that way with the message
//! SMTP would have carried; a refused token is fetched anew once,
//! throttling is waited out; each refusal is said in words with Graph's
//! code; the pacing holds; a password account still sends over SMTP.

use super::*;
use crate::accounts::microsoft_tests::{
    add, graph_api, microsoft_account, pool, private_key, sending_through, MAILBOX,
};
use crate::accounts::oauth::fake::{token, FakeTokenEndpoint};
use crate::accounts::oauth::tests::registration;
use crate::mail::send;
use crate::mail::{
    InlineImage, MailSender, NetworkSender, PacedSender, SmtpServer, Transport, MARK_CID,
};
use crate::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_mailbox::graph_fake::FakeGraph;
use dmarc_store::{save_settings, AppSettings, SendingSettings};
use futures::future::BoxFuture;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// Keeps each send's configuration and message, then sends it through
/// `inner` (the network, paced).
struct Tee {
    kept: Mutex<Vec<(SendConfig, Message)>>,
    inner: PacedSender<NetworkSender>,
}

impl Tee {
    fn new(interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            kept: Mutex::default(),
            inner: PacedSender::new(NetworkSender, interval),
        })
    }

    fn kept(&self) -> Vec<(SendConfig, Message)> {
        self.kept.lock().unwrap().clone()
    }
}

impl MailSender for Tee {
    fn send<'a>(
        &'a self,
        config: &'a SendConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        self.kept
            .lock()
            .unwrap()
            .push((config.clone(), message.clone()));
        self.inner.send(config, message)
    }
}

/// `mime` with what differs between two builds of one message made the
/// same: the `Date` and `Message-ID` lines and the MIME boundaries.
fn normalised(mime: &[u8]) -> String {
    let mut text = String::from_utf8(mime.to_vec()).expect("an ASCII message");
    let mut boundaries = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("boundary=\"") {
        let tail = &rest[at + 10..];
        let end = tail.find('"').expect("a closed boundary");
        boundaries.push(tail[..end].to_owned());
        rest = &tail[end..];
    }
    for (n, boundary) in boundaries.iter().enumerate() {
        text = text.replace(boundary.as_str(), &format!("BOUNDARY-{n}"));
    }
    text.split("\r\n")
        .map(|line| {
            if line.starts_with("Date: ") {
                "Date: DATE"
            } else if line.starts_with("Message-ID: ") {
                "Message-ID: ID"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Sending as `MAILBOX` at `graph` with `token`'s source.
fn graph_config(graph: &FakeGraph, endpoint: &FakeTokenEndpoint) -> SendConfig {
    let source = Arc::new(endpoint.source());
    SendConfig {
        transport: Transport::Graph(GraphSendConfig {
            address: MAILBOX.into(),
            api: graph_api(graph),
            token: Arc::new(crate::accounts::oauth::GraphAccountToken::new(
                source,
                crate::accounts::oauth::tests::certificate_request(),
            )),
        }),
        sender: MAILBOX.parse().unwrap(),
        sender_name: "Acme IT via Marcan".into(),
        reply_to: Some("help@example.com".parse().unwrap()),
    }
}

fn html_message() -> Message {
    Message {
        to: "owner@example.org".parse().unwrap(),
        subject: "Marcan: 2 changes".into(),
        text: "Two things changed.".into(),
        html: Some(format!(
            "<p><img src=\"cid:{MARK_CID}\">Two things changed.</p>"
        )),
        images: vec![InlineImage::marcan_mark()],
    }
}

/// What Graph receives is exactly the message `build` made, byte for byte:
/// the bytes SMTP would carry after `DATA`, headers, images and all.
#[tokio::test]
async fn graph_gets_the_built_message_byte_for_byte() {
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = FakeGraph::start(MAILBOX).await;
    let config = graph_config(&graph, &endpoint);
    let email = build(&config, &html_message()).unwrap();
    let mime = email.formatted();
    deliver_mime(config.graph().unwrap(), &mime).await.unwrap();
    let sent = graph.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].user, MAILBOX);
    assert_eq!(sent[0].mime, mime);
    let text = String::from_utf8(mime).unwrap();
    for header in [
        "From: \"Acme IT via Marcan\" <dmarc@example.com>\r\n",
        "To: owner@example.org\r\n",
        "Reply-To: help@example.com\r\n",
        "Subject: Marcan: 2 changes\r\n",
        "Content-ID: <marcan-mark>\r\n",
    ] {
        assert!(text.contains(header), "{header} in {text}");
    }
    // Every recipient SMTP would deliver to is in the headers Graph reads,
    // and only that one: lettre's envelope is its `To`, no `Cc`, no `Bcc`.
    let envelope: Vec<String> = email
        .envelope()
        .to()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(envelope, ["owner@example.org"]);
    assert!(
        !text.contains("\r\nCc:") && !text.contains("\r\nBcc:"),
        "{text}"
    );
    let request = graph.requests().pop().unwrap();
    assert_eq!(
        (request.method.as_str(), request.content_type.as_str()),
        ("POST", "text/plain")
    );
    assert_eq!(request.auth, format!("Bearer {}", token(1)));
}

/// A refused token is fetched anew once; throttling is waited out; each
/// refusal is said in words, Graph's code beside it, no token in either.
#[tokio::test]
async fn graphs_answers_become_send_errors_in_words() {
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = FakeGraph::start(MAILBOX).await;
    let config = graph_config(&graph, &endpoint);

    // The first token is refused: a new one is fetched, the send goes out.
    graph.accept_tokens(&[&token(2)]);
    send(&config, &html_message()).await.unwrap();
    assert_eq!(endpoint.grants(), 2);
    assert_eq!(graph.sent().len(), 1);
    // Refused again and again: one new token, then the sentence.
    graph.accept_tokens(&["none of these"]);
    let err = send(&config, &html_message()).await.unwrap_err();
    assert_eq!(err.reason(), "auth");
    assert!(
        err.message()
            .contains("refused the app registration's token"),
        "{err}"
    );
    assert!(err.detail().contains("InvalidAuthenticationToken"), "{err}");
    assert_eq!(err.docs(), None);
    assert_eq!(endpoint.grants(), 3);
    graph.accept_tokens(&[]);

    // Throttled once: waited out.
    graph.throttle(1, 1);
    let started = std::time::Instant::now();
    send(&config, &html_message()).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(1));
    // Throttled on and on: given up within the send's bound.
    graph.throttle(100, 0);
    let err = send(&config, &html_message()).await.unwrap_err();
    assert_eq!(
        (err.reason(), err.cause()),
        ("other", "server busy"),
        "{err}"
    );
    assert!(err.detail().contains("ApplicationThrottled"), "{err}");
    graph.throttle(0, 0);

    let cases = [
        (
            403,
            "ErrorAccessDenied",
            GraphRefusal::NoSendRole,
            "sender_rejected",
        ),
        // Marcan sends as the mailbox only: a send-as refusal is the role.
        (
            403,
            "ErrorSendAsDenied",
            GraphRefusal::NoSendRole,
            "sender_rejected",
        ),
        (
            404,
            "ErrorInvalidUser",
            GraphRefusal::NoMailbox,
            "sender_rejected",
        ),
        (
            400,
            "MailboxNotEnabledForRESTAPI",
            GraphRefusal::NoMailbox,
            "sender_rejected",
        ),
        (
            413,
            "RequestEntityTooLarge",
            GraphRefusal::TooLarge,
            "rejected",
        ),
        (
            400,
            "ErrorMimeContentInvalidBase64String",
            GraphRefusal::Refused,
            "rejected",
        ),
        (
            500,
            "ErrorInternalServerError",
            GraphRefusal::Refused,
            "rejected",
        ),
    ];
    for (status, code, refusal, reason) in cases {
        let status = StatusCode::from_u16(status).unwrap();
        graph.refuse_send(Some((status, code, "Graph says no.")));
        let err = send(&config, &html_message()).await.unwrap_err();
        graph.refuse_send(None);
        assert_eq!(
            err,
            SendError::Graph {
                mailbox: MAILBOX.into(),
                refusal: refusal.clone(),
                detail: format!("{code}: Graph says no."),
            }
        );
        assert_eq!(err.reason(), reason, "{code}");
        assert!(!err.is_permanent(), "{code}");
        assert!(!err.to_string().contains(&token(1)), "{err}");
        assert_eq!(
            err.message(),
            crate::wording::microsoft::graph_send_sentence(MAILBOX, &refusal)
        );
    }

    // The role does not cover the mailbox: the fake's own 403.
    graph.forbid(true);
    let err = send(&config, &html_message()).await.unwrap_err();
    assert!(err.message().contains("Application Mail.Send"), "{err}");
    assert!(err.detail().starts_with("ErrorAccessDenied"), "{err}");
    assert_eq!(err.docs(), Some("accounts#exchange-online"));
    graph.forbid(false);

    // Unreachable: said with the host; nothing hangs.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed =
        reqwest::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    drop(listener);
    let mut gone = config.clone();
    if let Transport::Graph(mailbox) = &mut gone.transport {
        mailbox.api = dmarc_mailbox::GraphApi::new(closed);
    }
    let err = send(&gone, &html_message()).await.unwrap_err();
    assert_eq!(
        (err.reason(), err.cause()),
        ("connect", "server unreachable")
    );
    assert!(
        err.message()
            .starts_with("Could not reach Microsoft Graph (127.0.0.1)"),
        "{err}"
    );

    // The whole send ran out of time.
    let err = timed_out(config.graph().unwrap(), Duration::from_secs(60));
    assert_eq!(err.reason(), "other");
    assert!(
        err.message().contains("may or may not have been sent"),
        "{err}"
    );
}

async fn call(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
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

/// Sending through a Microsoft account: the Sending test, a test alert and
/// a client report preview (with Marcan's mark inline) go out over Graph
/// as the mailbox, each the message SMTP would have carried; one at a time
/// with the pacing between them; and the same buttons through a password
/// account still go to its SMTP server.
#[tokio::test]
async fn a_microsoft_account_sends_every_kind_of_mail_over_graph() {
    let (_dir, pool) = pool().await;
    let endpoint = FakeTokenEndpoint::start(vec![registration()]).await;
    let graph = FakeGraph::start(MAILBOX).await;
    let id = add(&pool, &microsoft_account(), private_key()).await;
    save_settings(&pool, &sending_through(id).sending)
        .await
        .unwrap();
    let org = dmarc_store::get_or_create_organization(&pool, "Acme")
        .await
        .unwrap();
    let interval = Duration::from_millis(400);
    let tee = Tee::new(interval);
    let state = AppState::new(pool.clone())
        .with_token_source(endpoint.source())
        .with_graph_api(graph_api(&graph))
        .with_mail_sender(tee.clone());
    let app = crate::router_with_state(state.clone());

    let started = std::time::Instant::now();
    for (path, body) in [
        ("/api/settings/sending/test", json!({})),
        ("/api/settings/alerts/test", json!({})),
        (
            "/api/settings/client-reports/email/send",
            json!({ "organization_id": org.id }),
        ),
    ] {
        let (status, answer) = call(&app, path, body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {answer}");
    }
    assert!(started.elapsed() >= interval * 2, "paced");

    let kept = tee.kept();
    let sent = graph.sent();
    assert_eq!((kept.len(), sent.len()), (3, 3));
    assert_eq!(endpoint.grants(), 1, "one token for all three");
    for ((config, message), graph_sent) in kept.iter().zip(&sent) {
        assert_eq!(config.graph().map(|g| g.address.as_str()), Some(MAILBOX));
        assert_eq!(graph_sent.user, MAILBOX);
        assert_eq!(
            message.to.to_string(),
            MAILBOX,
            "the sender is the test recipient"
        );
        let built = build(config, message).unwrap().formatted();
        assert_eq!(normalised(&graph_sent.mime), normalised(&built));
    }
    let subjects: Vec<&str> = kept.iter().map(|(_, m)| m.subject.as_str()).collect();
    assert_eq!(subjects[0], "Marcan test message");
    assert!(subjects[1].starts_with("[Test] "), "{subjects:?}");
    assert!(subjects[2].starts_with("[Preview] Acme: "), "{subjects:?}");
    let report = String::from_utf8(sent[2].mime.clone()).unwrap();
    assert!(report.contains("Content-ID: <marcan-mark>\r\n"), "{report}");
    assert!(report.contains("multipart/related"), "{report}");

    // A refusal reaches the button as the sending test's error.
    graph.forbid(true);
    let (status, answer) = call(&app, "/api/settings/sending/test", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
    assert_eq!(answer["reason"], "sender_rejected");
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .contains("Application Mail.Send"),
        "{answer}"
    );
    assert!(answer["detail"]
        .as_str()
        .unwrap()
        .starts_with("ErrorAccessDenied"));
    graph.forbid(false);

    // A password account: the same button resolves to its SMTP server.
    let password = dmarc_store::AccountFields {
        name: "Relay".into(),
        address: "relay@example.com".into(),
        username: "relay@example.com".into(),
        smtp_host: "smtp.example.com".into(),
        smtp_port: Some(465),
        ..dmarc_store::AccountFields::default()
    };
    let relay = dmarc_store::insert_account(
        &pool,
        None,
        &password,
        &[dmarc_store::SecretRow::plain(
            dmarc_store::SecretKind::Password,
            "relay secret",
        )],
        0,
    )
    .await
    .unwrap();
    let config = state
        .accounts
        .sending_config(
            &AppSettings::load(&pool).await.unwrap(),
            &SendingSettings {
                enabled: true,
                account_id: Some(relay),
                ..SendingSettings::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        config.transport,
        Transport::Smtp(SmtpServer {
            host: "smtp.example.com".into(),
            port: 465,
            security: dmarc_store::SmtpSecurity::Tls,
            username: "relay@example.com".into(),
            password: "relay secret".into(),
        })
    );
}
