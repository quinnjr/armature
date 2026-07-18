//! High-level cache manager with convenience methods.

use crate::error::CacheResult;
use crate::traits::CacheStore;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

/// High-level cache manager with type-safe operations.
pub struct CacheManager<S: CacheStore> {
    store: Arc<S>,
    /// Per-key single-flight locks for `get_or_set`.
    ///
    /// Coalesces concurrent misses on the same key so only one `factory()`
    /// (typically a DB hit) runs; the rest await it and then read the
    /// now-populated cache entry. The outer `std::sync::Mutex` only guards the
    /// short map lookup/insert; the actual loader runs while holding the
    /// per-key `tokio::sync::Mutex`.
    inflight: StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

impl<S: CacheStore> CacheManager<S> {
    /// Create a new cache manager.
    pub fn new(store: S) -> Self {
        Self {
            store: Arc::new(store),
            inflight: StdMutex::new(HashMap::new()),
        }
    }

    /// Get a typed value from the cache.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> CacheResult<Option<T>> {
        if let Some(json) = self.store.get_json(key).await? {
            let value: T = serde_json::from_str(&json)
                .map_err(|e| crate::error::CacheError::Deserialization(e.to_string()))?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Set a typed value in the cache.
    pub async fn set<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| crate::error::CacheError::Serialization(e.to_string()))?;
        self.store.set_json(key, json, ttl).await
    }

    /// Get or set a value using a factory function.
    ///
    /// If the key exists, returns the cached value.
    /// If not, calls the factory function, caches the result, and returns it.
    ///
    /// # Single-flight
    ///
    /// Concurrent misses on the same key are coalesced: only one caller runs
    /// `factory()` while the others wait and then read the value it cached.
    /// This prevents a cache-miss stampede (many simultaneous DB loads) for hot
    /// keys. The returned value is identical to the non-coalesced behavior.
    pub async fn get_or_set<T, F, Fut>(
        &self,
        key: &str,
        ttl: Option<Duration>,
        factory: F,
    ) -> CacheResult<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = CacheResult<T>>,
    {
        // Fast path: a cache hit avoids taking the single-flight lock entirely.
        if let Some(value) = self.get(key).await? {
            return Ok(value);
        }

        // Slow path: acquire a per-key lock so only one loader runs.
        let key_lock = {
            let mut map = self.inflight.lock().unwrap();
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let guard = key_lock.lock().await;

        // Double-check: another loader may have populated the cache while we
        // were waiting for the lock.
        let result = if let Some(value) = self.get(key).await? {
            Ok(value)
        } else {
            match factory().await {
                Ok(value) => {
                    self.set(key, &value, ttl).await?;
                    Ok(value)
                }
                Err(e) => Err(e),
            }
        };
        drop(guard);

        // Clean up the map entry once no other caller is still referencing this
        // key's lock, to keep the map from growing unbounded across many keys.
        {
            let mut map = self.inflight.lock().unwrap();
            if let Some(existing) = map.get(key)
                && Arc::ptr_eq(existing, &key_lock)
                && Arc::strong_count(&key_lock) == 2
            {
                // strong_count == 2 => only the map and our local `key_lock`
                // hold a reference; no other task is waiting.
                map.remove(key);
            }
        }

        result
    }

    /// Delete a key from the cache.
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        self.store.delete(key).await
    }

    /// Check if a key exists.
    pub async fn exists(&self, key: &str) -> CacheResult<bool> {
        self.store.exists(key).await
    }

    /// Clear all keys.
    pub async fn clear(&self) -> CacheResult<()> {
        self.store.clear().await
    }

    /// Get the TTL of a key.
    pub async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
        self.store.ttl(key).await
    }

    /// Set the expiration of a key.
    pub async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()> {
        self.store.expire(key, ttl).await
    }

    /// Create a namespaced cache manager.
    pub fn namespace(&self, prefix: &str) -> NamespacedCache<S> {
        NamespacedCache {
            store: self.store.clone(),
            prefix: prefix.to_string(),
        }
    }
}

/// Namespaced cache manager that automatically prefixes all keys.
pub struct NamespacedCache<S: CacheStore> {
    store: Arc<S>,
    prefix: String,
}

impl<S: CacheStore> NamespacedCache<S> {
    fn build_key(&self, key: &str) -> String {
        format!("{}:{}", self.prefix, key)
    }

    /// Get a typed value from the cache.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> CacheResult<Option<T>> {
        let key = self.build_key(key);
        if let Some(json) = self.store.get_json(&key).await? {
            let value: T = serde_json::from_str(&json)
                .map_err(|e| crate::error::CacheError::Deserialization(e.to_string()))?;
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// Set a typed value in the cache.
    pub async fn set<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        let key = self.build_key(key);
        let json = serde_json::to_string(value)
            .map_err(|e| crate::error::CacheError::Serialization(e.to_string()))?;
        self.store.set_json(&key, json, ttl).await
    }

    /// Delete a key from the cache.
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        let key = self.build_key(key);
        self.store.delete(&key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiered::InMemoryCache;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_get_or_set_single_flight_coalesces_factory() {
        let manager = CacheManager::new(InMemoryCache::new());
        let calls = Arc::new(AtomicUsize::new(0));

        // Launch many concurrent get_or_set on the SAME key. The factory yields
        // once so the concurrent futures interleave and all reach the miss path
        // before the first loader finishes; single-flight must ensure the
        // factory runs exactly once.
        let make_fut = |calls: Arc<AtomicUsize>| {
            let manager = &manager;
            async move {
                manager
                    .get_or_set::<i64, _, _>("hot-key", None, || {
                        let calls = calls.clone();
                        async move {
                            tokio::task::yield_now().await;
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(42)
                        }
                    })
                    .await
                    .unwrap()
            }
        };

        let futs = (0..16).map(|_| make_fut(calls.clone()));
        let results = futures::future::join_all(futs).await;

        assert!(results.iter().all(|&v| v == 42));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "factory should run exactly once under single-flight"
        );
    }

    #[tokio::test]
    async fn test_get_or_set_returns_cached_value_without_calling_factory() {
        let manager = CacheManager::new(InMemoryCache::new());
        manager.set("k", &7_i64, None).await.unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_c = calls.clone();
        let value: i64 = manager
            .get_or_set("k", None, || {
                let calls = calls_c.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(99)
                }
            })
            .await
            .unwrap();

        assert_eq!(value, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_get_or_set_inflight_map_cleaned_up() {
        let manager = CacheManager::new(InMemoryCache::new());
        let _: i64 = manager
            .get_or_set("k", None, || async { Ok(1) })
            .await
            .unwrap();

        // After completion the per-key lock entry should be removed.
        assert!(manager.inflight.lock().unwrap().is_empty());
    }

    #[test]
    fn test_namespace_build_key() {
        struct MockStore;

        #[async_trait::async_trait]
        impl CacheStore for MockStore {
            async fn get_json(&self, _key: &str) -> CacheResult<Option<String>> {
                Ok(None)
            }
            async fn set_json(
                &self,
                _key: &str,
                _value: String,
                _ttl: Option<Duration>,
            ) -> CacheResult<()> {
                Ok(())
            }
            async fn delete(&self, _key: &str) -> CacheResult<()> {
                Ok(())
            }
            async fn exists(&self, _key: &str) -> CacheResult<bool> {
                Ok(false)
            }
            async fn clear(&self) -> CacheResult<()> {
                Ok(())
            }
            async fn ttl(&self, _key: &str) -> CacheResult<Option<Duration>> {
                Ok(None)
            }
            async fn expire(&self, _key: &str, _ttl: Duration) -> CacheResult<()> {
                Ok(())
            }
            async fn increment(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
            async fn decrement(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
        }

        let namespaced = NamespacedCache {
            store: Arc::new(MockStore),
            prefix: "users".to_string(),
        };

        assert_eq!(namespaced.build_key("123"), "users:123");
    }

    #[test]
    fn test_namespace_build_key_empty() {
        struct MockStore;

        #[async_trait::async_trait]
        impl CacheStore for MockStore {
            async fn get_json(&self, _key: &str) -> CacheResult<Option<String>> {
                Ok(None)
            }
            async fn set_json(
                &self,
                _key: &str,
                _value: String,
                _ttl: Option<Duration>,
            ) -> CacheResult<()> {
                Ok(())
            }
            async fn delete(&self, _key: &str) -> CacheResult<()> {
                Ok(())
            }
            async fn exists(&self, _key: &str) -> CacheResult<bool> {
                Ok(false)
            }
            async fn clear(&self) -> CacheResult<()> {
                Ok(())
            }
            async fn ttl(&self, _key: &str) -> CacheResult<Option<Duration>> {
                Ok(None)
            }
            async fn expire(&self, _key: &str, _ttl: Duration) -> CacheResult<()> {
                Ok(())
            }
            async fn increment(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
            async fn decrement(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
        }

        let namespaced = NamespacedCache {
            store: Arc::new(MockStore),
            prefix: "app".to_string(),
        };

        assert_eq!(namespaced.build_key(""), "app:");
    }

    #[test]
    fn test_namespace_build_key_with_colons() {
        struct MockStore;

        #[async_trait::async_trait]
        impl CacheStore for MockStore {
            async fn get_json(&self, _key: &str) -> CacheResult<Option<String>> {
                Ok(None)
            }
            async fn set_json(
                &self,
                _key: &str,
                _value: String,
                _ttl: Option<Duration>,
            ) -> CacheResult<()> {
                Ok(())
            }
            async fn delete(&self, _key: &str) -> CacheResult<()> {
                Ok(())
            }
            async fn exists(&self, _key: &str) -> CacheResult<bool> {
                Ok(false)
            }
            async fn clear(&self) -> CacheResult<()> {
                Ok(())
            }
            async fn ttl(&self, _key: &str) -> CacheResult<Option<Duration>> {
                Ok(None)
            }
            async fn expire(&self, _key: &str, _ttl: Duration) -> CacheResult<()> {
                Ok(())
            }
            async fn increment(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
            async fn decrement(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
        }

        let namespaced = NamespacedCache {
            store: Arc::new(MockStore),
            prefix: "app".to_string(),
        };

        assert_eq!(namespaced.build_key("user:123"), "app:user:123");
    }

    #[test]
    fn test_namespace_multiple_prefixes() {
        struct MockStore;

        #[async_trait::async_trait]
        impl CacheStore for MockStore {
            async fn get_json(&self, _key: &str) -> CacheResult<Option<String>> {
                Ok(None)
            }
            async fn set_json(
                &self,
                _key: &str,
                _value: String,
                _ttl: Option<Duration>,
            ) -> CacheResult<()> {
                Ok(())
            }
            async fn delete(&self, _key: &str) -> CacheResult<()> {
                Ok(())
            }
            async fn exists(&self, _key: &str) -> CacheResult<bool> {
                Ok(false)
            }
            async fn clear(&self) -> CacheResult<()> {
                Ok(())
            }
            async fn ttl(&self, _key: &str) -> CacheResult<Option<Duration>> {
                Ok(None)
            }
            async fn expire(&self, _key: &str, _ttl: Duration) -> CacheResult<()> {
                Ok(())
            }
            async fn increment(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
            async fn decrement(&self, _key: &str, _delta: i64) -> CacheResult<i64> {
                Ok(0)
            }
        }

        let ns1 = NamespacedCache {
            store: Arc::new(MockStore),
            prefix: "app1".to_string(),
        };

        let ns2 = NamespacedCache {
            store: Arc::new(MockStore),
            prefix: "app2".to_string(),
        };

        assert_eq!(ns1.build_key("key"), "app1:key");
        assert_eq!(ns2.build_key("key"), "app2:key");
        assert_ne!(ns1.build_key("key"), ns2.build_key("key"));
    }
}
