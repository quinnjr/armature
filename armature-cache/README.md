# armature-cache

Cache management for the Armature framework.

## Features

- **Multiple backends** — Redis (`RedisCache`), in-memory (`InMemoryCache`), and
  optional Memcached (`MemcachedCache`, behind the `memcached` feature)
- **TTL support** — per-entry expiration, plus a configurable default TTL
- **Async API** — non-blocking cache operations via the `CacheStore` trait
- **JSON values** — the store works with JSON string payloads (`get_json` /
  `set_json`)
- **Batch primitives** — `get_many` / `set_many` / `delete_many`, backed by
  native `MGET` / `MSET` / `DEL` on Redis
- **Multi-tier caching** — `TieredCache` (L1 in-memory + L2 distributed) with
  live hit/miss/promotion stats
- **Tag-based invalidation** — `TaggedCache` invalidates groups of keys by tag

## Installation

```toml
[dependencies]
armature-cache = "0.1"
```

The `redis` backend is enabled by default. Enable Memcached explicitly:

```toml
[dependencies]
armature-cache = { version = "0.1", features = ["memcached"] }
```

## Quick Start (Redis)

```rust,no_run
use armature_cache::{CacheConfig, CacheStore, RedisCache};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), armature_cache::CacheError> {
    // Build a config, then connect.
    let config = CacheConfig::redis("redis://localhost:6379")?
        .with_key_prefix("myapp")
        .with_default_ttl(Duration::from_secs(300))
        .with_connection_timeout(Duration::from_secs(5))
        .with_operation_timeout(Duration::from_secs(3))
        .with_max_connections(10);
    let cache = RedisCache::new(config).await?;

    // Set a JSON value with an explicit TTL.
    cache
        .set_json("key", "\"value\"".to_string(), Some(Duration::from_secs(60)))
        .await?;

    // Get it back.
    let value: Option<String> = cache.get_json("key").await?;
    assert_eq!(value.as_deref(), Some("\"value\""));

    // Delete it.
    cache.delete("key").await?;

    Ok(())
}
```

`connection_timeout`, `operation_timeout`, and `max_connections` are applied to
the Redis connection manager. An operation that exceeds `operation_timeout`
fails with `CacheError::Timeout`.

## In-Memory Cache

`InMemoryCache` implements the same `CacheStore` trait and is handy for tests or
as the L1 tier. It is bounded (default `DEFAULT_MAX_ENTRIES`) and evicts expired
entries lazily on read and eagerly when making room:

```rust,no_run
use armature_cache::{CacheStore, InMemoryCache};

# async fn example() -> Result<(), armature_cache::CacheError> {
let cache = InMemoryCache::new();
// or bound it explicitly: InMemoryCache::with_capacity(1_000)

cache.set_json("k", "\"v\"".to_string(), None).await?;
let v = cache.get_json("k").await?;
assert_eq!(v.as_deref(), Some("\"v\""));
# Ok(())
# }
```

## Multi-Tier Caching

```rust,no_run
use armature_cache::{InMemoryCache, TieredCache};
use std::sync::Arc;

# async fn example() -> Result<(), armature_cache::CacheError> {
let l1 = Arc::new(InMemoryCache::new());
let l2 = Arc::new(InMemoryCache::new()); // typically a RedisCache
let tiered = TieredCache::new(l1, l2);

tiered.set("key", "\"value\"".to_string(), None).await?;
let value = tiered.get("key").await?; // L1, falling back to L2 (and promoting)

// Live counters, not just config echoes.
let stats = tiered.stats().await;
let _ = (stats.l1_hits, stats.l2_hits, stats.misses, stats.promotions);
# Ok(())
# }
```

## Tag-Based Invalidation

```rust,no_run
use armature_cache::{InMemoryCache, TaggedCache};
use std::sync::Arc;

# async fn example() -> Result<(), armature_cache::CacheError> {
let tagged = TaggedCache::new(Arc::new(InMemoryCache::new()));

tagged
    .set_with_tags("user:123", "\"Alice\"".to_string(), &["users", "active"], None)
    .await?;

// Invalidate every key carrying any of these tags in a single batch delete.
tagged.invalidate_tags(&["users", "sessions"]).await?;
# Ok(())
# }
```

The tag index is persisted in the backing `CacheStore` itself (not a local,
per-process map), so it is visible across every instance sharing that store.
Only backends that override `CacheStore::set_add`/`set_remove`/`set_members`
with a native set type (`RedisCache`, via `SADD`/`SREM`/`SMEMBERS`) update it
atomically; other backends fall back to a non-atomic read-modify-write, which
can race under concurrent tagging of the same tag from different instances.

## Memcached (requires the `memcached` feature)

```rust,ignore
use armature_cache::{CacheConfig, CacheStore, MemcachedCache};

let config = CacheConfig::memcached("memcache://localhost:11211")?;
let cache = MemcachedCache::new(config).await?;
cache.set_json("k", "\"v\"".to_string(), None).await?;
```

Note: the memcached protocol exposes no way to read an item's remaining TTL, so
`MemcachedCache::ttl` always returns `Ok(None)`.

## License

MIT OR Apache-2.0
