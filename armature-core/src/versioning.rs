//! API Versioning Support
//!
//! This module provides comprehensive API versioning capabilities, supporting
//! multiple versioning strategies:
//!
//! - **URL Path Versioning**: `/v1/users`, `/v2/users`
//! - **Header Versioning**: `X-API-Version: 1` or custom headers
//! - **Query Parameter Versioning**: `/users?api-version=1`
//! - **Media Type Versioning**: `Accept: application/vnd.api.v1+json`
//!
//! # Examples
//!
//! ## Basic URL Path Versioning
//!
//! ```ignore
//! use armature_core::versioning::{ApiVersion, VersionedRouter, VersioningStrategy};
//!
//! let mut router = VersionedRouter::new(VersioningStrategy::UrlPath);
//!
//! // Register v1 routes
//! router.version(ApiVersion::new(1, 0), |v1| {
//!     v1.get("/users", get_users_v1);
//! });
//!
//! // Register v2 routes
//! router.version(ApiVersion::new(2, 0), |v2| {
//!     v2.get("/users", get_users_v2);
//! });
//! ```
//!
//! ## Header Versioning
//!
//! ```ignore
//! let router = VersionedRouter::new(VersioningStrategy::Header {
//!     header_name: "X-API-Version".into(),
//! });
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

// ============================================================================
// API Version
// ============================================================================

/// Represents an API version.
///
/// Supports both simple numeric versions (1, 2, 3) and semantic versions (1.0, 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiVersion {
    /// Major version number
    pub major: u32,
    /// Minor version number (optional, defaults to 0)
    pub minor: u32,
}

impl ApiVersion {
    /// Create a new API version.
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Create a version with only major number.
    pub const fn major_only(major: u32) -> Self {
        Self { major, minor: 0 }
    }

    /// Version 1.0
    pub const V1: Self = Self::new(1, 0);
    /// Version 2.0
    pub const V2: Self = Self::new(2, 0);
    /// Version 3.0
    pub const V3: Self = Self::new(3, 0);

    /// Check if this version is compatible with another.
    /// A version is compatible if the major versions match.
    pub fn is_compatible_with(&self, other: &ApiVersion) -> bool {
        self.major == other.major
    }

    /// Format as URL path prefix (e.g., "v1" or "v1.2")
    pub fn as_path_prefix(&self) -> String {
        if self.minor == 0 {
            format!("v{}", self.major)
        } else {
            format!("v{}.{}", self.major, self.minor)
        }
    }

    /// Parse from URL path prefix (e.g., "v1" or "v1.2")
    pub fn from_path_prefix(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V'))?;
        Self::from_str(s).ok()
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.minor == 0 {
            write!(f, "{}", self.major)
        } else {
            write!(f, "{}.{}", self.major, self.minor)
        }
    }
}

impl FromStr for ApiVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        // Handle "v1" or "V1" prefix
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);

        if let Some((major, minor)) = s.split_once('.') {
            let major: u32 = major.parse().map_err(|_| VersionParseError::InvalidFormat)?;
            let minor: u32 = minor.parse().map_err(|_| VersionParseError::InvalidFormat)?;
            Ok(ApiVersion::new(major, minor))
        } else {
            let major: u32 = s.parse().map_err(|_| VersionParseError::InvalidFormat)?;
            Ok(ApiVersion::major_only(major))
        }
    }
}

/// Error parsing an API version string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionParseError {
    /// Invalid version format
    InvalidFormat,
    /// Version string was empty
    Empty,
}

impl fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => write!(f, "Invalid version format"),
            Self::Empty => write!(f, "Empty version string"),
        }
    }
}

impl std::error::Error for VersionParseError {}

// ============================================================================
// Versioning Strategy
// ============================================================================

/// Strategy for extracting API version from requests.
#[derive(Debug, Clone)]
pub enum VersioningStrategy {
    /// Extract version from URL path prefix: `/v1/users`
    UrlPath,

    /// Extract version from custom URL path pattern: `/api/v1/users`
    UrlPathWithPrefix {
        /// Prefix before version (e.g., "/api")
        prefix: String,
    },

    /// Extract version from HTTP header: `X-API-Version: 1`
    Header {
        /// Header name to check
        header_name: String,
    },

    /// Extract version from query parameter: `/users?api-version=1`
    QueryParam {
        /// Query parameter name
        param_name: String,
    },

    /// Extract version from Accept header media type: `application/vnd.api.v1+json`
    MediaType {
        /// Vendor prefix (e.g., "vnd.myapi")
        vendor_prefix: String,
    },

    /// Try multiple strategies in order
    Combined(Vec<VersioningStrategy>),
}

impl VersioningStrategy {
    /// Create URL path versioning strategy.
    pub fn url_path() -> Self {
        Self::UrlPath
    }

    /// Create URL path versioning with a prefix.
    pub fn url_path_with_prefix(prefix: impl Into<String>) -> Self {
        Self::UrlPathWithPrefix {
            prefix: prefix.into(),
        }
    }

    /// Create header versioning strategy with default header name.
    pub fn header() -> Self {
        Self::Header {
            header_name: "X-API-Version".into(),
        }
    }

    /// Create header versioning strategy with custom header name.
    pub fn header_with_name(name: impl Into<String>) -> Self {
        Self::Header {
            header_name: name.into(),
        }
    }

    /// Create query parameter versioning strategy with default param name.
    pub fn query_param() -> Self {
        Self::QueryParam {
            param_name: "api-version".into(),
        }
    }

    /// Create query parameter versioning strategy with custom param name.
    pub fn query_param_with_name(name: impl Into<String>) -> Self {
        Self::QueryParam {
            param_name: name.into(),
        }
    }

    /// Create media type versioning strategy.
    pub fn media_type(vendor_prefix: impl Into<String>) -> Self {
        Self::MediaType {
            vendor_prefix: vendor_prefix.into(),
        }
    }

    /// Create a combined strategy that tries multiple strategies.
    pub fn combined(strategies: Vec<VersioningStrategy>) -> Self {
        Self::Combined(strategies)
    }

    /// Common combined strategy: URL path, then header, then query param.
    pub fn default_combined() -> Self {
        Self::Combined(vec![
            Self::url_path(),
            Self::header(),
            Self::query_param(),
        ])
    }

    /// Extract API version from a request.
    pub fn extract_version(&self, request: &HttpRequest) -> Option<ApiVersion> {
        match self {
            Self::UrlPath => extract_version_from_url_path(&request.path, None),
            Self::UrlPathWithPrefix { prefix } => {
                extract_version_from_url_path(&request.path, Some(prefix))
            }
            Self::Header { header_name } => {
                let header_value = request
                    .headers
                    .get(header_name)
                    .or_else(|| request.headers.get(&header_name.to_lowercase()))?;
                ApiVersion::from_str(header_value).ok()
            }
            Self::QueryParam { param_name } => {
                let param_value = request.query_param(param_name)?;
                ApiVersion::from_str(param_value).ok()
            }
            Self::MediaType { vendor_prefix } => {
                let accept = request
                    .headers
                    .get("accept")
                    .or_else(|| request.headers.get("Accept"))?;
                extract_version_from_media_type(accept, vendor_prefix)
            }
            Self::Combined(strategies) => {
                for strategy in strategies {
                    if let Some(version) = strategy.extract_version(request) {
                        return Some(version);
                    }
                }
                None
            }
        }
    }

    /// Strip version prefix from path (for URL path versioning).
    pub fn strip_version_from_path(&self, path: &str) -> String {
        match self {
            Self::UrlPath => strip_version_prefix(path, None),
            Self::UrlPathWithPrefix { prefix } => strip_version_prefix(path, Some(prefix)),
            _ => path.to_string(),
        }
    }
}

impl Default for VersioningStrategy {
    fn default() -> Self {
        Self::UrlPath
    }
}

/// Extract version from URL path.
fn extract_version_from_url_path(path: &str, prefix: Option<&str>) -> Option<ApiVersion> {
    let path = path.trim_start_matches('/');

    // Handle optional prefix
    let path = if let Some(prefix) = prefix {
        let prefix = prefix.trim_matches('/');
        path.strip_prefix(prefix)?.trim_start_matches('/')
    } else {
        path
    };

    // Get first segment
    let segment = path.split('/').next()?;
    ApiVersion::from_path_prefix(segment)
}

/// Strip version prefix from path.
fn strip_version_prefix(path: &str, prefix: Option<&str>) -> String {
    let path = path.trim_start_matches('/');

    // Handle optional prefix
    let (prefix_part, rest) = if let Some(prefix) = prefix {
        let prefix = prefix.trim_matches('/');
        if let Some(rest) = path.strip_prefix(prefix) {
            (format!("/{}", prefix), rest.trim_start_matches('/'))
        } else {
            return format!("/{}", path);
        }
    } else {
        (String::new(), path)
    };

    // Get first segment and check if it's a version
    let mut parts: Vec<&str> = rest.split('/').collect();
    if !parts.is_empty() && ApiVersion::from_path_prefix(parts[0]).is_some() {
        parts.remove(0);
    }

    if parts.is_empty() || (parts.len() == 1 && parts[0].is_empty()) {
        format!("{}/", prefix_part)
    } else {
        format!("{}/{}", prefix_part, parts.join("/"))
    }
}

/// Extract version from media type.
fn extract_version_from_media_type(accept: &str, vendor_prefix: &str) -> Option<ApiVersion> {
    // Parse Accept header looking for versioned media type
    // Format: application/vnd.myapi.v1+json
    for media_type in accept.split(',') {
        let media_type = media_type.trim();

        // Look for the vendor prefix
        if let Some(rest) = media_type.strip_prefix("application/") {
            if let Some(rest) = rest.strip_prefix(vendor_prefix) {
                let rest = rest.trim_start_matches('.');
                // Extract version before + or ;
                let version_part = rest.split(['+', ';']).next()?;
                if let Some(version) = ApiVersion::from_path_prefix(version_part) {
                    return Some(version);
                }
            }
        }
    }
    None
}

// ============================================================================
// Version Constraints
// ============================================================================

/// Constraint for matching API versions.
#[derive(Debug, Clone)]
pub enum VersionConstraint {
    /// Exact version match
    Exact(ApiVersion),
    /// Minimum version (inclusive)
    Minimum(ApiVersion),
    /// Maximum version (inclusive)
    Maximum(ApiVersion),
    /// Version range (inclusive)
    Range {
        min: ApiVersion,
        max: ApiVersion,
    },
    /// Any of the specified versions
    OneOf(Vec<ApiVersion>),
    /// All versions
    Any,
}

impl VersionConstraint {
    /// Check if a version matches this constraint.
    pub fn matches(&self, version: &ApiVersion) -> bool {
        match self {
            Self::Exact(v) => version == v,
            Self::Minimum(min) => version >= min,
            Self::Maximum(max) => version <= max,
            Self::Range { min, max } => version >= min && version <= max,
            Self::OneOf(versions) => versions.contains(version),
            Self::Any => true,
        }
    }

    /// Create an exact version constraint.
    pub fn exact(version: ApiVersion) -> Self {
        Self::Exact(version)
    }

    /// Create a minimum version constraint.
    pub fn minimum(version: ApiVersion) -> Self {
        Self::Minimum(version)
    }

    /// Create a version range constraint.
    pub fn range(min: ApiVersion, max: ApiVersion) -> Self {
        Self::Range { min, max }
    }
}

// ============================================================================
// Version Extractor for Request
// ============================================================================

/// Extension trait for extracting API version from requests.
pub trait VersionedRequest {
    /// Extract API version using the specified strategy.
    fn api_version(&self, strategy: &VersioningStrategy) -> Option<ApiVersion>;

    /// Extract API version from URL path.
    fn url_version(&self) -> Option<ApiVersion>;

    /// Extract API version from header.
    fn header_version(&self, header_name: &str) -> Option<ApiVersion>;

    /// Extract API version from query parameter.
    fn query_version(&self, param_name: &str) -> Option<ApiVersion>;
}

impl VersionedRequest for HttpRequest {
    fn api_version(&self, strategy: &VersioningStrategy) -> Option<ApiVersion> {
        strategy.extract_version(self)
    }

    fn url_version(&self) -> Option<ApiVersion> {
        extract_version_from_url_path(&self.path, None)
    }

    fn header_version(&self, header_name: &str) -> Option<ApiVersion> {
        let value = self
            .headers
            .get(header_name)
            .or_else(|| self.headers.get(&header_name.to_lowercase()))?;
        ApiVersion::from_str(value).ok()
    }

    fn query_version(&self, param_name: &str) -> Option<ApiVersion> {
        let value = self.query_param(param_name)?;
        ApiVersion::from_str(value).ok()
    }
}

// ============================================================================
// Version Response Headers
// ============================================================================

/// Extension trait for adding version headers to responses.
pub trait VersionedResponse {
    /// Add API version header to response.
    fn with_api_version(self, version: ApiVersion) -> Self;

    /// Add deprecated version warning header.
    fn with_deprecation_warning(self, message: &str) -> Self;

    /// Add supported versions header.
    fn with_supported_versions(self, versions: &[ApiVersion]) -> Self;

    /// Add sunset header for deprecated versions.
    fn with_sunset_date(self, date: &str) -> Self;
}

impl VersionedResponse for HttpResponse {
    fn with_api_version(mut self, version: ApiVersion) -> Self {
        self.headers
            .insert("X-API-Version".to_string(), version.to_string());
        self
    }

    fn with_deprecation_warning(mut self, message: &str) -> Self {
        self.headers
            .insert("X-API-Deprecated".to_string(), "true".to_string());
        self.headers
            .insert("X-API-Deprecation-Message".to_string(), message.to_string());
        // Add standard Deprecation header (RFC 8594)
        self.headers
            .insert("Deprecation".to_string(), "true".to_string());
        self
    }

    fn with_supported_versions(mut self, versions: &[ApiVersion]) -> Self {
        let versions_str = versions
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        self.headers
            .insert("X-API-Supported-Versions".to_string(), versions_str);
        self
    }

    fn with_sunset_date(mut self, date: &str) -> Self {
        // Sunset header (RFC 8594)
        self.headers
            .insert("Sunset".to_string(), date.to_string());
        self
    }
}

// ============================================================================
// Version Negotiation
// ============================================================================

/// Configuration for API version negotiation.
#[derive(Debug, Clone)]
pub struct VersionConfig {
    /// Versioning strategy to use
    pub strategy: VersioningStrategy,
    /// Default version when none is specified
    pub default_version: Option<ApiVersion>,
    /// Supported versions
    pub supported_versions: Vec<ApiVersion>,
    /// Deprecated versions (still work but emit warnings)
    pub deprecated_versions: Vec<ApiVersion>,
    /// Whether to allow requests without version
    pub require_version: bool,
    /// Whether to add version headers to responses
    pub add_response_headers: bool,
}

impl VersionConfig {
    /// Create a new version configuration.
    pub fn new(strategy: VersioningStrategy) -> Self {
        Self {
            strategy,
            default_version: None,
            supported_versions: Vec::new(),
            deprecated_versions: Vec::new(),
            require_version: false,
            add_response_headers: true,
        }
    }

    /// Set the default version.
    pub fn default_version(mut self, version: ApiVersion) -> Self {
        self.default_version = Some(version);
        self
    }

    /// Add a supported version.
    pub fn supported(mut self, version: ApiVersion) -> Self {
        self.supported_versions.push(version);
        self
    }

    /// Add multiple supported versions.
    pub fn supported_versions(mut self, versions: impl IntoIterator<Item = ApiVersion>) -> Self {
        self.supported_versions.extend(versions);
        self
    }

    /// Mark a version as deprecated.
    pub fn deprecated(mut self, version: ApiVersion) -> Self {
        self.deprecated_versions.push(version);
        self
    }

    /// Require version in requests.
    pub fn require_version(mut self, require: bool) -> Self {
        self.require_version = require;
        self
    }

    /// Configure response headers.
    pub fn add_response_headers(mut self, add: bool) -> Self {
        self.add_response_headers = add;
        self
    }

    /// Check if a version is supported.
    pub fn is_supported(&self, version: &ApiVersion) -> bool {
        self.supported_versions.is_empty() || self.supported_versions.contains(version)
    }

    /// Check if a version is deprecated.
    pub fn is_deprecated(&self, version: &ApiVersion) -> bool {
        self.deprecated_versions.contains(version)
    }

    /// Resolve version from request.
    pub fn resolve_version(&self, request: &HttpRequest) -> Result<ApiVersion, Error> {
        // Try to extract version
        if let Some(version) = self.strategy.extract_version(request) {
            // Check if supported
            if !self.is_supported(&version) {
                return Err(Error::BadRequest(format!(
                    "API version {} is not supported. Supported versions: {}",
                    version,
                    self.supported_versions
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            return Ok(version);
        }

        // Use default version if available
        if let Some(default) = self.default_version {
            return Ok(default);
        }

        // Version required but not provided
        if self.require_version {
            return Err(Error::BadRequest(
                "API version is required but not provided".to_string(),
            ));
        }

        // Return first supported version as fallback
        self.supported_versions
            .first()
            .copied()
            .ok_or_else(|| Error::Internal("No API versions configured".to_string()))
    }

    /// Apply version headers to response.
    pub fn apply_headers(&self, mut response: HttpResponse, version: &ApiVersion) -> HttpResponse {
        if self.add_response_headers {
            response = response.with_api_version(*version);

            if !self.supported_versions.is_empty() {
                response = response.with_supported_versions(&self.supported_versions);
            }

            if self.is_deprecated(version) {
                response = response.with_deprecation_warning(&format!(
                    "API version {} is deprecated",
                    version
                ));
            }
        }
        response
    }
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self::new(VersioningStrategy::default())
    }
}

// ============================================================================
// Versioned Handler
// ============================================================================

/// A handler that routes to different implementations based on API version.
pub struct VersionedHandler<T> {
    handlers: HashMap<ApiVersion, T>,
    fallback: Option<T>,
}

impl<T> VersionedHandler<T> {
    /// Create a new versioned handler.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            fallback: None,
        }
    }

    /// Register a handler for a specific version.
    pub fn version(mut self, version: ApiVersion, handler: T) -> Self {
        self.handlers.insert(version, handler);
        self
    }

    /// Set a fallback handler for unknown versions.
    pub fn fallback(mut self, handler: T) -> Self {
        self.fallback = Some(handler);
        self
    }

    /// Get the handler for a specific version.
    pub fn get(&self, version: &ApiVersion) -> Option<&T> {
        self.handlers.get(version).or(self.fallback.as_ref())
    }

    /// Get the handler for a version, or try compatible versions.
    pub fn get_compatible(&self, version: &ApiVersion) -> Option<&T> {
        // Try exact match first
        if let Some(handler) = self.handlers.get(version) {
            return Some(handler);
        }

        // Try same major version with lower minor
        for (v, handler) in &self.handlers {
            if v.major == version.major && v.minor <= version.minor {
                return Some(handler);
            }
        }

        self.fallback.as_ref()
    }
}

impl<T> Default for VersionedHandler<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_version_new() {
        let v = ApiVersion::new(1, 2);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
    }

    #[test]
    fn test_api_version_major_only() {
        let v = ApiVersion::major_only(3);
        assert_eq!(v.major, 3);
        assert_eq!(v.minor, 0);
    }

    #[test]
    fn test_api_version_display() {
        assert_eq!(ApiVersion::new(1, 0).to_string(), "1");
        assert_eq!(ApiVersion::new(2, 3).to_string(), "2.3");
    }

    #[test]
    fn test_api_version_from_str() {
        assert_eq!(ApiVersion::from_str("1").unwrap(), ApiVersion::V1);
        assert_eq!(ApiVersion::from_str("2.3").unwrap(), ApiVersion::new(2, 3));
        assert_eq!(ApiVersion::from_str("v1").unwrap(), ApiVersion::V1);
        assert_eq!(ApiVersion::from_str("V2").unwrap(), ApiVersion::V2);
    }

    #[test]
    fn test_api_version_as_path_prefix() {
        assert_eq!(ApiVersion::V1.as_path_prefix(), "v1");
        assert_eq!(ApiVersion::new(2, 3).as_path_prefix(), "v2.3");
    }

    #[test]
    fn test_api_version_from_path_prefix() {
        assert_eq!(ApiVersion::from_path_prefix("v1"), Some(ApiVersion::V1));
        assert_eq!(ApiVersion::from_path_prefix("v2.3"), Some(ApiVersion::new(2, 3)));
        assert_eq!(ApiVersion::from_path_prefix("invalid"), None);
    }

    #[test]
    fn test_api_version_compatibility() {
        let v1 = ApiVersion::new(1, 0);
        let v1_1 = ApiVersion::new(1, 1);
        let v2 = ApiVersion::new(2, 0);

        assert!(v1.is_compatible_with(&v1_1));
        assert!(!v1.is_compatible_with(&v2));
    }

    #[test]
    fn test_url_path_versioning() {
        let strategy = VersioningStrategy::url_path();

        let mut request = HttpRequest::new("GET", "/v1/users".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V1));

        request.path = "/v2/users".to_string();
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V2));

        request.path = "/users".to_string();
        assert_eq!(strategy.extract_version(&request), None);
    }

    #[test]
    fn test_header_versioning() {
        let strategy = VersioningStrategy::header();

        let mut request = HttpRequest::new("GET", "/users".to_string());
        request.headers.insert("X-API-Version".to_string(), "1".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V1));

        request.headers.clear();
        request.headers.insert("x-api-version".to_string(), "2".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V2));
    }

    #[test]
    fn test_query_param_versioning() {
        let strategy = VersioningStrategy::query_param();

        let mut request = HttpRequest::new("GET", "/users".to_string());
        request.push_query_param("api-version", "1");
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V1));
    }

    #[test]
    fn test_media_type_versioning() {
        let strategy = VersioningStrategy::media_type("vnd.myapi");

        let mut request = HttpRequest::new("GET", "/users".to_string());
        request.headers.insert("Accept".to_string(), "application/vnd.myapi.v1+json".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V1));

        request.headers.insert("Accept".to_string(), "application/vnd.myapi.v2+json".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V2));
    }

    #[test]
    fn test_combined_versioning() {
        let strategy = VersioningStrategy::default_combined();

        // URL path takes precedence
        let mut request = HttpRequest::new("GET", "/v1/users".to_string());
        request.headers.insert("X-API-Version".to_string(), "2".to_string());
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V1));

        // Falls back to header
        request.path = "/users".to_string();
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V2));

        // Falls back to query param
        request.headers.clear();
        request.push_query_param("api-version", "3");
        assert_eq!(strategy.extract_version(&request), Some(ApiVersion::V3));
    }

    #[test]
    fn test_strip_version_prefix() {
        assert_eq!(strip_version_prefix("/v1/users", None), "/users");
        assert_eq!(strip_version_prefix("/v2/users/123", None), "/users/123");
        assert_eq!(strip_version_prefix("/users", None), "/users");
        assert_eq!(strip_version_prefix("/api/v1/users", Some("/api")), "/api/users");
    }

    #[test]
    fn test_version_constraint() {
        let v1 = ApiVersion::V1;
        let v2 = ApiVersion::V2;
        let v3 = ApiVersion::V3;

        assert!(VersionConstraint::exact(v1).matches(&v1));
        assert!(!VersionConstraint::exact(v1).matches(&v2));

        assert!(VersionConstraint::minimum(v2).matches(&v2));
        assert!(VersionConstraint::minimum(v2).matches(&v3));
        assert!(!VersionConstraint::minimum(v2).matches(&v1));

        assert!(VersionConstraint::range(v1, v2).matches(&v1));
        assert!(VersionConstraint::range(v1, v2).matches(&v2));
        assert!(!VersionConstraint::range(v1, v2).matches(&v3));
    }

    #[test]
    fn test_version_config() {
        let config = VersionConfig::new(VersioningStrategy::url_path())
            .default_version(ApiVersion::V1)
            .supported_versions([ApiVersion::V1, ApiVersion::V2])
            .deprecated(ApiVersion::V1);

        assert!(config.is_supported(&ApiVersion::V1));
        assert!(config.is_supported(&ApiVersion::V2));
        assert!(!config.is_supported(&ApiVersion::V3));

        assert!(config.is_deprecated(&ApiVersion::V1));
        assert!(!config.is_deprecated(&ApiVersion::V2));
    }

    #[test]
    fn test_versioned_handler() {
        let handler = VersionedHandler::new()
            .version(ApiVersion::V1, "handler_v1")
            .version(ApiVersion::V2, "handler_v2")
            .fallback("fallback");

        assert_eq!(handler.get(&ApiVersion::V1), Some(&"handler_v1"));
        assert_eq!(handler.get(&ApiVersion::V2), Some(&"handler_v2"));
        assert_eq!(handler.get(&ApiVersion::V3), Some(&"fallback"));
    }

    #[test]
    fn test_versioned_response() {
        let response = HttpResponse::ok()
            .with_api_version(ApiVersion::V1)
            .with_deprecation_warning("Use v2 instead")
            .with_supported_versions(&[ApiVersion::V1, ApiVersion::V2]);

        assert_eq!(response.headers.get("X-API-Version"), Some(&"1".to_string()));
        assert_eq!(response.headers.get("X-API-Deprecated"), Some(&"true".to_string()));
    }
}


