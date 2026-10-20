//! Per-domain deep-dive endpoints: overview and failures. A domain's senders are
//! `GET /api/senders?domain_id=…`.

use super::identity::{with_identities, WithIdentity};
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path, Query};
use crate::filters::{parse_pagination, parse_sort, Page};
use crate::since::parse_since;
use crate::view::domain::{domain_page, DomainFailureRow, DomainOverview, Judging};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::readiness_window_start;
use dmarc_store::FailureOutcome;
use dmarc_store::{
    domain_exists, get_domain_overview, list_domain_failures, readiness_mail, unix_now,
    AppSettings, DomainFailureFilter, DomainFailureQuery, FailureSort,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct DomainSinceQuery {
    pub since: Option<String>,
}

/// Filters of the Failures tab; its CSV export takes the same ones. `since`
/// is the time window, the rest narrows the records.
#[derive(Debug, Deserialize)]
pub struct FailureFilterQuery {
    pub since: Option<String>,
    /// `all_failures` (delivered + forwarded + blocked), `delivered`, `forwarded`, `blocked`,
    /// `dkim_only` / `spf_only` (DMARC passed via that check only), or several of
    /// them separated by commas (a record matching any). Absent: all.
    pub outcome: Option<String>,
    pub source_ip: Option<String>,
}

impl FailureFilterQuery {
    /// The time window's lower bound.
    pub fn since_ts(&self) -> AppResult<Option<i64>> {
        parse_since(self.since.as_deref())
    }

    pub fn filter(&self) -> AppResult<DomainFailureFilter<'_>> {
        Ok(DomainFailureFilter {
            outcomes: parse_outcome(self.outcome.as_deref())?,
            source_ip: self
                .source_ip
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        })
    }
}

/// Paging and order of the Failures tab; its filters are [`FailureFilterQuery`].
#[derive(Debug, Deserialize)]
pub struct DomainFailuresPageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// `count`, `source_ip`, `outcome`, `org_name`, … ; `-` prefix for descending.
    pub sort: Option<String>,
}

/// `outcome`: one kind or a comma-separated set of them; absent or empty is all.
fn parse_outcome(raw: Option<&str>) -> AppResult<Vec<FailureOutcome>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|value| {
            FailureOutcome::parse(value).ok_or_else(|| {
                let expected: Vec<&str> = FailureOutcome::ALL.iter().map(|o| o.as_str()).collect();
                AppError::bad_request(format!(
                    "invalid outcome '{value}': expected {}, or several separated by commas",
                    expected.join(", ")
                ))
            })
        })
        .collect()
}

/// `GET /api/domains/{id}` — overview for one domain. Policy readiness is
/// judged on its own fixed window, not on `since`.
pub async fn get_domain(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(q): Query<DomainSinceQuery>,
) -> AppResult<Json<DomainOverview>> {
    let since = parse_since(q.since.as_deref())?;
    let overview = get_domain_overview(&state.pool, id, since)
        .await?
        .ok_or_else(|| AppError::not_found(format!("domain {id} not found")))?;
    let now = unix_now();
    let readiness_since = readiness_window_start(now);
    let policy = overview.policy.as_ref().and_then(|p| p.p.as_deref());
    let readiness = readiness_mail(&state.pool, id, policy, readiness_since).await?;
    let settings = AppSettings::load(&state.pool).await?;
    let at = Judging::new(now, &settings.monitoring);
    let report_address = settings.reporting.report_address.unwrap_or_default();
    let mut overview = domain_page(overview, &readiness, at, &report_address);
    let check_failed = state.dns.failing(&overview.row.domain);
    overview.note_failed_policy_check(check_failed);
    Ok(Json(overview))
}

/// `GET /api/domains/{id}/failures` — records lacking an aligned SPF or DKIM pass, most messages first.
pub async fn get_domain_failures(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(filters): Query<FailureFilterQuery>,
    Query(q): Query<DomainFailuresPageQuery>,
) -> AppResult<Json<Page<WithIdentity<DomainFailureRow>>>> {
    if !domain_exists(&state.pool, id).await? {
        return Err(AppError::not_found(format!("domain {id} not found")));
    }
    let (limit, offset) = parse_pagination(q.limit, q.offset)?;
    let sort = parse_sort(q.sort.as_deref(), FailureSort::DEFAULT)?;
    let since = filters.since_ts()?;
    let query = DomainFailureQuery {
        domain_id: id,
        since,
        filter: filters.filter()?,
        sort,
        limit,
        offset,
    };
    let (items, total) = list_domain_failures(&state.pool, &query).await?;
    let items: Vec<DomainFailureRow> = items.into_iter().map(Into::into).collect();
    let items = with_identities(&state, items, |row| &row.row.source_ip).await?;
    Ok(Json(Page::new(items, total, limit, offset)))
}
