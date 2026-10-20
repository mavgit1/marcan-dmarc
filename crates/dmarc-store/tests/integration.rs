//! Integration tests for dmarc-store.

use dmarc_assess::{DmarcVerdict, Outcome};
use dmarc_parser::{
    DkimAuthResult, PolicyOverride, PolicyPublished, Record, Report, SpfAuthResult,
};
use dmarc_store::{
    count_imap_quarantine, create_domain, delete_reports_older_than, dns_snapshot_is_fresh,
    find_domain_by_name, find_report_by_report_id, get_cursor, get_custom_dkim_selectors,
    get_default_organization, get_dns_snapshot, get_identity_snapshots, get_or_create_domain,
    get_or_create_organization, get_sender_by_ip, get_setting, ingest_report, ingest_reports, init,
    list_domain_failures, list_records_for_report, list_sender_ips_by_volume, list_senders,
    load_settings, quarantine_imap_message, reset_cursor, retention_cutoff, save_settings,
    scope_stats, set_cursor, set_custom_dkim_selectors, set_setting, spf_pass_domains,
    upsert_dns_snapshot, upsert_identity_snapshot, AppSettings, BrandingSettings,
    CreateDomainResult, DnsSettings, DnsSnapshot, DomainFailureFilter, DomainFailureQuery,
    FailureSort, ImapSecurity, IngestResult, LicenseSettings, MailboxSettings, MonitoringSettings,
    OnboardingSettings, OnboardingState, PolicySource, ReportingMode, ReportingSettings,
    RetentionSettings, Scope, SenderListQuery, SenderReceiverBreakdown, SenderSort,
    SenderTargetCount, SendingGroup, SendingSettings, SmtpSecurity, SnapshotDmarc,
};

fn sample_report(report_id: &str) -> Report {
    Report {
        report_id: report_id.to_owned(),
        org_name: "google.com".into(),
        email: "noreply-dmarc-support@google.com".into(),
        date_begin: 1_704_067_200,
        date_end: 1_704_153_599,
        policy: PolicyPublished {
            domain: "example.com".into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("quarantine".into()),
            sp: Some("none".into()),
            pct: Some(100),
            fo: Some("1".into()),
        },
        records: vec![
            Record {
                source_ip: "192.0.2.10".into(),
                count: 42,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("example.com".into()),
                envelope_from: Some("mail.example.com".into()),
                envelope_to: Some("recipient.example".into()),
                dkim_auth: vec![DkimAuthResult {
                    domain: "example.com".into(),
                    selector: Some("google".into()),
                    result: "pass".into(),
                }],
                spf_auth: vec![SpfAuthResult {
                    domain: "example.com".into(),
                    result: "pass".into(),
                    scope: Some("mfrom".into()),
                }],
                policy_overrides: vec![],
            },
            Record {
                source_ip: "198.51.100.7".into(),
                count: 3,
                disposition: Some("quarantine".into()),
                dkim_result: Some("fail".into()),
                spf_result: Some("fail".into()),
                header_from: Some("example.com".into()),
                envelope_from: None,
                envelope_to: None,
                dkim_auth: vec![DkimAuthResult {
                    domain: "evil.example".into(),
                    selector: None,
                    result: "fail".into(),
                }],
                spf_auth: vec![SpfAuthResult {
                    domain: "evil.example".into(),
                    result: "fail".into(),
                    scope: None,
                }],
                policy_overrides: vec![PolicyOverride {
                    reason_type: Some("forwarded".into()),
                    comment: Some("forwarded via mailing list".into()),
                }],
            },
            Record {
                source_ip: "203.0.113.50".into(),
                count: 7,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("fail".into()),
                header_from: Some("example.com".into()),
                envelope_from: None,
                envelope_to: None,
                dkim_auth: vec![],
                spf_auth: vec![],
                policy_overrides: vec![],
            },
        ],
        raw_xml: "<feedback/>".into(),
    }
}

fn all_failures() -> DomainFailureFilter<'static> {
    DomainFailureFilter {
        outcomes: Vec::new(),
        source_ip: None,
    }
}

async fn test_pool() -> (tempfile::TempDir, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let pool = init(&db_path).await.expect("init");
    (dir, pool)
}

/// Retention deletes the reports that ended before the cutoff, with their
/// records, auth rows and XML, and keeps the rest and the domain.
#[tokio::test]
async fn retention_deletes_reports_ended_before_the_cutoff() {
    let (_dir, pool) = test_pool().await;
    let now = 1_750_000_000;
    let cutoff = retention_cutoff(now, 12);
    assert_eq!(cutoff, now - 360 * 86_400);

    let mut old = sample_report("old");
    old.date_begin = cutoff - 2 * 86_400;
    old.date_end = cutoff - 1;
    let mut straddling = sample_report("straddling");
    straddling.date_begin = cutoff - 86_400;
    straddling.date_end = cutoff;
    let mut new = sample_report("new");
    new.date_begin = now - 86_400;
    new.date_end = now;
    for r in [&old, &straddling, &new] {
        ingest_report(&pool, r).await.expect("ingest");
    }

    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 100).await.unwrap(),
        1
    );
    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 100).await.unwrap(),
        0
    );

    let ids: Vec<String> = sqlx::query_scalar("SELECT report_id FROM reports ORDER BY report_id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, vec!["new", "straddling"]);
    for (table, column) in [("records", "report_id"), ("report_xml", "report_id")] {
        let orphans: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {column} NOT IN (SELECT id FROM reports)"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(orphans, 0, "{table} kept rows of a deleted report");
    }
    for table in ["record_dkim_auth", "record_spf_auth", "record_overrides"] {
        let orphans: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE record_id NOT IN (SELECT id FROM records)"
        ))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(orphans, 0, "{table} kept rows of a deleted record");
    }
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(records, 2 * new.records.len() as i64);
    assert!(find_domain_by_name(&pool, "example.com")
        .await
        .unwrap()
        .is_some());
}

/// Every connection is in WAL with synchronous=NORMAL, so a commit waits
/// for no fsync.
#[tokio::test]
async fn init_opens_every_connection_in_wal_with_synchronous_normal() {
    let (_dir, pool) = test_pool().await;
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(pool.acquire().await.unwrap());
    }
    for conn in &mut conns {
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
            .fetch_one(&mut **conn)
            .await
            .unwrap();
        assert_eq!((mode.as_str(), synchronous), ("wal", 1));
    }
}

/// A batch is stored in one transaction, in order, a repeat in it as a
/// duplicate; a report that fails stores nothing of the batch and is named.
#[tokio::test]
async fn ingest_reports_stores_a_batch_at_once_or_nothing() {
    let (_dir, pool) = test_pool().await;
    let reports = [sample_report("a"), sample_report("b"), sample_report("a")];
    let refs: Vec<&Report> = reports.iter().collect();
    let results = ingest_reports(&pool, &refs).await.unwrap();
    assert!(matches!(results[0], IngestResult::Inserted { .. }));
    assert!(matches!(results[1], IngestResult::Inserted { .. }));
    let IngestResult::Inserted { report_pk } = results[0] else {
        unreachable!()
    };
    assert_eq!(results[2], IngestResult::Duplicate { report_pk });

    sqlx::query(
        "CREATE TRIGGER refuse_bad BEFORE INSERT ON reports WHEN NEW.report_id = 'bad' \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let batch = [sample_report("c"), sample_report("bad"), sample_report("d")];
    let refs: Vec<&Report> = batch.iter().collect();
    let error = ingest_reports(&pool, &refs).await.unwrap_err();
    assert_eq!(error.index, Some(1));
    let ids: Vec<String> = sqlx::query_scalar("SELECT report_id FROM reports ORDER BY report_id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids, ["a", "b"], "nothing of the failed batch is stored");
}

/// One call deletes at most `limit` reports, the oldest first, so retention
/// can clear a large backlog one short write transaction at a time.
#[tokio::test]
async fn retention_deletes_at_most_one_chunk_per_call() {
    let (_dir, pool) = test_pool().await;
    let cutoff = 1_700_000_000;
    for (i, id) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        let mut r = sample_report(id);
        r.date_end = cutoff - 1_000 + i as i64;
        r.date_begin = r.date_end - 86_400;
        ingest_report(&pool, &r).await.expect("ingest");
    }
    let mut kept = sample_report("kept");
    kept.date_begin = cutoff;
    kept.date_end = cutoff + 86_400;
    ingest_report(&pool, &kept).await.expect("ingest");

    let left = || async {
        sqlx::query_scalar::<_, String>("SELECT report_id FROM reports ORDER BY report_id")
            .fetch_all(&pool)
            .await
            .unwrap()
    };
    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 2).await.unwrap(),
        2
    );
    assert_eq!(left().await, vec!["c", "d", "e", "kept"]);
    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 2).await.unwrap(),
        2
    );
    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 2).await.unwrap(),
        1
    );
    assert_eq!(
        delete_reports_older_than(&pool, cutoff, 2).await.unwrap(),
        0
    );
    assert_eq!(left().await, vec!["kept"]);
}

#[tokio::test]
async fn init_creates_missing_data_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("nested").join("data").join("marcan.db");
    let _pool = init(&db_path).await.expect("init");
    assert!(db_path.is_file());
}

#[cfg(unix)]
#[tokio::test]
async fn init_creates_owner_only_database_file() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _pool) = test_pool().await;
    let mode = std::fs::metadata(dir.path().join("test.db"))
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
async fn init_creates_schema() {
    let (_dir, pool) = test_pool().await;
    let organization = get_default_organization(&pool).await.expect("organization");
    assert_eq!(organization.id, 1);
    assert_eq!(organization.name, "Default");
    assert!(organization.is_default);
    let domain = get_or_create_domain(&pool, organization.id, "example.com")
        .await
        .expect("domain");
    assert_eq!(domain.domain, "example.com");

    // Idempotent organization/domain creation.
    let organization2 = get_default_organization(&pool)
        .await
        .expect("organization2");
    assert_eq!(organization.id, organization2.id);
    let domain2 = get_or_create_domain(&pool, organization.id, "example.com")
        .await
        .expect("domain2");
    assert_eq!(domain.id, domain2.id);
}

/// The record facts every count reads are columns written at ingest, with
/// defaults so a bare `INSERT INTO records` (test seeds) still has them.
#[tokio::test]
async fn init_creates_record_fact_columns() {
    let (_dir, pool) = test_pool().await;
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('records')")
        .fetch_all(&pool)
        .await
        .expect("table info");
    for col in [
        "dmarc_pass",
        "spf_aligned_pass",
        "dkim_aligned_pass",
        "forwarded",
    ] {
        assert!(columns.iter().any(|c| c == col), "records.{col} missing");
    }
}

#[tokio::test]
async fn init_creates_other_pass_index() {
    let (_dir, pool) = test_pool().await;
    let sql: Option<String> =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name = 'idx_spf_auth_other_pass'")
            .fetch_optional(&pool)
            .await
            .unwrap()
            .flatten();
    assert!(sql.expect("index missing").contains("aligned = 0"));
}

#[tokio::test]
async fn dns_snapshot_roundtrip_and_overwrite() {
    let (_dir, pool) = test_pool().await;
    get_or_create_domain(&pool, 1, "example.com").await.unwrap();
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: "example.com".into(),
            checked_at: 1_700_000_000,
            grade: "F".into(),
            score: 12,
            critical_count: 1,
            dmarc: Default::default(),
            report_json: r#"{"grade":"F"}"#.into(),
            checker_version: 2,
        },
    )
    .await
    .expect("insert");
    let first = get_dns_snapshot(&pool, "example.com")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(first.grade, "F");
    assert_eq!(first.critical_count, 1);

    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: "example.com".into(),
            checked_at: 1_700_000_100,
            grade: "A".into(),
            score: 95,
            critical_count: 0,
            dmarc: Default::default(),
            report_json: r#"{"grade":"A"}"#.into(),
            checker_version: 2,
        },
    )
    .await
    .expect("update");
    let second = get_dns_snapshot(&pool, "example.com")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(second.grade, "A");
    assert_eq!(second.score, 95);
    assert_eq!(second.checked_at, 1_700_000_100);
    assert_eq!(second.checker_version, 2);
    assert!(get_dns_snapshot(&pool, "missing.test")
        .await
        .expect("get missing")
        .is_none());
}

#[tokio::test]
async fn identity_snapshot_preserves_tri_state_confirmation() {
    let (_dir, pool) = test_pool().await;

    for (ip, ptr, confirmed) in [
        ("203.0.113.10", Some("mail.example.com"), Some(true)),
        ("203.0.113.11", Some("spoof.example.net"), Some(false)),
        ("203.0.113.12", None, None),
    ] {
        upsert_identity_snapshot(&pool, ip, 1_700_000_000, ptr, confirmed)
            .await
            .expect("upsert");
    }

    let batch = get_identity_snapshots(
        &pool,
        &[
            "203.0.113.10".into(),
            "203.0.113.11".into(),
            "203.0.113.12".into(),
            "203.0.113.99".into(),
        ],
    )
    .await
    .expect("batch");
    assert_eq!(batch.len(), 3);
    assert!(!batch.contains_key("203.0.113.99"));

    let confirmed = &batch["203.0.113.10"];
    assert_eq!(confirmed.ptr_hostname.as_deref(), Some("mail.example.com"));
    assert_eq!(confirmed.ptr_confirmed, Some(true));
    assert_eq!(batch["203.0.113.11"].ptr_confirmed, Some(false));
    let negative = &batch["203.0.113.12"];
    assert!(negative.ptr_hostname.is_none());
    assert!(
        negative.ptr_confirmed.is_none(),
        "NULL must stay None, not false"
    );
}

#[tokio::test]
async fn spf_pass_domains_name_the_busiest_unaligned_pass() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("spf-ev-1");
    let spf = |domain: &str, result: &str| SpfAuthResult {
        domain: domain.into(),
        result: result.into(),
        scope: Some("mfrom".into()),
    };
    report.records[0].spf_auth = vec![
        spf("example.com", "pass"),
        spf("bounce.vendor.example", "pass"),
    ];
    report.records[1].spf_auth = vec![spf("evil.example", "fail")];
    let mut second = report.records[0].clone();
    second.count = 50;
    second.spf_auth = vec![spf("Mail.Other.example", "pass")];
    report.records.push(second);
    ingest_report(&pool, &report).await.expect("ingest");

    let domains = spf_pass_domains(&pool, &["192.0.2.10".into(), "198.51.100.7".into()])
        .await
        .expect("query");
    // 50 messages for mail.other.example beat 42 for the vendor; the aligned
    // example.com pass and the failing evil.example never count.
    assert_eq!(
        domains.get("192.0.2.10").map(String::as_str),
        Some("mail.other.example")
    );
    assert!(!domains.contains_key("198.51.100.7"));
}

/// Reporters write tokens in any case and with stray spaces; the outcome is the
/// same either way (it used to depend on the exact spelling of `forwarded`).
#[tokio::test]
async fn outcome_ignores_case_and_spacing_of_reporter_tokens() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("norm-1");
    let failing = report.records[1].clone();
    let with = |disposition: &str, reason: &str, comment: Option<&str>| {
        let mut rec = failing.clone();
        rec.disposition = Some(disposition.into());
        rec.policy_overrides = vec![PolicyOverride {
            reason_type: Some(reason.into()),
            comment: comment.map(str::to_owned),
        }];
        rec
    };
    report.records = vec![
        with(" None ", " Forwarded ", None),
        with("NONE", "local_policy", Some("ARC=pass")),
        with("None", "other", None),
        with(" Reject", "forwarded", None),
    ];
    let pk = match ingest_report(&pool, &report).await.expect("ingest") {
        IngestResult::Inserted { report_pk } => report_pk,
        other => panic!("{other:?}"),
    };
    let outcomes: Vec<Outcome> = list_records_for_report(&pool, pk)
        .await
        .expect("records")
        .iter()
        .map(|r| r.outcome)
        .collect();
    assert_eq!(
        outcomes,
        [
            Outcome::Forwarded,
            Outcome::Forwarded,
            Outcome::Delivered,
            Outcome::Blocked
        ]
    );
}

/// The domain list's policy is the stored DNS check's, whatever the time
/// window: a domain whose reports are all older than the window still shows
/// it, the same as its overview.
#[tokio::test]
async fn domain_list_policy_ignores_the_time_window() {
    let (_dir, pool) = test_pool().await;
    let report = sample_report("old-1");
    ingest_report(&pool, &report).await.expect("ingest");
    let snapshot = DnsSnapshot {
        domain: "example.com".into(),
        checked_at: 1_700_000_000,
        grade: "B".into(),
        score: 80,
        critical_count: 0,
        dmarc: SnapshotDmarc {
            dmarc_record_count: 1,
            dmarc_record: Some("v=DMARC1; p=reject".into()),
            policy_p: Some("reject".into()),
            ..Default::default()
        },
        report_json: "{}".into(),
        checker_version: 2,
    };
    assert!(upsert_dns_snapshot(&pool, &snapshot).await.unwrap());
    let after_all_reports = report.date_end + 86_400;
    let rows =
        dmarc_store::list_domain_summaries(&pool, Scope::new(None, None, Some(after_all_reports)))
            .await
            .expect("list");
    let row = rows
        .iter()
        .find(|r| r.domain == "example.com")
        .expect("row");
    assert_eq!(row.mail.total_messages, 0, "no mail in the window");
    let policy = row.policy.as_ref().expect("the DNS policy");
    assert_eq!(policy.p.as_deref(), Some("reject"));
    assert_eq!(policy.sp, None, "the report's sp=none is not the policy");
    assert_eq!(row.policy_source, PolicySource::Dns);

    let overview = dmarc_store::get_domain_overview(&pool, row.id, Some(after_all_reports))
        .await
        .expect("overview")
        .expect("found");
    assert_eq!(overview.policy.and_then(|p| p.p).as_deref(), Some("reject"));
}

/// The policy a domain shows is the stored DNS check's DMARC record, never a
/// report's `policy_published`; without a record there is none, and the
/// source says why. The list, its policy sort and the overview all read it,
/// so they agree.
#[tokio::test]
async fn domain_list_and_overview_show_the_same_policy() {
    use PolicySource::*;
    let (_dir, pool) = test_pool().await;
    let snapshot = |domain: &str, dmarc: SnapshotDmarc| DnsSnapshot {
        domain: domain.into(),
        checked_at: 1_700_000_000,
        grade: "B".into(),
        score: 80,
        critical_count: 0,
        dmarc,
        report_json: "{}".into(),
        checker_version: 2,
    };
    let record = |p: &str| SnapshotDmarc {
        dmarc_record_count: 1,
        dmarc_record: Some(format!("v=DMARC1; p={p}; sp=none; pct=50")),
        policy_p: Some(p.into()),
        policy_sp: Some("none".into()),
        policy_pct: Some(50),
        ..Default::default()
    };
    let no_record = SnapshotDmarc::default();
    // Every report says p=quarantine sp=none pct=100.
    for (id, domain) in [
        // DNS says p=reject now.
        ("pol-1", "dns-wins.example"),
        // DNS has no record any more.
        ("pol-2", "removed.example"),
        // No DNS check yet.
        ("pol-3", "unchecked-report.example"),
        // The receivers report sp=, the DNS record has none.
        ("pol-4", "live.example"),
    ] {
        let mut report = sample_report(id);
        report.policy.domain = domain.into();
        ingest_report(&pool, &report).await.expect("ingest");
    }
    for domain in ["dns.example", "none.example", "unchecked.example"] {
        get_or_create_domain(&pool, 1, domain).await.unwrap();
    }
    for (domain, json) in [
        ("dns-wins.example", record("reject")),
        ("dns.example", record("none")),
        ("none.example", no_record.clone()),
        ("removed.example", no_record.clone()),
        (
            "live.example",
            SnapshotDmarc {
                dmarc_record_count: 1,
                dmarc_record: Some("v=DMARC1; p=quarantine; pct=100; fo=1".into()),
                policy_p: Some("quarantine".into()),
                policy_pct: Some(100),
                ..Default::default()
            },
        ),
    ] {
        assert!(upsert_dns_snapshot(&pool, &snapshot(domain, json))
            .await
            .unwrap());
    }

    let rows = dmarc_store::list_domain_summaries(&pool, Scope::ALL)
        .await
        .expect("list");
    let shown = |domain: &str| {
        let row = rows.iter().find(|r| r.domain == domain).expect(domain);
        let policy = row.policy.as_ref();
        (
            row.policy_source,
            policy.and_then(|p| p.p.as_deref()),
            policy.and_then(|p| p.sp.as_deref()),
            policy.and_then(|p| p.pct),
        )
    };
    let cases = [
        (
            "dns-wins.example",
            (Dns, Some("reject"), Some("none"), Some(50)),
        ),
        ("dns.example", (Dns, Some("none"), Some("none"), Some(50))),
        ("live.example", (Dns, Some("quarantine"), None, Some(100))),
        ("removed.example", (NotPublished, None, None, None)),
        ("none.example", (NotPublished, None, None, None)),
        ("unchecked-report.example", (NotChecked, None, None, None)),
        ("unchecked.example", (NotChecked, None, None, None)),
    ];
    for (domain, expected) in cases {
        assert_eq!(shown(domain), expected, "{domain}");
    }
    assert_eq!(rows.len(), cases.len());

    for row in &rows {
        let overview = dmarc_store::get_domain_overview(&pool, row.id, None)
            .await
            .expect("overview")
            .expect("found");
        assert_eq!(overview.policy_source, row.policy_source, "{}", row.domain);
        // No policy at all is `None`, never one of empty tags.
        assert_eq!(
            overview.policy.is_some(),
            row.policy_source == Dns,
            "{}",
            row.domain
        );
        assert_eq!(overview.policy, row.policy, "{}", row.domain);
        // The record text comes with the check that found exactly one.
        assert_eq!(
            overview.dmarc_record.is_some(),
            row.policy_source == Dns,
            "{}",
            row.domain
        );
    }

    // The policy sort orders by the same shown value.
    let (sorted, _) = dmarc_store::page_domain_summaries(
        &pool,
        &dmarc_store::DomainListQuery {
            scope: Scope::ALL,
            search: None,
            sort: dmarc_store::Sort::parse("policy_p").unwrap(),
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("sorted");
    let policies: Vec<Option<String>> = sorted
        .iter()
        .map(|r| r.policy.as_ref().and_then(|p| p.p.clone()))
        .collect();
    let mut expected = policies.clone();
    expected.sort();
    assert_eq!(policies, expected);
    assert_eq!(policies.last().unwrap().as_deref(), Some("reject"));
}

/// A sender's "failed" count is delivered failures plus blocked; a trusted
/// forward is legitimate mail broken in transit and has its own count.
#[tokio::test]
async fn sender_failed_count_leaves_out_forwarded_mail() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("failed-1");
    let passing = report.records[0].clone();
    let failing = report.records[1].clone();
    let with = |disposition: &str, reason: &str, count: i64| {
        let mut rec = failing.clone();
        // Same source as the passing record.
        rec.source_ip = passing.source_ip.clone();
        rec.disposition = Some(disposition.into());
        rec.count = count;
        rec.policy_overrides = vec![PolicyOverride {
            reason_type: Some(reason.into()),
            comment: None,
        }];
        rec
    };
    report.records = vec![
        passing.clone(),
        with("none", "forwarded", 5),
        with("none", "other", 2),
        // A forwarding reason does not make rejected mail forwarded: blocked.
        with("reject", "forwarded", 3),
    ];
    ingest_report(&pool, &report).await.expect("ingest");
    let (rows, total) = list_senders(
        &pool,
        &SenderListQuery {
            scope: Scope::ALL,
            failing: false,
            sort: SenderSort::DEFAULT,
            limit: 10,
            offset: 0,
        },
    )
    .await
    .expect("senders");
    assert_eq!(total, 1);
    let row = &rows[0];
    assert_eq!(row.total_messages, 42 + 5 + 2 + 3);
    assert_eq!(row.compliant_messages, 42);
    assert_eq!(row.failed_messages, 2 + 3);
    assert_eq!(row.forwarded_failures, 5);
    assert_eq!(
        row.total_messages,
        row.compliant_messages + row.failed_messages + row.forwarded_failures
    );
}

/// `failing` keeps the senders with failed (not forwarded) mail only: a
/// sender whose failures were all forwarded is left out.
#[tokio::test]
async fn failing_senders_leave_out_forwarded_only_senders() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("failing-1");
    let mut forwarded = report.records[1].clone();
    forwarded.source_ip = "192.0.2.99".into();
    forwarded.disposition = Some("none".into());
    forwarded.policy_overrides = vec![PolicyOverride {
        reason_type: Some("forwarded".into()),
        comment: None,
    }];
    report.records.push(forwarded);
    ingest_report(&pool, &report).await.expect("ingest");
    let query = |failing| SenderListQuery {
        scope: Scope::ALL,
        failing,
        sort: SenderSort::DEFAULT,
        limit: 10,
        offset: 0,
    };
    let (all, all_total) = list_senders(&pool, &query(false)).await.expect("all");
    assert_eq!(all_total, all.len() as i64);
    let (rows, total) = list_senders(&pool, &query(true)).await.expect("failing");
    let ips: Vec<&str> = rows.iter().map(|r| r.source_ip.as_str()).collect();
    // The passing sender and the forwarded-only one are left out.
    assert_eq!(ips, ["203.0.113.50", "198.51.100.7"]);
    assert_eq!(total, 2);
    assert_eq!(all_total, 4);
}

#[tokio::test]
async fn list_sender_ips_by_volume_orders_highest_first() {
    let (_dir, pool) = test_pool().await;
    let report = sample_report("vol-1");
    ingest_report(&pool, &report).await.expect("ingest");

    let ips = list_sender_ips_by_volume(&pool, 10).await.expect("list");
    assert!(!ips.is_empty());
    // sample_report has 192.0.2.10 with count 42 as the heaviest row.
    assert_eq!(ips[0], "192.0.2.10");
}

#[tokio::test]
async fn ingest_inserts_report_and_records() {
    let (_dir, pool) = test_pool().await;
    let report = sample_report("rpt-001");

    let result = ingest_report(&pool, &report).await.expect("ingest");
    let IngestResult::Inserted { report_pk } = result else {
        panic!("expected Inserted, got {result:?}");
    };

    let stored = find_report_by_report_id(&pool, "google.com", "rpt-001")
        .await
        .expect("find")
        .expect("present");
    assert_eq!(stored.id, report_pk);
    assert_eq!(stored.org_name, "google.com");
    assert_eq!(stored.total_messages, 52);
    assert_eq!(stored.compliant_messages, 42);
    assert_eq!(stored.policy_p.as_deref(), Some("quarantine"));
    assert_eq!(stored.policy_pct, Some(100));

    let records = list_records_for_report(&pool, report_pk)
        .await
        .expect("records");
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].source_ip, "192.0.2.10");
    assert_eq!(records[0].count, 42);
    assert_eq!(records[0].verdict, DmarcVerdict::PassBoth);
    assert_eq!(records[0].outcome, Outcome::Passed);
    assert_eq!(records[1].source_ip, "198.51.100.7");
    assert_eq!(records[1].verdict, DmarcVerdict::Fail);
    // Quarantined: blocked, even though the receiver also named a forwarding reason.
    assert_eq!(records[1].outcome, Outcome::Blocked);
    assert_eq!(records[2].count, 7);
    // No auth results: independent alignment is fail even if policy_evaluated said pass.
    assert_eq!(records[2].verdict, DmarcVerdict::Fail);
    assert_eq!(records[2].outcome, Outcome::Delivered);

    let flags: Vec<(Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT envelope_to, dmarc_pass FROM records WHERE report_id = ? ORDER BY id",
    )
    .bind(report_pk)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(flags[0], (Some("recipient.example".into()), Some(1)));
    assert_eq!(flags[1], (None, Some(0)));
    assert_eq!(flags[2], (None, Some(0)));

    let dkim: Vec<(Option<String>, Option<String>, String, i64)> = sqlx::query_as(
        "SELECT domain, selector, result, aligned FROM record_dkim_auth WHERE record_id = ? ORDER BY id",
    )
    .bind(records[0].id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(dkim.len(), 1);
    assert_eq!(dkim[0].0.as_deref(), Some("example.com"));
    assert_eq!(dkim[0].1.as_deref(), Some("google"));
    assert_eq!(dkim[0].2, "pass");
    assert_eq!(dkim[0].3, 1);

    let spf: Vec<(Option<String>, Option<String>, String, i64)> = sqlx::query_as(
        "SELECT domain, scope, result, aligned FROM record_spf_auth WHERE record_id = ? ORDER BY id",
    )
    .bind(records[0].id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(spf.len(), 1);
    assert_eq!(spf[0].0.as_deref(), Some("example.com"));
    assert_eq!(spf[0].1.as_deref(), Some("mfrom"));
    assert_eq!(spf[0].2, "pass");
    assert_eq!(spf[0].3, 1);

    let overrides: Vec<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT reason_type, comment FROM record_overrides WHERE record_id = ?")
            .bind(records[1].id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].0.as_deref(), Some("forwarded"));
    assert_eq!(
        overrides[0].1.as_deref(),
        Some("forwarded via mailing list")
    );

    let dkim_fail: (Option<String>, i64) =
        sqlx::query_as("SELECT domain, aligned FROM record_dkim_auth WHERE record_id = ?")
            .bind(records[1].id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(dkim_fail.0.as_deref(), Some("evil.example"));
    assert_eq!(dkim_fail.1, 0);
}

/// The report row's tally comes from the facts its records are stored with,
/// and every stored record's `dmarc_pass` is its two aligned passes
/// (`dmarc_assess::RecordFacts`): the outcome SQL reads the one, the verdict SQL
/// the others, so they must never disagree.
#[tokio::test]
async fn stored_facts_agree() {
    let (_dir, pool) = test_pool().await;
    ingest_report(&pool, &sample_report("rpt-derive"))
        .await
        .expect("ingest");

    let (total, compliant): (i64, i64) = sqlx::query_as(
        "SELECT total_messages, compliant_messages FROM reports WHERE report_id = ?",
    )
    .bind("rpt-derive")
    .fetch_one(&pool)
    .await
    .expect("stored report");

    // 42 aligned + 3 unaligned fail + 7 claimed-pass-but-unverifiable.
    assert_eq!(total, 52);
    assert_eq!(compliant, 42);

    let from_records: (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(count), 0),
            COALESCE(SUM(CASE WHEN dmarc_pass = 1 THEN count ELSE 0 END), 0)
        FROM records
        WHERE report_id = (SELECT id FROM reports WHERE report_id = ?)
        "#,
    )
    .bind("rpt-derive")
    .fetch_one(&pool)
    .await
    .expect("record sums");
    assert_eq!((total, compliant), from_records);

    let disagreeing: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM records \
         WHERE dmarc_pass != (spf_aligned_pass OR dkim_aligned_pass)",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(disagreeing, 0);

    // The table refuses a row whose pass the aligned flags do not carry.
    let refused = sqlx::query(
        "INSERT INTO records (report_id, source_ip, count, dmarc_pass) \
         SELECT id, '192.0.2.1', 1, 1 FROM reports LIMIT 1",
    )
    .execute(&pool)
    .await;
    assert!(
        refused.is_err(),
        "CHECK holds dmarc_pass to the aligned flags"
    );
}

#[tokio::test]
async fn reingest_same_report_id_is_noop() {
    let (_dir, pool) = test_pool().await;
    let report = sample_report("rpt-dup");

    let first = ingest_report(&pool, &report).await.expect("first");
    let IngestResult::Inserted { report_pk } = first else {
        panic!("expected Inserted");
    };

    let second = ingest_report(&pool, &report).await.expect("second");
    assert_eq!(second, IngestResult::Duplicate { report_pk });

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);

    let rec_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rec_count, 3);

    let dkim_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM record_dkim_auth")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(dkim_count, 2);
    let spf_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM record_spf_auth")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(spf_count, 2);
    let override_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM record_overrides")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(override_count, 1);

    let organizations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM organizations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(organizations, 1);
}

#[tokio::test]
async fn parser_to_store_end_to_end() {
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<feedback>
  <report_metadata>
    <org_name>yahoo.com</org_name>
    <email>dmarc_y@yahoo-inc.com</email>
    <report_id>yahoo-e2e-42</report_id>
    <date_range>
      <begin>1700000000</begin>
      <end>1700086399</end>
    </date_range>
  </report_metadata>
  <policy_published>
    <domain>acme.test</domain>
    <adkim>r</adkim>
    <aspf>r</aspf>
    <p>reject</p>
    <sp>reject</sp>
    <pct>100</pct>
  </policy_published>
  <record>
    <row>
      <source_ip>203.0.113.9</source_ip>
      <count>11</count>
      <policy_evaluated>
        <disposition>none</disposition>
        <dkim>pass</dkim>
        <spf>fail</spf>
      </policy_evaluated>
    </row>
    <identifiers>
      <header_from>acme.test</header_from>
    </identifiers>
    <auth_results>
      <dkim>
        <domain>acme.test</domain>
        <selector>s1</selector>
        <result>pass</result>
      </dkim>
      <spf>
        <domain>acme.test</domain>
        <result>fail</result>
      </spf>
    </auth_results>
  </record>
</feedback>
"#;

    let parsed = dmarc_parser::parse_report(XML.as_bytes(), Some("report.xml")).expect("parse");
    assert_eq!(parsed.report_id, "yahoo-e2e-42");
    assert_eq!(parsed.policy.domain, "acme.test");
    assert_eq!(parsed.records.len(), 1);

    let (_dir, pool) = test_pool().await;

    let result = ingest_report(&pool, &parsed).await.expect("ingest");
    assert!(matches!(result, IngestResult::Inserted { .. }));

    let stored = find_report_by_report_id(&pool, "yahoo.com", "yahoo-e2e-42")
        .await
        .expect("find")
        .expect("present");
    assert_eq!(stored.org_name, "yahoo.com");
    assert_eq!(stored.policy_p.as_deref(), Some("reject"));
    // The original XML is kept gzip-compressed beside the report.
    let xml_gz: Vec<u8> = sqlx::query_scalar("SELECT xml_gz FROM report_xml WHERE report_id = ?")
        .bind(stored.id)
        .fetch_one(&pool)
        .await
        .expect("raw xml");
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(&xml_gz[..]), &mut xml)
        .expect("gzip");
    assert!(xml.contains("<feedback>"));

    let domain_name: String = sqlx::query_scalar("SELECT domain FROM domains WHERE id = ?")
        .bind(stored.domain_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(domain_name, "acme.test");

    let records = list_records_for_report(&pool, stored.id)
        .await
        .expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_ip, "203.0.113.9");
    assert_eq!(records[0].count, 11);

    let again = ingest_report(&pool, &parsed).await.expect("reingest");
    assert!(matches!(again, IngestResult::Duplicate { .. }));
}

#[tokio::test]
async fn cursor_get_empty_set_get_and_upsert() {
    let (_dir, pool) = test_pool().await;

    let missing = get_cursor(&pool, "h|993|u", "INBOX")
        .await
        .expect("get empty");
    assert_eq!(missing, None);

    set_cursor(&pool, "h|993|u", "INBOX", 1001, 42)
        .await
        .expect("set");
    let got = get_cursor(&pool, "h|993|u", "INBOX")
        .await
        .expect("get after set");
    assert_eq!(got, Some((1001, 42)));

    // Other folders, and the same folder of another mailbox, stay independent.
    assert_eq!(
        get_cursor(&pool, "h|993|u", "Archive")
            .await
            .expect("other"),
        None
    );
    assert_eq!(
        get_cursor(&pool, "h|993|other", "INBOX")
            .await
            .expect("other"),
        None
    );

    // A reset forgets this cursor (and the folder's quarantine) only.
    set_cursor(&pool, "h|993|u", "Archive", 7, 5)
        .await
        .expect("set other");
    quarantine_imap_message(&pool, "h|993|u", "INBOX", 1001, 3, "bad", 0)
        .await
        .expect("quarantine");
    quarantine_imap_message(&pool, "h|993|other", "INBOX", 1001, 3, "bad", 0)
        .await
        .expect("quarantine other mailbox");
    reset_cursor(&pool, "h|993|u", "INBOX")
        .await
        .expect("reset");
    assert_eq!(
        get_cursor(&pool, "h|993|u", "INBOX")
            .await
            .expect("after reset"),
        None
    );
    assert_eq!(
        count_imap_quarantine(&pool, Some(("h|993|u", "INBOX")))
            .await
            .expect("count"),
        0
    );
    // Another mailbox's INBOX keeps its quarantine.
    assert_eq!(
        count_imap_quarantine(&pool, Some(("h|993|other", "INBOX")))
            .await
            .expect("count other"),
        1
    );
    assert_eq!(
        get_cursor(&pool, "h|993|u", "Archive").await.expect("kept"),
        Some((7, 5))
    );

    // Upsert overwrites uid_validity and last_uid.
    set_cursor(&pool, "h|993|u", "INBOX", 2002, 99)
        .await
        .expect("upsert");
    let updated = get_cursor(&pool, "h|993|u", "INBOX")
        .await
        .expect("get after upsert");
    assert_eq!(updated, Some((2002, 99)));
}

#[tokio::test]
async fn quarantine_keeps_one_row_per_message() {
    let (_dir, pool) = test_pool().await;

    assert_eq!(
        count_imap_quarantine(&pool, Some(("mbox", "INBOX")))
            .await
            .expect("count"),
        0
    );

    quarantine_imap_message(
        &pool,
        "mbox",
        "INBOX",
        1001,
        7,
        "parse failed",
        1_700_000_000,
    )
    .await
    .expect("record");
    assert_eq!(
        count_imap_quarantine(&pool, Some(("mbox", "INBOX")))
            .await
            .expect("count"),
        1
    );

    // Folders are independent, so one bad mailbox cannot inflate another's count.
    assert_eq!(
        count_imap_quarantine(&pool, Some(("mbox", "Archive")))
            .await
            .expect("other"),
        0
    );

    // A UIDVALIDITY reset can rewind the cursor onto the same UID. The earlier
    // failure must survive that, so the row records the latest error instead of
    // becoming a second row.
    quarantine_imap_message(
        &pool,
        "mbox",
        "INBOX",
        1001,
        7,
        "ingest failed",
        1_700_009_999,
    )
    .await
    .expect("re-record");
    assert_eq!(
        count_imap_quarantine(&pool, Some(("mbox", "INBOX")))
            .await
            .expect("count"),
        1
    );

    let (last, error): (i64, String) = sqlx::query_as(
        "SELECT last_failed_at, error
         FROM imap_quarantine WHERE folder = ? AND uid_validity = ? AND uid = ?",
    )
    .bind("INBOX")
    .bind(1001)
    .bind(7)
    .fetch_one(&pool)
    .await
    .expect("row");
    assert_eq!(last, 1_700_009_999);
    assert_eq!(error, "ingest failed");

    // A different generation of the same UID number is a different message.
    quarantine_imap_message(
        &pool,
        "mbox",
        "INBOX",
        2002,
        7,
        "parse failed",
        1_700_010_000,
    )
    .await
    .expect("new generation");
    assert_eq!(
        count_imap_quarantine(&pool, Some(("mbox", "INBOX")))
            .await
            .expect("count"),
        2
    );
}

#[tokio::test]
async fn dns_snapshot_version_mismatch_is_stale_even_when_recent() {
    let (_dir, pool) = test_pool().await;
    get_or_create_domain(&pool, 1, "stale-version.test")
        .await
        .unwrap();
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: "stale-version.test".into(),
            checked_at: 1_700_000_000,
            grade: "A".into(),
            score: 95,
            critical_count: 0,
            dmarc: Default::default(),
            report_json: r#"{"grade":"A"}"#.into(),
            checker_version: 1,
        },
    )
    .await
    .expect("insert");
    let snap = get_dns_snapshot(&pool, "stale-version.test")
        .await
        .expect("get")
        .expect("present");
    assert!(
        !dns_snapshot_is_fresh(&snap, 1_700_000_060, 3600, 2),
        "version mismatch must be stale regardless of age"
    );
    assert!(
        dns_snapshot_is_fresh(&snap, 1_700_000_060, 3600, 1),
        "matching version with recent checked_at is fresh"
    );
}

#[tokio::test]
async fn dns_snapshot_age_still_gates_matching_version() {
    let (_dir, pool) = test_pool().await;
    get_or_create_domain(&pool, 1, "aged.test").await.unwrap();
    upsert_dns_snapshot(
        &pool,
        &DnsSnapshot {
            domain: "aged.test".into(),
            checked_at: 1_700_000_000,
            grade: "B".into(),
            score: 80,
            critical_count: 0,
            dmarc: Default::default(),
            report_json: r#"{"grade":"B"}"#.into(),
            checker_version: 2,
        },
    )
    .await
    .expect("insert");
    let snap = get_dns_snapshot(&pool, "aged.test")
        .await
        .expect("get")
        .expect("present");
    assert!(dns_snapshot_is_fresh(&snap, 1_700_000_000 + 10, 3600, 2));
    assert!(!dns_snapshot_is_fresh(&snap, 1_700_000_000 + 3600, 3600, 2));
}

#[tokio::test]
async fn app_settings_get_and_set() {
    let (_dir, pool) = test_pool().await;

    // Initially none
    let val = get_setting(&pool, "imap_host")
        .await
        .expect("get missing setting");
    assert_eq!(val, None);

    // Set setting
    set_setting(&pool, "imap_host", "imap.mail.com")
        .await
        .expect("set setting");
    let val = get_setting(&pool, "imap_host").await.expect("get setting");
    assert_eq!(val.as_deref(), Some("imap.mail.com"));

    // Upsert setting
    set_setting(&pool, "imap_host", "imap.updated.com")
        .await
        .expect("upsert setting");
    let val = get_setting(&pool, "imap_host")
        .await
        .expect("get updated setting");
    assert_eq!(val.as_deref(), Some("imap.updated.com"));
}

/// Each settings group is saved on its own row and read back through
/// `AppSettings::load`, leaving the other groups as they were.
#[tokio::test]
async fn settings_groups_save_and_load_independently() {
    let (_dir, pool) = test_pool().await;
    assert_eq!(
        AppSettings::load(&pool).await.unwrap(),
        AppSettings::default()
    );

    let mailbox = MailboxSettings {
        host: "imap.example.com".into(),
        port: Some(143),
        security: Some(ImapSecurity::StartTls),
        username: "reports@example.com".into(),
        password: " secret ".into(),
        folder: Some("DMARC".into()),
        poll_interval_secs: Some(120),
    };
    let reporting = ReportingSettings {
        report_address: Some("dmarc@example.com".into()),
        reporting_mode: Some(ReportingMode::Forwarded),
    };
    let monitoring = MonitoringSettings {
        report_freshness_warning_hours: 48,
        domain_reports_stopped_days: 10,
    };
    let retention = RetentionSettings {
        report_retention_months: 24,
    };
    let dns = DnsSettings {
        nameservers: vec!["9.9.9.9".into()],
    };
    let sending = SendingSettings {
        group: SendingGroup {
            enabled: true,
            host: "smtp.example.com".into(),
            port: Some(465),
            security: SmtpSecurity::Tls,
            same_login_as_mailbox: false,
            username: "alerts@example.com".into(),
            sender: "dmarc@example.com".into(),
            reply_to: String::new(),
            test_recipient: "me@example.com".into(),
        },
        password: "smtp secret".into(),
    };
    let license = LicenseSettings {
        operator_key: Some("you@company.example.AbC".into()),
    };
    let branding = BrandingSettings {
        operator_name: "Acme IT".into(),
    };
    let onboarding = OnboardingSettings {
        state: Some(OnboardingState::Completed),
    };
    save_settings(&pool, &mailbox).await.unwrap();
    save_settings(&pool, &license).await.unwrap();
    save_settings(&pool, &branding).await.unwrap();
    save_settings(&pool, &reporting).await.unwrap();
    save_settings(&pool, &monitoring).await.unwrap();
    save_settings(&pool, &retention).await.unwrap();
    save_settings(&pool, &dns).await.unwrap();
    save_settings(&pool, &sending).await.unwrap();
    save_settings(&pool, &onboarding).await.unwrap();
    let expected = AppSettings {
        mailbox: mailbox.clone(),
        reporting: reporting.clone(),
        monitoring,
        retention,
        dns: dns.clone(),
        sending: sending.clone(),
        alerts: Default::default(),
        client_reports: Default::default(),
        license: license.clone(),
        branding: branding.clone(),
        onboarding,
    };
    assert_eq!(AppSettings::load(&pool).await.unwrap(), expected);
    assert_eq!(
        load_settings::<SendingSettings>(&pool).await.unwrap(),
        sending
    );

    // Saving one group again leaves the others.
    save_settings(&pool, &DnsSettings::default()).await.unwrap();
    save_settings(&pool, &ReportingSettings::default())
        .await
        .unwrap();
    assert_eq!(
        AppSettings::load(&pool).await.unwrap(),
        AppSettings {
            reporting: ReportingSettings::default(),
            dns: DnsSettings::default(),
            ..expected.clone()
        }
    );

    // A stored group that does not read is its default for the pages, and an
    // error for a caller that asks for it alone.
    set_setting(
        &pool,
        "retention",
        r#"{"report_retention_months":"twenty"}"#,
    )
    .await
    .unwrap();
    assert_eq!(
        AppSettings::load(&pool).await.unwrap().retention,
        RetentionSettings::default()
    );
    assert!(load_settings::<RetentionSettings>(&pool).await.is_err());
}

#[tokio::test]
async fn test_database_reingestion_resilience() {
    let (_dir, pool) = test_pool().await;

    // Build multi-signature reports
    let rpt1 = Report {
        report_id: "resilience-rpt-1".into(),
        org_name: "google.com".into(),
        email: "noreply@google.com".into(),
        date_begin: 1_704_067_200,
        date_end: 1_704_153_599,
        policy: PolicyPublished {
            domain: "resilience.example".into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("reject".into()),
            sp: Some("reject".into()),
            pct: Some(100),
            fo: None,
        },
        records: vec![
            Record {
                source_ip: "192.0.2.10".into(),
                count: 10,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("resilience.example".into()),
                envelope_from: Some("mail.resilience.example".into()),
                envelope_to: None,
                dkim_auth: vec![
                    DkimAuthResult {
                        domain: "resilience.example".into(),
                        selector: Some("s1".into()),
                        result: "pass".into(),
                    },
                    DkimAuthResult {
                        domain: "resilience.example".into(),
                        selector: Some("s2".into()),
                        result: "fail".into(),
                    },
                ],
                spf_auth: vec![
                    SpfAuthResult {
                        domain: "resilience.example".into(),
                        result: "pass".into(),
                        scope: Some("mfrom".into()),
                    },
                    SpfAuthResult {
                        domain: "mail.resilience.example".into(),
                        result: "fail".into(),
                        scope: Some("helo".into()),
                    },
                ],
                policy_overrides: vec![],
            },
            Record {
                source_ip: "198.51.100.20".into(),
                count: 5,
                disposition: Some("reject".into()),
                dkim_result: Some("fail".into()),
                spf_result: Some("fail".into()),
                header_from: Some("resilience.example".into()),
                envelope_from: Some("attacker.com".into()),
                envelope_to: None,
                dkim_auth: vec![DkimAuthResult {
                    domain: "evil.example".into(),
                    selector: Some("bad".into()),
                    result: "fail".into(),
                }],
                spf_auth: vec![SpfAuthResult {
                    domain: "evil.example".into(),
                    result: "fail".into(),
                    scope: Some("mfrom".into()),
                }],
                policy_overrides: vec![],
            },
        ],
        raw_xml: "<feedback/>".into(),
    };

    let rpt2 = Report {
        report_id: "resilience-rpt-2".into(),
        org_name: "yahoo.com".into(),
        email: "postmaster@yahoo.com".into(),
        date_begin: 1_704_153_600,
        date_end: 1_704_240_000,
        policy: PolicyPublished {
            domain: "resilience.example".into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("reject".into()),
            sp: Some("reject".into()),
            pct: Some(100),
            fo: None,
        },
        records: vec![Record {
            source_ip: "203.0.113.30".into(),
            count: 20,
            disposition: Some("none".into()),
            dkim_result: Some("fail".into()),
            spf_result: Some("pass".into()),
            header_from: Some("resilience.example".into()),
            envelope_from: Some("newsletter.resilience.example".into()),
            envelope_to: None,
            dkim_auth: vec![DkimAuthResult {
                domain: "unaligned.example".into(),
                selector: Some("news".into()),
                result: "pass".into(),
            }],
            spf_auth: vec![SpfAuthResult {
                domain: "resilience.example".into(),
                result: "pass".into(),
                scope: Some("mfrom".into()),
            }],
            policy_overrides: vec![],
        }],
        raw_xml: "<feedback/>".into(),
    };

    // 1. Initial Ingestion
    let res1 = ingest_report(&pool, &rpt1).await.expect("ingest rpt1");
    assert!(matches!(res1, IngestResult::Inserted { .. }));
    let res2 = ingest_report(&pool, &rpt2).await.expect("ingest rpt2");
    assert!(matches!(res2, IngestResult::Inserted { .. }));

    let record_count_initial: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .expect("count records");
    assert_eq!(record_count_initial, 3);

    let domain = get_or_create_domain(&pool, 1, "resilience.example")
        .await
        .expect("domain");
    let domain_id = domain.id;

    let stats_initial = scope_stats(&pool, Scope::domain(domain_id, None))
        .await
        .expect("stats");
    assert_eq!(stats_initial.mail.total_messages, 35);
    assert_eq!(stats_initial.mail.compliant_messages, 30); // 10 (rpt1 rec1) + 20 (rpt2 rec1)
    assert_eq!(stats_initial.report_count, 2);
    assert_eq!(stats_initial.active_domains, 1);

    let (failures_initial, failure_total_initial) = list_domain_failures(
        &pool,
        &DomainFailureQuery {
            domain_id,
            since: None,
            filter: all_failures(),
            sort: FailureSort::DEFAULT,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("failures");
    assert_eq!(failure_total_initial, 2);

    // 2. Re-ingest identical reports -> should be idempotent Duplicate no-op
    let dup1 = ingest_report(&pool, &rpt1).await.expect("reingest rpt1");
    assert!(matches!(dup1, IngestResult::Duplicate { .. }));
    let dup2 = ingest_report(&pool, &rpt2).await.expect("reingest rpt2");
    assert!(matches!(dup2, IngestResult::Duplicate { .. }));

    let record_count_after_dup: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .expect("count records");
    assert_eq!(record_count_after_dup, record_count_initial);

    // 3. Wipe records & reports (children first: nothing cascades, the app
    // never deletes report data).
    for table in [
        "record_dkim_auth",
        "record_spf_auth",
        "record_overrides",
        "records",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&pool)
            .await
            .expect("delete records");
    }
    sqlx::query("DELETE FROM reports")
        .execute(&pool)
        .await
        .expect("delete reports");

    let count_after_wipe: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .expect("count records");
    assert_eq!(count_after_wipe, 0);

    // 4. Re-ingest fresh
    let fresh1 = ingest_report(&pool, &rpt1)
        .await
        .expect("fresh ingest rpt1");
    assert!(matches!(fresh1, IngestResult::Inserted { .. }));
    let fresh2 = ingest_report(&pool, &rpt2)
        .await
        .expect("fresh ingest rpt2");
    assert!(matches!(fresh2, IngestResult::Inserted { .. }));

    // Assert identical domain IDs, counts, stats, and failures
    let domain_after = get_or_create_domain(&pool, 1, "resilience.example")
        .await
        .expect("domain after");
    assert_eq!(domain_after.id, domain_id);

    let record_count_fresh: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
        .fetch_one(&pool)
        .await
        .expect("count records");
    assert_eq!(record_count_fresh, record_count_initial);

    let stats_fresh = scope_stats(&pool, Scope::domain(domain_id, None))
        .await
        .expect("stats fresh");
    assert_eq!(stats_fresh.mail, stats_initial.mail);
    assert_eq!(stats_fresh.report_count, stats_initial.report_count);
    assert_eq!(stats_fresh.active_domains, stats_initial.active_domains);

    let (failures_fresh, failure_total_fresh) = list_domain_failures(
        &pool,
        &DomainFailureQuery {
            domain_id,
            since: None,
            filter: all_failures(),
            sort: FailureSort::DEFAULT,
            limit: 50,
            offset: 0,
        },
    )
    .await
    .expect("failures fresh");
    assert_eq!(failure_total_fresh, failure_total_initial);
    assert_eq!(failures_fresh.len(), failures_initial.len());
    for (f_fresh, f_init) in failures_fresh.iter().zip(failures_initial.iter()) {
        assert_eq!(f_fresh.source_ip, f_init.source_ip);
        assert_eq!(f_fresh.verdict, f_init.verdict);
        assert_eq!(f_fresh.disposition, f_init.disposition);
        assert_eq!(f_fresh.count, f_init.count);
        assert_eq!(f_fresh.checks, f_init.checks);
        assert_eq!(f_fresh.org_name, f_init.org_name);
    }
}

#[tokio::test]
async fn test_sender_receiver_breakdown() {
    let (_dir, pool) = test_pool().await;

    let rpt_google = Report {
        report_id: "rpt-rx-google".into(),
        org_name: "Google".into(),
        email: "noreply@google.com".into(),
        date_begin: 1_704_067_200,
        date_end: 1_704_153_600,
        policy: PolicyPublished {
            domain: "rx-test.example".into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("none".into()),
            sp: None,
            pct: Some(100),
            fo: None,
        },
        records: vec![
            Record {
                source_ip: "198.51.100.77".into(),
                count: 30,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("rx-test.example".into()),
                envelope_from: Some("mail.rx-test.example".into()),
                envelope_to: Some("target1.example".into()),
                dkim_auth: vec![],
                spf_auth: vec![],
                policy_overrides: vec![],
            },
            Record {
                source_ip: "198.51.100.77".into(),
                count: 10,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("rx-test.example".into()),
                envelope_from: Some("mail.rx-test.example".into()),
                envelope_to: None,
                dkim_auth: vec![],
                spf_auth: vec![],
                policy_overrides: vec![],
            },
        ],
        raw_xml: "<feedback/>".into(),
    };

    let rpt_yahoo = Report {
        report_id: "rpt-rx-yahoo".into(),
        org_name: "Yahoo".into(),
        email: "dmarc@yahoo.com".into(),
        date_begin: 1_704_067_200,
        date_end: 1_704_153_600,
        policy: PolicyPublished {
            domain: "rx-test.example".into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("none".into()),
            sp: None,
            pct: Some(100),
            fo: None,
        },
        records: vec![
            Record {
                source_ip: "198.51.100.77".into(),
                count: 50,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("rx-test.example".into()),
                envelope_from: Some("mail.rx-test.example".into()),
                envelope_to: Some("target2.example".into()),
                dkim_auth: vec![],
                spf_auth: vec![],
                policy_overrides: vec![],
            },
            Record {
                source_ip: "198.51.100.77".into(),
                count: 25,
                disposition: Some("none".into()),
                dkim_result: Some("pass".into()),
                spf_result: Some("pass".into()),
                header_from: Some("rx-test.example".into()),
                envelope_from: Some("mail.rx-test.example".into()),
                envelope_to: Some("target1.example".into()),
                dkim_auth: vec![],
                spf_auth: vec![],
                policy_overrides: vec![],
            },
        ],
        raw_xml: "<feedback/>".into(),
    };

    ingest_report(&pool, &rpt_google)
        .await
        .expect("ingest google");
    ingest_report(&pool, &rpt_yahoo)
        .await
        .expect("ingest yahoo");

    let detail = get_sender_by_ip(&pool, "198.51.100.77", Scope::ALL, 1_704_200_000)
        .await
        .expect("get_sender_by_ip")
        .expect("sender found");

    assert_eq!(detail.mail.total_messages, 115);
    assert_eq!(detail.items.len(), 1);

    let domain_row = &detail.items[0];
    assert_eq!(domain_row.domain, "rx-test.example");

    // Receivers must be ordered by total_messages descending:
    // 1. Yahoo: 75 msgs (target2.example: 50, target1.example: 25)
    // 2. Google: 40 msgs (target1.example: 30; null envelope_to counted in total_messages but omitted from targets)
    assert_eq!(
        domain_row.receivers,
        vec![
            SenderReceiverBreakdown {
                reporter: "Yahoo".into(),
                total_messages: 75,
                targets: vec![
                    SenderTargetCount {
                        target: "target2.example".into(),
                        count: 50,
                    },
                    SenderTargetCount {
                        target: "target1.example".into(),
                        count: 25,
                    },
                ],
            },
            SenderReceiverBreakdown {
                reporter: "Google".into(),
                total_messages: 40,
                targets: vec![SenderTargetCount {
                    target: "target1.example".into(),
                    count: 30,
                }],
            },
        ]
    );

    // Domain-level targets, summed across receivers
    assert_eq!(
        domain_row.targets,
        vec![
            SenderTargetCount {
                target: "target1.example".into(),
                count: 55,
            },
            SenderTargetCount {
                target: "target2.example".into(),
                count: 50,
            },
        ]
    );
}

#[tokio::test]
async fn test_custom_dkim_selectors_crud() {
    let (_dir, pool) = test_pool().await;

    // 1. Initially empty
    let initial = get_custom_dkim_selectors(&pool).await.expect("get initial");
    assert!(initial.is_empty());

    // 2. Replace: normalized, deduplicated, sorted, empty ones dropped
    let saved = set_custom_dkim_selectors(
        &pool,
        &[
            "mail".into(),
            " CORP2024  ".into(),
            "MAIL".into(),
            " ".into(),
        ],
    )
    .await
    .expect("set");
    assert_eq!(saved, vec!["corp2024".to_string(), "mail".to_string()]);

    // 3. Query persisted
    let fetched = get_custom_dkim_selectors(&pool).await.expect("fetch");
    assert_eq!(fetched, saved);

    // 4. Replace with nothing clears the pool
    let cleared = set_custom_dkim_selectors(&pool, &[]).await.expect("clear");
    assert!(cleared.is_empty());

    let final_fetch = get_custom_dkim_selectors(&pool).await.expect("final fetch");
    assert!(final_fetch.is_empty());
}

#[tokio::test]
async fn migration_seeds_exactly_one_default_organization() {
    let (_dir, pool) = test_pool().await;
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM organizations WHERE is_default = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);

    let other = get_or_create_organization(&pool, "Client A").await.unwrap();
    assert!(!other.is_default);
    // The partial unique index forbids a second default.
    let second = sqlx::query("UPDATE organizations SET is_default = 1 WHERE id = ?")
        .bind(other.id)
        .execute(&pool)
        .await;
    assert!(
        second.is_err(),
        "a second default organization must be rejected"
    );
}

#[tokio::test]
async fn domain_names_are_normalized_and_globally_unique() {
    let (_dir, pool) = test_pool().await;
    let default = get_default_organization(&pool).await.unwrap();
    let client = get_or_create_organization(&pool, "Client A").await.unwrap();

    let created = match create_domain(&pool, client.id, "  Example.COM. ")
        .await
        .unwrap()
    {
        CreateDomainResult::Created(d) => d,
        other => panic!("expected Created, got {other:?}"),
    };
    assert_eq!(created.domain, "example.com");
    assert_eq!(created.organization_id, client.id);

    // Same name (different spelling) in another organization is a conflict naming the holder.
    match create_domain(&pool, default.id, "EXAMPLE.com")
        .await
        .unwrap()
    {
        CreateDomainResult::AlreadyExists {
            domain,
            organization_name,
        } => {
            assert_eq!(domain.id, created.id);
            assert_eq!(organization_name, "Client A");
        }
        other => panic!("expected AlreadyExists, got {other:?}"),
    }

    // get_or_create_domain returns the existing row instead of duplicating it.
    let again = get_or_create_domain(&pool, default.id, "example.com.")
        .await
        .unwrap();
    assert_eq!(again.id, created.id);
    assert_eq!(again.organization_id, client.id);

    let found = find_domain_by_name(&pool, " Example.Com ").await.unwrap();
    assert_eq!(found.map(|d| d.id), Some(created.id));

    // A raw insert of the same name into another organization violates UNIQUE(domain).
    let dup =
        sqlx::query("INSERT INTO domains (organization_id, domain) VALUES (?, 'example.com')")
            .bind(default.id)
            .execute(&pool)
            .await;
    assert!(dup.is_err());
}

#[tokio::test]
async fn ingest_creates_unknown_domain_in_default_organization() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("default-co-1");
    report.policy.domain = "New-Domain.Example.".into();
    ingest_report(&pool, &report).await.expect("ingest");

    let domain = find_domain_by_name(&pool, "new-domain.example")
        .await
        .unwrap()
        .expect("domain created");
    assert_eq!(domain.domain, "new-domain.example");
    let default = get_default_organization(&pool).await.unwrap();
    assert_eq!(domain.organization_id, default.id);
}

#[tokio::test]
async fn ingest_uses_domain_from_non_default_organization() {
    let (_dir, pool) = test_pool().await;
    let client = get_or_create_organization(&pool, "Client B").await.unwrap();
    let existing = get_or_create_domain(&pool, client.id, "example.com")
        .await
        .unwrap();

    let mut report = sample_report("client-co-1");
    report.policy.domain = "EXAMPLE.COM".into();
    ingest_report(&pool, &report).await.expect("ingest");

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM domains")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1, "no duplicate domain in the default organization");
    let stored = find_report_by_report_id(&pool, "google.com", "client-co-1")
        .await
        .unwrap()
        .expect("report");
    assert_eq!(stored.domain_id, existing.id);
}

/// `report_id` is only unique per reporter: two organizations may reuse one.
#[tokio::test]
async fn same_report_id_from_two_reporters_is_kept_twice() {
    let (_dir, pool) = test_pool().await;
    let google = sample_report("shared-id-1");
    let mut yahoo = sample_report("shared-id-1");
    yahoo.org_name = "yahoo.com".into();

    let first = ingest_report(&pool, &google).await.unwrap();
    let second = ingest_report(&pool, &yahoo).await.unwrap();
    assert!(matches!(first, IngestResult::Inserted { .. }));
    assert!(matches!(second, IngestResult::Inserted { .. }));
    assert_ne!(first, second);

    // The same reporter re-sending is still a duplicate.
    let again = ingest_report(&pool, &yahoo).await.unwrap();
    let IngestResult::Inserted { report_pk } = second else {
        unreachable!()
    };
    assert_eq!(again, IngestResult::Duplicate { report_pk });

    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reports")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let found = find_report_by_report_id(&pool, "yahoo.com", "shared-id-1")
        .await
        .unwrap()
        .expect("yahoo report");
    assert_eq!(found.id, report_pk);
}

/// Auth results are stored trimmed and lowercase, so SQL `result = 'pass'`
/// counts what the alignment verdict (case-insensitive) counts.
#[tokio::test]
async fn auth_results_are_normalized_at_ingest() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("case-1");
    report.records[0].dkim_auth[0].result = " PASS ".into();
    report.records[0].spf_auth[0].result = "Pass".into();
    report.records[0].spf_result = Some("PASS".into());
    ingest_report(&pool, &report).await.unwrap();

    let dkim: Vec<(String,)> = sqlx::query_as("SELECT result FROM record_dkim_auth")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(dkim.iter().any(|(r,)| r == "pass"));
    assert!(dkim
        .iter()
        .all(|(r,)| r == r.trim() && *r == r.to_lowercase()));
    let (spf,): (String,) =
        sqlx::query_as("SELECT result FROM record_spf_auth ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(spf, "pass");
    let (verdict,): (Option<String>,) =
        sqlx::query_as("SELECT spf_result FROM records ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(verdict.as_deref(), Some("pass"));
}

/// 29 messages pass SPF for the domain; 3 bounces (empty envelope) pass only
/// for the sending server's name and carry no DKIM: the row says so in counts.
#[tokio::test]
async fn sender_rows_count_passes_for_other_domains() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("other-1");
    let mut ok = report.records[0].clone();
    ok.source_ip = "203.0.113.30".into();
    ok.count = 29;
    let mut bounce = ok.clone();
    bounce.count = 3;
    bounce.envelope_from = Some("<>".into());
    bounce.dkim_auth = vec![];
    bounce.spf_auth = vec![SpfAuthResult {
        domain: "mxout022.mail.hostpoint.ch".into(),
        result: "pass".into(),
        scope: Some("helo".into()),
    }];
    report.records = vec![ok, bounce];
    ingest_report(&pool, &report).await.expect("ingest");

    let detail = get_sender_by_ip(&pool, "203.0.113.30", Scope::ALL, 1_704_200_000)
        .await
        .expect("query")
        .expect("sender");
    let row = &detail.items[0];
    assert_eq!(row.mail.total_messages, 32);
    assert_eq!(row.spf_pass_messages, 29);
    assert_eq!(row.spf_other_only_messages, 3);
    assert_eq!(row.spf_other_passes.len(), 1);
    let other = &row.spf_other_passes[0];
    assert_eq!(other.domain, "mxout022.mail.hostpoint.ch");
    assert_eq!(other.messages, 3);
    assert!(other.helo);
    assert_eq!(row.dkim_pass_messages, 29);
    assert_eq!(row.dkim_unsigned_messages, 3);
    assert!(row.dkim_other_passes.is_empty());
}

/// A record whose SPF passed twice for the same other domain (different case,
/// mfrom + helo) counts once; DKIM selectors are listed per record; another
/// IP's records never leak into the detail.
#[tokio::test]
async fn sender_other_passes_dedupe_per_record_and_ignore_other_ips() {
    let (_dir, pool) = test_pool().await;
    let mut report = sample_report("other-2");
    let mut rec = report.records[0].clone();
    rec.source_ip = "203.0.113.31".into();
    rec.count = 5;
    rec.dkim_result = Some("fail".into());
    rec.spf_result = Some("fail".into());
    rec.dkim_auth = vec![
        DkimAuthResult {
            domain: "relay.example.net".into(),
            selector: Some("s1".into()),
            result: "pass".into(),
        },
        DkimAuthResult {
            domain: "Relay.Example.Net".into(),
            selector: Some("s2".into()),
            result: "pass".into(),
        },
    ];
    rec.spf_auth = vec![
        SpfAuthResult {
            domain: "Relay.Example.Net".into(),
            result: "pass".into(),
            scope: Some("mfrom".into()),
        },
        SpfAuthResult {
            domain: "relay.example.net".into(),
            result: "pass".into(),
            scope: Some("helo".into()),
        },
    ];
    let mut other_ip = rec.clone();
    other_ip.source_ip = "203.0.113.32".into();
    other_ip.count = 100;
    report.records = vec![rec, other_ip];
    ingest_report(&pool, &report).await.expect("ingest");

    let detail = get_sender_by_ip(&pool, "203.0.113.31", Scope::ALL, 1_704_200_000)
        .await
        .expect("query")
        .expect("sender");
    let row = &detail.items[0];
    assert_eq!(row.mail.total_messages, 5);
    assert_eq!(row.spf_other_passes.len(), 1);
    assert_eq!(row.spf_other_passes[0].domain, "relay.example.net");
    assert_eq!(row.spf_other_passes[0].messages, 5);
    assert!(
        !row.spf_other_passes[0].helo,
        "one mfrom pass: not helo-only"
    );
    assert_eq!(row.dkim_other_passes.len(), 1);
    assert_eq!(row.dkim_other_passes[0].messages, 5);
    let mut selectors: Vec<&str> = row
        .dkim_selectors
        .as_deref()
        .unwrap()
        .split(',')
        .map(str::trim)
        .collect();
    selectors.sort_unstable();
    assert_eq!(selectors, ["s1", "s2"]);
}

/// Ingest and "remove domain" racing: every report lands before the ignore
/// (and is deleted with the domain) or is dropped after it; none survives
/// under the ignored domain, and no writer fails with `SQLITE_BUSY`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ingest_racing_ignore_never_stores_under_an_ignored_domain() {
    let (_dir, pool) = test_pool().await;
    for round in 0..10 {
        let name = format!("race{round}.example");
        let domain = get_or_create_domain(&pool, 1, &name).await.unwrap();
        let mut ingests = Vec::new();
        for i in 0..8 {
            let pool = pool.clone();
            let mut report = sample_report(&format!("race-{round}-{i}"));
            report.policy.domain = name.clone();
            ingests.push(tokio::spawn(
                async move { ingest_report(&pool, &report).await },
            ));
        }
        let ignore = {
            let pool = pool.clone();
            tokio::spawn(async move { dmarc_store::ignore_domain(&pool, domain.id).await })
        };
        for task in ingests {
            task.await.unwrap().expect("ingest");
        }
        assert!(ignore.await.unwrap().expect("ignore"));
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE domain_id = ?")
            .bind(domain.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "round {round}");
    }
}

/// Write transactions take the write lock up front, so concurrent ones
/// queue instead of failing or mixing: the list is one of them, whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_selector_replaces_queue() {
    let (_dir, pool) = test_pool().await;
    let lists: Vec<Vec<String>> = (0..12)
        .map(|i| (0..5).map(|j| format!("s{i:02}x{j}")).collect())
        .collect();
    let writes: Vec<_> = lists
        .iter()
        .cloned()
        .map(|list| {
            let pool = pool.clone();
            tokio::spawn(async move { set_custom_dkim_selectors(&pool, &list).await })
        })
        .collect();
    for write in writes {
        write.await.unwrap().expect("replace");
    }
    let stored = get_custom_dkim_selectors(&pool).await.unwrap();
    assert!(lists.contains(&stored), "{stored:?}");
}

#[tokio::test]
async fn dns_snapshots_are_kept_only_for_monitored_domains() {
    let (_dir, pool) = test_pool().await;
    let snap = |domain: &str| DnsSnapshot {
        domain: domain.into(),
        checked_at: 1_700_000_000,
        grade: "A".into(),
        score: 95,
        critical_count: 0,
        dmarc: Default::default(),
        report_json: "{}".into(),
        checker_version: 1,
    };
    assert!(!upsert_dns_snapshot(&pool, &snap("unknown.example"))
        .await
        .unwrap());
    let domain = get_or_create_domain(&pool, 1, "gone.example")
        .await
        .unwrap();
    dmarc_store::ignore_domain(&pool, domain.id).await.unwrap();
    assert!(!upsert_dns_snapshot(&pool, &snap("gone.example"))
        .await
        .unwrap());
    get_or_create_domain(&pool, 1, "mine.example")
        .await
        .unwrap();
    assert!(upsert_dns_snapshot(&pool, &snap("mine.example"))
        .await
        .unwrap());
    assert!(get_dns_snapshot(&pool, "gone.example")
        .await
        .unwrap()
        .is_none());
    // Keyed by the domain's id: deleting the domain deletes its snapshot.
    sqlx::query("DELETE FROM domains WHERE domain = 'mine.example'")
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM dns_snapshots")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

/// The logo lives in its own table: saved, read back with its size, and
/// removed, without touching the settings.
#[tokio::test]
async fn logo_is_stored_apart_from_the_settings() {
    use dmarc_store::{
        get_logo, get_logo_info, set_logo, LogoInfo, LogoType, PixelSize, StoredLogo,
    };
    let (_dir, pool) = test_pool().await;
    assert_eq!(get_logo(&pool).await.unwrap(), None);
    let logo = StoredLogo {
        logo_type: LogoType::Png,
        bytes: b"\x89PNG\r\n\x1a\nrest".to_vec(),
        size: PixelSize {
            width: 640,
            height: 160,
        },
    };
    set_logo(&pool, Some(&logo)).await.unwrap();
    let webp = StoredLogo {
        logo_type: LogoType::Webp,
        bytes: b"RIFF....WEBP".to_vec(),
        size: PixelSize {
            width: 300,
            height: 60,
        },
    };
    set_logo(&pool, Some(&webp)).await.unwrap();
    assert_eq!(get_logo(&pool).await.unwrap(), Some(webp));
    assert_eq!(
        get_logo_info(&pool).await.unwrap(),
        Some(LogoInfo {
            logo_type: LogoType::Webp,
            bytes: 12,
            size: PixelSize {
                width: 300,
                height: 60,
            },
        })
    );
    let settings: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_settings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(settings, 0);
    set_logo(&pool, None).await.unwrap();
    set_logo(&pool, None).await.unwrap();
    assert_eq!(get_logo_info(&pool).await.unwrap(), None);
}

#[tokio::test]
async fn imap_passes_are_pruned_to_the_newest() {
    use dmarc_store::{imap_pass_status, record_imap_pass, ImapPassRow, IMAP_PASSES_KEPT};
    let (_dir, pool) = test_pool().await;
    for i in 0..(IMAP_PASSES_KEPT + 5) {
        record_imap_pass(
            &pool,
            &ImapPassRow {
                finished_at: i + 1,
                checked_at: Some(i),
                error: None,
            },
        )
        .await
        .unwrap();
    }
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imap_passes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, IMAP_PASSES_KEPT);
    let status = imap_pass_status(&pool).await.unwrap();
    assert_eq!(status.last_check_at, Some(IMAP_PASSES_KEPT + 4));
}

/// A long run of failures must not erase the last success: the newest
/// successful pass survives the prune however old it is.
#[tokio::test]
async fn pruning_keeps_the_newest_successful_pass() {
    use dmarc_store::{imap_pass_status, record_imap_pass, ImapPassRow, IMAP_PASSES_KEPT};
    let (_dir, pool) = test_pool().await;
    let pass = |i: i64, error: Option<&str>| ImapPassRow {
        finished_at: i + 1,
        checked_at: Some(i),
        error: error.map(str::to_owned),
    };
    record_imap_pass(&pool, &pass(0, None)).await.unwrap();
    for i in 1..(IMAP_PASSES_KEPT + 5) {
        record_imap_pass(&pool, &pass(i, Some("connection refused")))
            .await
            .unwrap();
    }
    let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM imap_passes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(kept, IMAP_PASSES_KEPT + 1);
    let status = imap_pass_status(&pool).await.unwrap();
    assert_eq!(status.last_success_at, Some(1));
    assert_eq!(status.last_check_at, Some(IMAP_PASSES_KEPT + 4));
    assert!(status.last_error.is_some());
}

#[tokio::test]
async fn client_report_recipients_are_replaced_whole_and_follow_their_organization() {
    use dmarc_store::{
        delete_organization, list_client_report_recipients, preview_client_report_recipients,
        replace_client_report_recipients, ClientReportRecipient,
    };
    let (_dir, pool) = test_pool().await;
    let acme = get_or_create_organization(&pool, "Acme").await.unwrap();
    let beta = get_or_create_organization(&pool, "Beta").await.unwrap();
    let r = |id: i64, org: &str, email: &str| ClientReportRecipient {
        organization_id: id,
        organization: org.into(),
        email: email.into(),
    };
    let first = [
        r(beta.id, "Beta", "b@beta.example"),
        r(acme.id, "Acme", "a@acme.example"),
    ];
    let changes = replace_client_report_recipients(&pool, &first)
        .await
        .unwrap();
    assert_eq!(changes.start.len(), 2);
    // Sorted by organization name.
    assert_eq!(
        list_client_report_recipients(&pool).await.unwrap(),
        [first[1].clone(), first[0].clone()]
    );

    let next = [
        r(acme.id, "Acme", "A@acme.example"),
        r(acme.id, "Acme", "c@acme.example"),
    ];
    let preview = preview_client_report_recipients(&pool, &next)
        .await
        .unwrap();
    assert_eq!(preview.start, [next[1].clone()]);
    assert_eq!(preview.stop, [first[0].clone()]);
    assert_eq!(preview.unchanged_count, 1);
    assert_eq!(
        list_client_report_recipients(&pool).await.unwrap().len(),
        2,
        "a preview writes nothing"
    );
    assert_eq!(
        replace_client_report_recipients(&pool, &next)
            .await
            .unwrap(),
        preview
    );

    delete_organization(&pool, acme.id).await.unwrap();
    assert!(list_client_report_recipients(&pool)
        .await
        .unwrap()
        .is_empty());
}
