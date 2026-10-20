//! `GET /api/reports` and `GET /api/reports/:id`.

use super::identity::identities;
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Path, Query};
use crate::filters::{parse_pagination, parse_sort, Page, ScopeQuery};
use crate::view::report::{report_record, ReportRecord, ReportSummary};
use crate::AppState;
use axum::extract::State;
use dmarc_store::{get_report_summary, list_records_for_report, ReportListQuery, ReportSort};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Deserialize)]
pub struct ReportsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Case-insensitive substring of the domain or the reporting organization.
    pub q: Option<String>,
    /// `date_begin`, `domain`, `org_name`, … ; `-` prefix for descending.
    pub sort: Option<String>,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ReportDetailResponse {
    #[serde(flatten)]
    pub report: ReportSummary,
    pub records: Vec<ReportRecord>,
}

pub async fn list_reports(
    State(state): State<Arc<AppState>>,
    Query(scope): Query<ScopeQuery>,
    Query(q): Query<ReportsQuery>,
) -> AppResult<Json<Page<ReportSummary>>> {
    let (limit, offset) = parse_pagination(q.limit, q.offset)?;
    let query = ReportListQuery {
        scope: scope.scope()?,
        search: q.q,
        sort: parse_sort(q.sort.as_deref(), ReportSort::DEFAULT)?,
        limit,
        offset,
    };
    let (items, total) = dmarc_store::list_reports(&state.pool, &query).await?;
    let items: Vec<ReportSummary> = items.into_iter().map(Into::into).collect();
    Ok(Json(Page::new(items, total, limit, offset)))
}

pub async fn get_report(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Json<ReportDetailResponse>> {
    let report: ReportSummary = get_report_summary(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("report {id} not found")))?
        .into();
    let records = list_records_for_report(&state.pool, id).await?;
    let names = identities(&state, records.iter().map(|r| r.source_ip.as_str())).await?;
    let records = records
        .into_iter()
        .map(|record| {
            let identity = names.get(&record.source_ip).cloned().unwrap_or_default();
            report_record(record, identity)
        })
        .collect();
    Ok(Json(ReportDetailResponse { report, records }))
}
