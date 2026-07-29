# armature-redis

Redis client integration for the Armature framework.

## Features

- **Connection Pooling** - Efficient connection management via bb8
- **Async Operations** - Non-blocking Redis commands
- **Pub/Sub** - Real-time messaging with channel and pattern subscriptions
- **Cluster Support** - Redis Cluster mode (via `redis`'s `cluster-async`, always enabled)
- **DI Integration** - Register `RedisService` in your application's DI container

## Installation

```toml
[dependencies]
armature-redis = "0.1"
```

## Quick Start

```rust,ignore
use armature_redis::{RedisService, RedisConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure Redis
    let config = RedisConfig::builder()
        .url("redis://localhost:6379")
        .pool_size(10)
        .build();

    // Create service (DI-ready)
    let redis = RedisService::new(config).await?;

    // Convenience methods
    redis.set_value("key", "value").await?;
    let value: Option<String> = redis.get_value("key").await?;

    // With expiration
    use std::time::Duration;
    redis.set_ex("temp_key", "value", Duration::from_secs(60)).await?;

    Ok(())
}
```

## Pub/Sub

```rust,ignore
use armature_redis::{RedisService, RedisConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = RedisConfig::builder().url("redis://localhost:6379").build();
    let service = RedisService::new(config).await?;

    // Subscribe
    let mut subscription = service.pubsub()?.subscribe("channel").await?;
    while let Some(message) = subscription.recv().await {
        println!("Received: {:?}", message);
    }

    // Publish
    service.pubsub()?.publish("channel", "Hello!").await?;

    Ok(())
}
```

## License

MIT OR Apache-2.0
