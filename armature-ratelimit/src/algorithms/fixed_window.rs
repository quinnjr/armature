//! Fixed Window Algorithm
//!
//! The fixed window algorithm divides time into fixed-size windows and counts
//! requests per window. It's simple and memory-efficient but can allow bursts
//! at window boundaries.
//!
//! ## How It Works
//!
//! 1. Time is divided into fixed windows (e.g., every minute)
//! 2. Each request increments the counter for the current window
//! 3. When a new window starts, the counter resets
//! 4. If count >= max_requests, the request is denied
//!
//! ## Boundary Issue
//!
//! A client could make max_requests at the end of window N and max_requests
//! at the start of window N+1, effectively doubling their rate. Use sliding
//! window if this is a concern.
//!
//! ## Example
//!
//! ```rust
//! use armature_ratelimit::algorithms::FixedWindow;
//! use armature_ratelimit::algorithms::RateLimitAlgorithm;
//! use std::time::Duration;
//!
//! let limiter = FixedWindow::new(5, Duration::from_secs(60)); // 5 requests per minute
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
use std::time::{Duration, Instant};

/// Default hard cap on retained keys; see `token_bucket::DEFAULT_MAX_KEYS`.
/// Bounds memory against untrusted, high-cardinality keys.
const DEFAULT_MAX_KEYS: usize = 100_000;

/// Window state for a key
#[derive(Debug, Clone)]
struct WindowState {
    /// Request count in current window
    count: u64,
    /// Window start time
    window_start: Instant,
}

/// Fixed window rate limiter
pub struct FixedWindow {
    /// Maximum requests allowed per window
    max_requests: u64,
    /// Window duration
    window: Duration,
    /// State per key
    windows: DashMap<String, WindowState>,
    /// Hard cap on retained keys; evict the oldest when a new key would exceed it.
    max_keys: usize,
}

impl FixedWindow {
    /// Create a new fixed window rate limiter
    ///
    /// # Arguments
    ///
    /// * `max_requests` - Maximum requests allowed per window
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
            windows: DashMap::new(),
            max_keys: DEFAULT_MAX_KEYS,
        }
    }

    /// Override the hard cap on retained keys (default [`DEFAULT_MAX_KEYS`]).
    pub fn with_max_keys(mut self, max_keys: usize) -> Self {
        self.max_keys = max_keys.max(1);
        self
    }

    /// Evict the single least-recently-started window to make room for a new key.
    fn evict_oldest(&self) {
        if let Some(oldest) = self
            .windows
            .iter()
            .min_by_key(|e| e.value().window_start)
            .map(|e| e.key().clone())
        {
            self.windows.remove(&oldest);
        }
    }

    /// Try to record a request
    pub fn try_acquire(&self, key: &str) -> (bool, u64) {
        let now = Instant::now();

        // Bound memory: evict the oldest entry before admitting a new key that
        // would exceed the cap.
        if !self.windows.contains_key(key) && self.windows.len() >= self.max_keys {
            self.evict_oldest();
        }

        let mut entry = self
            .windows
            .entry(key.to_string())
            .or_insert_with(|| WindowState {
                count: 0,
                window_start: now,
            });

        // Check if we're in a new window
        let elapsed = now.duration_since(entry.window_start);
        if elapsed >= self.window {
            // Reset for new window
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count < self.max_requests {
            entry.count += 1;
            let remaining = self.max_requests - entry.count;
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

    /// Get time until the current window resets for a key
    pub fn time_until_reset(&self, key: &str) -> Duration {
        let now = Instant::now();

        if let Some(entry) = self.windows.get(key) {
            let elapsed = now.duration_since(entry.window_start);
            if elapsed < self.window {
                return self.window - elapsed;
            }
        }

        self.window
    }
}

impl RateLimitAlgorithm for FixedWindow {
    fn check(&self, key: &str) -> (bool, u64) {
        self.try_acquire(key)
    }

    fn reset(&self, key: &str) {
        self.windows.remove(key);
    }

    fn remaining(&self, key: &str) -> u64 {
        let now = Instant::now();

        if let Some(entry) = self.windows.get(key) {
            let elapsed = now.duration_since(entry.window_start);
            if elapsed >= self.window {
                // Window expired, full capacity available
                return self.max_requests;
            }
            return self.max_requests.saturating_sub(entry.count);
        }

        self.max_requests
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_basic_limit() {
        let limiter = FixedWindow::new(5, Duration::from_secs(60));

        for i in (0..5).rev() {
            let (allowed, remaining) = limiter.check("test");
            assert!(allowed, "Request should be allowed");
            assert_eq!(remaining, i as u64);
        }

        // 6th request should be denied
        let (allowed, remaining) = limiter.check("test");
        assert!(!allowed, "6th request should be denied");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_window_reset() {
        let limiter = FixedWindow::new(2, Duration::from_millis(100));

        // Use up the limit
        limiter.check("test");
        limiter.check("test");
        let (allowed, _) = limiter.check("test");
        assert!(!allowed);

        // Wait for window to reset
        thread::sleep(Duration::from_millis(150));

        // Should be allowed again (new window)
        let (allowed, remaining) = limiter.check("test");
        assert!(allowed);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_different_keys() {
        let limiter = FixedWindow::new(2, Duration::from_secs(60));

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
        let limiter = FixedWindow::new(3, Duration::from_secs(60));

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
        let limiter = FixedWindow::new(5, Duration::from_secs(60));

        assert_eq!(limiter.remaining("test"), 5);

        limiter.check("test");
        assert_eq!(limiter.remaining("test"), 4);

        limiter.check("test");
        limiter.check("test");
        assert_eq!(limiter.remaining("test"), 2);
    }

    #[test]
    fn test_time_until_reset() {
        let limiter = FixedWindow::new(5, Duration::from_millis(500));

        // No requests yet - should return full window
        let time = limiter.time_until_reset("test");
        assert!(time <= Duration::from_millis(500));

        // Make a request to start the window
        limiter.check("test");

        // Time until reset should be less than full window
        thread::sleep(Duration::from_millis(100));
        let time = limiter.time_until_reset("test");
        assert!(time < Duration::from_millis(450));
    }

    #[test]
    #[should_panic(expected = "Max requests must be greater than 0")]
    fn test_zero_max_requests() {
        FixedWindow::new(0, Duration::from_secs(60));
    }

    #[test]
    #[should_panic(expected = "Window must be non-zero")]
    fn test_zero_window() {
        FixedWindow::new(10, Duration::ZERO);
    }
}
