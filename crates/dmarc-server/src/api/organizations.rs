//! `GET /api/organizations?since` — organization list with domain/report counts, the
//! scope's message totals and what needs attention across its domains.
//! `POST /api/organizations` — create an organization (409 when the name is taken).
//! `GET /api/organizations/{id}` — one organization (same shape as a list item). Per-domain
//! figures come from `GET /api/domains?organization_id=…`, totals from `/api/statistics`.
//! `GET /api/organizations/{id}/client-report` — when its next client report
//! goes out and to how many recipients; `null` when it gets none.
//! `PATCH /api/organizations/{id}` — rename an organization.
//! `DELETE /api/organizations/{id}` — delete a non-default organization; its domains
//! (and their reports) move to the default organization.

use crate::client_reports::job::schedule;
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path, Query};
use crate::filters::{Items, ScopeQuery};
use crate::view::domain::{domain_summaries, Judging};
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use dmarc_assess::{organization_attention, OrganizationAttention};
use dmarc_store::{
    list_client_report_recipients, list_domain_summaries, unix_now, AppSettings,
    DeleteOrganization, OrganizationWrite, Scope,
};
use dmarc_store::{MailCounts, OrganizationSummary};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// Body of create and rename.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct OrganizationNameRequest {
    pub name: String,
}

impl OrganizationNameRequest {
    fn name(&self) -> AppResult<&str> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(AppError::unprocessable("organization name cannot be empty"));
        }
        Ok(name)
    }
}

/// One organization with its domains' figures within `since`.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct OrganizationRow {
    #[serde(flatten)]
    pub organization: OrganizationSummary,
    #[serde(flatten)]
    #[ts(flatten)]
    pub mail: MailCounts,
    /// Failures quarantined or rejected.
    pub blocked_messages: i64,
    pub attention: OrganizationAttention,
}

pub async fn list_organizations(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ScopeQuery>,
) -> AppResult<Json<Items<OrganizationRow>>> {
    let organizations = dmarc_store::list_organization_summaries(&state.pool).await?;
    let rows = list_domain_summaries(&state.pool, Scope::new(None, None, q.since_ts()?)).await?;
    let domains = domain_summaries(rows, Judging::load(&state.pool, unix_now()).await?);
    let items = organizations
        .into_iter()
        .map(|organization| {
            let own: Vec<_> = domains
                .iter()
                .filter(|d| d.row.organization.id == organization.id)
                .collect();
            let mail: MailCounts = own.iter().map(|d| &d.row.mail).sum();
            OrganizationRow {
                blocked_messages: mail.blocked(),
                mail,
                attention: organization_attention(own.iter().map(|d| {
                    let grade = d.row.dns.as_ref().map(|s| (s.grade.as_str(), s.score));
                    (&d.assessment.verdicts, grade)
                })),
                organization,
            }
        })
        .collect();
    Ok(Json(Items::new(items)))
}

fn saved(write: OrganizationWrite, id: Option<i64>, name: &str) -> AppResult<OrganizationSummary> {
    match write {
        OrganizationWrite::Saved(organization) => Ok(organization),
        OrganizationWrite::NameTaken => Err(AppError::conflict(format!(
            "an organization named {name} already exists"
        ))),
        OrganizationWrite::NotFound => Err(match id {
            Some(id) => AppError::not_found(format!("organization {id} not found")),
            None => AppError::internal("organization vanished after create"),
        }),
    }
}

pub async fn create_organization(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OrganizationNameRequest>,
) -> AppResult<(StatusCode, Json<OrganizationSummary>)> {
    let name = req.name()?;
    let write = dmarc_store::create_organization(&state.pool, name).await?;
    Ok((StatusCode::CREATED, Json(saved(write, None, name)?)))
}

pub async fn get_organization(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<OrganizationSummary>> {
    let item = dmarc_store::get_organization_summary(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("organization {id} not found")))?;
    Ok(Json(item))
}

/// An organization's next client report.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct UpcomingClientReport {
    /// When it goes out (unix seconds): the report time that is due while
    /// one of the organization's recipients does not have it yet, else the
    /// next one.
    pub next_send_at: i64,
    /// The organization's lines in the recipient list.
    pub recipients: i64,
    /// Its due report is held until the mailbox is read again.
    pub held: bool,
}

/// The organization's next client report; `null` while the reports or
/// sending are off, or when the organization is not in the recipient list.
/// The organization page shows it from `client_report_notice_days` before.
pub async fn get_client_report(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<Option<UpcomingClientReport>>> {
    dmarc_store::get_organization_summary(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("organization {id} not found")))?;
    let settings = AppSettings::load(&state.pool).await?;
    if !settings.sending.group.enabled {
        return Ok(Json(None));
    }
    let schedule = schedule(&state.pool, &settings, unix_now()).await?;
    let recipients = list_client_report_recipients(&state.pool)
        .await?
        .iter()
        .filter(|r| r.organization_id == id)
        .count() as i64;
    Ok(Json(
        schedule
            .next_send_for(id)
            .filter(|_| recipients > 0)
            .map(|next_send_at| UpcomingClientReport {
                next_send_at,
                recipients,
                held: schedule.held_for(id),
            }),
    ))
}

/// Rename an organization. Names are unique (409 when taken).
pub async fn update_organization(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<OrganizationNameRequest>,
) -> AppResult<Json<OrganizationSummary>> {
    let name = req.name()?;
    let write = dmarc_store::rename_organization(&state.pool, id, name).await?;
    Ok(Json(saved(write, Some(id), name)?))
}

/// Delete an organization. The default organization cannot be deleted (409). Domains of
/// the deleted organization move to the default organization with their reports, so
/// deleting a client organization never deletes report data.
pub async fn delete_organization(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    match dmarc_store::delete_organization(&state.pool, id).await? {
        DeleteOrganization::Deleted => Ok(StatusCode::NO_CONTENT),
        DeleteOrganization::NotFound => {
            Err(AppError::not_found(format!("organization {id} not found")))
        }
        DeleteOrganization::IsDefault => Err(AppError::conflict(
            "the default organization cannot be deleted",
        )),
    }
}
