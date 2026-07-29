//! Rate limiter configuration and builder

use crate::RateLimiter;
use crate::algorithms::Algorithm;
use crate::error::{RateLimitError, RateLimitResult};
use crate::stores::{MemoryStore, RateLimitStore, StoreType};
use std::sync::Arc;
use std::time::Duration;
use tracing::debug;

/// Validate an algorithm's parameters before constructing the limiter.
/// Catches inputs that would otherwise produce arithmetic NaN or
/// inf inside the runtime check path. `refill_rate <= 0` and non-finite
/// rates are explicitly rejected; if a deployment legitimately wants a
/// "never refills" policy it should set `refill_rate` to a tiny positive
/// number (e.g. `f64::EPSILON`) and the bucket will treat refills as
/// effectively zero without exposing the divide-by-zero edge.
fn validate_algorithm(algorithm: &Algorithm) -> RateLimitResult<()> {
    match algorithm {
        Algorithm::TokenBucket {
            capacity,
            refill_rate,
        } => {
            if *capacity == 0 {
                return Err(RateLimitError::config("TokenBucket capacity must be > 0"));
            }
            if !refill_rate.is_finite() || *refill_rate <= 0.0 {
                return Err(RateLimitError::config(
                    "TokenBucket refill_rate must be finite and > 0",
                ));
            }
        }
        Algorithm::SlidingWindowLog {
            max_requests,
            window,
        }
        | Algorithm::FixedWindow {
            max_requests,
            window,
        } => {
            if *max_requests == 0 {
                return Err(RateLimitError::config("max_requests must be > 0"));
            }
            if window.is_zero() {
                return Err(RateLimitError::config("window must be > 0"));
            }
        }
    }
    Ok(())
}

/// Configuration for the rate limiter
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RateLimitConfig {
    /// Algorithm to use
    pub algorithm: Algorithm,
    /// Store type (memory, redis, etc.)
    pub store_type: StoreType,
    /// Key prefix for storage
    pub key_prefix: String,
    /// Include rate limit headers in responses
    pub include_headers: bool,
    /// Whether to allow requests through when the backing store errors or
    /// times out, instead of rejecting them.
    ///
    /// Defaults to `true` (fail-open) — see the [`Default`] impl for details.
    /// Set to `false` for fail-closed behavior (deny traffic on backend
    /// outage), which is generally preferable when rate limiting is used as
    /// a security or abuse control.
    ///
    /// Every time this flag causes a store error to be bypassed, the
    /// middleware records it: `RateLimitMiddleware::skip_on_error_count()`
    /// (armature-ratelimit's `middleware.rs`) exposes a running counter and a
    /// `tracing::warn!` is emitted at the point of bypass, so operators can
    /// detect a fail-open backend outage even though it isn't rejected.
    pub skip_on_error: bool,
    /// Custom error message when rate limited
    pub error_message: Option<String>,
    /// Bypass keys (these keys will never be rate limited)
    pub bypass_keys: Vec<String>,
    /// Per-operation timeout applied to the backing store (currently the Redis
    /// store). A store op that does not complete within this bound fails with
    /// [`crate::error::RateLimitError::Timeout`] instead of stalling the request
    /// path forever, letting `skip_on_error` decide fail-open vs fail-closed.
    pub operation_timeout: Duration,
    /// Number of trusted reverse proxies in front of the application.
    ///
    /// Controls how the client IP is derived from `X-Forwarded-For`: the
    /// rightmost hops are appended by *your* infrastructure and are
    /// trustworthy, so the client is selected `trusted_proxy_depth`-from-the-
    /// right. A value of `0` (the default) means **no** proxy is trusted and
    /// `X-Forwarded-For`/`X-Real-IP` are ignored entirely, because those
    /// headers are attacker-controlled when the request is not known to have
    /// passed through a trusted proxy. Set this to the exact number of proxies
    /// between the client and the app to enable IP rate limiting on proxied
    /// traffic.
    pub trusted_proxy_depth: usize,
}

impl Default for RateLimitConfig {
    /// # Fail-open by default
    ///
    /// `skip_on_error` defaults to `true`: if the backing store (e.g. Redis)
    /// errors or times out, requests are **allowed through** rather than
    /// rejected. This favors availability over strict enforcement, which is
    /// appropriate for many deployments but may be surprising if rate
    /// limiting is being relied on as a security or abuse control, where a
    /// backend outage should instead deny traffic. Callers that need
    /// fail-closed behavior under backend outage should explicitly set
    /// `skip_on_error: false` (or use [`RateLimiterBuilder::skip_on_error`]
    /// with `false` when building via [`RateLimitConfig::builder`]).
    fn default() -> Self {
        Self {
            algorithm: Algorithm::TokenBucket {
                capacity: 100,
                refill_rate: 10.0,
            },
            store_type: StoreType::Memory,
            key_prefix: "ratelimit".to_string(),
            include_headers: true,
            skip_on_error: true,
            error_message: None,
            bypass_keys: Vec::new(),
            operation_timeout: Duration::from_secs(3),
            trusted_proxy_depth: 0,
        }
    }
}

impl RateLimitConfig {
    /// Create a new configuration builder
    pub fn builder() -> RateLimiterBuilder {
        RateLimiterBuilder::new()
    }

    /// Check if a key should bypass rate limiting
    pub fn should_bypass(&self, key: &str) -> bool {
        self.bypass_keys.iter().any(|k| k == key)
    }
}

/// Builder for creating a RateLimiter
pub struct RateLimiterBuilder {
    algorithm: Option<Algorithm>,
    store_type: StoreType,
    key_prefix: String,
    include_headers: bool,
    skip_on_error: bool,
    error_message: Option<String>,
    bypass_keys: Vec<String>,
    operation_timeout: Duration,
    trusted_proxy_depth: usize,
    #[cfg(feature = "redis")]
    redis_url: Option<String>,
}

impl RateLimiterBuilder {
    /// Create a new builder with default values.
    ///
    /// # Fail-open by default
    ///
    /// Like [`RateLimitConfig::default`], the builder defaults
    /// `skip_on_error` to `true`: on a backing-store error or timeout,
    /// requests are allowed through rather than rejected. If rate limiting
    /// is being used as a security or abuse control, consider calling
    /// [`Self::skip_on_error`]`(false)` to fail closed (deny traffic) when
    /// the backend is unavailable instead.
    pub fn new() -> Self {
        Self {
            algorithm: None,
            store_type: StoreType::Memory,
            key_prefix: "ratelimit".to_string(),
            include_headers: true,
            skip_on_error: true,
            error_message: None,
            bypass_keys: Vec::new(),
            operation_timeout: Duration::from_secs(3),
            trusted_proxy_depth: 0,
            #[cfg(feature = "redis")]
            redis_url: None,
        }
    }

    /// Set the rate limiting algorithm
    pub fn algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = Some(algorithm);
        self
    }

    /// Use token bucket algorithm
    pub fn token_bucket(mut self, capacity: u64, refill_rate: f64) -> Self {
        self.algorithm = Some(Algorithm::TokenBucket {
            capacity,
            refill_rate,
        });
        self
    }

    /// Use sliding window log algorithm
    pub fn sliding_window(mut self, max_requests: u64, window: Duration) -> Self {
        self.algorithm = Some(Algorithm::SlidingWindowLog {
            max_requests,
            window,
        });
        self
    }

    /// Use fixed window algorithm
    pub fn fixed_window(mut self, max_requests: u64, window: Duration) -> Self {
        self.algorithm = Some(Algorithm::FixedWindow {
            max_requests,
            window,
        });
        self
    }

    /// Use in-memory store (default)
    pub fn memory_store(mut self) -> Self {
        self.store_type = StoreType::Memory;
        self
    }

    /// Use Redis store for distributed rate limiting
    #[cfg(feature = "redis")]
    pub fn redis_store(mut self, url: &str) -> Self {
        self.store_type = StoreType::Redis;
        self.redis_url = Some(url.to_string());
        self
    }

    /// Set the key prefix for storage
    pub fn key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Include rate limit headers in responses
    pub fn include_headers(mut self, include: bool) -> Self {
        self.include_headers = include;
        self
    }

    /// Set whether to skip (allow through) rate limiting on store errors.
    ///
    /// Defaults to `true` (fail-open); pass `false` for fail-closed behavior
    /// (deny traffic) when the backing store is unavailable. See
    /// [`RateLimiterBuilder::new`] for more on the default posture.
    pub fn skip_on_error(mut self, skip: bool) -> Self {
        self.skip_on_error = skip;
        self
    }

    /// Set the per-operation timeout applied to the backing store.
    ///
    /// A store op that exceeds this bound fails with
    /// [`crate::error::RateLimitError::Timeout`] rather than blocking the
    /// request path indefinitely.
    pub fn operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }

    /// Set the number of trusted reverse proxies in front of the app.
    ///
    /// See [`RateLimitConfig::trusted_proxy_depth`]. Defaults to `0`
    /// (`X-Forwarded-For`/`X-Real-IP` are not trusted).
    pub fn trusted_proxy_depth(mut self, depth: usize) -> Self {
        self.trusted_proxy_depth = depth;
        self
    }

    /// Set custom error message when rate limited
    pub fn error_message(mut self, message: impl Into<String>) -> Self {
        self.error_message = Some(message.into());
        self
    }

    /// Add a key that should bypass rate limiting
    pub fn bypass_key(mut self, key: impl Into<String>) -> Self {
        self.bypass_keys.push(key.into());
        self
    }

    /// Add multiple keys that should bypass rate limiting
    pub fn bypass_keys(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.bypass_keys.extend(keys.into_iter().map(|k| k.into()));
        self
    }

    /// Build the rate limiter
    pub async fn build(self) -> RateLimitResult<RateLimiter> {
        let algorithm = self
            .algorithm
            .ok_or_else(|| RateLimitError::config("Algorithm must be specified"))?;
        validate_algorithm(&algorithm)?;

        debug!(
            algorithm = ?algorithm,
            store_type = ?self.store_type,
            "Building rate limiter"
        );

        let config = RateLimitConfig {
            algorithm: algorithm.clone(),
            store_type: self.store_type.clone(),
            key_prefix: self.key_prefix.clone(),
            include_headers: self.include_headers,
            skip_on_error: self.skip_on_error,
            error_message: self.error_message,
            bypass_keys: self.bypass_keys,
            operation_timeout: self.operation_timeout,
            trusted_proxy_depth: self.trusted_proxy_depth,
        };

        let store: Arc<dyn RateLimitStore> = match self.store_type {
            StoreType::Memory => Arc::new(MemoryStore::new()),
            #[cfg(feature = "redis")]
            StoreType::Redis => {
                let url = self.redis_url.ok_or_else(|| {
                    RateLimitError::config("Redis URL must be specified for Redis store")
                })?;
                // Honor the configured key_prefix instead of the hardcoded
                // "ratelimit" default so multiple limiters can share a Redis
                // instance without colliding.
                Arc::new(
                    crate::stores::RedisStore::with_prefix(&url, self.key_prefix.clone())
                        .await?
                        .with_operation_timeout(self.operation_timeout),
                )
            }
            #[cfg(not(feature = "redis"))]
            StoreType::Redis => {
                return Err(RateLimitError::config(
                    "Redis feature is not enabled. Add `redis` feature to use Redis store.",
                ));
            }
        };

        Ok(RateLimiter::new(store, algorithm, config))
    }
}

impl Default for RateLimiterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RateLimitConfig::default();
        assert!(matches!(config.algorithm, Algorithm::TokenBucket { .. }));
        assert!(matches!(config.store_type, StoreType::Memory));
        assert!(config.include_headers);
        assert!(config.skip_on_error);
    }

    #[test]
    fn test_bypass_keys() {
        let config = RateLimitConfig {
            bypass_keys: vec!["admin".to_string(), "service".to_string()],
            ..Default::default()
        };

        assert!(config.should_bypass("admin"));
        assert!(config.should_bypass("service"));
        assert!(!config.should_bypass("user"));
    }

    #[tokio::test]
    async fn test_builder_token_bucket() {
        let limiter = RateLimiterBuilder::new()
            .token_bucket(100, 10.0)
            .key_prefix("test")
            .build()
            .await
            .unwrap();

        assert!(matches!(
            limiter.algorithm(),
            Algorithm::TokenBucket {
                capacity: 100,
                refill_rate: _
            }
        ));
    }

    #[tokio::test]
    async fn test_builder_fixed_window() {
        let limiter = RateLimiterBuilder::new()
            .fixed_window(50, Duration::from_secs(60))
            .build()
            .await
            .unwrap();

        assert!(matches!(
            limiter.algorithm(),
            Algorithm::FixedWindow {
                max_requests: 50,
                window: _
            }
        ));
    }

    #[tokio::test]
    async fn test_builder_missing_algorithm() {
        let result = RateLimiterBuilder::new().build().await;
        assert!(result.is_err());
    }
}
