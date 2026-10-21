//! Alerts: the evaluation over real store data (a baseline first, then only
//! what changed), the alert list, and the alerts settings group.

use axum::http::StatusCode;
use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
use dmarc_dns::dmarc::evaluate_dmarc;
use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
use dmarc_dns::spf::evaluate_spf;
use dmarc_dns::{assemble_report, TxtAnswer, TxtLookup, CHECKER_VERSION};
use dmarc_server::alerts::evaluate::evaluate_all;
use dmarc_store::{ingest_report, upsert_dns_snapshot, DnsSnapshot};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
mod common;
use common::{get_ok, put_json, rec, report_with_policy, seed_organizations, TestApp};

const DAY: i64 = 86_400;
const NOW: i64 = 1_900_000_000;

fn txt(record: &str) -> TxtLookup {
    TxtLookup::Records(TxtAnswer::from_records(vec![record.to_owned()]))
}

/// Store a complete DNS result for example.com with a DMARC record of `p`
/// and `spf` as its SPF record.
async fn store_dns(t: &TestApp, p: &str, spf: &str, checked_at: i64) {
    let domain = "example.com";
    let report = assemble_report(
        domain,
        evaluate_dmarc(
            domain,
            &txt(&format!("v=DMARC1; p={p}; rua=mailto:dmarc@example.com")),
            &[],
        ),
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
    let snapshot = DnsSnapshot {
        domain: domain.into(),
        checked_at,
        grade: report.grade.clone(),
        score: i64::from(report.score),
        critical_count: 0,
        dmarc: dmarc_server::dns_snapshot(&report).unwrap().dmarc,
        report_json: serde_json::to_string(&report).unwrap(),
        checker_version: i64::from(CHECKER_VERSION),
    };
    assert!(upsert_dns_snapshot(&t.pool, &snapshot).await.unwrap());
}

/// One report for example.com from `days_ago` before [`NOW`], all passing.
async fn ingest(t: &TestApp, id: &str, days_ago: i64, p: &str) {
    let report = report_with_policy(
        id,
        "example.com",
        NOW - days_ago * DAY,
        p,
        p,
        vec![rec("192.0.2.1", 500, "pass", "pass")],
    );
    ingest_report(&t.pool, &report).await.expect("ingest");
}

async fn evaluate(t: &TestApp, now: i64) -> usize {
    evaluate_all(&t.pool, now, &CancellationToken::new())
        .await
        .expect("evaluate")
        .events
}

/// The events of the list, oldest first, as `kind` plus what they carry.
async fn events(t: &TestApp, query: &str) -> Vec<Value> {
    let page = get_ok(&t.app, &format!("/api/alerts{query}")).await;
    let mut items: Vec<Value> = page["items"].as_array().unwrap().clone();
    items.reverse();
    items.into_iter().map(|i| i["event"].clone()).collect()
}

#[tokio::test]
async fn the_first_run_is_a_baseline_and_the_second_names_what_changed() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    ingest(&t, "r1", 3, "reject").await;
    store_dns(&t, "reject", "v=spf1 ip4:192.0.2.0/24 -all", NOW - DAY).await;

    assert_eq!(
        evaluate(&t, NOW).await,
        0,
        "the first evaluation raises nothing"
    );
    assert_eq!(get_ok(&t.app, "/api/alerts").await["total"], 0);
    // Nothing changed: nothing again.
    assert_eq!(evaluate(&t, NOW + 900).await, 0);

    // The DMARC record drops to p=quarantine and the SPF record gains a term;
    // the newest report still says p=reject (receivers lag DNS).
    store_dns(
        &t,
        "quarantine",
        "v=spf1 ip4:192.0.2.0/24 ip4:198.51.100.0/24 -all",
        NOW,
    )
    .await;
    ingest(&t, "r2", 1, "reject").await;
    assert_eq!(evaluate(&t, NOW + 1800).await, 2);

    let list = events(&t, "").await;
    let kinds: Vec<&str> = list.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    // The DMARC record's change is the policy change, one event.
    assert_eq!(kinds, ["policy_changed", "dns_record_changed"], "{list:#?}");
    assert_eq!(list[0]["from"]["p"], "reject");
    assert_eq!(list[0]["to"]["p"], "quarantine");
    assert_eq!(list[0]["to"]["rua"], json!(["mailto:dmarc@example.com"]));
    assert_eq!(list[0]["severity"], "warn");
    assert_eq!(
        list[1],
        json!({
            "kind": "dns_record_changed",
            "check": "spf",
            "change": {
                "kind": "spf_changed",
                "added": ["ip4:198.51.100.0/24"],
                "removed": [],
                "lookups_before": 0,
                "lookups_after": 0
            },
            "severity": "neutral"
        })
    );

    // Each item names its domain and when it was found.
    let page = get_ok(&t.app, "/api/alerts").await;
    let first = &page["items"][0];
    assert_eq!(first["domain"], "example.com");
    assert_eq!(first["organization"], "Acme");
    assert_eq!(first["created_at"], NOW + 1800);
    assert_eq!(first["notified_at"], Value::Null);

    // The same state again: nothing new.
    assert_eq!(evaluate(&t, NOW + 2700).await, 0);
}

/// A domain whose state cannot be read is skipped and named; the others are
/// evaluated and stored, and it is evaluated again once it can be read.
#[tokio::test]
async fn one_domain_that_fails_does_not_stop_the_others() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    ingest(&t, "r1", 3, "reject").await;
    store_dns(&t, "reject", "v=spf1 ip4:192.0.2.0/24 -all", NOW - DAY).await;
    let other = report_with_policy(
        "o1",
        "other.org",
        NOW - 3 * DAY,
        "none",
        "none",
        vec![rec("192.0.2.1", 10, "pass", "pass")],
    );
    ingest_report(&t.pool, &other).await.expect("ingest");
    // A value its overview cannot read.
    sqlx::query("UPDATE domains SET parked = 'broken' WHERE domain = 'other.org'")
        .execute(&t.pool)
        .await
        .unwrap();

    let run = |now: i64| {
        let pool = t.pool.clone();
        async move {
            evaluate_all(&pool, now, &CancellationToken::new())
                .await
                .expect("the run itself succeeds")
        }
    };
    let summary = run(NOW).await;
    assert_eq!(summary.domains, 1);
    assert_eq!(summary.skipped, ["other.org"]);

    // example.com's baseline was stored: a change to it is found next run,
    // while other.org is still skipped.
    store_dns(&t, "quarantine", "v=spf1 ip4:192.0.2.0/24 -all", NOW).await;
    let summary = run(NOW + 900).await;
    assert_eq!((summary.domains, summary.events), (1, 1));
    assert_eq!(summary.skipped, ["other.org"]);

    // Readable again: other.org gets its baseline, nothing is skipped.
    sqlx::query("UPDATE domains SET parked = 0 WHERE domain = 'other.org'")
        .execute(&t.pool)
        .await
        .unwrap();
    let summary = run(NOW + 1800).await;
    assert_eq!((summary.domains, summary.events), (2, 0));
    assert!(summary.skipped.is_empty());
}

/// Receivers report the same DNS record differently (one fills in `sp=`,
/// another leaves it out, as seen on a live server): no policy change while
/// DNS stays the same.
#[tokio::test]
async fn reports_that_disagree_on_the_policy_change_nothing() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    store_dns(&t, "none", "v=spf1 -all", NOW - DAY).await;
    let report = |id: &str, days_ago: i64, sp: Option<&str>| {
        let mut report = report_with_policy(
            id,
            "example.com",
            NOW - days_ago * DAY,
            "none",
            "none",
            vec![rec("192.0.2.1", 500, "pass", "pass")],
        );
        report.policy.sp = sp.map(Into::into);
        report
    };
    ingest_report(&t.pool, &report("r1", 3, Some("none")))
        .await
        .unwrap();
    assert_eq!(evaluate(&t, NOW).await, 0);
    ingest_report(&t.pool, &report("r2", 2, None))
        .await
        .unwrap();
    assert_eq!(evaluate(&t, NOW + 900).await, 0);
    assert_eq!(get_ok(&t.app, "/api/alerts").await["total"], 0);
}

/// Parking a domain swaps its issues (as on the owner's server: not
/// enforcing and DKIM coverage resolved, parked without p=reject and mail
/// passing for a parked domain opened); that is one event, and so is
/// unparking it.
#[tokio::test]
async fn parking_and_unparking_a_domain_is_one_event_each() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    ingest(&t, "r1", 3, "none").await;
    store_dns(&t, "none", "v=spf1 ip4:192.0.2.0/24 -all", NOW - DAY).await;
    assert_eq!(evaluate(&t, NOW).await, 0);
    let (id, _) = dmarc_store::list_monitored_domain_ids(&t.pool)
        .await
        .unwrap()
        .into_iter()
        .find(|(_, name)| name == "example.com")
        .unwrap();

    assert!(dmarc_store::set_domain_parked(&t.pool, id, true)
        .await
        .unwrap());
    assert_eq!(evaluate(&t, NOW + 900).await, 1);
    assert_eq!(evaluate(&t, NOW + 1800).await, 0);
    assert!(dmarc_store::set_domain_parked(&t.pool, id, false)
        .await
        .unwrap());
    assert_eq!(evaluate(&t, NOW + 2700).await, 1);
    assert_eq!(
        events(&t, "").await,
        [
            json!({ "kind": "parked_changed", "parked": true, "severity": "neutral" }),
            json!({ "kind": "parked_changed", "parked": false, "severity": "neutral" }),
        ]
    );
    let page = get_ok(&t.app, "/api/alerts").await;
    assert_eq!(page["items"][0]["catalogue_id"], "parked_changed.unparked");
    assert_eq!(page["items"][1]["catalogue_id"], "parked_changed.parked");
}

/// The first evaluation after the alert job held back for a mailbox import
/// takes the reports of the domains that got some since their last
/// evaluation as a new baseline (they landed in any order), while their DNS
/// changes still count. The hold and the evaluation times are both stored,
/// so nothing in memory is needed (as after a restart in the middle of the
/// import), and an evaluation that stored nothing keeps the rule for the
/// next one.
#[tokio::test]
async fn the_first_evaluation_after_an_import_rebaselines_its_domains() {
    async fn run(held: bool) -> Vec<Value> {
        let t = TestApp::new().await;
        seed_organizations(&t.pool).await;
        // Evaluation times before the reports are stored: the store compares
        // when a domain was evaluated with when its reports were stored.
        let base = dmarc_store::unix_now() - 3600;
        let ingest_at = |id: &'static str, at: i64| {
            let report = report_with_policy(
                id,
                "example.com",
                at,
                "reject",
                "reject",
                vec![rec("192.0.2.1", 500, "pass", "pass")],
            );
            let pool = t.pool.clone();
            async move { ingest_report(&pool, &report).await.expect("ingest") }
        };
        // Reports stopped 20 days ago: the issue is open at the baseline.
        ingest_at("r1", base - 20 * DAY).await;
        store_dns(&t, "reject", "v=spf1 ip4:192.0.2.0/24 -all", base - DAY).await;
        assert_eq!(evaluate(&t, base).await, 0);

        // A run held back for the import; the import brings a recent report;
        // DNS gains an SPF term meanwhile.
        if held {
            dmarc_store::set_alerts_held_at(&t.pool, base + 60)
                .await
                .unwrap();
        }
        ingest_at("r2", base - DAY).await;
        let wide = "v=spf1 ip4:192.0.2.0/24 ip4:198.51.100.0/24 -all";
        store_dns(&t, "reject", wide, base).await;
        // An evaluation that stops before storing anything.
        let stopped = CancellationToken::new();
        stopped.cancel();
        evaluate_all(&t.pool, base + 600, &stopped).await.unwrap();
        assert_eq!(events(&t, "").await, Vec::<Value>::new());

        let summary = evaluate_all(&t.pool, base + 900, &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(summary.rebaselined, usize::from(held));
        // The state after it is the baseline: nothing again.
        assert_eq!(evaluate(&t, base + 1800).await, 0);
        events(&t, "").await
    }
    let kinds = |list: &[Value]| -> Vec<String> {
        list.iter()
            .map(|e| {
                e["issue"]
                    .as_str()
                    .or_else(|| e["kind"].as_str())
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    assert_eq!(
        kinds(&run(false).await),
        ["reports_stopped", "dns_record_changed"]
    );
    assert_eq!(kinds(&run(true).await), ["dns_record_changed"]);
}

/// Reports count as stopped after the domain warning (the monitoring
/// setting), and the event names the days and the setting.
#[tokio::test]
async fn reports_stop_after_the_domain_warning_and_say_how_long() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    // A report covering the day that began four days ago: its end, three
    // days back, is the last report.
    ingest(&t, "r1", 4, "reject").await;
    store_dns(&t, "reject", "v=spf1 ip4:192.0.2.0/24 -all", NOW - DAY).await;
    // Three days without reports: fine by the default of 7.
    assert_eq!(evaluate(&t, NOW).await, 0);
    let (status, body) = put_json(
        &t.app,
        "/api/settings/monitoring",
        json!({ "report_freshness_warning_hours": 24, "domain_reports_stopped_days": 3 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["monitoring"]["domain_reports_stopped_days"], 3);
    assert_eq!(evaluate(&t, NOW + 900).await, 1);
    assert_eq!(
        events(&t, "").await,
        [json!({
            "kind": "issue_opened",
            "issue": "reports_stopped",
            "stopped": { "days": 3, "threshold_days": 3 },
            "severity": "warn"
        })]
    );
    // The list sends the alert email's sentence and the colour's word with it.
    let item = &get_ok(&t.app, "/api/alerts").await["items"][0];
    assert_eq!(item["text"], "No reports for 3 days. (Threshold 3 days.)");
    assert_eq!(item["severity_label"], "Warning");
    // The domain shows the threshold it was judged by, and meta serves it.
    let domains = get_ok(&t.app, "/api/domains").await;
    assert_eq!(
        domains["items"][0]["assessment"]["reports_stopped_after_days"],
        3
    );
    assert_eq!(get_ok(&t.app, "/api/meta").await["reports_stopped_days"], 3);
}

#[tokio::test]
async fn the_list_follows_the_scope_and_pages() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    // The SPF record gains a term, then loses it again: one change each time.
    let (narrow, wide) = (
        "v=spf1 ip4:192.0.2.0/24 -all",
        "v=spf1 ip4:192.0.2.0/24 ip4:198.51.100.0/24 -all",
    );
    ingest(&t, "r1", 3, "reject").await;
    store_dns(&t, "reject", narrow, NOW - DAY).await;
    evaluate(&t, NOW).await;
    store_dns(&t, "reject", wide, NOW).await;
    evaluate(&t, NOW + 900).await;
    store_dns(&t, "reject", narrow, NOW + 1000).await;
    evaluate(&t, NOW + 1800).await;

    let page = get_ok(&t.app, "/api/alerts?limit=1").await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    // Newest first: the term removed again.
    assert_eq!(page["items"][0]["created_at"], NOW + 1800);
    assert_eq!(
        page["items"][0]["event"]["change"]["removed"],
        json!(["ip4:198.51.100.0/24"])
    );
    let page2 = get_ok(&t.app, "/api/alerts?limit=1&offset=1").await;
    assert_eq!(page2["items"][0]["created_at"], NOW + 900);

    let domains = get_ok(&t.app, "/api/domains").await;
    let id = domains["items"][0]["id"].as_i64().unwrap();
    let org = domains["items"][0]["organization"]["id"].as_i64().unwrap();
    assert_eq!(
        get_ok(&t.app, &format!("/api/alerts?domain_id={id}")).await["total"],
        2
    );
    assert_eq!(
        get_ok(&t.app, &format!("/api/alerts?organization_id={org}")).await["total"],
        2
    );
    assert_eq!(
        get_ok(
            &t.app,
            &format!("/api/alerts?organization_id={}", org + 100)
        )
        .await["total"],
        0
    );
    // `since` is over when the change was found.
    assert_eq!(
        get_ok(&t.app, &format!("/api/alerts?since={}", NOW + 1000)).await["total"],
        1
    );
}

#[tokio::test]
async fn alerts_settings_are_off_by_default_and_validated() {
    let t = TestApp::new().await;
    let alerts = get_ok(&t.app, "/api/settings").await["alerts"].clone();
    assert_eq!(
        alerts,
        json!({
            "enabled": false,
            "recipients": [],
            "digest": "off",
            "digest_hour": 7,
            "digest_weekday": "monday",
            "base_url": "",
            "muted_kinds": [],
        })
    );

    let body = json!({
        "enabled": true,
        "recipients": ["ops@example.com", " noc@example.com ", "OPS@example.com"],
        "digest": "weekly",
        "digest_hour": 18,
        "digest_weekday": "friday",
        "base_url": "https://marcan.example.com/",
        "muted_kinds": ["policy_changed", "readiness_reached", "policy_changed"],
    });
    let (status, saved) = put_json(&t.app, "/api/settings/alerts", body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["alerts"],
        json!({
            "enabled": true,
            "recipients": ["ops@example.com", "noc@example.com"],
            "digest": "weekly",
            "digest_hour": 18,
            "digest_weekday": "friday",
            "base_url": "https://marcan.example.com",
            "muted_kinds": ["policy_changed", "readiness_reached"],
        })
    );

    // Every muted id must be a catalogue entry, which GET /api/meta lists,
    // with how often changes are checked.
    let meta = get_ok(&t.app, "/api/meta").await;
    assert_eq!(meta["alert_interval_minutes"], 15);
    assert_eq!(meta["new_sender_min_failures"], 10);
    let catalogue = meta["alert_catalogue"].clone();
    let ids: Vec<&str> = catalogue
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"policy_changed") && ids.contains(&"dns_record_changed.spf"));
    assert_eq!(
        catalogue[0],
        json!({
            "id": "issue_opened.delivered_failures",
            "group": "issues",
            "title": "New issue: delivered failures",
            "trigger": "An issue opened: mail that failed DMARC is delivered at a warning or failing rate.",
        })
    );
    // Each group once, with its heading, in the catalogue's order.
    assert_eq!(
        meta["alert_groups"][0],
        json!({ "group": "issues", "title": "Issues" })
    );
    assert_eq!(meta["alert_groups"].as_array().unwrap().len(), 7);
    let mut bad = body.clone();
    bad["muted_kinds"] = json!(["policy"]);
    let (status, err) = put_json(&t.app, "/api/settings/alerts", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["error"], "There is no kind of alert called policy.");
    // A kind not in the catalogue is refused.
    let mut bad = body.clone();
    bad["muted_kinds"] = json!(["dns_grade_changed"]);
    let (status, _) = put_json(&t.app, "/api/settings/alerts", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let mut bad = body.clone();
    bad["recipients"] = json!(["not an address"]);
    let (status, err) = put_json(&t.app, "/api/settings/alerts", bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        err["error"],
        "The recipient not an address is not a valid email address."
    );
    // On with nobody to tell: turned on without what it needs.
    let mut bad = body;
    bad["recipients"] = json!([]);
    let (status, _) = put_json(&t.app, "/api/settings/alerts", bad).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // The rejected saves changed nothing.
    assert_eq!(
        get_ok(&t.app, "/api/settings").await["alerts"]["recipients"],
        json!(["ops@example.com", "noc@example.com"])
    );
}

/// A failing record from `ip`: delivered, or forwarded when `forwarded`.
fn failing(ip: &str, count: i64, forwarded: bool) -> dmarc_parser::Record {
    let mut r = rec(ip, count, "fail", "fail");
    if forwarded {
        r.policy_overrides = vec![dmarc_parser::PolicyOverride {
            reason_type: Some("forwarded".into()),
            comment: None,
        }];
    }
    r
}

async fn ingest_records(t: &TestApp, id: &str, days_ago: i64, records: Vec<dmarc_parser::Record>) {
    let mut records = records;
    // Plenty of passing mail, so the failures open no issue of their own.
    records.push(rec("192.0.2.1", 100_000, "pass", "pass"));
    let report = report_with_policy(
        id,
        "example.com",
        NOW - days_ago * DAY,
        "none",
        "none",
        records,
    );
    ingest_report(&t.pool, &report).await.expect("ingest");
}

/// A sender new to the domain alerts once it fails with 10 messages or
/// more, forwarded mail not counted; senders failing at the baseline and
/// senders known before the window raise nothing, and a sender alerted
/// once stays quiet.
#[tokio::test]
async fn a_new_failing_sender_is_alerted_once() {
    let t = TestApp::new().await;
    seed_organizations(&t.pool).await;
    store_dns(&t, "none", "v=spf1 ip4:192.0.2.0/24 -all", NOW - DAY).await;
    // Known before the window.
    ingest_records(&t, "old", 40, vec![failing("198.51.100.9", 1, false)]).await;
    // New, and failing already at the first evaluation: the baseline.
    ingest_records(&t, "r1", 3, vec![failing("203.0.113.7", 20, false)]).await;
    assert_eq!(
        evaluate(&t, NOW).await,
        0,
        "the first evaluation is a baseline"
    );

    ingest_records(
        &t,
        "r2",
        1,
        vec![
            failing("203.0.113.5", 12, false),
            // Mostly forwarded: 9 failures count, below the threshold.
            failing("203.0.113.6", 50, true),
            failing("203.0.113.6", 9, false),
            // Known before the window, however much it fails now.
            failing("198.51.100.9", 300, false),
            failing("203.0.113.7", 30, false),
        ],
    )
    .await;
    assert_eq!(evaluate(&t, NOW + 900).await, 1);
    let page = get_ok(&t.app, "/api/alerts").await;
    let item = &page["items"][0];
    assert_eq!(
        item["event"],
        json!({
            "kind": "new_failing_sender",
            "sender": {
                "source_ip": "203.0.113.5",
                "name": null,
                "failed": 12,
                "delivered_failures": 12
            },
            "window_days": 30,
            "severity": "fail"
        })
    );
    assert_eq!(item["catalogue_id"], "new_failing_sender");
    assert_eq!(item["sender_ip"], "203.0.113.5");
    assert!(item["text"].as_str().unwrap().starts_with(
        "New sender 203.0.113.5 failed DMARC with 12 messages in the last 30 days (all delivered)"
    ));

    // It keeps failing: no second alert.
    ingest_records(&t, "r3", 0, vec![failing("203.0.113.5", 40, false)]).await;
    assert_eq!(evaluate(&t, NOW + 1800).await, 0);
}
