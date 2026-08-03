//! Sliding Window Log Algorithm
//!
//! The sliding window log algorithm provides precise rate limiting by tracking
//! individual request timestamps within a sliding time window.
//!
//! ## How It Works
//!
//! 1. Each request timestamp is logged
//! 2. When checking, all timestamps within the window are counted
//! 3. Old timestamps (outside the window) are removed
//! 4. If count < max_requests, the request is allowed
//!
//! ## Pros
//!
//! - Most accurate rate limiting
//! - No boundary issues like fixed window
//!
//! ## Cons
//!
//! - Higher memory usage (stores all timestamps)
//! - Slightly higher CPU usage (timestamp cleanup)
//!
//! ## Example
//!
//! ```rust
//! use armature_ratelimit::algorithms::SlidingWindowLog;
//! use armature_ratelimit::algorithms::RateLimitAlgorithm;
//! use std::time::Duration;
//!
//! let limiter = SlidingWindowLog::new(5, Duration::from_secs(60)); // 5 requests per minute
//!
//! // First 5 requests succeed
//! for _ in 0..5 {
//!     assert!(limiter.check("user1").0);
//! }
//!
//! // 6th request fails
//! assert!(!limiter.check("user1").0);
//! ```

use super::RateLimitAlgorithm;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default hard cap on retained keys; see `token_bucket::DEFAULT_MAX_KEYS`.
/// Bounds memory against untrusted, high-cardinality keys.
const DEFAULT_MAX_KEYS: usize = 100_000;

/// Sliding window log rate limiter
pub struct SlidingWindowLog {
    /// Maximum requests allowed in the window
    max_requests: u64,
    /// Window duration
    window: Duration,
    /// Request timestamps per key
    logs: DashMap<String, VecDeque<Instant>>,
    /// Hard cap on retained keys; evict the oldest when a new key would exceed it.
    max_keys: usize,
}

impl SlidingWindowLog {
    /// Create a new sliding window log rate limiter
    ///
    /// # Arguments
    ///
    /// * `max_requests` - Maximum requests allowed in the window
    /// * `window` - Window duration
    ///
    /// # Panics
    ///
    /// Panics if max_requests is 0 or window is zero duration
    pub fn new(max_requests: u64, window: Duration) -> Self {
        assert!(max_requests > 0, "Max requests must be greater than 0");
        assert!(!window.is_zero(), "Window must be non-zero");

        Self {
            max_requests,
            window,
            logs: DashMap::new(),
            max_keys: DEFAULT_MAX_KEYS,
        }
    }

    /// Override the hard cap on retained keys (default [`DEFAULT_MAX_KEYS`]).
    pub fn with_max_keys(mut self, max_keys: usize) -> Self {
        self.max_keys = max_keys.max(1);
        self
    }

    /// Reclaim a batch of the entries whose most recent request is oldest, to
    /// make room for new keys. See [`super::evict_oldest_batch`] for why this
    /// is batched.
    fn evict_oldest(&self) {
        super::evict_oldest_batch(
            &self.logs,
            super::eviction_batch_size(self.max_keys),
            |log| log.back().copied(),
        );
    }

    /// Clean up old timestamps and count current requests
    fn clean_and_count(&self, key: &str) -> u64 {
        let now = Instant::now();
        let cutoff = now - self.window;

        let mut entry = self.logs.entry(key.to_string()).or_default();

        // Remove old timestamps
        while let Some(front) = entry.front() {
            if *front < cutoff {
                entry.pop_front();
            } else {
                break;
            }
        }

        entry.len() as u64
    }

    /// Try to record a request
    pub fn try_acquire(&self, key: &str) -> (bool, u64) {
        let now = Instant::now();
        let cutoff = now - self.window;

        // Bound memory: evict the oldest entry before admitting a new key that
        // would exceed the cap.
        if !self.logs.contains_key(key) && self.logs.len() >= self.max_keys {
            self.evict_oldest();
        }

        let mut entry = self.logs.entry(key.to_string()).or_default();

        // Remove old timestamps
        while let Some(front) = entry.front() {
            if *front < cutoff {
                entry.pop_front();
            } else {
                break;
            }
        }

        let current_count = entry.len() as u64;

        if current_count < self.max_requests {
            entry.push_back(now);
            let remaining = self.max_requests - current_count - 1;
            (true, remaining)
        } else {
            (false, 0)
        }
    }

    /// Get the max requests setting
    pub fn max_requests(&self) -> u64 {
        self.max_requests
    }

    /// Get the window duration
    pub fn window(&self) -> Duration {
        self.window
    }
}

impl RateLimitAlgorithm for SlidingWindowLog {
    fn check(&self, key: &str) -> (bool, u64) {
        self.try_acquire(key)
    }

    fn reset(&self, key: &str) {
        self.logs.remove(key);
    }

    fn remaining(&self, key: &str) -> u64 {
        let count = self.clean_and_count(key);
        self.max_requests.saturating_sub(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_basic_limit() {
        let limiter = SlidingWindowLog::new(5, Duration::from_secs(60));

        for i in (0..5).rev() {
            let (allowed, remaining) = limiter.check("test");
            assert!(allowed);
            assert_eq!(remaining, i as u64);
        }

        // 6th request should be denied
        let (allowed, remaining) = limiter.check("test");
        assert!(!allowed);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_window_expiry() {
        let limiter = SlidingWindowLog::new(2, Duration::from_millis(100));

        // Use up the limit
        limiter.check("test");
        limiter.check("test");
        let (allowed, _) = limiter.check("test");
        assert!(!allowed);

        // Wait for window to expire
        thread::sleep(Duration::from_millis(150));

        // Should be allowed again
        let (allowed, remaining) = limiter.check("test");
        assert!(allowed);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_different_keys() {
        let limiter = SlidingWindowLog::new(2, Duration::from_secs(60));

        // Exhaust key1
        limiter.check("key1");
        limiter.check("key1");
        let (allowed, _) = limiter.check("key1");
        assert!(!allowed);

        // key2 should still work
        let (allowed, remaining) = limiter.check("key2");
        assert!(allowed);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_reset() {
        let limiter = SlidingWindowLog::new(3, Duration::from_secs(60));

        // Use up the limit
        limiter.check("test");
        limiter.check("test");
        limiter.check("test");
        assert!(!limiter.check("test").0);

        // Reset
        limiter.reset("test");

        // Should have full capacity again
        assert_eq!(limiter.remaining("test"), 3);
        assert!(limiter.check("test").0);
    }

    #[test]
    fn test_remaining() {
        let limiter = SlidingWindowLog::new(5, Duration::from_secs(60));

        assert_eq!(limiter.remaining("test"), 5);

        limiter.check("test");
        assert_eq!(limiter.remaining("test"), 4);

        limiter.check("test");
        limiter.check("test");
        assert_eq!(limiter.remaining("test"), 2);
    }

    #[test]
    #[should_panic(expected = "Max requests must be greater than 0")]
    fn test_zero_max_requests() {
        SlidingWindowLog::new(0, Duration::from_secs(60));
    }

    #[test]
    #[should_panic(expected = "Window must be non-zero")]
    fn test_zero_window() {
        SlidingWindowLog::new(10, Duration::ZERO);
    }
}
