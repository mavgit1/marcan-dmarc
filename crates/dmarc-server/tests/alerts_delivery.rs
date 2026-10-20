//! Alert emails with the mail transport stubbed (no socket is opened): one
//! email per delivery with every new event grouped by domain, the events
//! marked sent, the test alert, and the digest at its time.

use axum::http::StatusCode;
use dmarc_server::alerts::job::run_once;
use dmarc_server::mail::{MemorySender, SendError};
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{
    get_last_digest_at, ingest_report, init, save_settings, set_last_digest_at, AlertSettings,
    DigestFrequency, SendingGroup, SendingSettings, Weekday,
};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
mod common;
use common::{get_ok, post_json, rec, report_with_policy, seed_organizations};

const DAY: i64 = 86_400;
/// 2030-03-17 17:46:40 UTC.
const NOW: i64 = 1_900_000_000;
/// The events [`two_domains_change`] leads to.
const CHANGES: i64 = 3;

struct App {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    state: AppState,
    app: axum::Router,
    sender: Arc<MemorySender>,
}

async fn app_with(sender: MemorySender) -> App {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("test.db")).await.unwrap();
    let sender = Arc::new(sender);
    let state = AppState::new(pool.clone()).with_mail_sender(sender.clone());
    let app = router_with_state(state.clone());
    seed_organizations(&pool).await;
    App {
        _dir: dir,
        pool,
        state,
        app,
        sender,
    }
}

async fn app() -> App {
    app_with(MemorySender::default()).await
}

/// Sending through a made-up account (never contacted: the sender is stubbed).
async fn enable_sending(pool: &SqlitePool) {
    let sending = SendingSettings {
        group: SendingGroup {
            enabled: true,
            host: "smtp.example.com".into(),
            port: Some(465),
            same_login_as_mailbox: false,
            username: "alerts@example.com".into(),
            sender: "dmarc@example.com".into(),
            ..SendingGroup::default()
        },
        password: "secret".into(),
    };
    save_settings(pool, &sending).await.unwrap();
}

async fn enable_alerts(pool: &SqlitePool, digest: DigestFrequency) {
    let alerts = AlertSettings {
        enabled: true,
        recipients: vec!["ops@example.com".into()],
        digest,
        digest_hour: 7,
        digest_weekday: Weekday::Monday,
        base_url: "https://marcan.example.com".into(),
        muted_kinds: Vec::new(),
    };
    save_settings(pool, &alerts).await.unwrap();
}

async fn ingest(pool: &SqlitePool, id: &str, domain: &str, days_ago: i64, p: &str) {
    let report = report_with_policy(
        id,
        domain,
        NOW - days_ago * DAY,
        p,
        p,
        vec![rec("192.0.2.1", 500, "pass", "pass")],
    );
    ingest_report(pool, &report).await.expect("ingest");
}

async fn run(a: &App, now: i64) {
    run_once(&a.state, now, &CancellationToken::new()).await;
}

const SPF: &str = "v=spf1 ip4:192.0.2.0/24 -all";

/// Store `domain`'s DNS check with a DMARC record of `p` and `spf`.
async fn dns(a: &App, domain: &str, p: &str, spf: &str, at: i64) {
    let record = format!("v=DMARC1; p={p}");
    common::store_dns(&a.pool, domain, &record, spf, at).await;
}

/// Two domains change their DMARC record: example.com stops enforcing and
/// adds a sender to its SPF record, other.org tightens to quarantine. Their
/// reports still say what they did.
async fn two_domains_change(a: &App) {
    ingest(&a.pool, "e1", "example.com", 3, "reject").await;
    ingest(&a.pool, "o1", "other.org", 3, "none").await;
    dns(a, "example.com", "reject", SPF, NOW - DAY).await;
    dns(a, "other.org", "none", SPF, NOW - DAY).await;
    run(a, NOW).await;
    ingest(&a.pool, "e2", "example.com", 1, "reject").await;
    ingest(&a.pool, "o2", "other.org", 1, "none").await;
    let wider = "v=spf1 ip4:192.0.2.0/24 ip4:198.51.100.0/24 -all";
    dns(a, "example.com", "none", wider, NOW).await;
    dns(a, "other.org", "quarantine", SPF, NOW).await;
}

#[tokio::test]
async fn one_email_with_the_changes_grouped_and_the_events_marked_sent() {
    let a = app().await;
    enable_sending(&a.pool).await;
    enable_alerts(&a.pool, DigestFrequency::Off).await;
    two_domains_change(&a).await;
    assert!(a.sender.sent().is_empty(), "the baseline sends nothing");

    run(&a, NOW + 900).await;
    let sent = a.sender.sent();
    assert_eq!(sent.len(), 1, "one email per delivery and recipient");
    let mail = &sent[0];
    assert_eq!(mail.to.to_string(), "ops@example.com");
    assert_eq!(
        mail.subject,
        format!("Marcan: {CHANGES} changes on 2 domains")
    );
    let text = &mail.text;
    // Worst domain first; within it, worst line first.
    let example = text.find("\nexample.com\n").expect("example.com block");
    let other = text.find("\nother.org\n").expect("other.org block");
    assert!(example < other, "{text}");
    for line in [
        "  [Problem] DMARC policy changed: p reject → none.",
        "  [Note] SPF record changed: added ip4:198.51.100.0/24.",
        "  [Good] DMARC policy changed: p none → quarantine.",
        "  https://marcan.example.com/domains/",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(mail.html.as_deref().unwrap().contains("other.org"));

    // Every listed event is marked sent at the delivery time.
    let page = get_ok(&a.app, "/api/alerts").await;
    assert_eq!(page["total"], CHANGES);
    for item in page["items"].as_array().unwrap() {
        assert_eq!(item["notified_at"], NOW + 900, "{item}");
    }

    // Nothing new: no second email.
    run(&a, NOW + 1800).await;
    assert_eq!(a.sender.sent().len(), 1);
}

#[tokio::test]
async fn muted_kinds_are_listed_but_not_mailed() {
    let a = app().await;
    enable_sending(&a.pool).await;
    let alerts = AlertSettings {
        enabled: true,
        recipients: vec!["ops@example.com".into()],
        muted_kinds: vec!["policy_changed".into()],
        ..AlertSettings::default()
    };
    save_settings(&a.pool, &alerts).await.unwrap();
    two_domains_change(&a).await;
    run(&a, NOW + 900).await;

    let sent = a.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, "Marcan: 1 change on example.com");
    assert!(sent[0].text.contains("SPF record changed"));
    assert!(!sent[0].text.contains("DMARC policy changed"));

    // The list has them all, each with its catalogue entry; only the mailed ones are marked.
    let page = get_ok(&a.app, "/api/alerts").await;
    assert_eq!(page["total"], CHANGES);
    for item in page["items"].as_array().unwrap() {
        let muted = item["catalogue_id"] == "policy_changed";
        assert!(item["catalogue_id"].is_string(), "{item}");
        assert_eq!(item["notified_at"].is_null(), muted, "{item}");
    }

    // With every kind muted, no email at all.
    let b = app().await;
    enable_sending(&b.pool).await;
    let alerts = AlertSettings {
        muted_kinds: dmarc_assess::ALERT_CATALOGUE
            .iter()
            .map(|e| e.id().to_owned())
            .collect(),
        ..alerts
    };
    save_settings(&b.pool, &alerts).await.unwrap();
    two_domains_change(&b).await;
    run(&b, NOW + 900).await;
    assert!(b.sender.sent().is_empty());
}

#[tokio::test]
async fn without_sending_the_events_wait_and_old_ones_are_not_mailed() {
    let a = app().await;
    enable_alerts(&a.pool, DigestFrequency::Off).await;
    two_domains_change(&a).await;
    run(&a, NOW + 900).await;
    assert!(a.sender.sent().is_empty());
    let page = get_ok(&a.app, "/api/alerts").await;
    assert_eq!(page["items"][0]["notified_at"], Value::Null);

    // Sending switched on within a day: the waiting events go out.
    enable_sending(&a.pool).await;
    run(&a, NOW + 1800).await;
    assert_eq!(a.sender.sent().len(), 1);

    // Events older than a day are listed, never mailed.
    let b = app().await;
    enable_alerts(&b.pool, DigestFrequency::Off).await;
    two_domains_change(&b).await;
    run(&b, NOW + 900).await;
    enable_sending(&b.pool).await;
    run(&b, NOW + 900 + 2 * DAY).await;
    assert!(b.sender.sent().is_empty());
}

#[tokio::test]
async fn a_failing_server_keeps_the_events_for_the_next_run() {
    let a = app_with(MemorySender::failing(SendError::Auth {
        detail: "535 5.7.8 bad credentials".into(),
    }))
    .await;
    enable_sending(&a.pool).await;
    enable_alerts(&a.pool, DigestFrequency::Off).await;
    two_domains_change(&a).await;
    run(&a, NOW + 900).await;
    let page = get_ok(&a.app, "/api/alerts").await;
    assert_eq!(page["total"], CHANGES);
    assert_eq!(page["items"][0]["notified_at"], Value::Null);
}

#[tokio::test]
async fn the_test_alert_goes_to_the_test_recipient() {
    let a = app().await;
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["reason"], "disabled");

    // Not to the alert recipients (none yet): to the sender address by default.
    enable_sending(&a.pool).await;
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Test alert sent to dmarc@example.com.");

    // A saved test recipient, with the alert recipients set: only it gets the test.
    enable_alerts(&a.pool, DigestFrequency::Off).await;
    let mut settings = dmarc_store::AppSettings::load(&a.pool).await.unwrap();
    settings.sending.group.test_recipient = "me@example.org".into();
    save_settings(&a.pool, &settings.sending).await.unwrap();
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["message"], "Test alert sent to me@example.org.");
    let sent = a.sender.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].to.to_string(), "me@example.org");
    let sent = &sent[1..];
    assert_eq!(sent[0].subject, "[Test] Marcan: 4 changes on example.com");
    assert!(sent[0]
        .text
        .contains("[Problem] New issue: delivered failures."));
    // The made-up domain has no page to link to; the alert list has.
    assert!(!sent[0].text.contains("/domains/"));
    assert!(sent[0].text.contains("https://marcan.example.com/alerts"));
}

#[tokio::test]
async fn the_test_alert_passes_the_servers_refusal_on() {
    let a = app_with(MemorySender::failing(SendError::Auth {
        detail: "535 5.7.8 bad credentials".into(),
    }))
    .await;
    enable_sending(&a.pool).await;
    enable_alerts(&a.pool, DigestFrequency::Off).await;
    let (status, body) = post_json(&a.app, "/api/settings/alerts/test", json!({})).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["reason"], "auth");
    assert_eq!(body["detail"], "535 5.7.8 bad credentials");
}

#[tokio::test]
async fn the_digest_goes_out_at_its_time_even_when_nothing_changed() {
    let a = app().await;
    enable_sending(&a.pool).await;
    let mut alerts = AlertSettings {
        enabled: true,
        recipients: vec!["ops@example.com".into()],
        digest: DigestFrequency::Daily,
        ..AlertSettings::default()
    };
    save_settings(&a.pool, &alerts).await.unwrap();
    ingest(&a.pool, "e1", "example.com", 3, "reject").await;

    // The first run only remembers when the digest started.
    run(&a, NOW).await;
    assert!(a.sender.sent().is_empty());
    assert_eq!(get_last_digest_at(&a.pool).await.unwrap(), Some(NOW));

    // NOW is 17:46 UTC: the next 07:00 is the next morning.
    let next_seven = NOW - (NOW % DAY) + DAY + 7 * 3600;
    run(&a, next_seven - 60).await;
    assert!(a.sender.sent().is_empty());
    run(&a, next_seven + 60).await;
    let sent = a.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].subject,
        "Marcan daily digest: nothing changed, no domain needs attention"
    );
    assert!(sent[0].text.contains("Nothing changed."));
    // Not again before the next morning.
    run(&a, next_seven + 3600).await;
    assert_eq!(a.sender.sent().len(), 1);

    // A digest that was off for long starts over when switched on again.
    alerts.digest = DigestFrequency::Off;
    save_settings(&a.pool, &alerts).await.unwrap();
    set_last_digest_at(&a.pool, 1_000).await.unwrap();
    let body = json!({
        "enabled": true,
        "recipients": ["ops@example.com"],
        "digest": "weekly",
        "digest_hour": 7,
        "digest_weekday": "monday",
        "base_url": "",
        "muted_kinds": [],
    });
    let (status, _) = common::put_json(&a.app, "/api/settings/alerts", body).await;
    assert_eq!(status, StatusCode::OK);
    let restarted = get_last_digest_at(&a.pool).await.unwrap().unwrap();
    assert!(restarted >= dmarc_store::unix_now() - 60, "{restarted}");
}

#[tokio::test]
async fn with_alert_emails_off_no_digest_goes_out() {
    let a = app().await;
    enable_sending(&a.pool).await;
    let alerts = AlertSettings {
        enabled: false,
        recipients: vec!["ops@example.com".into()],
        digest: DigestFrequency::Daily,
        ..AlertSettings::default()
    };
    save_settings(&a.pool, &alerts).await.unwrap();
    set_last_digest_at(&a.pool, NOW - 2 * DAY).await.unwrap();

    // The digest is due (two days without one), but alert emails are off.
    run(&a, NOW).await;
    assert!(a.sender.sent().is_empty());
    assert_eq!(
        get_last_digest_at(&a.pool).await.unwrap(),
        Some(NOW - 2 * DAY)
    );

    // Switching alert emails on with the digest set starts its period, so
    // the digest does not go out at once.
    let body = json!({
        "enabled": true,
        "recipients": ["ops@example.com"],
        "digest": "daily",
        "digest_hour": 7,
        "digest_weekday": "monday",
        "base_url": "",
        "muted_kinds": [],
    });
    let (status, _) = common::put_json(&a.app, "/api/settings/alerts", body).await;
    assert_eq!(status, StatusCode::OK);
    let restarted = get_last_digest_at(&a.pool).await.unwrap().unwrap();
    assert!(restarted >= dmarc_store::unix_now() - 60, "{restarted}");
}

#[tokio::test]
async fn the_test_alert_uses_the_draft_on_screen_and_the_test_recipient() {
    let a = app().await;
    enable_sending(&a.pool).await;
    let draft = json!({
        "enabled": false,
        "recipients": ["team@example.com"],
        "digest": "off",
        "digest_hour": 7,
        "digest_weekday": "monday",
        "base_url": "",
        "muted_kinds": [],
    });
    let (status, body) = post_json(
        &a.app,
        "/api/settings/alerts/test",
        json!({ "alerts": draft.clone() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The recipients on screen do not get the test: the test recipient does.
    assert_eq!(body["message"], "Test alert sent to dmarc@example.com.");
    assert_eq!(a.sender.sent()[0].to.to_string(), "dmarc@example.com");
    // No base URL on screen: no links.
    assert!(!a.sender.sent()[0].text.contains("marcan.example.com"));

    // Nobody on screen is no reason to refuse it.
    let mut nobody = draft.clone();
    nobody["recipients"] = json!([]);
    let (status, body) = post_json(
        &a.app,
        "/api/settings/alerts/test",
        json!({ "alerts": nobody }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let mut bad = draft;
    bad["recipients"] = json!(["nobody"]);
    let (status, _) = post_json(
        &a.app,
        "/api/settings/alerts/test",
        json!({ "alerts": bad }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
