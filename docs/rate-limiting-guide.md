# Rate Limiting Guide

Rate limiting protects your API from abuse and ensures fair usage across all clients.
The `armature-ratelimit` crate provides a comprehensive, production-ready rate limiting solution.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Algorithms](#algorithms)
- [Storage Backends](#storage-backends)
- [Key Extraction](#key-extraction)
- [Middleware Integration](#middleware-integration)
- [Configuration](#configuration)
- [Best Practices](#best-practices)
- [Common Pitfalls](#common-pitfalls)
- [API Reference](#api-reference)

## Overview

Rate limiting controls how many requests a client can make within a time window.
This prevents:

- **Abuse**: Malicious actors overwhelming your API
- **Resource exhaustion**: A single client consuming all server resources
- **Cascading failures**: Overload propagating through your system
- **Cost overruns**: Excessive usage driving up infrastructure costs

## Features

- ✅ **Multiple Algorithms**: Token bucket, sliding window log, fixed window
- ✅ **Distributed Support**: Redis backend for multi-instance deployments
- ✅ **Flexible Key Extraction**: By IP, user ID, API key, or custom function
- ✅ **Standard Headers**: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`
- ✅ **Per-Route Limits**: Different limits for different endpoints
- ✅ **Bypass Rules**: Whitelist specific clients or API keys
- ✅ **Fail-Open Mode**: Continue serving requests if rate limit storage fails

## Quick Start

Add the dependency:

```toml
[dependencies]
armature-ratelimit = "0.1"
```

Basic usage:

```rust
use armature_ratelimit::{RateLimiter, Algorithm};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a rate limiter with token bucket algorithm
    let limiter = RateLimiter::builder()
        .algorithm(Algorithm::TokenBucket {
            capacity: 100,        // Maximum burst size
            refill_rate: 10.0,    // 10 tokens per second
        })
        .build()
        .await?;

    // Check if a request is allowed
    let result = limiter.check("client_ip_123").await?;

    if result.allowed {
        println!("Request allowed! {} remaining", result.remaining);
    } else {
        println!("Rate limited. Retry after {:?}", result.retry_after);
    }

    Ok(())
}
```

## Algorithms

### Token Bucket

The token bucket algorithm provides smooth rate limiting with burst capacity.

**How it works:**
1. A bucket starts full with `capacity` tokens
2. Each request consumes one token
3. Tokens are refilled at `refill_rate` per second
4. Requests are denied when the bucket is empty

**Best for:** APIs that allow occasional bursts but need average rate control.

```rust
use armature_ratelimit::Algorithm;

let algo = Algorithm::TokenBucket {
    capacity: 100,      // Allow bursts up to 100 requests
    refill_rate: 10.0,  // Steady rate of 10 requests/second
};
```

**Example scenario:** A user can make up to 100 rapid requests, then must wait
for tokens to refill at 10/second.

### Sliding Window Log

The sliding window log algorithm provides precise rate limiting by tracking
individual request timestamps.

**How it works:**
1. Each request timestamp is logged
2. On each request, count timestamps within the window
3. Deny if count exceeds `max_requests`
4. Old timestamps are automatically cleaned up

**Best for:** Strict rate limiting where accuracy is critical.

```rust
use armature_ratelimit::Algorithm;
use std::time::Duration;

let algo = Algorithm::SlidingWindowLog {
    max_requests: 100,              // 100 requests...
    window: Duration::from_secs(60), // ...per minute
};
```

**Trade-offs:**
- ✅ Most accurate algorithm
- ✅ No boundary issues
- ❌ Higher memory usage (stores all timestamps)

### Fixed Window

The fixed window algorithm divides time into fixed intervals and counts
requests per window.

**How it works:**
1. Time is divided into fixed windows (e.g., every minute)
2. Each request increments the counter for the current window
3. Counter resets when a new window starts
4. Deny if counter exceeds `max_requests`

**Best for:** Simple use cases, lowest resource usage.

```rust
use armature_ratelimit::Algorithm;
use std::time::Duration;

let algo = Algorithm::FixedWindow {
    max_requests: 100,
    window: Duration::from_secs(60),
};
```

**Trade-offs:**
- ✅ Simple and efficient
- ✅ Lowest memory usage
- ❌ Boundary burst issue: clients can make 2x requests at window boundaries

### Algorithm Comparison

| Algorithm | Accuracy | Memory | Complexity | Burst Handling |
|-----------|----------|--------|------------|----------------|
| Token Bucket | Medium | Low | Low | Controlled bursts |
| Sliding Window | High | Medium | Medium | No bursts |
| Fixed Window | Low | Very Low | Very Low | Boundary bursts |

## Storage Backends

### In-Memory Store (Default)

Uses DashMap for thread-safe concurrent access. Suitable for single-instance
deployments or development.

```rust
let limiter = RateLimiter::builder()
    .algorithm(Algorithm::token_bucket_default())
    .memory_store()  // This is the default
    .build()
    .await?;
```

**Pros:**
- Zero latency
- No external dependencies
- Simple setup

**Cons:**
- Not shared across instances
- State lost on restart

### Redis Store

Uses Redis for distributed rate limiting. Required for multi-instance deployments.

```rust
let limiter = RateLimiter::builder()
    .algorithm(Algorithm::token_bucket_default())
    .redis_store("redis://localhost:6379")
    .build()
    .await?;
```

**Pros:**
- Shared across all instances
- Persistent state
- Atomic operations via Lua scripts

**Cons:**
- Network latency
- Requires Redis infrastructure

**Enable the feature:**

```toml
[dependencies]
armature-ratelimit = { version = "0.1", features = ["redis"] }
```

## Key Extraction

Rate limits are applied per-key. The key extraction strategy determines how
clients are identified.

### By IP Address (Default)

```rust
use armature_ratelimit::{RateLimitMiddleware, KeyExtractor};

let middleware = RateLimitMiddleware::new(limiter)
    .with_extractor(KeyExtractor::Ip);
```

### By User ID

Requires authentication. Falls back to IP if user is not authenticated.

```rust
let middleware = RateLimitMiddleware::new(limiter)
    .with_extractor(KeyExtractor::UserId);
```

### By API Key

Extracts the key from a header (e.g., `X-API-Key`).

```rust
let middleware = RateLimitMiddleware::new(limiter)
    .with_extractor(KeyExtractor::ApiKey {
        header_name: "X-API-Key".to_string(),
    });
```

### By IP and Path

Different limits per endpoint.

```rust
let middleware = RateLimitMiddleware::new(limiter)
    .with_extractor(KeyExtractor::IpAndPath);
```

This creates keys like `192.168.1.1:/api/users`, allowing different rate limits
for different endpoints.

### Custom Extractor

Build complex extraction logic:

```rust
use armature_ratelimit::extractor::KeyExtractorBuilder;

let extractor = KeyExtractorBuilder::new()
    .prefer_user_id()           // Try user ID first
    .prefer_api_key("X-API-Key") // Then API key
    // Falls back to IP automatically
    .build();
```

## Middleware Integration

### Basic Middleware

```rust
use armature_ratelimit::{RateLimiter, RateLimitMiddleware, Algorithm};
use std::sync::Arc;

let limiter = Arc::new(
    RateLimiter::builder()
        .token_bucket(100, 10.0)
        .build()
        .await?
);

let middleware = RateLimitMiddleware::new(limiter)
    .with_headers(true)  // Include X-RateLimit-* headers
    .with_error_message("Too many requests. Please slow down.");
```

### Checking Requests

```rust
use armature_ratelimit::extractor::RequestInfo;
use std::net::{IpAddr, Ipv4Addr};

// Extract request info from your HTTP framework
let info = RequestInfo::new("/api/users", "GET")
    .with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
    .with_user_id("user_123");

// Check rate limit
let response = middleware.check(&info).await;

match response {
    RateLimitCheckResponse::Allowed { headers } => {
        // Add headers to response and continue
        if let Some(h) = headers {
            // Add X-RateLimit-Limit, X-RateLimit-Remaining, etc.
        }
    }
    RateLimitCheckResponse::Limited { headers, message, retry_after } => {
        // Return 429 Too Many Requests
        // Include Retry-After header
    }
}
```

## Configuration

### Builder Options

```rust
let limiter = RateLimiter::builder()
    // Algorithm (required)
    .algorithm(Algorithm::TokenBucket {
        capacity: 100,
        refill_rate: 10.0,
    })

    // Or use convenience methods
    .token_bucket(100, 10.0)
    .sliding_window(100, Duration::from_secs(60))
    .fixed_window(100, Duration::from_secs(60))

    // Storage backend
    .memory_store()
    .redis_store("redis://localhost:6379")

    // Key prefix for storage
    .key_prefix("api:ratelimit")

    // Include headers in responses
    .include_headers(true)

    // Fail open on storage errors
    .skip_on_error(true)

    // Custom error message
    .error_message("Rate limit exceeded")

    // Bypass specific keys
    .bypass_key("admin_api_key")
    .bypass_keys(["internal_service", "monitoring"])

    .build()
    .await?;
```

### Response Headers

When enabled, these headers are included in responses:

| Header | Description |
|--------|-------------|
| `X-RateLimit-Limit` | Maximum requests allowed |
| `X-RateLimit-Remaining` | Remaining requests in current window |
| `X-RateLimit-Reset` | Unix timestamp when the limit resets |
| `Retry-After` | Seconds until the client can retry (only on 429) |

## Best Practices

### 1. Choose the Right Algorithm

- **Token Bucket**: Most APIs—allows bursts, smooth average rate
- **Sliding Window**: Financial/gaming APIs—strict, no burst exploitation
- **Fixed Window**: High-volume, latency-sensitive—simple and fast

### 2. Use Redis for Production

```rust
// Single instance: memory is fine
let limiter = RateLimiter::builder()
    .token_bucket(100, 10.0)
    .build()
    .await?;

// Multiple instances: use Redis
let limiter = RateLimiter::builder()
    .token_bucket(100, 10.0)
    .redis_store("redis://redis-cluster:6379")
    .build()
    .await?;
```

### 3. Implement Tiered Limits

```rust
// Different limits for different user tiers
async fn check_rate_limit(user: &User, limiter: &RateLimiter) -> bool {
    let key = match user.tier {
        Tier::Free => format!("free:{}", user.id),
        Tier::Pro => format!("pro:{}", user.id),
        Tier::Enterprise => return true, // No limit
    };

    limiter.check(&key).await.map(|r| r.allowed).unwrap_or(true)
}
```

### 4. Include Helpful Headers

Always include rate limit headers so clients can self-regulate:

```rust
let middleware = RateLimitMiddleware::new(limiter)
    .with_headers(true);
```

### 5. Use Bypass for Internal Services

```rust
let limiter = RateLimiter::builder()
    .token_bucket(100, 10.0)
    .bypass_key("internal_service_key")
    .bypass_key("health_check_key")
    .build()
    .await?;
```

## Common Pitfalls

### ❌ Don't: Use IP-only limiting behind a proxy

```rust
// All requests will have the same IP (the proxy's IP)
let middleware = RateLimitMiddleware::new(limiter)
    .with_extractor(KeyExtractor::Ip);
```

### ✅ Do: Use X-Forwarded-For or X-Real-IP

```rust
// Extract the real client IP from headers
let info = RequestInfo::new(path, method)
    .with_header("X-Forwarded-For", forwarded_for);

// Or use the first IP from X-Forwarded-For
fn get_real_ip(headers: &Headers) -> Option<IpAddr> {
    headers.get("X-Forwarded-For")
        .and_then(|h| h.split(',').next())
        .and_then(|ip| ip.trim().parse().ok())
}
```

### ❌ Don't: Fail closed on errors

```rust
// If Redis is down, all requests will be denied!
let limiter = RateLimiter::builder()
    .skip_on_error(false)  // Bad for availability
    .build()
    .await?;
```

### ✅ Do: Fail open (default)

```rust
let limiter = RateLimiter::builder()
    .skip_on_error(true)  // Default, allows requests on storage failure
    .build()
    .await?;
```

### ❌ Don't: Use fixed window for strict limits

```rust
// Client can make 200 requests in 2 seconds by timing window boundaries
let algo = Algorithm::FixedWindow {
    max_requests: 100,
    window: Duration::from_secs(60),
};
```

### ✅ Do: Use sliding window for strict limits

```rust
// Accurate limiting, no boundary exploitation
let algo = Algorithm::SlidingWindowLog {
    max_requests: 100,
    window: Duration::from_secs(60),
};
```

## API Reference

### Core Types

- `RateLimiter` - Main rate limiter struct
- `RateLimiterBuilder` - Builder for configuring rate limiters
- `Algorithm` - Rate limiting algorithm enum
- `RateLimitCheckResult` - Result of a rate limit check

### Stores

- `MemoryStore` - In-memory storage using DashMap
- `RedisStore` - Redis-backed distributed storage (requires `redis` feature)

### Middleware

- `RateLimitMiddleware` - HTTP middleware for rate limiting
- `KeyExtractor` - Strategies for extracting rate limit keys
- `RequestInfo` - Request information for key extraction

### Errors

- `RateLimitError` - Error types for rate limiting operations
- `RateLimitHeaders` - Standard rate limit response headers

## Summary

Rate limiting is essential for production APIs. Key takeaways:

1. **Choose the right algorithm** for your use case
2. **Use Redis** for multi-instance deployments
3. **Include headers** so clients can self-regulate
4. **Fail open** to maintain availability
5. **Use tiered limits** for different user classes
6. **Handle proxy IPs** correctly

