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
pub struct RateLimitConfig {
    /// Algorithm to use
    pub algorithm: Algorithm,
    /// Store type (memory, redis, etc.)
    pub store_type: StoreType,
    /// Key prefix for storage
    pub key_prefix: String,
    /// Include rate limit headers in responses
    pub include_headers: bool,
    /// Skip rate limiting for certain conditions
    pub skip_on_error: bool,
    /// Custom error message when rate limited
    pub error_message: Option<String>,
    /// Bypass keys (these keys will never be rate limited)
    pub bypass_keys: Vec<String>,
}

impl Default for RateLimitConfig {
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
    #[cfg(feature = "redis")]
    redis_url: Option<String>,
}

impl RateLimiterBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        Self {
            algorithm: None,
            store_type: StoreType::Memory,
            key_prefix: "ratelimit".to_string(),
            include_headers: true,
            skip_on_error: true,
            error_message: None,
            bypass_keys: Vec::new(),
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

    /// Skip rate limiting on store errors
    pub fn skip_on_error(mut self, skip: bool) -> Self {
        self.skip_on_error = skip;
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
        };

        let store: Arc<dyn RateLimitStore> = match self.store_type {
            StoreType::Memory => Arc::new(MemoryStore::new()),
            #[cfg(feature = "redis")]
            StoreType::Redis => {
                let url = self.redis_url.ok_or_else(|| {
                    RateLimitError::config("Redis URL must be specified for Redis store")
                })?;
                Arc::new(crate::stores::RedisStore::new(&url).await?)
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
