//! `GET /api/reporting-setup`: whether each monitored domain's DMARC reports
//! reach a receiving account. The live lookups happen here (the domain's
//! DMARC record, the address domains' authorization records); `dmarc_assess`
//! decides each domain's [`dmarc_assess::ReportingStatus`] and each address
//! domain's authorization, and the UI only words them.
//!
//! The known addresses are the receiving accounts' and the addresses
//! forwarded to them ([`ReportAddresses`]). A domain's record that names one
//! of them carries that address domain's authorization in its own check; a
//! record that names none is also looked up at the default address's domain
//! (what it will need once it names the default address). Each address
//! domain's wildcard is looked up once.
//!
//! A domain whose reports arrive is not looked up (its record is shown on
//! request: `domain_id` asks for that one domain, always looked up).
//!
//! One domain (`domain_id`: its page, a row re-checked on its own) takes its
//! DMARC record from the domain's stored DNS check ([`dns_answer`]), so its
//! waiting card, header and scorecard show one check; `refresh` re-checks
//! and stores it. The list looks the records up live: a full check per
//! domain would be far slower than its one TXT lookup.

use crate::api::dns::dns_answer;
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Query};
use crate::view::domain::{judged_summary, Judging};
use crate::view::reporting_setup::{
    auth_state, named_known, reporting_domain, sort_domains, AddressAuthorization, ReportAddresses,
    ReportingDomainInput, ReportingSetupResponse,
};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::{mailbox_authorization, AuthState, MailboxAuthorization, ReportingState};
use dmarc_domain::report_authorization_needed;
use dmarc_store::{get_domain_summary, list_domain_summaries, unix_now, AppSettings, Scope};
use futures::future::join_all;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use std::sync::Arc;

/// A made-up domain: only the wildcard `*._report._dmarc.<address domain>`
/// answers for it, so its lookup says whether the wildcard is published.
const WILDCARD_PROBE: &str = "marcan-check.invalid";

/// Domains looked up at once.
const CONCURRENT_LOOKUPS: usize = 8;

#[derive(Debug, Deserialize)]
pub struct ReportingSetupQuery {
    /// One domain only (its record looked up even while its reports arrive);
    /// the address domains' authorization is then left out.
    pub domain_id: Option<i64>,
    /// When true, forget cached DNS answers first (Re-check).
    pub refresh: Option<bool>,
}

pub async fn get_reporting_setup(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ReportingSetupQuery>,
) -> AppResult<Json<ReportingSetupResponse>> {
    let settings = AppSettings::load(&state.pool).await?;
    // An account counts once it can read its mailbox: one still missing its
    // server or login is no report address yet.
    let snapshot = state.accounts.snapshot().await?;
    let known = ReportAddresses::of(snapshot.receiving());
    let default_domain = known.default_domain();
    let at = Judging::new(unix_now(), &settings.monitoring);
    let rows = match q.domain_id {
        Some(id) => vec![get_domain_summary(&state.pool, id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("domain {id} not found")))?],
        None => list_domain_summaries(&state.pool, Scope::ALL).await?,
    };
    let mut response = ReportingSetupResponse {
        addresses: known.accounts.clone(),
        default_address: known.default_address().to_owned(),
        authorizations: Vec::new(),
        domains: Vec::new(),
    };
    if default_domain.is_empty() {
        return Ok(Json(response));
    }

    let domains: Vec<_> = rows.into_iter().map(|r| judged_summary(r, at)).collect();
    let one = q.domain_id.is_some();
    let refresh = q.refresh.unwrap_or(false);
    let arriving = |reporting: ReportingState| reporting == ReportingState::Reporting;
    let address_domains = known.domains();

    // Each address domain's wildcard, when a listed domain may need it (or
    // none is listed yet, for the default's: it covers the domains to come).
    let wildcards: Vec<String> = if one {
        Vec::new()
    } else {
        address_domains
            .iter()
            .map(|(d, _)| d.clone())
            .filter(|d| {
                (domains.is_empty() && *d == default_domain)
                    || domains
                        .iter()
                        .any(|s| report_authorization_needed(&s.row.domain, d))
            })
            .collect()
    };
    let any_lookup = !wildcards.is_empty()
        || one
        || domains
            .iter()
            .any(|d| !arriving(d.assessment.reporting.state));
    // Nothing to look up (every domain's reports arrive, no wildcard is
    // needed): no DNS checker is needed.
    let engine = if any_lookup {
        Some(state.dns.checker(&state.pool).await?)
    } else {
        None
    };
    if let Some(engine) = engine.as_ref().filter(|_| refresh) {
        engine.clear_resolver_cache();
    }
    // Every lookup at once: a domain's own authorization record answers
    // through the wildcard too, so it needs no wait for the wildcard probes,
    // which only say whether the wildcard covers domains added later.
    // A handful of address domains at most: all at once.
    let wildcard_states = join_all(wildcards.into_iter().map(|d| {
        let engine = engine.clone();
        async move {
            let engine = engine.as_ref()?;
            let auth = engine.check_report_authorization(WILDCARD_PROBE, &d).await;
            Some((d, auth_state(auth.authorized)))
        }
    }));
    // One domain (its page, or a row looked up on its own) reads its DMARC
    // record from the stored DNS check, the one its scorecard and header
    // show; `refresh` re-checks and stores it.
    let mut stored_dmarc = match (one, domains.first()) {
        (true, Some(d)) => Some(
            dns_answer(&state, &d.row.domain, refresh)
                .await?
                .report
                .dmarc,
        ),
        _ => None,
    };
    let rows = stream::iter(domains)
        .map(|d| {
            let engine = engine.clone();
            let default_domain = default_domain.clone();
            let reporting = d.assessment.reporting.state;
            let auth_needed = report_authorization_needed(&d.row.domain, &default_domain);
            let stored = stored_dmarc.take();
            async move {
                let domain = d.row.domain;
                let lookup = one || !arriving(reporting);
                let engine = engine.filter(|_| lookup);
                let dmarc = async {
                    if stored.is_some() {
                        return stored;
                    }
                    let engine = engine.as_ref()?;
                    if refresh {
                        engine.invalidate_dmarc(&domain).await;
                    }
                    Some(engine.check_dmarc(&domain).await)
                };
                let default_auth = async {
                    let engine = engine.as_ref().filter(|_| auth_needed)?;
                    let auth = engine
                        .check_report_authorization(&domain, &default_domain)
                        .await;
                    Some(auth_state(auth.authorized))
                };
                let (dmarc, default_auth) = tokio::join!(dmarc, default_auth);
                ReportingDomainInput {
                    id: d.row.id,
                    domain,
                    last_report_at: d.row.last_report_at,
                    reporting,
                    dmarc,
                    default_auth,
                }
            }
        })
        .buffer_unordered(CONCURRENT_LOOKUPS)
        .collect::<Vec<ReportingDomainInput>>();
    let (wildcard_states, inputs) = tokio::join!(wildcard_states, rows);

    if !one {
        response.authorizations = address_domains
            .into_iter()
            .map(|(address_domain, addresses)| {
                let is_default = address_domain == default_domain;
                let external =
                    external_authorizations(&inputs, &known, &address_domain, is_default);
                let wildcard = wildcard_states
                    .iter()
                    .flatten()
                    .find(|(d, _)| *d == address_domain)
                    .map(|(_, s)| *s);
                let no_domains = inputs.is_empty() && is_default;
                let state = if external.is_empty() && !no_domains {
                    MailboxAuthorization::NotNeeded
                } else {
                    mailbox_authorization(wildcard, &external)
                };
                AddressAuthorization::new(&address_domain, addresses, state)
            })
            .collect();
    }
    response.domains = inputs
        .into_iter()
        .map(|i| reporting_domain(i, &known))
        .collect();
    sort_domains(&mut response.domains);
    Ok(Json(response))
}

/// How `address_domain` treats each listed domain that sends it reports and
/// is outside its organizational domain: a domain whose record names a known
/// address there (that record's own lookup), and, for the default address's
/// domain, one whose record names no known address yet (the separate
/// lookup) or whose reports arrive (accepted, as they show).
fn external_authorizations(
    inputs: &[ReportingDomainInput],
    known: &ReportAddresses,
    address_domain: &str,
    is_default: bool,
) -> Vec<AuthState> {
    inputs
        .iter()
        .filter(|i| report_authorization_needed(&i.domain, address_domain))
        .filter_map(|i| {
            if i.reporting == ReportingState::Reporting {
                return is_default.then_some(AuthState::Authorized);
            }
            let named = named_known(i.dmarc.as_ref(), known);
            if named.is_empty() {
                return is_default.then(|| i.default_auth.unwrap_or(AuthState::CheckFailed));
            }
            named
                .iter()
                .find(|n| n.domain == address_domain)
                .map(|n| n.auth.unwrap_or(AuthState::CheckFailed))
        })
        .collect()
}
