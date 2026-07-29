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

/// How often the background prune task calls [`RateLimitStore::cleanup`].
///
/// The in-memory store retains an entry per distinct rate-limit key and only
/// reclaims idle entries when `cleanup` runs, so this task must run for the
/// store to stay bounded. Sixty seconds keeps memory pressure low without
/// meaningful overhead (cleanup is O(keys) over cheap timestamp comparisons).
const DEFAULT_PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// Owns a background task that periodically calls [`RateLimitStore::cleanup`]
/// on the limiter's store, evicting idle/expired entries so the store cannot
/// grow without bound.
///
/// The task is tied to the lifetime of this handle: dropping the handle (which
/// happens when the owning [`RateLimiter`] is dropped) signals the task to stop
/// and aborts it, so it never outlives the limiter or leaks.
struct PruneTask {
    shutdown: Arc<tokio::sync::Notify>,
    handle: tokio::task::JoinHandle<()>,
}

impl PruneTask {
    /// Spawn a prune task that calls `store.cleanup()` every `interval`.
    ///
    /// Must be called from within a Tokio runtime (see the guarded call site
    /// in [`RateLimiter::new`]).
    fn spawn(store: Arc<dyn RateLimitStore>, interval: Duration) -> Self {
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let task_shutdown = shutdown.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Don't fire a burst of catch-up ticks if the task is starved.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick completes immediately; consume it so cleanup runs
            // one full interval after startup rather than right away.
            ticker.tick().await;

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Err(e) = store.cleanup().await {
                            warn!(error = %e, "Rate limit store cleanup failed");
                        }
                    }
                    _ = task_shutdown.notified() => {
                        debug!("Rate limit prune task shutting down");
                        break;
                    }
                }
            }
        });

        Self { shutdown, handle }
    }
}

impl Drop for PruneTask {
    fn drop(&mut self) {
        // Ask the task to stop gracefully, then abort in case it is currently
        // parked in `store.cleanup()` and can't observe the notification.
        self.shutdown.notify_waiters();
        self.handle.abort();
    }
}

/// The main rate limiter
pub struct RateLimiter {
    store: Arc<dyn RateLimitStore>,
    algorithm: Algorithm,
    config: RateLimitConfig,
    /// Background task that periodically prunes idle store entries. Kept alive
    /// for as long as the limiter lives; dropped (and stopped) with it. `None`
    /// only when the limiter is constructed outside a Tokio runtime.
    _prune_task: Option<PruneTask>,
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

        // Schedule periodic pruning of the store so idle per-key entries are
        // reclaimed and the store cannot grow without bound. Spawning requires
        // a Tokio runtime; `new` is normally reached via the async `build`, but
        // guard against direct calls made outside a runtime so we never panic.
        let prune_task = match tokio::runtime::Handle::try_current() {
            Ok(_) => Some(PruneTask::spawn(store.clone(), DEFAULT_PRUNE_INTERVAL)),
            Err(_) => {
                debug!(
                    "No Tokio runtime available; rate limit store prune task not \
                     scheduled (call RateLimiter within a runtime to enable it)"
                );
                None
            }
        };

        Self {
            store,
            algorithm,
            config,
            _prune_task: prune_task,
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
            // Retry after the configured window (the worst-case time until the
            // oldest logged request falls out of the sliding window), not a
            // hardcoded 1s.
            let retry_after = window;
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

        // Compute the window boundary in milliseconds so sub-second windows
        // (e.g. 500ms) never trigger an integer divide-by-zero: `window.as_secs()`
        // truncates to 0 for any window under a second, and `now / 0` panics.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let window_ms = window.as_millis().max(1);
        let reset_at_ms = ((now_ms / window_ms) + 1) * window_ms;
        // X-RateLimit-Reset is a Unix timestamp in seconds.
        let reset_at = (reset_at_ms / 1000) as u64;

        if result.0 {
            debug!(key = %key, remaining = result.1, "Fixed window: request allowed");
            Ok(RateLimitCheckResult::allowed(
                result.1,
                max_requests,
                reset_at,
            ))
        } else {
            // Time until the current window rolls over. Millisecond-precise so
            // sub-second windows report a real (non-zero, non-panicking) delay.
            let retry_after = Duration::from_millis((reset_at_ms - now_ms) as u64);
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
    async fn test_limiter_schedules_prune_task() {
        // Built inside a Tokio runtime, so the background prune task must be
        // wired up and owned by the limiter.
        let limiter = RateLimiter::builder()
            .token_bucket(5, 1.0)
            .build()
            .await
            .unwrap();
        assert!(
            limiter._prune_task.is_some(),
            "prune task should be scheduled when constructed within a runtime"
        );
    }

    #[tokio::test]
    async fn test_prune_task_runs_cleanup() {
        use crate::stores::{MemoryStore, RateLimitStore};

        // Idle TTL of ~1ms means the entry is immediately eligible for eviction.
        let store = Arc::new(MemoryStore::new().with_idle_ttl(Duration::from_millis(1)));
        store.token_bucket_check("k", 5, 1.0).await.unwrap();
        assert!(store.key_count() > 0);

        let dyn_store: Arc<dyn RateLimitStore> = store.clone();
        let task = PruneTask::spawn(dyn_store, Duration::from_millis(20));

        // Wait for at least one prune tick to fire and reclaim the idle entry.
        tokio::time::sleep(Duration::from_millis(90)).await;
        assert_eq!(
            store.key_count(),
            0,
            "scheduled prune task should have evicted the idle entry"
        );

        // Dropping the handle must stop the background task.
        drop(task);
    }

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

    /// Regression: a sub-second fixed window (e.g. 500ms) used to panic with an
    /// integer divide-by-zero in `check_fixed_window` because `window.as_secs()`
    /// truncated to 0 and `now / 0` panicked. It must now return a decision.
    #[tokio::test]
    async fn test_fixed_window_sub_second_does_not_panic() {
        let limiter = RateLimiter::builder()
            .fixed_window(2, Duration::from_millis(500))
            .build()
            .await
            .unwrap();

        // First two allowed, third denied — and crucially no panic on the
        // sub-second window arithmetic.
        let r1 = limiter.check("k").await.unwrap();
        assert!(r1.allowed);
        let r2 = limiter.check("k").await.unwrap();
        assert!(r2.allowed);
        let r3 = limiter.check("k").await.unwrap();
        assert!(!r3.allowed);
        // A denied sub-second window reports a real, bounded retry delay.
        let retry = r3.retry_after.expect("denied result carries retry_after");
        assert!(retry <= Duration::from_millis(500));
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
