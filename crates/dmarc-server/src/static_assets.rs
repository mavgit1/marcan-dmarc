//! Embedded static asset serving for the SvelteKit SPA.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::borrow::Cow;

/// The built UI (`pnpm build` in `web/`). Release builds require it; a debug
/// build may run without it, because in development Vite serves the UI.
#[derive(RustEmbed)]
#[folder = "../../web/build/"]
#[cfg_attr(debug_assertions, allow_missing = true)]
pub struct Assets;

const HTML: &str = "text/html; charset=utf-8";
const NO_CACHE: &str = "no-cache";

fn respond(
    status: StatusCode,
    content_type: &str,
    cache_control: &str,
    body: impl Into<Body>,
) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (header::CACHE_CONTROL, cache_control.to_owned()),
        ],
        body.into(),
    )
        .into_response()
}

fn file_body(data: Cow<'static, [u8]>) -> Body {
    match data {
        Cow::Borrowed(bytes) => Body::from(bytes),
        Cow::Owned(bytes) => Body::from(bytes),
    }
}

/// Fallback handler serving embedded static files or client-side SPA routing (`index.html`).
pub async fn serve_asset(uri: Uri) -> Response {
    let path = uri.path();

    // API routes that reach the fallback are missing endpoints: return JSON 404.
    if path.starts_with("/api/") || path == "/api" {
        return respond(
            StatusCode::NOT_FOUND,
            "application/json",
            NO_CACHE,
            r#"{"error":"not found"}"#,
        );
    }

    let trimmed = path.trim_start_matches('/');
    if !trimmed.is_empty() {
        if let Some(file) = Assets::get(trimmed) {
            let (content_type, cache_control) = if trimmed == "index.html" {
                (HTML.to_owned(), NO_CACHE)
            } else {
                let mime = mime_guess::from_path(trimmed)
                    .first_or_octet_stream()
                    .to_string();
                let cache = if trimmed.starts_with("_app/immutable/") {
                    "public, max-age=31536000, immutable"
                } else {
                    "public, max-age=3600"
                };
                (mime, cache)
            };
            return respond(
                StatusCode::OK,
                &content_type,
                cache_control,
                file_body(file.data),
            );
        }
    }

    // Anything else: index.html for SPA client-side routing.
    match Assets::get("index.html") {
        Some(index) => respond(StatusCode::OK, HTML, NO_CACHE, file_body(index.data)),
        None => respond(
            StatusCode::NOT_FOUND,
            "text/plain; charset=utf-8",
            NO_CACHE,
            "index.html not found",
        ),
    }
}
