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

/// Parse a `Retry-After` header value expressed in delta-seconds.
///
/// RFC 9110 also permits an HTTP-date form; push services in practice send
/// delta-seconds, and a date we cannot parse simply falls back to the caller's
/// default rather than fabricating a duration.
#[cfg(any(feature = "web-push", feature = "fcm", feature = "apns"))]
pub(crate) fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Map a non-success HTTP status onto a `PushError`, uniformly across every
/// provider.
///
/// Before this existed each provider mapped statuses ad hoc, so `413` and
/// `404` produced a structured error on Web Push but an opaque
/// `PushError::Provider` on FCM and APNS — meaning `should_remove_device()`
/// and `is_retryable()` answered differently for the same upstream condition
/// depending on which provider you happened to be talking to.
///
/// `subject` is what gets embedded in the device-removal errors. Callers pass
/// the device token where the token is a provider-issued opaque ID (FCM,
/// APNS); the Web Push caller passes a fixed description instead, because its
/// "token" is an attacker-supplied endpoint URL that must not be echoed into
/// error text or logs.
#[cfg(any(feature = "web-push", feature = "fcm", feature = "apns"))]
pub(crate) fn map_status(
    provider: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    subject: &str,
    payload_size: usize,
    payload_limit: usize,
) -> PushError {
    match status.as_u16() {
        401 | 403 => PushError::Auth(format!(
            "{provider} rejected the request credentials with {status}"
        )),
        404 | 410 => PushError::Unregistered(subject.to_string()),
        413 => PushError::PayloadTooLarge {
            size: payload_size,
            limit: payload_limit,
        },
        429 => PushError::RateLimited(retry_after_secs(headers).unwrap_or(60)),
        _ => PushError::Provider(format!("{provider} error {status}")),
    }
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
            // `web_push::WebPushError` doesn't carry the offending payload's
            // size, so we can't report a real number here (the reqwest-based
            // send path in `web_push.rs` does have the real size and uses it
            // directly rather than going through this `From` impl). Report
            // via `Provider` instead of fabricating a `PayloadTooLarge { size:
            // 0, .. }`, which would read as "0 bytes exceeds the limit" — a
            // contradiction in the error message itself.
            Self::Provider(err_string)
        } else {
            Self::Provider(err_string)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the shared status mapper, which only exists when at least one
    /// provider is compiled in.
    #[cfg(any(feature = "web-push", feature = "fcm", feature = "apns"))]
    mod status_mapping {
        use super::super::*;
        use reqwest::StatusCode;
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

        fn headers_with_retry_after(value: &str) -> HeaderMap {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            headers
        }

        #[test]
        fn map_status_413_is_payload_too_large_with_real_size() {
            let err = map_status(
                "test",
                StatusCode::PAYLOAD_TOO_LARGE,
                &HeaderMap::new(),
                "device-token",
                9000,
                4096,
            );
            match err {
                PushError::PayloadTooLarge { size, limit } => {
                    assert_eq!(size, 9000);
                    assert_eq!(limit, 4096);
                }
                other => panic!("expected PayloadTooLarge, got {other:?}"),
            }
        }

        #[test]
        fn map_status_404_and_410_are_unregistered() {
            for status in [StatusCode::NOT_FOUND, StatusCode::GONE] {
                let err = map_status("test", status, &HeaderMap::new(), "device-token", 0, 4096);
                assert!(
                    matches!(&err, PushError::Unregistered(s) if s == "device-token"),
                    "expected Unregistered for {status}, got {err:?}"
                );
                assert!(err.should_remove_device());
            }
        }

        #[test]
        fn map_status_401_and_403_are_auth() {
            for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
                let err = map_status("test", status, &HeaderMap::new(), "device-token", 0, 4096);
                assert!(
                    matches!(err, PushError::Auth(_)),
                    "expected Auth for {status}, got {err:?}"
                );
            }
        }

        #[test]
        fn map_status_429_reads_retry_after_header() {
            let err = map_status(
                "test",
                StatusCode::TOO_MANY_REQUESTS,
                &headers_with_retry_after("42"),
                "device-token",
                0,
                4096,
            );
            assert!(matches!(err, PushError::RateLimited(42)), "got {err:?}");
            assert_eq!(err.retry_after(), Some(std::time::Duration::from_secs(42)));
        }

        #[test]
        fn map_status_429_without_retry_after_defaults_to_60() {
            let err = map_status(
                "test",
                StatusCode::TOO_MANY_REQUESTS,
                &HeaderMap::new(),
                "device-token",
                0,
                4096,
            );
            assert!(matches!(err, PushError::RateLimited(60)), "got {err:?}");
        }

        #[test]
        fn map_status_429_with_http_date_retry_after_falls_back() {
            // An HTTP-date Retry-After is legal but not delta-seconds; we must fall
            // back to the default rather than fabricating a parsed value.
            let err = map_status(
                "test",
                StatusCode::TOO_MANY_REQUESTS,
                &headers_with_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
                "device-token",
                0,
                4096,
            );
            assert!(matches!(err, PushError::RateLimited(60)), "got {err:?}");
        }

        #[test]
        fn map_status_other_is_provider_and_does_not_echo_subject() {
            let err = map_status(
                "test",
                StatusCode::INTERNAL_SERVER_ERROR,
                &HeaderMap::new(),
                "https://attacker.example/secret-endpoint",
                0,
                4096,
            );
            assert!(matches!(err, PushError::Provider(_)), "got {err:?}");
            assert!(
                !err.to_string().contains("attacker.example"),
                "provider errors must not echo the subject: {err}"
            );
        }
    } // mod status_mapping

    #[test]
    fn auth_is_not_retryable_and_does_not_remove_device() {
        let err = PushError::Auth("bad key".to_string());
        assert!(!err.is_retryable());
        assert!(!err.should_remove_device());
    }

    #[test]
    fn timeout_is_retryable() {
        assert!(PushError::Timeout.is_retryable());
    }

    #[cfg(feature = "web-push")]
    #[test]
    fn web_push_payload_too_large_string_maps_to_provider_not_payload_too_large() {
        // `WebPushError` carries no size, so the `From` impl deliberately
        // reports `Provider` rather than a self-contradictory
        // `PayloadTooLarge { size: 0 }`. This pins that remap, which is
        // behavior-changing for callers matching on `PayloadTooLarge`.
        let err = PushError::from(web_push::WebPushError::PayloadTooLarge);
        assert!(
            err.to_string().to_lowercase().contains("payload"),
            "test fixture should exercise the payload branch: {err}"
        );
        assert!(
            matches!(err, PushError::Provider(_)),
            "expected Provider, got {err:?}"
        );
    }
}
