//! CSV downloads under `/api/export/`: domains, senders, auth issues and DNS
//! findings.
//!
//! Every file is UTF-8 with a byte-order mark (so spreadsheet apps detect the
//! encoding), RFC 4180 quoting (the `csv` crate, through [`crate::csv_text`]),
//! timestamps as ISO 8601 UTC (`2026-10-01T16:00:37Z`), booleans as `true` / `false`, and an empty cell
//! for "no value". A cell starting with `=`, `+`, `-` or `@` gets a leading `'`
//! so a spreadsheet does not evaluate report-supplied text as a formula.
//!
//! There is no row limit: rows are written to the response while the query is
//! still running. Parameters are validated before the first byte, so a bad
//! filter is still a JSON 400; a database error mid-stream aborts the download.

use crate::api::domain_detail::FailureFilterQuery;
use crate::api::identity::identities;
use crate::csv_text::{write_line, LineEnd};
use crate::error::AppResult;
use crate::extract::Query;
use crate::filters::ScopeQuery;
use crate::view::domain::{domain_summaries, DomainSummary, Judging};
use crate::AppState;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use dmarc_assess::{AuthIssue, Rate};
use dmarc_dns::DnsReport;
use dmarc_identity::SenderIdentity;
use dmarc_store::{
    list_domain_summaries, sender_export_ips, stream_auth_issue_export, stream_dns_snapshot_export,
    stream_sender_export, unix_now, AuthIssueExportRow, Scope, SenderExportRow, StoreError,
};
use serde::Deserialize;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Bytes buffered before a chunk is handed to the response body.
const CHUNK_BYTES: usize = 64 * 1024;
/// Chunks queued between the query task and the client.
const CHUNKS_IN_FLIGHT: usize = 4;

const DOMAINS_HEADER: &[&str] = &[
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
];

const SENDERS_HEADER: &[&str] = &[
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
];

const AUTH_ISSUES_HEADER: &[&str] = &[
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
];

const DNS_FINDINGS_HEADER: &[&str] = &[
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
];

/// Organization / domain scope of the exports without a time range.
#[derive(Debug, Deserialize)]
pub struct ExportScopeQuery {
    pub organization_id: Option<i64>,
    pub domain_id: Option<i64>,
}

impl ExportScopeQuery {
    fn scope(&self, since: Option<i64>) -> Scope {
        Scope::new(self.organization_id, self.domain_id, since)
    }
}

// ---- cell formatting -------------------------------------------------------

/// A value a spreadsheet could read as a formula gets a leading `'`; nothing
/// is dropped. Applied to every cell and to every item of a [`list`] cell
/// (some spreadsheets split cells on `;`), since report content is untrusted.
fn guard_formula(value: &str) -> std::borrow::Cow<'_, str> {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}").into()
    } else {
        value.into()
    }
}

/// Append one CSV line (CRLF-terminated, as RFC 4180 specifies), every cell
/// guarded against formulas first and then quoted where needed.
fn push_record<S: AsRef<str>>(buf: &mut Vec<u8>, fields: &[S]) {
    write_line(
        buf,
        LineEnd::Crlf,
        fields.iter().map(|field| guard_formula(field.as_ref())),
    );
}

/// Unix seconds as ISO 8601 UTC, e.g. `2026-10-01T16:00:37Z`.
fn iso(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

fn opt_iso(ts: Option<i64>) -> String {
    ts.map(iso).unwrap_or_default()
}

fn opt_text(value: Option<&str>) -> String {
    value.unwrap_or_default().to_owned()
}

fn opt_num(value: Option<i64>) -> String {
    value.map(|v| v.to_string()).unwrap_or_default()
}

fn boolean(value: bool) -> String {
    if value { "true" } else { "false" }.to_owned()
}

/// A [`Rate`] as a percentage with two decimals; empty when unmeasured.
fn rate(rate: &Rate) -> String {
    rate.pct.map(|p| format!("{p:.2}")).unwrap_or_default()
}

/// Several values in one cell, `;`-separated, each guarded on its own.
fn list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| guard_formula(item))
        .collect::<Vec<_>>()
        .join(";")
}

/// A unit enum as its API (`snake_case`) name, e.g. `delivered_failures`.
fn token<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

// ---- streaming -------------------------------------------------------------

type Chunk = Result<Bytes, std::io::Error>;

/// Encodes rows into a buffer and hands full chunks to the response body.
pub(crate) struct CsvSink {
    tx: mpsc::Sender<Chunk>,
    buf: Vec<u8>,
}

/// A full chunk waiting to be sent.
type Pending = Option<(mpsc::Sender<Chunk>, Bytes)>;

impl CsvSink {
    /// Encode one row. Returns a chunk to [`deliver`] once the buffer is full.
    fn push<S: AsRef<str>>(&mut self, fields: &[S]) -> Pending {
        push_record(&mut self.buf, fields);
        self.take_full()
    }

    /// Append already-encoded lines (see [`push_record`]).
    fn extend(&mut self, lines: &[u8]) -> Pending {
        self.buf.extend_from_slice(lines);
        self.take_full()
    }

    fn take_full(&mut self) -> Pending {
        (self.buf.len() >= CHUNK_BYTES)
            .then(|| (self.tx.clone(), Bytes::from(std::mem::take(&mut self.buf))))
    }

    /// Send the rest, or abort the download when the query failed.
    async fn finish(mut self, result: Result<(), StoreError>) {
        let last = match result {
            Ok(()) if self.buf.is_empty() => return,
            Ok(()) => Ok(Bytes::from(std::mem::take(&mut self.buf))),
            Err(e) => {
                tracing::error!(error = %e, "csv export failed mid-stream");
                Err(std::io::Error::other("export failed"))
            }
        };
        let _ = self.tx.send(last).await;
    }
}

/// Send a full chunk; `false` when the client has gone away (stop the query).
async fn deliver(pending: Pending) -> bool {
    match pending {
        Some((tx, chunk)) => tx.send(Ok(chunk)).await.is_ok(),
        None => true,
    }
}

/// A streaming CSV download: BOM and `header` first, then whatever `produce`
/// writes into the sink from a background task.
fn csv_download<F, Fut>(name: &str, header: &[&str], produce: F) -> Response
where
    F: FnOnce(CsvSink) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Chunk>(CHUNKS_IN_FLIGHT);
    let mut buf = Vec::with_capacity(CHUNK_BYTES + 1024);
    buf.extend_from_slice("\u{feff}".as_bytes());
    push_record(&mut buf, header);
    tokio::spawn(produce(CsvSink { tx, buf }));

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    let date = iso(unix_now());
    let filename = format!("marcan-{name}-{}.csv", &date[..date.len().min(10)]);
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

// ---- endpoints -------------------------------------------------------------

fn domain_fields(d: &DomainSummary) -> Vec<String> {
    let row = &d.row;
    let policy = row.policy.as_ref();
    let dns = row.dns.as_ref();
    vec![
        row.organization.name.clone(),
        row.domain.clone(),
        opt_text(policy.and_then(|p| p.p.as_deref())),
        opt_text(policy.and_then(|p| p.sp.as_deref())),
        opt_num(policy.and_then(|p| p.pct)),
        row.mail.total_messages.to_string(),
        row.mail.compliant_messages.to_string(),
        rate(&d.verdicts.delivered_compliance),
        row.mail.delivered_failures.to_string(),
        row.mail.forwarded_failures.to_string(),
        d.verdicts.blocked_messages.to_string(),
        row.no_aligned_dkim_messages.to_string(),
        row.parked.to_string(),
        row.reporter_count.to_string(),
        row.report_count.to_string(),
        opt_iso(row.last_report_at),
        opt_text(dns.map(|s| s.grade.as_str())),
        opt_num(dns.map(|s| s.score)),
        opt_num(dns.map(|s| s.critical_count)),
        opt_iso(dns.map(|s| s.checked_at)),
        token(&d.verdicts.health.state),
        list(
            &d.assessment
                .issues
                .iter()
                .map(|issue| token(&issue.kind))
                .collect::<Vec<_>>(),
        ),
    ]
}

/// `GET /api/export/domains.csv?organization_id&since` — the domain list's rows
/// (same query as `GET /api/domains`).
pub async fn export_domains_csv(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ScopeQuery>,
) -> AppResult<Response> {
    let scope = q.scope()?;
    let pool = state.pool.clone();
    Ok(csv_download(
        "domains",
        DOMAINS_HEADER,
        move |mut sink| async move {
            let listed = match Judging::load(&pool, unix_now()).await {
                Ok(at) => list_domain_summaries(&pool, scope)
                    .await
                    .map(|rows| (rows, at)),
                Err(e) => Err(e),
            };
            let result = match listed {
                Ok((rows, at)) => {
                    for row in &domain_summaries(rows, at) {
                        if !deliver(sink.push(&domain_fields(row))).await {
                            return;
                        }
                    }
                    Ok(())
                }
                Err(e) => Err(e),
            };
            sink.finish(result).await;
        },
    ))
}

fn sender_fields(row: &SenderExportRow, identity: Option<&SenderIdentity>) -> Vec<String> {
    let identity = identity.cloned().unwrap_or_default();
    vec![
        row.organization.clone(),
        row.domain.clone(),
        row.source_ip.clone(),
        identity.name.unwrap_or_default(),
        identity.operator_domain.unwrap_or_default(),
        identity.ptr_hostname.unwrap_or_default(),
        row.mail.total_messages.to_string(),
        row.mail.compliant_messages.to_string(),
        rate(&row.mail.mail().pass_rate()),
        row.spf_aligned_pass_messages.to_string(),
        row.dkim_aligned_pass_messages.to_string(),
        row.no_aligned_dkim_messages.to_string(),
        row.mail.delivered_failures.to_string(),
        row.mail.forwarded_failures.to_string(),
        row.blocked_messages.to_string(),
        list(&row.envelope_from),
        list(&row.dkim_selectors),
        list(&row.reporters),
        row.report_count.to_string(),
        iso(row.first_seen),
        iso(row.last_seen),
    ]
}

/// `GET /api/export/senders.csv?organization_id&domain_id&since` — one row per
/// sender IP × domain.
pub async fn export_senders_csv(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ScopeQuery>,
) -> AppResult<Response> {
    let scope = q.scope()?;
    // Named up front from stored evidence (never live DNS), like every page.
    let ips = sender_export_ips(&state.pool, scope).await?;
    let names = identities(&state, ips.iter().map(String::as_str)).await?;
    let pool = state.pool.clone();
    Ok(csv_download(
        "senders",
        SENDERS_HEADER,
        move |mut sink| async move {
            let result = stream_sender_export(&pool, scope, |row| {
                deliver(sink.push(&sender_fields(&row, names.get(&row.source_ip))))
            })
            .await;
            sink.finish(result).await;
        },
    ))
}

fn auth_issue_fields(row: &AuthIssueExportRow) -> Vec<String> {
    vec![
        row.report_id.clone(),
        row.record_id.to_string(),
        iso(row.date_begin),
        iso(row.date_end),
        row.reporter.clone(),
        row.organization.clone(),
        row.domain.clone(),
        row.source_ip.clone(),
        row.messages.to_string(),
        boolean(row.dmarc_pass),
        boolean(row.spf_aligned_pass),
        boolean(row.dkim_aligned_pass),
        row.outcome.as_str().to_owned(),
        AuthIssue::of(row.spf_aligned_pass, row.dkim_aligned_pass)
            .map(|issue| issue.as_str().to_owned())
            .unwrap_or_default(),
        opt_text(row.disposition.as_deref()),
        boolean(row.delivered),
        opt_text(row.header_from.as_deref()),
        opt_text(row.envelope_from.as_deref()),
        list(&row.spf_domains),
        list(&row.dkim_domains),
        list(&row.dkim_selectors),
        opt_text(row.reporter_spf_verdict.as_deref()),
        opt_text(row.reporter_dkim_verdict.as_deref()),
        list(&row.policy_overrides),
    ]
}

/// `GET /api/export/auth-issues.csv?organization_id&domain_id&since&outcome&source_ip`
/// — every record the Failures tab counts (SPF or DKIM lacks an aligned pass),
/// with the tab's filters. `outcome=all_failures|delivered|forwarded|blocked|dkim_only|spf_only`,
/// or several separated by commas, as on the tab; the `outcome` column reads `delivered|forwarded|blocked|passed`.
pub async fn export_auth_issues_csv(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ExportScopeQuery>,
    Query(filters): Query<FailureFilterQuery>,
) -> AppResult<Response> {
    let scope = scope.scope(filters.since_ts()?);
    filters.filter()?;
    let pool = state.pool.clone();
    Ok(csv_download(
        "auth-issues",
        AUTH_ISSUES_HEADER,
        move |mut sink| async move {
            let Ok(filter) = filters.filter() else {
                return;
            };
            let result = stream_auth_issue_export(&pool, scope, &filter, |row| {
                deliver(sink.push(&auth_issue_fields(&row)))
            })
            .await;
            sink.finish(result).await;
        },
    ))
}

/// `GET /api/export/dns-findings.csv?organization_id&domain_id` — one row per
/// finding of each domain's stored DNS snapshot (no live DNS; never-checked
/// domains have no rows).
pub async fn export_dns_findings_csv(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ExportScopeQuery>,
) -> AppResult<Response> {
    let pool = state.pool.clone();
    Ok(csv_download(
        "dns-findings",
        DNS_FINDINGS_HEADER,
        move |mut sink| async move {
            let result = stream_dns_snapshot_export(&pool, scope.scope(None), |snap| {
            let mut buf = Vec::new();
            match serde_json::from_str::<DnsReport>(&snap.report_json) {
                Ok(report) => {
                    for f in &report.findings {
                        push_record(
                            &mut buf,
                            &[
                                snap.organization.as_str(),
                                &snap.domain,
                                &iso(snap.checked_at),
                                &snap.grade,
                                &snap.score.to_string(),
                                f.check.as_str(),
                                f.severity.as_str(),
                                &f.code,
                                &f.points.to_string(),
                                &f.message,
                                &f.suggestion,
                            ],
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(domain = %snap.domain, error = %e, "stored dns snapshot is not valid JSON; skipped in export");
                }
            }
            deliver(sink.extend(&buf))
        })
        .await;
            sink.finish(result).await;
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line<S: AsRef<str>>(fields: &[S]) -> String {
        let mut buf = Vec::new();
        push_record(&mut buf, fields);
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(line(&["a", "b c", ""]), "a,b c,\r\n");
        assert_eq!(
            line(&["x,y", "say \"hi\"", "two\nlines"]),
            "\"x,y\",\"say \"\"hi\"\"\",\"two\nlines\"\r\n"
        );
    }

    #[test]
    fn formula_cells_are_neutralized() {
        assert_eq!(
            line(&["=1+1", "+cmd", "-2", "@SUM(A1)", "a=b", "1.5"]),
            "'=1+1,'+cmd,'-2,'@SUM(A1),a=b,1.5\r\n"
        );
        // Guarded first, then quoted: the quote char stays inside the field.
        assert_eq!(
            line(&["=HYPERLINK(\"x\",\"y\")"]),
            "\"'=HYPERLINK(\"\"x\"\",\"\"y\"\")\"\r\n"
        );
    }

    #[test]
    fn tab_and_carriage_return_prefixes_are_neutralized() {
        assert_eq!(line(&["\t=1+1"]), "'\t=1+1\r\n");
        assert_eq!(line(&["\r=1+1"]), "\"'\r=1+1\"\r\n");
    }

    #[test]
    fn every_list_item_is_guarded() {
        let items = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            list(&items(&["ok", "=HYPERLINK(\"http://x\")"])),
            "ok;'=HYPERLINK(\"http://x\")"
        );
        assert_eq!(list(&items(&["+a", "b", "@c", "-d"])), "'+a;b;'@c;'-d");
        // The cell guard does not add a second quote to a guarded first item.
        assert_eq!(line(&[list(&items(&["=x", "y"]))]), "'=x;y\r\n");
        assert_eq!(list(&[]), "");
    }

    #[test]
    fn timestamps_are_iso_utc() {
        assert_eq!(iso(1_790_870_437), "2026-10-01T16:00:37Z");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(opt_iso(None), "");
    }

    #[test]
    fn percentages_and_booleans() {
        assert_eq!(rate(&Rate::of(1, 3)), "33.33");
        assert_eq!(rate(&Rate::of(0, 0)), "");
        assert_eq!(boolean(true), "true");
        assert_eq!(boolean(false), "false");
    }
}
