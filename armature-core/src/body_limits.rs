//! Request body size limits configuration and middleware.
//!
//! This module provides configurable request body size limits to protect against
//! denial-of-service attacks and resource exhaustion from oversized payloads.
//!
//! ## Quick Start
//!
//! ```rust
//! use armature_core::body_limits::{BodyLimitConfig, BodyLimitMiddleware};
//!
//! // Create middleware with 1MB limit
//! let middleware = BodyLimitMiddleware::new(1024 * 1024);
//!
//! // Or with human-readable units
//! let middleware = BodyLimitMiddleware::megabytes(10);
//! ```
//!
//! ## Using the Decorator
//!
//! ```ignore
//! use armature::{post, body_limit};
//!
//! #[body_limit(5mb)]  // 5 megabyte limit
//! #[post("/upload")]
//! async fn upload_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
//!     Ok(HttpResponse::ok())
//! }
//!
//! #[body_limit(1kb)]  // 1 kilobyte limit (strict)
//! #[post("/small")]
//! async fn small_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
//!     Ok(HttpResponse::ok())
//! }
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Common size constants for convenience
pub mod sizes {
    /// 1 Kilobyte
    pub const KB: usize = 1024;
    /// 1 Megabyte
    pub const MB: usize = 1024 * 1024;
    /// 1 Gigabyte
    pub const GB: usize = 1024 * 1024 * 1024;

    /// 1 KB
    pub const ONE_KB: usize = KB;
    /// 4 KB
    pub const FOUR_KB: usize = 4 * KB;
    /// 8 KB
    pub const EIGHT_KB: usize = 8 * KB;
    /// 16 KB
    pub const SIXTEEN_KB: usize = 16 * KB;
    /// 64 KB
    pub const SIXTY_FOUR_KB: usize = 64 * KB;
    /// 128 KB
    pub const ONE_TWENTY_EIGHT_KB: usize = 128 * KB;
    /// 256 KB
    pub const TWO_FIFTY_SIX_KB: usize = 256 * KB;
    /// 512 KB
    pub const FIVE_TWELVE_KB: usize = 512 * KB;

    /// 1 MB
    pub const ONE_MB: usize = MB;
    /// 2 MB
    pub const TWO_MB: usize = 2 * MB;
    /// 5 MB
    pub const FIVE_MB: usize = 5 * MB;
    /// 10 MB
    pub const TEN_MB: usize = 10 * MB;
    /// 50 MB
    pub const FIFTY_MB: usize = 50 * MB;
    /// 100 MB
    pub const HUNDRED_MB: usize = 100 * MB;

    /// 1 GB
    pub const ONE_GB: usize = GB;
}

/// Configuration for request body size limits.
///
/// ## Example
///
/// ```rust
/// use armature_core::body_limits::BodyLimitConfig;
///
/// let config = BodyLimitConfig::new()
///     .default_limit_mb(10)                    // 10MB default
///     .route_limit("/api/upload", 100 * 1024 * 1024)  // 100MB for uploads
///     .route_limit_kb("/api/json", 64);        // 64KB for JSON endpoints
/// ```
#[derive(Debug, Clone)]
pub struct BodyLimitConfig {
    /// Default maximum body size in bytes
    pub default_limit: usize,
    /// Route-specific limits (path pattern -> max bytes)
    route_limits: HashMap<String, usize>,
    /// Whether to include limit in error messages
    pub include_limit_in_error: bool,
    /// Custom error message format
    pub error_message: Option<String>,
}

impl Default for BodyLimitConfig {
    fn default() -> Self {
        Self {
            default_limit: sizes::ONE_MB, // 1MB default
            route_limits: HashMap::new(),
            include_limit_in_error: true,
            error_message: None,
        }
    }
}

impl BodyLimitConfig {
    /// Creates a new body limit configuration with 1MB default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the default limit in bytes.
    pub fn default_limit(mut self, bytes: usize) -> Self {
        self.default_limit = bytes;
        self
    }

    /// Sets the default limit in kilobytes.
    pub fn default_limit_kb(mut self, kb: usize) -> Self {
        self.default_limit = kb * sizes::KB;
        self
    }

    /// Sets the default limit in megabytes.
    pub fn default_limit_mb(mut self, mb: usize) -> Self {
        self.default_limit = mb * sizes::MB;
        self
    }

    /// Adds a route-specific limit in bytes.
    pub fn route_limit(mut self, path: &str, bytes: usize) -> Self {
        self.route_limits.insert(path.to_string(), bytes);
        self
    }

    /// Adds a route-specific limit in kilobytes.
    pub fn route_limit_kb(mut self, path: &str, kb: usize) -> Self {
        self.route_limits.insert(path.to_string(), kb * sizes::KB);
        self
    }

    /// Adds a route-specific limit in megabytes.
    pub fn route_limit_mb(mut self, path: &str, mb: usize) -> Self {
        self.route_limits.insert(path.to_string(), mb * sizes::MB);
        self
    }

    /// Sets whether to include the limit in error messages.
    pub fn include_limit_in_error(mut self, include: bool) -> Self {
        self.include_limit_in_error = include;
        self
    }

    /// Sets a custom error message.
    ///
    /// Use `{limit}` as a placeholder for the limit value.
    /// Use `{size}` as a placeholder for the actual body size.
    pub fn error_message(mut self, message: &str) -> Self {
        self.error_message = Some(message.to_string());
        self
    }

    /// Gets the limit for a specific path.
    pub fn get_limit_for_path(&self, path: &str) -> usize {
        // Check for exact match first
        if let Some(limit) = self.route_limits.get(path) {
            return *limit;
        }

        // Check for prefix matches, preferring the longest (most specific)
        // pattern so overlapping patterns resolve deterministically.
        self.route_limits
            .iter()
            .filter(|(pattern, _)| path.starts_with(pattern.as_str()))
            .max_by_key(|(pattern, _)| pattern.len())
            .map(|(_, limit)| *limit)
            .unwrap_or(self.default_limit)
    }

    /// Formats the error message for a limit violation.
    pub fn format_error(&self, actual_size: usize, limit: usize) -> String {
        if let Some(ref message) = self.error_message {
            message
                .replace("{limit}", &format_bytes(limit))
                .replace("{size}", &format_bytes(actual_size))
        } else if self.include_limit_in_error {
            format!(
                "Request body size ({}) exceeds maximum allowed size ({})",
                format_bytes(actual_size),
                format_bytes(limit)
            )
        } else {
            "Request body too large".to_string()
        }
    }

    /// Creates a ConfigurableBodyLimitMiddleware from this configuration.
    pub fn into_middleware(self) -> ConfigurableBodyLimitMiddleware {
        ConfigurableBodyLimitMiddleware::new(self)
    }
}

/// Simple body limit middleware with a fixed size limit.
///
/// ## Example
///
/// ```rust
/// use armature_core::body_limits::BodyLimitMiddleware;
///
/// // Create middleware with 1MB limit
/// let middleware = BodyLimitMiddleware::new(1024 * 1024);
///
/// // Or use convenience methods
/// let middleware = BodyLimitMiddleware::megabytes(10);
/// let middleware = BodyLimitMiddleware::kilobytes(512);
/// ```
#[derive(Debug, Clone)]
pub struct BodyLimitMiddleware {
    /// Maximum body size in bytes
    pub max_size: usize,
}

impl BodyLimitMiddleware {
    /// Creates a new body limit middleware with the specified limit in bytes.
    pub fn new(max_size: usize) -> Self {
        Self { max_size }
    }

    /// Creates a new body limit middleware with the specified limit in kilobytes.
    pub fn kilobytes(kb: usize) -> Self {
        Self::new(kb * sizes::KB)
    }

    /// Creates a new body limit middleware with the specified limit in megabytes.
    pub fn megabytes(mb: usize) -> Self {
        Self::new(mb * sizes::MB)
    }

    /// Creates a new body limit middleware with the specified limit in gigabytes.
    pub fn gigabytes(gb: usize) -> Self {
        Self::new(gb * sizes::GB)
    }

    /// Returns the maximum size limit.
    pub fn limit(&self) -> usize {
        self.max_size
    }
}

#[async_trait]
impl crate::middleware::Middleware for BodyLimitMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: crate::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        if req.body.len() > self.max_size {
            return Err(Error::PayloadTooLarge(format!(
                "Request body size ({}) exceeds maximum allowed size ({})",
                format_bytes(req.body.len()),
                format_bytes(self.max_size)
            )));
        }
        next(req).await
    }
}

/// Configurable body limit middleware with per-route limits.
///
/// ## Example
///
/// ```rust
/// use armature_core::body_limits::{BodyLimitConfig, ConfigurableBodyLimitMiddleware};
///
/// let config = BodyLimitConfig::new()
///     .default_limit_mb(10)
///     .route_limit_mb("/api/upload", 100);
///
/// let middleware = ConfigurableBodyLimitMiddleware::new(config);
/// ```
#[derive(Debug, Clone)]
pub struct ConfigurableBodyLimitMiddleware {
    config: Arc<BodyLimitConfig>,
}

impl ConfigurableBodyLimitMiddleware {
    /// Creates a new configurable body limit middleware.
    pub fn new(config: BodyLimitConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Creates a middleware with a simple default limit in megabytes.
    pub fn with_default_mb(mb: usize) -> Self {
        Self::new(BodyLimitConfig::new().default_limit_mb(mb))
    }
}

#[async_trait]
impl crate::middleware::Middleware for ConfigurableBodyLimitMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: crate::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        let limit = self.config.get_limit_for_path(&req.path);

        if req.body.len() > limit {
            let error_msg = self.config.format_error(req.body.len(), limit);
            return Err(Error::PayloadTooLarge(error_msg));
        }

        next(req).await
    }
}

/// Builder for creating body limit middleware with a fluent API.
#[derive(Debug, Clone)]
pub struct BodyLimitBuilder {
    config: BodyLimitConfig,
}

impl Default for BodyLimitBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl BodyLimitBuilder {
    /// Creates a new builder with default settings.
    pub fn new() -> Self {
        Self {
            config: BodyLimitConfig::default(),
        }
    }

    /// Sets the default limit in bytes.
    pub fn default_bytes(mut self, bytes: usize) -> Self {
        self.config.default_limit = bytes;
        self
    }

    /// Sets the default limit in kilobytes.
    pub fn default_kb(mut self, kb: usize) -> Self {
        self.config.default_limit = kb * sizes::KB;
        self
    }

    /// Sets the default limit in megabytes.
    pub fn default_mb(mut self, mb: usize) -> Self {
        self.config.default_limit = mb * sizes::MB;
        self
    }

    /// Adds a route-specific limit in bytes.
    pub fn route(mut self, path: &str, bytes: usize) -> Self {
        self.config.route_limits.insert(path.to_string(), bytes);
        self
    }

    /// Adds a route-specific limit in kilobytes.
    pub fn route_kb(mut self, path: &str, kb: usize) -> Self {
        self.config
            .route_limits
            .insert(path.to_string(), kb * sizes::KB);
        self
    }

    /// Adds a route-specific limit in megabytes.
    pub fn route_mb(mut self, path: &str, mb: usize) -> Self {
        self.config
            .route_limits
            .insert(path.to_string(), mb * sizes::MB);
        self
    }

    /// Sets whether to show the limit in error messages.
    pub fn show_limit_in_error(mut self, show: bool) -> Self {
        self.config.include_limit_in_error = show;
        self
    }

    /// Sets a custom error message.
    pub fn error_message(mut self, message: &str) -> Self {
        self.config.error_message = Some(message.to_string());
        self
    }

    /// Builds a ConfigurableBodyLimitMiddleware.
    pub fn build(self) -> ConfigurableBodyLimitMiddleware {
        ConfigurableBodyLimitMiddleware::new(self.config)
    }

    /// Builds a simple BodyLimitMiddleware (uses default limit only).
    pub fn build_simple(self) -> BodyLimitMiddleware {
        BodyLimitMiddleware::new(self.config.default_limit)
    }
}

/// Formats a byte count into a human-readable string.
pub fn format_bytes(bytes: usize) -> String {
    if bytes >= sizes::GB {
        format!("{:.2} GB", bytes as f64 / sizes::GB as f64)
    } else if bytes >= sizes::MB {
        format!("{:.2} MB", bytes as f64 / sizes::MB as f64)
    } else if bytes >= sizes::KB {
        format!("{:.2} KB", bytes as f64 / sizes::KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Parses a size string into bytes.
///
/// Supports formats like "10mb", "512kb", "1gb", "1024" (bytes), "1024b".
pub fn parse_size(s: &str) -> Option<usize> {
    let s = s.trim().to_lowercase();

    // Try parsing as just a number (bytes)
    if let Ok(bytes) = s.parse::<usize>() {
        return Some(bytes);
    }

    // Try parsing with unit suffix
    let (num_str, multiplier) = if s.ends_with("gb") {
        (&s[..s.len() - 2], sizes::GB)
    } else if s.ends_with("mb") {
        (&s[..s.len() - 2], sizes::MB)
    } else if s.ends_with("kb") {
        (&s[..s.len() - 2], sizes::KB)
    } else if s.ends_with('g') {
        (&s[..s.len() - 1], sizes::GB)
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], sizes::MB)
    } else if s.ends_with('k') {
        (&s[..s.len() - 1], sizes::KB)
    } else if s.ends_with('b') {
        (&s[..s.len() - 1], 1)
    } else {
        return None;
    };

    let num: f64 = num_str.trim().parse().ok()?;
    Some((num * multiplier as f64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_constants() {
        assert_eq!(sizes::KB, 1024);
        assert_eq!(sizes::MB, 1024 * 1024);
        assert_eq!(sizes::GB, 1024 * 1024 * 1024);
        assert_eq!(sizes::TEN_MB, 10 * 1024 * 1024);
    }

    #[test]
    fn test_body_limit_config_default() {
        let config = BodyLimitConfig::new();
        assert_eq!(config.default_limit, sizes::ONE_MB);
    }

    #[test]
    fn test_body_limit_config_custom_default() {
        let config = BodyLimitConfig::new().default_limit_mb(10);
        assert_eq!(config.default_limit, sizes::TEN_MB);
    }

    #[test]
    fn test_body_limit_config_route_specific() {
        let config = BodyLimitConfig::new()
            .default_limit_mb(1)
            .route_limit_mb("/api/upload", 100)
            .route_limit_kb("/api/small", 64);

        assert_eq!(config.get_limit_for_path("/api/upload"), 100 * sizes::MB);
        assert_eq!(config.get_limit_for_path("/api/small"), 64 * sizes::KB);
        assert_eq!(config.get_limit_for_path("/api/other"), sizes::ONE_MB);
    }

    #[test]
    fn test_body_limit_config_prefix_matching() {
        let config = BodyLimitConfig::new()
            .default_limit_mb(1)
            .route_limit_mb("/api/upload", 100);

        assert_eq!(
            config.get_limit_for_path("/api/upload/123"),
            100 * sizes::MB
        );
    }

    #[test]
    fn test_body_limit_config_overlapping_prefixes() {
        // The longest (most specific) matching prefix must win regardless
        // of HashMap iteration order.
        let config = BodyLimitConfig::new()
            .default_limit_kb(64)
            .route_limit_mb("/api", 1)
            .route_limit_mb("/api/upload", 100);

        assert_eq!(
            config.get_limit_for_path("/api/upload/file"),
            100 * sizes::MB
        );
        assert_eq!(config.get_limit_for_path("/api/other"), sizes::MB);
        assert_eq!(config.get_limit_for_path("/health"), 64 * sizes::KB);
    }

    #[test]
    fn test_body_limit_middleware_creation() {
        let middleware = BodyLimitMiddleware::new(1024);
        assert_eq!(middleware.limit(), 1024);

        let middleware = BodyLimitMiddleware::megabytes(5);
        assert_eq!(middleware.limit(), 5 * sizes::MB);

        let middleware = BodyLimitMiddleware::kilobytes(512);
        assert_eq!(middleware.limit(), 512 * sizes::KB);
    }

    #[test]
    fn test_body_limit_builder() {
        let middleware = BodyLimitBuilder::new()
            .default_mb(10)
            .route_mb("/api/upload", 100)
            .route_kb("/api/small", 64)
            .build();

        assert_eq!(middleware.config.get_limit_for_path("/"), 10 * sizes::MB);
        assert_eq!(
            middleware.config.get_limit_for_path("/api/upload"),
            100 * sizes::MB
        );
        assert_eq!(
            middleware.config.get_limit_for_path("/api/small"),
            64 * sizes::KB
        );
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 bytes");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("1kb"), Some(1024));
        assert_eq!(parse_size("1KB"), Some(1024));
        assert_eq!(parse_size("1k"), Some(1024));
        assert_eq!(parse_size("1mb"), Some(1024 * 1024));
        assert_eq!(parse_size("1MB"), Some(1024 * 1024));
        assert_eq!(parse_size("1m"), Some(1024 * 1024));
        assert_eq!(parse_size("10mb"), Some(10 * 1024 * 1024));
        assert_eq!(parse_size("1gb"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("1.5mb"), Some((1.5 * 1024.0 * 1024.0) as usize));
        assert_eq!(parse_size("invalid"), None);
    }

    #[test]
    fn test_error_message_formatting() {
        let config = BodyLimitConfig::new().include_limit_in_error(true);
        let error = config.format_error(2 * sizes::MB, sizes::ONE_MB);
        assert!(error.contains("2.00 MB"));
        assert!(error.contains("1.00 MB"));

        let config = BodyLimitConfig::new().include_limit_in_error(false);
        let error = config.format_error(2 * sizes::MB, sizes::ONE_MB);
        assert_eq!(error, "Request body too large");

        let config = BodyLimitConfig::new().error_message("Body {size} exceeds {limit}");
        let error = config.format_error(2 * sizes::MB, sizes::ONE_MB);
        assert_eq!(error, "Body 2.00 MB exceeds 1.00 MB");
    }

    #[test]
    fn test_body_limit_config_into_middleware() {
        let config = BodyLimitConfig::new().default_limit_mb(5);
        let middleware = config.into_middleware();
        assert_eq!(middleware.config.default_limit, 5 * sizes::MB);
    }

    #[tokio::test]
    async fn test_body_limit_middleware_allows_small_body() {
        use crate::middleware::Middleware;

        let middleware = BodyLimitMiddleware::kilobytes(10);
        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        req.body = vec![0; 5 * sizes::KB]; // 5KB, within limit

        let result = middleware
            .handle(
                req,
                Box::new(|_req| Box::pin(async { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_body_limit_middleware_rejects_large_body() {
        use crate::middleware::Middleware;

        let middleware = BodyLimitMiddleware::kilobytes(1);
        let mut req = HttpRequest::new("POST".to_string(), "/api".to_string());
        req.body = vec![0; 5 * sizes::KB]; // 5KB, exceeds 1KB limit

        let result = middleware
            .handle(
                req,
                Box::new(|_req| Box::pin(async { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_configurable_middleware_route_limits() {
        use crate::middleware::Middleware;

        let config = BodyLimitConfig::new()
            .default_limit_kb(1)
            .route_limit_mb("/api/upload", 10);

        let middleware = ConfigurableBodyLimitMiddleware::new(config);

        // Small body on upload route should pass
        let mut req = HttpRequest::new("POST".to_string(), "/api/upload".to_string());
        req.body = vec![0; 5 * sizes::MB]; // 5MB

        let result = middleware
            .handle(
                req,
                Box::new(|_req| Box::pin(async { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(result.is_ok());

        // Same body on regular route should fail
        let mut req = HttpRequest::new("POST".to_string(), "/api/other".to_string());
        req.body = vec![0; 5 * sizes::KB]; // 5KB, exceeds 1KB default

        let result = middleware
            .handle(
                req,
                Box::new(|_req| Box::pin(async { Ok(HttpResponse::ok()) })),
            )
            .await;

        assert!(result.is_err());
    }
}
