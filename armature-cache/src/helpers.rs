//! Helper functions for common cache operations.

use crate::error::CacheResult;
use crate::traits::CacheStore;
use serde::{Serialize, de::DeserializeOwned};
use std::time::Duration;

/// Get a typed value from the cache.
pub async fn get<S: CacheStore, T: DeserializeOwned>(
    store: &S,
    key: &str,
) -> CacheResult<Option<T>> {
    if let Some(json) = store.get_json(key).await? {
        let value: T = serde_json::from_str(&json)
            .map_err(|e| crate::error::CacheError::Deserialization(e.to_string()))?;
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

/// Set a typed value in the cache.
///
/// A `ttl` of `None` means "unspecified" and lets the store fall back to its
/// configured `default_ttl`. Use [`set_forever`] to store an entry that
/// genuinely never expires.
pub async fn set<S: CacheStore, T: Serialize>(
    store: &S,
    key: &str,
    value: &T,
    ttl: Option<Duration>,
) -> CacheResult<()> {
    let json = serde_json::to_string(value)
        .map_err(|e| crate::error::CacheError::Serialization(e.to_string()))?;
    store.set_json(key, json, ttl).await
}

/// Set a typed value that never expires, bypassing the store's `default_ttl`.
///
/// See [`CacheStore::set_json_forever`] for why this is distinct from
/// `set(store, key, value, None)`.
pub async fn set_forever<S: CacheStore, T: Serialize>(
    store: &S,
    key: &str,
    value: &T,
) -> CacheResult<()> {
    let json = serde_json::to_string(value)
        .map_err(|e| crate::error::CacheError::Serialization(e.to_string()))?;
    store.set_json_forever(key, json).await
}

/// Remember a value for a given duration.
///
/// If the key exists, returns the cached value.
/// If not, calls the factory function, caches the result, and returns it.
pub async fn remember<S: CacheStore, T, F, Fut>(
    store: &S,
    key: &str,
    ttl: Duration,
    factory: F,
) -> CacheResult<T>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CacheResult<T>>,
{
    if let Some(value) = get(store, key).await? {
        return Ok(value);
    }

    let value = factory().await?;
    set(store, key, &value, Some(ttl)).await?;
    Ok(value)
}

/// Remember a value forever (no expiry at all).
///
/// The cached entry is written through [`CacheStore::set_json_forever`], so it
/// is stored without expiry even on a store configured with a `default_ttl`.
/// Writing it as `set(.., None)` would instead resolve to that default,
/// turning "forever" into "however long the default happens to be".
pub async fn remember_forever<S: CacheStore, T, F, Fut>(
    store: &S,
    key: &str,
    factory: F,
) -> CacheResult<T>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = CacheResult<T>>,
{
    if let Some(value) = get(store, key).await? {
        return Ok(value);
    }

    let value = factory().await?;
    set_forever(store, key, &value).await?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// A store shaped like the real network backends: it carries a
    /// `default_ttl` and resolves a `None` TTL against it in `set_json`, and
    /// overrides `set_json_forever` to skip that fallback. `InMemoryCache` has
    /// no `default_ttl` concept, so it cannot exercise this distinction.
    /// key -> (serialized value, effective TTL the write applied).
    type WrittenEntries = Arc<RwLock<HashMap<String, (String, Option<Duration>)>>>;

    struct DefaultTtlCache {
        default_ttl: Option<Duration>,
        data: WrittenEntries,
    }

    impl DefaultTtlCache {
        fn new(default_ttl: Duration) -> Self {
            Self {
                default_ttl: Some(default_ttl),
                data: Arc::new(RwLock::new(HashMap::new())),
            }
        }

        async fn stored_ttl(&self, key: &str) -> Option<Duration> {
            self.data.read().await.get(key).and_then(|(_, ttl)| *ttl)
        }
    }

    #[async_trait]
    impl CacheStore for DefaultTtlCache {
        async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
            Ok(self.data.read().await.get(key).map(|(v, _)| v.clone()))
        }

        async fn set_json(
            &self,
            key: &str,
            value: String,
            ttl: Option<Duration>,
        ) -> CacheResult<()> {
            let effective = ttl.or(self.default_ttl);
            self.data
                .write()
                .await
                .insert(key.to_string(), (value, effective));
            Ok(())
        }

        async fn set_json_forever(&self, key: &str, value: String) -> CacheResult<()> {
            self.data
                .write()
                .await
                .insert(key.to_string(), (value, None));
            Ok(())
        }

        async fn delete(&self, key: &str) -> CacheResult<()> {
            self.data.write().await.remove(key);
            Ok(())
        }

        async fn exists(&self, key: &str) -> CacheResult<bool> {
            Ok(self.data.read().await.contains_key(key))
        }

        async fn clear(&self) -> CacheResult<()> {
            self.data.write().await.clear();
            Ok(())
        }

        async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
            Ok(self.stored_ttl(key).await)
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

    /// Regression: on a store with a `default_ttl`, `remember_forever` must
    /// produce an entry with NO expiry. It previously wrote via
    /// `set(.., None)`, which the backend resolved to `default_ttl`, making a
    /// non-expiring entry unobtainable.
    #[tokio::test]
    async fn test_remember_forever_bypasses_default_ttl() {
        let store = DefaultTtlCache::new(Duration::from_secs(300));

        let value: i64 = remember_forever(&store, "k", || async { Ok(7) })
            .await
            .unwrap();
        assert_eq!(value, 7);

        assert_eq!(
            store.stored_ttl("k").await,
            None,
            "remember_forever must store without expiry, not with the default TTL"
        );
    }

    /// The contrast case: an unspecified TTL still resolves to `default_ttl`,
    /// so the existing `Option<Duration>` semantics are unchanged.
    #[tokio::test]
    async fn test_set_with_none_ttl_still_uses_default_ttl() {
        let store = DefaultTtlCache::new(Duration::from_secs(300));

        set(&store, "k", &7_i64, None).await.unwrap();
        assert_eq!(
            store.stored_ttl("k").await,
            Some(Duration::from_secs(300)),
            "an unspecified TTL must keep falling back to default_ttl"
        );

        set_forever(&store, "k2", &7_i64).await.unwrap();
        assert_eq!(store.stored_ttl("k2").await, None);
    }

    /// A cache hit short-circuits before the factory runs, for both
    /// `remember` and `remember_forever`.
    #[tokio::test]
    async fn test_remember_forever_returns_cached_value() {
        let store = DefaultTtlCache::new(Duration::from_secs(300));
        set_forever(&store, "k", &1_i64).await.unwrap();

        let value: i64 = remember_forever(&store, "k", || async {
            panic!("factory must not run on a cache hit")
        })
        .await
        .unwrap();
        assert_eq!(value, 1);
    }
}
