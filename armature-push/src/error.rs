//! Push notification error types.

use thiserror::Error;

/// Result type for push operations.
pub type Result<T> = std::result::Result<T, PushError>;

/// Push notification errors.
#[derive(Debug, Error)]
pub enum PushError {
    /// Invalid subscription or device token.
    #[error("Invalid subscription: {0}")]
    InvalidSubscription(String),

    /// Device token expired or invalid.
    #[error("Device token expired or invalid: {0}")]
    TokenExpired(String),

    /// Device unregistered.
    #[error("Device unregistered: {0}")]
    Unregistered(String),

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Rate limited.
    #[error("Rate limited, retry after {0} seconds")]
    RateLimited(u64),

    /// Payload too large.
    #[error("Payload too large: {size} bytes exceeds limit of {limit} bytes")]
    PayloadTooLarge {
        /// Actual size.
        size: usize,
        /// Maximum allowed size.
        limit: usize,
    },

    /// Provider error (FCM, APNS, Web Push).
    #[error("Provider error: {0}")]
    Provider(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Network error.
    #[error("Network error: {0}")]
    Network(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Timeout error.
    #[error("Operation timed out")]
    Timeout,

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl PushError {
    /// Check if this error indicates the device should be removed.
    pub fn should_remove_device(&self) -> bool {
        matches!(
            self,
            Self::TokenExpired(_) | Self::Unregistered(_) | Self::InvalidSubscription(_)
        )
    }

    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Network(_) | Self::Timeout | Self::RateLimited(_)
        )
    }

    /// Get retry-after duration if rate limited.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        if let Self::RateLimited(secs) = self {
            Some(std::time::Duration::from_secs(*secs))
        } else {
            None
        }
    }
}

impl From<reqwest::Error> for PushError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_connect() {
            Self::Network(err.to_string())
        } else {
            Self::Provider(err.to_string())
        }
    }
}

impl From<serde_json::Error> for PushError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(feature = "web-push")]
impl From<web_push::WebPushError> for PushError {
    fn from(err: web_push::WebPushError) -> Self {
        // Map web_push errors to our error types based on error message content
        let err_string = err.to_string();
        if err_string.contains("endpoint") || err_string.contains("invalid") {
            Self::InvalidSubscription(err_string)
        } else if err_string.contains("expired")
            || err_string.contains("unsubscribed")
            || err_string.contains("gone")
        {
            Self::Unregistered(err_string)
        } else if err_string.contains("rate") || err_string.contains("429") {
            Self::RateLimited(60)
        } else if err_string.contains("payload") || err_string.contains("too large") {
            Self::PayloadTooLarge {
                size: 0,
                limit: crate::web_push::WEB_PUSH_MAX_PAYLOAD,
            }
        } else {
            Self::Provider(err_string)
        }
    }
}
