//! The client report recipients as text (Settings → Client reports,
//! Recipients), a list like the others ([`ListText`]):
//!
//! `GET /api/settings/client-reports/recipients` — every organization with
//! its recipients, `organization,none` for one without.
//! `POST /api/settings/client-reports/recipients/preview` — a draft → who
//! starts and who stops getting a report, and the line errors; writes nothing.
//! `PUT /api/settings/client-reports/recipients` — a draft → the same, saved in
//! one transaction; refused with `errors` when the text has errors, and with
//! `409` when it would leave no recipient while the reports are on (switch
//! them off first).
//!
//! Format: one `organization,email` per line, or `organization,none` (see
//! [`crate::client_reports::list`]). Like the domain mapping, the list is a
//! projection: an organization the text names gets exactly the recipients
//! named for it, one it does not name keeps its own, so a line taken out
//! changes nothing.

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

/// Every organization by name, each with its recipients by address, or
/// `none`.
pub async fn get_recipients(State(state): State<Arc<AppState>>) -> AppResult<Json<ListText>> {
    let (mut organizations, _) = dmarc_store::scope_options(&state.pool).await?;
    organizations.sort_by_key(|org| org.name.to_lowercase());
    let recipients = dmarc_store::list_client_report_recipients(&state.pool).await?;
    Ok(Json(ListText {
        text: format_recipients(&organizations, &recipients),
    }))
}

/// What saving `text` would change. Lines with errors are left out of the
/// changes (an organization named only on such lines keeps its
/// recipients): the errors come first in the editor and block the save.
pub async fn preview_recipients(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<RecipientsSummary>> {
    let parsed = parse(&state.pool, &req.text).await?;
    let changes = dmarc_store::preview_client_report_recipients(
        &state.pool,
        &parsed.organizations,
        &parsed.recipients,
    )
    .await?;
    Ok(Json(RecipientsSummary {
        changes,
        errors: parsed.errors,
    }))
}

/// Give each organization `text` names the recipients named for it. Any
/// line error rejects the whole list (with the preview's `errors`) and
/// nothing is written. A save that would leave no recipient at all while the
/// reports are on is refused (`409`), as switching them on with an empty
/// list is: they would send nothing.
pub async fn save_recipients(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ListText>,
) -> AppResult<Json<RecipientsSummary>> {
    let parsed = parse(&state.pool, &req.text).await?;
    if !parsed.errors.is_empty() {
        return Err(rejected_lines(&parsed.errors, "list"));
    }
    let (named, next) = (&parsed.organizations, &parsed.recipients);
    let preview = dmarc_store::preview_client_report_recipients(&state.pool, named, next).await?;
    let left = preview.unchanged_count + preview.start.len();
    if left == 0 && AppSettings::load(&state.pool).await?.client_reports.enabled {
        return Err(AppError::conflict(NO_CLIENT_REPORT_RECIPIENTS));
    }
    let changes = dmarc_store::replace_client_report_recipients(&state.pool, named, next).await?;
    Ok(Json(RecipientsSummary {
        changes,
        errors: Vec::new(),
    }))
}
