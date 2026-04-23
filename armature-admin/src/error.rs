//! Error types for admin module

use thiserror::Error;

/// Admin error types
#[derive(Error, Debug)]
pub enum AdminError {
    /// Model not found
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Record not found
    #[error("Record not found: {model}/{id}")]
    RecordNotFound { model: String, id: String },

    /// Validation error
    #[error("Validation error: {0}")]
    Validation(String),

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Database error
    #[error("Database error: {0}")]
    Database(String),

    /// Template error
    #[error("Template error: {0}")]
    Template(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),
}

impl From<serde_json::Error> for AdminError {
    fn from(err: serde_json::Error) -> Self {
        AdminError::Serialization(err.to_string())
    }
}

/// Result type for admin operations
pub type AdminResult<T> = Result<T, AdminError>;
