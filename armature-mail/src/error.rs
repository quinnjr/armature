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
    #[error("Provider error: {0}")]
    Provider(String),

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
    /// Check if this error is retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Smtp(_) | Self::Network(_) | Self::Timeout | Self::RateLimited(_)
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
