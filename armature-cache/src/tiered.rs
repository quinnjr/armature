//! Multi-tier caching (L1/L2 cache layers)

use crate::error::CacheResult;
use crate::traits::CacheStore;
use async_trait::async_trait;
use std::collections::HashMap;
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
pub struct InMemoryCache {
    data: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// Maximum number of retained entries. `0` means unbounded.
    max_entries: usize,
}

#[derive(Clone)]
struct CacheEntry {
    value: String,
    expires_at: Option<tokio::time::Instant>,
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
            data: Arc::new(RwLock::new(HashMap::new())),
            max_entries,
        }
    }

    /// Number of entries currently held (including any not-yet-evicted expired
    /// ones). Primarily useful for tests and capacity assertions.
    pub async fn len(&self) -> usize {
        self.data.read().await.len()
    }

    /// Whether the cache currently holds no entries.
    pub async fn is_empty(&self) -> bool {
        self.data.read().await.is_empty()
    }

    /// Eagerly remove every expired entry in one pass.
    ///
    /// Expired entries are also reclaimed lazily (on read) and opportunistically
    /// (when making room for a new write); this method exposes an explicit full
    /// sweep for callers that want to reclaim memory proactively.
    pub async fn cleanup_expired(&self) {
        let mut data = self.data.write().await;
        let now = tokio::time::Instant::now();
        Self::prune_expired(&mut data, now);
    }

    /// Drop all entries whose TTL has elapsed as of `now`. Operates on an
    /// already-held write guard so callers avoid re-locking.
    fn prune_expired(data: &mut HashMap<String, CacheEntry>, now: tokio::time::Instant) {
        data.retain(|_, entry| entry.expires_at.is_none_or(|exp| exp > now));
    }

    /// Evict a single entry to make room, preferring the one nearest to expiry
    /// (entries without a TTL are evicted last).
    fn evict_one(data: &mut HashMap<String, CacheEntry>, now: tokio::time::Instant) {
        if let Some(victim) = data
            .iter()
            .min_by_key(|(_, entry)| match entry.expires_at {
                // Entries with a TTL sort before those without; among them the
                // soonest-to-expire is evicted first.
                Some(exp) => (0u8, exp),
                None => (1u8, now),
            })
            .map(|(key, _)| key.clone())
        {
            data.remove(&victim);
        }
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
            match data.get(key) {
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
        // dead keys that are read but never overwritten.
        let mut data = self.data.write().await;
        if let Some(entry) = data.get(key)
            && entry
                .expires_at
                .is_some_and(|exp| tokio::time::Instant::now() > exp)
        {
            data.remove(key);
        }
        Ok(None)
    }

    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        let now = tokio::time::Instant::now();
        let expires_at = ttl.map(|d| now + d);
        let entry = CacheEntry { value, expires_at };

        let mut data = self.data.write().await;

        // Enforce the capacity bound only when admitting a genuinely new key.
        if self.max_entries != 0 && data.len() >= self.max_entries && !data.contains_key(key) {
            // Reclaim expired entries first; only evict a live one if still full.
            Self::prune_expired(&mut data, now);
            if data.len() >= self.max_entries {
                Self::evict_one(&mut data, now);
            }
        }

        data.insert(key.to_string(), entry);
        Ok(())
    }

    async fn delete(&self, key: &str) -> CacheResult<()> {
        self.data.write().await.remove(key);
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
        if let Some(entry) = data.get(key) {
            if let Some(expires_at) = entry.expires_at {
                let now = tokio::time::Instant::now();
                if expires_at > now {
                    Ok(Some(expires_at - now))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()> {
        let mut data = self.data.write().await;
        if let Some(entry) = data.get_mut(key) {
            entry.expires_at = Some(tokio::time::Instant::now() + ttl);
        }
        Ok(())
    }

    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let mut data = self.data.write().await;
        let entry = data.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: "0".to_string(),
            expires_at: None,
        });

        let current: i64 = entry.value.parse().unwrap_or(0);
        let new_value = current + delta;
        entry.value = new_value.to_string();

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
}
