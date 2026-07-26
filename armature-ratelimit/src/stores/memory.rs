//! In-memory rate limit store
//!
//! Uses DashMap for thread-safe concurrent access. Suitable for single-instance
//! deployments or testing. For distributed deployments, use the Redis store.

use crate::error::RateLimitResult;
use crate::stores::RateLimitStore;
use async_trait::async_trait;
use dashmap::DashMap;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tracing::{debug, trace};

/// Default idle threshold after which a per-key entry is evicted by
/// [`MemoryStore::cleanup`]. Entries whose most recent activity is older
/// than this are dropped. Token buckets self-refill to full, so evicting an
/// idle bucket is behaviourally identical to keeping it (a returning client
/// simply gets a fresh, full bucket) — but it frees the memory that would
/// otherwise be retained forever for every distinct/spoofed client key.
const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(3600);

/// Token bucket state
#[derive(Debug, Clone)]
struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

/// Fixed window state
#[derive(Debug, Clone)]
struct FixedWindowState {
    count: u64,
    window_start: Instant,
    /// Configured request limit for this key, retained so `remaining` can
    /// report `limit - count` without the caller re-supplying it.
    limit: u64,
    /// Configured window length, retained so `remaining` can tell whether the
    /// current window has already rolled over (in which case full capacity is
    /// available again).
    window: Duration,
}

/// Sliding window state: the request-timestamp log plus the configured limit
/// and window, retained so `remaining` can count only the still-live entries
/// and report `limit - live`.
#[derive(Debug, Clone, Default)]
struct SlidingWindowState {
    log: VecDeque<Instant>,
    limit: u64,
    window: Duration,
}

/// In-memory rate limit store
///
/// # Unbounded growth / memory safety
///
/// Every distinct rate-limit key (IP, API key, user id, …) creates an entry
/// in one of the maps below. Because keys are derived from untrusted request
/// data, a client that rotates or spoofs its key on every request would, if
/// entries were never evicted, grow these maps without bound and eventually
/// exhaust memory (a network-reachable OOM).
///
/// To prevent that, [`MemoryStore::cleanup`] evicts entries — **including
/// token buckets** — that have been idle longer than `idle_ttl`, and (when
/// `max_keys` is set) enforces a hard cap by evicting the oldest entries.
/// `cleanup` must be scheduled to run periodically; the limiter spawns a
/// background prune task that does exactly this (see `PruneTask` in the crate
/// root). Calling `cleanup` on a schedule is what keeps the store bounded.
pub struct MemoryStore {
    /// Token bucket states
    token_buckets: DashMap<String, TokenBucketState>,
    /// Sliding window logs
    sliding_logs: DashMap<String, SlidingWindowState>,
    /// Fixed window states
    fixed_windows: DashMap<String, FixedWindowState>,
    /// Entries idle for longer than this are evicted by `cleanup`.
    idle_ttl: Duration,
    /// Optional hard cap on the number of entries per map. When `cleanup`
    /// finds a map over this size it evicts the oldest entries until it fits.
    max_keys: Option<usize>,
}

impl MemoryStore {
    /// Create a new in-memory store with default eviction settings
    /// (idle entries pruned after one hour, no hard cap).
    pub fn new() -> Self {
        debug!("Creating new in-memory rate limit store");
        Self {
            token_buckets: DashMap::new(),
            sliding_logs: DashMap::new(),
            fixed_windows: DashMap::new(),
            idle_ttl: DEFAULT_IDLE_TTL,
            max_keys: None,
        }
    }

    /// Set the idle threshold after which `cleanup` evicts an entry.
    ///
    /// Shorter values reclaim memory more aggressively; the value should be at
    /// least a few rate-limit windows so that active clients are never evicted
    /// mid-window.
    pub fn with_idle_ttl(mut self, idle_ttl: Duration) -> Self {
        self.idle_ttl = idle_ttl;
        self
    }

    /// Set a hard cap on the number of entries retained per algorithm map.
    ///
    /// When `cleanup` runs and a map exceeds this size, the oldest (least
    /// recently active) entries are evicted until the map is back within the
    /// cap. This bounds memory even under a sustained flood of unique keys
    /// arriving faster than the idle TTL would reclaim them.
    pub fn with_max_keys(mut self, max_keys: usize) -> Self {
        self.max_keys = Some(max_keys);
        self
    }

    /// Get the number of tracked keys (for monitoring)
    pub fn key_count(&self) -> usize {
        self.token_buckets.len() + self.sliding_logs.len() + self.fixed_windows.len()
    }

    /// Evict the oldest entries from `token_buckets` until at most `max` remain.
    ///
    /// Entries are collected and sorted by `last_refill` (oldest first) *before*
    /// any removal so no DashMap shard lock is held across the mutation (which
    /// would risk a deadlock).
    fn cap_token_buckets(&self, max: usize) {
        let len = self.token_buckets.len();
        if len <= max {
            return;
        }
        let mut entries: Vec<(String, Instant)> = self
            .token_buckets
            .iter()
            .map(|e| (e.key().clone(), e.value().last_refill))
            .collect();
        entries.sort_by_key(|(_, last_refill)| *last_refill);
        for (key, _) in entries.into_iter().take(len - max) {
            self.token_buckets.remove(&key);
        }
    }

    /// Evict the oldest entries from `fixed_windows` until at most `max` remain.
    fn cap_fixed_windows(&self, max: usize) {
        let len = self.fixed_windows.len();
        if len <= max {
            return;
        }
        let mut entries: Vec<(String, Instant)> = self
            .fixed_windows
            .iter()
            .map(|e| (e.key().clone(), e.value().window_start))
            .collect();
        entries.sort_by_key(|(_, window_start)| *window_start);
        for (key, _) in entries.into_iter().take(len - max) {
            self.fixed_windows.remove(&key);
        }
    }

    /// Evict the oldest entries from `sliding_logs` until at most `max` remain.
    fn cap_sliding_logs(&self, max: usize) {
        let len = self.sliding_logs.len();
        if len <= max {
            return;
        }
        let mut entries: Vec<(String, Instant)> = self
            .sliding_logs
            .iter()
            .filter_map(|e| e.value().log.back().map(|last| (e.key().clone(), *last)))
            .collect();
        entries.sort_by_key(|(_, last)| *last);
        for (key, _) in entries.into_iter().take(len.saturating_sub(max)) {
            self.sliding_logs.remove(&key);
        }
    }
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RateLimitStore for MemoryStore {
    async fn token_bucket_check(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, capacity = capacity, refill_rate = refill_rate, "Token bucket check");

        let now = Instant::now();

        let mut entry = self
            .token_buckets
            .entry(key.to_string())
            .or_insert_with(|| TokenBucketState {
                tokens: capacity as f64,
                last_refill: now,
            });

        // Refill based on elapsed time
        let elapsed = now.duration_since(entry.last_refill).as_secs_f64();
        let new_tokens = elapsed * refill_rate;
        entry.tokens = (entry.tokens + new_tokens).min(capacity as f64);
        entry.last_refill = now;

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            let remaining = entry.tokens as u64;
            trace!(key = %key, remaining = remaining, "Token bucket: allowed");
            Ok((true, remaining))
        } else {
            trace!(key = %key, "Token bucket: denied");
            Ok((false, 0))
        }
    }

    async fn sliding_window_check(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, max_requests = max_requests, window = ?window, "Sliding window check");

        let now = Instant::now();
        let cutoff = now - window;

        let mut entry = self.sliding_logs.entry(key.to_string()).or_default();
        entry.limit = max_requests;
        entry.window = window;

        // Remove old timestamps
        while let Some(front) = entry.log.front() {
            if *front < cutoff {
                entry.log.pop_front();
            } else {
                break;
            }
        }

        let current_count = entry.log.len() as u64;

        if current_count < max_requests {
            entry.log.push_back(now);
            let remaining = max_requests - current_count - 1;
            trace!(key = %key, remaining = remaining, "Sliding window: allowed");
            Ok((true, remaining))
        } else {
            trace!(key = %key, "Sliding window: denied");
            Ok((false, 0))
        }
    }

    async fn fixed_window_check(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, max_requests = max_requests, window = ?window, "Fixed window check");

        let now = Instant::now();

        let mut entry = self
            .fixed_windows
            .entry(key.to_string())
            .or_insert_with(|| FixedWindowState {
                count: 0,
                window_start: now,
                limit: max_requests,
                window,
            });
        entry.limit = max_requests;
        entry.window = window;

        // Check if we're in a new window
        let elapsed = now.duration_since(entry.window_start);
        if elapsed >= window {
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count < max_requests {
            entry.count += 1;
            let remaining = max_requests - entry.count;
            trace!(key = %key, remaining = remaining, "Fixed window: allowed");
            Ok((true, remaining))
        } else {
            trace!(key = %key, "Fixed window: denied");
            Ok((false, 0))
        }
    }

    async fn reset(&self, key: &str) -> RateLimitResult<()> {
        debug!(key = %key, "Resetting rate limit state");
        self.token_buckets.remove(key);
        self.sliding_logs.remove(key);
        self.fixed_windows.remove(key);
        Ok(())
    }

    async fn remaining(&self, key: &str) -> RateLimitResult<u64> {
        // A given limiter only ever uses one algorithm, so a key lives in at
        // most one of these maps. Report the remaining allowance for whichever
        // one holds it.
        if let Some(entry) = self.token_buckets.get(key) {
            return Ok(entry.tokens.max(0.0) as u64);
        }

        if let Some(entry) = self.fixed_windows.get(key) {
            let elapsed = Instant::now().duration_since(entry.window_start);
            // If the window has already rolled over, the full limit is free.
            if elapsed >= entry.window {
                return Ok(entry.limit);
            }
            return Ok(entry.limit.saturating_sub(entry.count));
        }

        if let Some(entry) = self.sliding_logs.get(key) {
            // Count only timestamps still inside the window; older ones will be
            // pruned on the next check but shouldn't count against `remaining`.
            let now = Instant::now();
            let live = entry
                .log
                .iter()
                .filter(|t| now.duration_since(**t) < entry.window)
                .count() as u64;
            return Ok(entry.limit.saturating_sub(live));
        }

        Ok(0)
    }

    async fn cleanup(&self) -> RateLimitResult<()> {
        debug!("Cleaning up idle rate limit entries");

        let now = Instant::now();
        let is_idle = |last: Instant| now.saturating_duration_since(last) >= self.idle_ttl;

        // Token buckets: evict idle entries. This is the critical fix for
        // unbounded growth — buckets self-refill to full, so an evicted idle
        // bucket is recreated (full) on the client's next request, making
        // eviction behaviourally safe while bounding memory.
        self.token_buckets
            .retain(|_, state| !is_idle(state.last_refill));

        // Sliding window: drop logs whose most recent request is idle (or
        // which have emptied out entirely).
        self.sliding_logs.retain(|_, state| match state.log.back() {
            Some(last) => !is_idle(*last),
            None => false,
        });

        // Fixed window: drop windows that started longer ago than the TTL.
        self.fixed_windows
            .retain(|_, state| !is_idle(state.window_start));

        // Optional hard cap: even if entries are not yet idle, never let any
        // single map grow past `max_keys`. Evict the oldest first.
        if let Some(max) = self.max_keys {
            self.cap_token_buckets(max);
            self.cap_sliding_logs(max);
            self.cap_fixed_windows(max);
        }

        debug!(key_count = self.key_count(), "Cleanup complete");

        Ok(())
    }

    fn store_type(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_bucket() {
        let store = MemoryStore::new();

        // First 5 requests should be allowed
        for i in (0..5).rev() {
            let (allowed, remaining) = store.token_bucket_check("test", 5, 1.0).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 6th should be denied
        let (allowed, _) = store.token_bucket_check("test", 5, 1.0).await.unwrap();
        assert!(!allowed);
    }

    #[tokio::test]
    async fn test_sliding_window() {
        let store = MemoryStore::new();
        let window = Duration::from_secs(60);

        // First 3 requests should be allowed
        for i in (0..3).rev() {
            let (allowed, remaining) = store.sliding_window_check("test", 3, window).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 4th should be denied
        let (allowed, _) = store.sliding_window_check("test", 3, window).await.unwrap();
        assert!(!allowed);
    }

    #[tokio::test]
    async fn test_fixed_window() {
        let store = MemoryStore::new();
        let window = Duration::from_secs(60);

        // First 3 requests should be allowed
        for i in (0..3).rev() {
            let (allowed, remaining) = store.fixed_window_check("test", 3, window).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 4th should be denied
        let (allowed, _) = store.fixed_window_check("test", 3, window).await.unwrap();
        assert!(!allowed);
    }

    #[tokio::test]
    async fn test_reset() {
        let store = MemoryStore::new();

        // Use up token bucket
        store.token_bucket_check("test", 1, 0.001).await.unwrap();
        let (allowed, _) = store.token_bucket_check("test", 1, 0.001).await.unwrap();
        assert!(!allowed);

        // Reset
        store.reset("test").await.unwrap();

        // Should be allowed again
        let (allowed, _) = store.token_bucket_check("test", 1, 0.001).await.unwrap();
        assert!(allowed);
    }

    #[tokio::test]
    async fn test_different_keys() {
        let store = MemoryStore::new();

        // Exhaust key1
        store.token_bucket_check("key1", 1, 0.001).await.unwrap();
        let (allowed, _) = store.token_bucket_check("key1", 1, 0.001).await.unwrap();
        assert!(!allowed);

        // key2 should still work
        let (allowed, _) = store.token_bucket_check("key2", 1, 0.001).await.unwrap();
        assert!(allowed);
    }

    #[tokio::test]
    async fn test_cleanup() {
        let store = MemoryStore::new();

        // Add some entries
        store.token_bucket_check("test", 10, 1.0).await.unwrap();
        store
            .sliding_window_check("test", 10, Duration::from_secs(60))
            .await
            .unwrap();
        store
            .fixed_window_check("test", 10, Duration::from_secs(60))
            .await
            .unwrap();

        assert!(store.key_count() > 0);

        // Cleanup (won't remove recent entries)
        store.cleanup().await.unwrap();

        // Entries should still exist (they're recent)
        assert!(store.key_count() > 0);
    }

    #[tokio::test]
    async fn test_cleanup_evicts_idle_token_buckets() {
        // Short idle TTL so the test can cross the threshold quickly.
        let store = MemoryStore::new().with_idle_ttl(Duration::from_millis(50));

        // Populate all three algorithm maps.
        store.token_bucket_check("idle", 10, 1.0).await.unwrap();
        store
            .sliding_window_check("idle", 10, Duration::from_secs(60))
            .await
            .unwrap();
        store
            .fixed_window_check("idle", 10, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(store.key_count() > 0);

        // Fresh entries must survive a cleanup.
        store.cleanup().await.unwrap();
        assert!(
            store.key_count() > 0,
            "recent entries must not be evicted by cleanup"
        );

        // Advance past the idle threshold, then cleanup must evict everything
        // — crucially including the token bucket (the entry that was
        // previously kept "indefinitely" and caused unbounded growth).
        tokio::time::sleep(Duration::from_millis(80)).await;
        store.cleanup().await.unwrap();
        assert_eq!(
            store.key_count(),
            0,
            "idle entries, including token buckets, must be evicted"
        );
    }

    #[tokio::test]
    async fn test_store_size_stays_bounded() {
        // Simulate a flood of distinct/spoofed client keys against a store
        // with a hard cap. Without a cap the map would grow to 10_000 (and,
        // in production, without bound); cleanup must bring it back under.
        let max = 100;
        let store = MemoryStore::new().with_max_keys(max);

        for i in 0..10_000 {
            store
                .token_bucket_check(&format!("client-{i}"), 10, 1.0)
                .await
                .unwrap();
        }
        assert_eq!(store.token_buckets.len(), 10_000);

        // Nothing is idle yet, but the cap must still be enforced.
        store.cleanup().await.unwrap();
        assert!(
            store.token_buckets.len() <= max,
            "token bucket map must be capped at max_keys, got {}",
            store.token_buckets.len()
        );
    }

    #[tokio::test]
    async fn test_remaining_fixed_window_decreases() {
        // Regression: `remaining` previously only handled the token bucket and
        // returned 0 for fixed windows.
        let store = MemoryStore::new();
        let window = Duration::from_secs(60);

        // No entry yet.
        assert_eq!(store.remaining("k").await.unwrap(), 0);

        store.fixed_window_check("k", 5, window).await.unwrap();
        assert_eq!(store.remaining("k").await.unwrap(), 4);

        store.fixed_window_check("k", 5, window).await.unwrap();
        store.fixed_window_check("k", 5, window).await.unwrap();
        assert_eq!(store.remaining("k").await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_remaining_sliding_window_decreases() {
        // Regression: `remaining` previously returned 0 for sliding windows.
        let store = MemoryStore::new();
        let window = Duration::from_secs(60);

        store.sliding_window_check("k", 5, window).await.unwrap();
        assert_eq!(store.remaining("k").await.unwrap(), 4);

        store.sliding_window_check("k", 5, window).await.unwrap();
        assert_eq!(store.remaining("k").await.unwrap(), 3);
    }

    #[test]
    fn test_store_type() {
        let store = MemoryStore::new();
        assert_eq!(store.store_type(), "memory");
    }
}
