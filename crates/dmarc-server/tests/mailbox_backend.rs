//! `StoreBackend`: the IMAP worker's `Backend` over the store, round-tripped
//! through a real database.

mod common;

use common::{rec, report, TestApp};
use dmarc_mailbox::{
    AttemptKey, AttemptKind, Backend, Cursor, FetchSummary, ImapConfig, ImapSecurity, Ingested,
    MailboxConfig, MailboxRef, MessageRef, PassRecord, DEFAULT_POLL_INTERVAL_SECS,
};
use dmarc_store::{
    get_default_organization, get_or_create_domain, ignore_domain, insert_account,
    receive_pass_status, save_settings, update_account, update_account_receiving, AccountFields,
    ImapSecurity as StoredSecurity, LoginKind, ReceiveChange, ReceivingSettings, SecretKind,
    SecretRow, SecretWrite,
};
use sqlx::SqlitePool;
use std::time::Duration;

/// Account 1's mailbox id.
const MAILBOX: &str = "1";
const FOLDER: &str = "INBOX";

/// Account 1, which the receive state and the passes below belong to.
async fn with_account(pool: &SqlitePool) {
    insert_account(pool, Some(1), &AccountFields::default(), &[], 0)
        .await
        .unwrap();
}

/// IMAP's spelling: the UIDVALIDITY and the UID in decimal.
fn message(generation: &'static str, message_id: &'static str) -> MessageRef<'static> {
    MessageRef {
        mailbox: MAILBOX,
        epoch: 0,
        folder: FOLDER,
        generation,
        message_id,
    }
}

fn key(generation: &str, kind: AttemptKind) -> AttemptKey {
    AttemptKey {
        mailbox: MAILBOX.into(),
        epoch: 0,
        folder: FOLDER.into(),
        generation: generation.into(),
        kind,
    }
}

/// An IMAP cursor: the UIDVALIDITY and the last UID in decimal.
fn cursor(generation: &str, position: &str) -> Cursor {
    Cursor {
        generation: generation.into(),
        position: position.into(),
    }
}

fn pass(started_at: i64, checked_at: Option<i64>, error: Option<&str>) -> PassRecord {
    PassRecord {
        mailbox: MAILBOX.into(),
        epoch: Some(0),
        started_at,
        finished_at: started_at + 1,
        checked_at,
        summary: FetchSummary {
            messages_seen: 3,
            reports_ingested: 2,
            ..FetchSummary::default()
        },
        error: error.map(str::to_owned),
        error_detail: None,
    }
}

/// Each account that receives reports is listed under its id once its login
/// is complete, in id order, and prepared under that id.
#[tokio::test]
async fn receiving_accounts_with_a_complete_login_are_the_mailboxes() {
    let t = TestApp::new().await;
    let backend = t.state.mailbox_backend();
    assert_eq!(backend.mailboxes().await.unwrap(), []);

    let mut fields = AccountFields {
        name: "Reports".into(),
        imap_host: "imap.example.com".into(),
        username: "u".into(),
        ..AccountFields::default()
    };
    insert_account(&t.pool, Some(1), &fields, &[], 0)
        .await
        .unwrap();
    // No password yet.
    assert_eq!(backend.mailboxes().await.unwrap(), []);
    assert_eq!(backend.prepare("1").await.unwrap(), None);

    fields.imap_port = 143;
    fields.imap_security = StoredSecurity::StartTls;
    fields.folder = "DMARC".into();
    let password = SecretRow::plain(SecretKind::Password, " p ");
    update_account(
        &t.pool,
        1,
        &fields,
        &[(SecretKind::Password, SecretWrite::Set(password))],
    )
    .await
    .unwrap();
    let listed = MailboxRef {
        id: "1".into(),
        poll: Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS),
    };
    assert_eq!(
        backend.mailboxes().await.unwrap(),
        std::slice::from_ref(&listed)
    );
    assert_eq!(
        backend.prepare("1").await.unwrap(),
        Some(MailboxConfig::Imap(ImapConfig {
            id: "1".into(),
            epoch: 0,
            host: "imap.example.com".into(),
            port: 143,
            security: ImapSecurity::StartTls,
            username: "u".into(),
            password: " p ".into(),
            folder: "DMARC".into(),
        }))
    );
    // An id that names no account (0.3's "host|port|username") is not prepared.
    assert_eq!(
        backend.prepare("imap.example.com|993|u").await.unwrap(),
        None
    );
    assert_eq!(backend.prepare("7").await.unwrap(), None);

    // Another account that receives is listed after it; one that does not
    // receive, or has an incomplete Microsoft login, is not.
    let second = AccountFields {
        name: "Second".into(),
        ..fields.clone()
    };
    let secret = || vec![SecretRow::plain(SecretKind::Password, "q")];
    insert_account(&t.pool, Some(2), &second, &secret(), 0)
        .await
        .unwrap();
    let off = AccountFields {
        receive_enabled: false,
        ..second.clone()
    };
    insert_account(&t.pool, Some(3), &off, &secret(), 0)
        .await
        .unwrap();
    let microsoft = AccountFields {
        login_kind: LoginKind::Microsoft,
        ..second.clone()
    };
    insert_account(&t.pool, Some(4), &microsoft, &secret(), 0)
        .await
        .unwrap();
    let ids: Vec<String> = backend
        .mailboxes()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["1", "2"]);
    assert_eq!(backend.prepare("3").await.unwrap(), None);
    assert_eq!(backend.prepare("4").await.unwrap(), None);

    // One poll interval for all of them.
    save_settings(
        &t.pool,
        &ReceivingSettings {
            poll_interval_secs: 120,
        },
    )
    .await
    .unwrap();
    assert!(backend
        .mailboxes()
        .await
        .unwrap()
        .iter()
        .all(|m| m.poll == Duration::from_secs(120)));
}

#[tokio::test]
async fn cursor_save_forgets_other_generations_and_reset_forgets_all() {
    let t = TestApp::new().await;
    with_account(&t.pool).await;
    let backend = t.state.mailbox_backend();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), None);

    let old = cursor("1", "10");
    backend
        .save_cursor(MAILBOX, 0, FOLDER, old.clone())
        .await
        .unwrap();
    backend
        .quarantine(message("1", "7"), "old generation", 100)
        .await
        .unwrap();
    backend
        .record_attempt(&key("1", AttemptKind::Message("9".into())), 100)
        .await
        .unwrap();
    // Another folder's rows are not this folder's business.
    backend
        .quarantine(
            MessageRef {
                folder: "Archive",
                ..message("1", "7")
            },
            "other folder",
            100,
        )
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), Some(old));
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, FOLDER, "1")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        backend
            .attempts(&key("1", AttemptKind::Message("9".into())))
            .await
            .unwrap(),
        1
    );

    // UIDVALIDITY changed: the old generation's quarantine and attempts go.
    let new = cursor("2", "0");
    backend
        .save_cursor(MAILBOX, 0, FOLDER, new.clone())
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), Some(new));
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, FOLDER, "2")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        backend
            .attempts(&key("1", AttemptKind::Message("9".into())))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, "Archive", "1")
            .await
            .unwrap(),
        1
    );

    backend
        .quarantine(message("2", "3"), "new generation", 200)
        .await
        .unwrap();
    backend
        .record_attempt(&key("2", AttemptKind::Batch("5".into())), 200)
        .await
        .unwrap();
    backend.reset_folder(MAILBOX, FOLDER).await.unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), None);
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, FOLDER, "2")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        backend
            .attempts(&key("2", AttemptKind::Batch("5".into())))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn attempts_count_up_per_key_and_clear() {
    let t = TestApp::new().await;
    with_account(&t.pool).await;
    let backend = t.state.mailbox_backend();
    let msg = key("1", AttemptKind::Message("4".into()));
    let batch = key("1", AttemptKind::Batch("4".into()));
    assert_eq!(backend.attempts(&msg).await.unwrap(), 0);
    assert_eq!(backend.record_attempt(&msg, 1).await.unwrap(), 1);
    assert_eq!(backend.record_attempt(&msg, 2).await.unwrap(), 2);
    // Same UID, different kind: a count of its own.
    assert_eq!(backend.record_attempt(&batch, 3).await.unwrap(), 1);
    assert_eq!(backend.attempts(&msg).await.unwrap(), 2);
    // Survives a fresh backend over the same database (a restart).
    let again = t.state.mailbox_backend();
    assert_eq!(again.attempts(&msg).await.unwrap(), 2);

    backend.clear_attempts(&msg).await.unwrap();
    assert_eq!(backend.attempts(&msg).await.unwrap(), 0);
    assert_eq!(backend.attempts(&batch).await.unwrap(), 1);
}

#[tokio::test]
async fn ingest_maps_the_store_outcomes() {
    let t = TestApp::new().await;
    let backend = t.state.mailbox_backend();
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
    let backend = t.state.mailbox_backend();
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
    let backend = t.state.mailbox_backend();
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
    with_account(&t.pool).await;
    let backend = t.state.mailbox_backend();
    let empty = receive_pass_status(&t.pool, 1).await.unwrap();
    assert_eq!(empty.last_check_at, None);
    assert_eq!(empty.last_success_at, None);
    assert_eq!(empty.last_error, None);

    // Could not connect: no check, an error.
    backend
        .record_pass(&pass(100, None, Some("connect failed")))
        .await
        .unwrap();
    let s = receive_pass_status(&t.pool, 1).await.unwrap();
    assert_eq!(s.last_check_at, None);
    assert_eq!(s.last_success_at, None);
    assert_eq!(s.last_error, Some(("connect failed".into(), 101)));

    // A success: the error is gone.
    backend
        .record_pass(&pass(200, Some(200), None))
        .await
        .unwrap();
    let s = receive_pass_status(&t.pool, 1).await.unwrap();
    assert_eq!(s.last_check_at, Some(200));
    assert_eq!(s.last_success_at, Some(201));
    assert_eq!(s.last_error, None);

    // A failure after the search: the check time moves, the success stays.
    backend
        .record_pass(&pass(300, Some(300), Some("batch timed out")))
        .await
        .unwrap();
    let s = receive_pass_status(&t.pool, 1).await.unwrap();
    assert_eq!(s.last_check_at, Some(300));
    assert_eq!(s.last_success_at, Some(201));
    assert_eq!(s.last_error, Some(("batch timed out".into(), 301)));

    // A pass that gave way to another mailbox: checked, no error, but the
    // mailbox is not read up to date, so no success.
    let mut yielded = pass(400, Some(400), None);
    yielded.summary.yielded = true;
    backend.record_pass(&yielded).await.unwrap();
    let s = receive_pass_status(&t.pool, 1).await.unwrap();
    assert_eq!(s.last_check_at, Some(400));
    assert_eq!(s.last_success_at, Some(201));
    assert_eq!(s.last_error, None);
}

/// A pass still running when a save made its account read another mailbox
/// (the save forgot the receive state and moved the epoch on) writes nothing
/// more: no old cursor, skipped message, retry count or pass comes back. The
/// next pass, prepared under the new epoch (by this backend or one made
/// after a restart: it keeps nothing per account), writes again; after the
/// account is deleted, the writes are quiet no-ops, not foreign-key errors.
#[tokio::test]
async fn a_pass_of_a_replaced_or_deleted_account_writes_nothing() {
    let t = TestApp::new().await;
    let backend = t.state.mailbox_backend();
    let fields = AccountFields {
        name: "Reports".into(),
        imap_host: "imap.example.com".into(),
        username: "u".into(),
        ..AccountFields::default()
    };
    let password = vec![SecretRow::plain(SecretKind::Password, "p")];
    insert_account(&t.pool, Some(1), &fields, &password, 0)
        .await
        .unwrap();
    let old = backend.prepare(MAILBOX).await.unwrap().unwrap();
    assert_eq!(old.epoch(), 0);
    let saved = cursor("7", "42");
    backend
        .save_cursor(MAILBOX, old.epoch(), FOLDER, saved.clone())
        .await
        .unwrap();
    assert_eq!(
        backend.cursor(MAILBOX, FOLDER).await.unwrap(),
        Some(saved.clone())
    );

    // Saved with another folder while the pass runs.
    let moved = AccountFields {
        folder: "DMARC".into(),
        ..fields.clone()
    };
    update_account_receiving(&t.pool, 1, &moved, &[], ReceiveChange::Replaced { at: 1 })
        .await
        .unwrap();
    let later = cursor("7", "50");
    backend
        .save_cursor(MAILBOX, old.epoch(), FOLDER, later.clone())
        .await
        .unwrap();
    backend
        .quarantine(message("7", "43"), "bad", 1)
        .await
        .unwrap();
    let attempt = key("7", AttemptKind::Message("44".into()));
    assert_eq!(backend.record_attempt(&attempt, 1).await.unwrap(), 0);
    backend
        .record_pass(&pass(10, Some(10), None))
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, FOLDER).await.unwrap(), None);
    assert_eq!(
        backend
            .quarantined_count(MAILBOX, FOLDER, "7")
            .await
            .unwrap(),
        0
    );
    assert_eq!(backend.attempts(&attempt).await.unwrap(), 0);
    assert_eq!(
        receive_pass_status(&t.pool, 1)
            .await
            .unwrap()
            .last_success_at,
        None
    );

    // The next pass reads the new folder under the next epoch, whichever
    // backend prepares it.
    let fresh = t.state.mailbox_backend();
    let cfg = fresh.prepare(MAILBOX).await.unwrap().unwrap();
    assert_eq!((cfg.folder(), cfg.epoch()), ("DMARC", 1));
    assert_eq!(backend.prepare(MAILBOX).await.unwrap(), Some(cfg.clone()));
    backend
        .save_cursor(MAILBOX, cfg.epoch(), "DMARC", saved.clone())
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, "DMARC").await.unwrap(), Some(saved));

    // Deleted while it runs.
    dmarc_store::delete_account(&t.pool, 1).await.unwrap();
    backend
        .save_cursor(MAILBOX, cfg.epoch(), "DMARC", later)
        .await
        .unwrap();
    backend
        .record_pass(&PassRecord {
            epoch: None,
            ..pass(20, Some(20), None)
        })
        .await
        .unwrap();
    assert_eq!(backend.cursor(MAILBOX, "DMARC").await.unwrap(), None);
}
