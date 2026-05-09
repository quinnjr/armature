# ETag and Conditional Requests

This guide covers HTTP ETag generation and conditional request handling in Armature, enabling efficient caching and optimistic concurrency control.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [ETags](#etags)
- [Conditional GET (If-None-Match)](#conditional-get-if-none-match)
- [Conditional PUT/DELETE (If-Match)](#conditional-putdelete-if-match)
- [Time-Based Conditions](#time-based-conditions)
- [Request Extensions](#request-extensions)
- [Response Extensions](#response-extensions)
- [Helper Functions](#helper-functions)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

## Overview

Conditional requests allow clients and servers to optimize HTTP communication by:

1. **Caching** - Avoid re-downloading unchanged resources (304 Not Modified)
2. **Concurrency Control** - Prevent lost updates when multiple clients modify resources (412 Precondition Failed)

Armature provides comprehensive support for:
- **ETag generation** (strong and weak)
- **If-None-Match** header (conditional GET)
- **If-Match** header (conditional PUT/DELETE)
- **If-Modified-Since** header (time-based caching)
- **If-Unmodified-Since** header (time-based concurrency)

## Features

- ✅ Strong and weak ETag support
- ✅ Multiple ETag generation methods (bytes, version, file metadata)
- ✅ If-None-Match for 304 responses
- ✅ If-Match for optimistic locking
- ✅ Time-based conditional headers
- ✅ Wildcard (`*`) support
- ✅ HttpRequest and HttpResponse extensions
- ✅ Convenience helper functions

## ETags

### Creating ETags

```rust
use armature_framework::prelude::*;

// Strong ETag - byte-for-byte identical
let strong = ETag::strong("abc123");
assert_eq!(strong.to_header_value(), "\"abc123\"");

// Weak ETag - semantically equivalent
let weak = ETag::weak("abc123");
assert_eq!(weak.to_header_value(), "W/\"abc123\"");
```

### Generating ETags

```rust
use armature_framework::prelude::*;
use std::time::SystemTime;

// From bytes (hash-based)
let data = b"Hello, World!";
let etag = ETag::from_bytes(data);

// From string content
let etag = ETag::from_str("Hello, World!");

// From version number (e.g., database revision)
let etag = ETag::from_version(42);  // "v42"

// From file metadata
let etag = ETag::from_file_metadata(1024, SystemTime::now());
```

### Parsing ETags

```rust
use armature_framework::prelude::*;

let strong = ETag::parse("\"abc123\"").unwrap();
assert!(!strong.weak);

let weak = ETag::parse("W/\"abc123\"").unwrap();
assert!(weak.weak);
```

### ETag Comparison

```rust
use armature_framework::prelude::*;

let strong1 = ETag::strong("abc");
let strong2 = ETag::strong("abc");
let weak1 = ETag::weak("abc");

// Strong comparison - both must be strong with identical values
assert!(strong1.strong_match(&strong2));
assert!(!strong1.strong_match(&weak1));

// Weak comparison - values must match (ignores weak flag)
assert!(strong1.weak_match(&weak1));
```

## Conditional GET (If-None-Match)

Use `If-None-Match` to return 304 Not Modified when the client already has the current version.

### Basic Usage

```rust
use armature_framework::prelude::*;

#[controller("/api")]
struct ResourceController;

#[controller]
impl ResourceController {
    #[get("/data")]
    async fn get_data(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        // Load your data
        let data = load_data();
        let etag = ETag::from_version(data.version);

        // Check if client has current version
        if request.if_none_match_matches(&etag) {
            return Ok(HttpResponse::not_modified_with_etag(&etag));
        }

        // Return full response with ETag
        HttpResponse::ok()
            .with_etag(&etag)
            .with_json(&data)
    }
}
```

### Using check_conditionals Helper

```rust
use armature_framework::prelude::*;

#[get("/resource/:id")]
async fn get_resource(
    &self,
    request: HttpRequest,
    #[param("id")] id: u64,
) -> Result<HttpResponse, Error> {
    let resource = load_resource(id);
    let etag = ETag::from_version(resource.version);
    let last_modified = resource.updated_at;

    // Check conditionals - returns early if 304 or 412
    if let Some(response) = check_conditionals(&request, Some(&etag), Some(last_modified)) {
        return Ok(response);
    }

    // Normal response with caching headers
    HttpResponse::ok()
        .with_etag(&etag)
        .with_last_modified(last_modified)
        .with_json(&resource)
}
```

## Conditional PUT/DELETE (If-Match)

Use `If-Match` for optimistic concurrency control - only update if the client has the current version.

### Basic Usage

```rust
use armature_framework::prelude::*;

#[put("/resource/:id")]
async fn update_resource(
    &self,
    request: HttpRequest,
    #[param("id")] id: u64,
    #[body] update: UpdateRequest,
) -> Result<HttpResponse, Error> {
    // Get current resource
    let current = load_resource(id);
    let current_etag = ETag::from_version(current.version);

    // Verify client has current version
    if !request.if_match_matches(&current_etag) {
        return Ok(HttpResponse::precondition_failed_with_message(
            "Resource has been modified by another client"
        ));
    }

    // Proceed with update
    let updated = save_resource(id, update);
    let new_etag = ETag::from_version(updated.version);

    HttpResponse::ok()
        .with_etag(&new_etag)
        .with_json(&updated)
}
```

### Requiring If-Match Header

```rust
use armature_framework::prelude::*;

#[delete("/resource/:id")]
async fn delete_resource(
    &self,
    request: HttpRequest,
    #[param("id")] id: u64,
) -> Result<HttpResponse, Error> {
    // Require If-Match header for safety
    let if_match = request.if_match()
        .ok_or_else(|| Error::BadRequest("If-Match header required".into()))?;

    let current = load_resource(id);
    let current_etag = ETag::from_version(current.version);

    if !if_match.contains_strong(&current_etag) {
        return Ok(HttpResponse::precondition_failed());
    }

    delete_resource(id);
    Ok(HttpResponse::no_content())
}
```

## Time-Based Conditions

### If-Modified-Since

```rust
use armature_framework::prelude::*;
use std::time::SystemTime;

#[get("/resource")]
async fn get_resource(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let resource = load_resource();
    let last_modified = resource.updated_at;

    // Check if resource was modified since the client's cached version
    if request.not_modified_since(last_modified) {
        return Ok(HttpResponse::not_modified()
            .with_last_modified(last_modified));
    }

    HttpResponse::ok()
        .with_last_modified(last_modified)
        .with_json(&resource)
}
```

### If-Unmodified-Since

```rust
use armature_framework::prelude::*;

#[put("/resource")]
async fn update_resource(
    &self,
    request: HttpRequest,
    #[body] update: UpdateRequest,
) -> Result<HttpResponse, Error> {
    let resource = load_resource();
    let last_modified = resource.updated_at;

    // Fail if resource was modified since the client's version
    if request.modified_since_precondition(last_modified) {
        return Ok(HttpResponse::precondition_failed_with_message(
            "Resource has been modified since your version"
        ));
    }

    // Proceed with update
    let updated = save_resource(update);
    HttpResponse::ok().with_json(&updated)
}
```

## Request Extensions

The `ConditionalRequest` trait adds methods to `HttpRequest`:

```rust
use armature_framework::prelude::*;

fn handle_request(request: &HttpRequest) {
    // Parse all conditional headers at once
    let headers = request.conditional_headers();

    // Get individual parsed headers
    let if_none_match = request.if_none_match();      // Option<ETagList>
    let if_match = request.if_match();                // Option<ETagList>
    let if_modified_since = request.if_modified_since();    // Option<SystemTime>
    let if_unmodified_since = request.if_unmodified_since(); // Option<SystemTime>

    // Quick checks
    let etag = ETag::strong("abc123");
    let matches_none_match = request.if_none_match_matches(&etag);
    let matches_if_match = request.if_match_matches(&etag);

    // Time-based checks
    let last_modified = std::time::SystemTime::now();
    let not_modified = request.not_modified_since(last_modified);
    let precondition_failed = request.modified_since_precondition(last_modified);

    // Evaluate all conditionals
    match request.evaluate_conditionals(Some(&etag), Some(last_modified)) {
        Some(304) => { /* Return 304 Not Modified */ }
        Some(412) => { /* Return 412 Precondition Failed */ }
        None => { /* Proceed normally */ }
    }
}
```

## Response Extensions

The `ConditionalResponse` trait adds methods to `HttpResponse`:

```rust
use armature_framework::prelude::*;
use std::time::SystemTime;

// Add ETag header
let response = HttpResponse::ok()
    .with_etag(&ETag::strong("abc123"));

// Add Last-Modified header
let response = HttpResponse::ok()
    .with_last_modified(SystemTime::now());

// Create 304 Not Modified
let response = HttpResponse::not_modified();
let response = HttpResponse::not_modified_with_etag(&ETag::strong("abc123"));

// Create 412 Precondition Failed
let response = HttpResponse::precondition_failed();
let response = HttpResponse::precondition_failed_with_message("Resource modified");
```

## Helper Functions

### check_conditionals

Handles the common pattern of checking conditionals and returning early:

```rust
use armature_framework::prelude::*;

#[get("/data")]
async fn get_data(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let data = load_data();
    let etag = ETag::from_bytes(&data);
    let last_modified = get_last_modified();

    // Returns Some(304) or Some(412) response if conditions met
    if let Some(response) = check_conditionals(&request, Some(&etag), Some(last_modified)) {
        return Ok(response);
    }

    // Normal response
    HttpResponse::ok()
        .with_etag(&etag)
        .with_last_modified(last_modified)
        .with_json(&data)
}
```

### cacheable_response

Creates a response with proper caching headers:

```rust
use armature_core::conditional::cacheable_response;

#[get("/data")]
async fn get_data(&self) -> Result<HttpResponse, Error> {
    let data = load_data();
    let etag = ETag::from_version(data.version);
    let last_modified = data.updated_at;

    cacheable_response(&data, &etag, Some(last_modified))
}
```

## Best Practices

### 1. Choose the Right ETag Type

```rust
// Strong ETag - for exact byte matching
// Use for: Static files, binary content
let strong = ETag::strong("hash");

// Weak ETag - for semantic equivalence
// Use for: JSON with optional whitespace, gzip variations
let weak = ETag::weak("hash");
```

### 2. Include ETags in All GET Responses

```rust
#[get("/resource")]
async fn get_resource(&self) -> Result<HttpResponse, Error> {
    let resource = load_resource();
    let etag = ETag::from_version(resource.version);

    HttpResponse::ok()
        .with_etag(&etag)  // Always include!
        .with_json(&resource)
}
```

### 3. Require If-Match for Destructive Operations

```rust
#[delete("/resource/:id")]
async fn delete_resource(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    // Require If-Match to prevent accidental deletions
    if request.if_match().is_none() {
        return Err(Error::BadRequest("If-Match header required".into()));
    }
    // ...
}
```

### 4. Add Vary Header for Content Negotiation

```rust
response.headers.insert(
    "Vary".to_string(),
    "Accept, Accept-Encoding".to_string(),
);
```

### 5. Use Appropriate Cache-Control

```rust
// Private, revalidate every time
response.headers.insert(
    "Cache-Control".to_string(),
    "private, must-revalidate".to_string(),
);

// Public, cacheable for 1 hour
response.headers.insert(
    "Cache-Control".to_string(),
    "public, max-age=3600".to_string(),
);
```

## Examples

### Complete CRUD with Optimistic Locking

```rust
use armature_framework::prelude::*;
use std::time::SystemTime;

#[derive(Serialize, Deserialize)]
struct Resource {
    id: u64,
    name: String,
    version: u64,
    updated_at: SystemTime,
}

#[controller("/resources")]
struct ResourceController;

#[controller]
impl ResourceController {
    #[get("/:id")]
    async fn get(&self, request: HttpRequest, #[param("id")] id: u64) -> Result<HttpResponse, Error> {
        let resource = load_resource(id)?;
        let etag = ETag::from_version(resource.version);

        // Check for 304
        if let Some(response) = check_conditionals(&request, Some(&etag), Some(resource.updated_at)) {
            return Ok(response);
        }

        HttpResponse::ok()
            .with_etag(&etag)
            .with_last_modified(resource.updated_at)
            .with_json(&resource)
    }

    #[put("/:id")]
    async fn update(
        &self,
        request: HttpRequest,
        #[param("id")] id: u64,
        #[body] update: UpdateRequest,
    ) -> Result<HttpResponse, Error> {
        let current = load_resource(id)?;
        let current_etag = ETag::from_version(current.version);

        // Require If-Match for optimistic locking
        if !request.if_match_matches(&current_etag) {
            return Ok(HttpResponse::precondition_failed_with_message(
                "Resource has been modified. Please refresh and try again."
            ));
        }

        let updated = save_resource(id, update)?;
        let new_etag = ETag::from_version(updated.version);

        HttpResponse::ok()
            .with_etag(&new_etag)
            .with_last_modified(updated.updated_at)
            .with_json(&updated)
    }

    #[delete("/:id")]
    async fn delete(&self, request: HttpRequest, #[param("id")] id: u64) -> Result<HttpResponse, Error> {
        let current = load_resource(id)?;
        let current_etag = ETag::from_version(current.version);

        // Require If-Match for safe deletion
        if !request.if_match_matches(&current_etag) {
            return Ok(HttpResponse::precondition_failed());
        }

        delete_resource(id)?;
        Ok(HttpResponse::no_content())
    }
}
```

### Caching API Responses

```rust
use armature_framework::prelude::*;
use armature_core::conditional::cacheable_response;

#[get("/products")]
async fn list_products(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
    let products = load_products();

    // Generate ETag from serialized data
    let data_bytes = serde_json::to_vec(&products)?;
    let etag = ETag::from_bytes(&data_bytes);

    // Check conditionals
    if let Some(response) = check_conditionals(&request, Some(&etag), None) {
        return Ok(response);
    }

    // Create cacheable response
    let mut response = HttpResponse::ok()
        .with_etag(&etag)
        .with_body(data_bytes);

    response.headers.insert("Content-Type".into(), "application/json".into());
    response.headers.insert("Cache-Control".into(), "public, max-age=300".into());
    response.headers.insert("Vary".into(), "Accept".into());

    Ok(response)
}
```

## Common Pitfalls

- ❌ Forgetting to include ETag in responses
- ❌ Using strong ETags for content that varies by compression
- ❌ Not requiring If-Match for PUT/DELETE
- ❌ Ignoring conditional headers (missing 304 optimization)

- ✅ Always include ETag in GET responses
- ✅ Use weak ETags for content with acceptable variations
- ✅ Require If-Match for optimistic concurrency control
- ✅ Return 304 when If-None-Match matches

## Summary

| Header | Use Case | Response |
|--------|----------|----------|
| `If-None-Match` | Conditional GET (caching) | 304 Not Modified |
| `If-Match` | Conditional PUT/DELETE (concurrency) | 412 Precondition Failed |
| `If-Modified-Since` | Time-based caching | 304 Not Modified |
| `If-Unmodified-Since` | Time-based concurrency | 412 Precondition Failed |

**Key Points:**

1. **Generate ETags** - Use `ETag::from_version()`, `ETag::from_bytes()`, etc.
2. **Check conditionals** - Use `check_conditionals()` or individual methods
3. **Return proper status** - 304 for cache hits, 412 for conflicts
4. **Include headers** - Always send ETag and Last-Modified in responses
5. **Require If-Match** - For safe PUT/DELETE operations


