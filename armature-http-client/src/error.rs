//! HTTP Client error types.

use std::time::Duration;
use thiserror::Error;

/// Result type for HTTP client operations.
pub type Result<T> = std::result::Result<T, HttpClientError>;

/// HTTP client errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HttpClientError {
    /// Request failed after all retries exhausted.
    #[error("Request failed after {attempts} attempts: {message}")]
    RetryExhausted {
        /// Number of attempts made.
        attempts: u32,
        /// Last error message.
        message: String,
    },

    /// Circuit breaker is open, rejecting requests.
    #[error("Circuit breaker is open, request rejected")]
    CircuitOpen,

    /// Request timed out.
    #[error("Request timed out after {0:?}")]
    Timeout(Duration),

    /// Connection error.
    #[error("Connection error: {0}")]
    Connection(String),

    /// Invalid URL.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Request building error.
    #[error("Failed to build request: {0}")]
    RequestBuild(String),

    /// Response error.
    #[error("Response error: {status} - {message}")]
    Response {
        /// HTTP status code.
        status: u16,
        /// Error message.
        message: String,
    },

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(String),

    /// Interceptor error.
    #[error("Interceptor error: {0}")]
    Interceptor(String),

    /// A request interceptor installed a streaming (non-buffered) body that
    /// cannot be replayed, but retries are configured. Such a body would be
    /// consumed by the first attempt and silently sent empty on every retry,
    /// so the request is rejected instead of sending an incomplete request.
    #[error(
        "streaming body set by interceptor is not retry-safe: the body cannot \
         be buffered/replayed for retry attempts (disable retries or have the \
         interceptor set a buffered body)"
    )]
    StreamingBodyNotRetryable,

    /// Underlying HTTP client error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// URL parsing error.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl HttpClientError {
    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout(_) => true,
            Self::Connection(_) => true,
            Self::Http(e) => e.is_timeout() || e.is_connect(),
            Self::Response { status, .. } => {
                // Retry on 5xx server errors and 429 rate limit
                *status >= 500 || *status == 429
            }
            _ => false,
        }
    }

    /// Check if this is a timeout error.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_)) || matches!(self, Self::Http(e) if e.is_timeout())
    }

    /// Check if this is a connection error.
    pub fn is_connection(&self) -> bool {
        matches!(self, Self::Connection(_)) || matches!(self, Self::Http(e) if e.is_connect())
    }

    /// Get the HTTP status code if this is a response error.
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Response { status, .. } => Some(*status),
            Self::Http(e) => e.status().map(|s| s.as_u16()),
            _ => None,
        }
    }
}
