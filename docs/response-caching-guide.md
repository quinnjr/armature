# Response Caching

This guide covers HTTP response caching in Armature, including Cache-Control headers, in-memory caching, and cache invalidation.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Cache-Control Headers](#cache-control-headers)
- [In-Memory Response Cache](#in-memory-response-cache)
- [Cache Keys](#cache-keys)
- [Request Extensions](#request-extensions)
- [Response Extensions](#response-extensions)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

## Overview

Armature provides comprehensive HTTP response caching support:

1. **Cache-Control headers** - Parse and generate Cache-Control directives
2. **In-memory cache** - Store and retrieve responses with TTL
3. **Cache keys** - Generate unique keys with Vary header support
4. **Extensions** - Convenient methods on HttpRequest and HttpResponse

## Features

- ✅ Full Cache-Control header parsing and generation
- ✅ All standard cache directives supported
- ✅ In-memory response cache with configurable TTL
- ✅ Vary header support for content negotiation
- ✅ Automatic cache eviction and stale entry purging
- ✅ Cache statistics
- ✅ Preset configurations for common scenarios

## Cache-Control Headers

### Parsing Cache-Control

```rust
use armature_framework::prelude::*;

let cc = CacheControl::parse("public, max-age=3600, must-revalidate");

assert!(cc.is_public());
assert_eq!(cc.get_max_age(), Some(3600));
assert!(cc.is_must_revalidate());
assert!(cc.is_cacheable());
```

### Building Cache-Control

```rust
use armature_framework::prelude::*;
use std::time::Duration;

let cc = CacheControl::new()
    .public()
    .max_age(Duration::from_secs(3600))
    .must_revalidate();

assert_eq!(cc.to_header_value(), "public, max-age=3600, must-revalidate");
```

### Preset Configurations

```rust
use armature_framework::prelude::*;
use std::time::Duration;

// Never cache (no-store, no-cache)
let never = CacheControl::never();

// Public cache with max-age
let public = CacheControl::public_max_age(Duration::from_secs(3600));

// Private cache with max-age
let private = CacheControl::private_max_age(Duration::from_secs(300));

// Immutable assets (versioned files)
let immutable = CacheControl::immutable_asset(Duration::from_secs(31536000)); // 1 year

// Must revalidate after TTL
let revalidate = CacheControl::revalidate(Duration::from_secs(60));
```

### Cache Directives

| Directive | Description |
|-----------|-------------|
| `public` | Response can be cached by any cache |
| `private` | Response is for single user, not shared caches |
| `no-store` | Response must not be stored |
| `no-cache` | Response can be stored but must be revalidated |
| `max-age=N` | Response is fresh for N seconds |
| `s-maxage=N` | Shared cache max-age (overrides max-age) |
| `must-revalidate` | Stale responses must be revalidated |
| `proxy-revalidate` | Shared caches must revalidate |
| `immutable` | Response will never change |
| `no-transform` | Response must not be transformed |

## In-Memory Response Cache

### Basic Usage

```rust
use armature_framework::prelude::*;
use std::time::Duration;

#[controller("/api")]
struct ApiController {
    cache: ResponseCache,
}

#[controller]
impl ApiController {
    #[get("/data")]
    async fn get_data(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        // Check cache first
        if let Some(cached) = self.cache.get(&request).await {
            return Ok(cached);
        }

        // Generate response
        let data = expensive_operation();
        let response = HttpResponse::ok()
            .cache_public(Duration::from_secs(300))
            .with_json(&data)?;

        // Store in cache
        self.cache.store(&request, &response).await;

        Ok(response)
    }
}
```

### Custom Configuration

```rust
use armature_framework::prelude::*;
use std::time::Duration;

let cache = ResponseCache::with_config(ResponseCacheConfig {
    max_entries: 10000,
    default_ttl: Duration::from_secs(600),
    max_body_size: 5 * 1024 * 1024,  // 5MB
    cacheable_status_codes: vec![200, 203, 204, 301],
    cacheable_methods: vec!["GET".into(), "HEAD".into()],
});
```

### Cache Operations

```rust
use armature_framework::prelude::*;

async fn cache_operations(cache: &ResponseCache, request: &HttpRequest, response: &HttpResponse) {
    // Store with default TTL
    cache.store(request, response).await;

    // Store with custom TTL
    cache.store_with_ttl(request, response, Duration::from_secs(60)).await;

    // Get cached response
    if let Some(cached) = cache.get(request).await {
        // Use cached response
    }

    // Invalidate specific entry
    cache.invalidate(request).await;

    // Invalidate by path prefix
    cache.invalidate_prefix("/api/users").await;

    // Clear all entries
    cache.clear().await;

    // Remove stale entries
    cache.purge_stale().await;

    // Get statistics
    let stats = cache.stats().await;
    println!("Entries: {}/{}", stats.total_entries, stats.max_entries);
    println!("Fresh: {}, Stale: {}", stats.fresh_entries, stats.stale_entries);
}
```

## Cache Keys

### Automatic Key Generation

```rust
use armature_framework::prelude::*;

let request = HttpRequest::new("GET".into(), "/api/users".into());
let key = request.cache_key();

// Key includes: method, path, sorted query params
println!("Key: {}", key);  // GET:/api/users
```

### Keys with Vary Headers

```rust
use armature_framework::prelude::*;

let mut request = HttpRequest::new("GET".into(), "/api/users".into());
request.headers.insert("Accept".into(), "application/json".into());
request.headers.insert("Accept-Language".into(), "en-US".into());

// Include Vary headers in key
let key = request.cache_key_with_vary(&["Accept", "Accept-Language"]);

// Different Accept values = different cache entries
```

### Cache with Vary Support

```rust
use armature_framework::prelude::*;

async fn get_with_vary(cache: &ResponseCache, request: &HttpRequest) -> Option<HttpResponse> {
    // Get with specific Vary headers
    cache.get_with_vary(request, &["Accept", "Accept-Encoding"]).await
}
```

## Request Extensions

```rust
use armature_framework::prelude::*;

fn analyze_request(request: &HttpRequest) {
    // Get Cache-Control from request
    if let Some(cc) = request.cache_control() {
        if cc.is_no_cache() {
            // Client wants fresh response
        }
    }

    // Check if request allows cached responses
    if request.allows_cached() {
        // Can serve from cache
    }

    // Get max-stale tolerance
    if let Some(max_stale) = request.max_stale() {
        // Client accepts stale responses up to max_stale seconds
    }

    // Generate cache key
    let key = request.cache_key();
}
```

## Response Extensions

```rust
use armature_framework::prelude::*;
use std::time::Duration;

fn build_cached_response() -> HttpResponse {
    HttpResponse::ok()
        // Set Cache-Control with builder
        .with_cache_control(
            CacheControl::new()
                .public()
                .max_age(Duration::from_secs(3600))
        )
        // Or use convenience methods
        .cache_public(Duration::from_secs(3600))
        // Set Vary header
        .with_vary(&["Accept", "Accept-Encoding"])
}

fn cache_control_shortcuts() {
    // No caching
    let _ = HttpResponse::ok().no_cache();

    // Public cache
    let _ = HttpResponse::ok().cache_public(Duration::from_secs(3600));

    // Private cache
    let _ = HttpResponse::ok().cache_private(Duration::from_secs(300));

    // Immutable assets
    let _ = HttpResponse::ok().cache_immutable(Duration::from_secs(31536000));
}

fn check_response_cacheability(response: &HttpResponse) {
    if let Some(cc) = response.get_cache_control() {
        println!("Cacheable: {}", cc.is_cacheable());
        println!("Max-Age: {:?}", cc.get_max_age());
    }

    if response.is_cacheable() {
        // Response can be cached
    }
}
```

## Best Practices

### 1. Choose the Right Cache-Control

```rust
use armature_framework::prelude::*;
use std::time::Duration;

// Static assets - cache for a long time with immutable
fn static_asset_response() -> HttpResponse {
    HttpResponse::ok()
        .cache_immutable(Duration::from_secs(31536000))  // 1 year
}

// API data - short cache, must revalidate
fn api_response() -> HttpResponse {
    HttpResponse::ok()
        .with_cache_control(
            CacheControl::new()
                .public()
                .max_age(Duration::from_secs(60))
                .must_revalidate()
        )
}

// User-specific data - private cache
fn user_data_response() -> HttpResponse {
    HttpResponse::ok()
        .cache_private(Duration::from_secs(300))
}

// Sensitive data - never cache
fn sensitive_response() -> HttpResponse {
    HttpResponse::ok().no_cache()
}
```

### 2. Use Vary Headers Correctly

```rust
use armature_framework::prelude::*;

fn content_negotiated_response() -> HttpResponse {
    HttpResponse::ok()
        .cache_public(Duration::from_secs(3600))
        .with_vary(&["Accept", "Accept-Encoding", "Accept-Language"])
}
```

### 3. Invalidate on Mutations

```rust
use armature_framework::prelude::*;

#[put("/users/:id")]
async fn update_user(
    &self,
    request: HttpRequest,
    #[param("id")] id: u64,
) -> Result<HttpResponse, Error> {
    // Update user
    let user = update_user_in_db(id)?;

    // Invalidate cache
    self.cache.invalidate_prefix(&format!("/users/{}", id)).await;

    HttpResponse::ok().with_json(&user)
}
```

### 4. Configure Cache Appropriately

```rust
use armature_framework::prelude::*;
use std::time::Duration;

// Small API responses
let api_cache = ResponseCache::with_config(
    ResponseCacheConfig::new()
        .max_entries(10000)
        .default_ttl(Duration::from_secs(60))
        .max_body_size(100 * 1024)  // 100KB
);

// Large file responses
let file_cache = ResponseCache::with_config(
    ResponseCacheConfig::new()
        .max_entries(100)
        .default_ttl(Duration::from_secs(3600))
        .max_body_size(10 * 1024 * 1024)  // 10MB
);
```

## Examples

### Complete Cached API Endpoint

```rust
use armature_framework::prelude::*;
use std::time::Duration;
use std::sync::Arc;

#[controller("/api")]
struct ProductController {
    cache: Arc<ResponseCache>,
}

#[controller]
impl ProductController {
    #[get("/products")]
    async fn list_products(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        // Check if client allows cached response
        if !request.allows_cached() {
            return self.fetch_fresh_products().await;
        }

        // Try cache
        if let Some(cached) = self.cache.get_with_vary(
            &request,
            &["Accept", "Accept-Language"]
        ).await {
            return Ok(cached);
        }

        // Fetch fresh data
        let products = load_products_from_db()?;

        // Build cacheable response
        let response = HttpResponse::ok()
            .with_json(&products)?
            .cache_public(Duration::from_secs(300))
            .with_vary(&["Accept", "Accept-Language"]);

        // Store in cache
        self.cache.store(&request, &response).await;

        Ok(response)
    }

    #[post("/products")]
    async fn create_product(
        &self,
        #[body] product: CreateProduct,
    ) -> Result<HttpResponse, Error> {
        let created = save_product_to_db(&product)?;

        // Invalidate product list cache
        self.cache.invalidate_prefix("/api/products").await;

        HttpResponse::created()
            .no_cache()
            .with_json(&created)
    }

    async fn fetch_fresh_products(&self) -> Result<HttpResponse, Error> {
        let products = load_products_from_db()?;
        HttpResponse::ok()
            .no_cache()
            .with_json(&products)
    }
}
```

### Cache Middleware Pattern

```rust
use armature_framework::prelude::*;
use std::sync::Arc;

struct CachingMiddleware {
    cache: Arc<ResponseCache>,
    ttl: Duration,
}

impl CachingMiddleware {
    async fn handle(
        &self,
        request: HttpRequest,
        next: impl FnOnce(HttpRequest) -> Result<HttpResponse, Error>,
    ) -> Result<HttpResponse, Error> {
        // Only cache GET requests
        if request.method != "GET" {
            return next(request);
        }

        // Check cache
        if let Some(cached) = self.cache.get(&request).await {
            return Ok(cached);
        }

        // Call handler
        let response = next(request.clone())?;

        // Cache if cacheable
        if response.is_cacheable() {
            self.cache.store_with_ttl(&request, &response, self.ttl).await;
        }

        Ok(response)
    }
}
```

### Versioned Static Assets

```rust
use armature_framework::prelude::*;
use std::time::Duration;

#[controller("/assets")]
struct AssetController;

#[controller]
impl AssetController {
    #[get("/:version/:filename")]
    async fn get_asset(
        &self,
        #[param("version")] version: String,
        #[param("filename")] filename: String,
    ) -> Result<HttpResponse, Error> {
        let content = load_asset(&filename)?;
        let content_type = guess_content_type(&filename);

        Ok(HttpResponse::ok()
            .with_header("Content-Type".into(), content_type)
            .with_body(content)
            // Version in URL = can cache forever
            .cache_immutable(Duration::from_secs(31536000)))
    }
}
```

## Common Pitfalls

- ❌ Caching user-specific data with public
- ❌ Forgetting Vary headers with content negotiation
- ❌ Not invalidating cache after mutations
- ❌ Caching responses with no-store directive

- ✅ Use `private` for user-specific data
- ✅ Include all negotiated headers in Vary
- ✅ Invalidate affected cache entries on write operations
- ✅ Check `is_cacheable()` before storing

## Summary

| Component | Purpose |
|-----------|---------|
| `CacheControl` | Parse/build Cache-Control headers |
| `ResponseCache` | In-memory response caching |
| `CacheKey` | Generate unique cache keys |
| `ResponseCacheConfig` | Configure cache behavior |

**Key Points:**

1. **Cache-Control** - Use appropriate directives for your content type
2. **Vary headers** - Include all headers that affect response content
3. **Invalidation** - Invalidate cache on mutations
4. **TTL** - Set reasonable expiration times
5. **Private data** - Use `private` or `no-store` for sensitive content


