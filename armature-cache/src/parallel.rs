//! Parallel batch operations for cache stores.

use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use futures::StreamExt;
use futures::future::join_all;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::time::Duration;

/// Default cap on how many warm-up factories [`ParallelCacheOps::warm_cache`]
/// runs at once.
///
/// The factory is a user-supplied loader — typically a database query — so an
/// unbounded fan-out would turn a 10,000-key warm-up into 10,000 simultaneous
/// queries: precisely the stampede that
/// [`CacheManager::get_or_set`](crate::manager::CacheManager::get_or_set)'s
/// single-flight exists to prevent. Use
/// [`ParallelCacheOps::warm_cache_with_concurrency`] to pick a different limit.
pub const DEFAULT_WARM_CONCURRENCY: usize = 32;

/// Parallel batch operations for cache stores.
///
/// This module provides high-performance batch operations that execute
/// multiple cache operations concurrently, significantly reducing total latency.
///
/// # Performance
///
/// - **get_many**: 10-100x faster than sequential gets (depending on network latency)
/// - **set_many**: 10-100x faster than sequential sets
/// - **delete_many**: Similar performance gains
///
/// # Examples
///
/// ```no_run
/// use armature_cache::*;
/// use armature_cache::parallel::*;
///
/// # async fn example() -> CacheResult<()> {
/// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
///
/// // Get multiple keys in parallel
/// let keys = vec!["user:1", "user:2", "user:3"];
/// let values = get_many_json(&cache, &keys).await?;
///
/// // Set multiple keys in parallel
/// let items = vec![
///     ("key1", "value1".to_string()),
///     ("key2", "value2".to_string()),
/// ];
/// set_many_json(&cache, &items, None).await?;
/// # Ok(())
/// # }
/// ```
pub struct ParallelCacheOps;

impl ParallelCacheOps {
    /// Get multiple JSON values in parallel.
    ///
    /// # Arguments
    ///
    /// * `store` - The cache store
    /// * `keys` - Slice of keys to fetch
    ///
    /// # Returns
    ///
    /// A vector of optional values in the same order as keys.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["key1", "key2", "key3"];
    /// let values = ParallelCacheOps::get_many_json(&cache, &keys).await?;
    ///
    /// for (key, value) in keys.iter().zip(values.iter()) {
    ///     println!("{}: {:?}", key, value);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_many_json<S: CacheStore>(
        store: &S,
        keys: &[&str],
    ) -> CacheResult<Vec<Option<String>>> {
        // Delegate to the store's native batch primitive. Backends like Redis
        // collapse this into a single `MGET` round-trip; others fall back to the
        // concurrent per-key loop. Order matches `keys` in both cases.
        store.mget(keys).await
    }

    /// Get multiple typed values in parallel.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type to deserialize into
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct User {
    ///     id: u64,
    ///     name: String,
    /// }
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["user:1", "user:2", "user:3"];
    /// let users: Vec<Option<User>> = ParallelCacheOps::get_many(&cache, &keys).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_many<S: CacheStore, T: DeserializeOwned>(
        store: &S,
        keys: &[&str],
    ) -> CacheResult<Vec<Option<T>>> {
        let json_values = Self::get_many_json(store, keys).await?;

        json_values
            .into_iter()
            .map(|opt_json| {
                opt_json
                    .map(|json| {
                        serde_json::from_str(&json)
                            .map_err(|e| CacheError::Deserialization(e.to_string()))
                    })
                    .transpose()
            })
            .collect()
    }

    /// Set multiple JSON values in parallel.
    ///
    /// # Arguments
    ///
    /// * `store` - The cache store
    /// * `items` - Slice of (key, value) tuples
    /// * `ttl` - Optional time-to-live for all items
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let items = vec![
    ///     ("key1", r#"{"value": 1}"#.to_string()),
    ///     ("key2", r#"{"value": 2}"#.to_string()),
    /// ];
    ///
    /// ParallelCacheOps::set_many_json(&cache, &items, Some(Duration::from_secs(3600))).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_many_json<S: CacheStore>(
        store: &S,
        items: &[(&str, String)],
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        // Delegate to the store's native batch primitive (e.g. Redis `MSET` /
        // pipelined `SET ... EX`), falling back to the per-key loop otherwise.
        store.mset(items, ttl).await
    }

    /// Set multiple typed values in parallel.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type to serialize from
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct Counter {
    ///     count: u64,
    /// }
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let items = vec![
    ///     ("counter:1", Counter { count: 10 }),
    ///     ("counter:2", Counter { count: 20 }),
    /// ];
    ///
    /// ParallelCacheOps::set_many(&cache, &items, None).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_many<S: CacheStore, T: Serialize>(
        store: &S,
        items: &[(&str, T)],
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        let json_items: Result<Vec<_>, _> = items
            .iter()
            .map(|(key, value)| {
                serde_json::to_string(value)
                    .map(|json| (*key, json))
                    .map_err(|e| CacheError::Serialization(e.to_string()))
            })
            .collect();

        let json_items = json_items?;
        let item_refs: Vec<_> = json_items.iter().map(|(k, v)| (*k, v.clone())).collect();

        Self::set_many_json(store, &item_refs, ttl).await
    }

    /// Delete multiple keys in parallel.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["key1", "key2", "key3"];
    /// ParallelCacheOps::delete_many(&cache, &keys).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_many<S: CacheStore>(store: &S, keys: &[&str]) -> CacheResult<()> {
        // Delegate to the store's native batch primitive (e.g. Redis variadic
        // `DEL`), falling back to the concurrent per-key loop otherwise.
        store.mdel(keys).await
    }

    /// Check if multiple keys exist in parallel.
    ///
    /// # Returns
    ///
    /// A vector of booleans indicating existence, in the same order as keys.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["key1", "key2", "key3"];
    /// let exists = ParallelCacheOps::exists_many(&cache, &keys).await?;
    ///
    /// for (key, exists) in keys.iter().zip(exists.iter()) {
    ///     println!("{}: {}", key, exists);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn exists_many<S: CacheStore>(store: &S, keys: &[&str]) -> CacheResult<Vec<bool>> {
        let futures = keys.iter().map(|key| store.exists(key));
        let results: Vec<CacheResult<bool>> = join_all(futures).await;

        results.into_iter().collect()
    }

    /// Get TTL for multiple keys in parallel.
    ///
    /// # Returns
    ///
    /// A vector of optional durations, in the same order as keys.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["key1", "key2", "key3"];
    /// let ttls = ParallelCacheOps::ttl_many(&cache, &keys).await?;
    ///
    /// for (key, ttl) in keys.iter().zip(ttls.iter()) {
    ///     println!("{}: {:?}", key, ttl);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn ttl_many<S: CacheStore>(
        store: &S,
        keys: &[&str],
    ) -> CacheResult<Vec<Option<Duration>>> {
        let futures = keys.iter().map(|key| store.ttl(key));
        let results: Vec<CacheResult<Option<Duration>>> = join_all(futures).await;

        results.into_iter().collect()
    }

    /// Cache warming: preload multiple keys into cache.
    ///
    /// # Concurrency
    ///
    /// At most [`DEFAULT_WARM_CONCURRENCY`] factories run at a time. The
    /// factory is the caller's loader (usually a DB fetch), so warming a large
    /// key set without a bound would fire one query per key simultaneously.
    /// Use [`Self::warm_cache_with_concurrency`] to choose the limit.
    ///
    /// # Type Parameters
    ///
    /// * `T` - The type to serialize
    /// * `F` - Factory function that returns data for a given key
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use armature_cache::*;
    /// use armature_cache::parallel::ParallelCacheOps;
    /// use std::time::Duration;
    ///
    /// # async fn example() -> CacheResult<()> {
    /// let cache = RedisCache::new(CacheConfig::redis("redis://localhost:6379")?).await?;
    ///
    /// let keys = vec!["user:1", "user:2", "user:3"];
    ///
    /// ParallelCacheOps::warm_cache(
    ///     &cache,
    ///     &keys,
    ///     Some(Duration::from_secs(3600)),
    ///     |key: &str| async move {
    ///         // Fetch from database
    ///         let data = format!("Data for {}", key);
    ///         Ok::<String, CacheError>(data)
    ///     },
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn warm_cache<S, T, F, Fut>(
        store: &S,
        keys: &[&str],
        ttl: Option<Duration>,
        factory: F,
    ) -> CacheResult<()>
    where
        S: CacheStore,
        T: Serialize,
        F: Fn(&str) -> Fut,
        Fut: std::future::Future<Output = CacheResult<T>>,
    {
        Self::warm_cache_with_concurrency(store, keys, ttl, DEFAULT_WARM_CONCURRENCY, factory).await
    }

    /// Cache warming with an explicit cap on concurrent factory invocations.
    ///
    /// `max_concurrent` is the number of factories (and their follow-up
    /// writes) allowed to be in flight at once; `0` is treated as `1`. See
    /// [`DEFAULT_WARM_CONCURRENCY`] for why the fan-out is bounded at all.
    ///
    /// The first error aborts the warm-up: keys already written stay written,
    /// and the remaining ones are not attempted.
    pub async fn warm_cache_with_concurrency<S, T, F, Fut>(
        store: &S,
        keys: &[&str],
        ttl: Option<Duration>,
        max_concurrent: usize,
        factory: F,
    ) -> CacheResult<()>
    where
        S: CacheStore,
        T: Serialize,
        F: Fn(&str) -> Fut,
        Fut: std::future::Future<Output = CacheResult<T>>,
    {
        let limit = max_concurrent.max(1);
        let factory = &factory;

        let mut warmed = futures::stream::iter(keys.iter().map(|key| async move {
            let value = factory(key).await?;
            let json = serde_json::to_string(&value)
                .map_err(|e| CacheError::Serialization(e.to_string()))?;
            store.set_json(key, json, ttl).await?;
            Ok::<(), CacheError>(())
        }))
        .buffer_unordered(limit);

        while let Some(result) = warmed.next().await {
            result?;
        }

        Ok(())
    }
}

/// Helper functions for parallel cache operations.
///
/// These functions provide a more convenient API than `ParallelCacheOps` methods.
/// Get multiple JSON values in parallel.
pub async fn get_many_json<S: CacheStore>(
    store: &S,
    keys: &[&str],
) -> CacheResult<Vec<Option<String>>> {
    ParallelCacheOps::get_many_json(store, keys).await
}

/// Get multiple typed values in parallel.
pub async fn get_many<S: CacheStore, T: DeserializeOwned>(
    store: &S,
    keys: &[&str],
) -> CacheResult<Vec<Option<T>>> {
    ParallelCacheOps::get_many(store, keys).await
}

/// Set multiple JSON values in parallel.
pub async fn set_many_json<S: CacheStore>(
    store: &S,
    items: &[(&str, String)],
    ttl: Option<Duration>,
) -> CacheResult<()> {
    ParallelCacheOps::set_many_json(store, items, ttl).await
}

/// Set multiple typed values in parallel.
pub async fn set_many<S: CacheStore, T: Serialize>(
    store: &S,
    items: &[(&str, T)],
    ttl: Option<Duration>,
) -> CacheResult<()> {
    ParallelCacheOps::set_many(store, items, ttl).await
}

/// Delete multiple keys in parallel.
pub async fn delete_many<S: CacheStore>(store: &S, keys: &[&str]) -> CacheResult<()> {
    ParallelCacheOps::delete_many(store, keys).await
}

/// Build a HashMap from multiple keys fetched in parallel.
pub async fn get_many_as_map<S: CacheStore, T: DeserializeOwned>(
    store: &S,
    keys: &[&str],
) -> CacheResult<HashMap<String, T>> {
    let values = get_many(store, keys).await?;

    let map: HashMap<String, T> = keys
        .iter()
        .zip(values)
        .filter_map(|(key, opt_value)| opt_value.map(|value| (key.to_string(), value)))
        .collect();

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiered::InMemoryCache;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Tracks how many factory invocations are in flight simultaneously.
    #[derive(Default)]
    struct ConcurrencyProbe {
        current: AtomicUsize,
        peak: AtomicUsize,
        total: AtomicUsize,
    }

    impl ConcurrencyProbe {
        fn enter(&self) {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.total.fetch_add(1, Ordering::SeqCst);
            self.peak.fetch_max(current, Ordering::SeqCst);
        }

        fn leave(&self) {
            self.current.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Regression: `warm_cache` used to `try_join_all` one future per key with
    /// no limit, so warming N keys ran N user factories (documented as DB
    /// fetches) simultaneously. The fan-out must now be bounded.
    #[tokio::test]
    async fn test_warm_cache_bounds_factory_concurrency() {
        let cache = InMemoryCache::new();
        let probe = Arc::new(ConcurrencyProbe::default());
        let keys: Vec<String> = (0..64).map(|i| format!("k{i}")).collect();
        let key_refs: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();

        const LIMIT: usize = 4;

        ParallelCacheOps::warm_cache_with_concurrency(
            &cache,
            &key_refs,
            None,
            LIMIT,
            |key: &str| {
                let probe = probe.clone();
                let key = key.to_string();
                async move {
                    probe.enter();
                    // Suspend so the other buffered futures get a chance to run
                    // and the peak reflects real overlap.
                    for _ in 0..3 {
                        tokio::task::yield_now().await;
                    }
                    probe.leave();
                    Ok::<String, CacheError>(format!("value-for-{key}"))
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(probe.total.load(Ordering::SeqCst), 64);
        assert!(
            probe.peak.load(Ordering::SeqCst) <= LIMIT,
            "warm_cache ran {} factories at once, limit was {LIMIT}",
            probe.peak.load(Ordering::SeqCst)
        );

        // Every key was still warmed.
        assert_eq!(
            cache.get_json("k0").await.unwrap(),
            Some("\"value-for-k0\"".to_string())
        );
        assert_eq!(
            cache.get_json("k63").await.unwrap(),
            Some("\"value-for-k63\"".to_string())
        );
    }

    /// The default entry point applies [`DEFAULT_WARM_CONCURRENCY`] rather
    /// than fanning out over every key.
    #[tokio::test]
    async fn test_warm_cache_default_limit_applies() {
        let cache = InMemoryCache::new();
        let probe = Arc::new(ConcurrencyProbe::default());
        let keys: Vec<String> = (0..DEFAULT_WARM_CONCURRENCY * 4)
            .map(|i| format!("k{i}"))
            .collect();
        let key_refs: Vec<&str> = keys.iter().map(|k| k.as_str()).collect();

        ParallelCacheOps::warm_cache(&cache, &key_refs, None, |_key: &str| {
            let probe = probe.clone();
            async move {
                probe.enter();
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                }
                probe.leave();
                Ok::<u32, CacheError>(1)
            }
        })
        .await
        .unwrap();

        assert!(probe.peak.load(Ordering::SeqCst) <= DEFAULT_WARM_CONCURRENCY);
        assert_eq!(
            probe.total.load(Ordering::SeqCst),
            DEFAULT_WARM_CONCURRENCY * 4
        );
    }

    /// A concurrency limit of 0 is clamped to 1 rather than deadlocking or
    /// silently doing nothing.
    #[tokio::test]
    async fn test_warm_cache_zero_concurrency_is_clamped_to_one() {
        let cache = InMemoryCache::new();
        let probe = Arc::new(ConcurrencyProbe::default());

        ParallelCacheOps::warm_cache_with_concurrency(
            &cache,
            &["a", "b", "c"],
            None,
            0,
            |_key: &str| {
                let probe = probe.clone();
                async move {
                    probe.enter();
                    tokio::task::yield_now().await;
                    probe.leave();
                    Ok::<u32, CacheError>(7)
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(probe.peak.load(Ordering::SeqCst), 1);
        assert_eq!(probe.total.load(Ordering::SeqCst), 3);
        assert_eq!(cache.get_json("c").await.unwrap(), Some("7".to_string()));
    }

    /// A factory failure aborts the warm-up and surfaces the error.
    #[tokio::test]
    async fn test_warm_cache_propagates_factory_error() {
        let cache = InMemoryCache::new();

        let result = ParallelCacheOps::warm_cache_with_concurrency(
            &cache,
            &["a", "b"],
            None,
            1,
            |key: &str| {
                let fails = key == "b";
                async move {
                    if fails {
                        Err(CacheError::Other("factory failed".to_string()))
                    } else {
                        Ok(1_u32)
                    }
                }
            },
        )
        .await;

        assert!(result.is_err());
    }
}
