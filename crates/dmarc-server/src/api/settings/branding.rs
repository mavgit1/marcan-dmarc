//! The branding group: the operator's name (`PUT /api/settings/branding`)
//! and logo (`POST` / `DELETE /api/settings/branding/logo`), shown in the
//! emails only, and how the emails look with a draft of both
//! (`POST /api/settings/branding/preview`). Free for everyone: saving never
//! looks at the operator key; only the preview reads it, to show the emails
//! as they go out now and in the other license state.
//!
//! The logo is uploaded as the raw image in the request body, with an
//! `image/*` content type (a form upload is refused, like every POST that is
//! not JSON: a form is what another site can post). The type is read from the
//! image's first bytes, never from the declared one: PNG, JPEG or WebP, at
//! most [`LOGO_MAX_BYTES`]. It is stored in its own table, apart from the
//! settings, with its size in pixels read from its header
//! ([`mail::image_size`]), from which the emails size it.

use super::settings_response;
use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::license::Standing;
use crate::mail;
use crate::mail::client_report::{self, ReportMonth};
use crate::view::settings::{EmailLook, EmailPreview, SettingsResponse};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use dmarc_store::{
    get_logo_info, save_settings, set_logo, AppSettings, BrandingSettings, LogoType, PixelSize,
    StoredLogo,
};
use serde::Deserialize;
use std::sync::Arc;
use ts_rs::TS;

/// The longest operator name: it has to fit the header and a From line.
pub const OPERATOR_NAME_MAX_CHARS: usize = 80;
/// The largest logo: small enough for every email it goes into.
pub const LOGO_MAX_BYTES: usize = 200 * 1024;

/// The operator name as entered, validated; `None` when blank.
pub(crate) fn operator_name(raw: &str) -> AppResult<Option<String>> {
    let name = raw.trim();
    if name.chars().any(char::is_control) {
        return Err(AppError::unprocessable(
            "The operator name must be one line of text.",
        ));
    }
    if name.chars().count() > OPERATOR_NAME_MAX_CHARS {
        return Err(AppError::unprocessable(format!(
            "The operator name can be at most {OPERATOR_NAME_MAX_CHARS} characters."
        )));
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// The image format of `bytes`, from its first bytes (its signature).
pub(crate) fn sniff_logo(bytes: &[u8]) -> Option<LogoType> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(PNG) {
        Some(LogoType::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(LogoType::Jpeg)
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(LogoType::Webp)
    } else {
        None
    }
}

/// `n` bytes in whole kilobytes, rounded up ("1 KB" for anything below).
fn kilobytes(n: usize) -> usize {
    n.div_ceil(1024).max(1)
}

/// The uploaded image as it is stored, checked by its bytes.
pub(crate) fn stored_logo(bytes: &[u8]) -> AppResult<StoredLogo> {
    if bytes.is_empty() {
        return Err(AppError::unprocessable("The logo file is empty."));
    }
    if bytes.len() > LOGO_MAX_BYTES {
        return Err(AppError::unprocessable(format!(
            "The logo is {} KB; it can be at most {} KB. Save it smaller or as a JPEG or WebP.",
            kilobytes(bytes.len()),
            LOGO_MAX_BYTES / 1024
        )));
    }
    let logo_type = sniff_logo(bytes)
        .ok_or_else(|| AppError::unprocessable("The logo must be a PNG, JPEG or WebP image."))?;
    let size = mail::image_size::pixel_size(bytes).ok_or_else(|| {
        AppError::unprocessable("The logo's size could not be read. Save it again as a PNG.")
    })?;
    Ok(StoredLogo {
        logo_type,
        bytes: bytes.to_vec(),
        size,
    })
}

/// Store the operator name; the logo stays. Answers every setting.
/// `PUT /api/settings/branding`: the operator name ([`BrandingSettings`]). The
/// logo has its own endpoint and is kept.
pub async fn put_branding(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BrandingSettings>,
) -> AppResult<Json<SettingsResponse>> {
    let operator_name = operator_name(&payload.operator_name)?;
    let operator_name = operator_name.unwrap_or_default();
    save_settings(&state.pool, &BrandingSettings { operator_name }).await?;
    settings_response(&state).await
}

/// `POST /api/settings/branding/logo`: replace the logo with the image in the
/// body. Answers every setting; 422 with the reason when the image is refused.
pub async fn upload_logo(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> AppResult<Json<SettingsResponse>> {
    set_logo(&state.pool, Some(&stored_logo(&body)?)).await?;
    settings_response(&state).await
}

/// `POST /api/settings/branding/preview`: the Branding page's draft, not
/// stored.
#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct BrandingPreviewRequest {
    /// As typed; blank: none.
    pub operator_name: String,
    /// The draft has a logo (the stored one, or one picked but not uploaded yet).
    pub has_logo: bool,
    /// The picked logo's size in pixels, as the browser read it; `null`
    /// for the stored logo, whose size the server knows.
    #[ts(optional = nullable)]
    #[serde(default)]
    pub logo_size: Option<PixelSize>,
}

/// The organization the preview's sample title names.
const PREVIEW_ORGANIZATION: &str = "Example Ltd";

/// How an email looks with `name` and a logo of that size or none, with the
/// notice shown or not: the code the emails use ([`mail::from_name`],
/// [`mail::email_top`], [`mail::sign_off`]) under a client report's title.
pub(crate) fn email_look(
    name: Option<&str>,
    logo: Option<PixelSize>,
    notice: bool,
    now: i64,
) -> EmailLook {
    EmailLook {
        notice_shown: notice,
        from_name: mail::from_name(name, notice),
        top: mail::email_top(name, logo, notice),
        title: client_report::title(PREVIEW_ORGANIZATION, &ReportMonth::before(now)),
        sign_off: mail::sign_off(name, notice),
    }
}

/// How the emails look with the draft's name and logo, as they go out now
/// (the stored license) and in the other license state. Stores nothing; 422
/// for a name that could not be saved.
pub async fn preview_branding(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BrandingPreviewRequest>,
) -> AppResult<Json<EmailPreview>> {
    let name = operator_name(&payload.operator_name)?;
    let settings = AppSettings::load(&state.pool).await?;
    let notice_shown = Standing::new(&settings.license).notice_shown;
    // The picked logo's size, else the stored logo's.
    let logo = if payload.has_logo {
        match payload.logo_size {
            Some(size) => Some(size),
            None => get_logo_info(&state.pool).await?.map(|l| l.size),
        }
    } else {
        None
    };
    let now = dmarc_store::unix_now();
    let look = |notice| email_look(name.as_deref(), logo, notice, now);
    Ok(Json(EmailPreview {
        current: look(notice_shown),
        other: look(!notice_shown),
        notice_shown,
        operator_name: name,
    }))
}

/// `DELETE /api/settings/branding/logo`: remove the logo (none is not an
/// error). Answers every setting.
pub async fn delete_logo(State(state): State<Arc<AppState>>) -> AppResult<Json<SettingsResponse>> {
    set_logo(&state.pool, None).await?;
    settings_response(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(name: &str) -> BrandingSettings {
        BrandingSettings {
            operator_name: name.into(),
        }
    }

    #[test]
    fn the_name_is_cleaned_up_and_checked() {
        let err = |r: BrandingSettings| operator_name(&r.operator_name).unwrap_err().message;
        assert_eq!(
            operator_name(" Acme IT ").unwrap().as_deref(),
            Some("Acme IT")
        );
        assert_eq!(operator_name("  ").unwrap(), None);
        assert_eq!(
            err(request("Acme\nIT")),
            "The operator name must be one line of text."
        );
        assert_eq!(
            err(request(&"a".repeat(81))),
            "The operator name can be at most 80 characters."
        );
    }

    #[test]
    fn a_logo_is_known_by_its_bytes() {
        assert_eq!(
            sniff_logo(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
            Some(LogoType::Png)
        );
        assert_eq!(sniff_logo(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(LogoType::Jpeg));
        assert_eq!(sniff_logo(b"RIFF\x24\0\0\0WEBPVP8 "), Some(LogoType::Webp));
        // GIF, SVG, a RIFF that is a WAV, and a short PNG prefix are not.
        assert_eq!(sniff_logo(b"GIF89a"), None);
        assert_eq!(
            sniff_logo(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"),
            None
        );
        assert_eq!(sniff_logo(b"RIFF\x24\0\0\0WAVEfmt "), None);
        assert_eq!(sniff_logo(b"\x89PNG"), None);
    }

    #[test]
    fn refused_logos_say_why() {
        let err = |b: &[u8]| stored_logo(b).unwrap_err();
        let e = err(b"<svg/>");
        assert_eq!(e.message, "The logo must be a PNG, JPEG or WebP image.");
        assert_eq!(e.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(err(b"").message, "The logo file is empty.");
        let mut big = PNG.to_vec();
        big.resize(LOGO_MAX_BYTES + 1, 0);
        assert_eq!(
            err(&big).message,
            "The logo is 201 KB; it can be at most 200 KB. Save it smaller or as a JPEG or WebP."
        );
        big.truncate(LOGO_MAX_BYTES);
        assert!(stored_logo(&big).is_ok());
        // A PNG whose header states no size.
        assert_eq!(
            err(b"\x89PNG\r\n\x1a\nrest").message,
            "The logo's size could not be read. Save it again as a PNG."
        );
    }

    /// The signature and an IHDR chunk stating 640 by 160 pixels.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\x02\x80\0\0\0\xa0\x08\x06\0\0\0";

    #[test]
    fn a_stored_logo_is_its_bytes_with_its_size() {
        let logo = stored_logo(PNG).unwrap();
        assert_eq!(logo.logo_type, LogoType::Png);
        assert_eq!(logo.bytes, PNG);
        assert_eq!(
            logo.size,
            PixelSize {
                width: 640,
                height: 160
            }
        );
    }
}
