//! Integration tests for armature-cache

use armature_cache::config::CacheBackend;
use armature_cache::*;
use std::time::Duration;

#[tokio::test]
async fn test_cache_config_creation() {
    let config = CacheConfig::redis("redis://localhost:6379").unwrap();
    assert_eq!(config.backend, CacheBackend::Redis);
    assert_eq!(config.url, "redis://localhost:6379");
}

#[tokio::test]
async fn test_cache_config_with_options() {
    let config = CacheConfig::redis("redis://localhost:6379")
        .unwrap()
        .with_key_prefix("myapp")
        .with_default_ttl(Duration::from_secs(7200));

    assert_eq!(config.key_prefix, Some("myapp".to_string()));
    assert_eq!(config.default_ttl, Some(Duration::from_secs(7200)));
}

#[cfg(feature = "memcached")]
#[tokio::test]
async fn test_cache_config_memcached() {
    let config = CacheConfig::memcached("localhost:11211").unwrap();
    assert_eq!(config.backend, CacheBackend::Memcached);
    assert_eq!(config.url, "localhost:11211");
}

#[test]
fn test_cache_backend_debug() {
    // CacheBackend doesn't implement Display, just Debug
    assert_eq!(format!("{:?}", CacheBackend::Redis), "Redis");
    #[cfg(feature = "memcached")]
    assert_eq!(format!("{:?}", CacheBackend::Memcached), "Memcached");
}

#[test]
fn test_cache_error_display() {
    let err = CacheError::Connection("Failed to connect".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Failed to connect"));
}

// Note: These tests would require Redis/Memcached running
// They are disabled by default but can be run with: cargo test -- --ignored

#[cfg(feature = "redis")]
#[tokio::test]
#[ignore]
async fn test_redis_cache_set_get() {
    let config = CacheConfig::redis("redis://localhost:6379").unwrap();
    let cache = RedisCache::new(config).await.unwrap();

    // Set value
    cache
        .set_json("test_key", "test_value".to_string(), None)
        .await
        .unwrap();

    // Get value
    let value: Option<String> = cache.get_json("test_key").await.unwrap();
    assert_eq!(value, Some("test_value".to_string()));

    // Delete value
    cache.delete("test_key").await.unwrap();
}

#[cfg(feature = "redis")]
#[tokio::test]
#[ignore]
async fn test_redis_cache_with_ttl() {
    let config = CacheConfig::redis("redis://localhost:6379").unwrap();
    let cache = RedisCache::new(config).await.unwrap();

    // Set value with 1 second TTL
    cache
        .set_json(
            "ttl_key",
            "ttl_value".to_string(),
            Some(Duration::from_secs(1)),
        )
        .await
        .unwrap();

    // Should exist immediately
    let value: Option<String> = cache.get_json("ttl_key").await.unwrap();
    assert_eq!(value, Some("ttl_value".to_string()));

    // Wait for expiration
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Should be expired
    let value: Option<String> = cache.get_json("ttl_key").await.unwrap();
    assert_eq!(value, None);
}

#[cfg(feature = "memcached")]
#[tokio::test]
#[ignore]
async fn test_memcached_cache_set_get() {
    let config = CacheConfig::memcached("localhost:11211").unwrap();
    let cache = MemcachedCache::new(config).await.unwrap();

    // Set value
    cache
        .set_json("test_key", "test_value".to_string(), None)
        .await
        .unwrap();

    // Get value
    let value: Option<String> = cache.get_json("test_key").await.unwrap();
    assert_eq!(value, Some("test_value".to_string()));

    // Delete value
    cache.delete("test_key").await.unwrap();
}

#[cfg(feature = "redis")]
#[tokio::test]
#[ignore]
async fn test_cache_manager() {
    let config = CacheConfig::redis("redis://localhost:6379")
        .unwrap()
        .with_key_prefix("test");
    let cache = RedisCache::new(config).await.unwrap();
    let manager = CacheManager::new(cache);

    // Set and get with namespace
    manager.set("user:123", &"John Doe", None).await.unwrap();
    let value: Option<String> = manager.get("user:123").await.unwrap();
    assert_eq!(value, Some("John Doe".to_string()));
}
