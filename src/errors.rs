use thiserror::Error;
use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;
use sqlx::Error as SqlxError;
use validator::ValidationErrors;

/// Application-wide error types
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Authorization failed: {0}")]
    Authorization(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict error: {0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("AI service error: {0}")]
    AIError(String),

    #[error("File operation error: {0}")]
    FileError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Rate limit exceeded")]
    RateLimitError,

    #[error("Internal server error: {0}")]
    InternalError(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("QC Gate error: {0}")]
    QCGateError(String),

    #[error("Project locked: {0}")]
    ProjectLockedError(String),
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub code: String,
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        let (status, error_code) = match self {
            AppError::Authentication(_) => (StatusCode::UNAUTHORIZED, "AUTH_ERROR"),
            AppError::Authorization(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            AppError::Validation(_) => (StatusCode::BAD_REQUEST, "VALIDATION_ERROR"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            AppError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "DATABASE_ERROR"),
            AppError::ExternalService(_) => (StatusCode::BAD_GATEWAY, "EXTERNAL_SERVICE_ERROR"),
            AppError::AIError(_) => (StatusCode::BAD_GATEWAY, "AI_SERVICE_ERROR"),
            AppError::FileError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "FILE_ERROR"),
            AppError::ConfigError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "CONFIG_ERROR"),
            AppError::RateLimitError => (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMIT"),
            AppError::InternalError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::QCGateError(_) => (StatusCode::UNPROCESSABLE_ENTITY, "QC_GATE_ERROR"),
            AppError::ProjectLockedError(_) => (StatusCode::LOCKED, "PROJECT_LOCKED"),
        };

        let response = ErrorResponse {
            error: error_code.to_string(),
            message: self.to_string(),
            details: None,
            code: error_code.to_string(),
        };

        HttpResponse::build(status).json(response)
    }
}

impl From<SqlxError> for AppError {
    fn from(err: SqlxError) -> Self {
        match err {
            SqlxError::RowNotFound => AppError::NotFound("Record not found".to_string()),
            SqlxError::Database(db_err) => {
                let message = db_err.message();
                if message.contains("unique constraint") || message.contains("duplicate key") {
                    AppError::Conflict("Resource already exists".to_string())
                } else if message.contains("foreign key") {
                    AppError::Validation("Referenced resource does not exist".to_string())
                } else {
                    AppError::Database(message.to_string())
                }
            }
            _ => AppError::Database(err.to_string()),
        }
    }
}

impl From<ValidationErrors> for AppError {
    fn from(err: ValidationErrors) -> Self {
        let errors: Vec<String> = err
            .field_errors()
            .into_iter()
            .flat_map(|(field, errors)| {
                errors.iter().map(move |e| {
                    format!(
                        "{}: {}",
                        field,
                        e.message.as_ref().map(|m| m.to_string()).unwrap_or_else(|| "Invalid value".to_string())
                    )
                })
            })
            .collect();
        AppError::Validation(errors.join(", "))
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        AppError::Authentication(format!("Token error: {}", err))
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::FileError(err.to_string())
    }
}

impl From<config::ConfigError> for AppError {
    fn from(err: config::ConfigError) -> Self {
        AppError::ConfigError(err.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::ExternalService(err.to_string())
    }
}

/// Helper type alias for Result with AppError
pub type AppResult<T> = Result<T, AppError>;
