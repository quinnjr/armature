//! Redis cache implementation.

use crate::config::CacheConfig;
use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use armature_log::{debug, trace};
use async_trait::async_trait;
use redis::{AsyncCommands, Client, aio::ConnectionManager};
use std::time::Duration;

/// Redis cache store.
#[derive(Clone)]
pub struct RedisCache {
    connection: ConnectionManager,
    config: CacheConfig,
}

impl RedisCache {
    /// Create a new Redis cache instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Cache configuration
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use armature_cache::*;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), CacheError> {
    ///     let config = CacheConfig::redis("redis://localhost:6379")?;
    ///     let cache = RedisCache::new(config).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new(config: CacheConfig) -> CacheResult<Self> {
        debug!("Connecting to Redis cache: {}", config.url);
        let client =
            Client::open(config.url.as_str()).map_err(|e| CacheError::Connection(e.to_string()))?;

        let connection = ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::Connection(e.to_string()))?;

        debug!("Redis cache connection established");
        Ok(Self { connection, config })
    }

    /// Get the underlying connection manager.
    pub fn connection(&self) -> &ConnectionManager {
        &self.connection
    }

    /// Build the full key with prefix.
    fn build_key(&self, key: &str) -> String {
        self.config.build_key(key)
    }
}

#[async_trait]
impl CacheStore for RedisCache {
    async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
        let key = self.build_key(key);
        trace!("Cache GET: {}", key);
        let mut conn = self.connection.clone();

        let value: Option<String> = conn.get(&key).await?;
        trace!(
            "Cache {} for: {}",
            if value.is_some() { "HIT" } else { "MISS" },
            key
        );
        Ok(value)
    }

    async fn set_json(&self, key: &str, value: String, ttl: Option<Duration>) -> CacheResult<()> {
        let key = self.build_key(key);
        trace!("Cache SET: {} (ttl: {:?})", key, ttl);
        let mut conn = self.connection.clone();

        let ttl = ttl.or(self.config.default_ttl);

        if let Some(ttl) = ttl {
            let ttl_seconds = ttl.as_secs();
            let _: () = conn.set_ex(&key, value, ttl_seconds).await?;
        } else {
            let _: () = conn.set(&key, value).await?;
        }

        Ok(())
    }

    async fn delete(&self, key: &str) -> CacheResult<()> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let _: () = conn.del(&key).await?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> CacheResult<bool> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let exists: bool = conn.exists(&key).await?;
        Ok(exists)
    }

    async fn clear(&self) -> CacheResult<()> {
        let mut conn = self.connection.clone();
        let _: () = redis::cmd("FLUSHDB").query_async(&mut conn).await?;
        Ok(())
    }

    async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();

        let ttl_seconds: i64 = conn.ttl(&key).await?;

        match ttl_seconds {
            -2 => Ok(None), // Key doesn't exist
            -1 => Ok(None), // Key has no expiration
            seconds if seconds > 0 => Ok(Some(Duration::from_secs(seconds as u64))),
            _ => Ok(None),
        }
    }

    async fn expire(&self, key: &str, ttl: Duration) -> CacheResult<()> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let ttl_seconds = ttl.as_secs();
        let _: () = conn.expire(&key, ttl_seconds as i64).await?;
        Ok(())
    }

    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let new_value: i64 = conn.incr(&key, delta).await?;
        Ok(new_value)
    }

    async fn decrement(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let new_value: i64 = conn.decr(&key, delta).await?;
        Ok(new_value)
    }

    /// Native multi-get: a single `MGET` round-trip instead of N `GET`s.
    ///
    /// `MGET` preserves argument order, so the returned vector matches `keys`
    /// element-for-element, with `None` for missing keys.
    async fn mget(&self, keys: &[&str]) -> CacheResult<Vec<Option<String>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let full_keys: Vec<String> = keys.iter().map(|k| self.build_key(k)).collect();
        trace!("Cache MGET: {} keys", full_keys.len());
        let mut conn = self.connection.clone();
        let values: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&full_keys)
            .query_async(&mut conn)
            .await?;
        Ok(values)
    }

    /// Native multi-set in a single round-trip.
    ///
    /// Without a TTL this is a plain `MSET`. `MSET` cannot express per-key
    /// expiry, so when a TTL applies we pipeline `SET ... EX` commands (still
    /// one round-trip), preserving the exact per-key TTL semantics of
    /// `set_json` (including the `default_ttl` fallback).
    async fn mset(&self, items: &[(&str, String)], ttl: Option<Duration>) -> CacheResult<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection.clone();
        let ttl = ttl.or(self.config.default_ttl);

        if let Some(ttl) = ttl {
            let ttl_seconds = ttl.as_secs();
            trace!("Cache MSET (pipelined SET EX): {} items", items.len());
            let mut pipe = redis::pipe();
            for (key, value) in items {
                pipe.cmd("SET")
                    .arg(self.build_key(key))
                    .arg(value)
                    .arg("EX")
                    .arg(ttl_seconds)
                    .ignore();
            }
            let _: () = pipe.query_async(&mut conn).await?;
        } else {
            trace!("Cache MSET: {} items", items.len());
            let mut cmd = redis::cmd("MSET");
            for (key, value) in items {
                cmd.arg(self.build_key(key)).arg(value);
            }
            let _: () = cmd.query_async(&mut conn).await?;
        }
        Ok(())
    }

    /// Native multi-delete: a single variadic `DEL` round-trip instead of N.
    async fn mdel(&self, keys: &[&str]) -> CacheResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let full_keys: Vec<String> = keys.iter().map(|k| self.build_key(k)).collect();
        trace!("Cache DEL: {} keys", full_keys.len());
        let mut conn = self.connection.clone();
        let _: () = redis::cmd("DEL")
            .arg(&full_keys)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_key() {
        let config = CacheConfig::redis("redis://localhost:6379")
            .unwrap()
            .with_key_prefix("test");

        // Note: Can't easily test async without a real Redis instance
        // This is just to verify the struct can be created
        assert_eq!(config.build_key("key"), "test:key");
    }
}
