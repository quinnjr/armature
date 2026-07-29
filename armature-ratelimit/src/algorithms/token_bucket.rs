//! Token Bucket Algorithm
//!
//! The token bucket algorithm allows smooth rate limiting with burst capacity.
//! Tokens are added at a constant rate and consumed on each request.
//!
//! ## How It Works
//!
//! 1. A bucket starts full with `capacity` tokens
//! 2. Each request consumes one token
//! 3. Tokens are added at `refill_rate` per second
//! 4. If no tokens are available, the request is denied
//!
//! ## Example
//!
//! ```rust
//! use armature_ratelimit::algorithms::TokenBucket;
//! use armature_ratelimit::algorithms::RateLimitAlgorithm;
//!
//! let bucket = TokenBucket::new(10, 1.0); // 10 capacity, 1 token/sec refill
//!
//! // First 10 requests succeed (burst)
//! for _ in 0..10 {
//!     assert!(bucket.check("user1").0);
//! }
//!
//! // 11th request fails (bucket empty)
//! assert!(!bucket.check("user1").0);
//! ```

use super::RateLimitAlgorithm;
use dashmap::DashMap;
use std::time::Instant;

/// Default hard cap on the number of distinct keys retained. Keys are derived
/// from untrusted request data, so without a bound a client rotating its key on
/// every request would grow the map without limit (a network-reachable OOM).
/// When the cap is reached, inserting a new key evicts the least-recently-active
/// one. Chosen generously so legitimate workloads never hit it.
const DEFAULT_MAX_KEYS: usize = 100_000;

/// Token bucket rate limiter state
#[derive(Debug, Clone)]
struct BucketState {
    /// Current number of tokens
    tokens: f64,
    /// Last time tokens were added
    last_refill: Instant,
}

/// Token bucket rate limiter
pub struct TokenBucket {
    /// Maximum tokens (burst capacity)
    capacity: u64,
    /// Tokens added per second
    refill_rate: f64,
    /// State per key
    buckets: DashMap<String, BucketState>,
    /// Hard cap on retained keys; evict the oldest when a new key would exceed it.
    max_keys: usize,
}

impl TokenBucket {
    /// Create a new token bucket rate limiter
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum tokens (burst capacity)
    /// * `refill_rate` - Tokens added per second
    ///
    /// # Panics
    ///
    /// Panics if capacity is 0 or refill_rate is <= 0
    pub fn new(capacity: u64, refill_rate: f64) -> Self {
        assert!(capacity > 0, "Capacity must be greater than 0");
        assert!(refill_rate > 0.0, "Refill rate must be greater than 0");

        Self {
            capacity,
            refill_rate,
            buckets: DashMap::new(),
            max_keys: DEFAULT_MAX_KEYS,
        }
    }

    /// Override the hard cap on retained keys (default [`DEFAULT_MAX_KEYS`]).
    pub fn with_max_keys(mut self, max_keys: usize) -> Self {
        self.max_keys = max_keys.max(1);
        self
    }

    /// Evict the single least-recently-refilled entry to make room for a new key.
    fn evict_oldest(&self) {
        if let Some(oldest) = self
            .buckets
            .iter()
            .min_by_key(|e| e.value().last_refill)
            .map(|e| e.key().clone())
        {
            self.buckets.remove(&oldest);
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&self, state: &mut BucketState) {
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let new_tokens = elapsed * self.refill_rate;

        state.tokens = (state.tokens + new_tokens).min(self.capacity as f64);
        state.last_refill = now;
    }

    /// Try to consume a token
    pub fn try_acquire(&self, key: &str) -> (bool, u64) {
        // Bound memory: if this is a new key that would push us over the cap,
        // evict the oldest entry first.
        if !self.buckets.contains_key(key) && self.buckets.len() >= self.max_keys {
            self.evict_oldest();
        }

        let mut entry = self
            .buckets
            .entry(key.to_string())
            .or_insert_with(|| BucketState {
                tokens: self.capacity as f64,
                last_refill: Instant::now(),
            });

        // Refill based on elapsed time
        self.refill(&mut entry);

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            (true, entry.tokens as u64)
        } else {
            (false, 0)
        }
    }

    /// Get the capacity
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Get the refill rate
    pub fn refill_rate(&self) -> f64 {
        self.refill_rate
    }
}

impl RateLimitAlgorithm for TokenBucket {
    fn check(&self, key: &str) -> (bool, u64) {
        self.try_acquire(key)
    }

    fn reset(&self, key: &str) {
        self.buckets.insert(
            key.to_string(),
            BucketState {
                tokens: self.capacity as f64,
                last_refill: Instant::now(),
            },
        );
    }

    fn remaining(&self, key: &str) -> u64 {
        self.buckets
            .get(key)
            .map(|state| state.tokens as u64)
            .unwrap_or(self.capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_initial_capacity() {
        let bucket = TokenBucket::new(10, 1.0);
        assert_eq!(bucket.remaining("test"), 10);
    }

    #[test]
    fn test_consume_tokens() {
        let bucket = TokenBucket::new(5, 1.0);

        for i in (0..5).rev() {
            let (allowed, remaining) = bucket.check("test");
            assert!(allowed);
            assert_eq!(remaining, i as u64);
        }

        // Should be denied now
        let (allowed, remaining) = bucket.check("test");
        assert!(!allowed);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_refill() {
        let bucket = TokenBucket::new(5, 10.0); // 10 tokens per second

        // Consume all tokens
        for _ in 0..5 {
            bucket.check("test");
        }

        // Wait for refill (100ms = 1 token at 10/sec)
        thread::sleep(Duration::from_millis(150));

        // Should be allowed again
        let (allowed, _) = bucket.check("test");
        assert!(allowed);
    }

    #[test]
    fn test_different_keys() {
        let bucket = TokenBucket::new(2, 0.1);

        // Exhaust key1
        bucket.check("key1");
        bucket.check("key1");
        let (allowed, _) = bucket.check("key1");
        assert!(!allowed);

        // key2 should still be full
        let (allowed, remaining) = bucket.check("key2");
        assert!(allowed);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_reset() {
        let bucket = TokenBucket::new(5, 0.1);

        // Consume all tokens
        for _ in 0..5 {
            bucket.check("test");
        }

        // Reset
        bucket.reset("test");

        // Should be full again
        assert_eq!(bucket.remaining("test"), 5);
    }

    #[test]
    fn test_max_keys_bounds_map() {
        // Regression: the standalone DashMap was unbounded. With a hard cap,
        // a flood of distinct keys must not grow it past the cap.
        let bucket = TokenBucket::new(10, 1.0).with_max_keys(100);
        for i in 0..10_000 {
            bucket.check(&format!("client-{i}"));
        }
        assert!(
            bucket.buckets.len() <= 100,
            "map must stay within max_keys, got {}",
            bucket.buckets.len()
        );
    }

    #[test]
    #[should_panic(expected = "Capacity must be greater than 0")]
    fn test_zero_capacity() {
        TokenBucket::new(0, 1.0);
    }

    #[test]
    #[should_panic(expected = "Refill rate must be greater than 0")]
    fn test_zero_refill_rate() {
        TokenBucket::new(10, 0.0);
    }
}
