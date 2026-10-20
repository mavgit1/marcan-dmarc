//! Integration tests for `GET /api/dns/check` (no network: served from a snapshot).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
use dmarc_dns::dmarc::evaluate_dmarc;
use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
use dmarc_dns::spf::evaluate_spf;
use dmarc_dns::{assemble_report, TxtAnswer, TxtLookup, CHECKER_VERSION};
use dmarc_server::router;
use dmarc_store::{init, upsert_dns_snapshot, DnsSnapshot};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn fresh_complete_snapshot_is_served_with_status_ok() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("dns_api.db")).await.unwrap();
    let domain = "status-ok.example";
    // Snapshots are kept only for monitored domains.
    dmarc_store::create_domain(&pool, 1, domain).await.unwrap();
    let dmarc = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1; p=reject".into()]));
    let spf = TxtLookup::Records(TxtAnswer::from_records(vec!["v=spf1 -all".into()]));
    let report = assemble_report(
        domain,
        evaluate_dmarc(domain, &dmarc, &[]),
        evaluate_spf(domain, &spf, &[]),
        evaluate_dkim(domain, &[], &DkimProbe::default()),
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
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: domain.into(),
            checked_at: report.checked_at.timestamp(),
            grade: report.grade.clone(),
            score: i64::from(report.score),
            critical_count: 0,
            dmarc: dmarc_server::dns_snapshot(&report).unwrap().dmarc,
            report_json: serde_json::to_string(&report).unwrap(),
            checker_version: i64::from(CHECKER_VERSION),
        },
    )
    .await
    .unwrap();

    let app = router(pool);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/dns/check?domain={domain}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["cached"], true);
    assert_eq!(json["last_attempt_at"], Value::Null);
    assert_eq!(json["domain"], domain);
    assert_eq!(json["core_measured"], true);
    assert_eq!(
        json["checked_at"],
        serde_json::to_value(report.checked_at).unwrap()
    );
}

/// A result of an older checker is still shown, marked `updating` on the DNS
/// check and as `dns_updating` on the domain list, until the re-check lands.
#[tokio::test]
async fn older_checker_result_is_served_as_updating() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("dns_api.db")).await.unwrap();
    let domain = "status-updating.example";
    dmarc_store::create_domain(&pool, 1, domain).await.unwrap();
    let dmarc = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1; p=reject".into()]));
    let spf = TxtLookup::Records(TxtAnswer::from_records(vec!["v=spf1 -all".into()]));
    let report = assemble_report(
        domain,
        evaluate_dmarc(domain, &dmarc, &[]),
        evaluate_spf(domain, &spf, &[]),
        evaluate_dkim(domain, &[], &DkimProbe::default()),
        evaluate_mta_sts(
            domain,
            &TxtLookup::NoRecords,
            None,
            &PolicyFetch::NotAttempted,
        ),
        evaluate_tls_rpt(domain, &TxtLookup::NoRecords),
        evaluate_bimi(domain, &TxtLookup::NoRecords),
    );
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: domain.into(),
            checked_at: report.checked_at.timestamp(),
            grade: report.grade.clone(),
            score: i64::from(report.score),
            critical_count: 0,
            dmarc: dmarc_server::dns_snapshot(&report).unwrap().dmarc,
            report_json: serde_json::to_string(&report).unwrap(),
            checker_version: i64::from(CHECKER_VERSION) - 1,
        },
    )
    .await
    .unwrap();

    let app = router(pool);
    let get = |uri: String| {
        let app = app.clone();
        async move {
            let response = app
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let check = get(format!("/api/dns/check?domain={domain}")).await;
    assert_eq!(check["status"], "updating");
    assert_eq!(check["grade"], report.grade.as_str());

    let list = get("/api/domains".into()).await;
    let row = &list["items"][0];
    assert_eq!(row["domain"], domain);
    assert_eq!(row["dns"]["grade"], report.grade.as_str());
    assert_eq!(row["dns_updating"], true);
    assert!(row["dns"].get("checker_version").is_none());
}

/// A demo database carries its own TXT records: the server answers from
/// them, wildcards included, and every other name does not exist. A real
/// database has none, and the checks use the network.
#[tokio::test]
async fn a_demo_database_answers_from_its_own_records() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("demo.db")).await.unwrap();
    assert!(dmarc_server::demo_resolver(&pool).await.unwrap().is_none());

    let records = std::collections::BTreeMap::from([
        (
            "_dmarc.example.com".to_owned(),
            vec!["v=DMARC1; p=reject".to_owned()],
        ),
        (
            "*._report._dmarc.example.com".to_owned(),
            vec!["v=DMARC1".to_owned()],
        ),
    ]);
    dmarc_store::set_demo_dns_records(&pool, &records)
        .await
        .unwrap();
    let resolver = dmarc_server::demo_resolver(&pool)
        .await
        .unwrap()
        .expect("demo resolver");
    assert_eq!(
        resolver.txt("_dmarc.example.com").await.records(),
        ["v=DMARC1; p=reject"]
    );
    assert_eq!(
        resolver
            .txt("example.org._report._dmarc.example.com")
            .await
            .records(),
        ["v=DMARC1"]
    );
    assert_eq!(
        resolver.txt("_dmarc.example.org").await,
        TxtLookup::NxDomain
    );
}
