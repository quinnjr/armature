//! Redis service for dependency injection.

use redis::AsyncCommands;
use redis::aio::MultiplexedConnection;
use std::sync::OnceLock;
use std::time::Duration;

use crate::{
    RedisConfig, RedisError, Result,
    pool::{RedisConnection, RedisPool, RedisPoolBuilder},
    pubsub::PubSub,
};

/// Redis service providing connection pool and convenience methods.
///
/// This is the main entry point for Redis operations and is designed
/// to be registered in the DI container.
pub struct RedisService {
    config: RedisConfig,
    pool: RedisPool,
    /// Lazily-built, reused `redis::Client` for `get_dedicated`.
    ///
    /// The client parses the connection URL and holds connection config; it is
    /// built once and reused so `get_dedicated` no longer calls
    /// `redis::Client::open(...)` on every invocation.
    dedicated_client: OnceLock<redis::Client>,
}

impl RedisService {
    /// Create a new Redis service.
    pub async fn new(config: RedisConfig) -> Result<Self> {
        let pool = RedisPoolBuilder::new(config.clone()).build().await?;
        Ok(Self {
            config,
            pool,
            dedicated_client: OnceLock::new(),
        })
    }

    /// Create from an existing pool.
    pub fn from_pool(config: RedisConfig, pool: RedisPool) -> Self {
        Self {
            config,
            pool,
            dedicated_client: OnceLock::new(),
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &RedisConfig {
        &self.config
    }

    /// Get the connection pool.
    pub fn pool(&self) -> &RedisPool {
        &self.pool
    }

    /// Get a connection from the pool.
    pub async fn get(&self) -> Result<RedisConnection<'_>> {
        self.pool.get().await
    }

    /// Bound a single command's execution by `RedisConfig::command_timeout`.
    ///
    /// Every convenience method below runs its Redis round-trip through this
    /// wrapper so `command_timeout` (documented on `RedisConfig` but
    /// previously never read anywhere) actually bounds command latency,
    /// instead of only pool checkout being bounded by `connection_timeout`.
    async fn with_timeout<T>(
        &self,
        fut: impl std::future::Future<Output = std::result::Result<T, redis::RedisError>>,
    ) -> Result<T> {
        match tokio::time::timeout(self.config.command_timeout, fut).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Err(RedisError::Timeout),
        }
    }

    /// Get or lazily build the reusable dedicated `redis::Client`.
    fn dedicated_client(&self) -> Result<&redis::Client> {
        if let Some(client) = self.dedicated_client.get() {
            return Ok(client);
        }
        let client = redis::Client::open(self.config.connection_url())
            .map_err(|e| RedisError::Connection(e.to_string()))?;
        // If another thread initialized it first, keep the existing one.
        let _ = self.dedicated_client.set(client);
        Ok(self
            .dedicated_client
            .get()
            .expect("dedicated_client was just set"))
    }

    /// Get a dedicated connection (not from pool).
    ///
    /// The underlying `redis::Client` is built once and cached, so repeated
    /// calls no longer re-parse the URL and rebuild the client. Each call still
    /// opens a fresh multiplexed connection (that is the point of a "dedicated"
    /// connection outside the pool).
    pub async fn get_dedicated(&self) -> Result<MultiplexedConnection> {
        let client = self.dedicated_client()?;
        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| RedisError::Connection(e.to_string()))
    }

    /// Create a Pub/Sub client.
    pub fn pubsub(&self) -> Result<PubSub> {
        PubSub::new(self.config.clone())
    }

    /// Check if the connection is healthy.
    pub async fn health_check(&self) -> Result<()> {
        let mut conn = self.get().await?;
        let _: String = self
            .with_timeout(redis::cmd("PING").query_async(&mut conn))
            .await?;
        Ok(())
    }

    /// Get pool statistics.
    pub fn pool_stats(&self) -> PoolStats {
        let state = self.pool.state();
        PoolStats {
            connections: state.connections,
            idle_connections: state.idle_connections,
        }
    }

    // Convenience methods for common operations

    /// Get a value.
    pub async fn get_value<T: redis::FromRedisValue>(&self, key: &str) -> Result<Option<T>> {
        let mut conn = self.get().await?;
        let value: Option<T> = self.with_timeout(conn.get(key)).await?;
        Ok(value)
    }

    /// Set a value.
    pub async fn set_value<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        value: T,
    ) -> Result<()> {
        let mut conn = self.get().await?;
        // Use redis::cmd to avoid ToSingleRedisArg requirement
        let _: () = self
            .with_timeout(redis::cmd("SET").arg(key).arg(value).query_async(&mut conn))
            .await?;
        Ok(())
    }

    /// Set a value with expiration.
    pub async fn set_ex<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        value: T,
        ttl: Duration,
    ) -> Result<()> {
        let mut conn = self.get().await?;
        // Use set with EX option instead of set_ex which requires ToSingleRedisArg
        let _: () = self
            .with_timeout(
                redis::cmd("SET")
                    .arg(key)
                    .arg(value)
                    .arg("EX")
                    .arg(ttl.as_secs())
                    .query_async(&mut conn),
            )
            .await?;
        Ok(())
    }

    /// Delete a key.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        let mut conn = self.get().await?;
        let deleted: u32 = self.with_timeout(conn.del(key)).await?;
        Ok(deleted > 0)
    }

    /// Check if a key exists.
    pub async fn exists(&self, key: &str) -> Result<bool> {
        let mut conn = self.get().await?;
        let exists: bool = self.with_timeout(conn.exists(key)).await?;
        Ok(exists)
    }

    /// Set expiration on a key.
    pub async fn expire(&self, key: &str, ttl: Duration) -> Result<bool> {
        let mut conn = self.get().await?;
        let result: bool = self
            .with_timeout(conn.expire(key, ttl.as_secs() as i64))
            .await?;
        Ok(result)
    }

    /// Get TTL of a key.
    pub async fn ttl(&self, key: &str) -> Result<Option<Duration>> {
        let mut conn = self.get().await?;
        let ttl: i64 = self.with_timeout(conn.ttl(key)).await?;
        if ttl < 0 {
            Ok(None)
        } else {
            Ok(Some(Duration::from_secs(ttl as u64)))
        }
    }

    /// Increment a counter.
    pub async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let mut conn = self.get().await?;
        let value: i64 = self.with_timeout(conn.incr(key, delta)).await?;
        Ok(value)
    }

    /// Hash get.
    pub async fn hget<T: redis::FromRedisValue>(
        &self,
        key: &str,
        field: &str,
    ) -> Result<Option<T>> {
        let mut conn = self.get().await?;
        let value: Option<T> = self.with_timeout(conn.hget(key, field)).await?;
        Ok(value)
    }

    /// Hash set.
    pub async fn hset<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        field: &str,
        value: T,
    ) -> Result<()> {
        let mut conn = self.get().await?;
        // Use redis::cmd to avoid ToSingleRedisArg requirement
        let _: () = self
            .with_timeout(
                redis::cmd("HSET")
                    .arg(key)
                    .arg(field)
                    .arg(value)
                    .query_async(&mut conn),
            )
            .await?;
        Ok(())
    }

    /// Hash get all.
    pub async fn hgetall<T: redis::FromRedisValue>(&self, key: &str) -> Result<T> {
        let mut conn = self.get().await?;
        let value: T = self.with_timeout(conn.hgetall(key)).await?;
        Ok(value)
    }

    /// List push (left).
    pub async fn lpush<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        values: &[T],
    ) -> Result<u64> {
        let mut conn = self.get().await?;
        let len: u64 = self.with_timeout(conn.lpush(key, values)).await?;
        Ok(len)
    }

    /// List pop (right).
    pub async fn rpop<T: redis::FromRedisValue>(&self, key: &str) -> Result<Option<T>> {
        let mut conn = self.get().await?;
        let value: Option<T> = self.with_timeout(conn.rpop(key, None)).await?;
        Ok(value)
    }

    /// Blocking list pop.
    ///
    /// Deliberately **not** wrapped in `with_timeout`/`command_timeout`: the
    /// caller-supplied `timeout` already bounds how long this blocks, and it
    /// may legitimately be longer than `command_timeout` (which defaults to
    /// 30s and is meant to bound ordinary, non-blocking round-trips).
    /// Wrapping this in `command_timeout` too would silently truncate a
    /// caller's longer block wait.
    pub async fn brpop<T: redis::FromRedisValue>(
        &self,
        key: &str,
        timeout: Duration,
    ) -> Result<Option<(String, T)>> {
        let mut conn = self.get().await?;
        let value: Option<(String, T)> = conn.brpop(key, timeout.as_secs() as f64).await?;
        Ok(value)
    }

    /// Set add.
    pub async fn sadd<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        members: &[T],
    ) -> Result<u64> {
        let mut conn = self.get().await?;
        let added: u64 = self.with_timeout(conn.sadd(key, members)).await?;
        Ok(added)
    }

    /// Set members.
    pub async fn smembers<T: redis::FromRedisValue>(&self, key: &str) -> Result<Vec<T>> {
        let mut conn = self.get().await?;
        let members: Vec<T> = self.with_timeout(conn.smembers(key)).await?;
        Ok(members)
    }

    /// Set is member.
    pub async fn sismember<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        key: &str,
        member: T,
    ) -> Result<bool> {
        let mut conn = self.get().await?;
        // Use redis::cmd to avoid ToSingleRedisArg requirement
        let is_member: bool = self
            .with_timeout(
                redis::cmd("SISMEMBER")
                    .arg(key)
                    .arg(member)
                    .query_async(&mut conn),
            )
            .await?;
        Ok(is_member)
    }

    /// Execute a Lua script.
    pub async fn eval<T: redis::FromRedisValue>(
        &self,
        script: &str,
        keys: &[&str],
        args: &[&str],
    ) -> Result<T> {
        let mut conn = self.get().await?;
        let script = redis::Script::new(script);
        let result: T = self
            .with_timeout(script.key(keys).arg(args).invoke_async(&mut conn))
            .await?;
        Ok(result)
    }

    // Multi-key convenience helpers (single round-trip instead of a loop).

    /// Get multiple values in a single `MGET` round-trip.
    ///
    /// Returns values in the **same order** as `keys`, with `None` for missing
    /// keys. Prefer this over looping [`get_value`](Self::get_value).
    pub async fn mget<T: redis::FromRedisValue>(&self, keys: &[&str]) -> Result<Vec<Option<T>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.get().await?;
        let values: Vec<Option<T>> = self
            .with_timeout(redis::cmd("MGET").arg(keys).query_async(&mut conn))
            .await?;
        Ok(values)
    }

    /// Set multiple key/value pairs in a single `MSET` round-trip.
    ///
    /// Note: `MSET` has no TTL support; use a [`pipeline`](Self::pipeline) of
    /// `SET ... EX` when per-key expiry is required.
    pub async fn mset<T: redis::ToRedisArgs + Send + Sync>(
        &self,
        items: &[(&str, T)],
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut conn = self.get().await?;
        let mut cmd = redis::cmd("MSET");
        for (key, value) in items {
            cmd.arg(*key).arg(value);
        }
        let _: () = self.with_timeout(cmd.query_async(&mut conn)).await?;
        Ok(())
    }

    /// Delete multiple keys in a single variadic `DEL` round-trip.
    ///
    /// Returns the number of keys actually removed.
    pub async fn del_many(&self, keys: &[&str]) -> Result<u64> {
        if keys.is_empty() {
            return Ok(0);
        }
        let mut conn = self.get().await?;
        let deleted: u64 = self
            .with_timeout(redis::cmd("DEL").arg(keys).query_async(&mut conn))
            .await?;
        Ok(deleted)
    }

    /// Execute a pipeline against a pooled connection in one round-trip.
    ///
    /// The `build` closure populates the pipeline; the result is decoded into
    /// `T`. This lets multi-key callers batch arbitrary commands without
    /// looping single-key operations.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use armature_redis::{RedisService, Result};
    /// # async fn example(redis: &RedisService) -> Result<()> {
    /// let (a, b): (Option<String>, Option<String>) = redis
    ///     .pipeline(|pipe| {
    ///         pipe.cmd("GET").arg("key:a");
    ///         pipe.cmd("GET").arg("key:b");
    ///     })
    ///     .await?;
    /// # let _ = (a, b);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn pipeline<T, F>(&self, build: F) -> Result<T>
    where
        T: redis::FromRedisValue,
        F: FnOnce(&mut redis::Pipeline),
    {
        let mut conn = self.get().await?;
        let mut pipe = redis::pipe();
        build(&mut pipe);
        let result: T = self.with_timeout(pipe.query_async(&mut conn)).await?;
        Ok(result)
    }
}

/// Connection pool statistics.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Total connections.
    pub connections: u32,
    /// Idle connections.
    pub idle_connections: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit-level proof that `with_timeout` actually enforces
    /// `command_timeout`, without needing a live server: a future that never
    /// resolves within the configured window must come back as
    /// `RedisError::Timeout`, and one that resolves in time must pass its
    /// value through unchanged.
    #[tokio::test]
    async fn with_timeout_enforces_command_timeout() {
        let config = RedisConfig::builder()
            .command_timeout(Duration::from_millis(20))
            .build();
        let redis = RedisService::from_pool(
            config,
            crate::pool::RedisPool::Single(bb8::Pool::builder().build_unchecked(
                crate::pool::NamedRedisConnectionManager::new(
                    bb8_redis::RedisConnectionManager::new("redis://127.0.0.1:1").unwrap(),
                    None,
                ),
            )),
        );

        // A future that resolves immediately must pass through untouched.
        let fast: std::result::Result<i32, redis::RedisError> = Ok(42);
        let result = redis.with_timeout(async { fast }).await;
        assert_eq!(result.unwrap(), 42);

        // A future that outlives `command_timeout` must time out.
        let slow = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok::<i32, redis::RedisError>(1)
        };
        let result = redis.with_timeout(slow).await;
        assert!(
            matches!(result, Err(RedisError::Timeout)),
            "expected Timeout, got {result:?}"
        );
    }

    /// End-to-end proof (requires Docker) that `command_timeout` bounds real
    /// command execution: a fast command against a live server succeeds, and
    /// a Lua script that intentionally blocks the server past
    /// `command_timeout` (via `DEBUG SLEEP`) comes back as a timeout error
    /// rather than hanging or succeeding after the configured window.
    #[tokio::test]
    async fn command_timeout_bounds_live_command_execution() {
        armature_testkit::skip_if_no_docker!();
        let container = armature_testkit::containers::RedisContainer::start().await;
        let config = RedisConfig::builder()
            .url(container.url())
            .command_timeout(Duration::from_millis(200))
            .build();
        let redis = RedisService::new(config).await.unwrap();

        // Fast command comfortably within the timeout succeeds.
        redis.set_value("wf2:timeout_test", "v").await.unwrap();
        let value: Option<String> = redis.get_value("wf2:timeout_test").await.unwrap();
        assert_eq!(value, Some("v".to_string()));

        // `DEBUG SLEEP` blocks the server for 1s; routed through the same
        // `with_timeout` wrapper every convenience method uses, it must time
        // out against a 200ms command_timeout instead of hanging or
        // eventually succeeding. (Note: `DEBUG SLEEP` can't be called from a
        // Lua script via `eval` — Redis disallows it there — so we exercise
        // `with_timeout` directly here, exactly as `eval`/`get_value`/etc do
        // internally.)
        let mut conn = redis.get().await.unwrap();
        let result: Result<()> = redis
            .with_timeout(
                redis::cmd("DEBUG")
                    .arg("SLEEP")
                    .arg(1)
                    .query_async(&mut conn),
            )
            .await;
        assert!(
            matches!(result, Err(RedisError::Timeout)),
            "expected Timeout, got {result:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_basic_operations() {
        let config = RedisConfig::builder().url("redis://localhost:6379").build();

        let redis = RedisService::new(config).await.unwrap();

        // Test set/get
        redis.set_value("test_key", "test_value").await.unwrap();
        let value: Option<String> = redis.get_value("test_key").await.unwrap();
        assert_eq!(value, Some("test_value".to_string()));

        // Clean up
        redis.delete("test_key").await.unwrap();
    }
}
