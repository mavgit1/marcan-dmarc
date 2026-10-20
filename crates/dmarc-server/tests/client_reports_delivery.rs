//! Client reports with the mail transport stubbed (no socket is opened):
//! each organization's report holds only its own domains, the job sends
//! once a month to each recipient (also across a restart mid-send), retries
//! a recipient not reached for three days and one refused for good never,
//! holds the reports while the mailbox is not read, and the preview needs
//! sending on.

use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use dmarc_assess::ClientReportHold;
use dmarc_server::alerts::evaluate::evaluate_all;
use dmarc_server::client_reports::job::{run_once, schedule, Delivery};
use dmarc_server::client_reports::report::organization_report;
use dmarc_server::mail::{Letterhead, MailSender, MemorySender, Message, SendError, SmtpConfig};
use dmarc_server::wording::client_reports::undelivered_line;
use dmarc_server::{router_with_state, AppState};
use dmarc_store::{
    client_reports_last_sent_at, get_or_create_domain, get_or_create_organization, ingest_report,
    init, save_settings, AppSettings, ClientReportSettings, SendingGroup, SendingSettings,
};
use futures::future::BoxFuture;
use serde_json::json;
use sqlx::SqlitePool;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
mod common;
use common::{post_json, put_json, rec, report_with_policy};

const DAY: i64 = 86_400;

/// One run of the job at `now`, never cancelled.
async fn run(state: &AppState, now: i64) -> Delivery {
    run_once(state, now, &CancellationToken::new()).await
}

/// A [`MemorySender`] whose servers refuse some addresses for good (a
/// `550`) or cannot be reached for others, and which can cancel a token once
/// a number of messages went out (a shutdown mid-run). `tried` counts every
/// send asked of it, failed or not.
struct Flaky {
    memory: Arc<MemorySender>,
    refused: Mutex<Vec<String>>,
    unreachable: Mutex<Vec<String>>,
    tried: Mutex<Vec<String>>,
    cancel_after: Option<(usize, CancellationToken)>,
}

impl Flaky {
    fn new(memory: Arc<MemorySender>) -> Self {
        Self {
            memory,
            refused: Mutex::default(),
            unreachable: Mutex::default(),
            tried: Mutex::default(),
            cancel_after: None,
        }
    }

    /// The address's server refuses it for good.
    fn refuse(&self, address: &str) {
        self.refused.lock().unwrap().push(address.into());
    }

    /// The address's mail does not get through for now.
    fn unreachable(&self, address: &str) {
        self.unreachable.lock().unwrap().push(address.into());
    }

    fn accept_all(&self) {
        self.refused.lock().unwrap().clear();
        self.unreachable.lock().unwrap().clear();
    }

    /// How often a send to `address` was tried.
    fn tries(&self, address: &str) -> usize {
        self.tried
            .lock()
            .unwrap()
            .iter()
            .filter(|t| *t == address)
            .count()
    }
}

impl MailSender for Flaky {
    fn send<'a>(
        &'a self,
        config: &'a SmtpConfig,
        message: &'a Message,
    ) -> BoxFuture<'a, Result<(), SendError>> {
        let to = message.to.to_string();
        self.tried.lock().unwrap().push(to.clone());
        if self.refused.lock().unwrap().contains(&to) {
            return Box::pin(std::future::ready(Err(SendError::Rejected {
                reply: format!("550 5.1.1 <{to}>: mailbox unavailable"),
            })));
        }
        if self.unreachable.lock().unwrap().contains(&to) {
            return Box::pin(std::future::ready(Err(SendError::Connect {
                host: config.host.clone(),
                port: config.port,
                silent: false,
                detail: "connection timed out".into(),
            })));
        }
        Box::pin(async move {
            self.memory.send(config, message).await?;
            if let Some((after, token)) = &self.cancel_after {
                if self.memory.sent().len() >= *after {
                    token.cancel();
                }
            }
            Ok(())
        })
    }
}

/// Every fixture domain's SPF record.
const SPF: &str = "v=spf1 ip4:192.0.2.0/24 ip4:198.51.100.0/24 -all";

/// 2030-03-01 07:00 UTC: the report day; the month is February 2030.
fn now() -> i64 {
    Utc.with_ymd_and_hms(2030, 3, 1, 7, 0, 0)
        .unwrap()
        .timestamp()
}

struct App {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    state: AppState,
    app: axum::Router,
    sender: Arc<MemorySender>,
    acme: i64,
    beta: i64,
}

/// What belongs to one organization in the fixture.
struct Tenant {
    organization: &'static str,
    domains: [&'static str; 2],
    ip: &'static str,
    reporter: &'static str,
    reporter_email: &'static str,
}

const ACME: Tenant = Tenant {
    organization: "Acme",
    domains: ["acme-mail.example", "acme-shop.example"],
    ip: "192.0.2.10",
    reporter: "google.com",
    reporter_email: "noreply@google.com",
};

const BETA: Tenant = Tenant {
    organization: "Beta Holdings",
    domains: ["beta-mail.example", "beta-news.example"],
    ip: "198.51.100.20",
    reporter: "Yahoo! Inc.",
    reporter_email: "dmarc@yahoo.example",
};

async fn ingest(pool: &SqlitePool, tenant: &Tenant, id: &str, domain: &str, at: i64, p: &str) {
    let mut report = report_with_policy(
        id,
        domain,
        at,
        p,
        p,
        vec![
            rec(tenant.ip, 400, "pass", "pass"),
            rec(tenant.ip, 3, "fail", "fail"),
        ],
    );
    report.org_name = tenant.reporter.into();
    report.email = tenant.reporter_email.into();
    ingest_report(pool, &report).await.expect("ingest");
}

async fn app_with(sender: MemorySender) -> App {
    let sender = Arc::new(sender);
    app_through(sender.clone(), sender).await
}

/// The fixture sending through `mail`, which keeps what it sent in `sender`.
async fn app_through(sender: Arc<MemorySender>, mail: Arc<dyn MailSender>) -> App {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("test.db")).await.unwrap();
    let state = AppState::new(pool.clone()).with_mail_sender(mail);
    let app = router_with_state(state.clone());
    let mut ids = Vec::new();
    for tenant in [&ACME, &BETA] {
        let org = get_or_create_organization(&pool, tenant.organization)
            .await
            .unwrap();
        for domain in tenant.domains {
            get_or_create_domain(&pool, org.id, domain).await.unwrap();
        }
        ids.push(org.id);
    }
    // Reports through February and into March, then changes in February:
    // a baseline on the 5th, a policy change found on the 20th (the DMARC
    // record in DNS; the reports follow it).
    let token = CancellationToken::new();
    for tenant in [&ACME, &BETA] {
        for domain in tenant.domains {
            for days_ago in [60, 40, 30, 25, 20] {
                let id = format!("{domain}-{days_ago}");
                ingest(&pool, tenant, &id, domain, now() - days_ago * DAY, "none").await;
            }
            common::store_dns(&pool, domain, "v=DMARC1; p=none", SPF, now() - 60 * DAY).await;
        }
    }
    evaluate_all(&pool, now() - 24 * DAY, &token).await.unwrap();
    for tenant in [&ACME, &BETA] {
        let domain = tenant.domains[0];
        ingest(
            &pool,
            tenant,
            &format!("{domain}-q"),
            domain,
            now() - 12 * DAY,
            "quarantine",
        )
        .await;
        common::store_dns(
            &pool,
            domain,
            "v=DMARC1; p=quarantine",
            SPF,
            now() - 12 * DAY,
        )
        .await;
    }
    evaluate_all(&pool, now() - 9 * DAY, &token).await.unwrap();
    // The mailbox was read an hour before the report day.
    common::record_mailbox_read(&pool, now() - 3600).await;
    App {
        _dir: dir,
        pool,
        state,
        app,
        sender,
        acme: ids[0],
        beta: ids[1],
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
            username: "reports@msp.example".into(),
            sender: "dmarc@msp.example".into(),
            ..SendingGroup::default()
        },
        password: "secret".into(),
    };
    save_settings(pool, &sending).await.unwrap();
}

async fn enable_reports(a: &App) {
    save_settings(
        &a.pool,
        &ClientReportSettings {
            enabled: true,
            day_of_month: 1,
            hour_utc: 7,
            reply_to: "help@msp.example".into(),
        },
    )
    .await
    .unwrap();
    let (status, body) = put_json(
        &a.app,
        "/api/settings/client-reports/recipients",
        json!({ "text": "Acme,it@acme.example\nBeta Holdings,ops@beta.example\nBeta Holdings,cto@beta.example\n" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// No other organization's domains, name, IPs or reporters, and no IPs or
/// reporters of its own either.
fn assert_isolated(output: &str, own: &Tenant, other: &Tenant) {
    for leak in other
        .domains
        .iter()
        .copied()
        .chain([
            other.ip,
            other.reporter,
            other.reporter_email,
            other.organization,
        ])
        .chain([own.ip, own.reporter, own.reporter_email])
    {
        assert!(
            !output.contains(leak),
            "{leak} appears in {}'s report:\n{output}",
            own.organization
        );
    }
}

#[tokio::test]
async fn each_report_holds_only_its_own_organization() {
    let a = app().await;
    for (id, own, other) in [(a.acme, &ACME, &BETA), (a.beta, &BETA, &ACME)] {
        let content =
            organization_report(&a.pool, id, own.organization, now(), &Letterhead::default())
                .await
                .unwrap();
        for output in [&content.subject, &content.text, &content.html] {
            assert_isolated(output, own, other);
        }
        for domain in own.domains {
            assert!(
                content.text.contains(domain),
                "{domain} missing in\n{}",
                content.text
            );
            assert!(
                content.html.contains(domain),
                "{domain} missing in\n{}",
                content.html
            );
        }
        // The month's change is there: the fixture is not empty.
        assert!(
            content
                .text
                .contains("DMARC policy changed: p none → quarantine."),
            "{}",
            content.text
        );
    }

    // What the job sends is the same, organization by organization.
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    assert!(matches!(run(&a.state, now()).await, Delivery::Sent { .. }));
    let sent = a.sender.sent();
    assert_eq!(sent.len(), 3);
    for mail in &sent {
        let (own, other) = if mail.to.to_string().ends_with("@acme.example") {
            (&ACME, &BETA)
        } else {
            (&BETA, &ACME)
        };
        assert!(
            mail.subject.starts_with(own.organization),
            "{}",
            mail.subject
        );
        assert_isolated(&mail.subject, own, other);
        assert_isolated(&mail.text, own, other);
        assert_isolated(mail.html.as_deref().unwrap(), own, other);
    }
}

#[tokio::test]
async fn the_job_sends_once_a_month() {
    let a = app().await;
    enable_sending(&a.pool).await;
    // Off: nothing, even at the hour.
    assert_eq!(run(&a.state, now()).await, Delivery::NotDue);
    enable_reports(&a).await;

    assert_eq!(run(&a.state, now() - 60).await, Delivery::NotDue);
    assert_eq!(
        run(&a.state, now() + 60).await,
        Delivery::Sent {
            organizations: 2,
            sent: 3,
            failed: 0
        }
    );
    let subjects: Vec<String> = a.sender.sent().iter().map(|m| m.subject.clone()).collect();
    assert_eq!(
        subjects,
        [
            "Acme: DMARC report for February 2030",
            "Beta Holdings: DMARC report for February 2030",
            "Beta Holdings: DMARC report for February 2030",
        ]
    );
    assert_eq!(
        client_reports_last_sent_at(&a.pool).await.unwrap(),
        Some(now() + 60)
    );

    // Not twice that day, nor later in the month.
    assert_eq!(run(&a.state, now() + 3600).await, Delivery::AlreadySent);
    assert_eq!(run(&a.state, now() + 10 * DAY).await, Delivery::AlreadySent);
    assert_eq!(a.sender.sent().len(), 3);

    // Next month's day: again.
    let april = Utc
        .with_ymd_and_hms(2030, 4, 1, 7, 30, 0)
        .unwrap()
        .timestamp();
    common::record_mailbox_read(&a.pool, april - 3600).await;
    assert!(matches!(
        run(&a.state, april).await,
        Delivery::Sent { sent: 3, .. }
    ));
    assert!(a.sender.sent()[3].subject.ends_with("March 2030"));
}

/// Recipient and month of every message sent, in order.
fn sends(sender: &MemorySender) -> Vec<(String, String)> {
    sender
        .sent()
        .iter()
        .map(|m| {
            let month = m.subject.rsplit(" for ").next().unwrap_or_default();
            (m.to.to_string(), month.to_owned())
        })
        .collect()
}

/// A shutdown between two emails stops the run there; the next run sends
/// the rest, and nobody gets the month's report twice.
#[tokio::test]
async fn a_restart_mid_send_sends_each_recipient_once() {
    let memory = Arc::new(MemorySender::default());
    let token = CancellationToken::new();
    let flaky = Flaky {
        cancel_after: Some((1, token.clone())),
        ..Flaky::new(memory.clone())
    };
    let a = app_through(memory, Arc::new(flaky)).await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    assert_eq!(
        run_once(&a.state, now(), &token).await,
        Delivery::Stopped { sent: 1 }
    );
    assert_eq!(a.sender.sent().len(), 1);

    // After the restart: Beta Holdings is still due, Acme is not.
    let settings = AppSettings::load(&a.pool).await.unwrap();
    let at = schedule(&a.pool, &settings, now() + 3600).await.unwrap();
    assert_eq!(at.next_send_at, Some(now()));
    assert_eq!(at.organizations_due, 1);
    assert_eq!(at.last_sent_at, Some(now()));
    assert_eq!(at.next_send_for(a.beta), Some(now()));
    assert!(at.next_send_for(a.acme) > Some(now() + 27 * DAY));

    assert_eq!(
        run(&a.state, now() + 3600).await,
        Delivery::Sent {
            organizations: 1,
            sent: 2,
            failed: 0
        }
    );
    assert_eq!(run(&a.state, now() + 2 * 3600).await, Delivery::AlreadySent);
    let feb = "February 2030".to_owned();
    assert_eq!(
        sends(&a.sender),
        [
            ("it@acme.example".to_owned(), feb.clone()),
            ("cto@beta.example".to_owned(), feb.clone()),
            ("ops@beta.example".to_owned(), feb),
        ]
    );
}

/// The undelivered line of Settings → Client reports at `at`.
async fn undelivered_at(a: &App, at: i64) -> Option<String> {
    let settings = AppSettings::load(&a.pool).await.unwrap();
    let schedule = schedule(&a.pool, &settings, at).await.unwrap();
    undelivered_line(schedule.attempted, &schedule.undelivered)
}

/// A recipient not reached (the server unreachable) is tried again on every
/// hourly run, also after the report day, for three days from the first
/// failure; the others are not sent it again. Once it gets through, the
/// failure is gone.
#[tokio::test]
async fn a_recipient_not_reached_is_retried_for_three_days() {
    let memory = Arc::new(MemorySender::default());
    let flaky = Arc::new(Flaky::new(memory.clone()));
    let a = app_through(memory, flaky.clone()).await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    flaky.unreachable("ops@beta.example");
    assert_eq!(
        run(&a.state, now()).await,
        Delivery::Sent {
            organizations: 2,
            sent: 2,
            failed: 1
        }
    );
    assert_eq!(run(&a.state, now() + 3600).await, Delivery::Failed);
    assert_eq!(
        undelivered_at(&a, now() + 3600).await.as_deref(),
        Some("1 of 3 reports not delivered: ops@beta.example (server unreachable, retrying until 4 March).")
    );

    // Two days later, past the report day: still due for Beta Holdings only.
    let later = now() + 2 * DAY;
    common::record_mailbox_read(&a.pool, later - 3600).await;
    let settings = AppSettings::load(&a.pool).await.unwrap();
    let at = schedule(&a.pool, &settings, later).await.unwrap();
    assert_eq!(at.next_send_for(a.beta), Some(now()));
    assert_eq!(at.organizations_due, 1);
    flaky.accept_all();
    assert_eq!(
        run(&a.state, later).await,
        Delivery::Sent {
            organizations: 1,
            sent: 1,
            failed: 0
        }
    );
    assert_eq!(run(&a.state, later + 3600).await, Delivery::AlreadySent);
    assert_eq!(undelivered_at(&a, later + 3600).await, None);
    let sent = sends(&a.sender);
    assert_eq!(sent.len(), 3);
    assert_eq!(
        sent[2],
        ("ops@beta.example".to_owned(), "February 2030".to_owned())
    );
    assert_eq!(
        client_reports_last_sent_at(&a.pool).await.unwrap(),
        Some(later)
    );
}

/// Three days after its first failure a recipient still not reached is
/// given up for the month: no more tries, and the line says so.
#[tokio::test]
async fn a_recipient_still_not_reached_after_three_days_is_given_up() {
    let memory = Arc::new(MemorySender::default());
    let flaky = Arc::new(Flaky::new(memory.clone()));
    let a = app_through(memory, flaky.clone()).await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    flaky.unreachable("ops@beta.example");
    assert!(matches!(
        run(&a.state, now()).await,
        Delivery::Sent { failed: 1, .. }
    ));
    let last_try = now() + 3 * DAY - 60;
    common::record_mailbox_read(&a.pool, last_try - 3600).await;
    assert_eq!(run(&a.state, last_try).await, Delivery::Failed);
    let over = now() + 3 * DAY;
    assert_eq!(run(&a.state, over).await, Delivery::Undeliverable);
    assert_eq!(run(&a.state, over + 3600).await, Delivery::Undeliverable);
    assert_eq!(flaky.tries("ops@beta.example"), 2);
    let settings = AppSettings::load(&a.pool).await.unwrap();
    assert_eq!(
        schedule(&a.pool, &settings, over)
            .await
            .unwrap()
            .next_send_for(a.beta),
        Some(now() + 31 * DAY),
        "nothing is due for Beta Holdings until April"
    );
    assert_eq!(
        undelivered_at(&a, over).await.as_deref(),
        Some("1 of 3 reports not delivered: ops@beta.example (server unreachable, given up after 3 days).")
    );
}

/// A server that refuses an address for good (a 5xx answer) is not asked
/// again this month.
#[tokio::test]
async fn a_refused_address_is_not_tried_again() {
    let memory = Arc::new(MemorySender::default());
    let flaky = Arc::new(Flaky::new(memory.clone()));
    let a = app_through(memory, flaky.clone()).await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    flaky.refuse("ops@beta.example");
    assert_eq!(
        run(&a.state, now()).await,
        Delivery::Sent {
            organizations: 2,
            sent: 2,
            failed: 1
        }
    );
    assert_eq!(run(&a.state, now() + 3600).await, Delivery::Undeliverable);
    assert_eq!(flaky.tries("ops@beta.example"), 1);
    assert_eq!(
        undelivered_at(&a, now() + 3600).await.as_deref(),
        Some("1 of 3 reports not delivered: ops@beta.example (mailbox does not exist).")
    );
    // Even if its server would take it now: refused for the month.
    flaky.accept_all();
    assert_eq!(
        run(&a.state, now() + 2 * 3600).await,
        Delivery::Undeliverable
    );
    assert_eq!(sends(&a.sender).len(), 2);
}

/// The retries end with the month: at the next report time a recipient
/// that failed is owed the new month's report, not the old one's.
#[tokio::test]
async fn the_month_ends_the_retries() {
    let memory = Arc::new(MemorySender::default());
    let flaky = Arc::new(Flaky::new(memory.clone()));
    let a = app_through(memory, flaky.clone()).await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    flaky.unreachable("ops@beta.example");
    assert!(matches!(
        run(&a.state, now()).await,
        Delivery::Sent { sent: 2, .. }
    ));
    let april = Utc
        .with_ymd_and_hms(2030, 4, 1, 7, 0, 0)
        .unwrap()
        .timestamp();
    common::record_mailbox_read(&a.pool, april - 2 * 3600).await;
    // The last hour of March's: given up long ago.
    assert_eq!(run(&a.state, april - 60).await, Delivery::Undeliverable);
    // April's report time: March's month goes to everyone, February's is over.
    assert!(matches!(
        run(&a.state, april + 60).await,
        Delivery::Sent {
            sent: 2,
            failed: 1,
            ..
        }
    ));
    flaky.accept_all();
    assert_eq!(
        run(&a.state, april + 3600).await,
        Delivery::Sent {
            organizations: 1,
            sent: 1,
            failed: 0
        }
    );
    let ops: Vec<String> = sends(&a.sender)
        .into_iter()
        .filter(|(to, _)| to == "ops@beta.example")
        .map(|(_, month)| month)
        .collect();
    assert_eq!(ops, ["March 2030"]);
}

#[tokio::test]
async fn a_failed_send_is_retried_later_that_day() {
    let a = app_with(MemorySender::failing(SendError::Auth {
        detail: "535 5.7.8 bad credentials".into(),
    }))
    .await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    assert_eq!(run(&a.state, now()).await, Delivery::Failed);
    assert_eq!(client_reports_last_sent_at(&a.pool).await.unwrap(), None);

    // Sending off: nothing is sent and nothing recorded.
    let b = app().await;
    enable_reports(&b).await;
    assert_eq!(run(&b.state, now()).await, Delivery::Failed);
    assert!(b.sender.sent().is_empty());
    enable_sending(&b.pool).await;
    assert!(matches!(
        run(&b.state, now() + 5 * 3600).await,
        Delivery::Sent { .. }
    ));
}

#[tokio::test]
async fn the_preview_goes_to_the_test_recipient_and_needs_sending_on() {
    let a = app().await;
    // Only sending: the monthly reports stay off throughout.
    let settings = dmarc_store::AppSettings::load(&a.pool).await.unwrap();
    assert!(!settings.client_reports.enabled);
    let body = json!({ "organization_id": a.acme });
    let (status, err) = post_json(
        &a.app,
        "/api/settings/client-reports/email/send",
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["reason"], "disabled");
    assert!(a.sender.sent().is_empty());

    enable_sending(&a.pool).await;
    let (status, ok) = post_json(&a.app, "/api/settings/client-reports/email/send", body).await;
    assert_eq!(status, StatusCode::OK, "{ok}");
    assert_eq!(
        ok["message"],
        "Preview of Acme's report sent to dmarc@msp.example."
    );
    let sent = a.sender.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to.to_string(), "dmarc@msp.example");
    assert!(
        sent[0]
            .subject
            .starts_with("[Preview] Acme: DMARC report for "),
        "{}",
        sent[0].subject
    );

    // A saved test recipient takes the sender's place; an address in the
    // request is not one the preview reads.
    let mut settings = dmarc_store::AppSettings::load(&a.pool).await.unwrap();
    settings.sending.group.test_recipient = "me@msp.example".into();
    save_settings(&a.pool, &settings.sending).await.unwrap();
    let (status, ok) = post_json(
        &a.app,
        "/api/settings/client-reports/email/send",
        json!({ "organization_id": a.beta, "to": "elsewhere@example.org" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{ok}");
    assert_eq!(a.sender.sent()[1].to.to_string(), "me@msp.example");
    let (status, _) = post_json(
        &a.app,
        "/api/settings/client-reports/email/send",
        json!({ "organization_id": 999 }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_preview_uses_the_reply_to_on_screen() {
    let a = app().await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    let url = "/api/settings/client-reports/email/send";
    let (status, body) = post_json(&a.app, url, json!({ "organization_id": a.acme })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = post_json(
        &a.app,
        url,
        json!({ "organization_id": a.acme, "reply_to": " desk@msp.example " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = post_json(
        &a.app,
        url,
        json!({ "organization_id": a.acme, "reply_to": "desk" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply_tos: Vec<Option<String>> = a
        .sender
        .configs()
        .iter()
        .map(|c| c.reply_to.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(
        reply_tos,
        [
            Some("help@msp.example".to_owned()),
            Some("desk@msp.example".to_owned())
        ]
    );
}

/// The client report opens by the license's standing: Marcan first and the
/// operator "Managed by" without a key, the operator alone with one; the
/// From name follows.
#[tokio::test]
async fn the_preview_opens_with_marcan_without_a_key_and_the_operator_alone_with_one() {
    let a = app().await;
    enable_sending(&a.pool).await;
    let branding = dmarc_store::BrandingSettings {
        operator_name: "Acme IT".into(),
    };
    save_settings(&a.pool, &branding).await.unwrap();
    let url = "/api/settings/client-reports/email/send";
    let (status, body) = post_json(&a.app, url, json!({ "organization_id": a.beta })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let unlicensed = &a.sender.sent()[0];
    assert!(
        unlicensed.text.starts_with(
            "Marcan, free software\nManaged by Acme IT\n\nDMARC report for Beta Holdings, "
        ),
        "{}",
        unlicensed.text
    );
    let html = unlicensed.html.as_deref().unwrap();
    assert!(html.contains("<img src=\"cid:marcan-mark\""), "{html}");
    assert!(html.contains(">Managed by Acme IT</p>"), "{html}");
    let cids: Vec<&str> = unlicensed.images.iter().map(|i| i.cid).collect();
    assert_eq!(cids, ["marcan-mark"]);

    let license = dmarc_store::LicenseSettings {
        operator_key: Some(common::TEST_OPERATOR_KEY.into()),
    };
    save_settings(&a.pool, &license).await.unwrap();
    let (status, body) = post_json(&a.app, url, json!({ "organization_id": a.beta })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let licensed = &a.sender.sent()[1];
    assert!(
        licensed
            .text
            .starts_with("Acme IT\n\nDMARC report for Beta Holdings, "),
        "{}",
        licensed.text
    );
    let html = licensed.html.as_deref().unwrap();
    assert!(html.contains("<p style=\"margin:0 0 4px;font-size:20px;font-weight:700\">Acme IT</p>"));
    // Nothing names Marcan, the subject included.
    assert!(!licensed.text.contains("Marcan"), "{}", licensed.text);
    assert!(!html.contains("Marcan"), "{html}");
    assert!(licensed.images.is_empty());
    assert_eq!(a.sender.from_names(), ["Acme IT via Marcan", "Acme IT"]);
}

/// The in-app preview renders the report as it would go out now, with its
/// inline images as data URLs, and needs neither sending nor the reports on.
#[tokio::test]
async fn the_preview_is_shown_without_sending_anything() {
    let a = app().await;
    let branding = dmarc_store::BrandingSettings {
        operator_name: "Acme IT".into(),
    };
    save_settings(&a.pool, &branding).await.unwrap();
    let url = format!(
        "/api/settings/client-reports/email?organization_id={}",
        a.acme
    );
    let (status, body) = common::get(&a.app, &url).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["subject"]
            .as_str()
            .unwrap()
            .starts_with("Acme: DMARC report for "),
        "{body}"
    );
    assert_eq!(body["from_name"], "Acme IT via Marcan");
    let html = body["html"].as_str().unwrap();
    assert!(html.contains("<img src=\"data:image/png;base64,"), "{html}");
    assert!(!html.contains("cid:"), "{html}");
    let text = body["text"].as_str().unwrap();
    assert!(
        text.starts_with("Marcan, free software\nManaged by Acme IT\n"),
        "{text}"
    );
    for domain in ACME.domains {
        assert!(text.contains(domain), "{text}");
    }
    assert_isolated(text, &ACME, &BETA);
    assert!(a.sender.sent().is_empty());

    let (status, _) = common::get(
        &a.app,
        "/api/settings/client-reports/email?organization_id=999",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The settings say when the reports go out next, to how many
/// organizations and when they last went out; the organization page's
/// endpoint says it for one organization, only while it will get one.
#[tokio::test]
async fn the_schedule_is_on_the_settings_and_the_organization() {
    let a = app().await;
    let client_report = |id: i64| {
        let app = a.app.clone();
        async move {
            let (status, body) =
                common::get(&app, &format!("/api/organizations/{id}/client-report")).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body
        }
    };
    let reports = common::get_ok(&a.app, "/api/settings").await["client_reports"].clone();
    assert_eq!(reports["next_send_at"], serde_json::Value::Null);
    assert_eq!(reports["organizations_due"], 0);
    assert_eq!(reports["last_sent_at"], serde_json::Value::Null);

    enable_reports(&a).await;
    let reports = common::get_ok(&a.app, "/api/settings").await["client_reports"].clone();
    assert_eq!(reports["organizations_due"], 2);
    let next = reports["next_send_at"].as_i64().expect("a next send");
    let today = dmarc_store::unix_now();
    assert!(next > today - DAY && next <= today + 32 * DAY, "{next}");
    let at = Utc.timestamp_opt(next, 0).unwrap();
    assert_eq!(at.format("%d %H:%M").to_string(), "01 07:00");
    // Sending off: no organization hears of a report.
    assert_eq!(client_report(a.acme).await, serde_json::Value::Null);

    enable_sending(&a.pool).await;
    assert_eq!(
        client_report(a.beta).await,
        json!({ "next_send_at": next, "recipients": 2, "held": false })
    );
    assert_eq!(client_report(a.acme).await["recipients"], 1);
    let unlisted = get_or_create_organization(&a.pool, "Initech")
        .await
        .unwrap();
    assert_eq!(client_report(unlisted.id).await, serde_json::Value::Null);
    let (status, _) = common::get(&a.app, "/api/organizations/999/client-report").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    assert!(matches!(run(&a.state, now()).await, Delivery::Sent { .. }));
    let reports = common::get_ok(&a.app, "/api/settings").await["client_reports"].clone();
    assert_eq!(reports["last_sent_at"], now());
    assert_eq!(reports["held_reason"], serde_json::Value::Null);

    // Off: no next send, and no organization hears of one.
    let mut settings = dmarc_store::AppSettings::load(&a.pool).await.unwrap();
    settings.client_reports.enabled = false;
    save_settings(&a.pool, &settings.client_reports)
        .await
        .unwrap();
    let reports = common::get_ok(&a.app, "/api/settings").await["client_reports"].clone();
    assert_eq!(reports["next_send_at"], serde_json::Value::Null);
    assert_eq!(client_report(a.beta).await, serde_json::Value::Null);
}

/// While the mailbox has not been read for three days, no report goes out:
/// the month stays due past its day and goes out on the first run that
/// finds the mailbox read again, once.
#[tokio::test]
async fn the_reports_are_held_while_the_mailbox_is_not_read() {
    let a = app().await;
    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    let april = Utc
        .with_ymd_and_hms(2030, 4, 1, 7, 0, 0)
        .unwrap()
        .timestamp();
    // The fixture's last read is an hour before 1 March: a month old.
    let last_read_at = Some(now() - 3600);
    let held = Delivery::Held(ClientReportHold::MailboxNotRead { last_read_at });
    assert_eq!(run(&a.state, april).await, held);
    assert!(a.sender.sent().is_empty());
    let settings = AppSettings::load(&a.pool).await.unwrap();
    let at = schedule(&a.pool, &settings, april + 3600).await.unwrap();
    assert_eq!(
        at.held,
        Some(ClientReportHold::MailboxNotRead { last_read_at })
    );
    assert_eq!(at.next_send_at, Some(april));

    // Past the report day: still held, still due.
    assert_eq!(run(&a.state, april + 3 * DAY).await, held);
    assert!(a.sender.sent().is_empty());

    // Read again: the next run sends March's reports, once.
    let read = april + 4 * DAY;
    common::record_mailbox_read(&a.pool, read).await;
    let at = schedule(&a.pool, &settings, read + 60).await.unwrap();
    assert_eq!(at.held, None);
    assert_eq!(at.next_send_at, Some(april));
    assert_eq!(
        run(&a.state, read + 3600).await,
        Delivery::Sent {
            organizations: 2,
            sent: 3,
            failed: 0
        }
    );
    assert!(a
        .sender
        .sent()
        .iter()
        .all(|m| m.subject.ends_with("March 2030")));
    assert_eq!(run(&a.state, read + 2 * 3600).await, Delivery::AlreadySent);
    assert_eq!(run(&a.state, read + 5 * DAY).await, Delivery::AlreadySent);
    let at = schedule(&a.pool, &settings, read + 5 * DAY).await.unwrap();
    assert_eq!(at.held, None);
    assert_eq!(at.last_sent_at, Some(read + 3600));
    assert_eq!(a.sender.sent().len(), 3);
}

/// The report reads the organization's calendar month against the month
/// before, then a block for each domain with something to say; the job
/// sends what the preview renders.
#[tokio::test]
async fn the_report_reads_the_month_and_words_what_needs_attention() {
    let a = app().await;
    // The fixture's acme-shop.example last reported between 7 and 14 days
    // before now: flagged at the 7-day warning this test is written
    // against, not at the default.
    save_settings(
        &a.pool,
        &dmarc_store::MonitoringSettings {
            domain_reports_stopped_days: 7,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let content = organization_report(&a.pool, a.acme, "Acme", now(), &Letterhead::default())
        .await
        .unwrap();
    let text = &content.text;
    // Both domains: one changed its policy in February, the other is at p=none.
    assert!(
        text.contains("\n\nDMARC report for Acme, February 2030\n\n2 of 2 domains need attention.\n\nIn February 2030, receivers reported 2,015 messages, up 25.0% on January: 2,000 passed DMARC (up 25.0%), 0 were blocked by policy (unchanged), 0 were forwarded (unchanged) and 15 failed but were delivered (up 25.0%).\n\n"),
        "{text}"
    );
    for line in [
        "\n\nacme-shop.example\n  Attention   ",
        "  Changes     1 change in February 2030:\n              - DMARC policy changed: p none → quarantine.\n",
    ] {
        assert!(text.contains(line), "missing {line:?} in\n{text}");
    }
    assert!(!text.contains("protected and unchanged"), "{text}");
    assert!(!text.contains("parked"), "{text}");

    enable_sending(&a.pool).await;
    enable_reports(&a).await;
    assert!(matches!(run(&a.state, now()).await, Delivery::Sent { .. }));
    let sent = a.sender.sent();
    let acme = sent
        .iter()
        .find(|m| m.to.to_string() == "it@acme.example")
        .expect("Acme's report");
    assert_eq!(&acme.text, text);
}
