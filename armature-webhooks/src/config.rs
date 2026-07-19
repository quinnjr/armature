//! Configuration for webhook client

use crate::RetryPolicy;
use std::time::Duration;

/// Configuration for the webhook client
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    /// Default timeout for webhook requests
    pub timeout: Duration,

    /// User-Agent header for outgoing requests
    pub user_agent: String,

    /// Default retry policy
    pub retry_policy: RetryPolicy,

    /// Whether to verify SSL certificates
    pub verify_ssl: bool,

    /// Maximum payload size in bytes
    pub max_payload_size: usize,

    /// Timestamp tolerance for signature verification (in seconds)
    pub timestamp_tolerance: u64,

    /// Default signing algorithm
    pub signing_algorithm: SigningAlgorithm,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            user_agent: format!("Armature-Webhooks/{}", env!("CARGO_PKG_VERSION")),
            retry_policy: RetryPolicy::default(),
            verify_ssl: true,
            max_payload_size: 1024 * 1024, // 1MB
            timestamp_tolerance: 300,      // 5 minutes
            signing_algorithm: SigningAlgorithm::HmacSha256,
        }
    }
}

impl WebhookConfig {
    /// Create a new configuration with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for custom configuration
    pub fn builder() -> WebhookConfigBuilder {
        WebhookConfigBuilder::new()
    }
}

/// Builder for WebhookConfig
#[derive(Debug, Clone, Default)]
pub struct WebhookConfigBuilder {
    config: WebhookConfig,
}

impl WebhookConfigBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self {
            config: WebhookConfig::default(),
        }
    }

    /// Set the request timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the timeout in seconds
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.config.timeout = Duration::from_secs(secs);
        self
    }

    /// Set the User-Agent header
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }

    /// Set the retry policy
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.config.retry_policy = policy;
        self
    }

    /// Disable retries
    pub fn no_retries(mut self) -> Self {
        self.config.retry_policy = RetryPolicy::none();
        self
    }

    /// Set SSL verification
    pub fn verify_ssl(mut self, verify: bool) -> Self {
        self.config.verify_ssl = verify;
        self
    }

    /// Set maximum payload size
    pub fn max_payload_size(mut self, size: usize) -> Self {
        self.config.max_payload_size = size;
        self
    }

    /// Set timestamp tolerance for signature verification
    pub fn timestamp_tolerance(mut self, seconds: u64) -> Self {
        self.config.timestamp_tolerance = seconds;
        self
    }

    /// Set the signing algorithm
    pub fn signing_algorithm(mut self, algorithm: SigningAlgorithm) -> Self {
        self.config.signing_algorithm = algorithm;
        self
    }

    /// Build the configuration
    pub fn build(self) -> WebhookConfig {
        self.config
    }
}

/// Supported signing algorithms
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SigningAlgorithm {
    /// HMAC-SHA256 (default, most common)
    #[default]
    HmacSha256,

    /// HMAC-SHA512 (more secure)
    HmacSha512,
}

impl SigningAlgorithm {
    /// Get the algorithm name for headers
    pub fn header_value(&self) -> &'static str {
        match self {
            Self::HmacSha256 => "sha256",
            Self::HmacSha512 => "sha512",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = WebhookConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert!(config.verify_ssl);
        assert_eq!(config.max_payload_size, 1024 * 1024);
    }

    #[test]
    fn test_builder() {
        let config = WebhookConfig::builder()
            .timeout_secs(60)
            .verify_ssl(false)
            .max_payload_size(2048)
            .build();

        assert_eq!(config.timeout, Duration::from_secs(60));
        assert!(!config.verify_ssl);
        assert_eq!(config.max_payload_size, 2048);
    }

    #[test]
    fn test_signing_algorithm() {
        assert_eq!(SigningAlgorithm::HmacSha256.header_value(), "sha256");
        assert_eq!(SigningAlgorithm::HmacSha512.header_value(), "sha512");
    }
}
