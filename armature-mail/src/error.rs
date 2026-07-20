//! Mail error types.

use thiserror::Error;

/// Result type for mail operations.
pub type Result<T> = std::result::Result<T, MailError>;

/// Mail errors.
#[derive(Debug, Error)]
pub enum MailError {
    /// SMTP connection error.
    #[error("SMTP error: {0}")]
    Smtp(String),

    /// Invalid email address.
    #[error("Invalid email address: {0}")]
    InvalidAddress(String),

    /// Missing required field.
    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    /// Template error.
    #[error("Template error: {0}")]
    Template(String),

    /// Template not found.
    #[error("Template not found: {0}")]
    TemplateNotFound(String),

    /// Attachment error.
    #[error("Attachment error: {0}")]
    Attachment(String),

    /// Configuration error.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Provider API error.
    ///
    /// `status` carries the HTTP status the provider replied with, when there
    /// was one. It is load-bearing: every API transport funnels its non-429
    /// failures through this variant, so without the status a transient 503 is
    /// indistinguishable from a permanent 400 and
    /// [`MailError::is_retryable`] has to guess.
    #[error("Provider error{}: {message}", .status.map(|s| format!(" ({s})")).unwrap_or_default())]
    Provider {
        /// HTTP status returned by the provider, if any.
        status: Option<u16>,
        /// Provider-supplied error message.
        message: String,
    },

    /// Authentication error.
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// Rate limited.
    #[error("Rate limited, retry after {0} seconds")]
    RateLimited(u64),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Network error.
    #[error("Network error: {0}")]
    Network(String),

    /// Timeout error.
    #[error("Operation timed out")]
    Timeout,

    /// Queue error.
    #[error("Queue error: {0}")]
    Queue(String),
}

impl MailError {
    /// Build a [`MailError::Provider`] from an HTTP status and message.
    pub fn provider(status: impl Into<Option<u16>>, message: impl Into<String>) -> Self {
        Self::Provider {
            status: status.into(),
            message: message.into(),
        }
    }

    /// Check if this error is retryable.
    ///
    /// A `Provider` error is retryable when the provider's status says the
    /// failure is transient: any 5xx, plus 408 (Request Timeout) and 429 (Too
    /// Many Requests). A 4xx is the provider rejecting the message itself, so
    /// retrying it just burns quota and delays the dead-letter.
    ///
    /// A `Provider` error with no status is treated as *not* retryable: nothing
    /// is known about it, and re-sending an email that may already have been
    /// accepted is worse than surfacing the failure.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Smtp(_) | Self::Network(_) | Self::Timeout | Self::RateLimited(_) => true,
            Self::Provider {
                status: Some(s), ..
            } => *s >= 500 || *s == 408 || *s == 429,
            _ => false,
        }
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

impl From<lettre::transport::smtp::Error> for MailError {
    fn from(err: lettre::transport::smtp::Error) -> Self {
        Self::Smtp(err.to_string())
    }
}

impl From<lettre::address::AddressError> for MailError {
    fn from(err: lettre::address::AddressError) -> Self {
        Self::InvalidAddress(err.to_string())
    }
}

impl From<lettre::error::Error> for MailError {
    fn from(err: lettre::error::Error) -> Self {
        Self::Smtp(err.to_string())
    }
}

impl From<serde_json::Error> for MailError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}

#[cfg(feature = "handlebars")]
impl From<handlebars::RenderError> for MailError {
    fn from(err: handlebars::RenderError) -> Self {
        Self::Template(err.to_string())
    }
}

#[cfg(feature = "handlebars")]
impl From<handlebars::TemplateError> for MailError {
    fn from(err: handlebars::TemplateError) -> Self {
        Self::Template(err.to_string())
    }
}

#[cfg(feature = "tera")]
impl From<tera::Error> for MailError {
    fn from(err: tera::Error) -> Self {
        Self::Template(err.to_string())
    }
}

#[cfg(feature = "minijinja")]
impl From<minijinja::Error> for MailError {
    fn from(err: minijinja::Error) -> Self {
        Self::Template(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Provider` used to be unconditionally non-retryable, so a transient
    /// SendGrid/SES 5xx got zero retries and was dead-lettered on first attempt.
    #[test]
    fn transient_provider_statuses_are_retryable() {
        for status in [500, 502, 503, 504, 408, 429] {
            assert!(
                MailError::provider(status, "transient").is_retryable(),
                "{status} should be retryable"
            );
        }
    }

    #[test]
    fn permanent_provider_statuses_are_not_retryable() {
        for status in [400, 401, 403, 404, 413, 422] {
            assert!(
                !MailError::provider(status, "permanent").is_retryable(),
                "{status} should not be retryable"
            );
        }
    }

    #[test]
    fn provider_without_a_status_is_not_retryable() {
        assert!(!MailError::provider(None, "unknown").is_retryable());
    }

    #[test]
    fn provider_display_includes_the_status() {
        assert_eq!(
            MailError::provider(503u16, "upstream down").to_string(),
            "Provider error (503): upstream down"
        );
        assert_eq!(
            MailError::provider(None, "no status").to_string(),
            "Provider error: no status"
        );
    }

    #[test]
    fn rate_limited_exposes_retry_after() {
        assert_eq!(
            MailError::RateLimited(300).retry_after(),
            Some(std::time::Duration::from_secs(300))
        );
        assert_eq!(MailError::Timeout.retry_after(), None);
    }
}
