//! Error types for OpenTelemetry integration

use thiserror::Error;

/// Result type alias for telemetry operations
pub type TelemetryResult<T> = Result<T, TelemetryError>;

/// Telemetry error types
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// Trace error
    #[error("Trace error: {0}")]
    Trace(#[from] opentelemetry_sdk::error::OTelSdkError),

    /// Metrics error
    #[error("Metrics error: {0}")]
    Metrics(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Exporter error
    #[error("Exporter error: {0}")]
    Exporter(String),

    /// Initialization error
    #[error("Initialization error: {0}")]
    Initialization(String),

    /// Shutdown error
    #[error("Shutdown error: {0}")]
    Shutdown(String),

    /// Generic error
    #[error("Telemetry error: {0}")]
    Other(String),
}

impl From<String> for TelemetryError {
    fn from(s: String) -> Self {
        TelemetryError::Other(s)
    }
}

impl From<&str> for TelemetryError {
    fn from(s: &str) -> Self {
        TelemetryError::Other(s.to_string())
    }
}
