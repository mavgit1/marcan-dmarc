//! Live DNS validation endpoints. Outbound lookups never become HTTP 500.
//!
//! `GET /api/dns/check` serves a persisted snapshot when one exists so the UI
//! is not gated on the public internet. A snapshot older than an hour is still
//! returned immediately, and a live check is spawned to refresh it.
//!
//! A domain's DNS result is in one of four states ([`DnsStatus`]):
//! - `ok`: the shown result is complete and the latest attempt succeeded.
//! - `updating`: the shown result comes from an older checker version
//!   ([`dns_outdated`]); a re-check is running, and the warm loop re-checks
//!   such domains before any other.
//! - `stale`: the latest attempt failed or was incomplete (a lookup timed out,
//!   SERVFAILed, or the SPF include walk did not finish), so the last complete
//!   result is shown and a background retry is scheduled with backoff.
//! - `failed`: no complete result exists; every check reads "Check failed".
//!
//! Only complete results are ever stored or cached; a partial one is never
//! shown as the domain's state.

use crate::error::{AppError, AppResult};
use crate::extract::{Json, Query};
use crate::snapshots::dns::{
    configured_selectors, dkim_probe, dns_outdated, load_complete_snapshot, persist_report,
    Attempts, DnsService, FRESH_SECS,
};
use crate::AppState;
use axum::extract::State;
use chrono::{DateTime, TimeZone, Utc};
use dmarc_assess::{
    critical_count, dns_grade_status, dns_status, worst_check, CheckKind, DnsStatus, Status,
};
use dmarc_dns::dkim::DkimProbe;
use dmarc_dns::{parse_ip, DnsReport, IpError, PtrLookup, CHECKER_VERSION};
use dmarc_domain::{normalize_domain, DomainError};
use dmarc_store::{dns_snapshot_is_fresh, unix_now};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Deserialize)]
pub struct DnsQuery {
    pub domain: String,
    pub selectors: Option<String>,
    /// When true, skip the snapshot and run a live check.
    pub refresh: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PtrQuery {
    pub ip: String,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct DnsCheckResponse {
    /// The shown result; its `checked_at` is when that result was measured.
    #[serde(flatten)]
    pub report: DnsReport,
    /// Verdict on `grade` (a critical finding fails any letter); `neutral`
    /// when no complete measurement backs the grade.
    pub grade_status: Status,
    /// A complete measurement backs `score` and `grade`; when false the UI
    /// shows no letter (the check failed or DMARC/SPF were not measured).
    pub graded: bool,
    /// The check of the worst finding: the drawer to open first.
    pub worst_check: CheckKind,
    pub cached: bool,
    pub status: DnsStatus,
    /// When this process last tried to check the domain (successful or not);
    /// `null` when it has not tried since it started.
    pub last_attempt_at: Option<DateTime<Utc>>,
}

/// `raw` as a normalized domain, or a `400` saying why it is not one.
pub(crate) fn parse_domain(raw: &str) -> AppResult<String> {
    normalize_domain(raw).map_err(|e| match e {
        DomainError::Empty => AppError::bad_request("domain must not be empty"),
        DomainError::TooLong => AppError::bad_request("domain is too long"),
        DomainError::Invalid => {
            AppError::bad_request(format!("domain is not a valid hostname: {raw}"))
        }
    })
}

fn parse_selectors(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn parse_ip_param(raw: &str) -> AppResult<std::net::IpAddr> {
    parse_ip(raw).map_err(|e| match e {
        IpError::Empty => AppError::bad_request("ip must not be empty"),
        IpError::Invalid => {
            AppError::bad_request(format!("ip is not a valid IPv4 or IPv6 address: {raw}"))
        }
    })
}

fn timestamp(secs: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(secs, 0).single()
}

/// The complete result a response shows, if any, and where it came from.
struct Shown {
    report: Option<DnsReport>,
    /// Checked by an older checker version ([`dns_outdated`]).
    outdated: bool,
    /// The answer's `cached`.
    cached: bool,
}

/// The response for a domain given the result to show and its attempt
/// record: `ok` when the latest attempt succeeded, `stale` when it failed but
/// an older complete result exists, `failed` when none does.
fn respond(
    shown: Shown,
    attempts: Option<Attempts>,
    domain: &str,
    probe: &DkimProbe,
) -> DnsCheckResponse {
    let Shown {
        report: shown,
        outdated,
        cached,
    } = shown;
    let last_attempt_at = attempts.and_then(|a| timestamp(a.last_attempt_at));
    let failing = attempts.is_some_and(|a| a.failures > 0);
    let status = dns_status(shown.is_some(), failing, outdated);
    match shown {
        Some(report) => DnsCheckResponse {
            grade_status: grade_verdict(&report),
            graded: report.core_measured,
            worst_check: worst_check(&report.findings),
            report,
            cached,
            status,
            last_attempt_at,
        },
        None => {
            let mut report = DnsReport::check_failed(domain, probe);
            if let Some(at) = last_attempt_at {
                report.checked_at = at;
            }
            DnsCheckResponse {
                worst_check: worst_check(&report.findings),
                report,
                grade_status: Status::Neutral,
                graded: false,
                cached,
                status,
                last_attempt_at,
            }
        }
    }
}

/// [`dns_grade_status`] of a shown result.
fn grade_verdict(report: &DnsReport) -> Status {
    dns_grade_status(
        report.core_measured,
        &report.grade,
        critical_count(&report.findings),
    )
}

/// Record a finished attempt, persist it when complete, and pick what to show:
/// the new result when complete, else the last complete one (stale), else
/// nothing (failed). Never the partial result.
async fn conclude_attempt(
    dns: &DnsService,
    pool: &SqlitePool,
    domain: &str,
    report: DnsReport,
    prior: Option<DnsReport>,
    probe: &DkimProbe,
) -> DnsCheckResponse {
    let complete = report.is_complete();
    let attempts = dns.record_attempt(domain, complete, unix_now());
    if complete {
        if let Err(e) = persist_report(pool, &report).await {
            tracing::warn!(error = %e, domain = %domain, "failed to persist dns snapshot");
        }
        let shown = Shown {
            report: Some(report),
            outdated: false,
            cached: false,
        };
        return respond(shown, Some(attempts), domain, probe);
    }
    // Another refresh may have stored a complete result meanwhile.
    let kept = match prior {
        Some(r) => Some(r),
        None => load_complete_snapshot(pool, domain)
            .await
            .ok()
            .flatten()
            .map(|(_, r)| r),
    };
    // The attempt failed, so the kept result shows as stale whatever its version.
    tracing::warn!(
        domain = %domain,
        failures = attempts.failures,
        kept_previous = kept.is_some(),
        next_retry_in_secs = attempts.next_retry_at() - attempts.last_attempt_at,
        "dns check incomplete; not stored"
    );
    let shown = Shown {
        report: kept,
        outdated: false,
        cached: true,
    };
    respond(shown, Some(attempts), domain, probe)
}

pub async fn check_all(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<DnsCheckResponse>> {
    let domain = parse_domain(&q.domain)?;
    let refresh = q.refresh.unwrap_or(false);

    let prior = load_complete_snapshot(&state.pool, &domain).await?;

    if !refresh {
        if let Some((snap, report)) = prior {
            // A changed selector list makes the stored DKIM verdict stale
            // even inside the freshness window: serve it, but re-check.
            let configured = DkimProbe::requested(&configured_selectors(&state.pool).await);
            let selectors_changed = report.dkim.configured_selectors != configured.configured;
            // A stale result (latest attempt failed) is retried too, with backoff.
            let failing = state.dns.failing(&domain);
            if selectors_changed
                || failing
                || !dns_snapshot_is_fresh(&snap, unix_now(), FRESH_SECS, CHECKER_VERSION)
            {
                state.dns.spawn_refresh(state.pool.clone(), domain.clone());
            }
            let probe = DkimProbe::default();
            let shown = Shown {
                report: Some(report),
                outdated: dns_outdated(snap.checker_version),
                cached: true,
            };
            return Ok(Json(respond(
                shown,
                state.dns.attempts_for(&domain),
                &domain,
                &probe,
            )));
        }
        // No complete result yet. A domain that just failed is retried in the
        // background with backoff, not on every page load.
        if let Some(a) = state.dns.attempts_for(&domain) {
            if a.failures > 0 && !a.retry_due(unix_now()) {
                let probe = dkim_probe(&state.pool, &domain).await;
                let shown = Shown {
                    report: None,
                    outdated: false,
                    cached: true,
                };
                return Ok(Json(respond(shown, Some(a), &domain, &probe)));
            }
        }
    }

    // Manual re-check (or first check): run now, with the interactive budget.
    let engine = state.dns.checker(&state.pool).await?;
    if refresh {
        engine.invalidate_domain(&domain).await;
    }
    let probe = dkim_probe(&state.pool, &domain).await;
    let report = engine.check_all(&domain, &probe).await;
    let prior = prior.map(|(_, r)| r);
    Ok(Json(
        conclude_attempt(&state.dns, &state.pool, &domain, report, prior, &probe).await,
    ))
}

/// `refresh=true` forgets cached answers for the domain first (this process's
/// result cache and the resolver's, negative answers included), so a record
/// the user just published shows up at once.
pub async fn check_dmarc(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::dmarc::DmarcCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let engine = state.dns.checker(&state.pool).await?;
    if q.refresh.unwrap_or(false) {
        engine.invalidate_dmarc(&domain).await;
    }
    Ok(Json(engine.check_dmarc(&domain).await))
}

pub async fn check_spf(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::spf::SpfCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.check_spf(&domain).await))
}

/// Explicit `selectors=` pins the list (the standalone inspector, which must
/// work for unmonitored domains), so the caller sees a verdict on each name
/// they asked about. Otherwise probe what the reports have observed plus the
/// configured selector list, exactly as the domain grade does.
pub async fn check_dkim(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::dkim::DkimCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let requested = parse_selectors(q.selectors.as_deref());
    let probe = if requested.is_empty() {
        dkim_probe(&state.pool, &domain).await
    } else {
        DkimProbe::requested(&requested)
    };
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.check_dkim(&domain, &probe).await))
}

pub async fn check_mta_sts(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::policy::MtaStsCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.check_mta_sts(&domain).await))
}

pub async fn check_tls_rpt(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::policy::TlsRptCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.check_tls_rpt(&domain).await))
}

pub async fn check_bimi(
    State(state): State<Arc<AppState>>,
    Query(q): Query<DnsQuery>,
) -> AppResult<Json<dmarc_dns::policy::BimiCheckResult>> {
    let domain = parse_domain(&q.domain)?;
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.check_bimi(&domain).await))
}

pub async fn check_ptr(
    State(state): State<Arc<AppState>>,
    Query(q): Query<PtrQuery>,
) -> AppResult<Json<PtrLookup>> {
    let ip = parse_ip_param(&q.ip)?;
    let engine = state.dns.checker(&state.pool).await?;
    Ok(Json(engine.lookup_ptr(ip).await))
}

#[cfg(test)]
mod persist_tests {
    use super::conclude_attempt;
    use crate::snapshots::dns::{
        persist_report, Attempts, DnsService, RETRY_BASE_SECS, RETRY_MAX_SECS,
    };
    use dmarc_assess::score_from_findings;
    use dmarc_assess::DnsStatus;
    use dmarc_dns::dkim::{evaluate_dkim, DkimProbe};
    use dmarc_dns::dmarc::evaluate_dmarc;
    use dmarc_dns::policy::{evaluate_bimi, evaluate_mta_sts, evaluate_tls_rpt, PolicyFetch};
    use dmarc_dns::spf::evaluate_spf;
    use dmarc_dns::{assemble_report, LookupStatus, TxtAnswer, TxtLookup};
    use dmarc_store::{get_dns_snapshot, init};

    fn pool_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("test.db")
    }

    fn unmeasured(domain: &str) -> dmarc_dns::DnsReport {
        let timeout = TxtLookup::Timeout;
        assemble_report(
            domain,
            evaluate_dmarc(domain, &timeout, &[]),
            evaluate_spf(domain, &timeout, &[]),
            evaluate_dkim(domain, &[], &DkimProbe::default()),
            evaluate_mta_sts(domain, &timeout, None, &PolicyFetch::NotAttempted),
            evaluate_tls_rpt(domain, &timeout),
            evaluate_bimi(domain, &timeout),
        )
    }

    fn measured(domain: &str) -> dmarc_dns::DnsReport {
        let dmarc = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1; p=reject".into()]));
        let spf = TxtLookup::Records(TxtAnswer::from_records(vec!["v=spf1 -all".into()]));
        assemble_report(
            domain,
            evaluate_dmarc(domain, &dmarc, &[]),
            evaluate_spf(domain, &spf, &[]),
            evaluate_dkim(domain, &[], &DkimProbe::default()),
            evaluate_mta_sts(
                domain,
                &TxtLookup::NoRecords,
                None,
                &PolicyFetch::NotAttempted,
            ),
            evaluate_tls_rpt(domain, &TxtLookup::NoRecords),
            evaluate_bimi(domain, &TxtLookup::NoRecords),
        )
    }

    /// DMARC and the root SPF record answered, but an include never did.
    fn incomplete_walk(domain: &str) -> dmarc_dns::DnsReport {
        let dmarc = TxtLookup::Records(TxtAnswer::from_records(vec!["v=DMARC1; p=reject".into()]));
        let spf = TxtLookup::Records(TxtAnswer::from_records(vec![
            "v=spf1 include:slow.example -all".into(),
        ]));
        assemble_report(
            domain,
            evaluate_dmarc(domain, &dmarc, &[]),
            evaluate_spf(domain, &spf, &[("slow.example".into(), TxtLookup::Timeout)]),
            evaluate_dkim(domain, &[], &DkimProbe::default()),
            evaluate_mta_sts(
                domain,
                &TxtLookup::NoRecords,
                None,
                &PolicyFetch::NotAttempted,
            ),
            evaluate_tls_rpt(domain, &TxtLookup::NoRecords),
            evaluate_bimi(domain, &TxtLookup::NoRecords),
        )
    }

    #[tokio::test]
    async fn incomplete_spf_walk_is_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        let domain = "walk.example";
        dmarc_store::create_domain(&pool, 1, domain).await.unwrap();
        let good = measured(domain);
        assert!(persist_report(&pool, &good).await.unwrap());
        let before = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();

        let partial = incomplete_walk(domain);
        assert!(!partial.is_complete());
        assert!(
            !partial.core_measured,
            "an incomplete walk must not be graded"
        );
        assert!(!persist_report(&pool, &partial).await.unwrap());
        let after = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();
        assert_eq!(after.report_json, before.report_json);
    }

    /// A failed re-check keeps showing the last complete result, marked stale;
    /// the next complete one makes it ok again.
    #[tokio::test]
    async fn incomplete_attempt_serves_previous_snapshot_as_stale() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        let domain = "stale-state.example";
        dmarc_store::create_domain(&pool, 1, domain).await.unwrap();
        let probe = DkimProbe::default();
        let dns = DnsService::default();
        let good = measured(domain);
        let first = conclude_attempt(&dns, &pool, domain, good.clone(), None, &probe).await;
        assert_eq!(first.status, DnsStatus::Ok);
        let stored = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();

        let resp = conclude_attempt(
            &dns,
            &pool,
            domain,
            incomplete_walk(domain),
            Some(good.clone()),
            &probe,
        )
        .await;
        assert_eq!(resp.status, DnsStatus::Stale);
        assert!(resp.cached);
        assert_eq!(resp.report.checked_at, good.checked_at);
        assert!(resp.report.is_complete(), "never the partial result");
        assert!(resp.last_attempt_at.is_some());
        let after = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();
        assert_eq!(after.report_json, stored.report_json);

        // Without `prior`, the stored snapshot is found and kept the same way.
        let resp = conclude_attempt(&dns, &pool, domain, unmeasured(domain), None, &probe).await;
        assert_eq!(resp.status, DnsStatus::Stale);
        assert!(resp.report.is_complete());

        let resp = conclude_attempt(&dns, &pool, domain, measured(domain), None, &probe).await;
        assert_eq!(resp.status, DnsStatus::Ok);
    }

    #[tokio::test]
    async fn incomplete_attempt_without_snapshot_is_failed() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        let domain = "never-graded.example";
        let resp = conclude_attempt(
            &DnsService::default(),
            &pool,
            domain,
            incomplete_walk(domain),
            None,
            &DkimProbe::default(),
        )
        .await;
        assert_eq!(resp.status, DnsStatus::Failed);
        assert!(!resp.report.core_measured);
        assert_eq!(resp.report.spf.lookup, LookupStatus::Timeout);
        assert_eq!(resp.report.dmarc.lookup, LookupStatus::Timeout);
        assert!(get_dns_snapshot(&pool, domain).await.unwrap().is_none());
    }

    #[test]
    fn failed_attempts_back_off_and_a_success_resets() {
        let domain = "backoff.example";
        let dns = DnsService::default();
        let t0 = 1_000_000;
        let a = dns.record_attempt(domain, false, t0);
        assert_eq!(a.failures, 1);
        assert!(!a.retry_due(t0 + RETRY_BASE_SECS - 1));
        assert!(a.retry_due(t0 + RETRY_BASE_SECS));
        let b = dns.record_attempt(domain, false, t0 + RETRY_BASE_SECS);
        assert_eq!(b.next_retry_at() - b.last_attempt_at, 2 * RETRY_BASE_SECS);
        let many = Attempts {
            last_attempt_at: t0,
            failures: 30,
        };
        assert_eq!(many.next_retry_at() - t0, RETRY_MAX_SECS);
        let ok = dns.record_attempt(domain, true, t0 + 2 * RETRY_BASE_SECS);
        assert_eq!(ok.failures, 0);
        assert!(ok.retry_due(ok.last_attempt_at));
    }

    #[tokio::test]
    async fn unmeasured_core_does_not_overwrite_existing_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        let domain = "keep.example";
        dmarc_store::create_domain(&pool, 1, domain).await.unwrap();
        let good = measured(domain);
        assert!(good.core_measured);
        assert!(persist_report(&pool, &good).await.unwrap());
        let before = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();

        let wrote = persist_report(&pool, &unmeasured(domain)).await.unwrap();
        assert!(!wrote, "unmeasured core must not persist");

        let after = get_dns_snapshot(&pool, domain).await.unwrap().unwrap();
        assert_eq!(after.grade, before.grade);
        assert_eq!(after.score, before.score);
        assert_eq!(after.checked_at, before.checked_at);
        assert_eq!(after.report_json, before.report_json);
        assert_eq!(after.checker_version, before.checker_version);
    }

    /// The store reads the policy out of the stored report JSON: a domain
    /// without reports shows its DNS record's, and one never checked says so.
    #[tokio::test]
    async fn a_stored_check_fills_in_the_policy_of_a_domain_without_reports() {
        use crate::view::domain::{domain_overview, domain_summaries};
        use dmarc_assess::EnforcementState;
        use dmarc_store::PolicySource;
        use dmarc_store::{get_domain_overview, list_domain_summaries, Scope};

        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        dmarc_store::create_domain(&pool, 1, "checked.example")
            .await
            .unwrap();
        dmarc_store::create_domain(&pool, 1, "pending.example")
            .await
            .unwrap();
        assert!(persist_report(&pool, &measured("checked.example"))
            .await
            .unwrap());

        let rows = list_domain_summaries(&pool, Scope::ALL).await.unwrap();
        let items = domain_summaries(rows, crate::view::domain::Judging::defaults_at(0));
        let checked = items
            .iter()
            .find(|d| d.row.domain == "checked.example")
            .unwrap();
        assert_eq!(checked.row.policy_source, PolicySource::Dns);
        assert_eq!(
            checked.row.policy.as_ref().and_then(|p| p.p.as_deref()),
            Some("reject")
        );
        assert_eq!(
            checked.assessment.enforcement.state,
            EnforcementState::Reject
        );
        let pending = items
            .iter()
            .find(|d| d.row.domain == "pending.example")
            .unwrap();
        assert_eq!(pending.row.policy_source, PolicySource::NotChecked);
        assert_eq!(pending.row.policy, None);

        let row = get_domain_overview(&pool, checked.row.id, None)
            .await
            .unwrap()
            .unwrap();
        let overview = domain_overview(
            row,
            &Default::default(),
            crate::view::domain::Judging::defaults_at(0),
        );
        assert_eq!(overview.row.policy_source, PolicySource::Dns);
        assert_eq!(
            overview.row.policy.and_then(|p| p.p).as_deref(),
            Some("reject")
        );
    }

    #[tokio::test]
    async fn only_monitored_domains_are_stored() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        // Looked up in the DNS inspector, not monitored: answered, not kept.
        assert!(!persist_report(&pool, &measured("visitor.example"))
            .await
            .unwrap());
        assert!(get_dns_snapshot(&pool, "visitor.example")
            .await
            .unwrap()
            .is_none());
        // Monitored: kept. Removed again: not kept.
        let created = dmarc_store::create_domain(&pool, 1, "mine.example")
            .await
            .unwrap();
        assert!(persist_report(&pool, &measured("mine.example"))
            .await
            .unwrap());
        let dmarc_store::CreateDomainResult::Created(row) = created else {
            panic!("a new domain")
        };
        assert!(dmarc_store::ignore_domain(&pool, row.id).await.unwrap());
        assert!(!persist_report(&pool, &measured("mine.example"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn unmeasured_first_check_creates_no_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let pool = init(&pool_path(&dir)).await.unwrap();
        let domain = "fresh.example";
        let wrote = persist_report(&pool, &unmeasured(domain)).await.unwrap();
        assert!(!wrote);
        assert!(get_dns_snapshot(&pool, domain).await.unwrap().is_none());
    }

    #[test]
    fn core_timeout_notices_do_not_change_score_for_identical_dns() {
        let domain = "stable.example";
        let good = measured(domain);
        let timed = unmeasured(domain);
        let mut combined = good.findings.clone();
        combined.extend(
            timed
                .findings
                .iter()
                .filter(|f| {
                    matches!(
                        f.code.as_str(),
                        "dmarc_timeout"
                            | "spf_timeout"
                            | "dmarc_servfail"
                            | "spf_servfail"
                            | "dkim_timeout"
                            | "dkim_servfail"
                    )
                })
                .cloned(),
        );
        assert_eq!(score_from_findings(&combined), good.score);
        assert_eq!(
            score_from_findings(&combined),
            score_from_findings(&good.findings)
        );
    }
}
