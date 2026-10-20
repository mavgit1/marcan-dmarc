//! The `Host` check and the anti-framing headers, through the whole router.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_server::{router_with_state, AllowedHosts, AppState};
use dmarc_store::init;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// A router like the binary's default: bound to 127.0.0.1:8080, with one
/// name a proxy forwards.
async fn trial_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let hosts = AllowedHosts::for_bind("127.0.0.1:8080", &["dmarc.example.com".to_owned()]);
    let state = AppState::new(pool).with_allowed_hosts(hosts);
    (dir, router_with_state(state))
}

async fn get_with_host(
    app: &axum::Router,
    path: &str,
    host: Option<&str>,
) -> axum::response::Response {
    let mut req = Request::builder().uri(path);
    if let Some(host) = host {
        req = req.header(header::HOST, host);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn assert_not_frameable(res: &axum::response::Response, what: &str) {
    assert_eq!(res.headers()[header::X_FRAME_OPTIONS], "DENY", "{what}");
    assert_eq!(
        res.headers()[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'",
        "{what}"
    );
}

/// A page that rebinds its own name to 127.0.0.1 sends that name as the
/// Host and is refused; the bind address, localhost and the listed name are
/// answered.
#[tokio::test]
async fn requests_for_other_hosts_are_refused() {
    let (_dir, app) = trial_app().await;
    for host in [
        "127.0.0.1:8080",
        "localhost:8080",
        "[::1]:8080",
        "dmarc.example.com",
    ] {
        let res = get_with_host(&app, "/api/meta", Some(host)).await;
        assert_eq!(res.status(), StatusCode::OK, "{host}");
    }
    for host in [Some("rebind.attacker.example:8080"), None] {
        let res = get_with_host(&app, "/api/meta", host).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST, "{host:?}");
        assert_not_frameable(&res, "refused");
        let body: serde_json::Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["code"], "bad_request", "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("--allowed-hosts"),
            "{body}"
        );
    }
}

/// The image binds every interface behind a proxy that forwards the public
/// name: with no names given, any host is answered.
#[tokio::test]
async fn every_interface_without_names_answers_any_host() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let state = AppState::new(pool).with_allowed_hosts(AllowedHosts::for_bind("0.0.0.0:8080", &[]));
    let app = router_with_state(state);
    let res = get_with_host(&app, "/healthz", Some("dmarc.example.org")).await;
    assert_eq!(res.status(), StatusCode::OK);
}

/// Every answer, an API answer, an error, the health check and the UI
/// fallback, forbids being shown in a frame.
#[tokio::test]
async fn every_response_forbids_framing() {
    let (_dir, app) = trial_app().await;
    for path in [
        "/healthz",
        "/api/meta",
        "/api/nothing-here",
        "/",
        "/settings",
    ] {
        let res = get_with_host(&app, path, Some("localhost:8080")).await;
        assert_not_frameable(&res, path);
    }
}
