//! `StoreBackend`: the IMAP worker's `Backend` over the store, round-tripped
//! through a real database.

mod common;

use common::{rec, report, TestApp};
use dmarc_imap::{
    AttemptKey, AttemptKind, Backend, Cursor, FetchSummary, ImapConfig, ImapSecurity, Ingested,
    MessageRef, PassRecord, DEFAULT_FOLDER, DEFAULT_POLL_INTERVAL_SECS, DEFAULT_PORT,
};
use dmarc_server::stored_imap_config;
use dmarc_store::{
    get_default_organization, get_or_create_domain, ignore_domain, imap_pass_status, save_settings,
    AppSettings, ImapSecurity as StoredSecurity, MailboxSettings,
};

const MAILBOX: &str = "imap.example.com|993|u";
const FOLDER: &str = "INBOX";

fn message(uid_validity: u32, uid: u32) -> MessageRef<'static> {
    MessageRef {
        mailbox: MAILBOX,
        folder: FOLDER,
        uid_validity,
        uid,
    }
}

fn key(uid_validity: u32, kind: AttemptKind) -> AttemptKey {
    AttemptKey {
        mailbox: MAILBOX.into(),
        folder: FOLDER.into(),
        uid_validity,
        kind,
    }
}

fn pass(started_at: i64, checked_at: Option<i64>, error: Option<&str>) -> PassRecord {
    PassRecord {
        started_at,
        finished_at: started_at + 1,
        checked_at,
        summary: FetchSummary {
            messages_seen: 3,
            reports_ingested: 2,
            ..FetchSummary::default()
        },
        error: error.map(str::to_owned),
    }
}

#[tokio::test]
async fn config_needs_host_username_and_password_and_applies_defaults() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    assert_eq!(backend.config().await.unwrap(), None);

    let mut mailbox = MailboxSettings {
        host: "imap.example.com".into(),
        username: "u".into(),
        ..MailboxSettings::default()
    };
    save_settings(&t.pool, &mailbox).await.unwrap();
    assert_eq!(backend.config().await.unwrap(), None);
    let partial = stored_imap_config(&AppSettings::load(&t.pool).await.unwrap().mailbox);
    assert_eq!(partial.host, "imap.example.com");
    assert_eq!(partial.port, DEFAULT_PORT);
    assert_eq!(partial.folder, DEFAULT_FOLDER);
    assert_eq!(partial.poll_interval_secs, DEFAULT_POLL_INTERVAL_SECS);
    assert!(!partial.is_complete());

    mailbox.password = " p ".into();
    mailbox.port = Some(143);
    mailbox.security = Some(StoredSecurity::StartTls);
    mailbox.folder = Some("DMARC".into());
    save_settings(&t.pool, &mailbox).await.unwrap();
    assert_eq!(
        backend.config().await.unwrap(),
        Some(ImapConfig {
            host: "imap.example.com".into(),
            port: 143,
            security: ImapSecurity::StartTls,
            username: "u".into(),
            password: " p ".into(),
            folder: "DMARC".into(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
        })
    );
}

#[tokio::test]
async fn cursor_save_forgets_other_generations_and_reset_forgets_all() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), None);

    let old = Cursor {
        uid_validity: 1,
        last_uid: 10,
    };
    backend.save_cursor(MAILBOX, FOLDER, old).await.unwrap();
    backend
        .quarantine(message(1, 7), "old generation", 100)
        .await
        .unwrap();
    backend
        .record_attempt(&key(1, AttemptKind::Message(9)), 100)
        .await
        .unwrap();
    // Another folder's rows are not this folder's business.
    backend
        .quarantine(
            MessageRef {
                folder: "Archive",
                ..message(1, 7)
            },
            "other folder",
            100,
        )
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), Some(old));
    assert_eq!(
        backend.quarantined_count(MAILBOX, FOLDER, 1).await.unwrap(),
        1
    );
    assert_eq!(
        backend
            .attempts(&key(1, AttemptKind::Message(9)))
            .await
            .unwrap(),
        1
    );

    // UIDVALIDITY changed: the old generation's quarantine and attempts go.
    let new = Cursor {
        uid_validity: 2,
        last_uid: 0,
    };
    backend.save_cursor(MAILBOX, FOLDER, new).await.unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), Some(new));
    assert_eq!(
        backend.quarantined_count(MAILBOX, FOLDER, 2).await.unwrap(),
        0
    );
    assert_eq!(
        backend
            .attempts(&key(1, AttemptKind::Message(9)))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, "Archive", 1)
            .await
            .unwrap(),
        1
    );

    backend
        .quarantine(message(2, 3), "new generation", 200)
        .await
        .unwrap();
    backend
        .record_attempt(&key(2, AttemptKind::Batch(5)), 200)
        .await
        .unwrap();
    backend.reset_folder(MAILBOX, FOLDER).await.unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), None);
    assert_eq!(
        backend.quarantined_count(MAILBOX, FOLDER, 2).await.unwrap(),
        0
    );
    assert_eq!(
        backend
            .attempts(&key(2, AttemptKind::Batch(5)))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn attempts_count_up_per_key_and_clear() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    let msg = key(1, AttemptKind::Message(4));
    let batch = key(1, AttemptKind::Batch(4));
    assert_eq!(backend.attempts(&msg).await.unwrap(), 0);
    assert_eq!(backend.record_attempt(&msg, 1).await.unwrap(), 1);
    assert_eq!(backend.record_attempt(&msg, 2).await.unwrap(), 2);
    // Same UID, different kind: a count of its own.
    assert_eq!(backend.record_attempt(&batch, 3).await.unwrap(), 1);
    assert_eq!(backend.attempts(&msg).await.unwrap(), 2);
    // Survives a fresh backend over the same database (a restart).
    let again = t.state.imap_backend();
    assert_eq!(again.attempts(&msg).await.unwrap(), 2);

    backend.clear_attempts(&msg).await.unwrap();
    assert_eq!(backend.attempts(&msg).await.unwrap(), 0);
    assert_eq!(backend.attempts(&batch).await.unwrap(), 1);
}

#[tokio::test]
async fn ingest_maps_the_store_outcomes() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    let good = report(
        "r-1",
        "example.com",
        1_700_000_000,
        vec![rec("192.0.2.1", 1, "pass", "pass")],
    );
    assert_eq!(backend.ingest(&good).await.unwrap(), Ingested::Inserted);
    assert_eq!(backend.ingest(&good).await.unwrap(), Ingested::Duplicate);

    let org = get_default_organization(&t.pool).await.unwrap();
    let domain = get_or_create_domain(&t.pool, org.id, "ignored.example")
        .await
        .unwrap();
    assert!(ignore_domain(&t.pool, domain.id).await.unwrap());
    let dropped = report(
        "r-2",
        "ignored.example",
        1_700_000_000,
        vec![rec("192.0.2.1", 1, "pass", "pass")],
    );
    assert_eq!(backend.ingest(&dropped).await.unwrap(), Ingested::Ignored);
}

/// A batch is one transaction with the same answers as one report at a
/// time; a failure that is not the report's stores nothing of the batch.
#[tokio::test]
async fn ingest_batch_stores_a_batch_at_once_or_nothing() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    let org = get_default_organization(&t.pool).await.unwrap();
    let domain = get_or_create_domain(&t.pool, org.id, "ignored.example")
        .await
        .unwrap();
    assert!(ignore_domain(&t.pool, domain.id).await.unwrap());
    let one = |id: &str, domain: &str| {
        report(
            id,
            domain,
            1_700_000_000,
            vec![rec("192.0.2.1", 1, "pass", "pass")],
        )
    };
    let (a, b, dropped) = (
        one("r-1", "example.com"),
        one("r-2", "other.example"),
        one("r-3", "ignored.example"),
    );
    assert_eq!(
        backend.ingest_batch(&[&a, &b, &a, &dropped]).await.unwrap(),
        [
            Ingested::Inserted,
            Ingested::Inserted,
            Ingested::Duplicate,
            Ingested::Ignored
        ]
    );

    sqlx::query("CREATE UNIQUE INDEX test_one_report_per_domain ON reports(domain_id)")
        .execute(&t.pool)
        .await
        .unwrap();
    let (c, d) = (one("r-4", "third.example"), one("r-5", "example.com"));
    let err = backend
        .ingest_batch(&[&c, &d])
        .await
        .expect_err("a constraint violation fails the batch");
    assert!(err.to_string().contains("UNIQUE"), "{err}");
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports")
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(stored, 2, "nothing of the failed batch is stored");
}

/// A database constraint the report runs into is the database's fault (a
/// schema bug, say), not the report's: the ingest fails, so the pass retries
/// the message instead of quarantining it for good.
#[tokio::test]
async fn a_constraint_violation_is_retried_not_rejected() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    // A constraint no report could satisfy twice: one report per domain.
    sqlx::query("CREATE UNIQUE INDEX test_one_report_per_domain ON reports(domain_id)")
        .execute(&t.pool)
        .await
        .unwrap();
    let first = report(
        "r-1",
        "example.com",
        1_700_000_000,
        vec![rec("192.0.2.1", 1, "pass", "pass")],
    );
    assert_eq!(backend.ingest(&first).await.unwrap(), Ingested::Inserted);
    let second = report(
        "r-2",
        "example.com",
        1_700_086_400,
        vec![rec("192.0.2.1", 1, "pass", "pass")],
    );
    let err = backend
        .ingest(&second)
        .await
        .expect_err("a constraint violation fails the ingest");
    assert!(err.to_string().contains("UNIQUE"), "{err}");
}

#[tokio::test]
async fn pass_status_picks_the_right_rows() {
    let t = TestApp::new().await;
    let backend = t.state.imap_backend();
    let empty = imap_pass_status(&t.pool).await.unwrap();
    assert_eq!(empty.last_check_at, None);
    assert_eq!(empty.last_success_at, None);
    assert_eq!(empty.last_error, None);

    // Could not connect: no check, an error.
    backend
        .record_pass(&pass(100, None, Some("connect failed")))
        .await
        .unwrap();
    let s = imap_pass_status(&t.pool).await.unwrap();
    assert_eq!(s.last_check_at, None);
    assert_eq!(s.last_success_at, None);
    assert_eq!(s.last_error, Some(("connect failed".into(), 101)));

    // A success: the error is gone.
    backend
        .record_pass(&pass(200, Some(200), None))
        .await
        .unwrap();
    let s = imap_pass_status(&t.pool).await.unwrap();
    assert_eq!(s.last_check_at, Some(200));
    assert_eq!(s.last_success_at, Some(201));
    assert_eq!(s.last_error, None);

    // A failure after the search: the check time moves, the success stays.
    backend
        .record_pass(&pass(300, Some(300), Some("batch timed out")))
        .await
        .unwrap();
    let s = imap_pass_status(&t.pool).await.unwrap();
    assert_eq!(s.last_check_at, Some(300));
    assert_eq!(s.last_success_at, Some(201));
    assert_eq!(s.last_error, Some(("batch timed out".into(), 301)));
}
