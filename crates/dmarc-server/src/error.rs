//! Shared HTTP error type with a consistent JSON body.

use crate::view::LineError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use ts_rs::TS;

/// Stable error codes returned in JSON bodies, one per status.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 400.
    BadRequest,
    /// 404.
    NotFound,
    /// 405.
    MethodNotAllowed,
    /// 409.
    Conflict,
    /// 415.
    UnsupportedMediaType,
    /// 422.
    Unprocessable,
    /// 500.
    Internal,
}

/// JSON error envelope: `{ "error": "...", "code": "bad_request" }`, plus
/// the fields some answers carry.
#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct ErrorBody {
    /// A sentence for the user.
    pub error: String,
    pub code: ErrorCode,
    /// A list edited as text that was refused: every line that cannot be saved.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub errors: Option<Vec<LineError>>,
    /// A failed test send: the step that failed (`resolve`, `connect`, `tls`,
    /// `starttls_not_offered`, `auth`, `rejected`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// A failed test send: the mail server's own words.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// A domain that already exists: the organization that has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub organization_name: Option<String>,
}

/// Application error mapped to HTTP status + JSON body ([`ErrorBody`]).
///
/// One rule for the status, in every handler:
///
/// - `400` ([`AppError::bad_request`]): the request is malformed. The body is
///   not JSON or lacks a field or has one of the wrong type, a query
///   parameter is missing or unreadable, a path segment is not what the
///   route takes (axum's own rejections, through [`crate::extract`]); a
///   `Host` the server does not answer to ([`crate::http_policy`]).
/// - `404` ([`AppError::not_found`]): what the request names does not exist.
/// - `409` ([`AppError::conflict`]): a valid request the current state
///   refuses. A name already taken; a feature turned on without what it
///   needs (sending without a server or a complete account, alerts without a
///   recipient, client reports with an empty recipient list); the stored
///   password reused for another account; a test or preview with sending off
///   or no test recipient.
/// - `415` ([`AppError::unsupported_media_type`]): a POST that is not JSON.
/// - `422` ([`AppError::unprocessable`]): a well-formed value that fails
///   validation. An address that is not one, a port, hour or day out of
///   range, a list with line errors, an operator key or logo that is not
///   valid; and a test send the mail server refused (with `reason`).
/// - `500` ([`AppError::internal`]): the server failed.
#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
    /// The fields some answers carry; boxed, so a handler's `Result` stays small.
    pub fields: Option<Box<ErrorFields>>,
}

/// The [`ErrorBody`] fields beyond `error` and `code`.
#[derive(Debug, Default)]
pub struct ErrorFields {
    pub errors: Option<Vec<LineError>>,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub organization_name: Option<String>,
}

impl AppError {
    fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            fields: None,
        }
    }

    fn fields(&mut self) -> &mut ErrorFields {
        self.fields.get_or_insert_with(Box::default)
    }

    /// 400, for a malformed request.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadRequest, message)
    }

    /// 422, for a well-formed value that fails validation.
    pub fn unprocessable(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Unprocessable,
            message,
        )
    }

    /// 415, for a body that is not JSON.
    pub fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ErrorCode::UnsupportedMediaType,
            message,
        )
    }

    /// 405, for a method the path does not take.
    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::MethodNotAllowed,
            message,
        )
    }

    /// 409, for a valid request the current state refuses.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            message,
        )
    }

    /// A refused list's line errors ([`ErrorBody::errors`]).
    pub fn with_line_errors(mut self, errors: Vec<LineError>) -> Self {
        self.fields().errors = Some(errors);
        self
    }

    /// A failed test send's step and the server's words ([`ErrorBody::reason`]).
    pub fn with_send_failure(mut self, reason: &str, detail: &str) -> Self {
        let fields = self.fields();
        fields.reason = Some(reason.to_owned());
        fields.detail = Some(detail.to_owned());
        self
    }

    /// The organization that already has a domain ([`ErrorBody::organization_name`]).
    pub fn with_organization_name(mut self, name: String) -> Self {
        self.fields().organization_name = Some(name);
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let fields = self.fields.map(|f| *f).unwrap_or_default();
        let body = ErrorBody {
            error: self.message,
            code: self.code,
            errors: fields.errors,
            reason: fields.reason,
            detail: fields.detail,
            organization_name: fields.organization_name,
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        Self::internal("database error")
    }
}

impl From<dmarc_store::StoreError> for AppError {
    fn from(err: dmarc_store::StoreError) -> Self {
        tracing::error!(error = %err, "store error");
        Self::internal("database error")
    }
}

/// Result alias for handlers.
pub type AppResult<T> = Result<T, AppError>;
