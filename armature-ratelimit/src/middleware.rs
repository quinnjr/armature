//! Rate limiting middleware for Armature
//!
//! This module provides middleware that can be used with the Armature framework
//! to add rate limiting to your application.

use crate::RateLimiter;
use crate::error::RateLimitHeaders;
use crate::extractor::{KeyExtractor, RequestInfo};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Number of trusted reverse proxies in front of the app. Controls how the
    /// client IP is read from `X-Forwarded-For`; `0` (default) trusts no proxy
    /// and ignores the header. Mirrors `RateLimitConfig::trusted_proxy_depth`.
    trusted_proxy_depth: usize,
    /// Count of store errors that were let through because `skip_on_error` is
    /// `true`, observed by this middleware instance. Incremented every time
    /// [`RateLimiter::check`] returns `Err` and the fail-open path is taken.
    /// This is the minimum observable signal for the default fail-open
    /// posture: with zero counters or logs, a backend outage silently
    /// disables rate limiting (an abuse/brute-force control on endpoints like
    /// login, password reset, or OTP) with nothing to detect it happened. See
    /// [`Self::skip_on_error_count`]. Mirrors
    /// `AuditMiddleware::write_failures` (armature-audit).
    skip_on_error_count: AtomicU64,
}

/// Select the client IP from an `X-Forwarded-For` value using the number of
/// trusted proxies (`depth`).
///
/// `X-Forwarded-For` is a comma-separated list `client, proxy1, proxy2, ...`
/// where each proxy *appends* the address it received the connection from, so
/// the **rightmost** hops are the ones added by your own infrastructure and are
/// the only trustworthy entries. The client is therefore selected
/// `depth`-from-the-right (1-indexed): with a single trusted proxy (`depth == 1`)
/// the rightmost hop is the client address your proxy observed.
///
/// A `depth` of `0` means **no proxy is trusted** — every entry is
/// attacker-controlled, so this returns `None` and the caller does not derive an
/// IP from the header at all. Trusting the first (leftmost) hop unconditionally,
/// as the code previously did, let an attacker rotate that hop per request to
/// mint a fresh bucket each time (limit bypass) or spoof a victim's IP (pollution).
///
/// Note: `armature_core::HttpRequest` does not currently expose the socket peer
/// address, so when XFF trust is not configured there is no non-spoofable peer to
/// fall back to; IP-based limiting is simply not applied to such requests. Enable
/// it explicitly by setting the trusted-proxy depth.
fn forwarded_ip_at_depth(xff: &str, depth: usize) -> Option<IpAddr> {
    if depth == 0 {
        return None;
    }
    let hops: Vec<&str> = xff
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    // `depth` counts from the right; out-of-range depth trusts nothing.
    let idx = hops.len().checked_sub(depth)?;
    hops.get(idx)?.parse().ok()
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
            trusted_proxy_depth: config.trusted_proxy_depth,
            skip_on_error_count: AtomicU64::new(0),
        }
    }

    /// Set the number of trusted reverse proxies in front of the app.
    ///
    /// Controls how the client IP is derived from `X-Forwarded-For`. Defaults to
    /// `0` (no proxy trusted; the header is ignored). See
    /// [`crate::config::RateLimitConfig::trusted_proxy_depth`].
    pub fn with_trusted_proxy_depth(mut self, depth: usize) -> Self {
        self.trusted_proxy_depth = depth;
        self
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
                    // opted to let traffic through rather than reject it. This is
                    // silent to a caller that isn't watching logs/metrics, so
                    // record it and warn loudly: rate limiting is commonly relied
                    // on as an abuse/brute-force control (login, password reset,
                    // OTP endpoints), and a backend outage bypasses it for
                    // everyone on the default `skip_on_error: true`.
                    self.skip_on_error_count.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        error = %e,
                        "rate limiter backend error, skip_on_error is enabled — request was allowed through unchecked"
                    );
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

    /// Number of store errors observed by this middleware instance that were
    /// let through because `skip_on_error` is `true`.
    ///
    /// Incremented every time a backend/store error occurs and the fail-open
    /// path is taken, independent of whether headers or logging are enabled.
    /// Wire this into whatever metrics system the application uses (e.g.
    /// export it as a `ratelimit_skip_on_error_total` counter) to get an
    /// observable signal that rate limiting was bypassed due to a backend
    /// outage, even under the default fail-open configuration. See
    /// [`crate::config::RateLimitConfig::skip_on_error`].
    pub fn skip_on_error_count(&self) -> u64 {
        self.skip_on_error_count.load(Ordering::Relaxed)
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
        // Derive the client IP from forwarding headers only when a trusted-proxy
        // depth is configured. `X-Forwarded-For`/`X-Real-IP` are attacker-
        // controlled unless the request is known to have transited a trusted
        // proxy, so with the default depth of 0 they are ignored entirely (an
        // attacker cannot rotate/spoof them to bypass or pollute rate limits).
        let ip = if self.trusted_proxy_depth == 0 {
            None
        } else {
            req.headers
                .get("x-forwarded-for")
                .and_then(|s| forwarded_ip_at_depth(s, self.trusted_proxy_depth))
                .or_else(|| {
                    req.headers
                        .get("x-real-ip")
                        .and_then(|s| s.trim().parse().ok())
                })
        };

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
    fn test_forwarded_ip_at_depth_selects_from_right() {
        let xff = "203.0.113.5, 70.41.3.18, 150.172.238.178";
        // depth 0: no proxy trusted, header ignored entirely.
        assert_eq!(forwarded_ip_at_depth(xff, 0), None);
        // depth 1: rightmost hop (added by our proxy).
        assert_eq!(
            forwarded_ip_at_depth(xff, 1),
            Some(IpAddr::V4(Ipv4Addr::new(150, 172, 238, 178)))
        );
        // depth 2: second from the right.
        assert_eq!(
            forwarded_ip_at_depth(xff, 2),
            Some(IpAddr::V4(Ipv4Addr::new(70, 41, 3, 18)))
        );
        // depth 3: the originating client (leftmost of a 3-hop chain).
        assert_eq!(
            forwarded_ip_at_depth(xff, 3),
            Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)))
        );
        // depth beyond the number of hops trusts nothing.
        assert_eq!(forwarded_ip_at_depth(xff, 4), None);
    }

    /// Regression: with the client's originating IP trusted (all 3 proxies
    /// configured), a multi-hop `X-Forwarded-For` must actually enforce the
    /// limit on that client (429 once exhausted). Previously the whole header
    /// was parsed as a single `IpAddr`, failed, dropped the key to `None`, and
    /// failed **open** — proxied IP rate limiting was silently disabled.
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
        // Trust all 3 hops so the leftmost (originating client) is the key.
        let middleware = RateLimitMiddleware::new(limiter).with_trusted_proxy_depth(3);

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

        // Second request must be rate limited on the client's IP.
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

    /// Security regression: an attacker who can set `X-Forwarded-For` used to
    /// mint a fresh bucket per request by rotating the (spoofable) first hop,
    /// because the middleware unconditionally trusted it. With one trusted proxy
    /// configured, the client is read from the *rightmost* hop (added by our
    /// proxy), so rotating the leftmost hop no longer creates a fresh bucket and
    /// the shared client still gets rate limited.
    #[tokio::test]
    async fn test_spoofed_first_hop_shares_bucket_with_trusted_proxy() {
        use armature_core::Middleware;

        let limiter = Arc::new(
            RateLimiter::builder()
                .token_bucket(1, f64::EPSILON)
                .build()
                .await
                .unwrap(),
        );
        let middleware = RateLimitMiddleware::new(limiter).with_trusted_proxy_depth(1);

        let make_req = |xff: &'static str| {
            let mut req =
                armature_core::HttpRequest::new("GET".to_string(), "/api/test".to_string());
            req.headers.insert("x-forwarded-for", xff);
            req
        };

        // Attacker rotates the leftmost hop; the rightmost (10.0.0.1) is our
        // proxy's observed client address and is the same both times.
        let r1 = middleware
            .handle(
                make_req("203.0.113.1, 10.0.0.1"),
                Box::new(|_r| Box::pin(async { Ok(armature_core::HttpResponse::ok()) })),
            )
            .await
            .unwrap();
        assert_eq!(r1.status, 200);

        let r2 = middleware
            .handle(
                make_req("203.0.113.2, 10.0.0.1"),
                Box::new(|_r| Box::pin(async { Ok(armature_core::HttpResponse::ok()) })),
            )
            .await
            .unwrap();
        assert_eq!(
            r2.status, 429,
            "rotating the spoofable first hop must not mint a fresh bucket"
        );
    }

    /// With the safe default (no trusted proxy), `X-Forwarded-For` is not
    /// trusted at all: no IP is derived from it, so a spoofed header cannot be
    /// used to target or partition rate-limit buckets.
    #[tokio::test]
    async fn test_xff_ignored_when_no_trusted_proxy() {
        let middleware = RateLimitMiddleware::new(create_test_limiter().await);
        assert_eq!(middleware.trusted_proxy_depth, 0);

        // Sanity: the selector itself refuses to trust anything at depth 0.
        assert_eq!(forwarded_ip_at_depth("1.2.3.4, 5.6.7.8", 0), None);
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

    /// Observability: every fail-open bypass caused by a backend error must be
    /// counted via `skip_on_error_count`, so an operator can detect that rate
    /// limiting was silently disabled during a backend outage.
    #[tokio::test]
    async fn test_skip_on_error_count_increments_on_backend_error() {
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

        assert_eq!(middleware.skip_on_error_count(), 0);

        let response = middleware.check(&info).await;
        assert!(response.is_allowed());
        assert_eq!(
            middleware.skip_on_error_count(),
            1,
            "a fail-open bypass must be counted"
        );

        // A second backend error increments the counter again.
        middleware.check(&info).await;
        assert_eq!(middleware.skip_on_error_count(), 2);
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
