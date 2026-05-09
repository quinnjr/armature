# Request Extractors

Type-safe extraction of data from HTTP requests using extractors and helper macros.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Extractor Types](#extractor-types)
- [Helper Macros](#helper-macros)
- [Traits](#traits)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The `armature-core` extractors module provides a type-safe way to extract data from HTTP requests, similar to NestJS decorators or Axum extractors. Instead of manually parsing request bodies, query parameters, and headers, you can use strongly-typed extractors that handle deserialization and error handling automatically.

## Features

- ✅ Type-safe body extraction with JSON deserialization
- ✅ Query parameter extraction with struct mapping
- ✅ Path parameter extraction with type coercion
- ✅ Header extraction with optional values
- ✅ Form data extraction
- ✅ Raw body access
- ✅ Content-Type and Method extractors
- ✅ Convenient helper macros for concise syntax

## Installation

Request extractors are included in `armature-core`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["core"] }
```

Or use directly:

```toml
[dependencies]
armature-core = "0.1"
```

## Extractor Types

### Body<T>

Extracts and deserializes the request body as JSON.

```rust
use armature_core::extractors::{Body, FromRequest};
use serde::Deserialize;

#[derive(Deserialize)]
struct CreateUser {
    name: String,
    email: String,
    age: Option<u32>,
}

fn create_user_handler(request: &HttpRequest) -> Result<(), Error> {
    let body: Body<CreateUser> = Body::from_request(request)?;

    println!("Creating user: {}", body.name);
    println!("Email: {}", body.email);

    // Access inner value
    let user = body.into_inner();

    Ok(())
}
```

### Query<T>

Extracts and deserializes query parameters into a struct.

```rust
use armature_core::extractors::{Query, FromRequest};
use serde::Deserialize;

#[derive(Deserialize)]
struct Pagination {
    page: Option<u32>,
    limit: Option<u32>,
    sort: Option<String>,
}

fn list_users_handler(request: &HttpRequest) -> Result<(), Error> {
    // Request: GET /users?page=2&limit=20&sort=name
    let query: Query<Pagination> = Query::from_request(request)?;

    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(10);

    println!("Page: {}, Limit: {}", page, limit);

    Ok(())
}
```

### Path<T>

Extracts a single path parameter by name and parses it to the specified type.

```rust
use armature_core::extractors::{Path, FromRequestNamed};

fn get_user_handler(request: &HttpRequest) -> Result<(), Error> {
    // Route: /users/:id
    let id: Path<u32> = Path::from_request(request, "id")?;

    println!("Fetching user with ID: {}", *id);

    // Convert to inner value
    let user_id: u32 = id.into_inner();

    Ok(())
}
```

### PathParams<T>

Extracts all path parameters into a struct.

```rust
use armature_core::extractors::{PathParams, FromRequest};
use serde::Deserialize;

#[derive(Deserialize)]
struct UserPostParams {
    user_id: u32,
    post_id: u32,
}

fn get_user_post_handler(request: &HttpRequest) -> Result<(), Error> {
    // Route: /users/:user_id/posts/:post_id
    let params: PathParams<UserPostParams> = PathParams::from_request(request)?;

    println!("User: {}, Post: {}", params.user_id, params.post_id);

    Ok(())
}
```

### Header

Extracts a single header value by name.

```rust
use armature_core::extractors::{Header, FromRequestNamed};

fn auth_handler(request: &HttpRequest) -> Result<(), Error> {
    let auth: Header = Header::from_request(request, "Authorization")?;

    println!("Auth header: {}", auth.value());

    // Get as owned String
    let token: String = auth.into_value();

    Ok(())
}
```

### Headers

Extracts all headers as a HashMap.

```rust
use armature_core::extractors::{Headers, FromRequest};

fn debug_handler(request: &HttpRequest) -> Result<(), Error> {
    let headers: Headers = Headers::from_request(request)?;

    for (name, value) in headers.iter() {
        println!("{}: {}", name, value);
    }

    // Check for specific header
    if let Some(content_type) = headers.get("Content-Type") {
        println!("Content-Type: {}", content_type);
    }

    Ok(())
}
```

### RawBody

Extracts the raw request body as bytes.

```rust
use armature_core::extractors::{RawBody, FromRequest};

fn raw_handler(request: &HttpRequest) -> Result<(), Error> {
    let raw: RawBody = RawBody::from_request(request)?;

    println!("Body length: {} bytes", raw.len());

    // Get as Vec<u8>
    let bytes: Vec<u8> = raw.into_inner();

    // Or as string (if valid UTF-8)
    let text = raw.as_str()?;

    Ok(())
}
```

### Form<T>

Extracts and deserializes form data (application/x-www-form-urlencoded).

```rust
use armature_core::extractors::{Form, FromRequest};
use serde::Deserialize;

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
    remember_me: Option<bool>,
}

fn login_handler(request: &HttpRequest) -> Result<(), Error> {
    let form: Form<LoginForm> = Form::from_request(request)?;

    println!("Username: {}", form.username);

    Ok(())
}
```

### ContentType

Extracts the Content-Type header.

```rust
use armature_core::extractors::{ContentType, FromRequest};

fn content_handler(request: &HttpRequest) -> Result<(), Error> {
    let content_type: ContentType = ContentType::from_request(request)?;

    if content_type.is_json() {
        // Handle JSON
    } else if content_type.is_form() {
        // Handle form data
    }

    println!("Content-Type: {}", content_type.value());

    Ok(())
}
```

### Method

Extracts the HTTP method.

```rust
use armature_core::extractors::{Method, FromRequest};

fn method_handler(request: &HttpRequest) -> Result<(), Error> {
    let method: Method = Method::from_request(request)?;

    match method.as_str() {
        "GET" => println!("Handling GET"),
        "POST" => println!("Handling POST"),
        _ => println!("Other method: {}", method.as_str()),
    }

    Ok(())
}
```

## Helper Macros

For more concise syntax, use the helper macros that extract and unwrap in one step.

### body!

Extract and deserialize request body.

```rust
use armature_framework::prelude::*;

fn handler(request: &HttpRequest) -> Result<(), Error> {
    // Extract body as CreateUser
    let user = body!(request, CreateUser)?;

    println!("Name: {}", user.name);

    Ok(())
}
```

### query!

Extract and deserialize query parameters.

```rust
use armature_framework::prelude::*;

fn handler(request: &HttpRequest) -> Result<(), Error> {
    // Extract query params as Pagination
    let pagination = query!(request, Pagination)?;

    println!("Page: {}", pagination.page.unwrap_or(1));

    Ok(())
}
```

### path!

Extract a path parameter by name with type conversion.

```rust
use armature_framework::prelude::*;

fn handler(request: &HttpRequest) -> Result<(), Error> {
    // Extract "id" parameter as u32
    let id: u32 = path!(request, "id", u32)?;

    // Extract "slug" parameter as String
    let slug: String = path!(request, "slug", String)?;

    println!("ID: {}, Slug: {}", id, slug);

    Ok(())
}
```

### header!

Extract a header value by name.

```rust
use armature_framework::prelude::*;

fn handler(request: &HttpRequest) -> Result<(), Error> {
    // Extract Authorization header
    let auth: String = header!(request, "Authorization")?;

    // Extract custom header
    let request_id: String = header!(request, "X-Request-ID")?;

    println!("Auth: {}", auth);

    Ok(())
}
```

## Traits

### FromRequest

Trait for extractors that don't need a parameter name.

```rust
pub trait FromRequest: Sized {
    fn from_request(request: &HttpRequest) -> Result<Self, Error>;
}
```

Implemented by: `Body<T>`, `Query<T>`, `PathParams<T>`, `Headers`, `RawBody`, `Form<T>`, `ContentType`, `Method`

### FromRequestNamed

Trait for extractors that require a parameter name.

```rust
pub trait FromRequestNamed: Sized {
    fn from_request(request: &HttpRequest, name: &str) -> Result<Self, Error>;
}
```

Implemented by: `Path<T>`, `Header`

## Best Practices

### 1. Use Specific Types

```rust
// ✅ Good - specific types
#[derive(Deserialize)]
struct CreateUser {
    name: String,
    email: String,
}
let body: Body<CreateUser> = Body::from_request(&request)?;

// ❌ Bad - generic JSON value
let body: Body<serde_json::Value> = Body::from_request(&request)?;
```

### 2. Make Fields Optional When Appropriate

```rust
#[derive(Deserialize)]
struct SearchParams {
    query: String,           // Required
    page: Option<u32>,       // Optional with default
    limit: Option<u32>,      // Optional with default
    sort: Option<String>,    // Optional
}

let params = query!(request, SearchParams)?;
let page = params.page.unwrap_or(1);
let limit = params.limit.unwrap_or(20).min(100); // Cap at 100
```

### 3. Handle Extraction Errors Gracefully

```rust
fn handler(request: &HttpRequest) -> HttpResponse {
    // Using match for custom error handling
    let body = match body!(request, CreateUser) {
        Ok(user) => user,
        Err(e) => {
            return HttpResponse::bad_request()
                .json(json!({ "error": format!("Invalid body: {}", e) }));
        }
    };

    // Process body...
    HttpResponse::ok()
}
```

### 4. Use Macros for Concise Code

```rust
// ✅ Concise with macros
fn handler(request: &HttpRequest) -> Result<HttpResponse, Error> {
    let user = body!(request, CreateUser)?;
    let filters = query!(request, Filters)?;
    let id: u32 = path!(request, "id", u32)?;
    let auth = header!(request, "Authorization")?;

    // Handle request...
    Ok(HttpResponse::ok())
}

// ❌ Verbose without macros
fn handler(request: &HttpRequest) -> Result<HttpResponse, Error> {
    let user = Body::<CreateUser>::from_request(request)?.into_inner();
    let filters = Query::<Filters>::from_request(request)?.into_inner();
    let id = Path::<u32>::from_request(request, "id")?.into_inner();
    let auth = Header::from_request(request, "Authorization")?.into_value();

    // Handle request...
    Ok(HttpResponse::ok())
}
```

### 5. Validate After Extraction

```rust
#[derive(Deserialize)]
struct CreateUser {
    name: String,
    email: String,
    age: u32,
}

fn handler(request: &HttpRequest) -> Result<HttpResponse, Error> {
    let user = body!(request, CreateUser)?;

    // Validate after extraction
    if user.name.is_empty() {
        return Err(Error::Validation("Name cannot be empty".into()));
    }
    if !user.email.contains('@') {
        return Err(Error::Validation("Invalid email format".into()));
    }
    if user.age < 18 {
        return Err(Error::Validation("Must be 18 or older".into()));
    }

    // Process valid user...
    Ok(HttpResponse::created())
}
```

## Common Pitfalls

- ❌ **Don't** forget to add `#[derive(Deserialize)]` on extraction structs
- ❌ **Don't** use `String` when a more specific type works (use `u32` for IDs)
- ❌ **Don't** ignore extraction errors in production code
- ✅ **Do** use `Option<T>` for optional parameters
- ✅ **Do** use the macros for cleaner code
- ✅ **Do** validate data after extraction

## API Reference

### Extractor Types

| Type | Trait | Description |
|------|-------|-------------|
| `Body<T>` | `FromRequest` | JSON body deserialization |
| `Query<T>` | `FromRequest` | Query parameter deserialization |
| `Path<T>` | `FromRequestNamed` | Single path parameter extraction |
| `PathParams<T>` | `FromRequest` | All path parameters deserialization |
| `Header` | `FromRequestNamed` | Single header extraction |
| `Headers` | `FromRequest` | All headers extraction |
| `RawBody` | `FromRequest` | Raw body bytes |
| `Form<T>` | `FromRequest` | Form data deserialization |
| `ContentType` | `FromRequest` | Content-Type header |
| `Method` | `FromRequest` | HTTP method |

### Macros

| Macro | Syntax | Returns |
|-------|--------|---------|
| `body!` | `body!(request, Type)` | `Result<Type, Error>` |
| `query!` | `query!(request, Type)` | `Result<Type, Error>` |
| `path!` | `path!(request, "name", Type)` | `Result<Type, Error>` |
| `header!` | `header!(request, "Name")` | `Result<String, Error>` |

## Summary

**Key Points:**

1. Use **extractors** for type-safe request data extraction
2. Use **macros** (`body!`, `query!`, `path!`, `header!`) for concise syntax
3. Implement `FromRequest` for extractors without names
4. Implement `FromRequestNamed` for named parameter extraction
5. Always validate data after extraction
6. Use `Option<T>` for optional fields

**Quick Reference:**

```rust
use armature_framework::prelude::*;

fn handler(request: &HttpRequest) -> Result<HttpResponse, Error> {
    // Body extraction
    let user = body!(request, CreateUser)?;

    // Query extraction
    let filters = query!(request, Filters)?;

    // Path extraction
    let id: u32 = path!(request, "id", u32)?;

    // Header extraction
    let auth = header!(request, "Authorization")?;

    Ok(HttpResponse::ok())
}
```

