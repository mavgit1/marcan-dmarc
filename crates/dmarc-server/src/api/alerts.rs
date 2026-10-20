//! `GET /api/alerts`: what changed, newest first, narrowed by the scope
//! (organization, domain, and `since` over when the change was found).

use crate::error::AppResult;
use crate::extract::{Json, Query};
use crate::filters::{parse_pagination, Page, ScopeQuery};
use crate::view::alerts::AlertItem;
use crate::AppState;
use axum::extract::State;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct AlertsPageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_alerts(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ScopeQuery>,
    Query(q): Query<AlertsPageQuery>,
) -> AppResult<Json<Page<AlertItem>>> {
    let (limit, offset) = parse_pagination(q.limit, q.offset)?;
    let (items, total) =
        dmarc_store::list_alert_events(&state.pool, scope.scope()?, limit, offset).await?;
    let items = items.into_iter().map(AlertItem::from).collect();
    Ok(Json(Page::new(items, total, limit, offset)))
}
