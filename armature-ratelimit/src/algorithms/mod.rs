//! Rate limiting algorithms
//!
//! This module provides different rate limiting algorithms:
//!
//! - **Token Bucket**: Smooth rate limiting with burst capacity
//! - **Sliding Window Log**: Precise rate limiting with individual request tracking
//! - **Fixed Window**: Simple rate limiting with fixed time windows

mod fixed_window;
mod sliding_window;
mod token_bucket;

pub use fixed_window::FixedWindow;
pub use sliding_window::SlidingWindowLog;
pub use token_bucket::TokenBucket;

use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Fraction of the key cap reclaimed by a single eviction pass: `max_keys / 64`.
///
/// Evicting exactly one entry per new key means every request past the cap pays
/// a full scan of the map — the key-rotating client the cap exists to stop turns
/// a memory-exhaustion attempt into a CPU-exhaustion one. Reclaiming a batch
/// amortises that scan over the next `max_keys / 64` insertions, so the
/// per-request cost is bounded by a small constant while the map stays capped.
pub(crate) const EVICTION_BATCH_DIVISOR: usize = 64;

/// Number of entries a single eviction pass reclaims for a given key cap.
pub(crate) fn eviction_batch_size(max_keys: usize) -> usize {
    (max_keys / EVICTION_BATCH_DIVISOR).max(1)
}

/// Evict the `batch` least-recently-active entries from `map`.
///
/// `last_active` reports an entry's most recent activity; entries for which it
/// yields `None` (an empty request log, say) carry no usable age and are evicted
/// first, since they hold state nothing is relying on.
///
/// The candidate list is materialised before any removal so no shard lock is
/// held across a mutation, and the batch boundary is found with a linear
/// selection rather than a full sort.
pub(crate) fn evict_oldest_batch<V>(
    map: &DashMap<String, V>,
    batch: usize,
    last_active: impl Fn(&V) -> Option<Instant>,
) {
    if batch == 0 {
        return;
    }

    let mut entries: Vec<(String, Option<Instant>)> = map
        .iter()
        .map(|e| (e.key().clone(), last_active(e.value())))
        .collect();

    if entries.is_empty() {
        return;
    }

    // `None` sorts before `Some`, so ageless entries lead.
    let batch = batch.min(entries.len());
    if batch < entries.len() {
        entries.select_nth_unstable_by_key(batch - 1, |(_, t)| *t);
    }

    for (key, _) in entries.into_iter().take(batch) {
        map.remove(&key);
    }
}

/// Rate limiting algorithm configuration
#[derive(Debug, Clone)]
pub enum Algorithm {
    /// Token bucket algorithm
    ///
    /// Tokens are added at a fixed rate and consumed on each request.
    /// Allows bursts up to the bucket capacity.
    TokenBucket {
        /// Maximum number of tokens (burst capacity)
        capacity: u64,
        /// Tokens added per second
        refill_rate: f64,
    },

    /// Sliding window log algorithm
    ///
    /// Tracks individual request timestamps within a sliding window.
    /// Most accurate but requires more storage.
    SlidingWindowLog {
        /// Maximum requests allowed in the window
        max_requests: u64,
        /// Window duration
        window: Duration,
    },

    /// Fixed window algorithm
    ///
    /// Divides time into fixed windows and counts requests per window.
    /// Simple but can allow bursts at window boundaries.
    FixedWindow {
        /// Maximum requests allowed per window
        max_requests: u64,
        /// Window duration
        window: Duration,
    },
}

impl Algorithm {
    /// Create a token bucket algorithm with default values (100 capacity, 10/sec refill)
    pub fn token_bucket_default() -> Self {
        Self::TokenBucket {
            capacity: 100,
            refill_rate: 10.0,
        }
    }

    /// Create a sliding window algorithm with default values (100 requests per minute)
    pub fn sliding_window_default() -> Self {
        Self::SlidingWindowLog {
            max_requests: 100,
            window: Duration::from_secs(60),
        }
    }

    /// Create a fixed window algorithm with default values (100 requests per minute)
    pub fn fixed_window_default() -> Self {
        Self::FixedWindow {
            max_requests: 100,
            window: Duration::from_secs(60),
        }
    }

    /// Get the effective limit for this algorithm
    pub fn limit(&self) -> u64 {
        match self {
            Algorithm::TokenBucket { capacity, .. } => *capacity,
            Algorithm::SlidingWindowLog { max_requests, .. } => *max_requests,
            Algorithm::FixedWindow { max_requests, .. } => *max_requests,
        }
    }

    /// Get a human-readable description of the algorithm
    pub fn description(&self) -> String {
        match self {
            Algorithm::TokenBucket {
                capacity,
                refill_rate,
            } => format!(
                "Token bucket: {} capacity, {:.2} tokens/sec refill",
                capacity, refill_rate
            ),
            Algorithm::SlidingWindowLog {
                max_requests,
                window,
            } => format!("Sliding window: {} requests per {:?}", max_requests, window),
            Algorithm::FixedWindow {
                max_requests,
                window,
            } => format!("Fixed window: {} requests per {:?}", max_requests, window),
        }
    }
}

/// Trait for rate limiting algorithm implementations
pub trait RateLimitAlgorithm: Send + Sync {
    /// Check if a request is allowed
    /// Returns (allowed, remaining_count)
    fn check(&self, key: &str) -> (bool, u64);

    /// Reset the state for a key
    fn reset(&self, key: &str);

    /// Get the current remaining count for a key
    fn remaining(&self, key: &str) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_limit() {
        assert_eq!(
            Algorithm::TokenBucket {
                capacity: 50,
                refill_rate: 5.0
            }
            .limit(),
            50
        );
        assert_eq!(
            Algorithm::SlidingWindowLog {
                max_requests: 100,
                window: Duration::from_secs(60)
            }
            .limit(),
            100
        );
        assert_eq!(
            Algorithm::FixedWindow {
                max_requests: 200,
                window: Duration::from_secs(30)
            }
            .limit(),
            200
        );
    }

    #[test]
    fn test_algorithm_description() {
        let algo = Algorithm::TokenBucket {
            capacity: 100,
            refill_rate: 10.0,
        };
        assert!(algo.description().contains("Token bucket"));
        assert!(algo.description().contains("100"));
    }

    #[test]
    fn test_default_algorithms() {
        let tb = Algorithm::token_bucket_default();
        assert!(matches!(
            tb,
            Algorithm::TokenBucket {
                capacity: 100,
                refill_rate: _
            }
        ));

        let sw = Algorithm::sliding_window_default();
        assert!(matches!(
            sw,
            Algorithm::SlidingWindowLog {
                max_requests: 100,
                ..
            }
        ));

        let fw = Algorithm::fixed_window_default();
        assert!(matches!(
            fw,
            Algorithm::FixedWindow {
                max_requests: 100,
                ..
            }
        ));
    }
}
