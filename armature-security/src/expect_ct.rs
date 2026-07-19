//! Expect-CT (Certificate Transparency)
//!
//! Helps detect and prevent use of misissued certificates.
//!
//! # Deprecated
//!
//! The `Expect-CT` header is deprecated and has been removed from all major browsers
//! (Chrome removed support in 2023; other engines never shipped it broadly), so sending
//! this header is a no-op in modern clients. Certificate Transparency is now enforced
//! independently by browsers and CT logs, without needing a site-served opt-in header.
//! [`SecurityMiddleware::enable_all`](crate::SecurityMiddleware::enable_all) and its
//! `Default` impl intentionally do not enable this by default; this type is kept for
//! backwards compatibility and for any legacy clients that still honor it.

/// Expect-CT configuration
#[derive(Debug, Clone)]
pub struct ExpectCtConfig {
    /// Max age in seconds
    pub max_age: u64,

    /// Enforce the policy
    pub enforce: bool,

    /// Report URI for violations
    pub report_uri: Option<String>,
}

impl ExpectCtConfig {
    /// Create a new Expect-CT configuration
    pub fn new(max_age: u64) -> Self {
        Self {
            max_age,
            enforce: false,
            report_uri: None,
        }
    }

    /// Enable enforcement
    pub fn enforce(mut self, enforce: bool) -> Self {
        self.enforce = enforce;
        self
    }

    /// Set report URI
    pub fn report_uri(mut self, uri: String) -> Self {
        self.report_uri = Some(uri);
        self
    }

    /// Convert to header value
    pub fn to_header_value(&self) -> String {
        let mut parts = vec![format!("max-age={}", self.max_age)];

        if self.enforce {
            parts.push("enforce".to_string());
        }

        if let Some(ref uri) = self.report_uri {
            parts.push(format!("report-uri=\"{}\"", uri));
        }

        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expect_ct_basic() {
        let config = ExpectCtConfig::new(86400);
        assert_eq!(config.to_header_value(), "max-age=86400");
    }

    #[test]
    fn test_expect_ct_enforce() {
        let config = ExpectCtConfig::new(86400).enforce(true);
        assert_eq!(config.to_header_value(), "max-age=86400, enforce");
    }

    #[test]
    fn test_expect_ct_report_uri() {
        let config =
            ExpectCtConfig::new(86400).report_uri("https://example.com/report".to_string());
        assert_eq!(
            config.to_header_value(),
            "max-age=86400, report-uri=\"https://example.com/report\""
        );
    }
}
