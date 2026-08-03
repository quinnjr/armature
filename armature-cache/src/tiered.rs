//! Multi-tier caching (L1/L2 cache layers)

use crate::error::CacheResult;
use crate::traits::CacheStore;
use async_trait::async_trait;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;

/// Multi-tier cache with L1 (in-memory) and L2 (distributed) layers
pub struct TieredCache<L1, L2>
where
    L1: CacheStore,
    L2: CacheStore,
{
    /// L1 cache (fast, local)
    l1: Arc<L1>,

    /// L2 cache (slower, distributed)
    l2: Arc<L2>,

    /// Configuration
    config: TieredCacheConfig,

    /// Live hit/miss/promotion counters, shared across clones.
    metrics: Arc<TieredMetrics>,
}

/// Atomic counters backing [`TieredCache::stats`].
#[derive(Debug, Default)]
struct TieredMetrics {
    l1_hits: AtomicU64,
    l2_hits: AtomicU64,
    misses: AtomicU64,
    promotions: AtomicU64,
}

/// Tiered cache configuration
#[derive(Debug, Clone)]
pub struct TieredCacheConfig {
    /// Enable L1 cache
    pub enable_l1: bool,

    /// Enable L2 cache
    pub enable_l2: bool,

    /// Write-through to L2 on L1 set
    pub write_through: bool,

    /// Promote L2 hits to L1
    pub promote_to_l1: bool,

    /// L1 TTL multiplier (fraction of L2 TTL)
    pub l1_ttl_fraction: f64,

    /// Fixed TTL applied to entries promoted from L2 into L1 on a read hit.
    ///
    /// # Policy
    ///
    /// Deriving the promoted L1 TTL from the *live* remaining L2 TTL would
    /// require an extra `TTL` round-trip to L2 on **every** promotion (the hot
    /// read path). To avoid that per-read cost we instead apply this fixed
    /// default. `None` means promoted entries are stored without expiry and
    /// rely on L1 capacity/eviction. Defaults to 60s.
    ///
    /// Note: the write path (`set`) still derives the L1 TTL as
    /// `l1_ttl_fraction * ttl` because the TTL is already known there without
    /// any extra round-trip.
    pub l1_promote_ttl: Option<Duration>,
}

impl Default for TieredCacheConfig {
    fn default() -> Self {
        Self {
            enable_l1: true,
            enable_l2: true,
            write_through: true,
            promote_to_l1: true,
            l1_ttl_fraction: 0.25, // L1 lives 1/4 as long as L2
            l1_promote_ttl: Some(Duration::from_secs(60)),
        }
    }
}

impl<L1, L2> TieredCache<L1, L2>
where
    L1: CacheStore,
    L2: CacheStore,
{
    /// Create new tiered cache
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use armature_cache::*;
    ///
    /// let l1 = Arc::new(InMemoryCache::new());
    /// let l2 = Arc::new(RedisCache::new(config).await?);
    /// let cache = TieredCache::new(l1, l2);
    /// ```
    pub fn new(l1: Arc<L1>, l2: Arc<L2>) -> Self {
        Self::with_config(l1, l2, TieredCacheConfig::default())
    }

    /// Create with custom configuration
    pub fn with_config(l1: Arc<L1>, l2: Arc<L2>, config: TieredCacheConfig) -> Self {
        Self {
            l1,
            l2,
            config,
            metrics: Arc::new(TieredMetrics::default()),
        }
    }

    /// Get value from cache (checks L1 then L2)
    pub async fn get(&self, key: &str) -> CacheResult<Option<String>> {
        // Try L1 first
        if self.config.enable_l1
            && let Some(value) = self.l1.get_json(key).await?
        {
            self.metrics.l1_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(value));
        }

        // Try L2
        if self.config.enable_l2
            && let Some(value) = self.l2.get_json(key).await?
        {
            self.metrics.l2_hits.fetch_add(1, Ordering::Relaxed);
            // Promote to L1 if configured.
            //
            // Use the fixed `l1_promote_ttl` rather than issuing an extra
            // `l2.ttl(key)` round-trip to derive it from the live L2 TTL. See
            // `TieredCacheConfig::l1_promote_ttl` for the policy rationale.
            if self.config.enable_l1 && self.config.promote_to_l1 {
                let l1_ttl = self.config.l1_promote_ttl;
                if self.l1.set_json(key, value.clone(), l1_ttl).await.is_ok() {
                    self.metrics.promotions.fetch_add(1, Ordering::Relaxed);
                }
            }
            return Ok(Some(value));
        }

        self.metrics.misses.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    /// Set value in cache (writes to both L1 and L2)
    pub async fn set(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        // Write to L2 first (source of truth)
        if self.config.enable_l2 {
            self.l2.set_json(key, value.clone(), ttl).await?;
        }

        // Write to L1 if write-through is enabled
        if self.config.enable_l1 && (self.config.write_through || !self.config.enable_l2) {
            let l1_ttl = ttl.map(|ttl| {
                Duration::from_secs_f64(ttl.as_secs_f64() * self.config.l1_ttl_fraction)
            });
            self.l1.set_json(key, value, l1_ttl).await?;
        }

        Ok(())
    }

    /// Delete from both L1 and L2
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        if self.config.enable_l1 {
            self.l1.delete(key).await?;
        }
        if self.config.enable_l2 {
            self.l2.delete(key).await?;
        }
        Ok(())
    }

    /// Check if key exists (checks L1 then L2)
    pub async fn exists(&self, key: &str) -> CacheResult<bool> {
        if self.config.enable_l1 && self.l1.exists(key).await? {
            return Ok(true);
        }
        if self.config.enable_l2 {
            return self.l2.exists(key).await;
        }
        Ok(false)
    }

    /// Clear both L1 and L2
    ///
    /// `L2::clear()` is whatever the L2 backend's `CacheStore::clear()` does.
    /// For `RedisCache` with a `key_prefix` configured, that is now scoped:
    /// it `SCAN`s for and `UNLINK`s only the keys under that prefix, not the
    /// whole database. **Remaining risk:** an L2 `RedisCache` with *no*
    /// `key_prefix` configured has no distinct slice of the keyspace to
    /// scope to and still falls back to unscoped `FLUSHDB`, wiping the
    /// *entire* Redis database/instance — so a `TieredCache` wrapping an
    /// unprefixed `RedisCache` that shares a Redis instance with other
    /// services/tenants should not call `clear()`. A `MemcachedCache` L2 has
    /// no prefix-scoped clear at all (the memcached protocol has no key
    /// enumeration primitive), so its `clear()` always wipes the whole
    /// memcached instance regardless of `key_prefix`.
    pub async fn clear(&self) -> CacheResult<()> {
        if self.config.enable_l1 {
            self.l1.clear().await?;
        }
        if self.config.enable_l2 {
            self.l2.clear().await?;
        }
        Ok(())
    }

    /// Get cache statistics.
    ///
    /// The `*_enabled`/`write_through`/`promote_to_l1` fields echo the static
    /// configuration; the `l1_hits`/`l2_hits`/`misses`/`promotions` counters are
    /// live totals accumulated across every `get` since construction (shared
    /// across clones).
    pub async fn stats(&self) -> CacheStats {
        CacheStats {
            l1_enabled: self.config.enable_l1,
            l2_enabled: self.config.enable_l2,
            write_through: self.config.write_through,
            promote_to_l1: self.config.promote_to_l1,
            l1_hits: self.metrics.l1_hits.load(Ordering::Relaxed),
            l2_hits: self.metrics.l2_hits.load(Ordering::Relaxed),
            misses: self.metrics.misses.load(Ordering::Relaxed),
            promotions: self.metrics.promotions.load(Ordering::Relaxed),
        }
    }
}

impl<L1, L2> Clone for TieredCache<L1, L2>
where
    L1: CacheStore,
    L2: CacheStore,
{
    fn clone(&self) -> Self {
        Self {
            l1: self.l1.clone(),
            l2: self.l2.clone(),
            config: self.config.clone(),
            metrics: self.metrics.clone(),
        }
    }
}

/// Cache statistics.
///
/// The boolean fields reflect configuration; the `u64` counters are live
/// running totals of `get` outcomes.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub l1_enabled: bool,
    pub l2_enabled: bool,
    pub write_through: bool,
    pub promote_to_l1: bool,
    /// Number of `get` calls served from L1.
    pub l1_hits: u64,
    /// Number of `get` calls served from L2 (after an L1 miss).
    pub l2_hits: u64,
    /// Number of `get` calls that found nothing in either tier.
    pub misses: u64,
    /// Number of L2 hits successfully copied back into L1.
    pub promotions: u64,
}

/// Default upper bound on the number of live entries an [`InMemoryCache`]
/// retains. Prevents the backing map from growing without limit when callers
/// never delete keys. Override with [`InMemoryCache::with_capacity`].
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// In-memory cache for L1 tier.
///
/// The cache is bounded: it holds at most `max_entries` live entries. Expired
/// entries are evicted lazily on read and eagerly when making room for a new
/// key; when the map is full of live entries the one nearest to expiry is
/// evicted to admit a new write.
///
/// # Eviction cost
///
/// Eviction order is maintained incrementally rather than recomputed: entries
/// carrying a TTL are tracked in a min-heap keyed by expiry, and entries
/// without one in an insertion-ordered queue. Making room is therefore
/// `O(log n)` (amortised) instead of the full `O(n)` map scan a naive
/// `min_by_key` would need on **every** admission once the map is full —
/// which, because every L2 -> L1 promotion in [`TieredCache::get`] is such an
/// admission, otherwise put a full scan of the (10,000-entry by default) map
/// on the hot read path and made filling the cache `O(n^2)`.
pub struct InMemoryCache {
    data: Arc<RwLock<CacheState>>,
    /// Maximum number of retained entries. `0` means unbounded.
    max_entries: usize,
}

/// The map plus the auxiliary structures that keep eviction order.
///
/// The two order-tracking structures use **lazy deletion**: removing or
/// overwriting a key does not touch them, so they can hold entries that no
/// longer describe the map. Every pop is therefore validated against `entries`
/// before it is acted on, and [`CacheState::compact`] rebuilds both once the
/// accumulated slack outgrows the live set.
#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CacheEntry>,
    /// Keys with a TTL, ordered soonest-expiry-first.
    by_expiry: BinaryHeap<Reverse<(tokio::time::Instant, String)>>,
    /// Keys without a TTL, in insertion order. These are evicted only once no
    /// TTL-carrying entry remains, matching the documented policy that
    /// unexpiring entries are evicted last.
    without_expiry: VecDeque<String>,
}

#[derive(Clone)]
struct CacheEntry {
    value: String,
    expires_at: Option<tokio::time::Instant>,
}

impl CacheState {
    /// Insert or overwrite `key`, recording it in the matching order structure.
    fn insert(&mut self, key: String, entry: CacheEntry) {
        match entry.expires_at {
            Some(expires_at) => self.by_expiry.push(Reverse((expires_at, key.clone()))),
            None => self.without_expiry.push_back(key.clone()),
        }
        self.entries.insert(key, entry);
        self.compact_if_slack();
    }

    /// Whether `key`'s live entry is the one that `expires_at` describes.
    /// Guards against acting on a stale order-structure record.
    fn is_current(&self, key: &str, expires_at: Option<tokio::time::Instant>) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| entry.expires_at == expires_at)
    }

    /// Drop every entry whose TTL has elapsed as of `now`.
    ///
    /// Only the expired prefix of the heap is examined, so this costs
    /// `O(k log n)` in the number of entries actually reclaimed rather than
    /// `O(n)` in the size of the map.
    fn prune_expired(&mut self, now: tokio::time::Instant) {
        while matches!(self.by_expiry.peek(), Some(Reverse((exp, _))) if *exp <= now) {
            let Some(Reverse((expires_at, key))) = self.by_expiry.pop() else {
                break;
            };
            if self.is_current(&key, Some(expires_at)) {
                self.entries.remove(&key);
            }
        }
    }

    /// Evict a single entry to make room, preferring the one nearest to expiry;
    /// entries without a TTL are evicted last, oldest first.
    fn evict_one(&mut self) {
        while let Some(Reverse((expires_at, key))) = self.by_expiry.pop() {
            if self.is_current(&key, Some(expires_at)) {
                self.entries.remove(&key);
                return;
            }
        }

        while let Some(key) = self.without_expiry.pop_front() {
            if self.is_current(&key, None) {
                self.entries.remove(&key);
                return;
            }
        }
    }

    /// Rebuild both order structures from `entries` once lazy deletion has left
    /// more slack than live data, so repeated overwrites/deletes cannot grow
    /// them without bound.
    fn compact_if_slack(&mut self) {
        let tracked = self.by_expiry.len() + self.without_expiry.len();
        if tracked > 2 * self.entries.len().max(16) {
            self.compact();
        }
    }

    /// Discard both order structures and rebuild them from the live entries.
    ///
    /// Insertion order among unexpiring entries is not recoverable from the
    /// map, so their relative eviction order is reset. That order is not part
    /// of the documented policy (which only fixes that they are evicted after
    /// every TTL-carrying entry), and compaction is rare.
    fn compact(&mut self) {
        let mut by_expiry = BinaryHeap::with_capacity(self.entries.len());
        let mut without_expiry = VecDeque::with_capacity(self.entries.len());

        for (key, entry) in &self.entries {
            match entry.expires_at {
                Some(expires_at) => by_expiry.push(Reverse((expires_at, key.clone()))),
                None => without_expiry.push_back(key.clone()),
            }
        }

        self.by_expiry = by_expiry;
        self.without_expiry = without_expiry;
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.by_expiry.clear();
        self.without_expiry.clear();
    }
}

impl InMemoryCache {
    /// Create a new in-memory cache bounded to [`DEFAULT_MAX_ENTRIES`] entries.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }

    /// Create a new in-memory cache bounded to `max_entries` live entries.
    ///
    /// Pass `0` for an explicitly unbounded cache (growth is then the caller's
    /// responsibility).
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            data: Arc::new(RwLock::new(CacheState::default())),
            max_entries,
        }
    }

    /// Number of entries currently held (including any not-yet-evicted expired
    /// ones). Primarily useful for tests and capacity assertions.
    pub async fn len(&self) -> usize {
        self.data.read().await.entries.len()
    }

    /// Whether the cache currently holds no entries.
    pub async fn is_empty(&self) -> bool {
        self.data.read().await.entries.is_empty()
    }

    /// Eagerly remove every expired entry in one pass.
    ///
    /// Expired entries are also reclaimed lazily (on read) and opportunistically
    /// (when making room for a new write); this method exposes an explicit full
    /// sweep for callers that want to reclaim memory proactively.
    pub async fn cleanup_expired(&self) {
        let mut data = self.data.write().await;
        let now = tokio::time::Instant::now();
        data.prune_expired(now);
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CacheStore for InMemoryCache {
    async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
        // Fast path under a read lock.
        {
            let data = self.data.read().await;
            match data.entries.get(key) {
                None => return Ok(None),
                Some(entry) => match entry.expires_at {
                    Some(expires_at) if tokio::time::Instant::now() > expires_at => {
                        // Expired — fall through to evict it under a write lock.
                    }
                    _ => return Ok(Some(entry.value.clone())),
                },
            }
        }

        // Lazy eviction: drop the expired entry so the map does not accumulate
        // dead keys that are read but never overwritten. The heap record is
        // left behind and skipped when it surfaces (lazy deletion).
        let mut data = self.data.write().await;
        if let Some(entry) = data.entries.get(key)
            && entry
                .expires_at
                .is_some_and(|exp| tokio::time::Instant::now() > exp)
        {
            data.entries.remove(key);
        }
        Ok(None)
    }

    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        let now = tokio::time::Instant::now();
        let expires_at = ttl.map(|d| now + d);
        let entry = CacheEntry { value, expires_at };

        let mut data = self.data.write().await;

        // Enforce the capacity bound only when admitting a genuinely new key.
        if self.max_entries != 0
            && data.entries.len() >= self.max_entries
            && !data.entries.contains_key(key)
        {
            // Reclaim expired entries first; only evict a live one if still full.
            data.prune_expired(now);
            if data.entries.len() >= self.max_entries {
                data.evict_one();
            }
        }

        data.insert(key.to_string(), entry);
        Ok(())
    }

    async fn delete(&self, key: &str) -> CacheResult<()> {
        self.data.write().await.entries.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> CacheResult<bool> {
        self.get_json(key).await.map(|v| v.is_some())
    }

    async fn clear(&self) -> CacheResult<()> {
        self.data.write().await.clear();
        Ok(())
    }

    async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
        let data = self.data.read().await;
        let now = tokio::time::Instant::now();
        Ok(data
            .entries
            .get(key)
            .and_then(|e| e.expires_at)
            .filter(|&x| x > now)
            .map(|x| x - now))
    }

    async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()> {
        let mut data = self.data.write().await;
        let expires_at = tokio::time::Instant::now() + ttl;

        let updated = match data.entries.get_mut(key) {
            Some(entry) => {
                entry.expires_at = Some(expires_at);
                true
            }
            None => false,
        };

        if updated {
            // The key now sorts by expiry, so record it where eviction can find
            // it. Any earlier record for it is stale and gets skipped on pop.
            data.by_expiry.push(Reverse((expires_at, key.to_string())));
            data.compact_if_slack();
        }
        Ok(())
    }

    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let mut data = self.data.write().await;

        let new_value = match data.entries.get_mut(key) {
            Some(entry) => {
                let current: i64 = entry.value.parse().unwrap_or(0);
                let new_value = current + delta;
                entry.value = new_value.to_string();
                new_value
            }
            None => {
                // A counter created here has no TTL, so it goes through
                // `insert` to be registered for eviction like any other write.
                data.insert(
                    key.to_string(),
                    CacheEntry {
                        value: delta.to_string(),
                        expires_at: None,
                    },
                );
                delta
            }
        };

        Ok(new_value)
    }

    async fn decrement(&self, key: &str, delta: i64) -> CacheResult<i64> {
        self.increment(key, -delta).await
    }
}

#[cfg(test)]
mod tests_tiered {
    use super::*;

    #[tokio::test]
    async fn test_tiered_cache() {
        let l1 = Arc::new(InMemoryCache::new());
        let l2 = Arc::new(InMemoryCache::new());
        let cache = TieredCache::new(l1.clone(), l2.clone());

        // Set value
        cache.set("test", "value".to_string(), None).await.unwrap();

        // Get from L1
        let value = l1.get_json("test").await.unwrap();
        assert!(value.is_some());

        // Get from tiered cache
        let value = cache.get("test").await.unwrap();
        assert_eq!(value, Some("value".to_string()));

        // Delete
        cache.delete("test").await.unwrap();
        let value = cache.get("test").await.unwrap();
        assert_eq!(value, None);
    }

    #[tokio::test]
    async fn test_l2_promotion() {
        let l1 = Arc::new(InMemoryCache::new());
        let l2 = Arc::new(InMemoryCache::new());
        let cache = TieredCache::new(l1.clone(), l2.clone());

        // Set in L2 only
        l2.set_json("key", "value".to_string(), None).await.unwrap();

        // Get from tiered cache (should promote to L1)
        let value = cache.get("key").await.unwrap();
        assert_eq!(value, Some("value".to_string()));

        // Check L1 was populated
        let l1_value = l1.get_json("key").await.unwrap();
        assert!(l1_value.is_some());
    }

    #[tokio::test]
    async fn test_promotion_uses_fixed_l1_ttl_no_l2_roundtrip() {
        let l1 = Arc::new(InMemoryCache::new());
        let l2 = Arc::new(InMemoryCache::new());
        let config = TieredCacheConfig {
            l1_promote_ttl: Some(Duration::from_secs(30)),
            ..TieredCacheConfig::default()
        };
        let cache = TieredCache::with_config(l1.clone(), l2.clone(), config);

        // Set in L2 only, with NO TTL. The old implementation would have read
        // L2's (absent) TTL and stored L1 without expiry; the new one applies
        // the fixed `l1_promote_ttl` regardless of L2's TTL.
        l2.set_json("key", "value".to_string(), None).await.unwrap();

        // Trigger promotion.
        let value = cache.get("key").await.unwrap();
        assert_eq!(value, Some("value".to_string()));

        // L1 entry should carry the fixed promote TTL (<= 30s and > 0), proving
        // it was derived from config, not from L2's (missing) TTL.
        let l1_ttl = l1.ttl("key").await.unwrap();
        let l1_ttl = l1_ttl.expect("promoted L1 entry should have a TTL");
        assert!(l1_ttl > Duration::from_secs(0));
        assert!(l1_ttl <= Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_promotion_with_no_l1_ttl_stores_without_expiry() {
        let l1 = Arc::new(InMemoryCache::new());
        let l2 = Arc::new(InMemoryCache::new());
        let config = TieredCacheConfig {
            l1_promote_ttl: None,
            ..TieredCacheConfig::default()
        };
        let cache = TieredCache::with_config(l1.clone(), l2.clone(), config);

        l2.set_json("key", "value".to_string(), None).await.unwrap();
        let _ = cache.get("key").await.unwrap();

        // No promote TTL configured -> promoted entry has no expiry.
        assert_eq!(l1.ttl("key").await.unwrap(), None);
        assert!(l1.get_json("key").await.unwrap().is_some());
    }

    /// Regression: `stats()` must report live hit/miss/promotion totals, not
    /// merely echo the configuration booleans.
    #[tokio::test]
    async fn test_stats_track_hits_misses_promotions() {
        let l1 = Arc::new(InMemoryCache::new());
        let l2 = Arc::new(InMemoryCache::new());
        let cache = TieredCache::new(l1.clone(), l2.clone());

        // Miss: absent from both tiers.
        assert_eq!(cache.get("absent").await.unwrap(), None);

        // Write-through populates L1; the read is an L1 hit.
        cache.set("k", "v".to_string(), None).await.unwrap();
        assert_eq!(cache.get("k").await.unwrap(), Some("v".to_string()));

        // L2-only key: an L2 hit that also promotes into L1.
        l2.set_json("only2", "v2".to_string(), None).await.unwrap();
        assert_eq!(cache.get("only2").await.unwrap(), Some("v2".to_string()));

        let stats = cache.stats().await;
        assert_eq!(stats.misses, 1, "one miss expected");
        assert_eq!(stats.l1_hits, 1, "one L1 hit expected");
        assert_eq!(stats.l2_hits, 1, "one L2 hit expected");
        assert_eq!(stats.promotions, 1, "one promotion expected");
    }

    /// Regression: expired L1 entries must actually be removed from the backing
    /// map on read, not just reported as `None` while lingering forever.
    #[tokio::test(start_paused = true)]
    async fn test_l1_expired_entries_are_evicted_on_read() {
        let cache = InMemoryCache::new();
        cache
            .set_json("k", "v".to_string(), Some(Duration::from_secs(1)))
            .await
            .unwrap();
        assert_eq!(cache.len().await, 1);

        tokio::time::advance(Duration::from_secs(2)).await;

        assert_eq!(cache.get_json("k").await.unwrap(), None);
        assert_eq!(
            cache.len().await,
            0,
            "expired entry must be evicted from the map, not retained"
        );
    }

    /// Regression: the map must stay bounded — admitting a new key when full
    /// evicts an existing entry instead of growing without limit.
    #[tokio::test]
    async fn test_l1_capacity_bound_is_enforced() {
        let cache = InMemoryCache::with_capacity(2);
        cache.set_json("a", "1".to_string(), None).await.unwrap();
        cache.set_json("b", "2".to_string(), None).await.unwrap();
        cache.set_json("c", "3".to_string(), None).await.unwrap();

        assert!(
            cache.len().await <= 2,
            "cache must not exceed its configured capacity of 2, got {}",
            cache.len().await
        );
        // The most recently written key must survive.
        assert_eq!(cache.get_json("c").await.unwrap(), Some("3".to_string()));
    }

    /// A full cache prefers to reclaim expired entries before evicting a live
    /// one, so unexpired keys survive when there is expired garbage to drop.
    #[tokio::test(start_paused = true)]
    async fn test_capacity_prefers_reclaiming_expired() {
        let cache = InMemoryCache::with_capacity(2);
        cache
            .set_json("short", "x".to_string(), Some(Duration::from_secs(1)))
            .await
            .unwrap();
        cache.set_json("keep", "y".to_string(), None).await.unwrap();

        tokio::time::advance(Duration::from_secs(2)).await;

        // Admitting "new" should reclaim the expired "short" rather than "keep".
        cache.set_json("new", "z".to_string(), None).await.unwrap();
        assert!(cache.len().await <= 2);
        assert_eq!(cache.get_json("keep").await.unwrap(), Some("y".to_string()));
        assert_eq!(cache.get_json("new").await.unwrap(), Some("z".to_string()));
    }

    /// The heap-ordered eviction must pick the same victim the old linear
    /// `min_by_key` scan did: among live entries, the one nearest to expiry.
    #[tokio::test(start_paused = true)]
    async fn test_evicts_the_entry_nearest_to_expiry() {
        let cache = InMemoryCache::with_capacity(3);
        cache
            .set_json("far", "1".to_string(), Some(Duration::from_secs(300)))
            .await
            .unwrap();
        cache
            .set_json("soon", "2".to_string(), Some(Duration::from_secs(10)))
            .await
            .unwrap();
        cache
            .set_json("mid", "3".to_string(), Some(Duration::from_secs(60)))
            .await
            .unwrap();

        // Full and nothing has expired yet, so a new key evicts a live one.
        cache.set_json("new", "4".to_string(), None).await.unwrap();

        assert_eq!(cache.len().await, 3);
        assert_eq!(
            cache.get_json("soon").await.unwrap(),
            None,
            "the soonest-to-expire entry must be the victim"
        );
        assert_eq!(cache.get_json("far").await.unwrap(), Some("1".to_string()));
        assert_eq!(cache.get_json("mid").await.unwrap(), Some("3".to_string()));
        assert_eq!(cache.get_json("new").await.unwrap(), Some("4".to_string()));
    }

    /// Entries without a TTL are evicted only once no TTL-carrying entry is
    /// left, matching the documented policy.
    #[tokio::test(start_paused = true)]
    async fn test_entries_without_ttl_are_evicted_last() {
        let cache = InMemoryCache::with_capacity(2);
        cache
            .set_json("nottl", "1".to_string(), None)
            .await
            .unwrap();
        cache
            .set_json("ttl", "2".to_string(), Some(Duration::from_secs(300)))
            .await
            .unwrap();

        cache.set_json("new", "3".to_string(), None).await.unwrap();

        assert_eq!(
            cache.get_json("ttl").await.unwrap(),
            None,
            "a TTL-carrying entry must be evicted before an unexpiring one"
        );
        assert_eq!(
            cache.get_json("nottl").await.unwrap(),
            Some("1".to_string())
        );

        // With no TTL-carrying entry left, the oldest unexpiring one goes.
        cache
            .set_json("newest", "4".to_string(), None)
            .await
            .unwrap();
        assert_eq!(cache.get_json("nottl").await.unwrap(), None);
        assert_eq!(cache.get_json("new").await.unwrap(), Some("3".to_string()));
        assert_eq!(
            cache.get_json("newest").await.unwrap(),
            Some("4".to_string())
        );
    }

    /// `expire()` re-registers the key in the expiry ordering, so a key given a
    /// TTL after the fact is still evicted ahead of unexpiring entries.
    #[tokio::test(start_paused = true)]
    async fn test_expire_updates_eviction_order() {
        let cache = InMemoryCache::with_capacity(2);
        cache.set_json("a", "1".to_string(), None).await.unwrap();
        cache.set_json("b", "2".to_string(), None).await.unwrap();

        // "b" was written second, so FIFO would evict "a" first; giving "b" a
        // TTL must move it ahead of every unexpiring entry instead.
        cache.expire("b", Duration::from_secs(300)).await.unwrap();
        cache.set_json("c", "3".to_string(), None).await.unwrap();

        assert_eq!(cache.get_json("b").await.unwrap(), None);
        assert_eq!(cache.get_json("a").await.unwrap(), Some("1".to_string()));
        assert_eq!(cache.get_json("c").await.unwrap(), Some("3".to_string()));
    }

    /// Lazy deletion must not leak: repeatedly overwriting the same keys leaves
    /// stale ordering records behind, and compaction has to reclaim them rather
    /// than let them grow without bound.
    #[tokio::test(start_paused = true)]
    async fn test_repeated_overwrites_do_not_grow_ordering_structures() {
        let cache = InMemoryCache::with_capacity(8);

        for round in 0..500 {
            for key in ["a", "b", "c", "d"] {
                cache
                    .set_json(key, round.to_string(), Some(Duration::from_secs(300)))
                    .await
                    .unwrap();
            }
        }

        let state = cache.data.read().await;
        assert_eq!(state.entries.len(), 4);
        assert!(
            state.by_expiry.len() + state.without_expiry.len() <= 2 * 16,
            "stale ordering records must be compacted away, got {} tracked for {} entries",
            state.by_expiry.len() + state.without_expiry.len(),
            state.entries.len()
        );
    }

    /// Filling a bounded cache with far more distinct keys than it can hold
    /// must stay correct: the bound holds and the most recent writes survive.
    #[tokio::test(start_paused = true)]
    async fn test_admission_churn_keeps_cache_bounded() {
        let cache = InMemoryCache::with_capacity(64);

        for i in 0..2_000 {
            cache
                .set_json(
                    &format!("k{i}"),
                    i.to_string(),
                    // Alternate so both ordering structures are exercised.
                    if i % 2 == 0 {
                        Some(Duration::from_secs(300 + i as u64))
                    } else {
                        None
                    },
                )
                .await
                .unwrap();
        }

        assert_eq!(cache.len().await, 64);
        assert_eq!(
            cache.get_json("k1999").await.unwrap(),
            Some("1999".to_string()),
            "the most recent write must survive"
        );
    }
}
