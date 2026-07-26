//! Redis-container-backed regression tests for armature-cache.
//!
//! These exercise the real Redis path against a throwaway container: that the
//! configured `operation_timeout` is actually enforced (surfacing as
//! `CacheError::Timeout`), and that normal operations still work once the
//! connection tuning from `CacheConfig` is applied. Every test self-skips when
//! Docker is unavailable, so the default `cargo test` never requires Docker.

#![cfg(feature = "redis")]

use armature_cache::{CacheConfig, CacheError, CacheStore, RedisCache};
use armature_testkit::containers::RedisContainer;
use std::time::Duration;

/// The configured `operation_timeout` must bound every operation. We stall the
/// (single-threaded) server with `DEBUG SLEEP` on a side connection, so a cache
/// op issued against it cannot complete within the short `operation_timeout`
/// and fails with `CacheError::Timeout`.
///
/// Against the pre-fix code (no timeout wrapping) this same call simply blocked
/// until the server woke up and then returned `Ok(..)` — the timeout was never
/// applied and `CacheError::Timeout` was unreachable.
#[tokio::test]
async fn redis_operation_timeout_is_enforced() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = CacheConfig::redis(redis.url())
        .unwrap()
        .with_connection_timeout(Duration::from_secs(5))
        .with_operation_timeout(Duration::from_millis(200));
    let cache = RedisCache::new(config).await.expect("connect to redis");

    // Stall the server for ~2s on a separate connection.
    let client = redis::Client::open(redis.url()).unwrap();
    let mut side = client
        .get_multiplexed_async_connection()
        .await
        .expect("side connection");
    tokio::spawn(async move {
        let _: redis::RedisResult<()> = redis::cmd("DEBUG")
            .arg("SLEEP")
            .arg(2.0_f64)
            .query_async(&mut side)
            .await;
    });
    // Let the server enter the sleep before we issue the cache op.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let err = cache
        .get_json("some-key")
        .await
        .expect_err("op against a stalled server must time out, not block");
    assert!(
        matches!(err, CacheError::Timeout),
        "expected CacheError::Timeout, got: {err:?}"
    );
}

/// With realistic timeouts and the connection tuning applied, ordinary
/// operations round-trip correctly.
#[tokio::test]
async fn redis_normal_operations_work_with_tuning() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = CacheConfig::redis(redis.url())
        .unwrap()
        .with_key_prefix("armature-cache-test")
        .with_connection_timeout(Duration::from_secs(5))
        .with_operation_timeout(Duration::from_secs(3))
        .with_max_connections(4);
    let cache = RedisCache::new(config).await.expect("connect to redis");

    cache
        .set_json("k", "\"value\"".to_string(), Some(Duration::from_secs(60)))
        .await
        .unwrap();
    assert_eq!(
        cache.get_json("k").await.unwrap(),
        Some("\"value\"".to_string())
    );

    // Native increment path.
    assert_eq!(cache.increment("counter", 5).await.unwrap(), 5);
    assert_eq!(cache.increment("counter", 3).await.unwrap(), 8);

    cache.delete("k").await.unwrap();
    assert_eq!(cache.get_json("k").await.unwrap(), None);
}
