//! Memcached-container-backed regression test for `MemcachedCache::increment`.
//!
//! Spins a throwaway `memcached` container (testcontainers-modules has no
//! memcached module, so we drive a `GenericImage` directly) and verifies the
//! atomic / create-at-zero increment semantics. Self-skips when Docker is
//! unavailable, and only compiles under the `memcached` feature.

#![cfg(feature = "memcached")]

use armature_cache::{CacheConfig, CacheStore, MemcachedCache};
use std::time::Duration;
use testcontainers::GenericImage;
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;

/// Connect with a short retry loop: `memcached` prints no readiness banner, so
/// we poll the port until the client connects (or give up after ~10s).
async fn connect(url: &str) -> MemcachedCache {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match MemcachedCache::new(CacheConfig::memcached(url).unwrap()).await {
            Ok(cache) => return cache,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("could not connect to memcached at {url}: {e}"),
        }
    }
}

/// `increment`/`decrement` must return the server's authoritative post-op
/// counter directly, and create a missing counter at zero (memcached's native
/// create-at-zero semantics) instead of erroring.
#[tokio::test]
async fn memcached_increment_create_at_zero_and_atomic() {
    armature_testkit::skip_if_no_docker!();

    let image = GenericImage::new("memcached", "1.6-alpine").with_exposed_port(11211.tcp());
    let container = image.start().await.expect("start memcached container");
    let port = container
        .get_host_port_ipv4(11211.tcp())
        .await
        .expect("memcached mapped port");
    let url = format!("memcache://127.0.0.1:{port}");

    let cache = connect(&url).await;
    cache.clear().await.unwrap();

    // Absent key: created at zero (delta not applied on creation) -> 0.
    assert_eq!(cache.increment("counter", 5).await.unwrap(), 0);
    // Existing key: atomic +3 -> authoritative 3 (from the incr itself).
    assert_eq!(cache.increment("counter", 3).await.unwrap(), 3);
    // Decrement an existing key -> 1.
    assert_eq!(cache.decrement("counter", 2).await.unwrap(), 1);
    // Decrement of an absent key: created at zero, clamps at zero.
    assert_eq!(cache.decrement("fresh", 5).await.unwrap(), 0);

    // The stored value is consistent with the returned counter.
    assert_eq!(
        cache.get_json("counter").await.unwrap().as_deref(),
        Some("1")
    );
}

/// Regression for the `delta.abs()` fabrication: when the post-increment
/// counter exceeds `i64::MAX`, the old code's re-read + `parse::<i64>()` failed
/// and silently returned `delta.abs()`. The fix returns the true server value
/// (as a lossless `u64 -> i64` bit-cast), which for a > i64::MAX counter is
/// negative — and never `delta.abs()`.
#[tokio::test]
async fn memcached_increment_never_fabricates_delta_abs() {
    armature_testkit::skip_if_no_docker!();

    let image = GenericImage::new("memcached", "1.6-alpine").with_exposed_port(11211.tcp());
    let container = image.start().await.expect("start memcached container");
    let port = container
        .get_host_port_ipv4(11211.tcp())
        .await
        .expect("memcached mapped port");
    let url = format!("memcache://127.0.0.1:{port}");

    let cache = connect(&url).await;
    cache.clear().await.unwrap();

    // Seed a counter just below u64::MAX, then increment by 1.
    let base: u64 = u64::MAX - 10; // well above i64::MAX
    cache.set_json("big", base.to_string(), None).await.unwrap();

    let returned = cache.increment("big", 1).await.unwrap();
    let expected = (base + 1) as i64; // lossless bit-cast; negative here

    assert_eq!(
        returned, expected,
        "increment must return the true server counter, not delta.abs()"
    );
    // The old code would have returned `delta.abs()` == 1 here.
    assert_ne!(returned, 1, "must not fabricate delta.abs()");
}
