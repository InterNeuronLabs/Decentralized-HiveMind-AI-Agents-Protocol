// orchestrator/src/error.rs
// Centralised error type that converts to Axum HTTP responses.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("rate limited")]
    RateLimited,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("identity error: {0}")]
    Identity(#[from] common::identity::IdentityError),
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Never leak internal error details to the caller.
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".into()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".into()),
            AppError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate limited".into()),
            AppError::Database(_) => {
                tracing::error!(error = %self, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
            AppError::Redis(_) => {
                tracing::error!(error = %self, "redis error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
            _ => {
                tracing::error!(error = %self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
