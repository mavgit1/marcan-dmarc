//! `GET /api/scope` — the organizations and domains the header's scope filter
//! offers: ids and names only, so loading it costs no report aggregation.

use crate::error::AppResult;
use crate::extract::Json;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{ScopeDomain, ScopeOrganization};
use serde::Serialize;
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ScopeOptions {
    pub organizations: Vec<ScopeOrganization>,
    pub domains: Vec<ScopeDomain>,
}

pub async fn get_scope(State(state): State<Arc<AppState>>) -> AppResult<Json<ScopeOptions>> {
    let (organizations, domains) = dmarc_store::scope_options(&state.pool).await?;
    Ok(Json(ScopeOptions {
        organizations,
        domains,
    }))
}
