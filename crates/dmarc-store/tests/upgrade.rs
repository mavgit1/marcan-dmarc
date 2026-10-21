//! Schema upgrades on real old databases: a database made by a released
//! version, opened with the current code, which runs the migrations it lacks.
//! A migration that cannot run on existing rows (a `NOT NULL` column without
//! a default, a table rebuilt without its data, an index the old rows
//! violate) fails here before it reaches anyone's database.
//!
//! The fixtures in `tests/fixtures/` are never edited: one is added per
//! release whose schema changed (`vX.Y.Z.db`), made the same way:
//!
//! 1. `cargo run -p dmarc-store --example demo-data -- <dir>` at the release
//!    tag (30 days of reports for four domains through the real ingest).
//! 2. The released image run once against it until `/healthz` answers, so the
//!    released binary itself opens it, stores its DNS and reverse-DNS
//!    snapshots and saves two settings through its own API:
//!    `docker run --rm -p 127.0.0.1:18321:8080 -v <dir>:/data ghcr.io/mavgit1/marcan:X.Y.Z`,
//!    then `PUT /api/settings/retention` `{"report_retention_months":24}` and
//!    `PUT /api/settings/branding` `{"operator_name":"Fixture IT"}`, then stop it.
//! 3. `sqlite3 <dir>/marcan.db "PRAGMA wal_checkpoint(TRUNCATE); VACUUM;"`,
//!    and only `marcan.db` is kept (no `-wal` or `-shm` file).
//!
//! `v0.2.1.db` was made that way (the demo data at a commit whose code was
//! the tag's), the first release with a frozen schema.

use dmarc_parser::{PolicyPublished, Record, Report, SpfAuthResult};
use dmarc_store::{
    get_dns_snapshot, ingest_report, init, list_domain_summaries, list_senders, save_settings,
    scope_stats, AppSettings, IngestResult, OnboardingState, RetentionSettings, Scope,
    SenderListQuery, SenderSort,
};
use sqlx::SqlitePool;
use std::path::Path;

/// The fixture `name` copied into a temp dir (the original is never opened
/// for writing) and opened with the current code: every pending migration ran.
async fn open_fixture(name: &str) -> (tempfile::TempDir, SqlitePool) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("marcan.db");
    std::fs::copy(&fixture, &path).expect("copy the fixture");
    let pool = init(&path)
        .await
        .unwrap_or_else(|e| panic!("{name} does not open with the current migrations: {e}"));
    (dir, pool)
}

/// The migrations in `migrations/`, by version.
fn migration_versions() -> Vec<i64> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut versions: Vec<i64> = std::fs::read_dir(dir)
        .expect("migrations dir")
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".sql")?;
            stem.split('_').next()?.parse().ok()
        })
        .collect();
    versions.sort_unstable();
    versions
}

/// A new report for one of the fixture's domains, written after the upgrade.
fn new_report() -> Report {
    Report {
        report_id: "upgrade-test-1".into(),
        org_name: "upgrade.test".into(),
        email: "dmarc@upgrade.test".into(),
        date_begin: 1_790_000_000,
        date_end: 1_790_086_399,
        policy: PolicyPublished {
            domain: "example.com".into(),
            adkim: Some("s".into()),
            aspf: Some("s".into()),
            p: Some("reject".into()),
            sp: None,
            pct: Some(100),
            fo: None,
        },
        records: vec![Record {
            source_ip: "192.0.2.77".into(),
            count: 5,
            disposition: Some("reject".into()),
            dkim_result: Some("fail".into()),
            spf_result: Some("fail".into()),
            header_from: Some("example.com".into()),
            envelope_from: Some("spoof.example".into()),
            envelope_to: None,
            dkim_auth: vec![],
            spf_auth: vec![SpfAuthResult {
                domain: "spoof.example".into(),
                result: "fail".into(),
                scope: Some("mfrom".into()),
            }],
            policy_overrides: vec![],
        }],
        raw_xml: "<feedback/>".into(),
    }
}

/// The v0.2.1 database upgraded: every migration applied, the old rows and
/// settings read as they were, and new rows and settings written over them.
#[tokio::test]
async fn a_v0_2_1_database_opens_and_keeps_its_data() {
    let (_dir, pool) = open_fixture("v0.2.1.db").await;

    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("migrations");
    assert_eq!(applied, migration_versions());

    // The reports as v0.2.1 stored them, counted by today's queries.
    let stats = scope_stats(&pool, Scope::ALL).await.expect("stats");
    assert_eq!(stats.report_count, 270);
    assert_eq!(stats.mail.total_messages, 33_855);
    assert_eq!(stats.mail.compliant_messages, 31_461);
    assert_eq!(stats.active_domains, 3);
    assert_eq!(stats.unique_senders, 11);

    let domains = list_domain_summaries(&pool, Scope::ALL)
        .await
        .expect("domains");
    let names: Vec<&str> = domains.iter().map(|d| d.domain.as_str()).collect();
    assert_eq!(
        names,
        [
            "example.com",
            "example.net",
            "example.org",
            "legacy.example.net"
        ]
    );

    let senders = SenderListQuery {
        scope: Scope::ALL,
        failing: false,
        sort: SenderSort::DEFAULT,
        limit: 50,
        offset: 0,
    };
    let (_, senders_total) = list_senders(&pool, &senders).await.expect("senders");
    assert_eq!(senders_total, 11);

    let snapshot = get_dns_snapshot(&pool, "example.com")
        .await
        .expect("dns snapshot")
        .expect("stored by v0.2.1");
    assert_eq!(snapshot.grade, "A");

    // The settings v0.2.1 saved, and the groups added since at their defaults.
    let settings = AppSettings::load(&pool).await.expect("settings");
    assert_eq!(settings.retention.report_retention_months, 24);
    assert_eq!(settings.branding.operator_name, "Fixture IT");
    assert_eq!(settings.onboarding.state, Some(OnboardingState::Completed));
    assert_eq!(
        settings.reporting.report_address.as_deref(),
        Some("dmarc@example.com")
    );
    assert_eq!(settings.mailbox, AppSettings::default().mailbox);
    // 0.2.1 had no version check, so no `updates` row: it reads as on.
    assert!(settings.updates.check);

    // Written after the upgrade: an old report again is a duplicate, a new one
    // counts with the rest, and a settings group saves over the old row.
    let (org_name, report_id): (String, String) =
        sqlx::query_as("SELECT org_name, report_id FROM reports ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("an old report");
    let mut again = new_report();
    again.org_name = org_name;
    again.report_id = report_id;
    assert!(matches!(
        ingest_report(&pool, &again).await.expect("ingest"),
        IngestResult::Duplicate { .. }
    ));
    assert!(matches!(
        ingest_report(&pool, &new_report()).await.expect("ingest"),
        IngestResult::Inserted { .. }
    ));
    let stats = scope_stats(&pool, Scope::ALL).await.expect("stats");
    assert_eq!(stats.report_count, 271);
    assert_eq!(stats.mail.total_messages, 33_860);

    save_settings(
        &pool,
        &RetentionSettings {
            report_retention_months: 36,
        },
    )
    .await
    .expect("save");
    let settings = AppSettings::load(&pool).await.expect("settings");
    assert_eq!(settings.retention.report_retention_months, 36);
    assert_eq!(settings.branding.operator_name, "Fixture IT");
}
