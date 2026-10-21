//! `GET` / `POST /api/domains` — list and create domains without requiring reports.
//! `PATCH /api/domains/{id}` — move a domain to another organization (`organization_id`).
//! `DELETE /api/domains/{id}` — remove a domain: its reports are deleted and it
//! is ignored, so new reports for it are dropped. `POST` with the same name
//! un-ignores it and re-imports the mailbox for its history.

use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path, Query};
use crate::filters::{parse_pagination, parse_sort, Page, ScopeQuery};
use crate::view::domain::{
    domain_summaries, domain_summary as judged_alone, judged_summary as judged, needing_attention,
    note_failed_checks, DomainSummary, Judging,
};
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use dmarc_store::{
    get_domain_summary, list_domain_summaries, page_domain_summaries, unix_now, CreateDomainResult,
    DomainListQuery, DomainSort, MoveDomain,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct CreateDomainBody {
    pub organization_id: i64,
    pub domain: String,
}

/// `POST /api/domains` answer: the domain, and whether it was an ignored domain
/// unignored (its history comes back with the mailbox re-import this starts)
/// rather than a new one.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct CreatedDomain {
    #[serde(flatten)]
    pub domain: DomainSummary,
    pub unignored: bool,
}

#[derive(Debug, Deserialize)]
pub struct DomainsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Case-insensitive substring of the domain or its organization's name.
    pub q: Option<String>,
    /// `domain`, `organization_name`, `dns_score`, `policy_p`, `total_messages`,
    /// `delivered_pct`, `delivered_failures`, `last_report_at`; `-` prefix for descending.
    pub sort: Option<String>,
}

/// The domain list, filtered, sorted and paged. Organization / domain narrow the
/// rows; `since` narrows the message volumes and reporters (not the report
/// coverage). Rows carry no `attention_rank` (see `GET /api/domains/attention`).
pub async fn list_domains(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ScopeQuery>,
    Query(q): Query<DomainsQuery>,
) -> AppResult<Json<Page<DomainSummary>>> {
    let (limit, offset) = parse_pagination(q.limit, q.offset)?;
    let query = DomainListQuery {
        scope: scope.scope()?,
        search: q.q,
        sort: parse_sort(q.sort.as_deref(), DomainSort::DEFAULT)?,
        limit,
        offset,
    };
    let (rows, total) = page_domain_summaries(&state.pool, &query).await?;
    let at = Judging::load(&state.pool, unix_now()).await?;
    let mut items: Vec<DomainSummary> = rows.into_iter().map(|r| judged(r, at)).collect();
    note_failed_checks(&mut items, &state.dns);
    Ok(Json(Page::new(items, total, limit, offset)))
}

#[derive(Debug, Deserialize)]
pub struct AttentionQuery {
    /// This many (default 50, at most 500) from `offset`; `total` counts
    /// them all.
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Case-insensitive substring of the domain or its organization's name,
    /// as on the domain list.
    pub q: Option<String>,
}

/// `GET /api/domains/attention`: the scope's domains with an open issue, in
/// their attention rank among all of the scope's domains, a page of them
/// plus how many there are: the dashboard's list (the first page) and the
/// Domains page's attention filter (every page, narrowed by `q`). The rank
/// is decided over the whole scope, so it is paged here, not in SQL.
pub async fn list_attention_domains(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ScopeQuery>,
    Query(page): Query<AttentionQuery>,
) -> AppResult<Json<Page<DomainSummary>>> {
    let (limit, offset) = parse_pagination(page.limit, page.offset)?;
    let rows = list_domain_summaries(&state.pool, q.scope()?).await?;
    let mut items = domain_summaries(rows, Judging::load(&state.pool, unix_now()).await?);
    note_failed_checks(&mut items, &state.dns);
    let search = page
        .q
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let open: Vec<&DomainSummary> = needing_attention(&items)
        .into_iter()
        .filter(|d| {
            search.as_deref().is_none_or(|s| {
                d.row.domain.to_lowercase().contains(s)
                    || d.row.organization.name.to_lowercase().contains(s)
            })
        })
        .collect();
    let total = open.len() as i64;
    let rows = open
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .cloned()
        .collect();
    Ok(Json(Page::new(rows, total, limit, offset)))
}

async fn domain_summary(state: &AppState, id: i64) -> AppResult<DomainSummary> {
    let row = get_domain_summary(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("domain {id} not found")))?;
    let mut item = judged_alone(row, Judging::load(&state.pool, unix_now()).await?);
    note_failed_checks(std::slice::from_mut(&mut item), &state.dns);
    Ok(item)
}

/// `201` with `unignored: false` for a new domain; `200` with `unignored: true`
/// when the name belonged to a removed domain, which is monitored again in the
/// requested organization. A visible domain of that name is a `409`.
pub async fn create_domain(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateDomainBody>,
) -> AppResult<(StatusCode, Json<CreatedDomain>)> {
    let domain = super::dns::parse_domain(&body.domain)?;

    match dmarc_store::create_domain(&state.pool, body.organization_id, &domain).await? {
        CreateDomainResult::Created(created) => {
            state.dns.wake();
            Ok((
                StatusCode::CREATED,
                Json(CreatedDomain {
                    domain: domain_summary(&state, created.id).await?,
                    unignored: false,
                }),
            ))
        }
        CreateDomainResult::Unignored(unignored) => {
            state.imap.request_reimport();
            state.dns.wake();
            Ok((
                StatusCode::OK,
                Json(CreatedDomain {
                    domain: domain_summary(&state, unignored.id).await?,
                    unignored: true,
                }),
            ))
        }
        CreateDomainResult::AlreadyExists {
            organization_name, ..
        } => Err(AppError::conflict(format!(
            "domain {domain} already exists in the organization {organization_name}"
        ))
        .with_organization_name(organization_name)),
        CreateDomainResult::OrganizationNotFound => Err(AppError::not_found(format!(
            "organization {} not found",
            body.organization_id
        ))),
    }
}

/// Any subset of the fields; at least one is required.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct UpdateDomainBody {
    pub organization_id: Option<i64>,
    pub parked: Option<bool>,
}

pub async fn update_domain(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateDomainBody>,
) -> AppResult<Json<DomainSummary>> {
    if body.organization_id.is_none() && body.parked.is_none() {
        return Err(AppError::bad_request(
            "nothing to update: send organization_id or parked",
        ));
    }
    if let Some(organization_id) = body.organization_id {
        match dmarc_store::move_domain(&state.pool, id, organization_id).await? {
            MoveDomain::Moved => {}
            MoveDomain::DomainNotFound => {
                return Err(AppError::not_found(format!("domain {id} not found")))
            }
            MoveDomain::OrganizationNotFound => {
                return Err(AppError::not_found(format!(
                    "organization {organization_id} not found"
                )))
            }
        }
    }
    if let Some(parked) = body.parked {
        if !dmarc_store::set_domain_parked(&state.pool, id, parked).await? {
            return Err(AppError::not_found(format!("domain {id} not found")));
        }
    }
    Ok(Json(domain_summary(&state, id).await?))
}

/// Remove a domain: delete its reports and block it. `404` when the domain is
/// unknown or already removed.
pub async fn delete_domain(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    if dmarc_store::ignore_domain(&state.pool, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found(format!("domain {id} not found")))
    }
}
