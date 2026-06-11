//! Parallel batch operations for cache stores.

use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use futures::future::{join_all, try_join_all};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::time::Duration;

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
        let futures = keys.iter().map(|key| store.get_json(key));
        let results: Vec<CacheResult<Option<String>>> = join_all(futures).await;

        results.into_iter().collect()
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
        let futures = items
            .iter()
            .map(|(key, value)| store.set_json(key, value.clone(), ttl));

        try_join_all(futures).await?;
        Ok(())
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
        let futures = keys.iter().map(|key| store.delete(key));
        try_join_all(futures).await?;
        Ok(())
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
        let mut futures = Vec::new();

        for key in keys {
            let fut = async {
                let value = factory(key).await?;
                let json = serde_json::to_string(&value)
                    .map_err(|e| CacheError::Serialization(e.to_string()))?;
                store.set_json(key, json, ttl).await?;
                Ok::<(), CacheError>(())
            };
            futures.push(fut);
        }

        try_join_all(futures).await?;
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
    #[test]
    fn test_parallel_ops_exist() {
        // Ensure the module compiles - this test validates the module is correctly defined
    }
}
