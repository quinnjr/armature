//! Error types for rate limiting

use std::time::Duration;
use thiserror::Error;

/// Result type for rate limiting operations
pub type RateLimitResult<T> = Result<T, RateLimitError>;

/// Rate limiting errors
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RateLimitError {
    /// Rate limit exceeded
    #[error("Rate limit exceeded. Retry after {retry_after:?}")]
    LimitExceeded {
        /// Remaining requests (always 0 when limit exceeded)
        remaining: u64,
        /// Total limit
        limit: u64,
        /// When the limit resets (Unix timestamp)
        reset_at: u64,
        /// Time to wait before retrying
        retry_after: Duration,
    },

    /// Store error (Redis, memory, etc.)
    #[error("Rate limit store error: {0}")]
    StoreError(String),

    /// A store operation exceeded its configured `operation_timeout`.
    ///
    /// Surfaced instead of blocking the request path indefinitely when the
    /// backing store (e.g. a partitioned Redis) stops responding. Treated as a
    /// store failure by the middleware, so `skip_on_error` decides fail-open vs
    /// fail-closed rather than the request hanging forever.
    #[error("Rate limit store operation timed out")]
    Timeout,

    /// Configuration error
    #[error("Rate limit configuration error: {0}")]
    ConfigError(String),

    /// Key extraction failed
    #[error("Failed to extract rate limit key: {0}")]
    KeyExtractionError(String),

    /// Invalid algorithm configuration
    #[error("Invalid algorithm configuration: {0}")]
    InvalidAlgorithm(String),

    /// Redis connection error
    #[cfg(feature = "redis")]
    #[error("Redis error: {0}")]
    RedisError(#[from] redis::RedisError),
}

impl RateLimitError {
    /// Create a new store error
    pub fn store<S: Into<String>>(msg: S) -> Self {
        Self::StoreError(msg.into())
    }

    /// Create a new configuration error
    pub fn config<S: Into<String>>(msg: S) -> Self {
        Self::ConfigError(msg.into())
    }

    /// Create a new key extraction error
    pub fn key_extraction<S: Into<String>>(msg: S) -> Self {
        Self::KeyExtractionError(msg.into())
    }

    /// Create a limit exceeded error
    pub fn limit_exceeded(limit: u64, reset_at: u64, retry_after: Duration) -> Self {
        Self::LimitExceeded {
            remaining: 0,
            limit,
            reset_at,
            retry_after,
        }
    }

    /// Check if this error is a rate limit exceeded error
    pub fn is_limit_exceeded(&self) -> bool {
        matches!(self, Self::LimitExceeded { .. })
    }

    /// Get the retry-after duration if this is a limit exceeded error
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::LimitExceeded { retry_after, .. } => Some(*retry_after),
            _ => None,
        }
    }

    /// Get rate limit headers for HTTP response
    pub fn headers(&self) -> Option<RateLimitHeaders> {
        match self {
            Self::LimitExceeded {
                remaining,
                limit,
                reset_at,
                retry_after,
            } => Some(RateLimitHeaders {
                limit: *limit,
                remaining: *remaining,
                reset: *reset_at,
                retry_after: Some(retry_after.as_secs()),
            }),
            _ => None,
        }
    }
}

/// Standard rate limit headers
#[derive(Debug, Clone)]
pub struct RateLimitHeaders {
    /// X-RateLimit-Limit: Maximum requests allowed
    pub limit: u64,
    /// X-RateLimit-Remaining: Requests remaining in current window
    pub remaining: u64,
    /// X-RateLimit-Reset: Unix timestamp when the limit resets
    pub reset: u64,
    /// Retry-After: Seconds until the client should retry (only when limited)
    pub retry_after: Option<u64>,
}

impl RateLimitHeaders {
    /// Create headers for an allowed request
    pub fn allowed(limit: u64, remaining: u64, reset: u64) -> Self {
        Self {
            limit,
            remaining,
            reset,
            retry_after: None,
        }
    }

    /// Create headers for a denied request
    pub fn denied(limit: u64, reset: u64, retry_after: u64) -> Self {
        Self {
            limit,
            remaining: 0,
            reset,
            retry_after: Some(retry_after),
        }
    }

    /// Get header name/value pairs
    pub fn to_header_pairs(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("X-RateLimit-Limit", self.limit.to_string()),
            ("X-RateLimit-Remaining", self.remaining.to_string()),
            ("X-RateLimit-Reset", self.reset.to_string()),
        ];

        if let Some(retry) = self.retry_after {
            headers.push(("Retry-After", retry.to_string()));
        }

        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_exceeded_error() {
        let error = RateLimitError::limit_exceeded(100, 1234567890, Duration::from_secs(30));

        assert!(error.is_limit_exceeded());
        assert_eq!(error.retry_after(), Some(Duration::from_secs(30)));

        let headers = error.headers().unwrap();
        assert_eq!(headers.limit, 100);
        assert_eq!(headers.remaining, 0);
        assert_eq!(headers.reset, 1234567890);
        assert_eq!(headers.retry_after, Some(30));
    }

    #[test]
    fn test_store_error() {
        let error = RateLimitError::store("connection failed");
        assert!(!error.is_limit_exceeded());
        assert_eq!(error.retry_after(), None);
        assert!(error.headers().is_none());
    }

    #[test]
    fn test_headers_to_pairs() {
        let headers = RateLimitHeaders::denied(100, 1234567890, 30);
        let pairs = headers.to_header_pairs();

        assert_eq!(pairs.len(), 4);
        assert!(
            pairs
                .iter()
                .any(|(k, v)| *k == "X-RateLimit-Limit" && v == "100")
        );
        assert!(
            pairs
                .iter()
                .any(|(k, v)| *k == "X-RateLimit-Remaining" && v == "0")
        );
        assert!(pairs.iter().any(|(k, v)| *k == "Retry-After" && v == "30"));
    }
}
