//! Analytics endpoints: statistics and timeseries.

use crate::error::{AppError, AppResult};
use crate::extract::{Json, Query};
use crate::filters::{Items, ScopeQuery};
use crate::view::stats::{statistics, Statistics, TimeseriesPoint};
use crate::AppState;
use axum::extract::State;
use dmarc_assess::previous_window;
use dmarc_store::{
    count_domains, list_timeseries, report_span, scope_stats, unix_now, TimeseriesBucket,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct TimeseriesQuery {
    pub bucket: Option<String>,
}

pub async fn get_timeseries(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ScopeQuery>,
    Query(q): Query<TimeseriesQuery>,
) -> AppResult<Json<Items<TimeseriesPoint>>> {
    let bucket = parse_bucket(q.bucket.as_deref())?;
    let items = list_timeseries(&state.pool, scope.scope()?, bucket, unix_now())
        .await?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(Items::new(items)))
}

/// `GET /api/statistics` — scope totals, delivered failures and the change
/// against the previous window of equal length.
pub async fn get_statistics(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ScopeQuery>,
) -> AppResult<Json<Statistics>> {
    let scope = q.scope()?;
    let current = scope_stats(&state.pool, scope).await?;
    let (first, last) = report_span(&state.pool, scope).await?;
    let previous = match previous_window(scope.since, first, last, unix_now()) {
        Some((since, until)) => {
            Some(scope_stats(&state.pool, scope.window(Some(since), Some(until))).await?)
        }
        None => None,
    };
    let domain_count = count_domains(&state.pool, scope).await?;
    Ok(Json(statistics(current, previous, domain_count)))
}

fn parse_bucket(raw: Option<&str>) -> AppResult<TimeseriesBucket> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(TimeseriesBucket::Day),
        Some("day") => Ok(TimeseriesBucket::Day),
        Some("week") => Ok(TimeseriesBucket::Week),
        Some(other) => Err(AppError::bad_request(format!(
            "invalid bucket '{other}': expected day or week"
        ))),
    }
}
