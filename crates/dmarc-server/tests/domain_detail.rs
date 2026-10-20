//! Integration tests for per-domain deep-dive endpoints.

use axum::http::StatusCode;
use dmarc_dns::TableResolver;
use dmarc_parser::{
    DkimAuthResult, PolicyOverride, PolicyPublished, Record, Report, SpfAuthResult,
};
use dmarc_server::{router, router_with_state, AppState};
use dmarc_store::{get_or_create_domain, get_or_create_organization, ingest_report, init};
use serde_json::Value;
use std::sync::Arc;
mod common;
use common::{get, get_raw, post_json, put_json, seed_organizations, send};

fn report(report_id: &str, domain: &str, date_begin: i64, records: Vec<Record>) -> Report {
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
            p: Some("reject".into()),
            sp: Some("quarantine".into()),
            pct: Some(100),
            fo: None,
        },
        records,
        raw_xml: "<feedback/>".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn rec(
    ip: &str,
    count: i64,
    disposition: Option<&str>,
    dkim_eval: Option<&str>,
    spf_eval: Option<&str>,
    dkim_auth: Vec<DkimAuthResult>,
    spf_auth: Vec<SpfAuthResult>,
    overrides: Vec<PolicyOverride>,
) -> Record {
    Record {
        source_ip: ip.into(),
        count,
        disposition: disposition.map(str::to_string),
        dkim_result: dkim_eval.map(str::to_string),
        spf_result: spf_eval.map(str::to_string),
        header_from: Some("example.com".into()),
        envelope_from: None,
        envelope_to: None,
        dkim_auth,
        spf_auth,
        policy_overrides: overrides,
    }
}

fn dkim(domain: &str, selector: Option<&str>, result: &str) -> DkimAuthResult {
    DkimAuthResult {
        domain: domain.into(),
        selector: selector.map(str::to_string),
        result: result.into(),
    }
}

fn spf(domain: &str, result: &str, scope: Option<&str>) -> SpfAuthResult {
    SpfAuthResult {
        domain: domain.into(),
        result: result.into(),
        scope: scope.map(str::to_string),
    }
}

async fn seeded_app() -> (tempfile::TempDir, axum::Router) {
    let (dir, _, app) = seeded().await;
    (dir, app)
}

async fn seeded() -> (tempfile::TempDir, sqlx::SqlitePool, axum::Router) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let pool = init(&db_path).await.expect("init");
    seed_organizations(&pool).await;

    // Record with TWO DKIM selectors and TWO SPF auth domains — fan-out bait.
    ingest_report(
        &pool,
        &report(
            "rpt-a1",
            "example.com",
            1_700_000_000,
            vec![
                rec(
                    "192.0.2.1",
                    40,
                    Some("none"),
                    Some("pass"),
                    Some("pass"),
                    vec![
                        dkim("example.com", Some("s1"), "pass"),
                        dkim("example.com", Some("s2"), "fail"),
                    ],
                    vec![
                        spf("example.com", "pass", Some("mfrom")),
                        spf("mail.example.com", "pass", Some("helo")),
                    ],
                    vec![],
                ),
                rec(
                    "192.0.2.2",
                    10,
                    Some("reject"),
                    Some("fail"),
                    Some("fail"),
                    vec![dkim("example.com", Some("s1"), "fail")],
                    vec![spf("example.com", "fail", Some("mfrom"))],
                    vec![PolicyOverride {
                        reason_type: Some("forwarded".into()),
                        comment: Some("ml".into()),
                    }],
                ),
            ],
        ),
    )
    .await
    .expect("ingest a1");

    // SPF-only failure (DKIM saves DMARC) + NULL disposition.
    ingest_report(
        &pool,
        &report(
            "rpt-a2",
            "example.com",
            1_710_000_000,
            vec![
                rec(
                    "192.0.2.1",
                    100,
                    Some("none"),
                    Some("pass"),
                    Some("fail"),
                    vec![dkim("example.com", Some("s1"), "pass")],
                    vec![spf("example.com", "fail", Some("mfrom"))],
                    vec![],
                ),
                rec(
                    "192.0.2.3",
                    7,
                    None,
                    Some("fail"),
                    Some("pass"),
                    vec![dkim("example.com", Some("s1"), "fail")],
                    vec![spf("example.com", "pass", Some("mfrom"))],
                    vec![],
                ),
            ],
        ),
    )
    .await
    .expect("ingest a2");

    // Same IP 192.0.2.1 also sends as other.org → other_domain_count.
    ingest_report(
        &pool,
        &report(
            "rpt-b1",
            "other.org",
            1_710_000_000,
            vec![rec(
                "192.0.2.1",
                5,
                Some("quarantine"),
                Some("fail"),
                Some("fail"),
                vec![dkim("other.org", Some("s1"), "fail")],
                vec![spf("other.org", "fail", Some("mfrom"))],
                vec![],
            )],
        ),
    )
    .await
    .expect("ingest b1");

    let organization = get_or_create_organization(&pool, "Acme")
        .await
        .expect("acme");
    get_or_create_domain(&pool, organization.id, "empty.test")
        .await
        .expect("empty domain");

    (dir, pool.clone(), router(pool))
}

async fn example_id(app: &axum::Router) -> i64 {
    let domains = get(app, "/api/domains").await.1;
    domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == "example.com")
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn overview_matches_record_sum_and_unknown_is_404() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    let (status, body) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["domain"], "example.com");
    assert_eq!(body["organization"]["name"], "Acme");
    // Not checked in DNS yet: no policy, though the reports name one.
    assert!(body["policy"].is_null());
    assert_eq!(body["policy_source"], "not_checked");
    // 40+10+100+7 — must not be inflated by the two DKIM / two SPF auth rows.
    assert_eq!(body["total_messages"], 157);
    assert_eq!(body["compliant_messages"], 147); // all but the 10-message reject
                                                 // The 10 failing messages were rejected, so nothing failing was delivered.
    assert_eq!(body["delivered_failures"], 0);
    assert_eq!(body["reporter_count"], 1);
    let domains = get(&app, "/api/domains").await.1;
    let listed = domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert!(body["last_report_at"].is_i64());
    assert_eq!(body["last_report_at"], listed["last_report_at"]);
    for gone in [
        "compliance_pct",
        "first_seen",
        "last_seen",
        "report_count",
        "unique_senders",
        "peer_max_reporters",
    ] {
        assert!(body.get(gone).is_none(), "{gone}");
    }

    let (status, body) = get(&app, "/api/domains/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    let empty_id = get(&app, "/api/domains").await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == "empty.test")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (status, body) = get(&app, &format!("/api/domains/{empty_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_messages"], 0);
    assert_eq!(body["delivered_failures"], 0);
    assert_eq!(body["reporter_count"], 0);
    assert!(body["policy"].is_null());
    assert!(body["last_report_at"].is_null());
}

/// The overview's policy is the DMARC record in DNS once a check is stored,
/// whatever the newest report says (it said `p=reject sp=quarantine`). It is
/// one object, `policy`: the overview has no `policy_p` of the list's shape.
#[tokio::test]
async fn overview_policy_comes_from_dns_once_checked() {
    let (_dir, pool, app) = seeded().await;
    let id = example_id(&app).await;
    let snapshot = dmarc_store::DnsSnapshot {
        domain: "example.com".into(),
        checked_at: 1_710_000_000,
        grade: "B".into(),
        score: 80,
        critical_count: 0,
        dmarc: dmarc_store::SnapshotDmarc {
            dmarc_record_count: 1,
            dmarc_record: Some("v=DMARC1; p=quarantine; pct=100; fo=1".into()),
            policy_p: Some("quarantine".into()),
            policy_pct: Some(100),
            ..Default::default()
        },
        report_json: "{}".into(),
        checker_version: 1,
    };
    assert!(dmarc_store::upsert_dns_snapshot(&pool, &snapshot)
        .await
        .unwrap());

    let (status, body) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["policy_source"], "dns");
    assert_eq!(body["policy"]["p"], "quarantine");
    assert_eq!(body["policy"]["pct"], 100);
    assert!(body["policy"]["sp"].is_null());
    assert_eq!(
        body["dmarc_record"],
        "v=DMARC1; p=quarantine; pct=100; fo=1"
    );
    assert!(body.get("policy_p").is_none());
    // The list row shows the same policy, flat.
    let domains = get(&app, "/api/domains").await.1;
    let listed = domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert_eq!(listed["policy_source"], "dns");
    assert_eq!(listed["policy"]["p"], "quarantine");
    assert_eq!(listed["policy"]["pct"], 100);
}

#[tokio::test]
async fn domain_senders_cross_domain_count_and_compliance() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    let (status, body) = get(&app, &format!("/api/senders?domain_id={id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items[0]["source_ip"], "192.0.2.1");
    assert_eq!(items[0]["total_messages"], 140);
    assert_eq!(items[0]["compliant_messages"], 140);
    // Also sends as other.org (outside this domain's scope).
    assert_eq!(items[0]["domain_count"], 2);
    assert_eq!(items[0]["compliance_pct"].as_f64().unwrap(), 100.0);
    // Aligned passes, not inflated by the two DKIM / two SPF auth rows: SPF aligned
    // on the 40-message record only, DKIM on both.
    assert_eq!(items[0]["spf_pass_messages"], 40);
    assert_eq!(items[0]["dkim_pass_messages"], 140);

    let fail_ip = items
        .iter()
        .find(|i| i["source_ip"] == "192.0.2.2")
        .unwrap();
    assert_eq!(fail_ip["compliant_messages"], 0);
    assert_eq!(fail_ip["compliance_pct"], 0.0);
    assert_eq!(fail_ip["domain_count"], 1);
}

#[tokio::test]
async fn failures_worst_first_and_outcome_filter() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    let (status, body) = get(&app, &format!("/api/domains/{id}/failures")).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty());
    // Worst-first: the 100-message SPF-only row outranks the 10-message both-fail.
    assert_eq!(items[0]["count"], 100);
    assert_eq!(items[0]["verdict"], "pass_dkim");
    assert_eq!(items[0]["source_ip"], "192.0.2.1");

    assert_eq!(items[0]["outcome"], "passed");

    let both = items.iter().find(|i| i["verdict"] == "fail").unwrap();
    assert_eq!(both["count"], 10);
    assert_eq!(both["source_ip"], "192.0.2.2");
    // Rejected: a forwarding reason does not make blocked mail "forwarded".
    assert_eq!(both["outcome"], "blocked");

    let dkim_only = items.iter().find(|i| i["verdict"] == "pass_spf").unwrap();
    assert_eq!(dkim_only["count"], 7);

    // Every outcome value, as (source IP, count) of the rows it lists.
    for (outcome, expected) in [
        ("all_failures", vec![("192.0.2.2", 10)]),
        ("blocked", vec![("192.0.2.2", 10)]),
        ("delivered", vec![]),
        ("forwarded", vec![]),
        ("dkim_only", vec![("192.0.2.1", 100)]),
        ("spf_only", vec![("192.0.2.3", 7)]),
        // A set: any of its kinds, as the tab's multi-select sends it.
        ("delivered,forwarded,blocked", vec![("192.0.2.2", 10)]),
        (
            "blocked,spf_only",
            vec![("192.0.2.2", 10), ("192.0.2.3", 7)],
        ),
        (
            "dkim_only,%20spf_only",
            vec![("192.0.2.1", 100), ("192.0.2.3", 7)],
        ),
        (
            "delivered,forwarded,blocked,dkim_only,spf_only",
            vec![("192.0.2.1", 100), ("192.0.2.2", 10), ("192.0.2.3", 7)],
        ),
    ] {
        let (status, body) = get(
            &app,
            &format!("/api/domains/{id}/failures?outcome={outcome}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{outcome}");
        let got: Vec<(&str, i64)> = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| {
                (
                    i["source_ip"].as_str().unwrap(),
                    i["count"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(got, expected, "outcome {outcome}");
    }

    // The overview counts each Show kind, as its list has it.
    let (_, overview) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(overview["dkim_only_messages"], 100);
    assert_eq!(overview["spf_only_messages"], 7);
    assert_eq!(overview["blocked_messages"], 10);

    // Worst outcome first: blocked before passed.
    let (_, body) = get(&app, &format!("/api/domains/{id}/failures?sort=outcome")).await;
    assert_eq!(body["items"][0]["outcome"], "blocked");

    // Test source_ip filter
    let (status, body) = get(
        &app,
        &format!("/api/domains/{id}/failures?source_ip=192.0.2.2"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["source_ip"], "192.0.2.2");
    assert_eq!(items[0]["count"], 10);

    // Test source_ip filter with no matches
    let (status, body) = get(
        &app,
        &format!("/api/domains/{id}/failures?source_ip=198.51.100.99"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert!(body["items"].as_array().unwrap().is_empty());

    let (status, body) = get(&app, &format!("/api/domains/{id}/failures?outcome=nope")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    let (status, _) = get(
        &app,
        &format!("/api/domains/{id}/failures?outcome=blocked,nope"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = get(&app, &format!("/api/domains/{id}/failures?limit=0")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = get(&app, &format!("/api/domains/{id}/failures?limit=501")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, body) = get(&app, "/api/domains/999999/failures").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    // Server-side sort and paging: smallest first, one row per page.
    let (status, body) = get(
        &app,
        &format!("/api/domains/{id}/failures?sort=count&limit=1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["count"], 7);
    let (_, body) = get(
        &app,
        &format!("/api/domains/{id}/failures?sort=count&limit=1&offset=2"),
    )
    .await;
    assert_eq!(body["items"][0]["count"], 100);
    let (status, _) = get(&app, &format!("/api/domains/{id}/failures?sort=nope")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The CSV behind "Export CSV" holds exactly the Failures tab's records.
#[tokio::test]
async fn failures_export_matches_the_tab() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    for query in [
        "",
        "&outcome=all_failures",
        "&outcome=delivered",
        "&outcome=forwarded",
        "&outcome=blocked",
        "&outcome=dkim_only",
        "&outcome=spf_only",
        "&outcome=blocked,dkim_only",
        "&source_ip=192.0.2.1",
        "&since=1705000000",
    ] {
        let (_, tab) = get(
            &app,
            &format!("/api/domains/{id}/failures?limit=500{query}"),
        )
        .await;
        let tab_messages: i64 = tab["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["count"].as_i64().unwrap())
            .sum();
        let (status, _, _, csv) = get_raw(
            &app,
            &format!("/api/export/auth-issues.csv?domain_id={id}{query}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{query}");
        let mut reader = csv::Reader::from_reader(csv.trim_start_matches('\u{feff}').as_bytes());
        let header = reader.headers().unwrap().clone();
        let count_col = header.iter().position(|h| h == "messages").unwrap();
        let csv_messages: i64 = reader
            .records()
            .map(|r| r.unwrap()[count_col].parse::<i64>().unwrap())
            .sum();
        assert_eq!(csv_messages, tab_messages, "filter {query:?}");
    }

    let (status, _, _, body) = get_raw(&app, "/api/export/auth-issues.csv?outcome=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("bad_request"), "{body}");
    let (status, _, _, _) = get_raw(&app, "/api/export/failures.csv").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the old failures export is gone"
    );
}

fn listed_domains(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn removing_a_domain_hides_it_but_keeps_its_reports() {
    let (_dir, pool, app) = seeded().await;
    let id = example_id(&app).await;
    let (_, before) = get(&app, "/api/statistics").await;
    assert_eq!(before["report_count"], 3);
    let (_, organizations) = get(&app, "/api/organizations").await;
    let acme = organization_by_name(&organizations, "Acme").clone();
    assert_eq!(acme["domain_count"], 2);
    assert_eq!(acme["report_count"], 2);

    let (status, _) = send(&app, "DELETE", &format!("/api/domains/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Hidden everywhere.
    let (status, _) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get(&app, &format!("/api/domains/{id}/failures")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = get(&app, "/api/domains").await;
    assert!(!listed_domains(&list).contains(&"example.com".to_owned()));
    let (_, list) = get(&app, &format!("/api/domains?domain_id={id}")).await;
    assert!(listed_domains(&list).is_empty());
    let (_, after) = get(&app, "/api/statistics").await;
    assert_eq!(
        after["report_count"], 1,
        "only other.org's report is counted"
    );
    assert_eq!(after["active_domains"], 1);
    let (_, scoped) = get(&app, &format!("/api/statistics?domain_id={id}")).await;
    assert_eq!(scoped["report_count"], 0);
    let (_, series) = get(&app, "/api/timeseries").await;
    let series_total: i64 = series["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["total_messages"].as_i64().unwrap())
        .sum();
    assert_eq!(series_total, 5, "only other.org's messages");
    let (_, reports) = get(&app, &format!("/api/reports?domain_id={id}")).await;
    assert_eq!(reports["total"], 0);
    let (_, reports) = get(&app, "/api/reports?q=example").await;
    assert_eq!(reports["total"], 0);
    let (_, senders) = get(&app, "/api/senders").await;
    assert_eq!(senders["total"], 1, "only 192.0.2.1 sends as other.org");
    // The shared sender keeps its other domain.
    let (status, sender) = get(&app, "/api/senders/192.0.2.1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sender["domain_count"], 1);
    let (status, _) = get(&app, "/api/senders/192.0.2.2").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, organizations) = get(&app, "/api/organizations").await;
    let acme = organization_by_name(&organizations, "Acme");
    assert_eq!(acme["domain_count"], 1, "empty.test is left");
    assert_eq!(acme["report_count"], 0);
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/domains/{id}"),
        Some(serde_json::json!({ "organization_id": acme["id"] })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!dmarc_store::list_monitored_domains(&pool)
        .await
        .unwrap()
        .contains(&"example.com".to_owned()));

    // Its reports are deleted.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE domain_id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);

    // Removing it again is a 404.
    let (status, body) = send(&app, "DELETE", &format!("/api/domains/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    // New reports for the removed domain are dropped.
    let new_report = || {
        report(
            "rpt-a3",
            "Example.COM",
            1_720_000_000,
            vec![rec(
                "192.0.2.9",
                3,
                Some("none"),
                Some("pass"),
                Some("pass"),
                vec![dkim("example.com", Some("s1"), "pass")],
                vec![spf("example.com", "pass", Some("mfrom"))],
                vec![],
            )],
        )
    };
    let result = ingest_report(&pool, &new_report()).await.unwrap();
    assert_eq!(result, dmarc_store::IngestResult::Ignored);
    let (_, after) = get(&app, "/api/statistics").await;
    assert_eq!(after["report_count"], 1);

    // Adding the name again un-ignores it, into the requested organization,
    // without history; reports arriving from then on are stored.
    let beta_id = organization_by_name(&organizations, "Beta")["id"]
        .as_i64()
        .unwrap();
    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": beta_id, "domain": "EXAMPLE.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["unignored"], true);
    assert_eq!(body["id"], id);
    assert_eq!(body["organization"]["id"], beta_id);
    assert_eq!(body["report_count"], 0);
    let result = ingest_report(&pool, &new_report()).await.unwrap();
    assert!(matches!(result, dmarc_store::IngestResult::Inserted { .. }));
    let (_, reports) = get(&app, &format!("/api/reports?domain_id={id}")).await;
    assert_eq!(reports["total"], 1);

    // A visible domain is still a conflict.
    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": beta_id, "domain": "example.com" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["organization_name"], "Beta");
}

#[tokio::test]
async fn removed_domain_is_ignored_in_the_mapping() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;
    let (status, _) = send(&app, "DELETE", &format!("/api/domains/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, mapping) = get(&app, "/api/settings/domain-mapping").await;
    assert!(
        mapping["text"]
            .as_str()
            .unwrap()
            .ends_with(",example.com,ignored\n"),
        "{mapping}"
    );
}

#[tokio::test]
async fn since_filters_overview() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    let (status, body) = get(&app, &format!("/api/domains/{id}?since=1705000000")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["reporter_count"], 1);
    assert_eq!(body["total_messages"], 107); // 100 + 7
}

#[tokio::test]
async fn create_domain_without_reports_and_reject_duplicates() {
    let (_dir, app) = seeded_app().await;
    let organization_id = get(&app, "/api/organizations").await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Acme")
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": organization_id, "domain": "Brand.New.example." }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["unignored"], false);
    assert_eq!(body["domain"], "brand.new.example");
    assert_eq!(body["organization"]["id"], organization_id);
    assert_eq!(body["report_count"], 0);
    assert_eq!(body["reporter_count"], 0);
    let id = body["id"].as_i64().unwrap();

    let (status, overview) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(overview["total_messages"], 0);
    assert_eq!(overview["reporter_count"], 0);

    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": organization_id, "domain": "brand.new.example" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");

    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": organization_id, "domain": "not a host" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");

    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": 999_999, "domain": "fresh.example" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}

fn organization_by_name<'a>(organizations: &'a Value, name: &str) -> &'a Value {
    organizations["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("organization {name} not listed"))
}

#[tokio::test]
async fn domain_names_are_unique_across_organizations() {
    let (_dir, app) = seeded_app().await;
    let (_, organizations) = get(&app, "/api/organizations").await;
    let beta_id = organization_by_name(&organizations, "Beta")["id"]
        .as_i64()
        .unwrap();

    // example.com lives in Acme; adding it to Beta (any spelling) is a 409 naming Acme.
    let (status, body) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": beta_id, "domain": " EXAMPLE.com. " }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");
    assert!(
        body["error"].as_str().unwrap().contains("Acme"),
        "conflict should name the owning organization: {body}"
    );
    assert_eq!(body["organization_name"], "Acme");
    assert!(body["error"].as_str().unwrap().contains("organization"));

    let (status, body) = get(&app, "/api/organizations/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "organization 999999 not found");
}

/// RFC 7489 §7.1 only applies across organizational domains, so the UI needs
/// each domain's organizational domain.
#[tokio::test]
async fn domains_expose_their_organizational_domain() {
    // A resolver nobody answers on, which fails at once.
    let (_dir, pool, _) = seeded().await;
    let state = AppState::new(pool).with_dns_resolver(Arc::new(TableResolver::failing()));
    let app = router_with_state(state);
    let (_, organizations) = get(&app, "/api/organizations").await;
    let beta_id = organization_by_name(&organizations, "Beta")["id"]
        .as_i64()
        .unwrap();
    let (status, created) = post_json(
        &app,
        "/api/domains",
        serde_json::json!({ "organization_id": beta_id, "domain": "shop.firma.co.uk" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["org_domain"], "firma.co.uk");
    let id = created["id"].as_i64().unwrap();

    let (_, list) = get(&app, "/api/domains").await;
    let item = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert_eq!(item["org_domain"], "firma.co.uk");
    let (_, overview) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(overview["org_domain"], "firma.co.uk");

    // Same organizational domain as the mailbox: no authorization record is
    // needed, so none is looked up and none is missing (the DMARC lookup
    // fails).
    let (status, _) = put_json(
        &app,
        "/api/settings/reporting",
        serde_json::json!({ "report_address": "dmarc@firma.co.uk", "reporting_mode": "direct" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, setup) = get(&app, &format!("/api/reporting-setup?domain_id={id}")).await;
    assert_eq!(status, StatusCode::OK);
    // One domain asked for: the mailbox domain's authorization is left out.
    assert_eq!(setup["domains"].as_array().unwrap().len(), 1);
    assert_eq!(setup["mailbox_authorization"], Value::Null);
    assert_eq!(setup["domains"][0]["mailbox_auth_missing"], false);
    assert_eq!(setup["domains"][0]["state"], "check_failed");
}

#[tokio::test]
async fn default_organization_is_listed_renamable_and_undeletable() {
    let (_dir, app) = seeded_app().await;
    let (_, organizations) = get(&app, "/api/organizations").await;
    let items = organizations["items"].as_array().unwrap();
    assert_eq!(items[0]["name"], "Default", "default is listed first");
    assert_eq!(items[0]["is_default"], true);
    assert_eq!(items.iter().filter(|c| c["is_default"] == true).count(), 1);
    let default_id = items[0]["id"].as_i64().unwrap();

    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/organizations/{default_id}"),
        Some(serde_json::json!({ "name": "  Contoso IT  " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Contoso IT");
    assert_eq!(body["is_default"], true);

    let (_, detail) = get(&app, &format!("/api/organizations/{default_id}")).await;
    assert_eq!(detail["is_default"], true);

    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/organizations/{default_id}"),
        Some(serde_json::json!({ "name": "Acme" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/organizations/{default_id}"),
        Some(serde_json::json!({ "name": "  " })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, body) = send(
        &app,
        "DELETE",
        &format!("/api/organizations/{default_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");
}

#[tokio::test]
async fn deleting_a_organization_moves_its_domains_to_the_default() {
    let (_dir, app) = seeded_app().await;
    let (_, organizations) = get(&app, "/api/organizations").await;
    let acme = organization_by_name(&organizations, "Acme");
    let acme_id = acme["id"].as_i64().unwrap();
    let acme_reports = acme["report_count"].as_i64().unwrap();
    assert!(acme_reports > 0);
    let default_id = organizations["items"][0]["id"].as_i64().unwrap();

    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/organizations/{acme_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, organizations) = get(&app, "/api/organizations").await;
    assert!(organizations["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["id"] != acme_id));
    let default = &organizations["items"][0];
    assert_eq!(default["id"], default_id);
    assert_eq!(default["report_count"].as_i64().unwrap(), acme_reports);

    let id = example_id(&app).await;
    let (_, overview) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(overview["organization"]["id"], default_id);

    let (status, _) = send(&app, "DELETE", "/api/organizations/999999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Forwarding fixture: a gmail.com forward (SPF passes for gmail.com, DKIM for
/// example.com) and a relay.example re-sign (both checks pass, neither aligned).
async fn forwarding_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = init(&dir.path().join("test.db")).await.expect("init");
    seed_organizations(&pool).await;
    let arc = |t: &str| PolicyOverride {
        reason_type: Some(t.into()),
        comment: Some("arc=pass".into()),
    };
    ingest_report(
        &pool,
        &report(
            "rpt-fwd",
            "example.com",
            1_700_000_000,
            vec![
                rec(
                    "209.85.220.41",
                    3,
                    Some("none"),
                    Some("pass"),
                    Some("fail"),
                    vec![dkim("example.com", Some("selector1"), "pass")],
                    vec![spf("gmail.com", "pass", Some("mfrom"))],
                    vec![arc("local_policy")],
                ),
                rec(
                    "198.51.100.7",
                    2,
                    Some("none"),
                    Some("fail"),
                    Some("fail"),
                    vec![dkim("relay.example", Some("google"), "pass")],
                    vec![spf("relay.example", "pass", Some("mfrom"))],
                    vec![arc("local_policy")],
                ),
            ],
        ),
    )
    .await
    .expect("ingest");
    (dir, router(pool))
}

/// Per-check results and the alignment stored at ingest, not name guesses.
#[tokio::test]
async fn failures_carry_per_check_alignment_and_overrides() {
    let (_dir, app) = forwarding_app().await;
    let id = example_id(&app).await;
    let (status, body) = get(&app, &format!("/api/domains/{id}/failures")).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);

    let gmail = items
        .iter()
        .find(|i| i["source_ip"] == "209.85.220.41")
        .unwrap();
    assert_eq!(gmail["verdict"], "pass_dkim");
    assert_eq!(
        gmail["spf_auth"],
        serde_json::json!([{"domain": "gmail.com", "scope": "mfrom", "result": "pass", "aligned": false,
                            "state": "pass_other", "status": "neutral"}])
    );
    assert_eq!(
        gmail["dkim_auth"],
        serde_json::json!([{"domain": "example.com", "selector": "selector1", "result": "pass", "aligned": true,
                            "state": "pass", "status": "pass"}])
    );
    assert_eq!(
        gmail["overrides"],
        serde_json::json!([{"type": "local_policy", "comment": "arc=pass"}])
    );
    assert!(gmail.get("spf_domain").is_none());
    assert!(gmail.get("override_reasons").is_none());

    let relay = items
        .iter()
        .find(|i| i["source_ip"] == "198.51.100.7")
        .unwrap();
    assert_eq!(relay["verdict"], "fail");
    assert_eq!(relay["spf_auth"][0]["domain"], "relay.example");
    assert_eq!(relay["spf_auth"][0]["result"], "pass");
    assert_eq!(relay["spf_auth"][0]["aligned"], false);
    assert_eq!(
        relay["dkim_auth"],
        serde_json::json!([{"domain": "relay.example", "selector": "google", "result": "pass", "aligned": false,
                            "state": "pass_other", "status": "neutral"}])
    );
    assert_eq!(relay["overrides"][0]["type"], "local_policy");
    assert_eq!(relay["overrides"][0]["comment"], "arc=pass");

    // Sorting by the first listed check still works.
    for sort in ["spf_domain", "-dkim_selector"] {
        let (status, body) = get(&app, &format!("/api/domains/{id}/failures?sort={sort}")).await;
        assert_eq!(status, StatusCode::OK, "{sort}");
        assert_eq!(body["total"], 2);
    }
    let (_, body) = get(&app, &format!("/api/domains/{id}/failures?sort=spf_domain")).await;
    assert_eq!(body["items"][0]["spf_auth"][0]["domain"], "gmail.com");
}

/// Report detail records list every check with its stored alignment.
#[tokio::test]
async fn report_records_carry_per_check_alignment() {
    let (_dir, app) = forwarding_app().await;
    let (_, list) = get(&app, "/api/reports").await;
    let report_id = list["items"][0]["id"].as_i64().unwrap();
    let (status, body) = get(&app, &format!("/api/reports/{report_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let records = body["records"].as_array().unwrap();
    let gmail = records
        .iter()
        .find(|r| r["source_ip"] == "209.85.220.41")
        .unwrap();
    assert_eq!(gmail["verdict"], "pass_dkim");
    assert_eq!(gmail["outcome"], "passed");
    assert_eq!(gmail["spf_auth"][0]["domain"], "gmail.com");
    assert_eq!(gmail["spf_auth"][0]["aligned"], false);
    assert_eq!(gmail["dkim_auth"][0]["selector"], "selector1");
    assert_eq!(gmail["dkim_auth"][0]["result"], "pass");
    assert_eq!(gmail["dkim_auth"][0]["aligned"], true);

    let relay = records
        .iter()
        .find(|r| r["source_ip"] == "198.51.100.7")
        .unwrap();
    assert_eq!(relay["verdict"], "fail");
    assert_eq!(
        relay["dkim_auth"],
        serde_json::json!([{"domain": "relay.example", "selector": "google", "result": "pass", "aligned": false,
                            "state": "pass_other", "status": "neutral"}])
    );
    assert_eq!(relay["spf_auth"][0]["aligned"], false);
}

#[tokio::test]
async fn parking_a_domain_and_dkim_coverage_fields() {
    let (_dir, app) = seeded_app().await;
    let id = example_id(&app).await;

    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/domains/{id}"),
        Some(serde_json::json!({ "parked": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["parked"], true);
    let (_, overview) = get(&app, &format!("/api/domains/{id}")).await;
    assert_eq!(overview["parked"], true);
    // Passing messages without an aligned DKIM pass, never more than the passing ones.
    let no_dkim = overview["no_aligned_dkim_messages"].as_i64().unwrap();
    assert!(no_dkim >= 0 && no_dkim <= overview["compliant_messages"].as_i64().unwrap());

    let (status, body) = send(
        &app,
        "PATCH",
        &format!("/api/domains/{id}"),
        Some(serde_json::json!({ "parked": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["parked"], false);

    // An empty update is rejected; an unknown domain is 404.
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/domains/{id}"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(
        &app,
        "PATCH",
        "/api/domains/99999",
        Some(serde_json::json!({ "parked": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
