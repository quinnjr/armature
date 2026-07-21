//! Redis-container-backed regression tests for the armature-ratelimit Redis store.
//!
//! These exercise the real Redis path for the Workflow-8 fixes: the sub-second
//! fixed-window divide-by-zero, the configured `key_prefix`, and per-algorithm
//! `remaining`. Every test self-skips when Docker is unavailable, so the default
//! `cargo test` never requires a container.
#![cfg(feature = "redis")]

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
