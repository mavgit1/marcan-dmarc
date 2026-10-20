//! The alerts group (`PUT /api/settings/alerts`): alert emails, the digest
//! and the base URL their links point at; and the test alert (`POST
//! /api/settings/alerts/test`).

use super::sending::{send_failed, test_recipient, SendTestResponse};
use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::mail::{self, parse_address, Letterhead, SmtpConfig};
use crate::view::settings::SettingsResponse;
use crate::AppState;
use axum::extract::State;
use dmarc_assess::catalogue_entry;
use dmarc_store::{
    save_settings, set_last_digest_at, unix_now, AlertSettings, AppSettings, DigestFrequency,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use ts_rs::TS;

/// `raw` as a base URL for links, trimmed and without a trailing slash:
/// `http(s)://` and a host, no spaces; empty when blank.
pub(crate) fn parse_base_url(raw: &str) -> AppResult<String> {
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() {
        return Ok(String::new());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let host = rest.and_then(|r| r.split('/').next()).unwrap_or_default();
    if host.is_empty() || url.chars().any(char::is_whitespace) {
        return Err(AppError::unprocessable(format!(
            "The base URL {url} is not a web address: write it as it opens Marcan, such as https://marcan.example.com."
        )));
    }
    Ok(url.to_owned())
}

/// Addresses typed one per entry or several to an entry (comma, semicolon
/// or line separated): trimmed, blanks and repeats (ignoring case) dropped,
/// in typed order.
fn split_recipients(entries: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        for item in entry.split(['\n', ',', ';']).map(str::trim) {
            if !item.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(item)) {
                out.push(item.to_owned());
            }
        }
    }
    out
}

/// Returned when alert emails are switched on with nobody to send them to;
/// the Save bar says the same.
const NO_RECIPIENT: &str = "Add at least one recipient first.";

/// The group the request asks for, validated and as it is stored:
/// recipients cleaned up, the base URL without a trailing slash, the muted
/// kinds sorted and each once.
pub(crate) fn alert_settings(payload: &AlertSettings) -> AppResult<AlertSettings> {
    let recipients = split_recipients(&payload.recipients);
    for recipient in &recipients {
        if parse_address(recipient).is_none() {
            return Err(AppError::unprocessable(format!(
                "The recipient {recipient} is not a valid email address."
            )));
        }
    }
    if recipients.is_empty() && payload.enabled {
        return Err(AppError::conflict(NO_RECIPIENT));
    }
    let digest_hour = Some(payload.digest_hour)
        .filter(|&h| h < 24)
        .ok_or_else(|| AppError::unprocessable("The digest hour must be between 0 and 23."))?;
    let mut muted_kinds = BTreeSet::new();
    for id in &payload.muted_kinds {
        if catalogue_entry(id.trim()).is_none() {
            return Err(AppError::unprocessable(format!(
                "There is no kind of alert called {id}."
            )));
        }
        muted_kinds.insert(id.trim().to_owned());
    }
    Ok(AlertSettings {
        enabled: payload.enabled,
        recipients,
        digest: payload.digest,
        digest_hour,
        digest_weekday: payload.digest_weekday,
        base_url: parse_base_url(&payload.base_url)?,
        muted_kinds: muted_kinds.into_iter().collect(),
    })
}

/// Store the group; answers every setting. Alerts may be on while sending is
/// off: events are listed in the UI either way, and mailed once sending is on.
/// `enabled` governs every alert email, the digest too. Switching the digest
/// on (its frequency, or alert emails with a frequency set) starts its period
/// now, so the first digest goes out at the next digest time, not at once.
pub async fn put_alerts(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AlertSettings>,
) -> AppResult<Json<SettingsResponse>> {
    let alerts = alert_settings(&payload)?;
    let before = AppSettings::load(&state.pool).await?.alerts;
    save_settings(&state.pool, &alerts).await?;
    let digest_on = |a: &AlertSettings| a.enabled && a.digest != DigestFrequency::Off;
    if !digest_on(&before) && digest_on(&alerts) {
        set_last_digest_at(&state.pool, unix_now()).await?;
    }
    settings_response(&state).await
}

const TEST_SUBJECT_PREFIX: &str = "[Test] ";

/// `POST /api/settings/alerts/test`.
#[derive(Debug, Deserialize, Default, TS)]
#[ts(export)]
#[ts(optional_fields)]
pub struct SendTestAlertRequest {
    /// The group as on screen, in the shape of its `PUT` (its `enabled` is
    /// ignored): its base URL is used. Absent: the stored group.
    pub alerts: Option<AlertSettings>,
}

/// `POST /api/settings/alerts/test`: a sample alert email (made-up changes on
/// example.com) to the saved test recipient (default: the sender address),
/// through the saved sending account. Errors are the sending test's: `409`
/// while sending is off or incomplete (or there is no test recipient), `422` when
/// the server could not be reached or refused, each with `reason` and
/// `detail`; `422` for a draft that is not valid.
pub async fn send_test(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SendTestAlertRequest>,
) -> AppResult<Json<SendTestResponse>> {
    let settings = AppSettings::load(&state.pool).await?;
    let config = SmtpConfig::from_settings(&settings).map_err(|e| send_failed(&e))?;
    let alerts = match &req.alerts {
        Some(draft) => alert_settings(&AlertSettings {
            enabled: false,
            ..draft.clone()
        })?,
        None => settings.alerts.clone(),
    };
    let to = test_recipient(&settings)?;
    let mut content = mail::alerts::render(
        &mail::alerts::sample_items(unix_now()),
        alerts.base_url(),
        &Letterhead::load(&state.pool, &settings).await,
    );
    content.subject = format!("{TEST_SUBJECT_PREFIX}{}", content.subject);
    state
        .mail
        .send(&config, &content.message(to.clone()))
        .await
        .map_err(|e| send_failed(&e))?;
    Ok(Json(SendTestResponse {
        ok: true,
        message: format!("Test alert sent to {to}."),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmarc_store::Weekday;

    fn request() -> AlertSettings {
        AlertSettings {
            enabled: true,
            recipients: vec![
                " ops@example.com ".into(),
                String::new(),
                "OPS@example.com".into(),
            ],
            digest: DigestFrequency::Daily,
            digest_hour: 7,
            digest_weekday: Weekday::Monday,
            base_url: " https://marcan.example.com/ ".into(),
            muted_kinds: vec!["health_changed".into(), "dns_record_changed.bimi".into()],
        }
    }

    #[test]
    fn a_valid_group_is_cleaned_up() {
        let s = alert_settings(&request()).unwrap();
        assert_eq!(s.recipients, ["ops@example.com"]);
        assert_eq!(s.base_url, "https://marcan.example.com");
        assert_eq!(s.muted_kinds, ["dns_record_changed.bimi", "health_changed"]);
    }

    #[test]
    fn invalid_values_are_named() {
        let err = |r: AlertSettings| alert_settings(&r).unwrap_err().message;
        assert_eq!(
            err(AlertSettings {
                recipients: vec!["nobody".into()],
                ..request()
            }),
            "The recipient nobody is not a valid email address."
        );
        assert_eq!(
            err(AlertSettings {
                recipients: vec![],
                ..request()
            }),
            "Add at least one recipient first."
        );
        assert!(err(AlertSettings {
            digest_hour: 24,
            ..request()
        })
        .starts_with("The digest hour"));
        assert!(err(AlertSettings {
            base_url: "marcan.example.com".into(),
            ..request()
        })
        .starts_with("The base URL marcan.example.com is not a web address"));
        assert!(err(AlertSettings {
            base_url: "https://".into(),
            ..request()
        })
        .starts_with("The base URL"));
        assert_eq!(
            err(AlertSettings {
                muted_kinds: vec!["dns_record_changed.spf".into(), "dns_changed".into()],
                ..request()
            }),
            "There is no kind of alert called dns_changed."
        );
        // Off with nobody to tell is fine.
        assert!(alert_settings(&AlertSettings {
            enabled: false,
            digest: DigestFrequency::Off,
            recipients: vec![],
            base_url: String::new(),
            ..request()
        })
        .is_ok());
    }
}
