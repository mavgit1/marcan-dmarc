//! The Microsoft login's certificate over the API: generated for an
//! unsaved form (held in memory under a draft id), downloaded before the
//! save, and a saved account's downloaded as `.cer`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_store::{insert_account, AccountFields, LoginKind, MicrosoftLogin};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

mod common;
use common::{post_json, TestApp};

/// `(status, content type, content disposition, body)` of a GET.
async fn download(app: &axum::Router, path: &str) -> (StatusCode, String, String, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let text = |name: header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let (ctype, disposition) = (
        text(header::CONTENT_TYPE),
        text(header::CONTENT_DISPOSITION),
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, ctype, disposition, body.to_vec())
}

#[tokio::test]
async fn a_certificate_is_generated_for_the_form_and_downloaded() {
    let t = TestApp::new().await;
    let (status, draft) = post_json(
        &t.app,
        "/api/accounts/certificate",
        json!({ "subject": " dmarc@example.com " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{draft}");
    let id = draft["draft_id"].as_str().unwrap();
    assert_eq!(id.len(), 32);
    assert_eq!(draft["subject"], "dmarc@example.com");
    assert_eq!(draft["thumbprint"].as_str().unwrap().len(), 40);
    assert!(draft["expires_at"].as_i64().unwrap() > dmarc_store::unix_now() + 700 * 86_400);
    assert!(draft.get("private_key_pem").is_none());

    let (status, ctype, disposition, der) =
        download(&t.app, &format!("/api/accounts/certificate/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ctype, "application/pkix-cert");
    assert_eq!(
        disposition,
        "attachment; filename=\"marcan-dmarc-example.com.cer\""
    );
    assert_eq!(der[0], 0x30, "DER");
    let (status, ..) = download(&t.app, "/api/accounts/certificate/0000").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A saved account's certificate, by its id.
    let (status, ..) = download(&t.app, "/api/accounts/1/certificate.cer").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let fields = AccountFields {
        name: "Reports".into(),
        address: "dmarc@example.com".into(),
        login_kind: LoginKind::Microsoft,
        microsoft: MicrosoftLogin {
            cert_der: Some(der.clone()),
            ..MicrosoftLogin::default()
        },
        ..AccountFields::default()
    };
    let account = insert_account(&t.pool, None, &fields, &[], 0)
        .await
        .unwrap();
    let (status, ctype, _, saved) =
        download(&t.app, &format!("/api/accounts/{account}/certificate.cer")).await;
    assert_eq!(
        (status, ctype.as_str()),
        (StatusCode::OK, "application/pkix-cert")
    );
    assert_eq!(saved, der);
}
