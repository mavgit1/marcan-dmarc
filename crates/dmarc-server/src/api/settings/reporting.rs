//! The reporting group (`PUT /api/settings/reporting`): the report address and
//! the reporting mode.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_dns::dmarc::is_valid_report_address;
use dmarc_store::{save_settings, AppSettings, ReportingMode, ReportingSettings};
use serde::Deserialize;
use std::sync::Arc;
use ts_rs::TS;

/// `PUT /api/settings/reporting`: the reporting group's two values, as the
/// settings answer them, each optional. The one group saved a value at a
/// time: the setup wizard stores the address with the connection and the
/// mode in a later step.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SaveReportingRequest {
    /// The address DMARC `rua=` tags point at. Absent: keep the stored one.
    /// Empty: none.
    pub report_address: Option<String>,
    /// Absent: keep the stored mode (the default until one is saved).
    pub reporting_mode: Option<ReportingMode>,
}

/// Store the fields that are given over the stored ones (`422` when the
/// report address is given and not an email address); answers every setting.
pub async fn put_reporting(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SaveReportingRequest>,
) -> AppResult<Json<SettingsResponse>> {
    let address = payload.report_address.as_deref().map(str::trim);
    if let Some(addr) = address.filter(|a| !a.is_empty()) {
        if !is_valid_report_address(addr) {
            return Err(AppError::unprocessable(format!(
                "report address is not a valid email address: {addr}"
            )));
        }
    }
    let stored = AppSettings::load(&state.pool).await?.reporting;
    let reporting = ReportingSettings {
        report_address: match address {
            Some(addr) => Some(addr.to_owned()).filter(|a| !a.is_empty()),
            None => stored.report_address,
        },
        reporting_mode: payload.reporting_mode.or(stored.reporting_mode),
    };
    save_settings(&state.pool, &reporting).await?;
    settings_response(&state).await
}
