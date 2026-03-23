//! Error types for collaboration module

use thiserror::Error;
use uuid::Uuid;

/// Collaboration error types
#[derive(Error, Debug)]
pub enum CollabError {
    /// Document not found
    #[error("Document not found: {0}")]
    DocumentNotFound(String),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(Uuid),

    /// Invalid operation
    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    /// Operation conflict
    #[error("Operation conflict: {0}")]
    Conflict(String),

    /// Causality violation (operation depends on unknown operations)
    #[error("Causality violation: missing dependency {0}")]
    CausalityViolation(Uuid),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Sync error
    #[error("Sync error: {0}")]
    Sync(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Timeout
    #[error("Operation timed out")]
    Timeout,

    /// Permission denied
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

impl From<serde_json::Error> for CollabError {
    fn from(err: serde_json::Error) -> Self {
        CollabError::Serialization(err.to_string())
    }
}

/// Result type for collaboration operations
pub type CollabResult<T> = Result<T, CollabError>;
