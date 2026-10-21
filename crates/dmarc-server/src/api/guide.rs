//! `GET /api/guide`: what the guide shows that no other endpoint serves, the
//! monitored domains counted by policy. The rest of the guide reads the
//! reporting setup, the settings, the health and `GET /api/meta`.

use crate::error::AppResult;
use crate::extract::Json;
use crate::view::domain::{judged_summary, Judging};
use crate::view::guide::{GuideResponse, PolicyCounts};
use crate::AppState;
use axum::extract::State;
use dmarc_store::{list_domain_summaries, unix_now, Scope};
use std::sync::Arc;

pub async fn get_guide(State(state): State<Arc<AppState>>) -> AppResult<Json<GuideResponse>> {
    let at = Judging::load(&state.pool, unix_now()).await?;
    let rows = list_domain_summaries(&state.pool, Scope::ALL).await?;
    let policies = PolicyCounts::of(
        rows.into_iter()
            .map(|row| judged_summary(row, at).assessment.enforcement.state),
    );
    Ok(Json(GuideResponse { policies }))
}
