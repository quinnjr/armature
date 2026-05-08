//! Multi-tier caching (L1/L2 cache layers)

use crate::error::CacheResult;
use crate::traits::CacheStore;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
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
}

impl Default for TieredCacheConfig {
    fn default() -> Self {
        Self {
            enable_l1: true,
            enable_l2: true,
            write_through: true,
            promote_to_l1: true,
            l1_ttl_fraction: 0.25, // L1 lives 1/4 as long as L2
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
        Self { l1, l2, config }
    }

    /// Get value from cache (checks L1 then L2)
    pub async fn get(&self, key: &str) -> CacheResult<Option<String>> {
        // Try L1 first
        if self.config.enable_l1
            && let Some(value) = self.l1.get_json(key).await?
        {
            return Ok(Some(value));
        }

        // Try L2
        if self.config.enable_l2
            && let Some(value) = self.l2.get_json(key).await?
        {
            // Promote to L1 if configured
            if self.config.enable_l1 && self.config.promote_to_l1 {
                // Use shorter TTL for L1
                let l2_ttl = self.l2.ttl(key).await?;
                let l1_ttl = l2_ttl.map(|ttl| {
                    Duration::from_secs_f64(ttl.as_secs_f64() * self.config.l1_ttl_fraction)
                });
                let _ = self.l1.set_json(key, value.clone(), l1_ttl).await;
            }
            return Ok(Some(value));
        }

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

    /// Get cache statistics
    pub async fn stats(&self) -> CacheStats {
        CacheStats {
            l1_enabled: self.config.enable_l1,
            l2_enabled: self.config.enable_l2,
            write_through: self.config.write_through,
            promote_to_l1: self.config.promote_to_l1,
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
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub l1_enabled: bool,
    pub l2_enabled: bool,
    pub write_through: bool,
    pub promote_to_l1: bool,
}

/// In-memory cache for L1 tier
pub struct InMemoryCache {
    data: Arc<RwLock<HashMap<String, CacheEntry>>>,
}

#[derive(Clone)]
struct CacheEntry {
    value: String,
    expires_at: Option<tokio::time::Instant>,
}

impl InMemoryCache {
    /// Create new in-memory cache
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Clean up expired entries
    #[allow(dead_code)]
    async fn cleanup_expired(&self) {
        let mut data = self.data.write().await;
        let now = tokio::time::Instant::now();
        data.retain(|_, entry| entry.expires_at.is_none_or(|exp| exp > now));
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
        let data = self.data.read().await;
        if let Some(entry) = data.get(key) {
            if let Some(expires_at) = entry.expires_at
                && tokio::time::Instant::now() > expires_at
            {
                return Ok(None); // Expired
            }
            Ok(Some(entry.value.clone()))
        } else {
            Ok(None)
        }
    }

    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        let expires_at = ttl.map(|d| tokio::time::Instant::now() + d);
        let entry = CacheEntry { value, expires_at };
        self.data.write().await.insert(key.to_string(), entry);
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
}
