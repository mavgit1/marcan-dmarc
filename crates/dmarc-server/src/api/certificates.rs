//! A Microsoft login's certificate: `POST /api/accounts/certificate`
//! generates a key pair and a self-signed certificate for a form not saved
//! yet (held in memory under a draft id the save request names, for
//! [`DRAFT_TTL`](crate::accounts::certificate::DRAFT_TTL)), which the save
//! stores with the account; `GET /api/accounts/{id}/certificate.cer`
//! downloads a saved account's certificate to upload it to the app
//! registration, and `GET /api/accounts/certificate/{draft_id}` an unsaved
//! one's (to renew without a gap: upload the next one before saving it).
//! The private key never leaves the server.

use crate::error::{AppError, AppResult};
use crate::extract::Json;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use dmarc_store::{get_account, unix_now};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

/// `POST /api/accounts/certificate`.
#[derive(Debug, Default, Deserialize, TS)]
#[ts(export)]
pub struct GenerateCertificateRequest {
    /// The certificate's name: the form's address, else its account name.
    #[serde(default)]
    pub subject: String,
}

/// A certificate generated for a form not saved yet.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
pub struct CertificateDraft {
    /// What the save request names (`login.draft_id`), until the save.
    pub draft_id: String,
    /// SHA-1, upper-case hex, as the Entra portal lists it once uploaded.
    pub thumbprint: String,
    /// Unix seconds.
    pub expires_at: i64,
    pub subject: String,
}

/// Generate a key pair and its certificate (off the async threads: an RSA
/// key takes a moment) and keep them for the form.
pub async fn generate_certificate(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GenerateCertificateRequest>,
) -> AppResult<Json<CertificateDraft>> {
    let subject = request.subject.trim().to_owned();
    let cert = tokio::task::spawn_blocking(move || {
        crate::accounts::certificate::generate(&subject, unix_now())
    })
    .await
    .map_err(|e| AppError::internal(format!("certificate generation stopped: {e}")))?
    .map_err(|e| {
        tracing::error!(error = %e, "certificate generation failed");
        AppError::internal("The certificate could not be generated.")
    })?;
    let answer = CertificateDraft {
        draft_id: String::new(),
        thumbprint: cert.thumbprint.clone(),
        expires_at: cert.expires_at,
        subject: cert.subject.clone(),
    };
    let draft_id = state.accounts.drafts().insert(cert);
    Ok(Json(CertificateDraft { draft_id, ..answer }))
}

/// The file name a certificate downloads as: `marcan-<name>.cer`, the name
/// reduced to letters, digits, dots and dashes.
fn filename(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "marcan.cer".to_owned()
    } else {
        format!("marcan-{slug}.cer")
    }
}

/// `der` as a certificate download (`application/pkix-cert`, RFC 2585).
fn certificate_file(der: Vec<u8>, name: &str) -> Response {
    let mut response = (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/pkix-cert"),
        )],
        der,
    )
        .into_response();
    let headers = response.headers_mut();
    if let Ok(value) =
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename(name)))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `GET /api/accounts/certificate/{draft_id}`: an unsaved form's certificate.
pub async fn download_draft_certificate(
    State(state): State<Arc<AppState>>,
    Path(draft_id): Path<String>,
) -> AppResult<Response> {
    let cert = state
        .accounts
        .drafts()
        .get(&draft_id)
        .ok_or_else(|| AppError::not_found(crate::wording::microsoft::DRAFT_GONE))?;
    Ok(certificate_file(cert.cert_der, &cert.subject))
}

/// `GET /api/accounts/{id}/certificate.cer`: a saved account's certificate.
pub async fn download_certificate(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> AppResult<Response> {
    let account = get_account(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("No such account."))?;
    let der = account
        .fields
        .microsoft
        .cert_der
        .ok_or_else(|| AppError::not_found("The account has no certificate."))?;
    let name = match account.fields.address.as_str() {
        "" => account.fields.name,
        address => address.to_owned(),
    };
    Ok(certificate_file(der, &name))
}

#[cfg(test)]
mod tests {
    use super::filename;

    #[test]
    fn the_file_is_named_after_the_account() {
        assert_eq!(
            filename("dmarc@example.com"),
            "marcan-dmarc-example.com.cer"
        );
        assert_eq!(filename("Reports (EU)"), "marcan-reports--eu.cer");
        assert_eq!(filename(""), "marcan.cer");
        assert_eq!(filename("\"/"), "marcan.cer");
    }
}
