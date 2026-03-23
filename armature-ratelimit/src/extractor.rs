//! Key extraction for rate limiting
//!
//! This module provides different strategies for extracting rate limit keys
//! from incoming requests.

use std::net::IpAddr;

/// Type alias for key extractor function
pub type KeyExtractorFn = Box<dyn Fn(&RequestInfo) -> Option<String> + Send + Sync>;

/// Information about an incoming request used for key extraction
#[derive(Debug, Clone)]
pub struct RequestInfo {
    /// Client IP address
    pub ip: Option<IpAddr>,
    /// Request path
    pub path: String,
    /// Request method (GET, POST, etc.)
    pub method: String,
    /// User ID (if authenticated)
    pub user_id: Option<String>,
    /// API key (from header or query)
    pub api_key: Option<String>,
    /// Custom headers that might be useful for key extraction
    pub headers: Vec<(String, String)>,
}

impl RequestInfo {
    /// Create a new request info
    pub fn new(path: impl Into<String>, method: impl Into<String>) -> Self {
        Self {
            ip: None,
            path: path.into(),
            method: method.into(),
            user_id: None,
            api_key: None,
            headers: Vec::new(),
        }
    }

    /// Set the IP address
    pub fn with_ip(mut self, ip: IpAddr) -> Self {
        self.ip = Some(ip);
        self
    }

    /// Set the user ID
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set the API key
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Add a header
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Get a header value by name (case-insensitive)
    pub fn get_header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| n.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
}

/// Key extraction strategies
#[derive(Debug, Clone, Default)]
pub enum KeyExtractor {
    /// Extract key from IP address
    #[default]
    Ip,
    /// Extract key from user ID (requires authentication)
    UserId,
    /// Extract key from API key header
    ApiKey {
        /// Header name for API key
        header_name: String,
    },
    /// Extract key from a custom header
    Header {
        /// Header name to extract
        name: String,
    },
    /// Combine IP and path for per-endpoint limiting
    IpAndPath,
    /// Combine user ID and path for per-endpoint limiting
    UserIdAndPath,
    /// Custom extractor function
    Custom(String), // Store description for Debug
}

impl KeyExtractor {
    /// Create an IP-based key extractor
    pub fn ip() -> Self {
        Self::Ip
    }

    /// Create a user ID-based key extractor
    pub fn user_id() -> Self {
        Self::UserId
    }

    /// Create an API key-based extractor
    pub fn api_key(header_name: impl Into<String>) -> Self {
        Self::ApiKey {
            header_name: header_name.into(),
        }
    }

    /// Create a header-based extractor
    pub fn header(name: impl Into<String>) -> Self {
        Self::Header { name: name.into() }
    }

    /// Create an IP and path extractor
    pub fn ip_and_path() -> Self {
        Self::IpAndPath
    }

    /// Create a user ID and path extractor
    pub fn user_id_and_path() -> Self {
        Self::UserIdAndPath
    }

    /// Extract the key from request info
    pub fn extract(&self, info: &RequestInfo) -> Option<String> {
        match self {
            Self::Ip => info.ip.map(|ip| ip.to_string()),
            Self::UserId => info.user_id.clone(),
            Self::ApiKey { header_name } => info.get_header(header_name).map(|s| s.to_string()),
            Self::Header { name } => info.get_header(name).map(|s| s.to_string()),
            Self::IpAndPath => info.ip.map(|ip| format!("{}:{}", ip, info.path)),
            Self::UserIdAndPath => info
                .user_id
                .as_ref()
                .map(|uid| format!("{}:{}", uid, info.path)),
            Self::Custom(_) => None, // Custom extractors use the function directly
        }
    }

    /// Get a description of this extractor
    pub fn description(&self) -> &str {
        match self {
            Self::Ip => "IP address",
            Self::UserId => "User ID",
            Self::ApiKey { .. } => "API key",
            Self::Header { .. } => "Custom header",
            Self::IpAndPath => "IP + Path",
            Self::UserIdAndPath => "User ID + Path",
            Self::Custom(desc) => desc,
        }
    }
}

/// Builder for creating complex key extractors
pub struct KeyExtractorBuilder {
    extractors: Vec<KeyExtractor>,
    fallback_to_ip: bool,
}

impl KeyExtractorBuilder {
    /// Create a new key extractor builder
    pub fn new() -> Self {
        Self {
            extractors: Vec::new(),
            fallback_to_ip: true,
        }
    }

    /// Add an extractor (extractors are tried in order)
    pub fn with_extractor(mut self, extractor: KeyExtractor) -> Self {
        self.extractors.push(extractor);
        self
    }

    /// Try user ID first
    pub fn prefer_user_id(self) -> Self {
        self.with_extractor(KeyExtractor::UserId)
    }

    /// Try API key first
    pub fn prefer_api_key(self, header_name: impl Into<String>) -> Self {
        self.with_extractor(KeyExtractor::ApiKey {
            header_name: header_name.into(),
        })
    }

    /// Disable fallback to IP if other extractors fail
    pub fn no_ip_fallback(mut self) -> Self {
        self.fallback_to_ip = false;
        self
    }

    /// Build the extractor function
    pub fn build(self) -> impl Fn(&RequestInfo) -> Option<String> + Send + Sync {
        let extractors = self.extractors;
        let fallback_to_ip = self.fallback_to_ip;

        move |info: &RequestInfo| {
            // Try each extractor in order
            for extractor in &extractors {
                if let Some(key) = extractor.extract(info) {
                    return Some(key);
                }
            }

            // Fallback to IP if enabled
            if fallback_to_ip {
                if let Some(ip) = info.ip {
                    return Some(ip.to_string());
                }
            }

            None
        }
    }
}

impl Default for KeyExtractorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn sample_request() -> RequestInfo {
        RequestInfo::new("/api/users", "GET")
            .with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
            .with_user_id("user_123")
            .with_api_key("sk_test_abc123")
            .with_header("X-API-Key", "sk_test_abc123")
            .with_header("X-Tenant-ID", "tenant_456")
    }

    #[test]
    fn test_ip_extractor() {
        let extractor = KeyExtractor::ip();
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "192.168.1.1");
    }

    #[test]
    fn test_user_id_extractor() {
        let extractor = KeyExtractor::user_id();
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "user_123");
    }

    #[test]
    fn test_api_key_extractor() {
        let extractor = KeyExtractor::api_key("X-API-Key");
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "sk_test_abc123");
    }

    #[test]
    fn test_header_extractor() {
        let extractor = KeyExtractor::header("X-Tenant-ID");
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "tenant_456");
    }

    #[test]
    fn test_ip_and_path_extractor() {
        let extractor = KeyExtractor::ip_and_path();
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "192.168.1.1:/api/users");
    }

    #[test]
    fn test_user_id_and_path_extractor() {
        let extractor = KeyExtractor::user_id_and_path();
        let request = sample_request();

        let key = extractor.extract(&request).unwrap();
        assert_eq!(key, "user_123:/api/users");
    }

    #[test]
    fn test_builder() {
        let extractor = KeyExtractorBuilder::new()
            .prefer_user_id()
            .prefer_api_key("X-API-Key")
            .build();

        let request = sample_request();
        let key = extractor(&request).unwrap();
        assert_eq!(key, "user_123"); // User ID is preferred

        // Without user ID, falls back to API key
        let request_no_user = RequestInfo::new("/api/users", "GET")
            .with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
            .with_header("X-API-Key", "sk_test_abc123");

        let key = extractor(&request_no_user).unwrap();
        assert_eq!(key, "sk_test_abc123");

        // Without user ID or API key, falls back to IP
        let request_ip_only = RequestInfo::new("/api/users", "GET")
            .with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));

        let key = extractor(&request_ip_only).unwrap();
        assert_eq!(key, "192.168.1.1");
    }

    #[test]
    fn test_builder_no_ip_fallback() {
        let extractor = KeyExtractorBuilder::new()
            .prefer_user_id()
            .no_ip_fallback()
            .build();

        // Without user ID, returns None
        let request = RequestInfo::new("/api/users", "GET")
            .with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));

        assert!(extractor(&request).is_none());
    }

    #[test]
    fn test_request_info_builder() {
        let info = RequestInfo::new("/api/test", "POST")
            .with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
            .with_user_id("u123")
            .with_header("Content-Type", "application/json");

        assert_eq!(info.path, "/api/test");
        assert_eq!(info.method, "POST");
        assert_eq!(info.ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert_eq!(info.user_id, Some("u123".to_string()));
        assert_eq!(info.get_header("content-type"), Some("application/json"));
    }
}
