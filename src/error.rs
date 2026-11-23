use axum::{
    extract::rejection::{JsonRejection, PathRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    /// Any other error status (405, 415, 422, 429, ...), e.g. a request that
    /// axum couldn't parse. Still rendered as `{"error": "..."}`.
    #[error("{1}")]
    Status(StatusCode, String),
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        let message = match &rejection {
            JsonRejection::MissingJsonContentType(_) => "Permintaan harus berformat JSON",
            JsonRejection::JsonSyntaxError(_) => "Format JSON tidak valid",
            JsonRejection::JsonDataError(_) => "Data yang dikirim tidak lengkap atau formatnya salah",
            _ => "Permintaan tidak dapat dibaca",
        };
        // axum's own explanation is English and technical: keep it for the log only.
        tracing::debug!("Rejected request body: {}", rejection.body_text());
        AppError::Status(rejection.status(), message.to_string())
    }
}

impl From<PathRejection> for AppError {
    fn from(rejection: PathRejection) -> Self {
        tracing::debug!("Rejected request path: {}", rejection.body_text());
        AppError::Status(rejection.status(), "Alamat (URL) tidak valid".to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Database(e) => {
                // A conflict with a concurrent request is not a fault: the request
                // is run again (see db::request_txn)
                if crate::db::request_txn::is_conflict(e) {
                    tracing::debug!("Database conflict: {:?}", e);
                } else {
                    tracing::error!("Database error: {:?}", e);
                }
                (StatusCode::INTERNAL_SERVER_ERROR, "Terjadi kesalahan pada server".to_string())
            }
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, "Terjadi kesalahan pada server".to_string())
            }
            AppError::Status(status, msg) => (*status, msg.clone()),
        };

        let body = Json(json!({
            "error": message
        }));

        (status, body).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
