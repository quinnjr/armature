//! SIEM error types

use thiserror::Error;

/// SIEM-related errors
#[derive(Error, Debug)]
pub enum SiemError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection error
    #[error("Connection failed: {0}")]
    Connection(String),

    /// Transport error (sending events)
    #[error("Transport error: {0}")]
    Transport(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// HTTP error (for HEC endpoints)
    #[cfg(feature = "http")]
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// URL parse error
    #[error("Invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),

    /// Authentication error
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Rate limited
    #[error("Rate limited, retry after: {0}ms")]
    RateLimited(u64),

    /// Provider not supported
    #[error("Provider not supported: {0}")]
    UnsupportedProvider(String),

    /// Format not supported
    #[error("Format not supported: {0}")]
    UnsupportedFormat(String),

    /// Batch full
    #[error("Batch is full, max size: {0}")]
    BatchFull(usize),

    /// Timeout
    #[error("Operation timed out after {0}ms")]
    Timeout(u64),
}

/// Result type for SIEM operations
pub type SiemResult<T> = Result<T, SiemError>;
