//! Error types: `Error` for the library API, `ApiError` for HTTP responses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{json, Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tls error: {0}")]
    Tls(String),
    #[error("{0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

/// An HTTP error: status plus a short machine-readable code, optionally with extra fields.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub extra: Map<String, Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code, extra: Map::new() }
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_string(), value.into());
        self
    }

    pub fn bad_request(code: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized")
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found")
    }

    pub fn conflict(code: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, code)
    }

    /// Map a disk error to a response without leaking paths or OS messages.
    pub fn from_io(err: &std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::StorageFull {
            Self::new(StatusCode::INSUFFICIENT_STORAGE, "disk_full")
        } else {
            tracing::warn!("disk error: {err}");
            Self::new(StatusCode::INTERNAL_SERVER_ERROR, "disk_error")
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = self.extra;
        body.insert("error".into(), json!(self.code));
        (self.status, axum::Json(Value::Object(body))).into_response()
    }
}
