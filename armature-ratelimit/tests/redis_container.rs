//! Redis-container-backed regression tests for the armature-ratelimit Redis store.
//!
//! These exercise the real Redis path for the Workflow-8 fixes: the sub-second
//! fixed-window divide-by-zero, the configured `key_prefix`, and per-algorithm
//! `remaining`. Every test self-skips when Docker is unavailable, so the default
//! `cargo test` never requires a container.
#![cfg(feature = "redis")]

use armature_ratelimit::error::RateLimitError;
use armature_ratelimit::stores::{RateLimitStore, RedisStore};
use armature_ratelimit::{Algorithm, RateLimiter};
use armature_testkit::containers::RedisContainer;
use std::time::Duration;

/// Fetch all Redis keys matching a glob pattern (test-only helper).
async fn keys(url: &str, pattern: &str) -> Vec<String> {
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("KEYS")
        .arg(pattern)
        .query_async(&mut conn)
        .await
        .unwrap()
}

/// Regression: a sub-second fixed window used to panic with an integer
/// divide-by-zero (`now / window.as_secs()` where `as_secs() == 0`). The Redis
/// store must now return a decision without panicking.
#[tokio::test]
async fn redis_fixed_window_sub_second_does_not_panic() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let store = RedisStore::new(&redis.url()).await.unwrap();
    store.reset("k").await.unwrap();

    let window = Duration::from_millis(500);
    let (allowed1, _) = store.fixed_window_check("k", 2, window).await.unwrap();
    let (allowed2, _) = store.fixed_window_check("k", 2, window).await.unwrap();
    let (allowed3, _) = store.fixed_window_check("k", 2, window).await.unwrap();

    assert!(allowed1);
    assert!(allowed2);
    assert!(
        !allowed3,
        "third request in a 2/500ms window must be denied"
    );
}

/// The configured `key_prefix` must be applied to the Redis keys instead of the
/// hardcoded "ratelimit" default.
#[tokio::test]
async fn redis_key_prefix_is_applied() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let url = redis.url();

    let limiter = RateLimiter::builder()
        .algorithm(Algorithm::FixedWindow {
            max_requests: 5,
            window: Duration::from_secs(60),
        })
        .redis_store(&url)
        .key_prefix("myapp")
        .build()
        .await
        .unwrap();

    limiter.check("client-1").await.unwrap();

    let prefixed = keys(&url, "myapp:*").await;
    assert!(
        !prefixed.is_empty(),
        "expected keys under the configured prefix, found none"
    );
    let default_prefixed = keys(&url, "ratelimit:*").await;
    assert!(
        default_prefixed.is_empty(),
        "no keys should use the hardcoded default prefix, found {default_prefixed:?}"
    );
}

/// `remaining` must decrease with usage for all three algorithms on the Redis
/// store (previously sliding/fixed returned 0).
#[tokio::test]
async fn redis_remaining_per_algorithm() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let store = RedisStore::new(&redis.url()).await.unwrap();

    // Token bucket.
    store.reset("tb").await.unwrap();
    store.token_bucket_check("tb", 5, 1.0).await.unwrap();
    assert_eq!(store.remaining("tb").await.unwrap(), 4);

    // Fixed window.
    store.reset("fw").await.unwrap();
    let w = Duration::from_secs(60);
    store.fixed_window_check("fw", 5, w).await.unwrap();
    assert_eq!(store.remaining("fw").await.unwrap(), 4);
    store.fixed_window_check("fw", 5, w).await.unwrap();
    assert_eq!(store.remaining("fw").await.unwrap(), 3);

    // Sliding window.
    store.reset("sw").await.unwrap();
    store.sliding_window_check("sw", 5, w).await.unwrap();
    assert_eq!(store.remaining("sw").await.unwrap(), 4);
    store.sliding_window_check("sw", 5, w).await.unwrap();
    assert_eq!(store.remaining("sw").await.unwrap(), 3);
}

/// Security regression: a same-millisecond burst must never admit more than
/// `max_requests`. Firing many requests concurrently collapses them into a
/// handful of distinct wall-clock milliseconds. Against the pre-fix
/// `ZADD key now now`, all requests sharing a millisecond overwrite one ZSET
/// member (set-semantic on the member), so `ZCARD` never exceeds the number of
/// distinct milliseconds and the limiter admits far beyond `max_requests` — a
/// distributed rate-limit bypass. With the unique-member fix each admitted
/// request adds its own member and the atomic Lua gate admits exactly
/// `max_requests`.
#[tokio::test]
async fn redis_sliding_window_same_ms_burst_does_not_bypass_limit() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;
    let store = RedisStore::new(&redis.url()).await.unwrap();
    store.reset("burst").await.unwrap();

    let max: u64 = 5;
    let window = Duration::from_secs(60);

    // Fire far more than the limit concurrently so many land in the same ms.
    let futures = (0..200).map(|_| store.sliding_window_check("burst", max, window));
    let results = futures::future::join_all(futures).await;
    let allowed = results.iter().filter(|r| r.as_ref().unwrap().0).count();

    assert_eq!(
        allowed, max as usize,
        "sliding window admitted {allowed} requests within the window, expected exactly {max}"
    );
}

/// The configured `operation_timeout` must bound every op. We stall the
/// (single-threaded) server with `DEBUG SLEEP` on a side connection so a store
/// op issued against it cannot complete in time and fails with
/// `RateLimitError::Timeout`.
///
/// Against the pre-fix code (bare `ConnectionManager`, no timeout wrapping) this
/// same call simply blocked until the server woke up and then returned `Ok(..)`.
#[tokio::test]
async fn redis_operation_timeout_is_enforced() {
    armature_testkit::skip_if_no_docker!();
    let redis = RedisContainer::start().await;

    let store = RedisStore::new(&redis.url())
        .await
        .unwrap()
        .with_operation_timeout(Duration::from_millis(200));

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
    // Let the server enter the sleep before issuing the store op.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let err = store
        .token_bucket_check("k", 10, 1.0)
        .await
        .expect_err("op against a stalled server must time out, not block");
    assert!(
        matches!(err, RateLimitError::Timeout),
        "expected RateLimitError::Timeout, got: {err:?}"
    );
}
