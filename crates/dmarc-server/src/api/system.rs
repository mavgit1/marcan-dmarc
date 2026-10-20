//! `GET /api/system/health` — system health and pipeline observability.

use crate::error::AppResult;
use crate::extract::Json;
use crate::imap_backend::stored_imap_config;
use crate::view::system::{DnsCheckProgress, ImapPhase, ImapProgressResponse};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::{last_poll, pipeline_status, LastPoll, Pipeline, PipelineFacts};
use dmarc_store::unix_now;
use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SystemHealthResponse {
    pub last_ingest_at: Option<i64>,
    pub hours_since_ingest: Option<f64>,
    pub last_check_at: Option<i64>,
    pub seconds_since_check: Option<i64>,
    /// Last IMAP pass that completed without error.
    pub imap_last_success_at: Option<i64>,
    /// Error from the last IMAP pass; `None` once a later pass succeeds.
    pub imap_last_error: Option<String>,
    pub imap_last_error_at: Option<i64>,
    /// Host, username and password are set, so the mailbox is polled.
    pub imap_configured: bool,
    /// Seconds between scheduled mailbox polls.
    pub imap_poll_interval_secs: u64,
    /// Hours without a new report before the pipeline status warns.
    pub report_freshness_warning_hours: u32,
    pub quarantine_count: i64,
    pub database_size_bytes: i64,
    pub total_messages: i64,
    pub total_reports: i64,
    /// The mailbox pass running right now; `None` while idle.
    pub imap_progress: Option<ImapProgressResponse>,
    /// The background DNS checks of the monitored domains.
    pub dns_checks: DnsCheckProgress,
    /// The most recent mailbox poll, successful or not, and whether it failed.
    pub last_poll: LastPoll,
    /// Is the mailbox polled, do reports arrive: the pipeline's verdict.
    pub pipeline: Pipeline,
}

/// `GET /healthz`: `ok` while the server answers. For uptime probes, which
/// need no login; `/api/system/health` carries details and stays behind it.
pub async fn healthz() -> &'static str {
    "ok"
}

pub async fn get_system_health(
    State(state): State<Arc<AppState>>,
) -> AppResult<Json<SystemHealthResponse>> {
    // Read progress before the stored outcome: a pass records its outcome and only
    // then clears its progress, so "no progress" here guarantees the outcome below
    // is that pass's, never a half-written one.
    let imap_progress: Option<ImapProgressResponse> = state.imap.progress().map(Into::into);

    let pool = &state.pool;
    let now = unix_now();
    let last_ingest_at = dmarc_store::last_ingest_at(pool).await?;
    let hours_since_ingest = last_ingest_at.map(|ts| (now - ts).max(0) as f64 / 3600.0);

    // Polled every few seconds: every setting in one query, the pass record
    // in three small ones.
    let settings = dmarc_store::AppSettings::load(pool).await?;
    let passes = dmarc_store::imap_pass_status(pool).await?;
    let last_check_at = passes.last_check_at;
    let imap_last_success_at = passes.last_success_at;
    let (imap_last_error, imap_last_error_at) = passes.last_error.unzip();
    let imap = stored_imap_config(&settings.mailbox);

    // Polled every few seconds: two covered aggregates, not the analytics query.
    let totals = dmarc_store::report_totals(pool).await?;
    let dns_checks = DnsCheckProgress::new(
        dmarc_store::dns_coverage(pool).await?,
        state.dns.warm_pending(),
    );

    let imap_configured = imap.is_complete();
    let report_freshness_warning_hours = settings.monitoring.report_freshness_warning_hours;
    let seconds_since_check = last_check_at.map(|ts| (now - ts).max(0));
    let pipeline = pipeline_status(&PipelineFacts {
        syncing: imap_progress.is_some(),
        importing: imap_progress
            .as_ref()
            .is_some_and(|p| p.phase == ImapPhase::Importing),
        imap_configured,
        imap_error: imap_last_error.is_some(),
        seconds_since_check,
        poll_interval_secs: imap.poll_interval_secs,
        hours_since_ingest,
        report_freshness_warning_hours,
    });

    let last_poll = last_poll(
        imap_progress.is_some(),
        last_check_at,
        imap_last_error.as_ref().and(imap_last_error_at),
    );

    Ok(Json(SystemHealthResponse {
        last_ingest_at,
        hours_since_ingest,
        last_check_at,
        seconds_since_check,
        imap_last_success_at,
        imap_last_error,
        imap_last_error_at,
        imap_configured,
        imap_poll_interval_secs: imap.poll_interval_secs,
        report_freshness_warning_hours,
        quarantine_count: dmarc_store::count_imap_quarantine(pool, None).await?,
        database_size_bytes: dmarc_store::database_size_bytes(pool).await?,
        total_messages: totals.total_messages,
        total_reports: totals.report_count,
        imap_progress,
        dns_checks,
        last_poll,
        pipeline,
    }))
}
