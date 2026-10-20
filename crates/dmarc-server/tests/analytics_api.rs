//! Analytics, sender and organization endpoints (CSV exports: `export_api.rs`).

use axum::http::StatusCode;
use dmarc_parser::{DkimAuthResult, PolicyOverride, Record, SpfAuthResult};
use dmarc_server::router;
use dmarc_store::{get_or_create_organization, ingest_report, init};
mod common;
use common::{get as get_json, get_raw, rec, report_with_policy, seed_organizations};
use dmarc_parser::Report;

/// A `p=quarantine`, `sp=none` report: this file's assertions tell the two apart.
fn report(report_id: &str, domain: &str, date_begin: i64, records: Vec<Record>) -> Report {
    report_with_policy(report_id, domain, date_begin, "quarantine", "none", records)
}

const MONDAY: i64 = 1_704_067_200;
const DAY: i64 = 86_400;

fn rec_full(
    ip: &str,
    count: i64,
    dkim: &str,
    spf: &str,
    disposition: Option<&str>,
    header_from: Option<&str>,
    overrides: Vec<PolicyOverride>,
) -> Record {
    Record {
        source_ip: ip.into(),
        count,
        disposition: disposition.map(str::to_owned),
        dkim_result: Some(dkim.into()),
        spf_result: Some(spf.into()),
        header_from: Some(header_from.unwrap_or("example.com").into()),
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
        policy_overrides: overrides,
    }
}

async fn seeded_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let pool = init(&db_path).await.expect("init");
    seed_organizations(&pool).await;

    ingest_report(
        &pool,
        &report(
            "rpt-mon",
            "example.com",
            MONDAY,
            vec![
                rec("192.0.2.1", 40, "pass", "pass"),
                rec_full(
                    "192.0.2.2",
                    10,
                    "fail",
                    "fail",
                    Some("none"),
                    Some("foo,bar.example"),
                    vec![PolicyOverride {
                        reason_type: Some("forwarded".into()),
                        comment: Some(r#"said "hello, world""#.into()),
                    }],
                ),
            ],
        ),
    )
    .await
    .expect("mon");

    ingest_report(
        &pool,
        &report(
            "rpt-wed",
            "example.com",
            MONDAY + 2 * DAY,
            vec![
                rec("192.0.2.1", 50, "pass", "fail"),
                rec("2001:db8::1", 80, "pass", "pass"),
            ],
        ),
    )
    .await
    .expect("wed");

    ingest_report(
        &pool,
        &report(
            "rpt-spoof",
            "other.org",
            MONDAY + 2 * DAY,
            vec![rec("192.0.2.99", 20, "fail", "fail")],
        ),
    )
    .await
    .expect("spoof");

    get_or_create_organization(&pool, "EmptyCo")
        .await
        .expect("empty");

    (dir, router(pool))
}

#[tokio::test]
async fn timeseries_fills_empty_day_and_sums() {
    let (_dir, app) = seeded_app().await;
    let (status, body) = get_json(&app, "/api/timeseries?bucket=day").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "monday, zero tuesday, wednesday");
    assert_eq!(items[0]["ts"], MONDAY);
    assert_eq!(items[0]["total_messages"], 50);
    assert_eq!(items[1]["ts"], MONDAY + DAY);
    assert_eq!(items[1]["total_messages"], 0);
    assert_eq!(items[1]["delivered_failures"], 0);
    assert_eq!(items[2]["ts"], MONDAY + 2 * DAY);
    let sum: i64 = items
        .iter()
        .map(|p| p["total_messages"].as_i64().unwrap())
        .sum();
    assert_eq!(sum, 200); // 50 + 0 + 150
}

#[tokio::test]
async fn timeseries_rejects_bad_bucket() {
    let (_dir, app) = seeded_app().await;
    let (status, body) = get_json(&app, "/api/timeseries?bucket=month").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
}

#[tokio::test]
async fn statistics_include_delivered_failures_and_previous_window() {
    let (_dir, app) = seeded_app().await;
    let (st, ext) = get_json(&app, "/api/statistics").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ext["total_messages"], 200);
    // 10 + 20 failing and delivered (disposition none); the 10 carry a `forwarded`
    // override, so they count as forwarded, not as delivered failures.
    assert_eq!(ext["delivered_failures"], 20);
    assert_eq!(ext["forwarded_failures"], 10);
    // No `since`: no previous window to compare against.
    assert_eq!(ext["has_previous_window"], false);
    assert_eq!(ext["prev_total_messages"], 0);
    assert!(ext["delivered_compliance_trend"].is_null());
    assert!(ext["delivered_failures_trend"].is_null());
    // Removed fields stay removed.
    for gone in [
        "total_messages_delta_pct",
        "delivered_compliance_pct_delta",
        "compliance_pct",
        "prev_compliance_pct",
        "spf_pass_pct",
        "dkim_pass_pct",
    ] {
        assert!(ext.get(gone).is_none(), "{gone}");
    }
}

#[tokio::test]
async fn sender_by_ip_ipv6_and_spoof_and_errors() {
    let (_dir, app) = seeded_app().await;

    let (status, _) = get_json(&app, "/api/senders/not-an-ip").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = get_json(&app, "/api/senders/203.0.113.9").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let encoded = "/api/senders/2001%3Adb8%3A%3A1";
    let (status, body) = get_json(&app, encoded).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_messages"], 80);
    assert_eq!(body["compliance_pct"].as_f64().unwrap(), 100.0);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["compliance_pct"].as_f64().unwrap(), 100.0);

    let (status, body) = get_json(&app, "/api/senders/192.0.2.99").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_messages"], 20);
    assert_eq!(body["compliance_pct"].as_f64().unwrap(), 0.0);
    assert_eq!(body["items"][0]["domain"], "other.org");
}

#[tokio::test]
async fn organization_detail_and_404() {
    let (_dir, app) = seeded_app().await;
    let acme_id = organization_id(&app, "Acme").await;

    // The organization endpoint is identity only; figures come from the scoped endpoints below.
    let (status, body) = get_json(&app, &format!("/api/organizations/{acme_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Acme");
    assert_eq!(body["is_default"], false);
    assert_eq!(body["domain_count"], 1);
    assert_eq!(body["report_count"], 2);
    assert!(body.get("items").is_none());

    let (status, body) = get_json(&app, "/api/organizations/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}

async fn organization_id(app: &axum::Router, name: &str) -> i64 {
    get_json(app, "/api/organizations").await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

/// The organization page's stat cards (`/api/statistics`) and table (`/api/domains`) share one
/// scope, so per-domain rows add up to the organization totals in every time range.
#[tokio::test]
async fn organization_domains_and_statistics_share_scope() {
    let (_dir, app) = seeded_app().await;
    let acme_id = organization_id(&app, "Acme").await;

    let (status, body) = get_json(&app, &format!("/api/domains?organization_id={acme_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let row = &items[0];
    assert_eq!(row["domain"], "example.com");
    assert_eq!(row["organization"]["name"], "Acme");
    assert_eq!(row["total_messages"], 180); // 50 + 130
    assert_eq!(row["compliant_messages"], 170); // 40 + 130
                                                // The 10 failing, delivered messages carry a `forwarded` override.
    assert_eq!(row["delivered_failures"], 0);
    assert_eq!(row["forwarded_failures"], 10);
    // The reports say p=quarantine, but the policy is the DNS record's: none yet.
    assert!(row["policy"].is_null());
    assert_eq!(row["policy_source"], "not_checked");
    assert_eq!(row["last_report_at"], MONDAY + 3 * DAY - 1);

    let (_, stats) = get_json(&app, &format!("/api/statistics?organization_id={acme_id}")).await;
    assert_eq!(stats["total_messages"], 180);
    assert_eq!(stats["compliant_messages"], 170);
    assert_eq!(stats["delivered_failures"], 0);
    assert_eq!(stats["forwarded_failures"], 10);

    // `since` narrows volumes but not coverage: the Monday report drops out of the
    // figures while the domain keeps its report count and newest report date.
    let since = MONDAY + DAY;
    let (status, body) = get_json(
        &app,
        &format!("/api/domains?organization_id={acme_id}&since={since}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let row = &body["items"][0];
    assert_eq!(row["total_messages"], 130);
    assert_eq!(row["compliant_messages"], 130);
    assert_eq!(row["delivered_failures"], 0);
    assert_eq!(row["report_count"], 2);
    assert_eq!(row["last_report_at"], MONDAY + 3 * DAY - 1);

    let (_, stats) = get_json(
        &app,
        &format!("/api/statistics?organization_id={acme_id}&since={since}"),
    )
    .await;
    assert_eq!(stats["total_messages"], 130);
    assert_eq!(stats["compliant_messages"], 130);

    // A organization without domains lists nothing; unknown ranges are rejected.
    let empty_id = organization_id(&app, "EmptyCo").await;
    let (_, body) = get_json(&app, &format!("/api/domains?organization_id={empty_id}")).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    let (status, _) = get_json(&app, "/api/domains?since=yesterday").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A record the reporter marked `dkim=pass` whose signing domain does not align.
/// Alignment-blind `policy_evaluated` would call it compliant; `dmarc_pass` does not.
fn rec_unaligned_dkim_pass(ip: &str, count: i64, dkim_domain: &str) -> Record {
    Record {
        source_ip: ip.into(),
        count,
        disposition: Some("none".into()),
        dkim_result: Some("pass".into()),
        spf_result: Some("fail".into()),
        header_from: Some("example.com".into()),
        envelope_from: None,
        envelope_to: None,
        dkim_auth: vec![DkimAuthResult {
            domain: dkim_domain.into(),
            selector: None,
            result: "pass".into(),
        }],
        spf_auth: vec![SpfAuthResult {
            domain: "example.com".into(),
            result: "fail".into(),
            scope: None,
        }],
        policy_overrides: vec![],
    }
}

async fn divergent_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let pool = init(&db_path).await.expect("init");
    seed_organizations(&pool).await;

    ingest_report(
        &pool,
        &report(
            "rpt-divergent",
            "example.com",
            MONDAY,
            vec![
                // 100 delivered messages the reporter calls a DKIM pass but that
                // are signed by an unrelated domain — spoofing, not compliance.
                rec_unaligned_dkim_pass("198.51.100.1", 100, "evil.example"),
                rec("198.51.100.2", 10, "pass", "fail"),
                rec_full(
                    "198.51.100.3",
                    5,
                    "fail",
                    "fail",
                    Some("quarantine"),
                    None,
                    vec![],
                ),
            ],
        ),
    )
    .await
    .expect("divergent");

    (dir, router(pool))
}

/// Compliance must mean `dmarc_pass` on every screen this test touches, or one
/// source IP would band differently depending on which page the user arrived from.
#[tokio::test]
async fn unaligned_dkim_pass_is_not_compliant_on_any_screen() {
    let (_dir, app) = divergent_app().await;

    let (status, body) = get_json(&app, "/api/senders").await;
    assert_eq!(status, StatusCode::OK);
    let spoofer = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["source_ip"] == "198.51.100.1")
        .expect("spoofing IP in senders");
    assert_eq!(spoofer["total_messages"], 100);
    assert_eq!(spoofer["compliant_messages"], 0);
    assert_eq!(spoofer["failed_messages"], 100);
    assert_eq!(spoofer["compliance_pct"].as_f64().unwrap(), 0.0);
    // Aligned DKIM pass only: the unrelated signature does not count.
    assert_eq!(spoofer["dkim_pass_messages"], 0);

    let (status, body) = get_json(&app, "/api/senders/198.51.100.1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["compliant_messages"], 0);
    assert_eq!(body["compliance_pct"].as_f64().unwrap(), 0.0);
    assert_eq!(body["items"][0]["compliant_messages"], 0);
    // Sender detail uses the same aligned pass definition as the senders list.
    assert_eq!(body["items"][0]["dkim_pass_messages"], 0);
    assert_eq!(body["items"][0]["spf_pass_messages"], 0);

    let (status, body) = get_json(&app, "/api/timeseries?bucket=day").await;
    assert_eq!(status, StatusCode::OK);
    let day = &body["items"][0];
    assert_eq!(day["total_messages"], 115);
    assert_eq!(day["compliant_messages"], 10);
    // 100 delivered spoofs (disposition none); the other 5 were blocked.
    assert_eq!(day["delivered_failures"], 100);

    let (status, ext) = get_json(&app, "/api/statistics").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ext["total_messages"], 115);
    assert_eq!(ext["compliant_messages"], 10);
    assert_eq!(ext["report_count"], 1);
    assert_eq!(ext["delivered_failures"], 100);
    // The dashboard's blocked count: total - compliant - delivered failures must
    // equal the real quarantine+reject volume, which only holds if both terms use
    // dmarc_pass.
    let blocked = ext["total_messages"].as_i64().unwrap()
        - ext["compliant_messages"].as_i64().unwrap()
        - ext["delivered_failures"].as_i64().unwrap();
    assert_eq!(blocked, 5);

    // source_ip, sender_name, operator_domain, ptr_hostname, total, compliant, pct
    let (_, _, _, body) = get_raw(&app, "/api/export/senders.csv").await;
    assert!(body.contains("198.51.100.1,,,,100,0,0.00,"), "{body}");

    let (_, _, _, body) = get_raw(&app, "/api/export/auth-issues.csv").await;
    assert!(
        body.contains(",198.51.100.1,100,false,false,false,delivered,both_failed,"),
        "unaligned DKIM pass must appear in the auth-issues export: {body}"
    );
}

/// The Failures tab's `outcome` filter: forwarded records are not delivered failures, and
/// each value lists exactly the records with that outcome on the row.
#[tokio::test]
async fn failures_filter_by_outcome() {
    let (_dir, app) = seeded_app().await;
    let (_, domains) = get_json(&app, "/api/domains").await;
    let domain_id = |name: &str| {
        domains["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["domain"] == name)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let rows = |body: &serde_json::Value| -> Vec<(String, String)> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["source_ip"].as_str().unwrap().to_owned(),
                    r["outcome"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let pair = |ip: &str, outcome: &str| (ip.to_owned(), outcome.to_owned());
    let example = domain_id("example.com");
    let other = domain_id("other.org");
    for (domain, outcome, expected) in [
        (
            example,
            "",
            vec![pair("192.0.2.1", "passed"), pair("192.0.2.2", "forwarded")],
        ),
        (
            example,
            "all_failures",
            vec![pair("192.0.2.2", "forwarded")],
        ),
        (example, "forwarded", vec![pair("192.0.2.2", "forwarded")]),
        (example, "delivered", vec![]),
        (example, "blocked", vec![]),
        (example, "dkim_only", vec![pair("192.0.2.1", "passed")]),
        (example, "spf_only", vec![]),
        (other, "delivered", vec![pair("192.0.2.99", "delivered")]),
        (other, "all_failures", vec![pair("192.0.2.99", "delivered")]),
        (other, "forwarded", vec![]),
    ] {
        let (status, body) = get_json(
            &app,
            &format!("/api/domains/{domain}/failures?outcome={outcome}&sort=source_ip"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{outcome}");
        assert_eq!(rows(&body), expected, "domain {domain} outcome {outcome:?}");
    }
    let (status, _) = get_json(
        &app,
        &format!("/api/domains/{example}/failures?outcome=nope"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The export's outcome column agrees with the tab.
    let (_, _, _, body) = get_raw(&app, "/api/export/auth-issues.csv?outcome=forwarded").await;
    assert!(
        body.contains(",192.0.2.2,10,false,false,false,forwarded,both_failed,"),
        "{body}"
    );
    assert!(!body.contains("192.0.2.99"), "{body}");
}
