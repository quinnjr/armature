//! Redis rate limit store
//!
//! Uses Redis for distributed rate limiting across multiple instances.
//! Requires the `redis` feature to be enabled.

use crate::error::{RateLimitError, RateLimitResult};
use crate::stores::RateLimitStore;
use async_trait::async_trait;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use std::time::Duration;
use tracing::{debug, trace};

/// Redis-backed rate limit store
///
/// Supports distributed rate limiting across multiple application instances.
/// Uses Lua scripts for atomic operations.
pub struct RedisStore {
    /// Redis connection manager
    conn: ConnectionManager,
    /// Key prefix
    prefix: String,
}

impl RedisStore {
    /// Create a new Redis store
    ///
    /// # Arguments
    ///
    /// * `url` - Redis connection URL (e.g., "redis://localhost:6379")
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub async fn new(url: &str) -> RateLimitResult<Self> {
        debug!(url = %url, "Connecting to Redis for rate limiting");

        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;

        Ok(Self {
            conn,
            prefix: "ratelimit".to_string(),
        })
    }

    /// Create a new Redis store with a custom prefix
    pub async fn with_prefix(url: &str, prefix: impl Into<String>) -> RateLimitResult<Self> {
        let mut store = Self::new(url).await?;
        store.prefix = prefix.into();
        Ok(store)
    }

    /// Get the full key with prefix
    fn key(&self, suffix: &str) -> String {
        format!("{}:{}", self.prefix, suffix)
    }
}

#[async_trait]
impl RateLimitStore for RedisStore {
    async fn token_bucket_check(
        &self,
        key: &str,
        capacity: u64,
        refill_rate: f64,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, capacity = capacity, refill_rate = refill_rate, "Redis token bucket check");

        let full_key = self.key(&format!("tb:{}", key));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        // Lua script for atomic token bucket operation
        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local capacity = tonumber(ARGV[1])
            local refill_rate = tonumber(ARGV[2])
            local now = tonumber(ARGV[3])
            local ttl = math.ceil(capacity / refill_rate) + 10

            local data = redis.call('HMGET', key, 'tokens', 'last_refill')
            local tokens = tonumber(data[1]) or capacity
            local last_refill = tonumber(data[2]) or now

            -- Refill tokens
            local elapsed = now - last_refill
            tokens = math.min(capacity, tokens + elapsed * refill_rate)

            -- Try to consume a token
            if tokens >= 1 then
                tokens = tokens - 1
                redis.call('HMSET', key, 'tokens', tokens, 'last_refill', now)
                redis.call('EXPIRE', key, ttl)
                return {1, math.floor(tokens)}
            else
                redis.call('HMSET', key, 'tokens', tokens, 'last_refill', now)
                redis.call('EXPIRE', key, ttl)
                return {0, 0}
            end
            "#,
        );

        let mut conn = self.conn.clone();
        let result: (i32, i64) = script
            .key(&full_key)
            .arg(capacity)
            .arg(refill_rate)
            .arg(now)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| RateLimitError::store(e.to_string()))?;

        let allowed = result.0 == 1;
        let remaining = result.1 as u64;

        if allowed {
            trace!(key = %key, remaining = remaining, "Redis token bucket: allowed");
        } else {
            trace!(key = %key, "Redis token bucket: denied");
        }

        Ok((allowed, remaining))
    }

    async fn sliding_window_check(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, max_requests = max_requests, window = ?window, "Redis sliding window check");

        let full_key = self.key(&format!("sw:{}", key));
        let window_ms = window.as_millis() as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let cutoff = now - window_ms;

        // Lua script for atomic sliding window operation
        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local max_requests = tonumber(ARGV[1])
            local now = tonumber(ARGV[2])
            local cutoff = tonumber(ARGV[3])
            local window_secs = tonumber(ARGV[4])

            -- Remove old entries
            redis.call('ZREMRANGEBYSCORE', key, 0, cutoff)

            -- Count current entries
            local count = redis.call('ZCARD', key)

            if count < max_requests then
                -- Add new entry
                redis.call('ZADD', key, now, now)
                redis.call('EXPIRE', key, window_secs + 10)
                return {1, max_requests - count - 1}
            else
                return {0, 0}
            end
            "#,
        );

        let mut conn = self.conn.clone();
        let result: (i32, i64) = script
            .key(&full_key)
            .arg(max_requests)
            .arg(now)
            .arg(cutoff)
            .arg(window.as_secs())
            .invoke_async(&mut conn)
            .await
            .map_err(|e| RateLimitError::store(e.to_string()))?;

        let allowed = result.0 == 1;
        let remaining = result.1 as u64;

        if allowed {
            trace!(key = %key, remaining = remaining, "Redis sliding window: allowed");
        } else {
            trace!(key = %key, "Redis sliding window: denied");
        }

        Ok((allowed, remaining))
    }

    async fn fixed_window_check(
        &self,
        key: &str,
        max_requests: u64,
        window: Duration,
    ) -> RateLimitResult<(bool, u64)> {
        trace!(key = %key, max_requests = max_requests, window = ?window, "Redis fixed window check");

        let window_secs = window.as_secs();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let window_id = now / window_secs;
        let full_key = self.key(&format!("fw:{}:{}", key, window_id));

        let mut conn = self.conn.clone();

        // Increment and get current count
        let count: i64 = conn
            .incr(&full_key, 1)
            .await
            .map_err(|e| RateLimitError::store(e.to_string()))?;

        // Set expiry on first request
        if count == 1 {
            let _: () = conn
                .expire(&full_key, window_secs as i64 + 10)
                .await
                .map_err(|e| RateLimitError::store(e.to_string()))?;
        }

        let count = count as u64;

        if count <= max_requests {
            let remaining = max_requests - count;
            trace!(key = %key, remaining = remaining, "Redis fixed window: allowed");
            Ok((true, remaining))
        } else {
            trace!(key = %key, "Redis fixed window: denied");
            Ok((false, 0))
        }
    }

    async fn reset(&self, key: &str) -> RateLimitResult<()> {
        debug!(key = %key, "Resetting rate limit state in Redis");

        let mut conn = self.conn.clone();

        // Delete all keys for this rate limit key
        let patterns = [
            self.key(&format!("tb:{}", key)),
            self.key(&format!("sw:{}", key)),
            self.key(&format!("fw:{}:*", key)),
        ];

        for pattern in &patterns {
            if pattern.contains('*') {
                // Cursored SCAN instead of the blocking `KEYS <pattern>`.
                // KEYS scans the entire keyspace in one shot and stalls the
                // single-threaded Redis server on large datasets; SCAN walks
                // the keyspace incrementally in bounded chunks. Matches are
                // removed in batches via a single variadic UNLINK per batch
                // (UNLINK reclaims memory in a background thread, unlike the
                // synchronous DEL — and one UNLINK per batch replaces the old
                // N+1 per-key DELs).
                let mut cursor: u64 = 0;
                loop {
                    let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg(pattern)
                        .arg("COUNT")
                        .arg(100)
                        .query_async(&mut conn)
                        .await
                        .map_err(|e| RateLimitError::store(e.to_string()))?;

                    if !keys.is_empty() {
                        let _: () = redis::cmd("UNLINK")
                            .arg(&keys)
                            .query_async(&mut conn)
                            .await
                            .map_err(|e| RateLimitError::store(e.to_string()))?;
                    }

                    // A returned cursor of 0 signals the iteration is complete.
                    if next_cursor == 0 {
                        break;
                    }
                    cursor = next_cursor;
                }
            } else {
                let _: () = conn
                    .del(pattern)
                    .await
                    .map_err(|e| RateLimitError::store(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn remaining(&self, key: &str) -> RateLimitResult<u64> {
        let mut conn = self.conn.clone();

        // Try token bucket first
        let tb_key = self.key(&format!("tb:{}", key));
        let tokens: Option<f64> = conn
            .hget(&tb_key, "tokens")
            .await
            .map_err(|e| RateLimitError::store(e.to_string()))?;

        if let Some(t) = tokens {
            return Ok(t as u64);
        }

        // Try sliding window
        let sw_key = self.key(&format!("sw:{}", key));
        let count: i64 = conn
            .zcard(&sw_key)
            .await
            .map_err(|e| RateLimitError::store(e.to_string()))?;

        if count > 0 {
            // Can't determine max without knowing the config
            return Ok(0);
        }

        Ok(0)
    }

    async fn cleanup(&self) -> RateLimitResult<()> {
        debug!("Redis cleanup is automatic via TTL");
        Ok(())
    }

    fn store_type(&self) -> &'static str {
        "redis"
    }
}

impl std::fmt::Debug for RedisStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisStore")
            .field("prefix", &self.prefix)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    // Redis tests require a running Redis instance
    // Run with: cargo test --features redis -- --ignored

    use super::*;

    #[tokio::test]
    #[ignore = "Requires running Redis instance"]
    async fn test_redis_token_bucket() {
        let store = RedisStore::new("redis://localhost:6379").await.unwrap();

        // Reset first
        store.reset("test").await.unwrap();

        // First 5 requests should be allowed
        for i in (0..5).rev() {
            let (allowed, remaining) = store.token_bucket_check("test", 5, 1.0).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 6th should be denied
        let (allowed, _) = store.token_bucket_check("test", 5, 1.0).await.unwrap();
        assert!(!allowed);
    }

    #[tokio::test]
    #[ignore = "Requires running Redis instance"]
    async fn test_redis_sliding_window() {
        let store = RedisStore::new("redis://localhost:6379").await.unwrap();

        // Reset first
        store.reset("test").await.unwrap();

        let window = Duration::from_secs(60);

        // First 3 requests should be allowed
        for i in (0..3).rev() {
            let (allowed, remaining) = store.sliding_window_check("test", 3, window).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 4th should be denied
        let (allowed, _) = store.sliding_window_check("test", 3, window).await.unwrap();
        assert!(!allowed);
    }

    #[tokio::test]
    #[ignore = "Requires running Redis instance"]
    async fn test_redis_fixed_window() {
        let store = RedisStore::new("redis://localhost:6379").await.unwrap();

        // Reset first
        store.reset("test").await.unwrap();

        let window = Duration::from_secs(60);

        // First 3 requests should be allowed
        for i in (0..3).rev() {
            let (allowed, remaining) = store.fixed_window_check("test", 3, window).await.unwrap();
            assert!(allowed);
            assert_eq!(remaining, i);
        }

        // 4th should be denied
        let (allowed, _) = store.fixed_window_check("test", 3, window).await.unwrap();
        assert!(!allowed);
    }
}
