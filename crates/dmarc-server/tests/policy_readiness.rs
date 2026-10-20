//! Policy-step readiness on the domain overview: judged on the last 30 days
//! whatever `since` is, with the volume and history it needs under the
//! current policy.

use dmarc_store::{ingest_report, unix_now};
use serde_json::Value;
mod common;
use common::{get_ok, rec, report_with_policy, TestApp};

const DAY: i64 = 86_400;
/// Reports begin an hour before their day mark, so the one 30 days ago lies
/// just outside the window however fast the test runs.
const HOUR: i64 = 3_600;

/// Ingests one `p=none` report for `domain` per `(days ago, passing, failing)`,
/// and stores the DNS check its policy comes from: `p=none`.
async fn seed(app: &TestApp, domain: &str, reports: &[(i64, i64, i64)]) {
    let reports: Vec<_> = reports
        .iter()
        .map(|&(days_ago, pass, fail)| (days_ago, "none", pass, fail))
        .collect();
    seed_under(app, domain, "none", &reports).await;
}

/// Ingests one report for `domain` per `(days ago, applied p=, passing,
/// failing)`, and stores the DNS check with `p=dns_policy`.
async fn seed_under(
    app: &TestApp,
    domain: &str,
    dns_policy: &str,
    reports: &[(i64, &str, i64, i64)],
) {
    let now = unix_now();
    for (i, &(days_ago, applied, pass, fail)) in reports.iter().enumerate() {
        let mut records = vec![rec("192.0.2.1", pass, "pass", "pass")];
        if fail > 0 {
            records.push(rec("198.51.100.7", fail, "fail", "fail"));
        }
        let report = report_with_policy(
            &format!("{domain}-{i}"),
            domain,
            now - days_ago * DAY - HOUR,
            applied,
            applied,
            records,
        );
        ingest_report(&app.pool, &report).await.expect("ingest");
    }
    let record = format!("v=DMARC1; p={dns_policy}");
    common::store_dns(&app.pool, domain, &record, "v=spf1 -all", now).await;
}

/// One report a day under `p=applied`, `messages` passing each, from
/// `from` days ago to `to` days ago.
fn daily(from: i64, to: i64, applied: &str, messages: i64) -> Vec<(i64, &str, i64, i64)> {
    (to..=from)
        .rev()
        .map(|d| (d, applied, messages, 0))
        .collect()
}

async fn overview(app: &TestApp, domain: &str, since: &str) -> Value {
    let domains = get_ok(&app.app, "/api/domains").await;
    let id = domains["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["domain"] == domain)
        .unwrap_or_else(|| panic!("{domain} listed"))["id"]
        .as_i64()
        .unwrap();
    get_ok(&app.app, &format!("/api/domains/{id}?since={since}")).await
}

#[tokio::test]
async fn readiness_needs_volume_history_and_compliance_over_thirty_days() {
    let app = TestApp::new().await;
    // 60 days of history; the window's 20 days of reports hold 400 messages
    // at 99.5%. The 45-day-old report fails badly, but lies outside it.
    seed(
        &app,
        "ready.test",
        &[(45, 10, 90), (20, 200, 0), (5, 198, 2)],
    )
    .await;
    seed(&app, "little.test", &[(45, 10, 0), (5, 50, 0)]).await;
    seed(&app, "new.test", &[(10, 150, 0), (3, 150, 0)]).await;

    let ready = overview(&app, "ready.test", "7d").await;
    let r = &ready["policy_readiness"];
    assert_eq!(r["state"]["kind"], "ready", "{r}");
    assert_eq!(r["state"]["next"], "quarantine");
    assert_eq!(r["status"], "pass");
    // Worded by the server: the verdict, the header's line and the rules.
    assert_eq!(r["lead"], "Ready for p=quarantine");
    assert_eq!(r["headline"], "Ready for p=quarantine");
    assert_eq!(r["evidence"].as_array().unwrap().len(), 4);
    assert_eq!(r["steps"][0]["when"], "Publish now");
    assert_eq!(r["steps"][0]["primary"], true);
    assert_eq!(r["window"]["messages"], 400);
    let pct = r["window"]["delivered_compliance"]["pct"].as_f64().unwrap();
    assert!((pct - 99.5).abs() < 1e-9, "{pct}");
    // The page's time range does not move the verdict (the window's start
    // moves with the clock only).
    let without_start = |mut v: Value| {
        v["policy_readiness"]["window"]["since"] = Value::Null;
        v["policy_readiness"].clone()
    };
    let all_time = overview(&app, "ready.test", "365d").await;
    assert_eq!(without_start(all_time), without_start(ready.clone()));
    // At p=none every report carries p=none: all of the window's mail is
    // under the current policy, and its 45 days of reports cover the window.
    assert_eq!(r["window"]["policy"], "monitor");
    assert_eq!(r["window"]["policy_messages"], 400);
    assert_eq!(r["window"]["policy_share_pct"], 100.0);
    assert_eq!(r["window"]["other_policy"], "unknown");
    assert_eq!(r["history_days"], 30);
    assert_eq!(r["criteria"]["enough_mail"], true);
    assert_eq!(r["criteria"]["full_history"], true);
    assert_eq!(r["criteria"]["policy_share_met"], true);
    assert_eq!(r["criteria"]["compliance_met"], true);
    assert_eq!(r["plan"][0]["p"], "quarantine");
    assert_eq!(r["plan"][1]["p"], "reject");

    // Who a stricter policy would affect: the window's failing senders, the
    // same messages as the window's failed count.
    assert_eq!(r["window"]["failed"], 2);
    let since = r["window"]["since"].as_i64().unwrap();
    let id = ready["id"].as_i64().unwrap();
    let senders = get_ok(
        &app.app,
        &format!("/api/senders?domain_id={id}&since={since}&failing=true&sort=-failed_messages"),
    )
    .await;
    assert_eq!(senders["total"], 1, "{senders}");
    assert_eq!(senders["items"][0]["source_ip"], "198.51.100.7");
    assert_eq!(senders["items"][0]["failed_messages"], 2);

    let little = overview(&app, "little.test", "30d").await;
    let r = &little["policy_readiness"];
    assert_eq!(r["state"]["kind"], "not_ready", "{r}");
    assert_eq!(r["state"]["reason"]["kind"], "too_little_mail");
    assert_eq!(r["window"]["messages"], 50);
    assert_eq!(r["status"], "neutral");

    let new = overview(&app, "new.test", "30d").await;
    let r = &new["policy_readiness"];
    assert_eq!(r["state"]["reason"]["kind"], "not_enough_history", "{r}");
    let first = r["window"]["policy_first_report_at"].as_i64().unwrap();
    assert_eq!(r["history_days"], 10);
    assert_eq!(
        r["state"]["reason"]["window_full_at"].as_i64().unwrap(),
        first + 30 * DAY
    );

    // The reports say p=none, but without a DNS check there is no policy to
    // judge: a report's policy is a receiver's view, not the domain's.
    let report = report_with_policy(
        "unchecked-0",
        "unchecked.test",
        unix_now() - 20 * DAY,
        "none",
        "none",
        vec![rec("192.0.2.1", 400, "pass", "pass")],
    );
    ingest_report(&app.pool, &report).await.expect("ingest");
    let unchecked = overview(&app, "unchecked.test", "30d").await;
    assert_eq!(unchecked["policy_source"], "not_checked");
    let state = &unchecked["policy_readiness"]["state"];
    assert_eq!(state["kind"], "not_applicable", "{state}");
    assert_eq!(state["reason"], "no_policy");
}

#[tokio::test]
async fn readiness_counts_only_the_evidence_under_the_current_policy() {
    let app = TestApp::new().await;
    // Ready for p=quarantine after 40 days of p=none; p=quarantine published
    // this morning, and the first report under it already holds 300 messages.
    let mut today = daily(40, 1, "none", 20);
    today.push((0, "quarantine", 300, 0));
    seed_under(&app, "today.test", "quarantine", &today).await;
    // The same domain 30 days on: p=none until 35 days ago, p=quarantine since.
    let mut month = daily(60, 35, "none", 20);
    month.extend(daily(32, 0, "quarantine", 20));
    seed_under(&app, "month.test", "quarantine", &month).await;

    let today = overview(&app, "today.test", "30d").await;
    let r = &today["policy_readiness"];
    assert_eq!(r["state"]["kind"], "not_ready", "{r}");
    assert_eq!(r["state"]["next"], "reject");
    assert_eq!(r["state"]["reason"]["kind"], "not_enough_history");
    assert_eq!(r["window"]["policy"], "quarantine");
    assert_eq!(r["window"]["policy_messages"], 300);
    assert_eq!(r["window"]["other_policy"], "monitor");
    assert_eq!(r["history_days"], 0);
    let first = r["window"]["policy_first_report_at"].as_i64().unwrap();
    assert_eq!(
        r["state"]["reason"]["window_full_at"].as_i64().unwrap(),
        first + 30 * DAY
    );
    // The window's p=none mail still counts toward the totals and compliance.
    assert_eq!(r["window"]["messages"], 300 + 29 * 20);
    assert_eq!(r["criteria"]["compliance_met"], true);
    assert_eq!(r["criteria"]["enough_mail"], true);
    assert_eq!(r["criteria"]["policy_share_met"], false);

    let month = overview(&app, "month.test", "30d").await;
    let r = &month["policy_readiness"];
    assert_eq!(r["state"]["kind"], "ready", "{r}");
    assert_eq!(r["state"]["next"], "reject");
    assert_eq!(r["history_days"], 30);
    assert_eq!(r["window"]["policy_messages"], 600);
    assert_eq!(r["window"]["policy_share_pct"], 100.0);
}

#[tokio::test]
async fn stragglers_under_the_old_policy_block_only_above_five_percent() {
    let app = TestApp::new().await;
    // 40 days of p=quarantine, 30 messages a day: 900 in the window. One
    // receiver still evaluated some of it under p=none 20 days ago.
    let mut ten = daily(40, 0, "quarantine", 30);
    ten.push((20, "none", 100, 0));
    seed_under(&app, "ten.test", "quarantine", &ten).await;
    let mut three = daily(40, 0, "quarantine", 30);
    three.push((20, "none", 28, 0));
    seed_under(&app, "three.test", "quarantine", &three).await;

    let ten = overview(&app, "ten.test", "30d").await;
    let r = &ten["policy_readiness"];
    assert_eq!(r["state"]["kind"], "not_ready", "{r}");
    assert_eq!(r["state"]["reason"]["kind"], "old_policy_applied");
    assert_eq!(r["window"]["messages"], 1000);
    assert_eq!(r["window"]["policy_messages"], 900);
    assert_eq!(r["window"]["policy_share_pct"], 90.0);
    assert_eq!(r["window"]["other_policy"], "monitor");
    // A straggler inside the window does not restart the clock.
    assert_eq!(r["criteria"]["full_history"], true);
    assert_eq!(r["history_days"], 30);

    let three = overview(&app, "three.test", "30d").await;
    let r = &three["policy_readiness"];
    assert_eq!(r["state"]["kind"], "ready", "{r}");
    assert_eq!(r["criteria"]["policy_share_met"], true);
    assert_eq!(r["window"]["other_policy"], "monitor");
}

/// A parked domain without a DMARC record is told to publish one at
/// p=reject now, like a parked domain at p=none, not "nothing to judge".
#[tokio::test]
async fn a_parked_domain_without_a_record_publishes_reject_now() {
    let app = TestApp::new().await;
    seed(&app, "parked.example", &[(3, 0, 20)]).await;
    // The last DNS check found no DMARC record.
    common::store_dns(
        &app.pool,
        "parked.example",
        "not a dmarc record",
        "v=spf1 -all",
        unix_now() + 1,
    )
    .await;
    let id = overview(&app, "parked.example", "30d").await["id"]
        .as_i64()
        .unwrap();
    assert!(dmarc_store::set_domain_parked(&app.pool, id, true)
        .await
        .unwrap());

    let o = overview(&app, "parked.example", "30d").await;
    assert_eq!(o["policy_source"], "not_published");
    let r = &o["policy_readiness"];
    assert_eq!(r["state"]["kind"], "not_applicable", "{r:#}");
    assert_eq!(r["state"]["reason"], "parked");
    assert_eq!(r["lead"], "Publish p=reject now");
    assert_eq!(r["detail"], "the domain is marked as parked.");
    assert_eq!(
        r["plan"],
        serde_json::json!([{ "p": "reject", "pct": 100 }])
    );
    assert_eq!(r["steps"][0]["when"], "Publish now");
    assert_eq!(r["steps"][0]["primary"], true);
    assert_eq!(r["parked"]["lead"], "Publish p=reject now.");
    let issues: Vec<&str> = o["assessment"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert!(issues.contains(&"parked_not_rejecting"), "{issues:?}");
}

#[tokio::test]
async fn meta_serves_the_readiness_numbers() {
    let app = TestApp::new().await;
    let meta = get_ok(&app.app, "/api/meta").await;
    assert_eq!(meta["readiness_window_days"], 30);
    assert_eq!(meta["readiness_min_messages"], 200);
    assert_eq!(meta["readiness_min_policy_share_pct"], 95.0);
}
