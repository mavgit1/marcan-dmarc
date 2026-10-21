//! Integration tests for the REST API against a temp SQLite DB.

use axum::http::StatusCode;
use dmarc_server::router;
use dmarc_store::{
    get_default_organization, get_or_create_domain, get_or_create_organization, ingest_report, init,
};
use serde_json::Value;
mod common;
use common::{get, post_json, rec, report, seed_organizations, send};

async fn seeded_app() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let pool = init(&db_path).await.expect("init");
    seed_organizations(&pool).await;

    // Organization A / example.com
    ingest_report(
        &pool,
        &report(
            "rpt-a1",
            "example.com",
            1_700_000_000,
            vec![
                rec("192.0.2.1", 40, "pass", "pass"),
                rec("192.0.2.2", 10, "fail", "fail"),
            ],
        ),
    )
    .await
    .expect("ingest a1");

    ingest_report(
        &pool,
        &report(
            "rpt-a2",
            "example.com",
            1_710_000_000,
            vec![rec("192.0.2.1", 100, "pass", "fail")],
        ),
    )
    .await
    .expect("ingest a2");
    // Its policy is its DNS record's (the reports say the same).
    common::store_dns(
        &pool,
        "example.com",
        "v=DMARC1; p=reject",
        "v=spf1 ip4:192.0.2.0/24 -all",
        1_710_000_000,
    )
    .await;

    // Organization B / other.org
    ingest_report(
        &pool,
        &report(
            "rpt-b1",
            "other.org",
            1_710_000_000,
            vec![rec("198.51.100.1", 5, "fail", "fail")],
        ),
    )
    .await
    .expect("ingest b1");

    // Ensure empty organization still listed
    get_or_create_organization(&pool, "EmptyCo")
        .await
        .expect("empty co");

    (dir, router(pool))
}

#[tokio::test]
async fn statistics_global_and_filters() {
    let (_dir, app) = seeded_app().await;

    let (status, body) = get(&app, "/api/statistics").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_messages"], 155); // 50 + 100 + 5
    assert_eq!(body["compliant_messages"], 140); // 40 + 100
    assert_eq!(body["unique_senders"], 3);
    assert_eq!(body["active_domains"], 2);
    assert_eq!(body["report_count"], 3);

    // since timestamp excludes older report
    let (status, body) = get(&app, "/api/statistics?since=1705000000").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["report_count"], 2);
    assert_eq!(body["total_messages"], 105);

    // organization filter
    let organizations = get(&app, "/api/organizations").await.1;
    let acme_id = organizations["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Acme")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (status, body) = get(&app, &format!("/api/statistics?organization_id={acme_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["report_count"], 2);
    assert_eq!(body["total_messages"], 150);
    assert_eq!(body["active_domains"], 1);
}

#[tokio::test]
async fn since_bad_request() {
    let (_dir, app) = seeded_app().await;
    let (status, body) = get(&app, "/api/statistics?since=yesterday").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    assert!(body["error"].as_str().unwrap().contains("since"));
}

#[tokio::test]
async fn organizations_and_domains() {
    let (_dir, app) = seeded_app().await;

    let (status, body) = get(&app, "/api/organizations").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert!(items.len() >= 3);
    let acme = items.iter().find(|c| c["name"] == "Acme").unwrap();
    assert_eq!(acme["domain_count"], 1);
    assert_eq!(acme["report_count"], 2);
    let empty = items.iter().find(|c| c["name"] == "EmptyCo").unwrap();
    assert_eq!(empty["domain_count"], 0);
    assert_eq!(empty["report_count"], 0);

    let (status, body) = get(&app, "/api/domains").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);
    let example = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == "example.com")
        .unwrap();
    assert_eq!(example["total_messages"], 150);
    assert_eq!(example["compliant_messages"], 140);
    // Only the 10 failing messages (disposition none) were delivered.
    assert_eq!(example["delivered_failures"], 10);
    assert_eq!(example["reporter_count"], 1);
    assert_eq!(example["policy"]["p"], "reject");

    let beta_id = get(&app, "/api/organizations").await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Beta")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (status, body) = get(&app, &format!("/api/domains?organization_id={beta_id}")).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["domain"], "other.org");
    assert_eq!(items[0]["report_count"], 1);
}

fn domain_names(body: &Value) -> Vec<&str> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["domain"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn domains_list_filters_sorts_and_pages() {
    let (_dir, app) = seeded_app().await;

    let (status, body) = get(&app, "/api/domains?limit=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    assert_eq!(body["limit"], 1);
    assert_eq!(body["offset"], 0);
    assert_eq!(domain_names(&body), ["example.com"]);
    // A rank is a place in the whole scope, never in one page.
    assert!(body["items"][0]["attention_rank"].is_null());
    let (_, body) = get(&app, "/api/domains?limit=1&offset=1").await;
    assert_eq!(domain_names(&body), ["other.org"]);

    let (_, body) = get(&app, "/api/domains?sort=-total_messages").await;
    assert_eq!(domain_names(&body), ["example.com", "other.org"]);
    let (_, body) = get(&app, "/api/domains?sort=total_messages").await;
    assert_eq!(domain_names(&body), ["other.org", "example.com"]);
    // example.com delivered 140 of 150 authenticated, other.org none of 5.
    let (_, body) = get(&app, "/api/domains?sort=delivered_pct").await;
    assert_eq!(domain_names(&body), ["other.org", "example.com"]);
    let (_, body) = get(&app, "/api/domains?sort=-organization_name").await;
    assert_eq!(domain_names(&body), ["other.org", "example.com"]);
    let (status, _) = get(&app, "/api/domains?sort=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The filter matches the domain or its organization's name, in any case.
    let (_, body) = get(&app, "/api/domains?q=EXAMPLE").await;
    assert_eq!(body["total"], 1);
    assert_eq!(domain_names(&body), ["example.com"]);
    let (_, body) = get(&app, "/api/domains?q=beta").await;
    assert_eq!(domain_names(&body), ["other.org"]);
    let (_, body) = get(&app, "/api/domains?q=%25").await;
    assert_eq!(body["total"], 0);

    // The stat cards count the whole scope, whatever the list's filter.
    let (_, stats) = get(&app, "/api/statistics").await;
    assert_eq!(stats["domain_count"], 2);
}

#[tokio::test]
async fn attention_domains_are_ranked_over_the_whole_scope() {
    let (_dir, app) = seeded_app().await;

    let (status, body) = get(&app, "/api/domains/attention").await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert!(!items.is_empty());
    let ranks: Vec<i64> = items
        .iter()
        .map(|d| d["attention_rank"].as_i64().unwrap())
        .collect();
    let mut sorted = ranks.clone();
    sorted.sort_unstable();
    assert_eq!(ranks, sorted);
    assert!(items
        .iter()
        .all(|d| !d["assessment"]["issues"].as_array().unwrap().is_empty()));
    assert_eq!(body["total"], items.len());

    // A limit keeps the first ranks and still counts them all.
    let (_, first) = get(&app, "/api/domains/attention?limit=1").await;
    assert_eq!(first["total"], body["total"]);
    assert_eq!(first["limit"], 1);
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["items"][0]["domain"], items[0]["domain"]);
    // The Domains page's attention filter pages on: the next rank, and its
    // rank in the whole scope.
    if items.len() > 1 {
        let (_, second) = get(&app, "/api/domains/attention?limit=1&offset=1").await;
        assert_eq!(second["total"], body["total"]);
        assert_eq!(second["offset"], 1);
        assert_eq!(second["items"][0]["domain"], items[1]["domain"]);
        assert_eq!(
            second["items"][0]["attention_rank"],
            items[1]["attention_rank"]
        );
    }
    // The search narrows the ranked list without re-ranking it.
    let name = items[0]["domain"].as_str().unwrap().to_uppercase();
    let (_, found) = get(&app, &format!("/api/domains/attention?q={name}")).await;
    assert_eq!(found["total"], 1);
    assert_eq!(
        found["items"][0]["attention_rank"],
        items[0]["attention_rank"]
    );
    let (_, none) = get(&app, "/api/domains/attention?q=no-such-domain").await;
    assert_eq!(none["total"], 0);
    let (status, _) = get(&app, "/api/domains/attention?limit=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reports_list_pagination_and_detail() {
    let (_dir, app) = seeded_app().await;

    let (status, body) = get(&app, "/api/reports?limit=1&offset=0").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(body["limit"], 1);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    // Newest first
    assert_eq!(body["items"][0]["report_id"], "rpt-b1");
    // All five messages failed auth and were delivered (disposition none).
    assert_eq!(body["items"][0]["delivered_failures"], 5);

    let (status, body) = get(&app, "/api/reports?limit=1&offset=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    let id = get(&app, "/api/reports?limit=50").await.1["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["report_id"] == "rpt-a1")
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    let (status, body) = get(&app, &format!("/api/reports/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["report_id"], "rpt-a1");
    assert_eq!(body["domain"], "example.com");
    // 10 failing messages still delivered; blocked spoofs are not counted here.
    assert_eq!(body["delivered_failures"], 10);
    // Delivered compliance is a share of every delivered message, the
    // passed ones and the delivered failures.
    assert_eq!(
        body["delivered_messages"],
        body["compliant_messages"].as_i64().unwrap() + 10
    );
    assert_eq!(body["records"].as_array().unwrap().len(), 2);
    assert_eq!(body["records"][0]["source_ip"], "192.0.2.1");
    // Each record carries its outcome's place worst first, to sort by.
    for record in body["records"].as_array().unwrap() {
        let rank = match record["outcome"].as_str().unwrap() {
            "delivered" => 0,
            "forwarded" => 1,
            "blocked" => 2,
            _ => 3,
        };
        assert_eq!(record["outcome_rank"], rank, "{record}");
    }

    let (status, body) = get(&app, "/api/reports/999999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");
}

#[tokio::test]
async fn senders_sorted_and_paged() {
    let (_dir, app) = seeded_app().await;

    // Busiest first by default.
    let (status, body) = get(&app, "/api/senders?limit=10").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(body["limit"], 10);
    assert_eq!(body["offset"], 0);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items[0]["source_ip"], "192.0.2.1");
    assert_eq!(items[0]["total_messages"], 140); // 40 + 100
    assert_eq!(items[0]["compliant_messages"], 140);
    assert_eq!(items[0]["failed_messages"], 0);
    assert_eq!(items[0]["compliance_pct"].as_f64().unwrap(), 100.0);
    assert_eq!(items[0]["report_count"], 2);
    assert_eq!(items[0]["domain_count"], 1);

    let fail_ip = items
        .iter()
        .find(|i| i["source_ip"] == "192.0.2.2")
        .unwrap();
    assert_eq!(fail_ip["failed_messages"], 10);
    assert_eq!(fail_ip["compliant_messages"], 0);

    // Most failed mail first, one per page, with the total of the whole scope.
    let (status, body) = get(&app, "/api/senders?sort=-failed_messages&limit=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 3);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["source_ip"], "192.0.2.2");
    let (_, body) = get(&app, "/api/senders?sort=-failed_messages&limit=1&offset=1").await;
    assert_eq!(body["items"][0]["source_ip"], "198.51.100.1");
    assert_eq!(body["offset"], 1);

    // Ascending by address, and scoped to one domain.
    let (_, body) = get(&app, "/api/senders?sort=source_ip").await;
    let ips: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["source_ip"].as_str().unwrap())
        .collect();
    assert_eq!(ips, ["192.0.2.1", "192.0.2.2", "198.51.100.1"]);

    let (status, body) = get(&app, "/api/senders?sort=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
}

#[tokio::test]
async fn reports_search_and_sort_cover_every_page() {
    let (_dir, app) = seeded_app().await;

    // The search runs on the server, so it finds reports beyond the first page.
    let (status, body) = get(&app, "/api/reports?limit=1&q=OTHER.ORG").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["domain"], "other.org");
    let (_, body) = get(&app, "/api/reports?q=no-such-thing").await;
    assert_eq!(body["total"], 0);
    // LIKE wildcards in the query are literal.
    let (_, body) = get(&app, "/api/reports?q=%25").await;
    assert_eq!(body["total"], 0);

    // Sorting runs before paging: the smallest report comes first on page one.
    let (_, body) = get(&app, "/api/reports?limit=1&sort=total_messages").await;
    assert_eq!(body["items"][0]["total_messages"], 5);
    let (_, body) = get(&app, "/api/reports?limit=1&sort=-total_messages").await;
    assert_eq!(body["items"][0]["total_messages"], 100);
    let (_, body) = get(&app, "/api/reports?sort=domain").await;
    assert_eq!(body["items"][0]["domain"], "example.com");
    let (_, body) = get(&app, "/api/reports?sort=-delivered_pct").await;
    assert_eq!(body["total"], 3);

    let (status, _) = get(&app, "/api/reports?sort=raw_report").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reports_bad_limit() {
    let (_dir, app) = seeded_app().await;
    let (status, body) = get(&app, "/api/reports?limit=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
}

#[tokio::test]
async fn domain_id_filter_on_reports() {
    let (_dir, app) = seeded_app().await;
    let domains = get(&app, "/api/domains").await.1;
    let example_id = domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == "example.com")
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    let (status, body) = get(&app, &format!("/api/reports?domain_id={example_id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 2);
    for item in body["items"].as_array().unwrap() {
        assert_eq!(item["domain"], "example.com");
    }
}

/// Smoke: store init + router construction works (IMAP absence is a main.rs concern).
#[tokio::test]
async fn router_builds_without_imap() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("smoke.db");
    let pool = init(&db_path).await.unwrap();
    let _organization = get_default_organization(&pool).await.unwrap();
    let _domain = get_or_create_domain(&pool, 1, "smoke.test").await.unwrap();
    let app = router(pool);
    let (status, _) = get(&app, "/api/organizations").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn create_organization_success_and_validation() {
    let (_dir, app) = seeded_app().await;

    // Successful creation with whitespace trimming
    let (status, body) = post_json(
        &app,
        "/api/organizations",
        serde_json::json!({ "name": "  OmniCorp  " }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["name"], "OmniCorp");
    assert_eq!(body["domain_count"], 0);
    assert_eq!(body["report_count"], 0);
    let created_id = body["id"].as_i64().unwrap();
    assert!(created_id > 0);

    // Verify it appears in GET /api/organizations
    let (status, list) = get(&app, "/api/organizations").await;
    assert_eq!(status, StatusCode::OK);
    let items = list["items"].as_array().unwrap();
    let found = items.iter().find(|c| c["name"] == "OmniCorp").unwrap();
    assert_eq!(found["id"], created_id);

    // Empty name rejected
    let (status, err) = post_json(
        &app,
        "/api/organizations",
        serde_json::json!({ "name": "   " }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "unprocessable");

    // A taken name is a conflict, not a silent get-or-create.
    let (status, err) = post_json(
        &app,
        "/api/organizations",
        serde_json::json!({ "name": "OmniCorp" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(err["code"], "conflict");

    // So is the same name in other case: the domain mapping could not tell them apart.
    let (status, _) = post_json(
        &app,
        "/api/organizations",
        serde_json::json!({ "name": "omnicorp" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = send(
        &app,
        "PATCH",
        &format!("/api/organizations/{created_id}"),
        Some(serde_json::json!({ "name": "OMNICORP" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an organization may change the case of its own name"
    );
}

/// Verdicts come from the server; the UI only words them.
#[tokio::test]
async fn domains_organizations_and_health_carry_their_verdicts() {
    let (_dir, app) = seeded_app().await;

    let (status, domains) = get(&app, "/api/domains").await;
    assert_eq!(status, StatusCode::OK);
    let example = domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == "example.com")
        .unwrap();
    let assessment = &example["assessment"];
    // 140 of 150 delivered messages passed: 93.3%, below the 95% line.
    assert_eq!(example["delivered_compliance"]["status"], "fail");
    assert_eq!(example["health"]["state"], "critical");
    assert_eq!(assessment["enforcement"]["state"], "reject");
    assert_eq!(assessment["issues"][0]["kind"], "delivered_failures");
    assert_eq!(assessment["issues"][0]["severity"], "critical");

    let (_, organizations) = get(&app, "/api/organizations").await;
    let by_name = |name: &str| {
        organizations["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap()
            .clone()
    };
    let acme = by_name("Acme");
    assert_eq!(acme["total_messages"], 150);
    assert_eq!(acme["attention"]["domains_needing_attention"], 1);
    assert_eq!(acme["attention"]["status"], "fail");
    let empty = by_name("EmptyCo");
    assert_eq!(empty["attention"]["domains_needing_attention"], 0);
    assert_eq!(empty["attention"]["status"], "neutral");

    let (_, health) = get(&app, "/api/system/health").await;
    assert_eq!(health["pipeline"]["mailbox"], "not_configured");
    assert_eq!(health["pipeline"]["headline"], "all_well");

    let (_, meta) = get(&app, "/api/meta").await;
    assert_eq!(meta["rate_pass_min_pct"], 99.0);
    assert_eq!(meta["severity_points"][0]["severity"], "critical");
    assert_eq!(meta["grade_bands"][0]["grade"], "A");
    // The ranges the settings forms check before saving, the server's own.
    assert_eq!(meta["client_report_day_min"], 1);
    assert_eq!(meta["client_report_day_max"], 28);
    assert_eq!(meta["operator_name_max_chars"], 80);
    assert_eq!(meta["report_retention_months_min"], 1);
    assert_eq!(meta["report_retention_months_max"], 120);
}

/// The scope filter's list: ids and names, default organization first, no removed domains.
#[tokio::test]
async fn scope_lists_organizations_and_monitored_domains() {
    let (_dir, app) = seeded_app().await;
    let (status, scope) = get(&app, "/api/scope").await;
    assert_eq!(status, StatusCode::OK);
    let organizations = scope["organizations"].as_array().unwrap();
    assert_eq!(organizations[0]["name"], "Default");
    let domains = scope["domains"].as_array().unwrap();
    let example = domains
        .iter()
        .find(|d| d["domain"] == "example.com")
        .unwrap();
    let id = example["id"].as_i64().unwrap();
    assert!(example["organization_id"].is_i64());
    assert!(example.get("report_count").is_none());
    assert_eq!(example["has_reports"], true);

    let (status, _) = send(
        &app,
        "DELETE",
        &format!("/api/domains/{id}"),
        Some(Value::Null),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, scope) = get(&app, "/api/scope").await;
    assert!(!scope["domains"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["id"] == id));
}

#[tokio::test]
async fn guide_counts_the_domains_at_each_policy() {
    let t = common::TestApp::new().await;
    let organization = get_default_organization(&t.pool).await.unwrap();
    let spf = "v=spf1 -all";
    for (domain, dmarc) in [
        ("none-a.test", Some("v=DMARC1; p=none")),
        (
            "none-b.test",
            Some("v=DMARC1; p=none; rua=mailto:r@none-b.test"),
        ),
        ("quarantine.test", Some("v=DMARC1; p=quarantine")),
        ("reject.test", Some("v=DMARC1; p=reject")),
        ("unchecked.test", None),
    ] {
        get_or_create_domain(&t.pool, organization.id, domain)
            .await
            .unwrap();
        if let Some(dmarc) = dmarc {
            common::store_dns(&t.pool, domain, dmarc, spf, 1_710_000_000).await;
        }
    }
    let (status, guide) = get(&t.app, "/api/guide").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        guide["policies"],
        serde_json::json!({ "monitor": 2, "quarantine": 1, "reject": 1, "unknown": 1 })
    );
}
