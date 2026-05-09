# Use Middleware Decorator Guide

Armature provides the `#[use_middleware]` decorator for applying middleware to individual route handlers or entire controllers. This enables declarative middleware configuration at the route level.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Basic Usage](#basic-usage)
- [Multiple Middleware](#multiple-middleware)
- [Controller-Level Middleware](#controller-level-middleware)
- [Built-in Middleware](#built-in-middleware)
- [Custom Middleware](#custom-middleware)
- [Execution Order](#execution-order)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The `#[use_middleware]` decorator wraps route handlers with a middleware chain, allowing you to:

- Apply middleware to specific routes without affecting others
- Chain multiple middleware together
- Keep middleware configuration close to the routes they affect
- Create reusable middleware combinations

## Features

- ✅ Route-level middleware application
- ✅ Controller-level middleware inheritance
- ✅ Multiple middleware chaining
- ✅ Works with all HTTP method decorators
- ✅ Compatible with request extractors
- ✅ Type-safe middleware configuration

## Basic Usage

### Single Middleware

Apply a single middleware to a route:

```rust
use armature_framework::{get, use_middleware};
use armature_core::{HttpRequest, HttpResponse, Error, LoggerMiddleware};

#[use_middleware(LoggerMiddleware::new())]
#[get("/users")]
async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok().with_json(&serde_json::json!({
        "users": []
    }))?)
}
```

### With Configuration

Middleware can be configured inline:

```rust
use armature_framework::{get, use_middleware};
use armature_core::{HttpRequest, HttpResponse, Error, CorsMiddleware};

#[use_middleware(CorsMiddleware::new().allow_origin("https://example.com"))]
#[get("/api/data")]
async fn get_data(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

## Multiple Middleware

Chain multiple middleware together by separating them with commas:

```rust
use armature_framework::{get, use_middleware};
use armature_core::{
    HttpRequest, HttpResponse, Error,
    LoggerMiddleware, CorsMiddleware, SecurityHeadersMiddleware
};

#[use_middleware(
    LoggerMiddleware::new(),
    CorsMiddleware::new(),
    SecurityHeadersMiddleware::new()
)]
#[get("/protected")]
async fn protected_endpoint(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

Middleware executes in the order specified:
1. `LoggerMiddleware` runs first (logs incoming request)
2. `CorsMiddleware` runs second (adds CORS headers)
3. `SecurityHeadersMiddleware` runs third (adds security headers)
4. Handler executes
5. Middleware runs in reverse order for response processing

## Controller-Level Middleware

Apply middleware to all routes in a controller using the `#[middleware]` decorator:

```rust
use armature_framework::{controller, get, post, middleware};
use armature_core::{HttpRequest, HttpResponse, Error, LoggerMiddleware, CorsMiddleware};

#[middleware(LoggerMiddleware::new(), CorsMiddleware::new())]
#[controller("/api")]
struct ApiController;

impl ApiController {
    // All routes in this controller automatically have logging and CORS

    #[get("/users")]
    async fn get_users(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::ok())
    }

    #[post("/users")]
    async fn create_user(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        Ok(HttpResponse::created())
    }
}
```

### Combining Controller and Route Middleware

Route-level middleware adds to controller-level middleware:

```rust
use armature_framework::{controller, get, middleware, use_middleware};
use armature_core::{
    HttpRequest, HttpResponse, Error,
    LoggerMiddleware, TimeoutMiddleware
};

#[middleware(LoggerMiddleware::new())]  // Applied to all routes
#[controller("/api")]
struct ApiController;

impl ApiController {
    #[get("/fast")]
    async fn fast_endpoint(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        // Only has LoggerMiddleware
        Ok(HttpResponse::ok())
    }

    #[use_middleware(TimeoutMiddleware::new(30))]  // Additional middleware
    #[get("/slow")]
    async fn slow_endpoint(&self, req: HttpRequest) -> Result<HttpResponse, Error> {
        // Has LoggerMiddleware + TimeoutMiddleware
        Ok(HttpResponse::ok())
    }
}
```

## Built-in Middleware

Armature provides several built-in middleware:

| Middleware | Purpose |
|------------|---------|
| `LoggerMiddleware` | Log requests and responses |
| `LoggingMiddleware` | Structured logging with tracing |
| `CorsMiddleware` | Handle CORS headers |
| `SecurityHeadersMiddleware` | Add security headers (HSTS, XSS, etc.) |
| `TimeoutMiddleware` | Request timeout handling |
| `BodySizeLimitMiddleware` | Limit request body size |
| `RequestIdMiddleware` | Add unique request IDs |
| `CompressionMiddleware` | Response compression hints |

### Example: Common Middleware Stack

```rust
#[use_middleware(
    RequestIdMiddleware,
    LoggingMiddleware::new(),
    SecurityHeadersMiddleware::new(),
    CorsMiddleware::new().allow_credentials(true),
    TimeoutMiddleware::new(30)
)]
#[get("/api/secure")]
async fn secure_endpoint(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

## Custom Middleware

Create custom middleware by implementing the `Middleware` trait:

```rust
use armature_core::{HttpRequest, HttpResponse, Error};
use armature_core::middleware::{Middleware, Next};
use async_trait::async_trait;

/// Authentication middleware
pub struct AuthMiddleware {
    api_key: String,
}

impl AuthMiddleware {
    pub fn new(api_key: &str) -> Self {
        Self { api_key: api_key.to_string() }
    }
}

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Check for API key in headers
        let auth_header = req.headers.get("X-API-Key")
            .or_else(|| req.headers.get("x-api-key"));

        match auth_header {
            Some(key) if key == &self.api_key => {
                // Valid key, proceed to handler
                next(req).await
            }
            Some(_) => {
                // Invalid key
                Ok(HttpResponse::new(401)
                    .with_json(&serde_json::json!({
                        "error": "Invalid API key"
                    }))?)
            }
            None => {
                // Missing key
                Ok(HttpResponse::new(401)
                    .with_json(&serde_json::json!({
                        "error": "Missing API key"
                    }))?)
            }
        }
    }
}
```

Use your custom middleware:

```rust
use armature_framework::{get, use_middleware};
use my_app::AuthMiddleware;

#[use_middleware(AuthMiddleware::new("secret-key"))]
#[get("/protected")]
async fn protected(req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}
```

### Middleware with State

Middleware can maintain state:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub struct RateLimitMiddleware {
    requests: Arc<AtomicU64>,
    max_requests: u64,
}

impl RateLimitMiddleware {
    pub fn new(max_requests: u64) -> Self {
        Self {
            requests: Arc::new(AtomicU64::new(0)),
            max_requests,
        }
    }
}

#[async_trait]
impl Middleware for RateLimitMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let current = self.requests.fetch_add(1, Ordering::Relaxed);

        if current >= self.max_requests {
            return Ok(HttpResponse::new(429)
                .with_json(&serde_json::json!({
                    "error": "Too many requests"
                }))?);
        }

        next(req).await
    }
}
```

## Execution Order

Understanding middleware execution order is crucial:

```
Request Flow:
┌─────────────────────────────────────────────────────┐
│  Incoming Request                                    │
├─────────────────────────────────────────────────────┤
│  ↓ Middleware 1 (before)                            │
│  ↓ Middleware 2 (before)                            │
│  ↓ Middleware 3 (before)                            │
├─────────────────────────────────────────────────────┤
│  → Handler Executes                                  │
├─────────────────────────────────────────────────────┤
│  ↑ Middleware 3 (after)                             │
│  ↑ Middleware 2 (after)                             │
│  ↑ Middleware 1 (after)                             │
├─────────────────────────────────────────────────────┤
│  Outgoing Response                                   │
└─────────────────────────────────────────────────────┘
```

### Example: Logging Timing

```rust
pub struct TimingMiddleware;

#[async_trait]
impl Middleware for TimingMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let start = std::time::Instant::now();

        // Execute next middleware/handler
        let response = next(req).await?;

        // After handler completes
        let duration = start.elapsed();
        println!("Request took {:?}", duration);

        Ok(response)
    }
}
```

## Best Practices

### 1. Order Middleware Carefully

Place middleware in logical order:

```rust
#[use_middleware(
    RequestIdMiddleware,       // First: assign request ID
    LoggingMiddleware::new(),  // Second: log with request ID
    AuthMiddleware::new(),     // Third: authenticate
    TimeoutMiddleware::new(30) // Fourth: enforce timeout
)]
```

### 2. Keep Middleware Focused

Each middleware should do one thing well:

```rust
// ✅ Good: Single responsibility
pub struct AuthMiddleware;  // Only handles authentication
pub struct LoggingMiddleware;  // Only handles logging

// ❌ Bad: Multiple responsibilities
pub struct EverythingMiddleware;  // Auth + Logging + CORS + ...
```

### 3. Use Controller-Level for Common Middleware

```rust
// ✅ Good: Common middleware at controller level
#[middleware(LoggingMiddleware::new(), CorsMiddleware::new())]
#[controller("/api")]
struct ApiController;

// ❌ Bad: Repeating on every route
impl ApiController {
    #[use_middleware(LoggingMiddleware::new(), CorsMiddleware::new())]
    #[get("/users")]
    async fn get_users(&self, req: HttpRequest) -> Result<HttpResponse, Error> { ... }

    #[use_middleware(LoggingMiddleware::new(), CorsMiddleware::new())]
    #[get("/posts")]
    async fn get_posts(&self, req: HttpRequest) -> Result<HttpResponse, Error> { ... }
}
```

### 4. Handle Errors Gracefully

Middleware should handle its own errors:

```rust
#[async_trait]
impl Middleware for MyMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        // Handle potential errors
        match self.validate(&req) {
            Ok(()) => next(req).await,
            Err(e) => {
                // Return error response instead of propagating
                Ok(HttpResponse::bad_request()
                    .with_json(&serde_json::json!({
                        "error": e.to_string()
                    }))?)
            }
        }
    }
}
```

### 5. Avoid Cloning Large Data

```rust
// ✅ Good: Only clone what's needed
let path = req.path.clone();
let method = req.method.clone();
let response = next(req).await?;

// ❌ Bad: Cloning entire request
let req_clone = req.clone();
let response = next(req).await?;
// req_clone is now stale
```

## Common Pitfalls

### ❌ Forgetting to Call `next`

```rust
// Bad: Handler never executes
async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
    // Missing: next(req).await
    Ok(HttpResponse::ok())  // Short-circuits the chain
}

// Good: Always call next (or return error response)
async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
    if !self.validate(&req) {
        return Ok(HttpResponse::unauthorized());
    }
    next(req).await  // Continue to handler
}
```

### ❌ Modifying Response Without Returning It

```rust
// Bad: Modifications lost
async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
    let response = next(req).await?;
    response.headers.insert("X-Custom".to_string(), "value".to_string());
    // Missing: return modified response
    Ok(HttpResponse::ok())
}

// Good: Return modified response
async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
    let mut response = next(req).await?;
    response.headers.insert("X-Custom".to_string(), "value".to_string());
    Ok(response)
}
```

## API Reference

### Decorators

| Decorator | Target | Description |
|-----------|--------|-------------|
| `#[use_middleware(...)]` | Function | Apply middleware to a route handler |
| `#[middleware(...)]` | Struct | Apply middleware to a controller |

### Built-in Middleware Types

| Type | Constructor | Description |
|------|-------------|-------------|
| `LoggerMiddleware` | `new()` | Simple console logging |
| `LoggingMiddleware` | `new()` | Structured tracing logs |
| `CorsMiddleware` | `new()` | CORS header management |
| `SecurityHeadersMiddleware` | `new()` | Security headers (HSTS, XSS, etc.) |
| `TimeoutMiddleware` | `new(seconds)` | Request timeout |
| `BodySizeLimitMiddleware` | `new(bytes)` | Body size limits |
| `RequestIdMiddleware` | (unit struct) | Request ID generation |
| `CompressionMiddleware` | `new()` | Compression hints |

## Summary

**Key Points:**

1. **`#[use_middleware]`** applies middleware to individual routes
2. **`#[middleware]`** applies middleware to entire controllers
3. Middleware executes in **order specified** (before handler) and **reverse order** (after handler)
4. Always call `next(req).await` unless short-circuiting
5. Use controller-level middleware for common functionality
6. Keep middleware focused on single responsibilities

**Quick Reference:**

```rust
// Route-level
#[use_middleware(LoggerMiddleware::new())]
#[get("/users")]
async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> { ... }

// Controller-level
#[middleware(LoggerMiddleware::new())]
#[controller("/api")]
struct ApiController;

// Multiple middleware
#[use_middleware(
    LoggerMiddleware::new(),
    CorsMiddleware::new(),
    TimeoutMiddleware::new(30)
)]
#[get("/protected")]
async fn protected(req: HttpRequest) -> Result<HttpResponse, Error> { ... }
```

