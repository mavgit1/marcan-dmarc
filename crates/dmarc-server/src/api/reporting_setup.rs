//! `GET /api/reporting-setup`: whether each monitored domain's DMARC reports
//! reach the mailbox. The live lookups happen here (the domain's DMARC record,
//! the mailbox domain's authorization records); `dmarc_assess` decides each
//! domain's [`dmarc_assess::ReportingStatus`] and the mailbox domain's
//! authorization, and the UI only words them.
//!
//! A domain whose reports arrive is not looked up (its record is shown on
//! request: `domain_id` asks for that one domain, always looked up).

use crate::error::{AppError, AppResult};
use crate::extract::{Json, Query};
use crate::view::domain::{judged_summary, Judging};
use crate::view::reporting_setup::{
    auth_state, reporting_domain, sort_domains, ReportingDomainInput, ReportingSetupResponse,
};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::{mailbox_authorization, AuthState, ReportingState};
use dmarc_domain::{normalize_name, report_authorization_needed};
use dmarc_store::{
    get_domain_summary, list_domain_summaries, unix_now, AppSettings, ReportingMode, Scope,
};
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

/// A made-up domain: only the wildcard `*._report._dmarc.<mailbox domain>`
/// answers for it, so its lookup says whether the wildcard is published.
const WILDCARD_PROBE: &str = "marcan-check.invalid";

/// Domains looked up at once.
const CONCURRENT_LOOKUPS: usize = 8;

#[derive(Debug, Deserialize)]
pub struct ReportingSetupQuery {
    /// One domain only (its record looked up even while its reports arrive);
    /// the mailbox domain's authorization is then left out.
    pub domain_id: Option<i64>,
    /// When true, forget cached DNS answers first (Re-check).
    pub refresh: Option<bool>,
}

pub async fn get_reporting_setup(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ReportingSetupQuery>,
) -> AppResult<Json<ReportingSetupResponse>> {
    let settings = AppSettings::load(&state.pool).await?;
    let report_address = settings
        .reporting
        .report_address
        .clone()
        .unwrap_or_default();
    let mode = settings.reporting.reporting_mode.unwrap_or_default();
    let mailbox_domain = report_address
        .rsplit_once('@')
        .map(|(_, domain)| normalize_name(domain))
        .unwrap_or_default();
    let at = Judging::new(unix_now(), &settings.monitoring);
    let rows = match q.domain_id {
        Some(id) => vec![get_domain_summary(&state.pool, id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("domain {id} not found")))?],
        None => list_domain_summaries(&state.pool, Scope::ALL).await?,
    };
    let mut response = ReportingSetupResponse {
        report_address: report_address.clone(),
        mailbox_domain: mailbox_domain.clone(),
        reporting_mode: mode,
        mailbox_authorization: None,
        domains: Vec::new(),
    };
    if mailbox_domain.is_empty() {
        return Ok(Json(response));
    }

    let domains: Vec<_> = rows.into_iter().map(|r| judged_summary(r, at)).collect();
    let direct = mode == ReportingMode::Direct;
    let one = q.domain_id.is_some();
    let refresh = q.refresh.unwrap_or(false);
    let needs_auth = |domain: &str| direct && report_authorization_needed(domain, &mailbox_domain);
    let arriving = |reporting: ReportingState| reporting == ReportingState::Reporting;

    let probe_wildcard =
        !one && direct && (domains.is_empty() || domains.iter().any(|d| needs_auth(&d.row.domain)));
    let any_lookup = probe_wildcard
        || one
        || domains
            .iter()
            .any(|d| !arriving(d.assessment.reporting.state));
    // Nothing to look up (every domain's reports arrive, none needs the
    // wildcard): no DNS checker is needed.
    let engine = if any_lookup {
        Some(state.dns.checker(&state.pool).await?)
    } else {
        None
    };
    if let Some(engine) = engine.as_ref().filter(|_| refresh) {
        engine.clear_resolver_cache();
    }
    // Every lookup at once: a domain's own authorization record answers
    // through the wildcard too, so it needs no wait for the wildcard probe,
    // which only says whether the wildcard covers domains added later.
    let wildcard = async {
        match engine.as_ref().filter(|_| probe_wildcard) {
            Some(engine) => {
                let auth = engine
                    .check_report_authorization(WILDCARD_PROBE, &mailbox_domain)
                    .await;
                Some(auth_state(auth.authorized))
            }
            None => None,
        }
    };
    let rows = stream::iter(domains)
        .map(|d| {
            let engine = engine.clone();
            let mailbox_domain = mailbox_domain.clone();
            let reporting = d.assessment.reporting.state;
            let auth_needed = needs_auth(&d.row.domain);
            async move {
                let domain = d.row.domain;
                let lookup = one || !arriving(reporting);
                let engine = engine.filter(|_| lookup);
                let dmarc = async {
                    let engine = engine.as_ref()?;
                    if refresh {
                        engine.invalidate_dmarc(&domain).await;
                    }
                    Some(engine.check_dmarc(&domain).await)
                };
                let mailbox_auth = async {
                    let engine = engine.as_ref().filter(|_| auth_needed)?;
                    let auth = engine
                        .check_report_authorization(&domain, &mailbox_domain)
                        .await;
                    Some(auth_state(auth.authorized))
                };
                let (dmarc, mailbox_auth) = tokio::join!(dmarc, mailbox_auth);
                ReportingDomainInput {
                    id: d.row.id,
                    domain,
                    last_report_at: d.row.last_report_at,
                    reporting,
                    dmarc,
                    mailbox_auth,
                }
            }
        })
        .buffer_unordered(CONCURRENT_LOOKUPS)
        .collect::<Vec<ReportingDomainInput>>();
    let (wildcard, inputs) = tokio::join!(wildcard, rows);

    if !one && direct {
        // Each domain outside the mailbox's organizational domain: accepted by
        // its own record, or shown to be by its arriving reports.
        let external: Vec<AuthState> = inputs
            .iter()
            .filter(|i| needs_auth(&i.domain))
            .map(|i| {
                if arriving(i.reporting) {
                    AuthState::Authorized
                } else {
                    i.mailbox_auth.unwrap_or(AuthState::CheckFailed)
                }
            })
            .collect();
        response.mailbox_authorization = Some(mailbox_authorization(wildcard, &external).into());
    }
    response.domains = inputs
        .into_iter()
        .map(|i| reporting_domain(i, mode, &report_address))
        .collect();
    sort_domains(&mut response.domains);
    Ok(Json(response))
}
