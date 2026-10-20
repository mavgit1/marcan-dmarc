//! Integration tests for static asset SPA fallback and /api/system/health.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_server::router;
use dmarc_store::init;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_system_health_empty_db() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = init(&tmp.path().join("marcan.db")).await.unwrap();
    let app = router(pool);

    let req = Request::builder()
        .uri("/api/system/health")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );

    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();

    assert!(json.get("last_ingest_at").unwrap().is_null());
    assert!(json.get("hours_since_ingest").unwrap().is_null());
    assert_eq!(json.get("quarantine_count").unwrap().as_i64(), Some(0));
    assert_eq!(json.get("total_messages").unwrap().as_i64(), Some(0));
    assert_eq!(json.get("total_reports").unwrap().as_i64(), Some(0));
    assert!(json.get("imap_progress").unwrap().is_null());
    assert!(json.get("imap_last_error").unwrap().is_null());
    assert!(json.get("imap_last_error_at").unwrap().is_null());
    assert!(json.get("imap_last_success_at").unwrap().is_null());
    // Measured from the open pool, so a temp-file database reports its real size.
    assert!(json.get("database_size_bytes").unwrap().as_i64().unwrap() > 0);
    assert_eq!(json["imap_configured"], false);
    assert!(json["imap_poll_interval_secs"].as_u64().unwrap() > 0);
    assert_eq!(json["report_freshness_warning_hours"], 24);
}

/// Health counts the domains with a DNS result; a new domain asks for a
/// warm pass, so its check shows as running until that pass starts.
#[tokio::test]
async fn health_reports_dns_check_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = init(&tmp.path().join("marcan.db")).await.unwrap();
    let app = router(pool);
    let get_health = || async {
        let req = Request::builder()
            .uri("/api/system/health")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };

    let health = get_health().await;
    assert_eq!(health["dns_checks"]["domains"], 0);
    assert_eq!(health["dns_checks"]["checked"], 0);
    assert_eq!(health["dns_checks"]["checking"], false);

    let req = Request::builder()
        .method("POST")
        .uri("/api/domains")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"organization_id": 1, "domain": "new.example"}"#,
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    let health = get_health().await;
    assert_eq!(health["dns_checks"]["domains"], 1);
    assert_eq!(health["dns_checks"]["checked"], 0);
    assert_eq!(health["dns_checks"]["running"], true);
    assert_eq!(health["dns_checks"]["checking"], true);
}

/// The freshness threshold is saved under one name and read back through health.
#[tokio::test]
async fn report_freshness_threshold_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = init(&tmp.path().join("marcan.db")).await.unwrap();
    let app = router(pool);
    let send = |method: &str, uri: &str, body: Option<&str>| {
        let mut req = Request::builder().method(method).uri(uri);
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_owned())))
            .unwrap();
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };

    let (status, body) = send(
        "PUT",
        "/api/settings/monitoring",
        Some(r#"{"report_freshness_warning_hours": 500, "domain_reports_stopped_days": 7}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["monitoring"]["report_freshness_warning_hours"], 168,
        "clamped to a week"
    );
    let (_, health) = send("GET", "/api/system/health", None).await;
    assert_eq!(health["report_freshness_warning_hours"], 168);

    // The old telemetry endpoint is gone.
    let (status, _) = send("GET", "/api/settings/telemetry", None).await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn healthz_answers_ok_without_details() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = init(&tmp.path().join("marcan.db")).await.unwrap();
    let response = router(pool)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ok");
}

#[tokio::test]
async fn test_static_assets_spa_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = init(&tmp.path().join("marcan.db")).await.unwrap();

    // 1. Root route "/" should serve index.html with 200 and text/html
    let app = router(pool.clone());
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.starts_with("text/html"));
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);
    assert!(body_str.contains("<!DOCTYPE html>") || body_str.contains("<html"));

    // 2. Client-side route like "/domains" should fall back to index.html with 200
    let app = router(pool.clone());
    let req = Request::builder()
        .uri("/domains")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(ct.starts_with("text/html"));

    // 3. Missing /api/* route must return JSON 404, never index.html!
    let app = router(pool.clone());
    let req = Request::builder()
        .uri("/api/this-does-not-exist")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/json");
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json.get("error").unwrap().as_str(), Some("not found"));

    // 4. Embedded static file like "/robots.txt" should be served directly
    let app = router(pool.clone());
    let req = Request::builder()
        .uri("/robots.txt")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
