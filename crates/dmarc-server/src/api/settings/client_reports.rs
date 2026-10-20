//! The client reports group (`PUT /api/settings/client-reports`): the
//! monthly reports' switch, their day and hour, and where replies go; and
//! one organization's report as it would go out now, rendered (`GET
//! /api/settings/client-reports/email`) or sent to the test recipient
//! (`POST /api/settings/client-reports/email/send`). The recipient list is
//! its own resource (`/api/settings/client-reports/recipients`, `api::client_reports`).

use super::sending::{send_failed, test_recipient, SendTestResponse};
use super::settings_response;
use crate::client_reports::report::{organization_report, report_config};
use crate::error::{AppError, AppResult};
use crate::extract::{Json, Query};
use crate::mail::{parse_address, possessive, Letterhead};
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{
    get_organization_summary, list_client_report_recipients, save_settings, unix_now, AppSettings,
    ClientReportSettings, CLIENT_REPORT_DAY_RANGE,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// `POST /api/settings/client-reports/email/send`.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SendClientReportEmailRequest {
    pub organization_id: i64,
    /// The client reports' Reply-To as on screen; blank: the sending group's.
    /// Absent: the saved one.
    pub reply_to: Option<String>,
}

const PREVIEW_SUBJECT_PREFIX: &str = "[Preview] ";

/// Render organization `organization_id`'s report as it would go out now and
/// send it to the saved test recipient (default: the sender address), through
/// the saved sending account and the client reports' Reply-To (`reply_to` as
/// on screen, else the saved one). Errors are the test sends': `409` while
/// sending is off or incomplete (or there is no test recipient), `422` when the server could not
/// be reached or refused, each with `reason` and `detail`; `404` for an
/// unknown organization. The organization need not be listed.
pub async fn send_email(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendClientReportEmailRequest>,
) -> AppResult<Json<SendTestResponse>> {
    let mut settings = AppSettings::load(&state.pool).await?;
    if let Some(raw) = &req.reply_to {
        let reply_to = raw.trim();
        if !reply_to.is_empty() && parse_address(reply_to).is_none() {
            return Err(AppError::unprocessable(format!(
                "The Reply-To address {reply_to} is not a valid email address."
            )));
        }
        settings.client_reports.reply_to = reply_to.to_owned();
    }
    let config = report_config(&settings).map_err(|e| send_failed(&e))?;
    let organization = get_organization_summary(&state.pool, req.organization_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!("organization {} not found", req.organization_id))
        })?;
    let to = test_recipient(&settings)?;
    let mut content = organization_report(
        &state.pool,
        organization.id,
        &organization.name,
        unix_now(),
        &Letterhead::load(&state.pool, &settings).await,
    )
    .await?;
    content.subject = format!("{PREVIEW_SUBJECT_PREFIX}{}", content.subject);
    state
        .mail
        .send(&config, &content.message(to.clone()))
        .await
        .map_err(|e| send_failed(&e))?;
    Ok(Json(SendTestResponse {
        ok: true,
        message: format!(
            "Preview of {} report sent to {to}.",
            possessive(&organization.name)
        ),
    }))
}

/// `GET /api/settings/client-reports/email?organization_id=`.
#[derive(Debug, Deserialize)]
pub struct ClientReportEmailQuery {
    pub organization_id: i64,
}

/// One organization's client report as it would go out now, rendered by
/// the code the monthly send uses.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ClientReportEmail {
    pub subject: String,
    /// The `From` display name ([`crate::mail::from_name`]).
    pub from_name: String,
    /// The HTML version, its inline images (the Marcan mark, the logo) as
    /// `data:` URLs so a sandboxed frame shows them.
    pub html: String,
    /// The plain-text version.
    pub text: String,
}

/// Render organization `organization_id`'s report as it would go out now,
/// under the saved branding and license. Stores nothing and sends nothing,
/// so it works with sending and the reports off; `404` for an unknown
/// organization, which need not be listed.
pub async fn show_email(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ClientReportEmailQuery>,
) -> AppResult<Json<ClientReportEmail>> {
    let settings = AppSettings::load(&state.pool).await?;
    let organization = get_organization_summary(&state.pool, q.organization_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!("organization {} not found", q.organization_id))
        })?;
    let letterhead = Letterhead::load(&state.pool, &settings).await;
    let content = organization_report(
        &state.pool,
        organization.id,
        &organization.name,
        unix_now(),
        &letterhead,
    )
    .await?;
    Ok(Json(ClientReportEmail {
        html: content.html_with_data_urls(),
        subject: content.subject,
        from_name: letterhead.from_name(),
        text: content.text,
    }))
}

/// The group the request asks for, validated.
pub(crate) fn client_report_settings(
    payload: &ClientReportSettings,
) -> AppResult<ClientReportSettings> {
    let (first, last) = CLIENT_REPORT_DAY_RANGE;
    let day_of_month = Some(payload.day_of_month)
        .filter(|d| (first..=last).contains(d))
        .ok_or_else(|| {
            AppError::unprocessable(format!(
                "The day of the month must be between {first} and {last}, so every month has it."
            ))
        })?;
    let hour_utc = Some(payload.hour_utc)
        .filter(|&h| h < 24)
        .ok_or_else(|| AppError::unprocessable("The hour must be between 0 and 23."))?;
    let reply_to = match payload.reply_to.trim() {
        "" => String::new(),
        raw => parse_address(raw)
            .ok_or_else(|| {
                AppError::unprocessable(format!(
                    "The Reply-To address {raw} is not a valid email address."
                ))
            })?
            .to_string(),
    };
    Ok(ClientReportSettings {
        enabled: payload.enabled,
        day_of_month,
        hour_utc,
        reply_to,
    })
}

/// Returned when the reports would be on with nobody to send them to
/// (switched on with an empty list, or the list emptied while they are on);
/// the Save bar says the same.
pub(crate) const NO_RECIPIENTS: &str = "Add at least one recipient to the list first.";

/// Store the group; answers every setting. The reports may be on while
/// sending is off: they go out from the first report day sending is on. They
/// may not be on with an empty recipient list (`409`), which would send
/// nothing.
pub async fn put_client_reports(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClientReportSettings>,
) -> AppResult<Json<SettingsResponse>> {
    let reports = client_report_settings(&payload)?;
    if reports.enabled && list_client_report_recipients(&state.pool).await?.is_empty() {
        return Err(AppError::conflict(NO_RECIPIENTS));
    }
    save_settings(&state.pool, &reports).await?;
    settings_response(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ClientReportSettings {
        ClientReportSettings {
            enabled: true,
            day_of_month: 3,
            hour_utc: 6,
            reply_to: " support@msp.example ".into(),
        }
    }

    #[test]
    fn a_valid_group_is_cleaned_up() {
        let s = client_report_settings(&request()).unwrap();
        assert_eq!(s.day_of_month, 3);
        assert_eq!(s.reply_to, "support@msp.example");
        let blank = client_report_settings(&ClientReportSettings {
            reply_to: "  ".into(),
            ..request()
        })
        .unwrap();
        assert_eq!(blank.reply_to, "");
    }

    #[test]
    fn invalid_values_are_named() {
        let err = |r: ClientReportSettings| client_report_settings(&r).unwrap_err().message;
        assert!(err(ClientReportSettings {
            day_of_month: 29,
            ..request()
        })
        .starts_with("The day of the month must be between 1 and 28"));
        assert!(err(ClientReportSettings {
            day_of_month: 0,
            ..request()
        })
        .starts_with("The day of the month"));
        assert_eq!(
            err(ClientReportSettings {
                hour_utc: 24,
                ..request()
            }),
            "The hour must be between 0 and 23."
        );
        assert_eq!(
            err(ClientReportSettings {
                reply_to: "support".into(),
                ..request()
            }),
            "The Reply-To address support is not a valid email address."
        );
    }
}
