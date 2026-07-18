//! Cache store trait definition.

use crate::error::CacheResult;
use async_trait::async_trait;
use std::time::Duration;

/// Cache store trait for different cache backends.
#[async_trait]
pub trait CacheStore: Send + Sync {
    /// Get a JSON value from the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some(value))` if the key exists, `Ok(None)` if not found,
    /// or an error if the operation fails.
    async fn get_json(&self, key: &str) -> CacheResult<Option<String>>;

    /// Set a JSON value in the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    /// * `value` - The JSON string value
    /// * `ttl` - Optional time-to-live duration
    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()>;

    /// Delete a key from the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key to delete
    async fn delete(&self, key: &str) -> CacheResult<()>;

    /// Check if a key exists in the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key to check
    async fn exists(&self, key: &str) -> CacheResult<bool>;

    /// Clear all keys from the cache.
    ///
    /// **Warning:** This operation may be destructive and affect all keys.
    async fn clear(&self) -> CacheResult<()>;

    /// Get the TTL (time-to-live) of a key.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some(duration))` if the key has a TTL, `Ok(None)` if the key
    /// has no expiration or doesn't exist.
    async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>>;

    /// Set or update the expiration time for a key.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    /// * `ttl` - The new time-to-live duration
    async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()>;

    /// Increment a numeric value.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    /// * `delta` - The amount to increment by
    ///
    /// # Returns
    ///
    /// Returns the new value after incrementing.
    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64>;

    /// Decrement a numeric value.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key
    /// * `delta` - The amount to decrement by
    ///
    /// # Returns
    ///
    /// Returns the new value after decrementing.
    async fn decrement(&self, key: &str, delta: i64) -> CacheResult<i64>;

    // ========== Native Batch Primitives ==========
    //
    // These are the low-level batch operations that backends can override with
    // a single native command (e.g. Redis `MGET`/`MSET`/variadic `DEL`) to turn
    // N round-trips into one. The DEFAULT implementations fall back to the
    // per-key loop (run concurrently), so existing `CacheStore` impls keep
    // working unchanged. The higher-level `get_many`/`set_many`/`delete_many`
    // methods and `ParallelCacheOps` delegate here, so overriding these three
    // methods is enough to accelerate all batch APIs.

    /// Get multiple keys in a single batch operation.
    ///
    /// Returns a vector of `Option<String>` in the **same order** as `keys`;
    /// `None` indicates a missing key.
    ///
    /// The default implementation issues one `get_json` per key concurrently;
    /// backends should override this with a native multi-get (e.g. `MGET`).
    async fn mget(&self, keys: &[&str]) -> CacheResult<Vec<Option<String>>> {
        use futures::future::try_join_all;

        let futures = keys.iter().map(|key| self.get_json(key));
        try_join_all(futures).await
    }

    /// Set multiple key/value pairs in a single batch operation.
    ///
    /// The default implementation issues one `set_json` per pair concurrently;
    /// backends should override this with a native multi-set (e.g. `MSET`, or a
    /// pipeline of `SET ... EX` when a TTL is required).
    async fn mset(&self, items: &[(&str, String)], ttl: Option<Duration>) -> CacheResult<()> {
        use futures::future::try_join_all;

        let futures = items
            .iter()
            .map(|(key, value)| self.set_json(key, value.clone(), ttl));
        try_join_all(futures).await?;
        Ok(())
    }

    /// Delete multiple keys in a single batch operation.
    ///
    /// The default implementation issues one `delete` per key concurrently;
    /// backends should override this with a variadic `DEL`/`UNLINK`.
    async fn mdel(&self, keys: &[&str]) -> CacheResult<()> {
        use futures::future::try_join_all;

        let futures = keys.iter().map(|key| self.delete(key));
        try_join_all(futures).await?;
        Ok(())
    }

    // ========== Batch Operations (Parallel) ==========

    /// Get multiple keys in parallel.
    ///
    /// This operation fetches multiple cache keys concurrently, significantly
    /// reducing total latency compared to sequential gets.
    ///
    /// # Arguments
    ///
    /// * `keys` - Slice of cache keys to fetch
    ///
    /// # Returns
    ///
    /// Returns a vector of `Option<String>` in the same order as the input keys.
    /// `None` indicates the key was not found.
    ///
    /// # Performance
    ///
    /// - **Sequential:** O(n * network_latency)
    /// - **Parallel:** O(max(network_latencies)) ≈ O(network_latency)
    /// - **Speedup:** 10-100x for network-bound operations
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_cache::*;
    /// # async fn example(cache: &impl CacheStore) -> CacheResult<()> {
    /// // Fetch 100 user profiles in parallel
    /// let keys: Vec<String> = (1..=100).map(|i| format!("user:{}", i)).collect();
    /// let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
    /// let profiles = cache.get_many(&key_refs).await?;
    ///
    /// // Sequential: ~1000ms (10ms * 100)
    /// // Parallel:   ~15ms (max of all parallel requests)
    /// # Ok(())
    /// # }
    /// ```
    async fn get_many(&self, keys: &[&str]) -> CacheResult<Vec<Option<String>>> {
        // Delegate to the native batch primitive so backends that override
        // `mget` (e.g. Redis `MGET`) accelerate this path automatically.
        self.mget(keys).await
    }

    /// Set multiple key-value pairs in parallel.
    ///
    /// # Arguments
    ///
    /// * `items` - Slice of (key, value) tuples
    /// * `ttl` - Optional time-to-live for all keys
    ///
    /// # Performance
    ///
    /// 10-100x faster than sequential sets for network-bound operations.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_cache::*;
    /// # async fn example(cache: &impl CacheStore) -> CacheResult<()> {
    /// use std::time::Duration;
    ///
    /// let items = vec![
    ///     ("user:1", r#"{"name":"Alice"}"#.to_string()),
    ///     ("user:2", r#"{"name":"Bob"}"#.to_string()),
    /// ];
    ///
    /// cache.set_many(&items, Some(Duration::from_secs(3600))).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn set_many(&self, items: &[(&str, String)], ttl: Option<Duration>) -> CacheResult<()> {
        // Delegate to the native batch primitive (e.g. Redis `MSET`/pipeline).
        self.mset(items, ttl).await
    }

    /// Delete multiple keys in parallel.
    ///
    /// # Arguments
    ///
    /// * `keys` - Slice of cache keys to delete
    ///
    /// # Performance
    ///
    /// 10-100x faster than sequential deletes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_cache::*;
    /// # async fn example(cache: &impl CacheStore) -> CacheResult<()> {
    /// // Bulk cache invalidation
    /// let keys = vec!["session:1", "session:2", "session:3"];
    /// cache.delete_many(&keys).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn delete_many(&self, keys: &[&str]) -> CacheResult<()> {
        // Delegate to the native batch primitive (e.g. Redis variadic `DEL`).
        self.mdel(keys).await
    }

    /// Check existence of multiple keys in parallel.
    ///
    /// # Arguments
    ///
    /// * `keys` - Slice of cache keys to check
    ///
    /// # Returns
    ///
    /// Returns a vector of booleans in the same order as input keys.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_cache::*;
    /// # async fn example(cache: &impl CacheStore) -> CacheResult<()> {
    /// let keys = vec!["user:1", "user:2", "user:3"];
    /// let exists = cache.exists_many(&keys).await?;
    ///
    /// for (key, exists) in keys.iter().zip(exists.iter()) {
    ///     println!("{}: {}", key, exists);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn exists_many(&self, keys: &[&str]) -> CacheResult<Vec<bool>> {
        use futures::future::try_join_all;

        let futures = keys.iter().map(|key| self.exists(key));
        try_join_all(futures).await
    }

    /// Get TTL for multiple keys in parallel.
    ///
    /// # Arguments
    ///
    /// * `keys` - Slice of cache keys
    ///
    /// # Returns
    ///
    /// Returns a vector of `Option<Duration>` for each key.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_cache::*;
    /// # async fn example(cache: &impl CacheStore) -> CacheResult<()> {
    /// let keys = vec!["session:1", "session:2"];
    /// let ttls = cache.ttl_many(&keys).await?;
    ///
    /// for (key, ttl) in keys.iter().zip(ttls.iter()) {
    ///     match ttl {
    ///         Some(duration) => println!("{}: expires in {:?}", key, duration),
    ///         None => println!("{}: no expiration", key),
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn ttl_many(&self, keys: &[&str]) -> CacheResult<Vec<Option<Duration>>> {
        use futures::future::try_join_all;

        let futures = keys.iter().map(|key| self.ttl(key));
        try_join_all(futures).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiered::InMemoryCache;

    // `InMemoryCache` does NOT override `mget`/`mset`/`mdel`, so these tests
    // exercise the trait's DEFAULT (per-key loop) implementations.

    #[tokio::test]
    async fn test_mget_default_fallback_preserves_order() {
        let cache = InMemoryCache::new();
        cache.set_json("a", "1".to_string(), None).await.unwrap();
        cache.set_json("c", "3".to_string(), None).await.unwrap();

        let got = cache.mget(&["a", "b", "c"]).await.unwrap();
        assert_eq!(
            got,
            vec![Some("1".to_string()), None, Some("3".to_string())]
        );
    }

    #[tokio::test]
    async fn test_mset_and_mdel_default_fallback() {
        let cache = InMemoryCache::new();
        cache
            .mset(&[("x", "10".to_string()), ("y", "20".to_string())], None)
            .await
            .unwrap();
        assert_eq!(cache.get_json("x").await.unwrap(), Some("10".to_string()));
        assert_eq!(cache.get_json("y").await.unwrap(), Some("20".to_string()));

        cache.mdel(&["x", "y"]).await.unwrap();
        assert_eq!(cache.get_json("x").await.unwrap(), None);
        assert_eq!(cache.get_json("y").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_public_batch_methods_delegate_to_primitives() {
        let cache = InMemoryCache::new();
        cache.set_json("k1", "v1".to_string(), None).await.unwrap();

        // get_many delegates to mget
        let got = cache.get_many(&["k1", "k2"]).await.unwrap();
        assert_eq!(got, vec![Some("v1".to_string()), None]);

        // set_many delegates to mset
        cache
            .set_many(&[("k3", "v3".to_string())], None)
            .await
            .unwrap();
        assert_eq!(cache.get_json("k3").await.unwrap(), Some("v3".to_string()));

        // delete_many delegates to mdel
        cache.delete_many(&["k1", "k3"]).await.unwrap();
        assert_eq!(cache.get_json("k1").await.unwrap(), None);
        assert_eq!(cache.get_json("k3").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_mget_empty_keys() {
        let cache = InMemoryCache::new();
        let got = cache.mget(&[]).await.unwrap();
        assert!(got.is_empty());
    }
}
