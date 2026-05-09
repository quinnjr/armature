//! # Armature Rate Limiting
//!
//! A comprehensive rate limiting module for the Armature framework with multiple
//! algorithms and storage backends.
//!
//! ## Features
//!
//! - **Multiple Algorithms**: Token bucket, sliding window log, and fixed window
//! - **Storage Backends**: In-memory (DashMap) and Redis for distributed deployments
//! - **Flexible Key Extraction**: By IP, user ID, API key, or custom function
//! - **Standard Headers**: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`
//! - **Per-route Configuration**: Different limits for different endpoints
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use armature_ratelimit::{RateLimiter, RateLimitConfig, Algorithm};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a rate limiter with token bucket algorithm
//! let limiter = RateLimiter::builder()
//!     .algorithm(Algorithm::TokenBucket {
//!         capacity: 100,
//!         refill_rate: 10.0,
//!     })
//!     .build()
//!     .await?;
//!
//! // Check if a request is allowed
//! let result = limiter.check("user_123").await?;
//! if result.allowed {
//!     println!("Request allowed, {} remaining", result.remaining);
//! } else {
//!     println!("Rate limited, retry after {:?}", result.reset_at);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Algorithms
//!
//! ### Token Bucket
//!
//! Smooth rate limiting with burst capacity. Tokens are added at a fixed rate
//! and consumed on each request. Best for APIs that allow occasional bursts.
//!
//! ```rust
//! use armature_ratelimit::Algorithm;
//!
//! let algo = Algorithm::TokenBucket {
//!     capacity: 100,      // Maximum burst size
//!     refill_rate: 10.0,  // Tokens per second
//! };
//! ```
//!
//! ### Sliding Window Log
//!
//! Precise rate limiting that tracks individual request timestamps.
//! Best for strict rate limiting where accuracy is critical.
//!
//! ```rust
//! use armature_ratelimit::Algorithm;
//! use std::time::Duration;
//!
//! let algo = Algorithm::SlidingWindowLog {
//!     max_requests: 100,
//!     window: Duration::from_secs(60),
//! };
//! ```
//!
//! ### Fixed Window
//!
//! Simple rate limiting with fixed time windows.
//! Best for basic use cases where simplicity is preferred.
//!
//! ```rust
//! use armature_ratelimit::Algorithm;
//! use std::time::Duration;
//!
//! let algo = Algorithm::FixedWindow {
//!     max_requests: 100,
//!     window: Duration::from_secs(60),
//! };
//! ```

pub mod algorithms;
pub mod config;
pub mod error;
pub mod extractor;
pub mod middleware;
pub mod stores;

pub use algorithms::{Algorithm, RateLimitAlgorithm};
pub use config::{RateLimitConfig, RateLimiterBuilder};
pub use error::{RateLimitError, RateLimitResult};
pub use extractor::{KeyExtractor, KeyExtractorFn};
pub use middleware::RateLimitMiddleware;
pub use stores::{MemoryStore, RateLimitStore, StoreType};

#[cfg(feature = "redis")]
pub use stores::RedisStore;

use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace, warn};

/// Result of a rate limit check
#[derive(Debug, Clone)]
pub struct RateLimitCheckResult {
    /// Whether the request is allowed
    pub allowed: bool,
    /// Number of remaining requests in the current window
    pub remaining: u64,
    /// Maximum number of requests allowed
    pub limit: u64,
    /// When the rate limit resets (Unix timestamp in seconds)
    pub reset_at: u64,
    /// Time until reset
    pub retry_after: Option<Duration>,
}

impl RateLimitCheckResult {
    /// Create a new allowed result
    pub fn allowed(remaining: u64, limit: u64, reset_at: u64) -> Self {
        Self {
            allowed: true,
            remaining,
            limit,
            reset_at,
            retry_after: None,
        }
    }

    /// Create a new denied result
    pub fn denied(limit: u64, reset_at: u64, retry_after: Duration) -> Self {
        Self {
            allowed: false,
            remaining: 0,
            limit,
            reset_at,
            retry_after: Some(retry_after),
        }
    }
}

/// The main rate limiter
pub struct RateLimiter {
    store: Arc<dyn RateLimitStore>,
    algorithm: Algorithm,
    config: RateLimitConfig,
}

impl RateLimiter {
    /// Create a new rate limiter builder
    pub fn builder() -> RateLimiterBuilder {
        RateLimiterBuilder::new()
    }

    /// Create a new rate limiter with the given store and algorithm
    pub fn new(
        store: Arc<dyn RateLimitStore>,
        algorithm: Algorithm,
        config: RateLimitConfig,
    ) -> Self {
        debug!(
            algorithm = ?algorithm,
            "Creating new rate limiter"
        );
        Self {
            store,
            algorithm,
            config,
        }
    }

    /// Check if a request with the given key is allowed
    pub async fn check(&self, key: &str) -> RateLimitResult<RateLimitCheckResult> {
        trace!(key = %key, "Checking rate limit");

        match &self.algorithm {
            Algorithm::TokenBucket {
                capacity,
                refill_rate,
            } => self.check_token_bucket(key, *capacity, *refill_rate).await,
            Algorithm::SlidingWindowLog {
                max_requests,
                window,
            } => self.check_sliding_window(key, *max_requests, *window).await,
            Algorithm::FixedWindow {
                max_requests,
                window,
            } => self.check_fixed_window(key, *max_requests, *window).await,
        }
    }

    /// Check using token bucket algorithm
    async fn check_token_bucket(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
    ) -> RateLimitResult<RateLimitCheckResult> {
        let result = self
            .store
            .token_bucket_check(key, capacity, refill_rate)
            .await?;

        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Guard against `refill_rate <= 0`: a zero rate means tokens
        // never refill, so reset_at is effectively never (use u64::MAX
        // as the sentinel) and retry_after collapses to a very long
        // duration. Without these guards, `1.0 / 0.0 = inf` cascades
        // into `inf as u64` (saturated) and `Duration::from_secs_f64`
        // panics on its own NaN/inf inputs.
        let reset_at = if refill_rate > 0.0 && refill_rate.is_finite() {
            let secs_to_full = (capacity as f64 / refill_rate).clamp(0.0, u64::MAX as f64) as u64;
            now_secs.saturating_add(secs_to_full)
        } else {
            u64::MAX
        };

        if result.0 {
            debug!(key = %key, remaining = result.1, "Token bucket: request allowed");
            Ok(RateLimitCheckResult::allowed(result.1, capacity, reset_at))
        } else {
            let retry_after = if refill_rate > 0.0 && refill_rate.is_finite() {
                let secs = (1.0 / refill_rate).clamp(0.0, u64::MAX as f64);
                Duration::from_secs_f64(secs)
            } else {
                // Sentinel — caller treats this as "retry indefinitely
                // postponed" the same way it treats an open-ended reset_at.
                Duration::from_secs(u64::MAX)
            };
            warn!(key = %key, retry_after = ?retry_after, "Token bucket: request denied");
            Ok(RateLimitCheckResult::denied(
                capacity,
                reset_at,
                retry_after,
            ))
        }
    }

    /// Check using sliding window log algorithm
    async fn check_sliding_window(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<RateLimitCheckResult> {
        let result = self
            .store
            .sliding_window_check(key, max_requests, window)
            .await?;

        let reset_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + window.as_secs();

        if result.0 {
            debug!(key = %key, remaining = result.1, "Sliding window: request allowed");
            Ok(RateLimitCheckResult::allowed(
                result.1,
                max_requests,
                reset_at,
            ))
        } else {
            let retry_after = Duration::from_secs(1);
            warn!(key = %key, retry_after = ?retry_after, "Sliding window: request denied");
            Ok(RateLimitCheckResult::denied(
                max_requests,
                reset_at,
                retry_after,
            ))
        }
    }

    /// Check using fixed window algorithm
    async fn check_fixed_window(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<RateLimitCheckResult> {
        let result = self
            .store
            .fixed_window_check(key, max_requests, window)
            .await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let window_secs = window.as_secs();
        let reset_at = ((now / window_secs) + 1) * window_secs;

        if result.0 {
            debug!(key = %key, remaining = result.1, "Fixed window: request allowed");
            Ok(RateLimitCheckResult::allowed(
                result.1,
                max_requests,
                reset_at,
            ))
        } else {
            let retry_after = Duration::from_secs(reset_at - now);
            warn!(key = %key, retry_after = ?retry_after, "Fixed window: request denied");
            Ok(RateLimitCheckResult::denied(
                max_requests,
                reset_at,
                retry_after,
            ))
        }
    }

    /// Get the algorithm used by this rate limiter
    pub fn algorithm(&self) -> &Algorithm {
        &self.algorithm
    }

    /// Get the configuration
    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Reset the rate limit for a key
    pub async fn reset(&self, key: &str) -> RateLimitResult<()> {
        debug!(key = %key, "Resetting rate limit");
        self.store.reset(key).await
    }
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("algorithm", &self.algorithm)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_bucket_basic() {
        let limiter = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 5,
                refill_rate: 1.0,
            })
            .build()
            .await
            .unwrap();

        // First 5 requests should be allowed
        for i in 0..5 {
            let result = limiter.check("test_key").await.unwrap();
            assert!(result.allowed, "Request {} should be allowed", i);
        }

        // 6th request should be denied
        let result = limiter.check("test_key").await.unwrap();
        assert!(!result.allowed, "6th request should be denied");
    }

    #[tokio::test]
    async fn test_fixed_window_basic() {
        let limiter = RateLimiter::builder()
            .algorithm(Algorithm::FixedWindow {
                max_requests: 3,
                window: Duration::from_secs(60),
            })
            .build()
            .await
            .unwrap();

        // First 3 requests should be allowed
        for i in 0..3 {
            let result = limiter.check("test_key").await.unwrap();
            assert!(result.allowed, "Request {} should be allowed", i);
        }

        // 4th request should be denied
        let result = limiter.check("test_key").await.unwrap();
        assert!(!result.allowed, "4th request should be denied");
    }

    #[tokio::test]
    async fn test_different_keys() {
        let limiter = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 2,
                refill_rate: 1.0,
            })
            .build()
            .await
            .unwrap();

        // Exhaust key1
        limiter.check("key1").await.unwrap();
        limiter.check("key1").await.unwrap();
        let result = limiter.check("key1").await.unwrap();
        assert!(!result.allowed);

        // key2 should still work
        let result = limiter.check("key2").await.unwrap();
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn test_reset() {
        let limiter = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 1,
                refill_rate: 0.001,
            })
            .build()
            .await
            .unwrap();

        // Exhaust the limit
        limiter.check("test_key").await.unwrap();
        let result = limiter.check("test_key").await.unwrap();
        assert!(!result.allowed);

        // Reset and try again
        limiter.reset("test_key").await.unwrap();
        let result = limiter.check("test_key").await.unwrap();
        assert!(result.allowed);
    }

    /// Regression: token-bucket with refill_rate == 0.0 used to panic
    /// in `check_token_bucket` (divide-by-zero in the reset_at and
    /// retry_after calculations). The builder now rejects it before
    /// any runtime divide can happen.
    #[tokio::test]
    async fn test_token_bucket_zero_refill_rate_rejected_at_build() {
        let result = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 2,
                refill_rate: 0.0,
            })
            .build()
            .await;
        assert!(result.is_err(), "build should reject refill_rate == 0.0");
    }

    /// Regression: NaN refill_rate poisons the in-memory bucket
    /// (`tokens + NaN = NaN`, `NaN.min(capacity) = capacity`) so the
    /// limiter would never deny. Now rejected at build time.
    #[tokio::test]
    async fn test_token_bucket_nan_refill_rate_rejected_at_build() {
        let result = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 1,
                refill_rate: f64::NAN,
            })
            .build()
            .await;
        assert!(result.is_err(), "build should reject NaN refill_rate");
    }

    /// Boundary: a tiny but finite positive rate should still build
    /// cleanly and behave sensibly (refills are effectively zero on
    /// human timescales but the divide-by-zero path is not hit).
    #[tokio::test]
    async fn test_token_bucket_tiny_positive_rate_is_accepted() {
        let limiter = RateLimiter::builder()
            .algorithm(Algorithm::TokenBucket {
                capacity: 1,
                refill_rate: f64::EPSILON,
            })
            .build()
            .await
            .expect("tiny positive rate should build");
        assert!(limiter.check("k").await.unwrap().allowed);
        assert!(!limiter.check("k").await.unwrap().allowed);
    }
}
