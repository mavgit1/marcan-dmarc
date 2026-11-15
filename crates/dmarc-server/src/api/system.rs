//! `GET /api/system/health` — system health and pipeline observability.

use crate::accounts::stalest;
use crate::error::AppResult;
use crate::extract::Json;
use crate::view::accounts::AccountHealth;
use crate::view::system::{DnsCheckProgress, PassPhase, PassProgressResponse};
use crate::wording::accounts::locked_sentence;
use crate::AppState;
use axum::extract::State;
use dmarc_assess::{last_poll, pipeline_status, LastPoll, Pipeline, PipelineFacts};
use dmarc_store::{unix_now, ReceivePassStatus};
use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

/// The pipeline's health. The mailbox fields add up every receiving account
/// (`accounts`): the stalest check and success, the first error, the
/// quarantine of all of them, so with one account they are its own.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct SystemHealthResponse {
    pub last_ingest_at: Option<i64>,
    pub hours_since_ingest: Option<f64>,
    /// The stalest receiving account's last check; `null` while one has
    /// none yet.
    pub last_check_at: Option<i64>,
    pub seconds_since_check: Option<i64>,
    /// The stalest receiving account's last pass without error.
    pub receive_last_success_at: Option<i64>,
    /// The first failing receiving account's error (its last pass's, or why
    /// it cannot be read); `None` while none fails.
    pub receive_last_error: Option<String>,
    pub receive_last_error_at: Option<i64>,
    /// Some account receives reports with its server and login set, so it
    /// is polled.
    pub receive_configured: bool,
    /// Seconds between one account's scheduled polls.
    pub receive_poll_interval_secs: u64,
    /// Every receiving account with its server and login set, in id order.
    pub accounts: Vec<AccountHealth>,
    /// Hours without a new report before the pipeline status warns.
    pub report_freshness_warning_hours: u32,
    pub quarantine_count: i64,
    pub database_size_bytes: i64,
    pub total_messages: i64,
    pub total_reports: i64,
    /// The mailbox pass running right now; `None` while idle.
    pub pass_progress: Option<PassProgressResponse>,
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
    // Before the progress: a pass that starts in between shows as running, never
    // as an import between its passes.
    let import_unfinished = state.mailboxes.import_unfinished();
    // Read progress before the stored outcome: a pass records its outcome and only
    // then clears its progress, so "no progress" here guarantees the outcome below
    // is that pass's, never a half-written one.
    let pass_progress: Option<PassProgressResponse> = state.mailboxes.progress().map(Into::into);

    let pool = &state.pool;
    let now = unix_now();
    let last_ingest_at = dmarc_store::last_ingest_at(pool).await?;
    let hours_since_ingest = last_ingest_at.map(|ts| (now - ts).max(0) as f64 / 3600.0);

    // Polled every few seconds: every setting in one query, the accounts in
    // two, and three small ones per receiving account.
    let settings = dmarc_store::AppSettings::load(pool).await?;
    let poll_interval_secs = settings.receiving.poll_interval_secs;
    let report_freshness_warning_hours = settings.monitoring.report_freshness_warning_hours;
    let snapshot = state.accounts.snapshot().await?;
    let running = pass_progress.as_ref().and_then(|p| p.account_id);
    let mut accounts = Vec::new();
    let mut checks = Vec::new();
    let mut successes = Vec::new();
    let mut first_error: Option<(String, Option<i64>)> = None;
    let mut quarantine_count = 0;
    for account in snapshot.receiving() {
        let passes = dmarc_store::receive_pass_status(pool, account.id).await?;
        let lock = snapshot.lock(account.id);
        let ReceivePassStatus {
            last_check_at,
            last_success_at,
            last_error,
            last_error_detail,
        } = passes;
        let (error, error_detail) = match lock {
            Some(reason) => (
                Some((locked_sentence(&account.fields.name, reason), None)),
                None,
            ),
            None => (
                last_error.map(|(text, at)| (text, Some(at))),
                last_error_detail,
            ),
        };
        let pass = pass_progress
            .as_ref()
            .filter(|_| running == Some(account.id));
        let verdict = pipeline_status(&PipelineFacts {
            syncing: pass.is_some(),
            importing: pass.is_some_and(|p| p.phase == PassPhase::Importing),
            // The worker says whether any import is unfinished, not whose: an
            // account's own line is its polling.
            import_unfinished: false,
            receive_configured: true,
            receive_error: error.is_some(),
            seconds_since_check: last_check_at.map(|ts| (now - ts).max(0)),
            poll_interval_secs,
            hours_since_ingest: None,
            report_freshness_warning_hours,
        });
        let quarantined =
            dmarc_store::count_receive_quarantine(pool, Some(account.id), None).await?;
        quarantine_count += quarantined;
        checks.push(last_check_at);
        successes.push(last_success_at);
        if first_error.is_none() {
            first_error.clone_from(&error);
        }
        accounts.push(AccountHealth {
            id: account.id,
            name: account.fields.name.clone(),
            address: account.fields.address.clone(),
            state: verdict.mailbox,
            status: verdict.mailbox_status,
            last_check_at,
            last_success_at,
            last_error: error.map(|(text, _)| text),
            last_error_detail: error_detail,
            quarantine_count: quarantined,
        });
    }
    let last_check_at = stalest(&checks);
    let receive_last_success_at = stalest(&successes);
    let (receive_last_error, receive_last_error_at) = match first_error {
        Some((text, at)) => (Some(text), at),
        None => (None, None),
    };

    // Polled every few seconds: two covered aggregates, not the analytics query.
    let totals = dmarc_store::report_totals(pool).await?;
    let dns_checks = DnsCheckProgress::new(
        dmarc_store::dns_coverage(pool).await?,
        state.dns.warm_pending(),
    );

    let receive_configured = !accounts.is_empty();
    let seconds_since_check = last_check_at.map(|ts| (now - ts).max(0));
    let pipeline = pipeline_status(&PipelineFacts {
        syncing: pass_progress.is_some(),
        importing: pass_progress
            .as_ref()
            .is_some_and(|p| p.phase == PassPhase::Importing),
        import_unfinished,
        receive_configured,
        receive_error: receive_last_error.is_some(),
        seconds_since_check,
        poll_interval_secs,
        hours_since_ingest,
        report_freshness_warning_hours,
    });

    let last_poll = last_poll(
        pass_progress.is_some(),
        last_check_at,
        receive_last_error.as_ref().and(receive_last_error_at),
    );

    Ok(Json(SystemHealthResponse {
        last_ingest_at,
        hours_since_ingest,
        last_check_at,
        seconds_since_check,
        receive_last_success_at,
        receive_last_error,
        receive_last_error_at,
        receive_configured,
        receive_poll_interval_secs: poll_interval_secs,
        accounts,
        report_freshness_warning_hours,
        quarantine_count,
        database_size_bytes: dmarc_store::database_size_bytes(pool).await?,
        total_messages: totals.total_messages,
        total_reports: totals.report_count,
        pass_progress,
        dns_checks,
        last_poll,
        pipeline,
    }))
}
