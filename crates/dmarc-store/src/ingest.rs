//! Idempotent report ingestion into SQLite.

use crate::queries::begin_write;
use crate::StoreError;
use dmarc_assess::RecordFacts;
use dmarc_domain::normalize_name;
use dmarc_parser::{
    dkim_auth_aligned, spf_auth_aligned, spf_counts_for_dmarc, AlignmentMode, Record, Report,
};
use sqlx::sqlite::{SqliteConnection, SqlitePool};
use sqlx::{Sqlite, Transaction};

/// Outcome of attempting to ingest a parsed report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestResult {
    /// Report was newly inserted (along with its records).
    Inserted { report_pk: i64 },
    /// A report with the same reporter and `report_id` already existed; nothing changed.
    Duplicate { report_pk: i64 },
    /// The report's domain is ignored (removed): the report was dropped.
    Ignored,
}

fn bool_int(v: bool) -> i64 {
    i64::from(v)
}

/// Report tokens (auth results, disposition, override reason) as SQL compares
/// them: trimmed and lowercase, so `result = 'pass'` agrees with the
/// case-insensitive [`RecordFacts`] behind `dmarc_pass`, and the outcome conditions in
/// [`crate::outcome`] see `none` / `forwarded` however the reporter wrote them.
/// Blank becomes NULL.
fn normalize_token(raw: Option<&str>) -> Option<String> {
    raw.map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
}

/// One record's checks with the alignment computed at ingest, and the facts
/// `records` stores as columns, decided by [`RecordFacts`].
struct RecordChecks<'a> {
    record: &'a Record,
    dkim: Vec<(&'a dmarc_parser::DkimAuthResult, bool)>,
    spf: Vec<(&'a dmarc_parser::SpfAuthResult, bool)>,
    facts: RecordFacts,
}

impl<'a> RecordChecks<'a> {
    fn of(rec: &'a Record, dkim_mode: AlignmentMode, spf_mode: AlignmentMode) -> Self {
        let header_from = rec.header_from.as_deref();
        let dkim: Vec<_> = rec
            .dkim_auth
            .iter()
            .map(|d| (d, dkim_auth_aligned(d, header_from, dkim_mode)))
            .collect();
        let spf: Vec<_> = rec
            .spf_auth
            .iter()
            .map(|s| {
                let aligned = spf_counts_for_dmarc(s, &rec.spf_auth)
                    && spf_auth_aligned(s, header_from, spf_mode);
                (s, aligned)
            })
            .collect();
        let facts = RecordFacts::of(
            spf.iter().map(|(s, aligned)| (s.result.as_str(), *aligned)),
            dkim.iter()
                .map(|(d, aligned)| (d.result.as_str(), *aligned)),
            rec.policy_overrides
                .iter()
                .map(|o| (o.reason_type.as_deref(), o.comment.as_deref())),
        );
        Self {
            record: rec,
            dkim,
            spf,
            facts,
        }
    }
}

/// Insert per-auth-result and override rows for one record.
async fn insert_record_children(
    conn: &mut SqliteConnection,
    record_id: i64,
    checks: &RecordChecks<'_>,
) -> Result<(), StoreError> {
    let rec = checks.record;
    for (dkim, aligned) in &checks.dkim {
        sqlx::query(
            r#"
            INSERT INTO record_dkim_auth (record_id, domain, selector, result, aligned)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(record_id)
        .bind(&dkim.domain)
        .bind(&dkim.selector)
        .bind(normalize_token(Some(&dkim.result)))
        .bind(bool_int(*aligned))
        .execute(&mut *conn)
        .await?;
    }

    for (spf, aligned) in &checks.spf {
        sqlx::query(
            r#"
            INSERT INTO record_spf_auth (record_id, domain, scope, result, aligned)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(record_id)
        .bind(&spf.domain)
        .bind(&spf.scope)
        .bind(normalize_token(Some(&spf.result)))
        .bind(bool_int(*aligned))
        .execute(&mut *conn)
        .await?;
    }

    for reason in &rec.policy_overrides {
        sqlx::query(
            r#"
            INSERT INTO record_overrides (record_id, reason_type, comment)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(record_id)
        .bind(normalize_token(reason.reason_type.as_deref()))
        .bind(&reason.comment)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

/// Message tallies of a report, summed from the facts its records are stored
/// with, so `reports.compliant_messages` cannot disagree with `records.dmarc_pass`.
struct RecordTotals {
    total_messages: i64,
    compliant_messages: i64,
}

impl RecordTotals {
    fn of(checks: &[RecordChecks<'_>]) -> Self {
        // The parser bounds each count; saturating so no input can wrap or panic.
        checks.iter().fold(
            Self {
                total_messages: 0,
                compliant_messages: 0,
            },
            |sum, c| Self {
                total_messages: sum.total_messages.saturating_add(c.record.count),
                compliant_messages: if c.facts.dmarc_pass() {
                    sum.compliant_messages.saturating_add(c.record.count)
                } else {
                    sum.compliant_messages
                },
            },
        )
    }
}

/// Insert record rows plus child auth/override tables for a newly ingested report.
async fn insert_records_with_detail(
    tx: &mut Transaction<'_, Sqlite>,
    report_pk: i64,
    checks: &[RecordChecks<'_>],
) -> Result<(), StoreError> {
    for checks in checks {
        let rec = checks.record;
        let facts = checks.facts;
        let inserted = sqlx::query(
            r#"
            INSERT INTO records (
                report_id, source_ip, count, disposition, dkim_result, spf_result,
                header_from, envelope_from, envelope_to, dmarc_pass,
                spf_aligned_pass, dkim_aligned_pass, forwarded
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(report_pk)
        .bind(&rec.source_ip)
        .bind(rec.count)
        .bind(normalize_token(rec.disposition.as_deref()))
        .bind(normalize_token(rec.dkim_result.as_deref()))
        .bind(normalize_token(rec.spf_result.as_deref()))
        .bind(&rec.header_from)
        .bind(&rec.envelope_from)
        .bind(&rec.envelope_to)
        .bind(bool_int(facts.dmarc_pass()))
        .bind(bool_int(facts.spf_aligned_pass))
        .bind(bool_int(facts.dkim_aligned_pass))
        .bind(bool_int(facts.forwarded))
        .execute(&mut **tx)
        .await?;

        insert_record_children(tx, inserted.last_insert_rowid(), checks).await?;
    }
    Ok(())
}

/// Ingest a parsed DMARC report: resolve the domain, then insert report + records.
///
/// The report's domain is looked up by name across all organizations; a domain not
/// seen before is created in the default organization, and an ignored one drops the
/// report.
/// A duplicate (`org_name`, `report_id`) is a no-op, so re-fetching the same mail
/// is safe. Different reporters may reuse an id; each report is kept.
///
/// Everything happens in one write transaction, so a domain ignored meanwhile
/// (which deletes its reports) can never end up holding this one.
pub async fn ingest_report(pool: &SqlitePool, report: &Report) -> Result<IngestResult, StoreError> {
    let mut tx = begin_write(pool).await?;
    let result = ingest_in(&mut tx, report).await?;
    tx.commit().await?;
    Ok(result)
}

/// Why [`ingest_reports`] stored nothing.
#[derive(Debug)]
pub struct BatchIngestError {
    /// The position in the slice of the report that failed; `None` when the
    /// transaction itself could not begin or commit.
    pub index: Option<usize>,
    pub error: StoreError,
}

/// [`ingest_report`] for several reports in one write transaction, in order:
/// one commit for a whole batch of mail instead of one per report, which is
/// what a first import on a slow filesystem waits for. Any failure rolls back
/// the whole call, so nothing of the batch is stored, and names the report
/// that failed; the caller decides whether to leave it out and store the
/// rest again. A report repeated in the slice is stored once, then a
/// duplicate.
pub async fn ingest_reports(
    pool: &SqlitePool,
    reports: &[&Report],
) -> Result<Vec<IngestResult>, BatchIngestError> {
    let failed = |index| move |error| BatchIngestError { index, error };
    let mut tx = begin_write(pool).await.map_err(failed(None))?;
    let mut results = Vec::with_capacity(reports.len());
    for (index, report) in reports.iter().enumerate() {
        results.push(
            ingest_in(&mut tx, report)
                .await
                .map_err(failed(Some(index)))?,
        );
    }
    tx.commit()
        .await
        .map_err(|e| failed(None)(StoreError::from(e)))?;
    Ok(results)
}

/// The body of [`ingest_report`] inside the caller's write transaction.
async fn ingest_in(
    tx: &mut Transaction<'_, Sqlite>,
    report: &Report,
) -> Result<IngestResult, StoreError> {
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM reports WHERE org_name = ? AND report_id = ?")
            .bind(&report.org_name)
            .bind(&report.report_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(report_pk) = existing {
        return Ok(IngestResult::Duplicate { report_pk });
    }

    // A domain not seen before lands in the default organization.
    let name = normalize_name(&report.policy.domain);
    sqlx::query(
        "INSERT OR IGNORE INTO domains (organization_id, domain) \
         SELECT id, ? FROM organizations WHERE is_default = 1",
    )
    .bind(&name)
    .execute(&mut **tx)
    .await?;
    let (domain_id, ignored): (i64, bool) =
        sqlx::query_as("SELECT id, ignored FROM domains WHERE domain = ?")
            .bind(&name)
            .fetch_one(&mut **tx)
            .await?;
    if ignored {
        return Ok(IngestResult::Ignored);
    }

    let dkim_mode = AlignmentMode::from_tag(report.policy.adkim.as_deref());
    let spf_mode = AlignmentMode::from_tag(report.policy.aspf.as_deref());
    let checks: Vec<_> = report
        .records
        .iter()
        .map(|rec| RecordChecks::of(rec, dkim_mode, spf_mode))
        .collect();
    // Tallies derived from the rows about to be written, not the caller's.
    let totals = RecordTotals::of(&checks);

    let insert = sqlx::query(
        r#"
        INSERT INTO reports (
            report_id, org_name, email, domain_id, date_begin, date_end,
            policy_p, policy_sp, policy_pct, total_messages, compliant_messages
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&report.report_id)
    .bind(&report.org_name)
    .bind(&report.email)
    .bind(domain_id)
    .bind(report.date_begin)
    .bind(report.date_end)
    .bind(&report.policy.p)
    .bind(&report.policy.sp)
    .bind(report.policy.pct.map(i64::from))
    .bind(totals.total_messages)
    .bind(totals.compliant_messages)
    .execute(&mut **tx)
    .await?;

    let report_pk = insert.last_insert_rowid();
    sqlx::query("INSERT INTO report_xml (report_id, xml_gz) VALUES (?, ?)")
        .bind(report_pk)
        .bind(gzip(&report.raw_xml)?)
        .execute(&mut **tx)
        .await?;
    insert_records_with_detail(tx, report_pk, &checks).await?;
    Ok(IngestResult::Inserted { report_pk })
}

/// `xml` gzip-compressed for `report_xml` (report XML shrinks about 10-20x).
fn gzip(xml: &str) -> Result<Vec<u8>, StoreError> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(xml.as_bytes())?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_parser::{DkimAuthResult, PolicyOverride, SpfAuthResult};

    fn rec(header_from: &str, dkim: Vec<DkimAuthResult>, spf: Vec<SpfAuthResult>) -> Record {
        Record {
            source_ip: "192.0.2.1".into(),
            count: 1,
            disposition: None,
            dkim_result: None,
            spf_result: None,
            header_from: Some(header_from.into()),
            envelope_from: None,
            envelope_to: None,
            dkim_auth: dkim,
            spf_auth: spf,
            policy_overrides: Vec::<PolicyOverride>::new(),
        }
    }

    fn dkim(domain: &str, result: &str) -> DkimAuthResult {
        DkimAuthResult {
            domain: domain.into(),
            selector: None,
            result: result.into(),
        }
    }

    fn spf(domain: &str, result: &str) -> SpfAuthResult {
        SpfAuthResult {
            domain: domain.into(),
            result: result.into(),
            scope: None,
        }
    }

    fn facts(record: &Record, mode: AlignmentMode) -> RecordFacts {
        RecordChecks::of(record, mode, mode).facts
    }

    #[test]
    fn dmarc_pass_needs_pass_and_aligned() {
        use AlignmentMode::{Relaxed, Strict};
        let pass_aligned = rec(
            "example.com",
            vec![dkim("mail.example.com", "pass")],
            vec![],
        );
        let f = facts(&pass_aligned, Relaxed);
        assert!(f.dkim_aligned_pass && !f.spf_aligned_pass && f.dmarc_pass());
        // Strict: a subdomain signature does not align.
        assert!(!facts(&pass_aligned, Strict).dmarc_pass());

        let fail_aligned = rec("example.com", vec![], vec![spf("example.com", "fail")]);
        assert!(!facts(&fail_aligned, Relaxed).dmarc_pass());

        let pass_unaligned = rec(
            "example.com",
            vec![dkim("evil.example", "pass")],
            vec![spf("unrelated.net", "pass")],
        );
        assert_eq!(facts(&pass_unaligned, Relaxed), RecordFacts::default());
        assert_eq!(
            facts(&rec("example.com", vec![], vec![]), Relaxed),
            RecordFacts::default()
        );

        // Any one passing aligned result is enough.
        let mixed = rec(
            "example.com",
            vec![dkim("evil.example", "pass"), dkim("example.com", "fail")],
            vec![spf("mail.example.com", "PASS")],
        );
        let f = facts(&mixed, Relaxed);
        assert!(f.spf_aligned_pass && !f.dkim_aligned_pass && f.dmarc_pass());
    }

    #[test]
    fn helo_spf_counts_only_without_a_mail_from_result() {
        let helo = |domain: &str| SpfAuthResult {
            scope: Some("helo".into()),
            ..spf(domain, "pass")
        };
        let mfrom_fail = SpfAuthResult {
            scope: Some("mfrom".into()),
            ..spf("bounce.other.net", "fail")
        };
        // A HELO pass beside a failing MAIL FROM result is not DMARC's SPF.
        let r = rec(
            "example.com",
            vec![],
            vec![helo("mail.example.com"), mfrom_fail],
        );
        assert!(!facts(&r, AlignmentMode::Relaxed).dmarc_pass());
        // Alone (an empty MAIL FROM), the HELO result stands in.
        let r = rec("example.com", vec![], vec![helo("mail.example.com")]);
        assert!(facts(&r, AlignmentMode::Relaxed).dmarc_pass());
    }
}
