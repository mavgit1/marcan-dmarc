//! Integration tests for the bulk domain mapping API.

use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::SqlitePool;
mod common;
use common::{send, TestApp};

/// Store `count` bare reports for `domain`.
async fn add_reports(pool: &SqlitePool, domain: &str, count: usize) {
    for i in 0..count {
        sqlx::query(
            "INSERT INTO reports (report_id, org_name, email, domain_id, date_begin, date_end, \
             total_messages, compliant_messages) \
             SELECT ?, 'rep', 'r@example', id, 0, 1, 1, 1 FROM domains WHERE domain = ?",
        )
        .bind(format!("{domain}-{i}"))
        .bind(domain)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn mapping_text(app: &axum::Router) -> String {
    let (status, body) = send(app, "GET", "/api/settings/domain-mapping", None).await;
    assert_eq!(status, StatusCode::OK);
    body["text"].as_str().unwrap().to_owned()
}

async fn preview(app: &axum::Router, text: &str) -> Value {
    let (status, body) = send(
        app,
        "POST",
        "/api/settings/domain-mapping/preview",
        Some(json!({ "text": text })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn apply(app: &axum::Router, text: &str) -> (StatusCode, Value) {
    send(
        app,
        "PUT",
        "/api/settings/domain-mapping",
        Some(json!({ "text": text })),
    )
    .await
}

async fn create_organization(app: &axum::Router, name: &str) -> i64 {
    let (status, body) = send(
        app,
        "POST",
        "/api/organizations",
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_i64().unwrap()
}

async fn create_domain(app: &axum::Router, organization_id: i64, domain: &str) -> i64 {
    let (status, body) = send(
        app,
        "POST",
        "/api/domains",
        Some(json!({ "organization_id": organization_id, "domain": domain })),
    )
    .await;
    assert!(status.is_success(), "{body}");
    body["id"].as_i64().unwrap()
}

/// Acme: a.com, b.com · Beta: c.com · Default: d.com · ignored: gone.com (was in Beta).
async fn seed(app: &axum::Router) -> (i64, i64) {
    let acme = create_organization(app, "Acme").await;
    let beta = create_organization(app, "Beta").await;
    create_domain(app, acme, "a.com").await;
    create_domain(app, acme, "b.com").await;
    create_domain(app, beta, "c.com").await;
    create_domain(app, 1, "d.com").await;
    let gone = create_domain(app, beta, "gone.com").await;
    let (status, _) = send(app, "DELETE", &format!("/api/domains/{gone}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    (acme, beta)
}

fn errors(body: &Value) -> Vec<(u64, String)> {
    body["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["line"].as_u64().unwrap(),
                e["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn get_lists_current_mapping_sorted_with_ignored_domains_last() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    assert_eq!(
        mapping_text(&app).await,
        "Acme,a.com\nAcme,b.com\nBeta,c.com\nDefault,d.com\nBeta,gone.com,ignored\n"
    );
}

#[tokio::test]
async fn current_mapping_previews_as_no_change() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let text = mapping_text(&app).await;
    let body = preview(&app, &text).await;
    assert_eq!(body["unchanged_count"], 5);
    assert_eq!(
        body["changes"],
        json!({ "move": [], "add": [], "ignore": [], "unignore": [], "create_orgs": [], "park": [], "unpark": [] })
    );
    assert_eq!(body["errors"], json!([]));
}

#[tokio::test]
async fn parse_errors_carry_line_numbers() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    let text = "early.com\nAcme, ok.com\nAcme, not a domain\nAcme, dup.com\nAcme,\nAcme, x.com, maybe\nBeta, DUP.com\na,b,c,d\n";
    let body = preview(&app, text).await;
    let errs = errors(&body);
    let lines: Vec<u64> = errs.iter().map(|(l, _)| *l).collect();
    assert_eq!(lines, [1, 3, 5, 6, 7, 8], "{errs:?}");
    assert!(errs[0].1.contains("got 1 column"));
    assert!(errs[1].1.contains("not a valid domain"));
    assert!(errs[2].1.contains("domain is empty"));
    assert!(errs[3].1.contains("`parked`, `ignored` or empty"));
    assert!(errs[4]
        .1
        .contains("dup.com is already listed under Acme (line 4)"));
    assert!(errs[5].1.contains("4 columns"));
    // Valid lines are still previewed.
    let added: Vec<&str> = body["changes"]["add"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["domain"].as_str().unwrap())
        .collect();
    assert_eq!(added, ["dup.com", "ok.com"]);
}

#[tokio::test]
async fn preview_has_no_side_effects() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let before = mapping_text(&app).await;
    let body = preview(&app, "Beta,a.com\nNew Org,new.com\nNew Org,gone.com\n").await;
    assert_eq!(
        body["changes"],
        json!({
            "move": [{ "domain": "a.com", "from": "Acme", "to": "Beta" }],
            "add": [{ "domain": "new.com", "org": "New Org" }],
            "ignore": [],
            "unignore": [{ "domain": "gone.com", "org": "New Org" }],
            "create_orgs": ["New Org"],
            "park": [],
            "unpark": [],
        })
    );
    assert_eq!(mapping_text(&app).await, before);
    let (_, organizations) = send(&app, "GET", "/api/organizations", None).await;
    assert_eq!(organizations["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn apply_moves_adds_creates_and_unignores_leaving_unlisted_alone() {
    let t = TestApp::new().await;
    let (_pool, app, imap) = (t.pool.clone(), t.app.clone(), t.state.imap.clone());
    seed(&app).await;
    let text = "organization,domain\n# bulk\n Beta, A.com. \nBeta,c.com\nNew Org,new.com\nNew Org,gone.com\n";
    let (status, body) = apply(&app, text).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changes"]["move"].as_array().unwrap().len(), 1);
    assert_eq!(body["changes"]["add"].as_array().unwrap().len(), 1);
    assert_eq!(body["changes"]["unignore"].as_array().unwrap().len(), 1);
    assert_eq!(body["changes"]["create_orgs"], json!(["New Org"]));
    assert_eq!(body["unchanged_count"], 1);
    assert_eq!(body["errors"], json!([]));

    // Monitoring gone.com again re-imports the mailbox for its history.
    assert!(imap.reimport_requested());

    // b.com (Acme) and d.com (Default) were not listed and stay put; Acme keeps existing.
    assert_eq!(
        mapping_text(&app).await,
        "Acme,b.com\nBeta,a.com\nBeta,c.com\nDefault,d.com\nNew Org,gone.com\nNew Org,new.com\n"
    );

    // Applying the same text again changes nothing.
    let again = preview(&app, text).await;
    assert_eq!(again["unchanged_count"], 4);
    assert_eq!(again["changes"]["create_orgs"], json!([]));
}

#[tokio::test]
async fn ignored_deletes_reports_and_drops_the_parked_flag() {
    let t = TestApp::new().await;
    let (pool, app, imap) = (t.pool.clone(), t.app.clone(), t.state.imap.clone());
    seed(&app).await;
    add_reports(&pool, "a.com", 3).await;
    apply(&app, "Acme,b.com,parked\n").await;

    // The preview counts the reports that ignoring deletes; parking is moot.
    let text = "Acme,a.com,ignored\nAcme,b.com,ignored\nSpam,spoof.example,ignored\n";
    let body = preview(&app, text).await;
    assert_eq!(
        body["changes"],
        json!({
            "move": [],
            "add": [],
            "ignore": [
                { "domain": "a.com", "reports": 3 },
                { "domain": "b.com", "reports": 0 },
                { "domain": "spoof.example", "reports": 0 },
            ],
            "unignore": [],
            "create_orgs": ["Spam"],
            "park": [],
            "unpark": [],
        })
    );

    let (status, body) = apply(&app, text).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
    assert!(!imap.reimport_requested());
    assert_eq!(
        mapping_text(&app).await,
        "Beta,c.com\nDefault,d.com\nAcme,a.com,ignored\nAcme,b.com,ignored\n\
         Beta,gone.com,ignored\nSpam,spoof.example,ignored\n"
    );

    // An ignored domain can change organization and stays ignored.
    let body = preview(&app, "Acme,gone.com,ignored\n").await;
    assert_eq!(
        body["changes"]["move"],
        json!([{ "domain": "gone.com", "from": "Beta", "to": "Acme" }])
    );
    assert_eq!(body["changes"]["unignore"], json!([]));

    // Listing it as parked monitors it again, parked.
    let (status, body) = apply(&app, "Acme,a.com,parked\n").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["changes"]["unignore"],
        json!([{ "domain": "a.com", "org": "Acme" }])
    );
    assert_eq!(body["changes"]["park"], json!(["a.com"]));
    assert!(imap.reimport_requested());
}

#[tokio::test]
async fn parked_flag_round_trips_and_counts_as_a_change() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let (status, body) = apply(
        &app,
        "Acme,a.com,parked\nAcme,b.com\nFresh,new.com,parked\n",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changes"]["park"], json!(["a.com", "new.com"]));
    assert_eq!(body["changes"]["unpark"], json!([]));
    assert_eq!(body["unchanged_count"], 1);
    assert_eq!(
        mapping_text(&app).await,
        "Acme,a.com,parked\nAcme,b.com\nBeta,c.com\nDefault,d.com\nFresh,new.com,parked\n\
         Beta,gone.com,ignored\n"
    );

    // Listing a parked domain without the flag unparks it, even when it also moves.
    let body = preview(&app, "Beta,a.com\n").await;
    assert_eq!(body["changes"]["unpark"], json!(["a.com"]));
    assert_eq!(body["changes"]["move"].as_array().unwrap().len(), 1);
    assert_eq!(body["unchanged_count"], 0);
}

#[tokio::test]
async fn org_names_match_case_insensitively() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let (status, body) = apply(&app, "BETA,a.com\nbeta,x.com\n").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["changes"]["create_orgs"], json!([]));
    assert_eq!(
        body["changes"]["move"],
        json!([{ "domain": "a.com", "from": "Acme", "to": "Beta" }])
    );
    assert_eq!(
        body["changes"]["add"],
        json!([{ "domain": "x.com", "org": "Beta" }])
    );
    let (_, organizations) = send(&app, "GET", "/api/organizations", None).await;
    assert_eq!(organizations["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn apply_with_errors_is_rejected_and_writes_nothing() {
    let t = TestApp::new().await;
    let (_pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let before = mapping_text(&app).await;
    let (status, body) = apply(&app, "Fresh,a.com\nFresh,new.com\nFresh,bad_domain!\n").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "unprocessable");
    assert_eq!(errors(&body).iter().map(|e| e.0).collect::<Vec<_>>(), [3]);
    assert_eq!(mapping_text(&app).await, before);
}

#[tokio::test]
async fn apply_rolls_back_entirely_when_a_write_fails() {
    let t = TestApp::new().await;
    let (pool, app) = (t.pool.clone(), t.app.clone());
    seed(&app).await;
    let before = mapping_text(&app).await;
    // Make the last write (an add) fail after the org create, move and restore ran.
    sqlx::query(
        "CREATE TRIGGER fail_boom BEFORE INSERT ON domains WHEN NEW.domain = 'boom.com' \
         BEGIN SELECT RAISE(ABORT, 'boom'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, _) = apply(&app, "Fresh,a.com\nFresh,gone.com\nFresh,boom.com\n").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(mapping_text(&app).await, before);
    let (_, organizations) = send(&app, "GET", "/api/organizations", None).await;
    assert!(organizations["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["name"] != "Fresh"));
}
