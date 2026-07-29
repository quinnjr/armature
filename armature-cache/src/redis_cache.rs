//! Redis cache implementation.

use crate::config::CacheConfig;
use crate::error::{CacheError, CacheResult};
use crate::traits::CacheStore;
use armature_log::{debug, trace};
use async_trait::async_trait;
use redis::{AsyncCommands, Client, aio::ConnectionManager, aio::ConnectionManagerConfig};
use std::future::Future;
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

        // Apply the connection tuning from `CacheConfig`:
        //
        // * `connection_timeout` bounds each attempt to (re)establish the TCP
        //   connection to the server.
        // * `max_connections` caps the number of commands the multiplexed
        //   manager keeps in flight concurrently (the manager multiplexes over
        //   a single socket, so this is the pool-equivalent back-pressure knob).
        //
        // The per-operation timeout is intentionally *not* wired into the
        // manager's `response_timeout`: doing so would surface as an opaque
        // `redis::RedisError`. We instead enforce `operation_timeout` ourselves
        // (see `with_op_timeout`) so a slow op fails as `CacheError::Timeout`.
        let mut manager_config =
            ConnectionManagerConfig::new().set_connection_timeout(Some(config.connection_timeout));
        if config.max_connections > 0 {
            manager_config = manager_config.set_concurrency_limit(config.max_connections);
        }

        let connection = ConnectionManager::new_with_config(client, manager_config)
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

    /// Run a Redis future under the configured `operation_timeout`.
    ///
    /// When the operation does not complete within `operation_timeout` the
    /// future is dropped and the call resolves to [`CacheError::Timeout`]
    /// rather than blocking indefinitely (or waiting out a much longer default
    /// socket timeout). This is what makes `CacheError::Timeout` reachable.
    async fn with_op_timeout<F, T>(&self, fut: F) -> CacheResult<T>
    where
        F: Future<Output = redis::RedisResult<T>>,
    {
        match tokio::time::timeout(self.config.operation_timeout, fut).await {
            Ok(result) => Ok(result?),
            Err(_) => Err(CacheError::Timeout),
        }
    }

    /// `SCAN` for every key matching `pattern` and remove them all via
    /// batched `UNLINK` calls. Used by [`CacheStore::clear`] to scope
    /// clearing to `key_prefix` instead of `FLUSHDB`-ing the whole database.
    ///
    /// Keys are collected from the `SCAN` cursor first, then removed in
    /// bounded-size `UNLINK` batches so a very large matching set doesn't
    /// build one huge variadic command.
    async fn scan_and_unlink(
        conn: &mut ConnectionManager,
        pattern: String,
    ) -> redis::RedisResult<()> {
        use futures::StreamExt;

        /// Bound on how many keys go into a single `UNLINK` call.
        const UNLINK_BATCH_SIZE: usize = 500;

        let mut matched: Vec<String> = Vec::new();
        {
            let mut iter: redis::AsyncIter<'_, String> = conn.scan_match(pattern.as_str()).await?;
            while let Some(key) = iter.next().await {
                matched.push(key?);
            }
        }

        for chunk in matched.chunks(UNLINK_BATCH_SIZE) {
            let _: () = redis::cmd("UNLINK").arg(chunk).query_async(conn).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl CacheStore for RedisCache {
    async fn get_json(&self, key: &str) -> CacheResult<Option<String>> {
        let key = self.build_key(key);
        trace!("Cache GET: {}", key);
        let mut conn = self.connection.clone();

        let value: Option<String> = self.with_op_timeout(conn.get(&key)).await?;
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
            let _: () = self
                .with_op_timeout(conn.set_ex(&key, value, ttl_seconds))
                .await?;
        } else {
            let _: () = self.with_op_timeout(conn.set(&key, value)).await?;
        }

        Ok(())
    }

    async fn delete(&self, key: &str) -> CacheResult<()> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let _: () = self.with_op_timeout(conn.del(&key)).await?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> CacheResult<bool> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let exists: bool = self.with_op_timeout(conn.exists(&key)).await?;
        Ok(exists)
    }

    /// Clear this cache's keys.
    ///
    /// When `key_prefix` is configured, this is **scoped** to that prefix: it
    /// `SCAN`s for every key matching `{key_prefix}:*` and removes them with
    /// batched `UNLINK` calls, so it only wipes keys this cache actually
    /// wrote — not the whole Redis database/instance. `SCAN` is cursor-based
    /// and non-blocking (unlike `KEYS`, which is O(N) and stalls the
    /// single-threaded server for the entire keyspace); `UNLINK` reclaims
    /// memory off the main thread instead of blocking on `DEL`.
    ///
    /// When no `key_prefix` is configured, this cache has no distinct slice
    /// of the keyspace to scope to, so it falls back to the previous
    /// unscoped `FLUSHDB` behavior — this remains destructive to the entire
    /// Redis database/instance, so an unprefixed `RedisCache` sharing a
    /// Redis instance with other services/tenants should not call `clear()`.
    async fn clear(&self) -> CacheResult<()> {
        match self.config.key_prefix.as_deref() {
            Some(prefix) if !prefix.is_empty() => {
                let pattern = format!("{prefix}:*");
                let mut conn = self.connection.clone();
                self.with_op_timeout(Self::scan_and_unlink(&mut conn, pattern))
                    .await?;
            }
            _ => {
                let mut conn = self.connection.clone();
                let _: () = self
                    .with_op_timeout(redis::cmd("FLUSHDB").query_async(&mut conn))
                    .await?;
            }
        }
        Ok(())
    }

    async fn ttl(&self, key: &str) -> CacheResult<Option<Duration>> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();

        let ttl_seconds: i64 = self.with_op_timeout(conn.ttl(&key)).await?;

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
        let _: () = self
            .with_op_timeout(conn.expire(&key, ttl_seconds as i64))
            .await?;
        Ok(())
    }

    async fn increment(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let new_value: i64 = self.with_op_timeout(conn.incr(&key, delta)).await?;
        Ok(new_value)
    }

    async fn decrement(&self, key: &str, delta: i64) -> CacheResult<i64> {
        let key = self.build_key(key);
        let mut conn = self.connection.clone();
        let new_value: i64 = self.with_op_timeout(conn.decr(&key, delta)).await?;
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
        let values: Vec<Option<String>> = self
            .with_op_timeout(redis::cmd("MGET").arg(&full_keys).query_async(&mut conn))
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
            let _: () = self.with_op_timeout(pipe.query_async(&mut conn)).await?;
        } else {
            trace!("Cache MSET: {} items", items.len());
            let mut cmd = redis::cmd("MSET");
            for (key, value) in items {
                cmd.arg(self.build_key(key)).arg(value);
            }
            let _: () = self.with_op_timeout(cmd.query_async(&mut conn)).await?;
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
        let _: () = self
            .with_op_timeout(redis::cmd("DEL").arg(&full_keys).query_async(&mut conn))
            .await?;
        Ok(())
    }

    /// `RedisCache` backs `set_add`/`set_remove`/`set_members` with native
    /// `SADD`/`SREM`/`SMEMBERS`, which are atomic — see [`CacheStore::supports_atomic_sets`].
    fn supports_atomic_sets(&self) -> bool {
        true
    }

    /// Native `SADD`: atomically adds `member` to a Redis Set, unlike the
    /// trait default's non-atomic get/modify/set. This is what makes
    /// [`crate::invalidation::TaggedCache`]'s tag index safe to update
    /// concurrently from multiple instances sharing this backend.
    async fn set_add(&self, set_key: &str, member: &str) -> CacheResult<()> {
        let set_key = self.build_key(set_key);
        let mut conn = self.connection.clone();
        let _: () = self.with_op_timeout(conn.sadd(&set_key, member)).await?;
        Ok(())
    }

    /// Native `SREM`: atomically removes `member` from a Redis Set.
    async fn set_remove(&self, set_key: &str, member: &str) -> CacheResult<()> {
        let set_key = self.build_key(set_key);
        let mut conn = self.connection.clone();
        let _: () = self.with_op_timeout(conn.srem(&set_key, member)).await?;
        Ok(())
    }

    /// Native `SMEMBERS`.
    async fn set_members(&self, set_key: &str) -> CacheResult<Vec<String>> {
        let set_key = self.build_key(set_key);
        let mut conn = self.connection.clone();
        let members: Vec<String> = self.with_op_timeout(conn.smembers(&set_key)).await?;
        Ok(members)
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
