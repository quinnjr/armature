# API Versioning Guide

Armature provides comprehensive API versioning support with multiple strategies for version extraction and flexible version management.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Versioning Strategies](#versioning-strategies)
- [Basic Usage](#basic-usage)
- [Version Configuration](#version-configuration)
- [Versioned Handlers](#versioned-handlers)
- [Request and Response Extensions](#request-and-response-extensions)
- [Migration Strategies](#migration-strategies)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

API versioning allows you to evolve your API over time while maintaining backward compatibility. Armature supports multiple versioning strategies:

- **URL Path**: `/v1/users`, `/v2/users`
- **Header**: `X-API-Version: 1`
- **Query Parameter**: `/users?api-version=1`
- **Media Type**: `Accept: application/vnd.myapi.v1+json`

## Features

- ✅ Multiple versioning strategies
- ✅ Semantic versioning support (1.0, 2.1, etc.)
- ✅ Version constraints (exact, minimum, range)
- ✅ Deprecated version warnings
- ✅ Response header injection
- ✅ Combined/fallback strategies
- ✅ Version-specific route handlers

## Versioning Strategies

### URL Path Versioning

The most common and visible approach:

```rust
use armature_core::versioning::{VersioningStrategy, ApiVersion};

let strategy = VersioningStrategy::url_path();

// Matches: /v1/users, /v2/users/123, /v3/posts
```

With a prefix:

```rust
let strategy = VersioningStrategy::url_path_with_prefix("/api");

// Matches: /api/v1/users, /api/v2/users
```

### Header Versioning

Version in HTTP header (cleaner URLs):

```rust
let strategy = VersioningStrategy::header();
// Uses X-API-Version header by default

// Or custom header name
let strategy = VersioningStrategy::header_with_name("API-Version");
```

Client sends:
```http
GET /users HTTP/1.1
X-API-Version: 2
```

### Query Parameter Versioning

Version in query string:

```rust
let strategy = VersioningStrategy::query_param();
// Uses api-version parameter by default

// Or custom parameter name
let strategy = VersioningStrategy::query_param_with_name("version");
```

URL: `/users?api-version=2`

### Media Type Versioning

Version in Accept header (RESTful approach):

```rust
let strategy = VersioningStrategy::media_type("vnd.myapi");
```

Client sends:
```http
GET /users HTTP/1.1
Accept: application/vnd.myapi.v2+json
```

### Combined Strategies

Try multiple strategies in order:

```rust
// Default: URL path → Header → Query param
let strategy = VersioningStrategy::default_combined();

// Custom order
let strategy = VersioningStrategy::combined(vec![
    VersioningStrategy::header(),
    VersioningStrategy::query_param(),
    VersioningStrategy::url_path(),
]);
```

## Basic Usage

### Extracting Version from Request

```rust
use armature_core::{HttpRequest, HttpResponse, Error};
use armature_core::versioning::{VersioningStrategy, VersionedRequest, ApiVersion};

async fn handle_request(req: HttpRequest) -> Result<HttpResponse, Error> {
    let strategy = VersioningStrategy::url_path();

    // Extract version
    let version = req.api_version(&strategy)
        .unwrap_or(ApiVersion::V1);

    // Route based on version
    match version.major {
        1 => handle_v1(req).await,
        2 => handle_v2(req).await,
        _ => Err(Error::BadRequest(format!(
            "Unsupported API version: {}",
            version
        ))),
    }
}
```

### Convenience Methods

```rust
use armature_core::versioning::VersionedRequest;

// URL path version
let version = req.url_version();

// Header version
let version = req.header_version("X-API-Version");

// Query param version
let version = req.query_version("api-version");
```

### Adding Version to Response

```rust
use armature_core::versioning::{VersionedResponse, ApiVersion};

let response = HttpResponse::ok()
    .with_json(&data)?
    .with_api_version(ApiVersion::V2);
```

## Version Configuration

### Basic Configuration

```rust
use armature_core::versioning::{VersionConfig, VersioningStrategy, ApiVersion};

let config = VersionConfig::new(VersioningStrategy::url_path())
    .default_version(ApiVersion::V2)
    .supported_versions([ApiVersion::V1, ApiVersion::V2, ApiVersion::V3])
    .deprecated(ApiVersion::V1)
    .require_version(false)
    .add_response_headers(true);
```

### Resolving Version from Request

```rust
async fn handle(req: HttpRequest, config: &VersionConfig) -> Result<HttpResponse, Error> {
    // Resolve version (uses default if not provided)
    let version = config.resolve_version(&req)?;

    // Process request...
    let response = process(&req, &version)?;

    // Apply version headers
    Ok(config.apply_headers(response, &version))
}
```

### Configuration Options

| Option | Description |
|--------|-------------|
| `default_version` | Version to use when none provided |
| `supported_versions` | List of valid versions |
| `deprecated` | Mark version as deprecated |
| `require_version` | Fail if version not provided |
| `add_response_headers` | Add version info to responses |

## Versioned Handlers

Route to different handlers based on version:

```rust
use armature_core::versioning::{VersionedHandler, ApiVersion};

// Create versioned handler registry
let handler = VersionedHandler::new()
    .version(ApiVersion::V1, handle_users_v1)
    .version(ApiVersion::V2, handle_users_v2)
    .version(ApiVersion::V3, handle_users_v3)
    .fallback(handle_users_latest);

// Get handler for version
if let Some(handler) = handler.get(&version) {
    handler(req).await
}

// Or get compatible handler (same major version)
if let Some(handler) = handler.get_compatible(&version) {
    handler(req).await
}
```

### Version-Specific Route Implementation

```rust
use armature_core::{HttpRequest, HttpResponse, Error};

// V1 implementation
async fn get_users_v1(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Simple user list
    let users: Vec<UserV1> = db.get_users().await?;
    HttpResponse::ok().with_json(&users)
}

// V2 implementation with pagination
async fn get_users_v2(req: HttpRequest) -> Result<HttpResponse, Error> {
    let page = req.query("page")
        .and_then(|p| p.parse().ok())
        .unwrap_or(1);

    let users: PaginatedResult<UserV2> = db.get_users_paginated(page).await?;
    HttpResponse::ok().with_json(&users)
}

// V3 implementation with cursor-based pagination
async fn get_users_v3(req: HttpRequest) -> Result<HttpResponse, Error> {
    let cursor = req.query("cursor");

    let result: CursorResult<UserV3> = db.get_users_cursor(cursor).await?;
    HttpResponse::ok().with_json(&result)
}
```

## Request and Response Extensions

### VersionedRequest Trait

```rust
pub trait VersionedRequest {
    /// Extract version using strategy
    fn api_version(&self, strategy: &VersioningStrategy) -> Option<ApiVersion>;

    /// Extract from URL path
    fn url_version(&self) -> Option<ApiVersion>;

    /// Extract from header
    fn header_version(&self, header_name: &str) -> Option<ApiVersion>;

    /// Extract from query parameter
    fn query_version(&self, param_name: &str) -> Option<ApiVersion>;
}
```

### VersionedResponse Trait

```rust
pub trait VersionedResponse {
    /// Add X-API-Version header
    fn with_api_version(self, version: ApiVersion) -> Self;

    /// Add deprecation warning headers
    fn with_deprecation_warning(self, message: &str) -> Self;

    /// Add X-API-Supported-Versions header
    fn with_supported_versions(self, versions: &[ApiVersion]) -> Self;

    /// Add Sunset header (RFC 8594)
    fn with_sunset_date(self, date: &str) -> Self;
}
```

### Response Headers

When `add_response_headers` is enabled:

```http
HTTP/1.1 200 OK
X-API-Version: 2
X-API-Supported-Versions: 1, 2, 3
```

For deprecated versions:

```http
HTTP/1.1 200 OK
X-API-Version: 1
X-API-Deprecated: true
X-API-Deprecation-Message: API version 1 is deprecated
Deprecation: true
```

With sunset date:

```http
HTTP/1.1 200 OK
Sunset: Sat, 31 Dec 2024 23:59:59 GMT
```

## Migration Strategies

### Gradual Deprecation

1. **Announce deprecation** (add warning headers)
2. **Set sunset date** (RFC 8594)
3. **Monitor usage** (log deprecated version calls)
4. **Remove support** (return 410 Gone)

```rust
let config = VersionConfig::new(VersioningStrategy::url_path())
    .supported_versions([ApiVersion::V1, ApiVersion::V2, ApiVersion::V3])
    .deprecated(ApiVersion::V1);

async fn handle(req: HttpRequest) -> Result<HttpResponse, Error> {
    let version = config.resolve_version(&req)?;
    let response = process(&req, &version)?;

    // Add sunset date for v1
    if version == ApiVersion::V1 {
        return Ok(response
            .with_api_version(version)
            .with_deprecation_warning("v1 will be removed on 2025-01-01")
            .with_sunset_date("Sat, 01 Jan 2025 00:00:00 GMT"));
    }

    Ok(config.apply_headers(response, &version))
}
```

### Version Compatibility

```rust
use armature_core::versioning::VersionConstraint;

// Accept any v2.x
let constraint = VersionConstraint::range(
    ApiVersion::new(2, 0),
    ApiVersion::new(2, 99),
);

if constraint.matches(&requested_version) {
    // Handle request
}
```

## Best Practices

### 1. Choose the Right Strategy

| Strategy | Pros | Cons | Use When |
|----------|------|------|----------|
| URL Path | Visible, cacheable | URL pollution | Public APIs |
| Header | Clean URLs | Less discoverable | Internal APIs |
| Query Param | Easy to test | Can be stripped | Development |
| Media Type | RESTful | Complex | HATEOAS APIs |

### 2. Start with URL Path

```rust
// Most widely understood
let strategy = VersioningStrategy::url_path();

// URLs like: /v1/users, /v2/users
```

### 3. Always Set a Default Version

```rust
let config = VersionConfig::new(strategy)
    .default_version(ApiVersion::V1);  // Don't break existing clients
```

### 4. Document Deprecation Timeline

```rust
fn deprecated_response(response: HttpResponse) -> HttpResponse {
    response
        .with_deprecation_warning("v1 deprecated since 2024-01-01")
        .with_sunset_date("Sat, 01 Jul 2024 00:00:00 GMT")
}
```

### 5. Use Semantic Versioning

```rust
// Major.Minor format
let version = ApiVersion::new(2, 1);  // v2.1

// Breaking changes → increment major
// Non-breaking additions → increment minor
```

### 6. Maintain Version Parity

Keep similar functionality across versions when possible:

```rust
// Both versions return users, just different formats
async fn get_users_v1(req: HttpRequest) -> Result<HttpResponse, Error> { ... }
async fn get_users_v2(req: HttpRequest) -> Result<HttpResponse, Error> { ... }
```

### 7. Version Your Data Models

```rust
// Separate models per version
mod v1 {
    #[derive(Serialize)]
    pub struct User {
        pub id: u64,
        pub name: String,
    }
}

mod v2 {
    #[derive(Serialize)]
    pub struct User {
        pub id: u64,
        pub first_name: String,
        pub last_name: String,
        pub email: String,
    }
}
```

## Common Pitfalls

### ❌ Breaking Changes in Minor Versions

```rust
// Bad: Breaking change in v1.1
let v1_0 = ApiVersion::new(1, 0);
let v1_1 = ApiVersion::new(1, 1);  // Changed field name - breaking!

// Good: Use v2 for breaking changes
let v2 = ApiVersion::new(2, 0);
```

### ❌ No Default Version

```rust
// Bad: Fails for clients without version
let config = VersionConfig::new(strategy)
    .require_version(true);  // Forces all clients to upgrade

// Good: Graceful fallback
let config = VersionConfig::new(strategy)
    .default_version(ApiVersion::V1);
```

### ❌ Removing Versions Too Quickly

```rust
// Bad: Sudden removal
supported_versions: [ApiVersion::V3]  // V1, V2 clients break!

// Good: Gradual deprecation
supported_versions: [ApiVersion::V1, ApiVersion::V2, ApiVersion::V3]
deprecated: [ApiVersion::V1]  // Warn first
```

## API Reference

### Types

| Type | Description |
|------|-------------|
| `ApiVersion` | Version number (major.minor) |
| `VersioningStrategy` | Strategy for extracting version |
| `VersionConfig` | Configuration for version handling |
| `VersionConstraint` | Version matching rules |
| `VersionedHandler<T>` | Version-specific handler registry |
| `VersionParseError` | Error parsing version string |

### Constants

```rust
ApiVersion::V1  // Version 1.0
ApiVersion::V2  // Version 2.0
ApiVersion::V3  // Version 3.0
```

### Traits

| Trait | Description |
|-------|-------------|
| `VersionedRequest` | Request version extraction methods |
| `VersionedResponse` | Response version header methods |

## Summary

**Key Points:**

1. **Choose a strategy** - URL path is most common for public APIs
2. **Configure properly** - Set default version and supported versions
3. **Handle deprecation gracefully** - Use warning headers and sunset dates
4. **Version your models** - Keep separate DTOs per version
5. **Document changes** - Maintain clear changelog

**Quick Reference:**

```rust
use armature_core::versioning::{
    ApiVersion, VersionConfig, VersioningStrategy,
    VersionedRequest, VersionedResponse
};

// Setup
let config = VersionConfig::new(VersioningStrategy::url_path())
    .default_version(ApiVersion::V1)
    .supported_versions([ApiVersion::V1, ApiVersion::V2])
    .deprecated(ApiVersion::V1);

// Extract version
let version = config.resolve_version(&req)?;

// Version-specific handling
match version.major {
    1 => handle_v1(req).await,
    2 => handle_v2(req).await,
    _ => Err(Error::BadRequest("Unsupported version")),
}

// Add response headers
response.with_api_version(version)
```


