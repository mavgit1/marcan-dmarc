//! Shared harness for the API integration tests: an in-process app over a
//! temp SQLite database, request helpers, and report fixtures.
//!
//! Every file uses only what it needs, so `dead_code` is allowed here.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_parser::{DkimAuthResult, PolicyPublished, Record, Report, SpfAuthResult};
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{get_or_create_domain, get_or_create_organization, init};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

/// An operator key for `test@example.com`, signed by the test keypair that
/// the `test-keys` feature makes the server trust; a release build rejects it.
pub const TEST_OPERATOR_KEY: &str = dmarc_server::license::test_keys::TEST_KEY;

/// A router over a fresh temp database, with the pool and state it was built from.
pub struct TestApp {
    pub dir: tempfile::TempDir,
    pub pool: SqlitePool,
    pub state: AppState,
    pub app: axum::Router,
}

impl TestApp {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = init(&dir.path().join("test.db")).await.expect("init");
        Self::over(dir, pool, AppState::new)
    }

    /// [`Self::new`] whose DNS checks ask `resolver`, which answers at once.
    pub async fn with_resolver(resolver: dmarc_dns::TableResolver) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = init(&dir.path().join("test.db")).await.expect("init");
        let resolver = std::sync::Arc::new(resolver);
        Self::over(dir, pool, |pool| {
            AppState::new(pool).with_dns_resolver(resolver)
        })
    }

    fn over(
        dir: tempfile::TempDir,
        pool: SqlitePool,
        state: impl FnOnce(SqlitePool) -> AppState,
    ) -> Self {
        let state = state(pool.clone());
        let app = router_with_state(state.clone());
        Self {
            dir,
            pool,
            state,
            app,
        }
    }
}

/// `(status, JSON body)` of a GET; `Null` for an empty or non-JSON body.
pub async fn get(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    send(app, "GET", path, None).await
}

/// The JSON body of a GET that must answer 200.
pub async fn get_ok(app: &axum::Router, path: &str) -> Value {
    let (status, body) = get(app, path).await;
    assert_eq!(status, StatusCode::OK, "{path}: {body}");
    body
}

/// `(status, JSON body)` of a JSON POST.
pub async fn post_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    send(app, "POST", path, Some(body)).await
}

/// `(status, JSON body)` of a JSON PUT.
pub async fn put_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    send(app, "PUT", path, Some(body)).await
}

/// `(status, JSON body)` of any request; `body` is sent as JSON when given.
pub async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(path);
    let request = match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

/// `(status, JSON body)` of a POST of raw bytes with `content_type` (an upload).
pub async fn post_bytes(
    app: &axum::Router,
    path: &str,
    content_type: &str,
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `(status, content type, content disposition, body)` of a GET, for downloads.
pub async fn get_raw(app: &axum::Router, path: &str) -> (StatusCode, String, String, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let header_text = |name: header::HeaderName| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let ctype = header_text(header::CONTENT_TYPE);
    let disp = header_text(header::CONTENT_DISPOSITION);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        ctype,
        disp,
        String::from_utf8(bytes.to_vec()).unwrap(),
    )
}

/// A report from `google.com` for `domain` with `p=reject`, relaxed alignment,
/// and tallies derived from its records.
pub fn report(report_id: &str, domain: &str, date_begin: i64, records: Vec<Record>) -> Report {
    report_with_policy(report_id, domain, date_begin, "reject", "reject", records)
}

/// [`report`] with explicit `p=` and `sp=`.
pub fn report_with_policy(
    report_id: &str,
    domain: &str,
    date_begin: i64,
    p: &str,
    sp: &str,
    records: Vec<Record>,
) -> Report {
    Report {
        report_id: report_id.into(),
        org_name: "google.com".into(),
        email: "noreply@google.com".into(),
        date_begin,
        date_end: date_begin + 86_399,
        policy: PolicyPublished {
            domain: domain.into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some(p.into()),
            sp: Some(sp.into()),
            pct: Some(100),
            fo: None,
        },
        records,
        raw_xml: "<feedback/>".into(),
    }
}

/// A delivered record from `example.com` with one DKIM and one SPF result.
pub fn rec(ip: &str, count: i64, dkim: &str, spf: &str) -> Record {
    Record {
        source_ip: ip.into(),
        count,
        disposition: Some("none".into()),
        dkim_result: Some(dkim.into()),
        spf_result: Some(spf.into()),
        header_from: Some("example.com".into()),
        envelope_from: None,
        envelope_to: None,
        dkim_auth: vec![DkimAuthResult {
            domain: "example.com".into(),
            selector: None,
            result: dkim.into(),
        }],
        spf_auth: vec![SpfAuthResult {
            domain: "example.com".into(),
            result: spf.into(),
            scope: None,
        }],
        policy_overrides: vec![],
    }
}

/// Ingest puts domains into whichever organization already holds them, so a
/// fixture pre-assigns its domains to the organizations its assertions expect:
/// Acme owns example.com, Beta owns other.org.
pub async fn seed_organizations(pool: &SqlitePool) {
    for (organization, domain) in [("Acme", "example.com"), ("Beta", "other.org")] {
        let organization = get_or_create_organization(pool, organization)
            .await
            .expect("organization");
        get_or_create_domain(pool, organization.id, domain)
            .await
            .expect("domain");
    }
}

/// Record a mailbox pass that finished without error at `at`: the mailbox
/// was read then.
pub async fn record_mailbox_read(pool: &SqlitePool, at: i64) {
    let pass = dmarc_store::ImapPassRow {
        finished_at: at,
        checked_at: Some(at),
        ..Default::default()
    };
    dmarc_store::record_imap_pass(pool, &pass).await.unwrap();
}

/// Store a complete DNS result for `domain` with `dmarc` and `spf` as its
/// records (no DKIM, MTA-STS, TLS-RPT or BIMI), checked at `checked_at`.
pub async fn store_dns(pool: &SqlitePool, domain: &str, dmarc: &str, spf: &str, checked_at: i64) {
    use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
    use dmarc_dns::dmarc::evaluate_dmarc;
    use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
    use dmarc_dns::spf::evaluate_spf;
    use dmarc_dns::{assemble_report, TxtAnswer, TxtLookup};
    let txt = |record: &str| TxtLookup::Records(TxtAnswer::from_records(vec![record.to_owned()]));
    let report = assemble_report(
        domain,
        evaluate_dmarc(domain, &txt(dmarc), &[]),
        evaluate_spf(domain, &txt(spf), &[]),
        evaluate_dkim(domain, &[], &DkimProbe::new(vec![], &[])),
        evaluate_mta_sts(
            domain,
            &TxtLookup::NoRecords,
            None,
            &PolicyFetch::NotAttempted,
        ),
        evaluate_tls_rpt(domain, &TxtLookup::NoRecords),
        evaluate_bimi(domain, &TxtLookup::NoRecords),
    );
    assert!(report.is_complete());
    let snapshot = dmarc_store::DnsSnapshot {
        checked_at,
        ..dmarc_server::dns_snapshot(&report).unwrap()
    };
    assert!(dmarc_store::upsert_dns_snapshot(pool, &snapshot)
        .await
        .unwrap());
}
