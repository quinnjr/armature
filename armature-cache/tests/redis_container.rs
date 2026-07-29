//! Redis-container-backed regression tests for armature-cache.
//!
//! These exercise the real Redis path against a throwaway container: that the
//! configured `operation_timeout` is actually enforced (surfacing as
//! `CacheError::Timeout`), and that normal operations still work once the
//! connection tuning from `CacheConfig` is applied. Every test self-skips when
//! Docker is unavailable, so the default `cargo test` never requires Docker.

#![cfg(feature = "redis")]

use armature_cache::{CacheConfig, CacheError, CacheStore, RedisCache, TaggedCache};
use armature_testkit::containers::RedisContainer;
use std::sync::Arc;
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

/// `RedisCache`'s `set_add`/`set_remove`/`set_members` overrides must use the
/// native `SADD`/`SREM`/`SMEMBERS` commands (not the trait's default,
/// non-atomic get/modify/set-json fallback).
#[tokio::test]
async fn redis_native_set_primitives_round_trip() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = CacheConfig::redis(redis.url()).unwrap();
    let cache = RedisCache::new(config).await.expect("connect to redis");

    cache.set_add("myset", "a").await.unwrap();
    cache.set_add("myset", "b").await.unwrap();
    cache.set_add("myset", "a").await.unwrap(); // duplicate: no-op

    let mut members = cache.set_members("myset").await.unwrap();
    members.sort();
    assert_eq!(members, vec!["a".to_string(), "b".to_string()]);

    cache.set_remove("myset", "a").await.unwrap();
    assert_eq!(
        cache.set_members("myset").await.unwrap(),
        vec!["b".to_string()]
    );

    // Verify it really is a Redis Set (not a JSON blob under `get_json`) by
    // issuing SMEMBERS directly against the raw connection.
    let mut conn = redis::Client::open(redis.url())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let raw: Vec<String> = redis::AsyncCommands::smembers(&mut conn, "myset")
        .await
        .unwrap();
    assert_eq!(raw, vec!["b".to_string()]);
}

/// Regression for Finding 2 against a REAL distributed backend: two
/// independent `TaggedCache` instances, each wrapping its OWN `RedisCache`
/// connection to the SAME Redis server (simulating two app processes), must
/// share tag visibility. The old implementation kept tag bookkeeping in a
/// local, per-process `HashMap`, so this would have failed — instance B would
/// have seen no keys for a tag only instance A ever wrote.
#[tokio::test]
async fn redis_tagged_cache_tag_index_visible_across_instances() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = || CacheConfig::redis(redis.url()).unwrap();
    let cache_a = Arc::new(RedisCache::new(config()).await.expect("connect a"));
    let cache_b = Arc::new(RedisCache::new(config()).await.expect("connect b"));

    let instance_a = TaggedCache::new(cache_a);
    let instance_b = TaggedCache::new(cache_b);

    instance_a
        .set_with_tags(
            "user:1",
            "\"Alice\"".to_string(),
            &["users"],
            Some(Duration::from_secs(60)),
        )
        .await
        .unwrap();

    // Instance B, on its own Redis connection, must see the tag membership
    // instance A persisted to the shared backend.
    let keys = instance_b.get_keys_by_tag("users").await.unwrap();
    assert_eq!(keys, vec!["user:1".to_string()]);

    // ...and be able to invalidate it.
    instance_b.invalidate_tag("users").await.unwrap();

    assert_eq!(instance_a.get("user:1").await.unwrap(), None);
    assert!(instance_a.list_tags().await.unwrap().is_empty());
}

/// Regression for Finding 1 (HIGH): `RedisCache::clear()` with a `key_prefix`
/// configured must be SCOPED to that prefix — it must not `FLUSHDB` the
/// entire shared Redis instance. We write keys under two different prefixes
/// (simulating two services/tenants sharing one Redis) and confirm that
/// clearing one cache leaves the other's keys untouched.
///
/// Against the pre-fix code (`clear()` == unconditional `FLUSHDB`) this test
/// would have failed: `other_cache`'s key would have been wiped too.
#[tokio::test]
async fn redis_clear_with_key_prefix_is_scoped_not_flushdb() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let mine_config = CacheConfig::redis(redis.url())
        .unwrap()
        .with_key_prefix("service-a");
    let mine = RedisCache::new(mine_config).await.expect("connect mine");

    let other_config = CacheConfig::redis(redis.url())
        .unwrap()
        .with_key_prefix("service-b");
    let other = RedisCache::new(other_config).await.expect("connect other");

    for i in 0..20 {
        mine.set_json(&format!("k{i}"), "\"v\"".to_string(), None)
            .await
            .unwrap();
    }
    other
        .set_json("k0", "\"other-value\"".to_string(), None)
        .await
        .unwrap();

    mine.clear().await.unwrap();

    for i in 0..20 {
        assert_eq!(
            mine.get_json(&format!("k{i}")).await.unwrap(),
            None,
            "service-a's own key {i} should have been cleared"
        );
    }
    assert_eq!(
        other.get_json("k0").await.unwrap(),
        Some("\"other-value\"".to_string()),
        "service-b's key must survive service-a's scoped clear()"
    );
}

/// A `RedisCache` with NO `key_prefix` configured has no distinct slice of
/// the keyspace to scope to, so `clear()` still falls back to the documented
/// unscoped `FLUSHDB` behavior.
#[tokio::test]
async fn redis_clear_without_key_prefix_still_flushes_whole_db() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = CacheConfig::redis(redis.url()).unwrap();
    let cache = RedisCache::new(config).await.expect("connect");

    cache
        .set_json("unscoped-key", "\"v\"".to_string(), None)
        .await
        .unwrap();
    cache.clear().await.unwrap();

    assert_eq!(cache.get_json("unscoped-key").await.unwrap(), None);
}

/// Regression for Finding 3: `RedisCache` must report
/// `supports_atomic_sets() == true`, matching its native `SADD`/`SREM`/
/// `SMEMBERS`-backed `set_add`/`set_remove`/`set_members` overrides.
#[tokio::test]
async fn redis_supports_atomic_sets_is_true() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let config = CacheConfig::redis(redis.url()).unwrap();
    let cache = RedisCache::new(config).await.expect("connect");

    assert!(cache.supports_atomic_sets());
}
