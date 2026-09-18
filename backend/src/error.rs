use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub error: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, error: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            error: error.into(),
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn too_many(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // 5xx responses must never expose internals (SQL text, connection
        // details, library versions). Log them server-side and return a
        // generic message to the client.
        let (status, message) = if self.status.is_server_error() {
            tracing::error!(
                status = self.status.as_u16(),
                error = %self.message,
                "request failed with internal error"
            );
            (self.status, "Internal server error".to_string())
        } else {
            (self.status, self.message.clone())
        };
        let body = Json(ErrResult {
            error: self.error,
            message,
        });
        (status, body).into_response()
    }
}

#[derive(Serialize)]
pub struct ErrResult {
    pub error: String,
    pub message: String,
}
