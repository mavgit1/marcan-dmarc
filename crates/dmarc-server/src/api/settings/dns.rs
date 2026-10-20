//! The DNS group (`PUT /api/settings/dns`): the resolvers every DNS check uses.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{save_settings, DnsSettings};
use std::net::IpAddr;
use std::sync::Arc;

/// Store the resolvers (blanks dropped, each as its address reads) and
/// rebuild the checker for them; answers every setting.
pub async fn put_dns(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DnsSettings>,
) -> AppResult<Json<SettingsResponse>> {
    let requested: Vec<&str> = payload
        .nameservers
        .iter()
        .map(|ns| ns.trim())
        .filter(|ns| !ns.is_empty())
        .collect();
    if requested.len() > 4 {
        return Err(AppError::unprocessable(
            "at most 4 nameservers can be configured",
        ));
    }
    let mut nameservers = Vec::with_capacity(requested.len());
    for ns in requested {
        let ip = ns
            .parse::<IpAddr>()
            .map_err(|_| AppError::unprocessable(format!("invalid IP address format: {ns}")))?;
        nameservers.push(ip.to_string());
    }
    save_settings(&state.pool, &DnsSettings { nameservers }).await?;
    state.dns.reconfigure(&state.pool).await;
    settings_response(&state).await
}
