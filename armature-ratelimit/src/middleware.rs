//! Rate limiting middleware for Armature
//!
//! This module provides middleware that can be used with the Armature framework
//! to add rate limiting to your application.

use crate::RateLimiter;
use crate::error::RateLimitHeaders;
use crate::extractor::{KeyExtractor, RequestInfo};
use std::net::IpAddr;
use std::sync::Arc;
use tracing::{debug, info, trace, warn};

/// Rate limiting middleware for Armature applications
pub struct RateLimitMiddleware {
    /// The rate limiter instance
    limiter: Arc<RateLimiter>,
    /// Key extraction strategy
    key_extractor: KeyExtractor,
    /// Whether to add rate limit headers to responses
    include_headers: bool,
    /// Custom message for rate limit exceeded responses
    error_message: String,
    /// Keys that bypass rate limiting
    bypass_keys: Vec<String>,
    /// When the store errors: `true` fails open (allow the request), `false`
    /// fails closed (deny). Mirrors `RateLimitConfig::skip_on_error`.
    skip_on_error: bool,
}

/// Parse only the **first hop** of an `X-Forwarded-For` value.
///
/// `X-Forwarded-For` is a comma-separated list `client, proxy1, proxy2, ...`;
/// the originating client is the first entry. Parsing the whole header string
/// as a single `IpAddr` always fails on multi-hop traffic, which previously
/// collapsed the extracted key to `None` and silently **disabled IP rate
/// limiting for all proxied requests** (a fail-open security gap).
fn first_forwarded_ip(xff: &str) -> Option<IpAddr> {
    xff.split(',').next()?.trim().parse().ok()
}

impl RateLimitMiddleware {
    /// Create a new rate limit middleware
    pub fn new(limiter: Arc<RateLimiter>) -> Self {
        let config = limiter.config().clone();
        Self {
            limiter,
            key_extractor: KeyExtractor::Ip,
            include_headers: config.include_headers,
            skip_on_error: config.skip_on_error,
            error_message: config
                .error_message
                .unwrap_or_else(|| "Rate limit exceeded".to_string()),
            bypass_keys: config.bypass_keys,
        }
    }

    /// Create middleware with a custom key extractor
    pub fn with_extractor(mut self, extractor: KeyExtractor) -> Self {
        self.key_extractor = extractor;
        self
    }

    /// Set whether to include rate limit headers
    pub fn with_headers(mut self, include: bool) -> Self {
        self.include_headers = include;
        self
    }

    /// Set custom error message
    pub fn with_error_message(mut self, message: impl Into<String>) -> Self {
        self.error_message = message.into();
        self
    }

    /// Add bypass keys
    pub fn with_bypass_keys(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.bypass_keys.extend(keys.into_iter().map(|k| k.into()));
        self
    }

    /// Check if a request should be rate limited
    ///
    /// Returns Ok with headers if allowed, Err with response if rate limited.
    pub async fn check(&self, info: &RequestInfo) -> RateLimitCheckResponse {
        // Extract key
        let key = match self.key_extractor.extract(info) {
            Some(k) => k,
            None => {
                warn!("Could not extract rate limit key, allowing request");
                return RateLimitCheckResponse::Allowed { headers: None };
            }
        };

        trace!(key = %key, "Checking rate limit");

        // Check bypass
        if self.bypass_keys.contains(&key) {
            debug!(key = %key, "Key is in bypass list, allowing request");
            return RateLimitCheckResponse::Allowed { headers: None };
        }

        // Check rate limit
        match self.limiter.check(&key).await {
            Ok(result) => {
                let headers = if self.include_headers {
                    Some(RateLimitHeaders::allowed(
                        result.limit,
                        result.remaining,
                        result.reset_at,
                    ))
                } else {
                    None
                };

                if result.allowed {
                    trace!(key = %key, remaining = result.remaining, "Request allowed");
                    RateLimitCheckResponse::Allowed { headers }
                } else {
                    info!(key = %key, retry_after = ?result.retry_after, "Rate limit exceeded");
                    RateLimitCheckResponse::Limited {
                        headers: if self.include_headers {
                            Some(RateLimitHeaders::denied(
                                result.limit,
                                result.reset_at,
                                result.retry_after.map(|d| d.as_secs()).unwrap_or(1),
                            ))
                        } else {
                            None
                        },
                        message: self.error_message.clone(),
                        retry_after: result.retry_after.map(|d| d.as_secs()),
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Rate limit check failed");
                if self.skip_on_error {
                    // Fail open: the store is unavailable but the deployment has
                    // opted to let traffic through rather than reject it.
                    RateLimitCheckResponse::Allowed { headers: None }
                } else {
                    // Fail closed: `skip_on_error(false)` means a store error
                    // must deny the request, not silently disable rate limiting.
                    RateLimitCheckResponse::Limited {
                        headers: None,
                        message: self.error_message.clone(),
                        retry_after: None,
                    }
                }
            }
        }
    }

    /// Extract request info from common HTTP request types
    pub fn extract_request_info(
        ip: Option<IpAddr>,
        path: &str,
        method: &str,
        user_id: Option<&str>,
        headers: &[(String, String)],
    ) -> RequestInfo {
        let mut info = RequestInfo::new(path, method);

        if let Some(ip) = ip {
            info = info.with_ip(ip);
        }

        if let Some(uid) = user_id {
            info = info.with_user_id(uid);
        }

        for (name, value) in headers {
            info = info.with_header(name.clone(), value.clone());
        }

        // Try to extract API key from common headers
        for (name, value) in headers {
            let name_lower = name.to_lowercase();
            if name_lower == "x-api-key" || name_lower == "authorization" {
                info = info.with_api_key(value.clone());
                break;
            }
        }

        info
    }

    /// Get the underlying rate limiter
    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }
}

/// Response from rate limit check
#[derive(Debug)]
pub enum RateLimitCheckResponse {
    /// Request is allowed
    Allowed {
        /// Rate limit headers to include in response
        headers: Option<RateLimitHeaders>,
    },
    /// Request is rate limited
    Limited {
        /// Rate limit headers to include in response
        headers: Option<RateLimitHeaders>,
        /// Error message
        message: String,
        /// Seconds until the client should retry
        retry_after: Option<u64>,
    },
}

impl RateLimitCheckResponse {
    /// Check if the request is allowed
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Check if the request is limited
    pub fn is_limited(&self) -> bool {
        matches!(self, Self::Limited { .. })
    }

    /// Get the headers if any
    pub fn headers(&self) -> Option<&RateLimitHeaders> {
        match self {
            Self::Allowed { headers } => headers.as_ref(),
            Self::Limited { headers, .. } => headers.as_ref(),
        }
    }

    /// Get the error message if limited
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Limited { message, .. } => Some(message),
            _ => None,
        }
    }

    /// Get the retry-after value if limited
    pub fn retry_after(&self) -> Option<u64> {
        match self {
            Self::Limited { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Builder for configuring rate limit middleware
pub struct RateLimitMiddlewareBuilder {
    limiter: Option<Arc<RateLimiter>>,
    key_extractor: KeyExtractor,
    include_headers: bool,
    error_message: Option<String>,
    bypass_keys: Vec<String>,
}

impl RateLimitMiddlewareBuilder {
    /// Create a new middleware builder
    pub fn new() -> Self {
        Self {
            limiter: None,
            key_extractor: KeyExtractor::Ip,
            include_headers: true,
            error_message: None,
            bypass_keys: Vec::new(),
        }
    }

    /// Set the rate limiter
    pub fn limiter(mut self, limiter: Arc<RateLimiter>) -> Self {
        self.limiter = Some(limiter);
        self
    }

    /// Set the key extractor
    pub fn key_extractor(mut self, extractor: KeyExtractor) -> Self {
        self.key_extractor = extractor;
        self
    }

    /// Extract key from IP address
    pub fn by_ip(mut self) -> Self {
        self.key_extractor = KeyExtractor::Ip;
        self
    }

    /// Extract key from user ID
    pub fn by_user_id(mut self) -> Self {
        self.key_extractor = KeyExtractor::UserId;
        self
    }

    /// Extract key from API key header
    pub fn by_api_key(mut self, header_name: impl Into<String>) -> Self {
        self.key_extractor = KeyExtractor::ApiKey {
            header_name: header_name.into(),
        };
        self
    }

    /// Extract key from IP and path combination
    pub fn by_ip_and_path(mut self) -> Self {
        self.key_extractor = KeyExtractor::IpAndPath;
        self
    }

    /// Include rate limit headers in responses
    pub fn include_headers(mut self, include: bool) -> Self {
        self.include_headers = include;
        self
    }

    /// Set custom error message
    pub fn error_message(mut self, message: impl Into<String>) -> Self {
        self.error_message = Some(message.into());
        self
    }

    /// Add a bypass key
    pub fn bypass_key(mut self, key: impl Into<String>) -> Self {
        self.bypass_keys.push(key.into());
        self
    }

    /// Build the middleware
    pub fn build(self) -> Option<RateLimitMiddleware> {
        let limiter = self.limiter?;

        let mut middleware = RateLimitMiddleware::new(limiter)
            .with_extractor(self.key_extractor)
            .with_headers(self.include_headers)
            .with_bypass_keys(self.bypass_keys);

        if let Some(msg) = self.error_message {
            middleware = middleware.with_error_message(msg);
        }

        Some(middleware)
    }
}

impl Default for RateLimitMiddlewareBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement the armature_core::Middleware trait for RateLimitMiddleware
/// This allows it to be used in a MiddlewareChain
#[async_trait::async_trait]
impl armature_core::Middleware for RateLimitMiddleware {
    async fn handle(
        &self,
        req: armature_core::HttpRequest,
        next: Box<
            dyn FnOnce(
                    armature_core::HttpRequest,
                ) -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<
                                Output = Result<armature_core::HttpResponse, armature_core::Error>,
                            > + Send,
                    >,
                > + Send,
        >,
    ) -> Result<armature_core::HttpResponse, armature_core::Error> {
        // Extract request info
        // Prefer the first hop of X-Forwarded-For (the originating client);
        // fall back to X-Real-IP. Parsing the whole comma-separated XFF as one
        // IpAddr fails on any multi-hop chain, which used to disable IP rate
        // limiting for all proxied traffic.
        let ip = req
            .headers
            .get("x-forwarded-for")
            .and_then(|s| first_forwarded_ip(s))
            .or_else(|| {
                req.headers
                    .get("x-real-ip")
                    .and_then(|s| s.trim().parse().ok())
            });

        let user_id = req.headers.get("x-user-id").map(|s| s.as_str());

        let headers: Vec<(String, String)> = req
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let info = Self::extract_request_info(ip, &req.path, &req.method, user_id, &headers);

        // Check rate limit
        match self.check(&info).await {
            RateLimitCheckResponse::Allowed { headers } => {
                // Request is allowed, call next handler
                let mut response = next(req).await?;

                // Add rate limit headers if present
                if let Some(h) = headers {
                    response
                        .headers
                        .insert("X-RateLimit-Limit".to_string(), h.limit.to_string());
                    response
                        .headers
                        .insert("X-RateLimit-Remaining".to_string(), h.remaining.to_string());
                    response
                        .headers
                        .insert("X-RateLimit-Reset".to_string(), h.reset.to_string());
                }

                Ok(response)
            }
            RateLimitCheckResponse::Limited {
                headers,
                message,
                retry_after,
            } => {
                // Request is rate limited
                let mut response = armature_core::HttpResponse::new(429).with_body(
                    serde_json::json!({
                        "error": "Too Many Requests",
                        "message": message
                    })
                    .to_string()
                    .into_bytes(),
                );

                response
                    .headers
                    .insert("Content-Type".to_string(), "application/json".to_string());

                if let Some(retry) = retry_after {
                    response
                        .headers
                        .insert("Retry-After".to_string(), retry.to_string());
                }

                if let Some(h) = headers {
                    response
                        .headers
                        .insert("X-RateLimit-Limit".to_string(), h.limit.to_string());
                    response
                        .headers
                        .insert("X-RateLimit-Remaining".to_string(), "0".to_string());
                    response
                        .headers
                        .insert("X-RateLimit-Reset".to_string(), h.reset.to_string());
                }

                Ok(response)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::Algorithm;
    use std::net::Ipv4Addr;

    async fn create_test_limiter() -> Arc<RateLimiter> {
        Arc::new(
            RateLimiter::builder()
                .algorithm(Algorithm::TokenBucket {
                    capacity: 5,
                    refill_rate: 1.0,
                })
                .build()
                .await
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn test_middleware_allows_requests() {
        let limiter = create_test_limiter().await;
        let middleware = RateLimitMiddleware::new(limiter);

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        let response = middleware.check(&info).await;
        assert!(response.is_allowed());
        assert!(response.headers().is_some());
    }

    #[tokio::test]
    async fn test_middleware_limits_requests() {
        let limiter = create_test_limiter().await;
        let middleware = RateLimitMiddleware::new(limiter);

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        // Use up all tokens
        for _ in 0..5 {
            middleware.check(&info).await;
        }

        // Next request should be limited
        let response = middleware.check(&info).await;
        assert!(response.is_limited());
        assert!(response.message().is_some());
    }

    #[tokio::test]
    async fn test_bypass_keys() {
        let limiter = create_test_limiter().await;
        let middleware = RateLimitMiddleware::new(limiter).with_bypass_keys(["admin_key"]);

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        // Use up all tokens
        for _ in 0..10 {
            middleware.check(&info).await;
        }

        // Need to use API key extractor for this
        let middleware_api = RateLimitMiddleware::new(Arc::new(
            RateLimiter::builder()
                .token_bucket(1, 0.001)
                .bypass_key("admin_key")
                .build()
                .await
                .unwrap(),
        ))
        .with_extractor(KeyExtractor::ApiKey {
            header_name: "X-API-Key".to_string(),
        })
        .with_bypass_keys(["admin_key"]);

        let bypass_info =
            RequestInfo::new("/api/test", "GET").with_header("X-API-Key", "admin_key");

        let response = middleware_api.check(&bypass_info).await;
        assert!(response.is_allowed());
    }

    #[tokio::test]
    async fn test_custom_error_message() {
        let limiter = Arc::new(
            RateLimiter::builder()
                .token_bucket(1, 0.001)
                .build()
                .await
                .unwrap(),
        );
        let middleware =
            RateLimitMiddleware::new(limiter).with_error_message("Custom rate limit message");

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        // Use up token
        middleware.check(&info).await;

        // Next request should have custom message
        let response = middleware.check(&info).await;
        assert!(response.is_limited());
        assert_eq!(response.message(), Some("Custom rate limit message"));
    }

    #[tokio::test]
    async fn test_extract_request_info() {
        let headers = vec![
            ("X-API-Key".to_string(), "test_key".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        let info = RateLimitMiddleware::extract_request_info(
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))),
            "/api/users",
            "POST",
            Some("user_123"),
            &headers,
        );

        assert_eq!(info.ip, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert_eq!(info.path, "/api/users");
        assert_eq!(info.method, "POST");
        assert_eq!(info.user_id, Some("user_123".to_string()));
        assert_eq!(info.api_key, Some("test_key".to_string()));
    }

    #[test]
    fn test_first_forwarded_ip_parses_first_hop() {
        // Multi-hop chain: the client is the first entry.
        assert_eq!(
            first_forwarded_ip("203.0.113.5, 70.41.3.18, 150.172.238.178"),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)))
        );
        // Single hop, no whitespace.
        assert_eq!(
            first_forwarded_ip("198.51.100.7"),
            Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)))
        );
        // Garbage first hop yields None (not a silent pass-through).
        assert_eq!(first_forwarded_ip("not-an-ip, 10.0.0.1"), None);
    }

    /// Regression: a multi-hop `X-Forwarded-For` used to be parsed as a single
    /// `IpAddr`, fail, drop the key to `None`, and fail **open** — proxied IP
    /// rate limiting was silently disabled. The middleware must now key on the
    /// first hop and actually enforce the limit (429 once exhausted).
    #[tokio::test]
    async fn test_multi_hop_xff_rate_limits_on_client() {
        use armature_core::Middleware;

        let limiter = Arc::new(
            RateLimiter::builder()
                .token_bucket(1, f64::EPSILON)
                .build()
                .await
                .unwrap(),
        );
        let middleware = RateLimitMiddleware::new(limiter);

        let make_req = || {
            let mut req =
                armature_core::HttpRequest::new("GET".to_string(), "/api/test".to_string());
            req.headers.insert(
                "x-forwarded-for",
                "203.0.113.5, 70.41.3.18, 150.172.238.178",
            );
            req
        };

        // First request consumes the single token -> allowed.
        let r1 = middleware
            .handle(
                make_req(),
                Box::new(|_r| Box::pin(async { Ok(armature_core::HttpResponse::ok()) })),
            )
            .await
            .unwrap();
        assert_eq!(r1.status, 200);

        // Second request must be rate limited on the client's first hop.
        let r2 = middleware
            .handle(
                make_req(),
                Box::new(|_r| Box::pin(async { Ok(armature_core::HttpResponse::ok()) })),
            )
            .await
            .unwrap();
        assert_eq!(
            r2.status, 429,
            "proxied client must be rate limited, not failed open"
        );
    }

    /// A store that always errors, used to exercise the fail-open/closed path.
    struct ErrorStore;

    #[async_trait::async_trait]
    impl crate::stores::RateLimitStore for ErrorStore {
        async fn token_bucket_check(
            &self,
            _key: &str,
            _capacity: u64,
            _refill_rate: f64,
        ) -> crate::error::RateLimitResult<(bool, u64)> {
            Err(crate::error::RateLimitError::store("store down"))
        }
        async fn sliding_window_check(
            &self,
            _key: &str,
            _max_requests: u64,
            _window: std::time::Duration,
        ) -> crate::error::RateLimitResult<(bool, u64)> {
            Err(crate::error::RateLimitError::store("store down"))
        }
        async fn fixed_window_check(
            &self,
            _key: &str,
            _max_requests: u64,
            _window: std::time::Duration,
        ) -> crate::error::RateLimitResult<(bool, u64)> {
            Err(crate::error::RateLimitError::store("store down"))
        }
        async fn reset(&self, _key: &str) -> crate::error::RateLimitResult<()> {
            Ok(())
        }
        async fn remaining(&self, _key: &str) -> crate::error::RateLimitResult<u64> {
            Ok(0)
        }
        fn store_type(&self) -> &'static str {
            "error"
        }
    }

    /// Regression: `skip_on_error(false)` must fail **closed** — a store error
    /// denies the request. Previously the middleware always failed open.
    #[tokio::test]
    async fn test_skip_on_error_false_fails_closed() {
        use crate::config::RateLimitConfig;

        let config = RateLimitConfig {
            skip_on_error: false,
            ..Default::default()
        };
        let limiter = Arc::new(RateLimiter::new(
            Arc::new(ErrorStore),
            config.algorithm.clone(),
            config,
        ));
        let middleware = RateLimitMiddleware::new(limiter);

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        let response = middleware.check(&info).await;
        assert!(
            response.is_limited(),
            "skip_on_error(false) must deny on store error"
        );
    }

    /// Complement: `skip_on_error(true)` (the default) still fails open.
    #[tokio::test]
    async fn test_skip_on_error_true_fails_open() {
        use crate::config::RateLimitConfig;

        let config = RateLimitConfig {
            skip_on_error: true,
            ..Default::default()
        };
        let limiter = Arc::new(RateLimiter::new(
            Arc::new(ErrorStore),
            config.algorithm.clone(),
            config,
        ));
        let middleware = RateLimitMiddleware::new(limiter);

        let info =
            RequestInfo::new("/api/test", "GET").with_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        let response = middleware.check(&info).await;
        assert!(
            response.is_allowed(),
            "skip_on_error(true) must allow on store error"
        );
    }

    #[test]
    fn test_response_methods() {
        let allowed = RateLimitCheckResponse::Allowed { headers: None };
        assert!(allowed.is_allowed());
        assert!(!allowed.is_limited());
        assert!(allowed.message().is_none());
        assert!(allowed.retry_after().is_none());

        let limited = RateLimitCheckResponse::Limited {
            headers: None,
            message: "Too many requests".to_string(),
            retry_after: Some(60),
        };
        assert!(!limited.is_allowed());
        assert!(limited.is_limited());
        assert_eq!(limited.message(), Some("Too many requests"));
        assert_eq!(limited.retry_after(), Some(60));
    }
}
