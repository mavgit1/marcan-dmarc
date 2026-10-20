//! The extractors every handler uses: axum's [`Json`], [`Query`] and
//! [`Path`], with a rejection (a body that is not JSON or lacks a field, a
//! missing or mistyped query parameter, a path segment of the wrong type)
//! answered as an [`AppError`], so it is a JSON [`ErrorBody`](crate::error::ErrorBody)
//! like every other error instead of axum's plain text. A malformed request
//! is a `400` (see [`AppError`]); a POST that is not JSON a `415`.

use crate::error::AppError;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// The error answer of one of axum's rejections, by its status and text.
fn rejected(status: StatusCode, text: String) -> AppError {
    if status.is_server_error() {
        tracing::error!(%status, error = %text, "request could not be read");
        AppError::internal(text)
    } else if status == StatusCode::UNSUPPORTED_MEDIA_TYPE {
        AppError::unsupported_media_type(text)
    } else {
        AppError::bad_request(text)
    }
}

/// A JSON body ([`axum::Json`]); also the JSON answer of a handler.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<S, T> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(json_rejected(&rejection)),
        }
    }
}

fn json_rejected(rejection: &JsonRejection) -> AppError {
    rejected(rejection.status(), rejection.body_text())
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// The query string ([`axum::extract::Query`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<S, T> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(query_rejected(&rejection)),
        }
    }
}

fn query_rejected(rejection: &QueryRejection) -> AppError {
    rejected(rejection.status(), rejection.body_text())
}

/// The route's path parameters ([`axum::extract::Path`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct Path<T>(pub T);

impl<S, T> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Self(value)),
            Err(rejection) => Err(path_rejected(&rejection)),
        }
    }
}

fn path_rejected(rejection: &PathRejection) -> AppError {
    rejected(rejection.status(), rejection.body_text())
}
