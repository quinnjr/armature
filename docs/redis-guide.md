# Redis Integration Guide

Armature provides a unified Redis integration through `armature-redis` with connection pooling, pub/sub, and dependency injection.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Basic Usage](#basic-usage)
- [Connection Pooling](#connection-pooling)
- [Pub/Sub](#pubsub)
- [Dependency Injection](#dependency-injection)
- [Using with Other Crates](#using-with-other-crates)
- [Configuration](#configuration)
- [Best Practices](#best-practices)
- [Summary](#summary)

## Overview

The `armature-redis` crate provides a centralized Redis client that other Armature crates depend on. This ensures:

- **Single Connection Pool**: All crates share one pool
- **DI Integration**: Redis is injected from the DI container
- **Consistent Configuration**: One place to configure Redis
- **Connection Reuse**: Efficient resource utilization

## Features

- ✅ Connection pooling with bb8
- ✅ Pub/Sub messaging
- ✅ Cluster support
- ✅ TLS encryption
- ✅ Sentinel support
- ✅ DI container integration
- ✅ Convenience methods for common operations

## Installation

```toml
[dependencies]
armature-redis = "0.1"
```

With optional features:

```toml
[dependencies]
armature-redis = { version = "0.1", features = ["cluster", "tls"] }
```

## Basic Usage

```rust
use armature_redis::{RedisService, RedisConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure Redis
    let config = RedisConfig::builder()
        .url("redis://localhost:6379")
        .pool_size(10)
        .build();

    // Create service
    let redis = RedisService::new(config).await?;

    // Use convenience methods
    redis.set_value("key", "value").await?;
    let value: Option<String> = redis.get_value("key").await?;

    // Or get a connection for raw commands
    let mut conn = redis.get().await?;
    redis::cmd("HSET")
        .arg("hash")
        .arg("field")
        .arg("value")
        .query_async(&mut *conn)
        .await?;

    Ok(())
}
```

## Connection Pooling

The service manages a connection pool automatically:

```rust
use armature_redis::{RedisService, RedisConfig};

let config = RedisConfig::builder()
    .url("redis://localhost:6379")
    .pool_size(20)        // Max connections
    .min_idle(Some(5))    // Keep at least 5 idle
    .connection_timeout(Duration::from_secs(5))
    .build();

let redis = RedisService::new(config).await?;

// Get pool statistics
let stats = redis.pool_stats();
println!("Connections: {}, Idle: {}", stats.connections, stats.idle_connections);
```

## Pub/Sub

Redis pub/sub for real-time messaging:

```rust
use armature_redis::{RedisService, RedisConfig};

let redis = RedisService::new(config).await?;
let pubsub = redis.pubsub()?;

// Subscribe to a channel
let mut subscription = pubsub.subscribe("events").await?;

// Spawn task to receive messages
tokio::spawn(async move {
    while let Some(message) = subscription.recv().await {
        println!("Received: {} on {}", message.payload, message.channel);
    }
});

// Publish messages
pubsub.publish("events", "Hello, World!").await?;
```

Pattern subscriptions:

```rust
// Subscribe to all channels matching "user:*"
let mut subscription = pubsub.psubscribe("user:*").await?;
```

## Dependency Injection

Register Redis as a singleton in your application:

```rust
use armature_framework::prelude::*;
use armature_redis::{RedisService, RedisConfig};
use std::sync::Arc;

#[module]
struct RedisModule;

#[module_impl]
impl RedisModule {
    #[provider(singleton)]
    async fn redis_service() -> Arc<RedisService> {
        let config = RedisConfig::from_env().build();
        Arc::new(RedisService::new(config).await.unwrap())
    }
}

// Use in controllers
#[controller("/data")]
struct DataController;

#[controller_impl]
impl DataController {
    #[get("/:key")]
    async fn get_data(
        &self,
        #[inject] redis: Arc<RedisService>,
        key: Path<String>,
    ) -> Result<Json<Value>, HttpError> {
        let value: Option<String> = redis.get_value(&key).await?;
        Ok(Json(json!({ "value": value })))
    }
}
```

## Using with Other Crates

Other Armature crates automatically use `armature-redis` when the `redis` feature is enabled:

### armature-cache

```rust
use armature_cache::RedisCache;
use armature_redis::RedisService;
use std::sync::Arc;

#[module_impl]
impl CacheModule {
    #[provider(singleton)]
    fn redis_cache(redis: Arc<RedisService>) -> RedisCache {
        RedisCache::from_service(redis)
    }
}
```

### armature-queue

```rust
use armature_queue::Queue;
use armature_redis::RedisService;

#[module_impl]
impl QueueModule {
    #[provider(singleton)]
    fn job_queue(redis: Arc<RedisService>) -> Queue {
        Queue::from_service(redis, "jobs")
    }
}
```

### armature-ratelimit

```rust
use armature_ratelimit::RedisRateLimiter;
use armature_redis::RedisService;

#[module_impl]
impl RateLimitModule {
    #[provider(singleton)]
    fn rate_limiter(redis: Arc<RedisService>) -> RedisRateLimiter {
        RedisRateLimiter::from_service(redis)
    }
}
```

### armature-distributed

```rust
use armature_distributed::{DistributedLock, LeaderElection};
use armature_redis::RedisService;

#[module_impl]
impl DistributedModule {
    #[provider(singleton)]
    fn distributed_lock(redis: Arc<RedisService>) -> DistributedLock {
        DistributedLock::from_service(redis)
    }

    #[provider(singleton)]
    fn leader_election(redis: Arc<RedisService>) -> LeaderElection {
        LeaderElection::from_service(redis, "my-service")
    }
}
```

### armature-session

```rust
use armature_session::RedisSessionStore;
use armature_redis::RedisService;

#[module_impl]
impl SessionModule {
    #[provider(singleton)]
    fn session_store(redis: Arc<RedisService>) -> RedisSessionStore {
        RedisSessionStore::from_service(redis)
    }
}
```

## Configuration

### Environment Variables

```bash
# Required
REDIS_URL=redis://localhost:6379

# Optional
REDIS_POOL_SIZE=10
REDIS_DATABASE=0
REDIS_USERNAME=default
REDIS_PASSWORD=secret
REDIS_TLS=true
REDIS_CLUSTER=true
REDIS_CLUSTER_NODES=node1:6379,node2:6379,node3:6379
```

### Programmatic Configuration

```rust
let config = RedisConfig::builder()
    .url("redis://localhost:6379")
    .pool_size(20)
    .min_idle(5)
    .database(1)
    .username("user")
    .password("secret")
    .connection_timeout(Duration::from_secs(5))
    .command_timeout(Duration::from_secs(30))
    .tls(true)
    .connection_name("my-app")
    .build();
```

### Cluster Mode

```rust
let config = RedisConfig::builder()
    .cluster_nodes(vec![
        "redis://node1:6379".to_string(),
        "redis://node2:6379".to_string(),
        "redis://node3:6379".to_string(),
    ])
    .build();
```

## Best Practices

### 1. Register as Singleton

Always register `RedisService` as a singleton to share the connection pool:

```rust
#[provider(singleton)]  // Important!
async fn redis_service() -> Arc<RedisService> {
    // ...
}
```

### 2. Use Environment Configuration

```rust
// Prefer environment-based config for flexibility
let config = RedisConfig::from_env().build();
```

### 3. Use Convenience Methods

The service provides convenience methods for common operations:

```rust
// Instead of raw commands
redis.set_value("key", "value").await?;
redis.get_value::<String>("key").await?;
redis.delete("key").await?;
redis.incr("counter", 1).await?;
redis.hset("hash", "field", "value").await?;
```

### 4. Handle Connection Errors

```rust
match redis.get_value::<String>("key").await {
    Ok(Some(value)) => println!("Found: {}", value),
    Ok(None) => println!("Key not found"),
    Err(e) if e.is_retryable() => {
        // Retry logic
    }
    Err(e) => return Err(e.into()),
}
```

### 5. Use Pub/Sub for Real-Time Events

```rust
// Publisher
redis.pubsub()?.publish("events", &event_json).await?;

// Subscriber
let mut sub = redis.pubsub()?.subscribe("events").await?;
while let Some(msg) = sub.recv().await {
    handle_event(&msg.payload);
}
```

## Summary

**Key Concepts:**

1. **Centralized Redis**: `armature-redis` is the single source for Redis connections
2. **Connection Pooling**: Efficient connection management with bb8
3. **DI Integration**: Register once, inject everywhere
4. **Crate Integration**: All Redis-dependent crates use `armature-redis`
5. **Pub/Sub**: Built-in messaging support

**Crates Using armature-redis:**

| Crate | Purpose |
|-------|---------|
| `armature-cache` | Redis caching backend |
| `armature-queue` | Job queue storage |
| `armature-distributed` | Locks, leader election |
| `armature-ratelimit` | Rate limiting storage |
| `armature-session` | Session storage |

**Configuration Quick Reference:**

```bash
REDIS_URL=redis://localhost:6379
REDIS_POOL_SIZE=10
REDIS_PASSWORD=secret
```

