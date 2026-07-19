//! Tenant-Aware Caching
//!
//! Provides automatic cache key prefixing for multi-tenant applications.

use crate::tenant::Tenant;
use async_trait::async_trait;
use std::time::Duration;

/// Cache provider trait
///
/// Users implement this with their cache backend (Redis, Memcached, etc.).
#[async_trait]
pub trait CacheProvider: Send + Sync {
    /// Get value from cache
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError>;

    /// Set value in cache
    async fn set(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>)
    -> Result<(), CacheError>;

    /// Delete value from cache
    async fn delete(&self, key: &str) -> Result<(), CacheError>;

    /// Delete multiple keys from cache.
    ///
    /// Used by bulk operations such as [`TenantCache::clear_tenant`] to
    /// avoid one round-trip per key. The default implementation simply
    /// loops over [`Self::delete`], so existing `CacheProvider`
    /// implementations keep compiling and behaving identically without any
    /// changes. Providers with a native batch/variadic delete (e.g. Redis
    /// `DEL`/`UNLINK`) should override this for a real performance win.
    async fn delete_many(&self, keys: &[String]) -> Result<(), CacheError> {
        for key in keys {
            self.delete(key).await?;
        }
        Ok(())
    }

    /// Check if key exists
    async fn exists(&self, key: &str) -> Result<bool, CacheError>;

    /// Clear all keys (use with caution!)
    async fn clear(&self) -> Result<(), CacheError>;

    /// List all keys matching a given prefix.
    ///
    /// Used to support prefix-scoped bulk operations such as
    /// [`TenantCache::clear_tenant`]. Providers that support pattern/prefix
    /// scanning (e.g. Redis `SCAN`) should override this; an in-memory
    /// provider can simply filter its keyspace by prefix.
    ///
    /// The default implementation reports the operation as unsupported so
    /// existing `CacheProvider` implementations keep compiling without any
    /// changes.
    async fn keys_with_prefix(&self, _prefix: &str) -> Result<Vec<String>, CacheError> {
        Err(CacheError::Error(
            "keys_with_prefix is not implemented by this cache provider".to_string(),
        ))
    }
}

/// Cache errors
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Cache error: {0}")]
    Error(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Connection error: {0}")]
    Connection(String),
}

/// Tenant-aware cache wrapper
///
/// Automatically prefixes cache keys with tenant ID.
pub struct TenantCache<P: CacheProvider> {
    provider: P,
}

impl<P: CacheProvider> TenantCache<P> {
    /// Create new tenant cache with injected provider
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let redis_provider = RedisCache::new("redis://localhost");
    /// let cache = TenantCache::new(redis_provider);
    /// ```
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Get value with tenant prefix
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let value = cache.get(&tenant, "users:1").await?;
    /// ```
    pub async fn get(&self, tenant: &Tenant, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
        let prefixed_key = tenant.cache_key(key);
        self.provider.get(&prefixed_key).await
    }

    /// Set value with tenant prefix
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// cache.set(&tenant, "users:1", data, Some(Duration::from_secs(3600))).await?;
    /// ```
    pub async fn set(
        &self,
        tenant: &Tenant,
        key: &str,
        value: Vec<u8>,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let prefixed_key = tenant.cache_key(key);
        self.provider.set(&prefixed_key, value, ttl).await
    }

    /// Delete value with tenant prefix
    pub async fn delete(&self, tenant: &Tenant, key: &str) -> Result<(), CacheError> {
        let prefixed_key = tenant.cache_key(key);
        self.provider.delete(&prefixed_key).await
    }

    /// Check if key exists with tenant prefix
    pub async fn exists(&self, tenant: &Tenant, key: &str) -> Result<bool, CacheError> {
        let prefixed_key = tenant.cache_key(key);
        self.provider.exists(&prefixed_key).await
    }

    /// Clear all tenant keys
    ///
    /// **Warning**: This clears ALL keys for the tenant!
    ///
    /// Scans the provider for every key under this tenant's prefix (via
    /// [`CacheProvider::keys_with_prefix`]) and deletes each one.
    pub async fn clear_tenant(&self, tenant: &Tenant) -> Result<(), CacheError> {
        let prefix = tenant.cache_key("");
        let keys = self.provider.keys_with_prefix(&prefix).await?;
        self.provider.delete_many(&keys).await?;
        Ok(())
    }

    /// Get value with JSON deserialization
    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        tenant: &Tenant,
        key: &str,
    ) -> Result<Option<T>, CacheError> {
        match self.get(tenant, key).await? {
            Some(data) => {
                let value = serde_json::from_slice(&data)
                    .map_err(|e| CacheError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Set value with JSON serialization
    pub async fn set_json<T: serde::Serialize>(
        &self,
        tenant: &Tenant,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> Result<(), CacheError> {
        let data =
            serde_json::to_vec(value).map_err(|e| CacheError::Serialization(e.to_string()))?;
        self.set(tenant, key, data, ttl).await
    }
}

/// Cache key builder
///
/// Helps build complex cache keys with tenant prefix.
pub struct CacheKeyBuilder {
    parts: Vec<String>,
}

impl CacheKeyBuilder {
    /// Create new key builder
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    /// Add part to key
    pub fn part(mut self, part: impl Into<String>) -> Self {
        self.parts.push(part.into());
        self
    }

    /// Build final key (without tenant prefix)
    pub fn build(&self) -> String {
        self.parts.join(":")
    }

    /// Build final key with tenant prefix
    pub fn build_for_tenant(&self, tenant: &Tenant) -> String {
        tenant.cache_key(&self.build())
    }
}

impl Default for CacheKeyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    struct MockCacheProvider {
        data: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MockCacheProvider {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl CacheProvider for MockCacheProvider {
        async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, CacheError> {
            let data = self.data.lock().await;
            Ok(data.get(key).cloned())
        }

        async fn set(
            &self,
            key: &str,
            value: Vec<u8>,
            _ttl: Option<Duration>,
        ) -> Result<(), CacheError> {
            let mut data = self.data.lock().await;
            data.insert(key.to_string(), value);
            Ok(())
        }

        async fn delete(&self, key: &str) -> Result<(), CacheError> {
            let mut data = self.data.lock().await;
            data.remove(key);
            Ok(())
        }

        async fn exists(&self, key: &str) -> Result<bool, CacheError> {
            let data = self.data.lock().await;
            Ok(data.contains_key(key))
        }

        async fn clear(&self) -> Result<(), CacheError> {
            let mut data = self.data.lock().await;
            data.clear();
            Ok(())
        }

        async fn keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, CacheError> {
            let data = self.data.lock().await;
            Ok(data
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
    }

    #[tokio::test]
    async fn test_tenant_cache_set_get() {
        let provider = MockCacheProvider::new();
        let cache = TenantCache::new(provider);

        let tenant = Tenant::new("tenant-1", "acme");
        let value = b"test data".to_vec();

        cache
            .set(&tenant, "test:key", value.clone(), None)
            .await
            .unwrap();
        let retrieved = cache.get(&tenant, "test:key").await.unwrap();

        assert_eq!(retrieved, Some(value));
    }

    #[tokio::test]
    async fn test_tenant_isolation() {
        let provider = MockCacheProvider::new();
        let cache = TenantCache::new(provider);

        let tenant1 = Tenant::new("tenant-1", "acme");
        let tenant2 = Tenant::new("tenant-2", "globex");

        cache
            .set(&tenant1, "key", b"value1".to_vec(), None)
            .await
            .unwrap();
        cache
            .set(&tenant2, "key", b"value2".to_vec(), None)
            .await
            .unwrap();

        let val1 = cache.get(&tenant1, "key").await.unwrap();
        let val2 = cache.get(&tenant2, "key").await.unwrap();

        assert_eq!(val1, Some(b"value1".to_vec()));
        assert_eq!(val2, Some(b"value2".to_vec()));
    }

    #[tokio::test]
    async fn test_cache_json() {
        let provider = MockCacheProvider::new();
        let cache = TenantCache::new(provider);

        let tenant = Tenant::new("tenant-1", "acme");

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct User {
            id: u32,
            name: String,
        }

        let user = User {
            id: 1,
            name: "Alice".to_string(),
        };

        cache
            .set_json(&tenant, "user:1", &user, None)
            .await
            .unwrap();
        let retrieved: Option<User> = cache.get_json(&tenant, "user:1").await.unwrap();

        assert_eq!(retrieved, Some(user));
    }

    #[tokio::test]
    async fn test_cache_key_builder() {
        let tenant = Tenant::new("tenant-123", "acme");

        let key = CacheKeyBuilder::new()
            .part("users")
            .part("1")
            .part("profile")
            .build_for_tenant(&tenant);

        assert_eq!(key, "tenant:tenant-123:users:1:profile");
    }

    #[tokio::test]
    async fn test_clear_tenant_removes_all_tenant_keys_by_prefix() {
        let provider = MockCacheProvider::new();
        let cache = TenantCache::new(provider);

        let tenant = Tenant::new("tenant-1", "acme");
        let other_tenant = Tenant::new("tenant-2", "globex");

        // N keys for the target tenant.
        const N: usize = 25;
        for i in 0..N {
            cache
                .set(&tenant, &format!("key-{i}"), b"value".to_vec(), None)
                .await
                .unwrap();
        }
        // A key for a different tenant that must survive.
        cache
            .set(&other_tenant, "key-0", b"other".to_vec(), None)
            .await
            .unwrap();

        cache.clear_tenant(&tenant).await.unwrap();

        for i in 0..N {
            assert!(
                !cache.exists(&tenant, &format!("key-{i}")).await.unwrap(),
                "key-{i} should have been cleared"
            );
        }
        assert!(
            cache.exists(&other_tenant, "key-0").await.unwrap(),
            "other tenant's key must not be affected"
        );
    }

    #[tokio::test]
    async fn test_cache_delete() {
        let provider = MockCacheProvider::new();
        let cache = TenantCache::new(provider);

        let tenant = Tenant::new("tenant-1", "acme");

        cache
            .set(&tenant, "key", b"value".to_vec(), None)
            .await
            .unwrap();
        assert!(cache.exists(&tenant, "key").await.unwrap());

        cache.delete(&tenant, "key").await.unwrap();
        assert!(!cache.exists(&tenant, "key").await.unwrap());
    }
}
