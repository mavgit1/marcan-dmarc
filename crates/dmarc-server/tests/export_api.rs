//! CSV exports under `/api/export/`: format (BOM, quoting, formula guard, ISO
//! timestamps), every endpoint's header row, scope and filters, and that large
//! exports are not truncated.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use dmarc_parser::{
    DkimAuthResult, PolicyOverride, PolicyPublished, Record, Report, SpfAuthResult,
};
use dmarc_server::router;
use dmarc_store::{
    find_domain_by_name, get_or_create_domain, get_or_create_organization, ignore_domain,
    ingest_report, init, upsert_dns_snapshot, upsert_identity_snapshot, DnsSnapshot,
};
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use std::collections::HashMap;
use tower::ServiceExt;
mod common;
use common::get_ok;

const MONDAY: i64 = 1_704_067_200; // 2024-01-01T00:00:00Z
const DAY: i64 = 86_400;

struct RecSpec<'a> {
    ip: &'a str,
    count: i64,
    dkim: &'a str,
    spf: &'a str,
    disposition: &'a str,
    header_from: &'a str,
    envelope_from: Option<&'a str>,
    selector: Option<&'a str>,
    overrides: Vec<PolicyOverride>,
}

impl<'a> RecSpec<'a> {
    fn new(ip: &'a str, count: i64, dkim: &'a str, spf: &'a str) -> Self {
        Self {
            ip,
            count,
            dkim,
            spf,
            disposition: "none",
            header_from: "example.com",
            envelope_from: None,
            selector: None,
            overrides: vec![],
        }
    }

    fn record(self) -> Record {
        Record {
            source_ip: self.ip.into(),
            count: self.count,
            disposition: Some(self.disposition.into()),
            dkim_result: Some(self.dkim.into()),
            spf_result: Some(self.spf.into()),
            header_from: Some(self.header_from.into()),
            envelope_from: self.envelope_from.map(str::to_owned),
            envelope_to: None,
            dkim_auth: vec![DkimAuthResult {
                domain: self.header_from.into(),
                selector: self.selector.map(str::to_owned),
                result: self.dkim.into(),
            }],
            spf_auth: vec![SpfAuthResult {
                domain: self.header_from.into(),
                result: self.spf.into(),
                scope: None,
            }],
            policy_overrides: self.overrides,
        }
    }
}

fn report(id: &str, org: &str, domain: &str, begin: i64, records: Vec<RecSpec>) -> Report {
    let records: Vec<Record> = records.into_iter().map(RecSpec::record).collect();
    Report {
        report_id: id.into(),
        org_name: org.into(),
        email: "noreply@reporter.example".into(),
        date_begin: begin,
        date_end: begin + DAY - 1,
        policy: PolicyPublished {
            domain: domain.into(),
            adkim: Some("r".into()),
            aspf: Some("r".into()),
            p: Some("quarantine".into()),
            sp: Some("none".into()),
            pct: Some(100),
            fo: None,
        },
        records,
        raw_xml: "<feedback/>".into(),
    }
}

/// Acme owns example.com, Beta owns other.org.
async fn fixture() -> (tempfile::TempDir, SqlitePool, axum::Router, i64, i64) {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    let acme = get_or_create_organization(&pool, "Acme").await.unwrap();
    let beta = get_or_create_organization(&pool, "Beta").await.unwrap();
    get_or_create_domain(&pool, acme.id, "example.com")
        .await
        .unwrap();
    get_or_create_domain(&pool, beta.id, "other.org")
        .await
        .unwrap();

    let mut good = RecSpec::new("192.0.2.1", 40, "pass", "pass");
    good.envelope_from = Some("mail.example.com");
    good.selector = Some("s1");
    let mut forwarded = RecSpec::new("192.0.2.2", 10, "fail", "fail");
    forwarded.header_from = "foo,bar.example";
    forwarded.overrides = vec![PolicyOverride {
        reason_type: Some("forwarded".into()),
        comment: Some(r#"said "hello, world""#.into()),
    }];
    ingest_report(
        &pool,
        &report(
            "r1",
            "google.com",
            "example.com",
            MONDAY,
            vec![good, forwarded],
        ),
    )
    .await
    .unwrap();

    let mut dkim_only = RecSpec::new("192.0.2.1", 50, "pass", "fail");
    dkim_only.selector = Some("s1");
    ingest_report(
        &pool,
        &report(
            "r1", // same id, different reporter: kept (UNIQUE per org_name)
            "=1+2",
            "example.com",
            MONDAY + 2 * DAY,
            vec![dkim_only, RecSpec::new("2001:db8::1", 80, "pass", "pass")],
        ),
    )
    .await
    .unwrap();

    let mut spoof = RecSpec::new("192.0.2.99", 20, "fail", "fail");
    spoof.disposition = "reject";
    spoof.header_from = "other.org";
    ingest_report(
        &pool,
        &report(
            "r3",
            "yahoo.com",
            "other.org",
            MONDAY + 2 * DAY,
            vec![spoof],
        ),
    )
    .await
    .unwrap();

    (dir, pool.clone(), router(pool), acme.id, beta.id)
}

struct Csv {
    status: StatusCode,
    content_type: String,
    disposition: String,
    raw: String,
    header: Vec<String>,
    rows: Vec<HashMap<String, String>>,
}

async fn get_csv(app: &axum::Router, path: &str) -> Csv {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let get = |name| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    let content_type = get(header::CONTENT_TYPE);
    let disposition = get(header::CONTENT_DISPOSITION);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let raw = String::from_utf8(bytes.to_vec()).unwrap();
    let mut header = Vec::new();
    let mut rows = Vec::new();
    if status == StatusCode::OK {
        let mut reader = csv::Reader::from_reader(raw.trim_start_matches('\u{feff}').as_bytes());
        header = reader
            .headers()
            .unwrap()
            .iter()
            .map(str::to_owned)
            .collect();
        for record in reader.records() {
            let record = record.unwrap();
            rows.push(
                header
                    .iter()
                    .cloned()
                    .zip(record.iter().map(str::to_owned))
                    .collect(),
            );
        }
    }
    Csv {
        status,
        content_type,
        disposition,
        raw,
        header,
        rows,
    }
}

fn find<'a>(
    rows: &'a [HashMap<String, String>],
    key: &str,
    value: &str,
) -> &'a HashMap<String, String> {
    rows.iter()
        .find(|r| r[key] == value)
        .unwrap_or_else(|| panic!("no row with {key}={value}"))
}

#[tokio::test]
async fn every_export_has_bom_header_row_and_attachment() {
    let (_dir, _pool, app, _, _) = fixture().await;
    let cases: [(&str, &str, &[&str]); 4] = [
        (
            "domains",
            "/api/export/domains.csv",
            &[
                "organization",
                "domain",
                "policy_p",
                "policy_sp",
                "policy_pct",
                "total_messages",
                "compliant_messages",
                "delivered_compliance_pct",
                "delivered_failures",
                "forwarded_failures",
                "blocked_by_policy",
                "no_aligned_dkim_messages",
                "parked",
                "reporter_count",
                "report_count",
                "last_report_at",
                "dns_grade",
                "dns_score",
                "dns_critical_count",
                "dns_checked_at",
                "health",
                "issues",
            ],
        ),
        (
            "senders",
            "/api/export/senders.csv",
            &[
                "organization",
                "domain",
                "source_ip",
                "sender_name",
                "operator_domain",
                "ptr_hostname",
                "total_messages",
                "compliant_messages",
                "compliance_pct",
                "spf_aligned_pass_messages",
                "dkim_aligned_pass_messages",
                "no_aligned_dkim_messages",
                "delivered_failures",
                "forwarded_failures",
                "blocked_by_policy",
                "envelope_from",
                "dkim_selectors",
                "reporters",
                "report_count",
                "first_seen",
                "last_seen",
            ],
        ),
        (
            "auth-issues",
            "/api/export/auth-issues.csv",
            &[
                "report_id",
                "record_id",
                "date_begin",
                "date_end",
                "reporter",
                "organization",
                "domain",
                "source_ip",
                "messages",
                "dmarc_pass",
                "spf_aligned_pass",
                "dkim_aligned_pass",
                "outcome",
                "issue",
                "disposition",
                "delivered",
                "header_from",
                "envelope_from",
                "spf_domain",
                "dkim_domain",
                "dkim_selector",
                "reporter_spf_verdict",
                "reporter_dkim_verdict",
                "policy_override",
            ],
        ),
        (
            "dns-findings",
            "/api/export/dns-findings.csv",
            &[
                "organization",
                "domain",
                "checked_at",
                "grade",
                "score",
                "check",
                "severity",
                "code",
                "points",
                "message",
                "suggestion",
            ],
        ),
    ];
    for (name, path, expected) in cases {
        let csv = get_csv(&app, path).await;
        assert_eq!(csv.status, StatusCode::OK, "{path}");
        assert_eq!(csv.content_type, "text/csv; charset=utf-8");
        assert!(
            csv.disposition
                .starts_with(&format!("attachment; filename=\"marcan-{name}-")),
            "{}",
            csv.disposition
        );
        assert!(csv.disposition.ends_with(".csv\""), "{}", csv.disposition);
        assert!(csv.raw.starts_with('\u{feff}'), "{path} lacks a BOM");
        let first_line = csv
            .raw
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .unwrap();
        assert_eq!(first_line, expected.join(","), "{path}");
        assert_eq!(csv.header, expected);
        assert!(csv.raw.ends_with("\r\n"));
    }
}

#[tokio::test]
async fn auth_issues_rows_escaping_and_filters() {
    let (_dir, _pool, app, _, beta) = fixture().await;
    let csv = get_csv(&app, "/api/export/auth-issues.csv").await;
    // 192.0.2.2 and 192.0.2.99 failed both; 192.0.2.1 failed SPF while DKIM passed.
    assert_eq!(csv.rows.len(), 3);

    let fwd = find(&csv.rows, "source_ip", "192.0.2.2");
    assert_eq!(fwd["report_id"], "r1");
    assert_eq!(fwd["reporter"], "google.com");
    assert_eq!(fwd["organization"], "Acme");
    assert_eq!(fwd["domain"], "example.com");
    assert_eq!(fwd["date_begin"], "2024-01-01T00:00:00Z");
    assert_eq!(fwd["date_end"], "2024-01-01T23:59:59Z");
    assert_eq!(fwd["messages"], "10");
    assert_eq!(fwd["dmarc_pass"], "false");
    assert_eq!(fwd["issue"], "both_failed");
    assert_eq!(fwd["disposition"], "none");
    assert_eq!(fwd["delivered"], "true");
    // Comma and quotes survive the round trip.
    assert_eq!(fwd["header_from"], "foo,bar.example");
    assert_eq!(fwd["policy_override"], r#"forwarded: said "hello, world""#);
    assert_eq!(fwd["envelope_from"], "", "null is an empty cell");
    assert_eq!(fwd["reporter_spf_verdict"], "fail");
    assert!(csv.raw.contains("\"foo,bar.example\""));

    let dkim_only = find(&csv.rows, "source_ip", "192.0.2.1");
    assert_eq!(dkim_only["issue"], "spf_failed");
    assert_eq!(dkim_only["spf_aligned_pass"], "false");
    assert_eq!(dkim_only["dkim_aligned_pass"], "true");
    assert_eq!(dkim_only["dmarc_pass"], "true");
    assert_eq!(dkim_only["dkim_selector"], "s1");
    // Report-supplied text cannot start a formula.
    assert_eq!(dkim_only["reporter"], "'=1+2");

    let spoof = find(&csv.rows, "source_ip", "192.0.2.99");
    assert_eq!(spoof["disposition"], "reject");
    assert_eq!(spoof["delivered"], "false");
    assert_eq!(spoof["outcome"], "blocked");
    assert_eq!(fwd["outcome"], "forwarded");
    assert_eq!(dkim_only["outcome"], "passed");

    for (query, expected) in [
        ("outcome=dkim_only", 1),
        ("outcome=spf_only", 0),
        ("outcome=all_failures", 2),
        ("outcome=blocked", 1),
        ("outcome=forwarded", 1),
        ("outcome=delivered", 0),
        ("source_ip=192.0.2.2", 1),
        (&format!("since={}", MONDAY + DAY) as &str, 2),
        (&format!("organization_id={beta}") as &str, 1),
    ] {
        let csv = get_csv(&app, &format!("/api/export/auth-issues.csv?{query}")).await;
        assert_eq!(csv.rows.len(), expected, "{query}");
    }
    let bad = get_csv(&app, "/api/export/auth-issues.csv?outcome=bogus").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    assert!(bad.raw.contains("\"code\":\"bad_request\""));
}

#[tokio::test]
async fn senders_rows_are_per_ip_and_domain_and_fully_scoped() {
    let (_dir, pool, app, acme, beta) = fixture().await;
    upsert_identity_snapshot(
        &pool,
        "192.0.2.1",
        MONDAY,
        Some("mx1.mail.example.net"),
        Some(true),
    )
    .await
    .unwrap();

    let csv = get_csv(&app, "/api/export/senders.csv").await;
    assert_eq!(csv.rows.len(), 4);
    let row = find(&csv.rows, "source_ip", "192.0.2.1");
    assert_eq!(row["organization"], "Acme");
    assert_eq!(row["domain"], "example.com");
    assert_eq!(row["ptr_hostname"], "mx1.mail.example.net");
    assert_eq!(row["operator_domain"], "example.net");
    assert_eq!(row["sender_name"], "", "example.net is not in the name map");
    assert_eq!(row["total_messages"], "90");
    assert_eq!(row["compliant_messages"], "90");
    assert_eq!(row["compliance_pct"], "100.00");
    assert_eq!(row["spf_aligned_pass_messages"], "40");
    assert_eq!(row["dkim_aligned_pass_messages"], "90");
    assert_eq!(row["delivered_failures"], "0");
    assert_eq!(row["blocked_by_policy"], "0");
    assert_eq!(row["envelope_from"], "mail.example.com");
    assert_eq!(row["dkim_selectors"], "s1");
    assert_eq!(row["reporters"], "'=1+2;google.com");
    assert_eq!(row["report_count"], "2");
    assert_eq!(row["first_seen"], "2024-01-01T00:00:00Z");
    assert_eq!(row["last_seen"], "2024-01-03T23:59:59Z");

    let spoof = find(&csv.rows, "source_ip", "192.0.2.99");
    assert_eq!(spoof["blocked_by_policy"], "20");
    assert_eq!(spoof["compliance_pct"], "0.00");
    assert_eq!(spoof["operator_domain"], "");

    // Every column follows `since`, not just the row set.
    let csv = get_csv(
        &app,
        &format!("/api/export/senders.csv?since={}", MONDAY + DAY),
    )
    .await;
    let row = find(&csv.rows, "source_ip", "192.0.2.1");
    assert_eq!(row["total_messages"], "50");
    assert_eq!(row["spf_aligned_pass_messages"], "0");
    assert_eq!(row["report_count"], "1");
    assert_eq!(row["reporters"], "'=1+2");
    assert_eq!(row["first_seen"], "2024-01-03T00:00:00Z");

    let csv = get_csv(
        &app,
        &format!("/api/export/senders.csv?organization_id={beta}"),
    )
    .await;
    assert_eq!(csv.rows.len(), 1);
    let csv = get_csv(
        &app,
        &format!("/api/export/senders.csv?organization_id={acme}"),
    )
    .await;
    assert_eq!(csv.rows.len(), 3);

    // The totals match the senders list.
    let list = get_ok(&app, &format!("/api/senders?organization_id={acme}")).await;
    let list_total: i64 = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["total_messages"].as_i64().unwrap())
        .sum();
    let csv_total: i64 = csv
        .rows
        .iter()
        .map(|r| r["total_messages"].parse::<i64>().unwrap())
        .sum();
    assert_eq!(csv_total, list_total);
}

#[tokio::test]
async fn domains_rows_match_the_domain_list() {
    let (_dir, _pool, app, acme, _) = fixture().await;
    let csv = get_csv(&app, "/api/export/domains.csv").await;
    assert_eq!(csv.rows.len(), 2);
    let json = get_ok(&app, "/api/domains").await;
    for item in json["items"].as_array().unwrap() {
        let row = find(&csv.rows, "domain", item["domain"].as_str().unwrap());
        assert_eq!(
            row["organization"],
            item["organization"]["name"].as_str().unwrap()
        );
        assert_eq!(row["total_messages"], item["total_messages"].to_string());
        assert_eq!(
            row["compliant_messages"],
            item["compliant_messages"].to_string()
        );
        assert_eq!(
            row["delivered_failures"],
            item["delivered_failures"].to_string()
        );
        assert_eq!(row["report_count"], item["report_count"].to_string());
        let assessment = &item["assessment"];
        // The rate health is judged on, as the domain list shows it.
        let delivered = item["delivered_compliance"]["pct"]
            .as_f64()
            .map(|p| format!("{p:.2}"))
            .unwrap_or_default();
        assert_eq!(row["delivered_compliance_pct"], delivered);
        assert_eq!(row["health"], item["health"]["state"].as_str().unwrap());
        let issues: Vec<&str> = assessment["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["kind"].as_str().unwrap())
            .collect();
        assert_eq!(row["issues"], issues.join(";"));
    }
    let ex = find(&csv.rows, "domain", "example.com");
    assert!(
        ex["issues"].contains("reports_stopped"),
        "last report in 2024"
    );
    // Never checked in DNS: no policy, whatever the reports said.
    assert_eq!(ex["policy_p"], "");
    assert_eq!(ex["policy_sp"], "");
    assert_eq!(ex["policy_pct"], "");
    assert_eq!(ex["total_messages"], "180");
    assert_eq!(ex["reporter_count"], "2");
    assert_eq!(ex["last_report_at"], "2024-01-03T23:59:59Z");
    assert_eq!(ex["dns_grade"], "", "never checked");
    let other = find(&csv.rows, "domain", "other.org");
    assert_eq!(other["blocked_by_policy"], "20");

    let csv = get_csv(
        &app,
        &format!("/api/export/domains.csv?organization_id={acme}"),
    )
    .await;
    assert_eq!(csv.rows.len(), 1);
    let csv = get_csv(
        &app,
        &format!("/api/export/domains.csv?since={}", MONDAY + DAY),
    )
    .await;
    assert_eq!(
        find(&csv.rows, "domain", "example.com")["total_messages"],
        "130"
    );
    let bad = get_csv(&app, "/api/export/domains.csv?since=yesterday").await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

/// Store a DNS snapshot for `domain` (no live DNS); returns its report.
async fn store_snapshot(pool: &SqlitePool, domain: &str) -> dmarc_dns::DnsReport {
    use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
    use dmarc_dns::dmarc::evaluate_dmarc;
    use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
    use dmarc_dns::spf::evaluate_spf;
    use dmarc_dns::{assemble_report, TxtAnswer, TxtLookup};

    let txt = |v: &str| TxtLookup::Records(TxtAnswer::from_records(vec![v.into()]));
    let report = assemble_report(
        domain,
        evaluate_dmarc(domain, &txt("v=DMARC1; p=none"), &[]),
        evaluate_spf(domain, &txt("v=spf1 ~all"), &[]),
        evaluate_dkim(domain, &[], &DkimProbe::default()),
        evaluate_mta_sts(
            domain,
            &TxtLookup::NoRecords,
            None,
            &PolicyFetch::NotAttempted,
        ),
        evaluate_tls_rpt(domain, &TxtLookup::NoRecords),
        evaluate_bimi(domain, &TxtLookup::NoRecords),
    );
    upsert_dns_snapshot(
        pool,
        &DnsSnapshot {
            checked_at: MONDAY,
            checker_version: 1,
            ..dmarc_server::dns_snapshot(&report).unwrap()
        },
    )
    .await
    .unwrap();
    report
}

#[tokio::test]
async fn dns_findings_come_from_stored_snapshots_with_points() {
    let (_dir, pool, app, _, beta) = fixture().await;
    let domain = "example.com";
    let report = store_snapshot(&pool, domain).await;

    let csv = get_csv(&app, "/api/export/dns-findings.csv").await;
    assert_eq!(csv.rows.len(), report.findings.len());
    assert!(csv.rows.iter().all(|r| r["organization"] == "Acme"
        && r["domain"] == domain
        && r["checked_at"] == "2024-01-01T00:00:00Z"
        && r["score"] == report.score.to_string()));
    let points: i64 = csv
        .rows
        .iter()
        .map(|r| r["points"].parse::<i64>().unwrap())
        .sum();
    assert_eq!(100 - points, i64::from(report.score));
    let missing_dmarc_policy = csv.rows.iter().find(|r| r["check"] == "dmarc").unwrap();
    assert!(!missing_dmarc_policy["severity"].is_empty());

    let csv = get_csv(
        &app,
        &format!("/api/export/dns-findings.csv?organization_id={beta}"),
    )
    .await;
    assert!(csv.rows.is_empty(), "other.org has no snapshot");
}

/// A removed (blocked) domain is in none of the four exports.
#[tokio::test]
async fn removed_domains_are_left_out_of_every_export() {
    let (_dir, pool, app, _, _) = fixture().await;
    store_snapshot(&pool, "example.com").await;
    store_snapshot(&pool, "other.org").await;
    let exports = [
        "/api/export/domains.csv",
        "/api/export/senders.csv",
        "/api/export/auth-issues.csv",
        "/api/export/dns-findings.csv",
    ];
    for path in exports {
        let csv = get_csv(&app, path).await;
        assert!(
            csv.rows.iter().any(|r| r["domain"] == "example.com"),
            "{path} lists example.com before"
        );
    }

    let id = find_domain_by_name(&pool, "example.com")
        .await
        .unwrap()
        .unwrap()
        .id;
    assert!(ignore_domain(&pool, id).await.unwrap());
    for path in exports {
        let csv = get_csv(&app, path).await;
        assert_eq!(csv.status, StatusCode::OK, "{path}");
        assert!(
            csv.rows.iter().all(|r| r["domain"] != "example.com"),
            "{path} still lists example.com"
        );
        assert!(
            csv.rows.iter().any(|r| r["domain"] == "other.org"),
            "{path} lost other.org"
        );
        let scoped = get_csv(&app, &format!("{path}?domain_id={id}")).await;
        assert!(scoped.rows.is_empty(), "{path}?domain_id");
    }
}

/// The old exports stopped at 10,000 rows; these stream every row.
#[tokio::test]
async fn large_exports_are_not_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let pool = init(&dir.path().join("t.db")).await.unwrap();
    const N: i64 = 10_050;
    sqlx::query("INSERT INTO domains (id, organization_id, domain) VALUES (5, 1, 'bulk.example')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO reports (id, report_id, org_name, email, domain_id, date_begin, date_end, \
         total_messages, compliant_messages) \
         VALUES (1, 'bulk', 'google.com', 'x@y', 5, 1704067200, 1704153599, ?, 0)",
    )
    .bind(N)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "WITH RECURSIVE n(x) AS (SELECT 0 UNION ALL SELECT x + 1 FROM n WHERE x + 1 < ?) \
         INSERT INTO records (report_id, source_ip, count, disposition, header_from, dmarc_pass) \
         SELECT 1, '10.' || (x / 65536) || '.' || ((x / 256) % 256) || '.' || (x % 256), 1, \
                'none', 'bulk.example', 0 FROM n",
    )
    .bind(N)
    .execute(&pool)
    .await
    .unwrap();
    let app = router(pool);

    let issues = get_csv(&app, "/api/export/auth-issues.csv").await;
    assert_eq!(issues.rows.len() as i64, N);
    assert!(
        issues.raw.len() > 64 * 1024,
        "spans several streamed chunks"
    );
    let senders = get_csv(&app, "/api/export/senders.csv").await;
    assert_eq!(senders.rows.len() as i64, N);
    let total: i64 = senders
        .rows
        .iter()
        .map(|r| r["total_messages"].parse::<i64>().unwrap())
        .sum();
    assert_eq!(total, N);
}

/// Every export's bytes on the fixture (formula guard, quoted commas and
/// quotes, empty cells, CRLF, BOM), pinned in `tests/snapshots/`. A change
/// to the CSV encoding shows here as a diff; after a deliberate change to the
/// columns or the fixture, rewrite the files with
/// `MARCAN_UPDATE_SNAPSHOTS=1 cargo test -p dmarc-server --test export_api`.
#[tokio::test]
async fn exports_match_their_snapshots_byte_for_byte() {
    let (_dir, pool, app, _, _) = fixture().await;
    store_snapshot(&pool, "example.com").await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let update = std::env::var_os("MARCAN_UPDATE_SNAPSHOTS").is_some();
    for name in ["domains", "senders", "auth-issues", "dns-findings"] {
        let csv = get_csv(&app, &format!("/api/export/{name}.csv")).await;
        assert_eq!(csv.status, StatusCode::OK, "{name}");
        let path = dir.join(format!("export-{name}.csv"));
        if update {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, csv.raw.as_bytes()).unwrap();
            continue;
        }
        let expected = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            csv.raw.as_bytes() == expected,
            "{name}.csv differs from {}:\n{}\nRewrite with MARCAN_UPDATE_SNAPSHOTS=1 if the change is meant.",
            path.display(),
            csv.raw.escape_debug()
        );
    }
}
