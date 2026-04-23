//! Analytics error types

use thiserror::Error;

/// Errors that can occur in the analytics module
#[derive(Debug, Error)]
pub enum AnalyticsError {
    /// Analytics is disabled
    #[error("Analytics is disabled")]
    Disabled,

    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Export error
    #[error("Export error: {0}")]
    Export(String),

    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),
}

impl From<serde_json::Error> for AnalyticsError {
    fn from(err: serde_json::Error) -> Self {
        AnalyticsError::Serialization(err.to_string())
    }
}
