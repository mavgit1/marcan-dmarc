//! The client report recipients as text (Settings → Client reports,
//! Recipients), a list like the others ([`ListText`]):
//!
//! `GET /api/settings/client-reports/recipients` — the list.
//! `POST /api/settings/client-reports/recipients/preview` — a draft → who
//! starts and who stops getting a report, and the line errors; writes nothing.
//! `PUT /api/settings/client-reports/recipients` — a draft → the same, saved in
//! one transaction; refused with `errors` when the text has errors, and with
//! `409` when it is empty while the reports are on (switch them off first).
//!
//! Format: one `organization,email` per line (see
//! [`crate::client_reports::list`]). Unlike the domain mapping, the list is
//! the full truth: the saved list replaces every stored recipient, so a line
//! taken out stops that report.

use super::mapping::rejected_lines;
use super::settings::NO_CLIENT_REPORT_RECIPIENTS;
use crate::client_reports::list::{format_recipients, parse_recipients, ParsedRecipients};
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::view::{LineError, ListText};
use crate::AppState;
use axum::extract::State;
use dmarc_store::{AppSettings, ClientReportChanges};
use serde::Serialize;
use sqlx::SqlitePool;
use std::sync::Arc;
use ts_rs::TS;

/// What saving a list changes (preview) or changed (save).
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct RecipientsSummary {
    pub changes: ClientReportChanges,
    /// Lines that cannot be saved; their recipients are left out of `changes`.
    pub errors: Vec<LineError>,
}

async fn parse(pool: &SqlitePool, text: &str) -> AppResult<ParsedRecipients> {
    let (organizations, _) = dmarc_store::scope_options(pool).await?;
    Ok(parse_recipients(text, &organizations))
}

/// The stored list, by organization name, then address.
pub async fn get_recipients(State(state): State<Arc<AppState>>) -> AppResult<Json<ListText>> {
    let recipients = dmarc_store::list_client_report_recipients(&state.pool).await?;
    Ok(Json(ListText {
        text: format_recipients(&recipients),
    }))
}

/// What saving `text` would change. Lines with errors are left out of the
/// changes, so they would read as "stops" for a recipient stored now: the
/// errors come first in the editor and block the save.
pub async fn preview_recipients(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<RecipientsSummary>> {
    let parsed = parse(&state.pool, &req.text).await?;
    let changes =
        dmarc_store::preview_client_report_recipients(&state.pool, &parsed.recipients).await?;
    Ok(Json(RecipientsSummary {
        changes,
        errors: parsed.errors,
    }))
}

/// Replace the stored recipients with `text`. Any line error rejects the
/// whole list (with the preview's `errors`) and nothing is written. An empty
/// list while the reports are on is refused (`409`), as switching them on
/// with an empty list is: they would send nothing.
pub async fn save_recipients(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<RecipientsSummary>> {
    let parsed = parse(&state.pool, &req.text).await?;
    if !parsed.errors.is_empty() {
        return Err(rejected_lines(&parsed.errors, "list"));
    }
    if parsed.recipients.is_empty() && AppSettings::load(&state.pool).await?.client_reports.enabled
    {
        return Err(AppError::conflict(NO_CLIENT_REPORT_RECIPIENTS));
    }
    let changes =
        dmarc_store::replace_client_report_recipients(&state.pool, &parsed.recipients).await?;
    Ok(Json(RecipientsSummary {
        changes,
        errors: Vec::new(),
    }))
}
