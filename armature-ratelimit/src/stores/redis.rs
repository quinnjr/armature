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
    /// Per-operation timeout. Every store op runs under this bound so a
    /// partitioned/unresponsive Redis returns [`RateLimitError::Timeout`]
    /// instead of stalling the request path forever.
    operation_timeout: Duration,
}

/// Default per-operation timeout when none is configured.
const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(3);

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
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        })
    }

    /// Create a new Redis store with a custom prefix
    pub async fn with_prefix(url: &str, prefix: impl Into<String>) -> RateLimitResult<Self> {
        let mut store = Self::new(url).await?;
        store.prefix = prefix.into();
        Ok(store)
    }

    /// Override the per-operation timeout (default 3s).
    pub fn with_operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }

    /// Get the full key with prefix
    fn key(&self, suffix: &str) -> String {
        format!("{}:{}", self.prefix, suffix)
    }

    /// Run a Redis future under the configured `operation_timeout`.
    ///
    /// When the op does not complete in time the future is dropped and this
    /// resolves to [`RateLimitError::Timeout`] rather than blocking forever
    /// (which would keep the middleware's `skip_on_error` guard from ever
    /// engaging). Redis-level errors are mapped to [`RateLimitError::store`].
    async fn with_op_timeout<F, T>(&self, fut: F) -> RateLimitResult<T>
    where
        F: std::future::Future<Output = redis::RedisResult<T>>,
    {
        match tokio::time::timeout(self.operation_timeout, fut).await {
            Ok(result) => result.map_err(|e| RateLimitError::store(e.to_string())),
            Err(_) => Err(RateLimitError::Timeout),
        }
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
        let result: (i32, i64) = self
            .with_op_timeout(
                script
                    .key(&full_key)
                    .arg(capacity)
                    .arg(refill_rate)
                    .arg(now)
                    .invoke_async(&mut conn),
            )
            .await?;

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

        // Lua script for atomic sliding window operation.
        //
        // The ZSET member MUST be unique per admitted request. The score stays
        // equal to `now` so `ZREMRANGEBYSCORE` still prunes by timestamp, but
        // the member is `now .. '-' .. <monotonic seq>` so two requests landing
        // in the same millisecond add two distinct members instead of ZADD
        // set-semantically overwriting a single member (which made `ZCARD`
        // undercount and admit requests far beyond `max_requests` under a
        // same-millisecond burst — a distributed rate-limit bypass).
        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local maxkey = KEYS[2]
            local seqkey = KEYS[3]
            local max_requests = tonumber(ARGV[1])
            local now = tonumber(ARGV[2])
            local cutoff = tonumber(ARGV[3])
            local window_secs = tonumber(ARGV[4])
            local ttl = window_secs + 10

            -- Record the configured limit so `remaining` can report limit-count.
            redis.call('SET', maxkey, max_requests, 'EX', ttl)

            -- Remove old entries
            redis.call('ZREMRANGEBYSCORE', key, 0, cutoff)

            -- Count current entries
            local count = redis.call('ZCARD', key)

            if count < max_requests then
                -- Unique member so same-millisecond requests don't collapse.
                local seq = redis.call('INCR', seqkey)
                redis.call('EXPIRE', seqkey, ttl)
                redis.call('ZADD', key, now, now .. '-' .. seq)
                redis.call('EXPIRE', key, ttl)
                return {1, max_requests - count - 1}
            else
                return {0, 0}
            end
            "#,
        );

        let swmax_key = self.key(&format!("swmax:{}", key));
        let swseq_key = self.key(&format!("swseq:{}", key));
        let mut conn = self.conn.clone();
        let result: (i32, i64) = self
            .with_op_timeout(
                script
                    .key(&full_key)
                    .key(&swmax_key)
                    .key(&swseq_key)
                    .arg(max_requests)
                    .arg(now)
                    .arg(cutoff)
                    .arg(window.as_secs())
                    .invoke_async(&mut conn),
            )
            .await?;

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

        // Compute the window bucket in milliseconds so sub-second windows
        // (e.g. 500ms) never divide by zero: `window.as_secs()` truncates to 0
        // for any window under a second and `now / 0` panics.
        let window_ms = (window.as_millis() as u64).max(1);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let window_id = now_ms / window_ms;
        let full_key = self.key(&format!("fw:{}:{}", key, window_id));
        // TTL must outlive the window; ceil to whole seconds (min 1s) + slack.
        let ttl_secs = (window_ms.div_ceil(1000).max(1) + 10) as i64;

        let mut conn = self.conn.clone();

        // Single atomic INCR-and-initialize. Doing the INCR, the TTL, and the
        // two companion writes in one Lua script collapses the old 4 round-trips
        // into 1 and removes the crash-between-INCR-and-EXPIRE race that could
        // leave a permanently un-TTL'd (leaked) counter key.
        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local fwmax = KEYS[2]
            local fwcur = KEYS[3]
            local max_requests = ARGV[1]
            local window_id = ARGV[2]
            local ttl = tonumber(ARGV[3])

            local count = redis.call('INCR', key)
            if count == 1 then
                redis.call('EXPIRE', key, ttl)
                redis.call('SET', fwmax, max_requests, 'EX', ttl)
                redis.call('SET', fwcur, window_id, 'EX', ttl)
            end
            return count
            "#,
        );

        let count: i64 = self
            .with_op_timeout(
                script
                    .key(&full_key)
                    .key(self.key(&format!("fwmax:{}", key)))
                    .key(self.key(&format!("fwcur:{}", key)))
                    .arg(max_requests)
                    .arg(window_id)
                    .arg(ttl_secs)
                    .invoke_async(&mut conn),
            )
            .await?;

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

        // All fixed (non-wildcard) keys for this rate-limit key are removed in
        // one variadic UNLINK (a single round-trip) instead of one DEL each.
        let fixed_keys = [
            self.key(&format!("tb:{}", key)),
            self.key(&format!("sw:{}", key)),
            self.key(&format!("swmax:{}", key)),
            self.key(&format!("swseq:{}", key)),
            self.key(&format!("fwmax:{}", key)),
            self.key(&format!("fwcur:{}", key)),
        ];
        let _: () = self
            .with_op_timeout(redis::cmd("UNLINK").arg(&fixed_keys).query_async(&mut conn))
            .await?;

        // The fixed-window bucket keys carry the rolling window id, so they need
        // a pattern sweep. Cursored SCAN instead of the blocking `KEYS`, with a
        // single variadic UNLINK per batch (UNLINK reclaims memory off-thread).
        let pattern = self.key(&format!("fw:{}:*", key));
        let mut cursor: u64 = 0;
        loop {
            let (next_cursor, keys): (u64, Vec<String>) = self
                .with_op_timeout(
                    redis::cmd("SCAN")
                        .arg(cursor)
                        .arg("MATCH")
                        .arg(&pattern)
                        .arg("COUNT")
                        .arg(100)
                        .query_async(&mut conn),
                )
                .await?;

            if !keys.is_empty() {
                let _: () = self
                    .with_op_timeout(redis::cmd("UNLINK").arg(&keys).query_async(&mut conn))
                    .await?;
            }

            // A returned cursor of 0 signals the iteration is complete.
            if next_cursor == 0 {
                break;
            }
            cursor = next_cursor;
        }

        Ok(())
    }

    async fn remaining(&self, key: &str) -> RateLimitResult<u64> {
        let mut conn = self.conn.clone();

        // Token bucket: remaining == floor(tokens). Probed first so the common
        // token-bucket case short-circuits after a single round-trip.
        let tb_key = self.key(&format!("tb:{}", key));
        let tokens: Option<f64> = self.with_op_timeout(conn.hget(&tb_key, "tokens")).await?;

        if let Some(t) = tokens {
            return Ok(t.max(0.0) as u64);
        }

        // The three independent companion pointers (fixed-window bucket id +
        // limit, sliding-window limit) are fetched in one MGET rather than up to
        // three dependent GETs.
        let (fwcur, fwmax, swmax): (Option<u64>, Option<u64>, Option<u64>) = self
            .with_op_timeout(
                redis::cmd("MGET")
                    .arg(self.key(&format!("fwcur:{}", key)))
                    .arg(self.key(&format!("fwmax:{}", key)))
                    .arg(self.key(&format!("swmax:{}", key)))
                    .query_async(&mut conn),
            )
            .await?;

        // Fixed window: locate the live window bucket via the companion pointer
        // and its recorded limit, then report limit - count.
        if let (Some(window_id), Some(max)) = (fwcur, fwmax) {
            let count: Option<u64> = self
                .with_op_timeout(conn.get(self.key(&format!("fw:{}:{}", key, window_id))))
                .await?;
            return Ok(max.saturating_sub(count.unwrap_or(0)));
        }

        // Sliding window: remaining == limit - live entries.
        if let Some(max) = swmax {
            let sw_key = self.key(&format!("sw:{}", key));
            let count: i64 = self.with_op_timeout(conn.zcard(&sw_key)).await?;
            return Ok(max.saturating_sub(count as u64));
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
