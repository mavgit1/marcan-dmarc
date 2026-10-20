//! The license group (`PUT /api/settings/license`): the operator key. A valid
//! key switches the free-software notice off; every feature works without it.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::license::verify_key;
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_store::{save_settings, AppSettings, LicenseSettings};
use serde::Deserialize;
use std::sync::Arc;
use ts_rs::TS;

/// `PUT /api/settings/license`: the whole group. Its one value is a secret,
/// which the settings never answer (they name it: `has_key`, `key_hint`).
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SaveLicenseRequest {
    /// The operator key. Absent: keep the stored key. Empty: remove it.
    /// Otherwise the key as entered, checked before it is stored.
    pub operator_key: Option<String>,
}

pub(crate) const INVALID_KEY: &str = "This key is not valid.";

/// The group the request asks for over the `stored` one, validated: a new
/// key must verify (`422`, and nothing is stored).
pub(crate) fn license_settings(
    payload: &SaveLicenseRequest,
    stored: &LicenseSettings,
) -> AppResult<LicenseSettings> {
    let operator_key = match payload.operator_key.as_deref().map(str::trim) {
        None => stored.operator_key.clone(),
        Some("") => None,
        Some(key) => {
            verify_key(key).map_err(|_| AppError::unprocessable(INVALID_KEY))?;
            Some(key.to_owned())
        }
    };
    Ok(LicenseSettings { operator_key })
}

/// Store the group; answers every setting.
pub async fn put_license(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SaveLicenseRequest>,
) -> AppResult<Json<SettingsResponse>> {
    let stored = AppSettings::load(&state.pool).await?.license;
    let license = license_settings(&payload, &stored)?;
    save_settings(&state.pool, &license).await?;
    settings_response(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    use crate::license::test_keys::TEST_KEY as KEY;

    fn request(key: Option<&str>) -> SaveLicenseRequest {
        SaveLicenseRequest {
            operator_key: key.map(str::to_owned),
        }
    }

    fn with_key() -> LicenseSettings {
        LicenseSettings {
            operator_key: Some(KEY.into()),
        }
    }

    #[test]
    fn a_valid_key_is_stored_as_entered() {
        for stored in [LicenseSettings::default(), with_key()] {
            let s = license_settings(&request(Some(&format!("  {KEY} "))), &stored).unwrap();
            assert_eq!(s.operator_key.as_deref(), Some(KEY));
        }
    }

    #[test]
    fn an_invalid_key_is_refused() {
        let err =
            license_settings(&request(Some("test@example.com.AAAA")), &with_key()).unwrap_err();
        assert_eq!(err.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err.message, "This key is not valid.");
    }

    #[test]
    fn absent_keeps_the_key_and_empty_removes_it() {
        let kept = license_settings(&request(None), &with_key()).unwrap();
        assert_eq!(kept, with_key());
        let cleared = license_settings(&request(Some(" ")), &with_key()).unwrap();
        assert_eq!(cleared, LicenseSettings::default());
    }
}
