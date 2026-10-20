//! Sender identity in API responses. Offline: evidence is seeded (reverse DNS
//! snapshots, report SPF results, a DNS snapshot), never looked up.

use axum::http::StatusCode;
use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
use dmarc_dns::dmarc::evaluate_dmarc;
use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
use dmarc_dns::spf::evaluate_spf;
use dmarc_dns::{assemble_report, TxtAnswer, TxtLookup, CHECKER_VERSION};
use dmarc_parser::{DkimAuthResult, PolicyPublished, Record, Report, SpfAuthResult};
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{
    get_or_create_domain, get_or_create_organization, ingest_report, init, upsert_dns_snapshot,
    upsert_identity_snapshot, DnsSnapshot,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
mod common;
use common::{get_ok as get, post_json as post, put_json as put};

const DOMAIN: &str = "example.com";
const BEGIN: i64 = 1_704_067_200;

fn record(ip: &str, count: i64, spf: Vec<(&str, &str)>, dkim: &str) -> Record {
    Record {
        source_ip: ip.into(),
        count,
        disposition: Some("none".into()),
        dkim_result: Some(dkim.into()),
        spf_result: Some("pass".into()),
        header_from: Some(DOMAIN.into()),
        envelope_from: None,
        envelope_to: None,
        dkim_auth: vec![DkimAuthResult {
            domain: DOMAIN.into(),
            selector: Some("s1".into()),
            result: dkim.into(),
        }],
        spf_auth: spf
            .into_iter()
            .map(|(domain, result)| SpfAuthResult {
                domain: domain.into(),
                result: result.into(),
                scope: Some("mfrom".into()),
            })
            .collect(),
        policy_overrides: vec![],
    }
}

/// 192.0.2.1: SPF passed for a Mailchimp bounce domain (and the SPF record
/// lists it). 192.0.2.2: failing, no evidence. 192.0.2.3: confirmed PTR.
async fn seed() -> (tempfile::TempDir, SqlitePool, i64) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let organization = get_or_create_organization(&pool, "Acme").await.unwrap();
    let domain = get_or_create_domain(&pool, organization.id, DOMAIN)
        .await
        .unwrap();
    let records = vec![
        record("192.0.2.1", 10, vec![("bounce.mcsv.net", "pass")], "pass"),
        record("192.0.2.2", 5, vec![("evil.example", "fail")], "fail"),
        record("192.0.2.3", 7, vec![(DOMAIN, "pass")], "pass"),
    ];
    let report = Report {
        report_id: "r1".into(),
        org_name: "google.com".into(),
        email: "noreply@reporter.example".into(),
        date_begin: BEGIN,
        date_end: BEGIN + 86_399,
        policy: PolicyPublished {
            domain: DOMAIN.into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("none".into()),
            sp: None,
            pct: Some(100),
            fo: None,
        },
        records,
        raw_xml: "<feedback/>".into(),
    };
    ingest_report(&pool, &report).await.unwrap();
    upsert_identity_snapshot(
        &pool,
        "192.0.2.3",
        BEGIN,
        Some("a1-2.smtp-out.amazonses.com"),
        Some(true),
    )
    .await
    .unwrap();

    let spf = TxtLookup::Records(TxtAnswer::from_records(vec![
        "v=spf1 ip4:192.0.2.0/31 -all".into(),
    ]));
    let dmarc = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1; p=none".into()]));
    let dns = assemble_report(
        DOMAIN,
        evaluate_dmarc(DOMAIN, &dmarc, &[]),
        evaluate_spf(DOMAIN, &spf, &[]),
        evaluate_dkim(DOMAIN, &[], &DkimProbe::default()),
        evaluate_mta_sts(
            DOMAIN,
            &TxtLookup::NoRecords,
            None,
            &PolicyFetch::NotAttempted,
        ),
        evaluate_tls_rpt(DOMAIN, &TxtLookup::NoRecords),
        evaluate_bimi(DOMAIN, &TxtLookup::NoRecords),
    );
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: DOMAIN.into(),
            checked_at: dns.checked_at.timestamp(),
            grade: dns.grade.clone(),
            score: i64::from(dns.score),
            critical_count: 0,
            dmarc: dmarc_server::dns_snapshot(&dns).unwrap().dmarc,
            report_json: serde_json::to_string(&dns).unwrap(),
            checker_version: i64::from(CHECKER_VERSION),
        },
    )
    .await
    .unwrap();
    (dir, pool, domain.id)
}

fn by_ip<'a>(items: &'a Value, ip: &str) -> &'a Value {
    items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["source_ip"] == ip)
        .unwrap_or_else(|| panic!("{ip} missing"))
}

#[tokio::test]
async fn sender_rows_carry_their_identity() {
    let (_dir, pool, _) = seed().await;
    let app = router_with_state(AppState::new(pool));
    let list = get(&app, "/api/senders").await;

    let spf = &by_ip(&list["items"], "192.0.2.1")["identity"];
    assert_eq!(spf["source"], "spf");
    assert_eq!(spf["operator_domain"], "mcsv.net");
    assert_eq!(spf["spf_domain"], "bounce.mcsv.net");
    assert!(spf["name"].as_str().unwrap().contains("Mailchimp"));

    let ptr = &by_ip(&list["items"], "192.0.2.3")["identity"];
    assert_eq!(ptr["source"], "ptr");
    assert_eq!(ptr["name"], "Amazon SES");
    assert_eq!(ptr["ptr_confirmed"], true);
    assert!(
        ptr["spf_domain"].is_null(),
        "an aligned pass names the customer, not the operator"
    );

    let unknown = &by_ip(&list["items"], "192.0.2.2")["identity"];
    for field in [
        "name",
        "operator_domain",
        "source",
        "ptr_hostname",
        "ptr_confirmed",
        "spf_domain",
    ] {
        assert!(unknown.get(field).is_some_and(Value::is_null), "{field}");
    }
}

#[tokio::test]
async fn every_ip_listing_is_named_the_same_way() {
    let (_dir, pool, domain_id) = seed().await;
    let app = router_with_state(AppState::new(pool));

    let failures = get(&app, &format!("/api/domains/{domain_id}/failures")).await;
    assert!(by_ip(&failures["items"], "192.0.2.2")["identity"].is_object());

    let reports = get(&app, "/api/reports").await;
    let id = reports["items"][0]["id"].as_i64().unwrap();
    let report = get(&app, &format!("/api/reports/{id}")).await;
    assert_eq!(
        by_ip(&report["records"], "192.0.2.3")["identity"]["name"],
        "Amazon SES"
    );
}

#[tokio::test]
async fn sender_detail_shows_where_spf_lists_the_ip() {
    let (_dir, pool, domain_id) = seed().await;
    let app = router_with_state(AppState::new(pool));

    let detail = get(&app, "/api/senders/192.0.2.1").await;
    assert_eq!(detail["identity"]["operator_domain"], "mcsv.net");
    assert_eq!(detail["spf_listings"][0]["domain_id"], domain_id);
    assert_eq!(detail["spf_listings"][0]["domain"], DOMAIN);
    assert_eq!(detail["spf_listings"][0]["term"], "ip4:192.0.2.0/31");

    let outside = get(&app, "/api/senders/192.0.2.3").await;
    assert_eq!(outside["spf_listings"], Value::Array(vec![]));
}

/// Overrides saved in Settings rename operators on the very next request.
#[tokio::test]
async fn saved_overrides_rename_operators() {
    let (_dir, pool, _) = seed().await;
    let app = router_with_state(AppState::new(pool));
    let text = json!({ "text": "mcsv.net,\namazonses.com, Our mailer\n" });
    let preview = post(&app, "/api/settings/sender-names/preview", text.clone()).await;
    assert_eq!(preview.0, StatusCode::OK);
    assert_eq!(preview.1["errors"], json!([]));
    assert_eq!(
        preview.1["changes"]["add"],
        json!([
            { "target": "mcsv.net", "name": null },
            { "target": "amazonses.com", "name": "Our mailer" },
        ])
    );
    let (status, saved) = put(&app, "/api/settings/sender-names", text).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved, preview.1, "saving changes what the preview said");
    assert_eq!(
        get(&app, "/api/settings/sender-names").await["text"],
        "amazonses.com,Our mailer\nmcsv.net,\n"
    );

    let list = get(&app, "/api/senders").await;
    assert_eq!(
        by_ip(&list["items"], "192.0.2.3")["identity"]["name"],
        "Our mailer"
    );
    let hidden = &by_ip(&list["items"], "192.0.2.1")["identity"];
    assert!(hidden["name"].is_null());
    assert_eq!(hidden["operator_domain"], "mcsv.net");
}

/// An IP or range override beats the operator's name, the narrowest first.
#[tokio::test]
async fn ip_overrides_beat_the_operator_name() {
    let (_dir, pool, _) = seed().await;
    let app = router_with_state(AppState::new(pool));
    let (status, saved) = put(
        &app,
        "/api/settings/sender-names",
        json!({ "text": "192.0.2.0/24, Office\n192.0.2.3, Relay\namazonses.com, Our mailer\n" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        get(&app, "/api/settings/sender-names").await["text"],
        "amazonses.com,Our mailer\n192.0.2.0/24,Office\n192.0.2.3,Relay\n"
    );
    let list = get(&app, "/api/senders").await;
    assert_eq!(
        by_ip(&list["items"], "192.0.2.3")["identity"]["name"],
        "Relay"
    );
    assert_eq!(
        by_ip(&list["items"], "192.0.2.1")["identity"]["name"],
        "Office"
    );
}

#[tokio::test]
async fn bad_override_lines_are_all_reported_and_nothing_is_saved() {
    let (_dir, pool, _) = seed().await;
    let app = router_with_state(AppState::new(pool));
    let text = json!({ "text": "not a domain,X\nok.example,Y\nok.example,Z\n" });
    let lines = |body: &serde_json::Value| -> Vec<i64> {
        body["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["line"].as_i64().unwrap())
            .collect()
    };
    let (status, preview) = post(&app, "/api/settings/sender-names/preview", text.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lines(&preview), [1, 3]);
    let (status, body) = put(&app, "/api/settings/sender-names", text).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(lines(&body), [1, 3]);
    assert_eq!(get(&app, "/api/settings/sender-names").await["text"], "");
}
