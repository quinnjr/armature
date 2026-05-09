# Request Timeouts and Limits

This guide covers configuring request timeouts, body size limits, and other server settings in Armature.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Basic Usage](#basic-usage)
- [Configuration Options](#configuration-options)
- [Preset Configurations](#preset-configurations)
- [Timeout Behavior](#timeout-behavior)
- [Error Responses](#error-responses)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

## Overview

Armature provides configurable request timeouts and size limits to protect your application from:

- **Slow clients** - Connections that take too long to send data
- **Large payloads** - Requests that exceed memory limits
- **Slow handlers** - Request handlers that take too long to respond
- **Resource exhaustion** - Too many headers or oversized headers

## Features

- ✅ Configurable request timeout for entire request lifecycle
- ✅ Separate body read timeout for large uploads
- ✅ Maximum body size limits with early rejection
- ✅ Maximum header size and count limits
- ✅ Keep-alive configuration
- ✅ Preset configurations for common use cases
- ✅ JSON error responses with detailed messages

## Basic Usage

### Default Configuration

By default, Armature uses sensible defaults:

```rust
use armature_framework::prelude::*;

#[tokio::main]
async fn main() {
    // Uses default ServerConfig:
    // - 30 second request timeout
    // - 60 second body timeout
    // - 1MB max body size
    // - 8KB max header size
    let app = Application::create::<AppModule>().await;
    app.listen(3000).await.unwrap();
}
```

### Custom Configuration

```rust
use armature_framework::prelude::*;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let config = ServerConfig::new()
        .request_timeout(Duration::from_secs(60))
        .body_timeout(Duration::from_secs(120))
        .max_body_size(10 * 1024 * 1024)  // 10MB
        .max_header_size(16 * 1024);      // 16KB

    let app = Application::create_with_config::<AppModule>(config).await;
    app.listen(3000).await.unwrap();
}
```

## Configuration Options

### `ServerConfig` Fields

| Option | Default | Description |
|--------|---------|-------------|
| `request_timeout` | 30 seconds | Maximum time for entire request (headers + body + handler) |
| `body_timeout` | 60 seconds | Maximum time to read the request body |
| `max_body_size` | 1MB | Maximum request body size in bytes |
| `max_header_size` | 8KB | Maximum total header size in bytes |
| `max_headers` | 100 | Maximum number of headers |
| `keep_alive` | true | Enable HTTP keep-alive connections |
| `keep_alive_timeout` | 5 seconds | Keep-alive timeout duration |

### Builder Methods

```rust
use armature_framework::prelude::*;
use std::time::Duration;

let config = ServerConfig::new()
    // Request timing
    .request_timeout(Duration::from_secs(30))
    .body_timeout(Duration::from_secs(60))

    // Size limits
    .max_body_size(1_048_576)     // 1MB
    .max_header_size(8_192)       // 8KB
    .max_headers(100)

    // Connection settings
    .keep_alive(true)
    .keep_alive_timeout(Duration::from_secs(5));
```

## Preset Configurations

Armature provides preset configurations for common use cases:

### API Server (Default)

Optimized for typical API workloads:

```rust
let config = ServerConfig::api();
// - Request timeout: 30s
// - Body timeout: 30s
// - Max body: 1MB
```

### File Upload Server

Optimized for handling large file uploads:

```rust
let config = ServerConfig::file_upload();
// - Request timeout: 5 minutes
// - Body timeout: 10 minutes
// - Max body: 100MB
```

### No Timeout

For internal services with other timeout mechanisms:

```rust
let config = ServerConfig::no_timeout();
// ⚠️ Use with caution! Only for trusted internal services.
```

## Timeout Behavior

### Request Timeout

The request timeout covers the entire request lifecycle after headers are received:

```
┌─────────────────────────────────────────────────────┐
│                  Request Timeout                     │
├─────────────────────────────────────────────────────┤
│  Read Body  │  Execute Handler  │  Send Response    │
└─────────────────────────────────────────────────────┘
```

If the timeout expires, a `408 Request Timeout` response is sent.

### Body Timeout

The body timeout specifically covers reading the request body:

```
┌─────────────────────┐
│    Body Timeout     │
├─────────────────────┤
│     Read Body       │
└─────────────────────┘
```

This is separate from the request timeout to allow longer times for large uploads while keeping handler execution fast.

## Error Responses

Armature returns JSON error responses when limits are exceeded:

### 408 Request Timeout

```json
{
  "error": "Request Timeout",
  "message": "Request timed out after 30 seconds",
  "status": 408
}
```

### 413 Payload Too Large

```json
{
  "error": "Payload Too Large",
  "message": "Request body too large: 5242880 bytes (max: 1048576 bytes)",
  "status": 413
}
```

### 431 Request Header Fields Too Large

```json
{
  "error": "Request Header Fields Too Large",
  "message": "Too many headers: 150 (max: 100)",
  "status": 431
}
```

## Best Practices

### 1. Set Appropriate Timeouts

Consider your application's needs:

```rust
// API endpoint - fast responses expected
let api_config = ServerConfig::new()
    .request_timeout(Duration::from_secs(10));

// Long-running report generation
let report_config = ServerConfig::new()
    .request_timeout(Duration::from_secs(300));
```

### 2. Size Limits Based on Content

```rust
// JSON API - small payloads
let json_api = ServerConfig::new()
    .max_body_size(100 * 1024);  // 100KB

// Image upload API
let image_api = ServerConfig::new()
    .max_body_size(5 * 1024 * 1024);  // 5MB

// Video upload API
let video_api = ServerConfig::new()
    .max_body_size(500 * 1024 * 1024);  // 500MB
```

### 3. Consider Client Network Conditions

```rust
// Mobile-friendly settings
let mobile_config = ServerConfig::new()
    .body_timeout(Duration::from_secs(120))  // Slower uploads
    .request_timeout(Duration::from_secs(60));
```

### 4. Defense in Depth

Combine with other protections:

```rust
use armature_framework::prelude::*;
use armature_ratelimit::*;

// Server-level limits
let config = ServerConfig::new()
    .max_body_size(1024 * 1024)
    .request_timeout(Duration::from_secs(30));

// Plus rate limiting
let rate_limit = RateLimiter::new(100, Duration::from_secs(60));
```

## Examples

### Complete API Server

```rust
use armature_framework::prelude::*;
use std::time::Duration;

#[module]
struct AppModule;

#[controller("/api")]
struct ApiController;

#[controller]
impl ApiController {
    #[post("/data")]
    async fn create_data(&self, #[body] data: CreateRequest) -> HttpResponse {
        // Handler implementation
        HttpResponse::created()
    }
}

#[tokio::main]
async fn main() {
    // Initialize logging
    let _guard = Application::init_logging();

    // Configure server
    let config = ServerConfig::new()
        .request_timeout(Duration::from_secs(30))
        .body_timeout(Duration::from_secs(30))
        .max_body_size(1024 * 1024)  // 1MB
        .max_headers(50)
        .keep_alive(true)
        .keep_alive_timeout(Duration::from_secs(10));

    // Create and run application
    let app = Application::create_with_config::<AppModule>(config).await;

    println!("Server configuration:");
    println!("  Request timeout: {:?}", app.config().request_timeout);
    println!("  Max body size: {} bytes", app.config().max_body_size);

    app.listen(3000).await.unwrap();
}
```

### File Upload Server

```rust
use armature_framework::prelude::*;
use std::time::Duration;

#[module]
struct UploadModule;

#[controller("/upload")]
struct UploadController;

#[controller]
impl UploadController {
    #[post("/file")]
    async fn upload_file(&self, request: HttpRequest) -> HttpResponse {
        let fields = request.multipart()?;
        // Process uploaded file
        HttpResponse::ok()
    }
}

#[tokio::main]
async fn main() {
    let config = ServerConfig::file_upload()
        .max_body_size(200 * 1024 * 1024);  // 200MB limit

    let app = Application::create_with_config::<UploadModule>(config).await;
    app.listen(3000).await.unwrap();
}
```

### Mixed Configuration with Middleware

For applications needing different limits for different routes, use middleware:

```rust
use armature_framework::prelude::*;

// Custom middleware to check body size per-route
async fn check_body_size(
    request: HttpRequest,
    max_size: usize,
) -> Result<HttpRequest, Error> {
    if request.body.len() > max_size {
        return Err(Error::PayloadTooLarge(format!(
            "Body exceeds limit of {} bytes",
            max_size
        )));
    }
    Ok(request)
}
```

## Common Pitfalls

- ❌ Setting timeouts too low for legitimate use cases
- ❌ Allowing unlimited body sizes on public endpoints
- ❌ Using `no_timeout()` on public-facing services
- ❌ Not considering slow network conditions

- ✅ Testing timeout behavior in development
- ✅ Setting limits based on actual requirements
- ✅ Monitoring timeout and size limit errors
- ✅ Providing clear error messages to clients

## Summary

| Configuration | Use Case |
|--------------|----------|
| `ServerConfig::api()` | Standard REST APIs |
| `ServerConfig::file_upload()` | File upload services |
| Custom config | Specific requirements |

**Key Points:**

1. **Always set timeouts** - Protect against slow clients and stuck handlers
2. **Size limits are critical** - Prevent memory exhaustion attacks
3. **Use presets** - Start with `api()` or `file_upload()` for common cases
4. **Monitor errors** - Track 408 and 413 responses to tune settings
5. **Test thoroughly** - Verify timeout behavior before production


