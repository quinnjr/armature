//! Request timeout configuration and utilities.
//!
//! This module provides timeout configuration for HTTP requests, supporting:
//! - Global default timeouts
//! - Per-route timeout overrides
//! - Timeout middleware
//! - Graceful timeout handling
//!
//! ## Quick Start
//!
//! ```rust
//! use armature_core::timeout::{TimeoutConfig, TimeoutMiddleware};
//!
//! // Create a timeout middleware with 30 second default
//! let middleware = TimeoutMiddleware::new(30);
//!
//! // Or with custom configuration
//! let config = TimeoutConfig::new()
//!     .default_timeout(30)  // 30 seconds default
//!     .route_timeout("/api/upload", 300)  // 5 minutes for uploads
//!     .route_timeout("/api/report", 120);  // 2 minutes for reports
//! ```
//!
//! ## Using the Decorator
//!
//! ```ignore
//! use armature::{get, timeout};
//!
//! #[timeout(5)]  // 5 second timeout
//! #[get("/quick")]
//! async fn quick_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
//!     Ok(HttpResponse::ok())
//! }
//! ```

use crate::{Error, HttpRequest, HttpResponse};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for request timeouts.
///
/// Allows setting default and route-specific timeouts for request handling.
///
/// ## Example
///
/// ```rust
/// use armature_core::timeout::TimeoutConfig;
///
/// let config = TimeoutConfig::new()
///     .default_timeout(30)                    // 30 seconds default
///     .route_timeout("/api/slow", 120)       // 2 minutes for slow endpoints
///     .route_timeout_ms("/api/fast", 500);   // 500ms for fast endpoints
/// ```
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Default timeout for all requests
    pub default: Duration,
    /// Route-specific timeouts (path pattern -> timeout)
    route_timeouts: HashMap<String, Duration>,
    /// Whether to include timeout info in error responses
    pub include_timeout_in_error: bool,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            default: Duration::from_secs(30),
            route_timeouts: HashMap::new(),
            include_timeout_in_error: true,
        }
    }
}

impl TimeoutConfig {
    /// Creates a new timeout configuration with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the default timeout in seconds.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use armature_core::timeout::TimeoutConfig;
    ///
    /// let config = TimeoutConfig::new().default_timeout(60);
    /// ```
    pub fn default_timeout(mut self, seconds: u64) -> Self {
        self.default = Duration::from_secs(seconds);
        self
    }

    /// Sets the default timeout in milliseconds.
    pub fn default_timeout_ms(mut self, ms: u64) -> Self {
        self.default = Duration::from_millis(ms);
        self
    }

    /// Sets the default timeout from a Duration.
    pub fn default_timeout_duration(mut self, duration: Duration) -> Self {
        self.default = duration;
        self
    }

    /// Adds a route-specific timeout in seconds.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use armature_core::timeout::TimeoutConfig;
    ///
    /// let config = TimeoutConfig::new()
    ///     .route_timeout("/api/upload", 300)    // 5 minutes
    ///     .route_timeout("/api/export", 120);   // 2 minutes
    /// ```
    pub fn route_timeout(mut self, path: &str, seconds: u64) -> Self {
        self.route_timeouts
            .insert(path.to_string(), Duration::from_secs(seconds));
        self
    }

    /// Adds a route-specific timeout in milliseconds.
    pub fn route_timeout_ms(mut self, path: &str, ms: u64) -> Self {
        self.route_timeouts
            .insert(path.to_string(), Duration::from_millis(ms));
        self
    }

    /// Adds a route-specific timeout from a Duration.
    pub fn route_timeout_duration(mut self, path: &str, duration: Duration) -> Self {
        self.route_timeouts.insert(path.to_string(), duration);
        self
    }

    /// Sets whether to include timeout duration in error messages.
    pub fn include_timeout_in_error(mut self, include: bool) -> Self {
        self.include_timeout_in_error = include;
        self
    }

    /// Gets the timeout for a specific path.
    ///
    /// Returns the route-specific timeout if configured, otherwise the default.
    ///
    /// Resolution order:
    /// 1. An exact match against a configured route pattern wins outright.
    /// 2. Otherwise, among all configured patterns that are a prefix of `path`,
    ///    the **longest** matching pattern wins (longest-prefix-wins). This is
    ///    deterministic regardless of `HashMap` iteration order. Ties between
    ///    equal-length patterns are broken by string (byte) ordering, so the
    ///    result is stable across process restarts.
    /// 3. If no pattern matches, the configured default timeout is returned.
    pub fn get_timeout_for_path(&self, path: &str) -> Duration {
        // Check for exact match first
        if let Some(timeout) = self.route_timeouts.get(path) {
            return *timeout;
        }

        // Check for prefix matches (e.g., "/api/upload" matches "/api/upload/123").
        // Longest-prefix-wins, with ties broken deterministically by pattern
        // string ordering, so the result does not depend on HashMap iteration
        // order (which is randomized per-process).
        let mut best: Option<(&str, Duration)> = None;
        for (pattern, timeout) in &self.route_timeouts {
            if path.starts_with(pattern.as_str()) {
                match best {
                    Some((best_pattern, _))
                        if pattern.len() < best_pattern.len()
                            || (pattern.len() == best_pattern.len()
                                && pattern.as_str() <= best_pattern) => {}
                    _ => best = Some((pattern.as_str(), *timeout)),
                }
            }
        }

        if let Some((_, timeout)) = best {
            return timeout;
        }

        self.default
    }

    /// Creates a TimeoutMiddleware from this configuration.
    pub fn into_middleware(self) -> ConfigurableTimeoutMiddleware {
        ConfigurableTimeoutMiddleware::new(self)
    }
}

/// A simple timeout middleware with a fixed duration.
///
/// For more advanced use cases with per-route timeouts, use `ConfigurableTimeoutMiddleware`.
///
/// ## Example
///
/// ```rust
/// use armature_core::timeout::TimeoutMiddleware;
///
/// // Create middleware with 30 second timeout
/// let middleware = TimeoutMiddleware::new(30);
///
/// // Create middleware with millisecond precision
/// let fast_middleware = TimeoutMiddleware::from_millis(500);
/// ```
#[derive(Debug, Clone)]
pub struct TimeoutMiddleware {
    duration: Duration,
}

impl TimeoutMiddleware {
    /// Creates a new timeout middleware with the specified timeout in seconds.
    pub fn new(seconds: u64) -> Self {
        Self {
            duration: Duration::from_secs(seconds),
        }
    }

    /// Creates a new timeout middleware with the specified timeout in milliseconds.
    pub fn from_millis(ms: u64) -> Self {
        Self {
            duration: Duration::from_millis(ms),
        }
    }

    /// Creates a new timeout middleware from a Duration.
    pub fn from_duration(duration: Duration) -> Self {
        Self { duration }
    }

    /// Returns the configured timeout duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

#[async_trait]
impl crate::middleware::Middleware for TimeoutMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: crate::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        match tokio::time::timeout(self.duration, next(req)).await {
            Ok(result) => result,
            Err(_) => Err(Error::RequestTimeout(format!(
                "Request exceeded timeout of {:?}",
                self.duration
            ))),
        }
    }
}

/// A configurable timeout middleware that supports per-route timeouts.
///
/// ## Example
///
/// ```rust
/// use armature_core::timeout::{TimeoutConfig, ConfigurableTimeoutMiddleware};
///
/// let config = TimeoutConfig::new()
///     .default_timeout(30)
///     .route_timeout("/api/upload", 300);
///
/// let middleware = ConfigurableTimeoutMiddleware::new(config);
/// ```
#[derive(Debug, Clone)]
pub struct ConfigurableTimeoutMiddleware {
    config: Arc<TimeoutConfig>,
}

impl ConfigurableTimeoutMiddleware {
    /// Creates a new configurable timeout middleware.
    pub fn new(config: TimeoutConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Creates a middleware with a simple default timeout.
    pub fn with_default(seconds: u64) -> Self {
        Self::new(TimeoutConfig::new().default_timeout(seconds))
    }
}

#[async_trait]
impl crate::middleware::Middleware for ConfigurableTimeoutMiddleware {
    async fn handle(
        &self,
        req: HttpRequest,
        next: crate::middleware::Next,
    ) -> Result<HttpResponse, Error> {
        let timeout = self.config.get_timeout_for_path(&req.path);

        match tokio::time::timeout(timeout, next(req)).await {
            Ok(result) => result,
            Err(_) => {
                let error_msg = if self.config.include_timeout_in_error {
                    format!("Request exceeded timeout of {:?}", timeout)
                } else {
                    "Request timeout".to_string()
                };
                Err(Error::RequestTimeout(error_msg))
            }
        }
    }
}

/// Builder for creating timeout middleware with a fluent API.
#[derive(Debug, Clone)]
pub struct TimeoutBuilder {
    config: TimeoutConfig,
}

impl Default for TimeoutBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeoutBuilder {
    /// Creates a new timeout builder.
    pub fn new() -> Self {
        Self {
            config: TimeoutConfig::default(),
        }
    }

    /// Sets the default timeout in seconds.
    pub fn default_seconds(mut self, seconds: u64) -> Self {
        self.config.default = Duration::from_secs(seconds);
        self
    }

    /// Sets the default timeout in milliseconds.
    pub fn default_millis(mut self, ms: u64) -> Self {
        self.config.default = Duration::from_millis(ms);
        self
    }

    /// Adds a route-specific timeout in seconds.
    pub fn route(mut self, path: &str, seconds: u64) -> Self {
        self.config
            .route_timeouts
            .insert(path.to_string(), Duration::from_secs(seconds));
        self
    }

    /// Adds a route-specific timeout in milliseconds.
    pub fn route_millis(mut self, path: &str, ms: u64) -> Self {
        self.config
            .route_timeouts
            .insert(path.to_string(), Duration::from_millis(ms));
        self
    }

    /// Controls whether timeout duration appears in error messages.
    pub fn show_timeout_in_error(mut self, show: bool) -> Self {
        self.config.include_timeout_in_error = show;
        self
    }

    /// Builds a ConfigurableTimeoutMiddleware.
    pub fn build(self) -> ConfigurableTimeoutMiddleware {
        ConfigurableTimeoutMiddleware::new(self.config)
    }

    /// Builds a simple TimeoutMiddleware (uses default timeout only).
    pub fn build_simple(self) -> TimeoutMiddleware {
        TimeoutMiddleware::from_duration(self.config.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_config_default() {
        let config = TimeoutConfig::new();
        assert_eq!(config.default, Duration::from_secs(30));
        assert!(config.route_timeouts.is_empty());
    }

    #[test]
    fn test_timeout_config_custom_default() {
        let config = TimeoutConfig::new().default_timeout(60);
        assert_eq!(config.default, Duration::from_secs(60));
    }

    #[test]
    fn test_timeout_config_route_specific() {
        let config = TimeoutConfig::new()
            .default_timeout(30)
            .route_timeout("/api/upload", 300)
            .route_timeout("/api/fast", 5);

        assert_eq!(
            config.get_timeout_for_path("/api/upload"),
            Duration::from_secs(300)
        );
        assert_eq!(
            config.get_timeout_for_path("/api/fast"),
            Duration::from_secs(5)
        );
        assert_eq!(
            config.get_timeout_for_path("/api/other"),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_timeout_config_prefix_matching() {
        let config = TimeoutConfig::new()
            .default_timeout(30)
            .route_timeout("/api/upload", 300);

        // Prefix matching should work
        assert_eq!(
            config.get_timeout_for_path("/api/upload/123"),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn test_timeout_config_prefix_matching_longest_wins() {
        // Two overlapping prefixes are configured with different timeouts.
        // The longer (more specific) matching prefix must win, regardless of
        // HashMap iteration order.
        let config = TimeoutConfig::new()
            .default_timeout(30)
            .route_timeout("/api", 60)
            .route_timeout("/api/upload", 300);

        assert_eq!(
            config.get_timeout_for_path("/api/upload/123"),
            Duration::from_secs(300)
        );

        // A path matching only the shorter prefix still gets that timeout.
        assert_eq!(
            config.get_timeout_for_path("/api/other"),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn test_timeout_middleware_creation() {
        let middleware = TimeoutMiddleware::new(30);
        assert_eq!(middleware.duration(), Duration::from_secs(30));

        let middleware_ms = TimeoutMiddleware::from_millis(500);
        assert_eq!(middleware_ms.duration(), Duration::from_millis(500));
    }

    #[test]
    fn test_timeout_builder() {
        let middleware = TimeoutBuilder::new()
            .default_seconds(60)
            .route("/api/upload", 300)
            .route_millis("/api/fast", 100)
            .show_timeout_in_error(false)
            .build();

        assert_eq!(
            middleware.config.get_timeout_for_path("/"),
            Duration::from_secs(60)
        );
        assert_eq!(
            middleware.config.get_timeout_for_path("/api/upload"),
            Duration::from_secs(300)
        );
        assert_eq!(
            middleware.config.get_timeout_for_path("/api/fast"),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn test_timeout_config_milliseconds() {
        let config = TimeoutConfig::new()
            .default_timeout_ms(500)
            .route_timeout_ms("/api/realtime", 100);

        assert_eq!(config.default, Duration::from_millis(500));
        assert_eq!(
            config.get_timeout_for_path("/api/realtime"),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn test_timeout_config_into_middleware() {
        let config = TimeoutConfig::new().default_timeout(45);
        let middleware = config.into_middleware();

        assert_eq!(middleware.config.default, Duration::from_secs(45));
    }
}
