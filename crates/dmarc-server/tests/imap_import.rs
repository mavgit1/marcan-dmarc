//! The import end to end: a pass of the real IMAP worker against the scripted
//! fake server (`dmarc-imap`'s `test-support` feature), through the real
//! `StoreBackend` into the real store.

use axum::http::StatusCode;
use dmarc_dns::TableResolver;
use dmarc_imap::fake_server::{FakeMessage, FakeServer};
use dmarc_imap::SyncOutcome;
use serde_json::json;
use std::io::Write;
use std::sync::{Arc, Mutex};

mod common;
use common::{get_ok, put_json, TestApp};

/// Everything logged while it is the thread's subscriber, as plain text.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogBuffer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// Three messages: a valid report, the same report again, and an attachment
/// that is no report. One pass stores the report once, counts the duplicate,
/// quarantines the broken one, moves the cursor past all three and logs the
/// batch with its timings.
#[tokio::test]
async fn one_pass_imports_through_the_real_backend() {
    let t = TestApp::with_resolver(TableResolver::new()).await;
    let (status, body) = put_json(
        &t.app,
        "/api/settings/mailbox",
        json!({
            "host": "imap.example.com",
            "port": 993,
            "username": "reports@example.com",
            "password": "secret",
            "folder": "INBOX",
            "security": "tls",
            "poll_interval_secs": 300,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let server = FakeServer::new(
        7,
        vec![
            FakeMessage::report(1),
            FakeMessage::duplicate_of(2, 1),
            FakeMessage::broken_report(3),
        ],
    );
    let logs = LogBuffer::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let outcome = {
        let _logging = tracing::subscriber::set_default(subscriber);
        t.state
            .imap
            .sync_with_fake_server(&t.state.imap_backend(), server)
            .await
    };

    let Ok(SyncOutcome::Fetched(summary)) = outcome else {
        panic!("the pass failed: {outcome:?}");
    };
    assert_eq!(summary.messages_seen, 3);
    assert_eq!(summary.reports_ingested, 1);
    assert_eq!(summary.reports_duplicate, 1);
    assert_eq!(summary.messages_quarantined, 1);
    assert_eq!(summary.ending_uid, Some(3));
    assert_eq!(t.state.imap.passes_done(), 1);

    // The report, once, and the domain it is about.
    let reports: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.report_id, d.domain FROM reports r JOIN domains d ON d.id = r.domain_id",
    )
    .fetch_all(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        reports,
        vec![("fake-1".to_owned(), "example.com".to_owned())]
    );

    // The cursor stands after the last message of the folder's generation.
    let cursor: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT folder, uid_validity, last_uid FROM imap_cursor")
            .fetch_all(&t.pool)
            .await
            .unwrap();
    assert_eq!(cursor, vec![("INBOX".to_owned(), 7, 3)]);

    // The broken attachment is quarantined with why, never retried.
    let quarantine: Vec<(i64, i64, String)> =
        sqlx::query_as("SELECT uid_validity, uid, error FROM imap_quarantine")
            .fetch_all(&t.pool)
            .await
            .unwrap();
    assert_eq!(quarantine.len(), 1, "{quarantine:?}");
    assert_eq!((quarantine[0].0, quarantine[0].1), (7, 3));
    assert!(!quarantine[0].2.is_empty());

    // The pass is recorded as a success, and the health check says so.
    let passes: Vec<(Option<i64>, Option<String>)> =
        sqlx::query_as("SELECT checked_at, error FROM imap_passes")
            .fetch_all(&t.pool)
            .await
            .unwrap();
    assert_eq!(passes.len(), 1);
    assert!(passes[0].0.is_some());
    assert_eq!(passes[0].1, None);
    let health = get_ok(&t.app, "/api/system/health").await;
    assert_eq!(health["total_reports"], 1, "{health}");
    assert_eq!(health["quarantine_count"], 1, "{health}");
    assert_eq!(
        health["imap_last_error"],
        serde_json::Value::Null,
        "{health}"
    );
    assert!(health["imap_last_success_at"].is_i64(), "{health}");

    // One batch, logged with where its time went and where the cursor stands.
    let text = logs.text();
    let batch = text
        .lines()
        .find(|l| l.contains("total_ms="))
        .unwrap_or_else(|| panic!("no batch line in:\n{text}"));
    for field in [
        "batch=1",
        "batches=1",
        "messages=3",
        "fetch_ms=",
        "parse_ms=",
        "store_ms=",
        "cursor=3",
    ] {
        assert!(batch.contains(field), "{field} missing: {batch}");
    }
}
