# Micro-Framework Guide

A lightweight, Actix-style API for building web applications without the full module/controller system.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Routing](#routing)
- [Middleware](#middleware)
- [State Management](#state-management)
- [Scopes](#scopes)
- [Built-in Middleware](#built-in-middleware)
- [Configuration](#configuration)
- [Best Practices](#best-practices)
- [When to Use](#when-to-use)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The micro-framework provides a minimal, function-based API for building HTTP services. It's ideal for:

- Microservices
- Simple APIs
- Prototyping
- Learning Armature
- Projects that don't need DI or decorators

```text
┌─────────────────────────────────────────────────────────────────┐
│                    Micro-Framework Mode                          │
│                                                                  │
│  App::new()                                                     │
│    .data(State::new())          // Shared state                 │
│    .wrap(Logger::default())     // Middleware                   │
│    .route("/", get(index))      // Simple routes                │
│    .service(                    // Resource groups              │
│        scope("/api")                                            │
│            .route("/users", get(list_users))                    │
│            .route("/users/:id", get(get_user))                  │
│    )                                                            │
│    .run("0.0.0.0:8080")                                         │
│    .await?;                                                     │
└─────────────────────────────────────────────────────────────────┘
```

## Features

- ✅ Fluent builder API
- ✅ Function-based handlers
- ✅ Path parameters (`:id`, `:name`)
- ✅ Query string parsing
- ✅ Shared state via `Data<T>`
- ✅ Composable middleware
- ✅ Route scoping/grouping
- ✅ Built-in CORS, Logger, Compress
- ✅ JSON request/response helpers
- ✅ Zero-cost when not used

## Quick Start

### Minimal Example

```rust
use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};

async fn hello(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok().with_body(b"Hello, World!".to_vec()))
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .route("/", get(hello))
        .run("127.0.0.1:8080")
        .await
}
```

### JSON API Example

```rust
use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id: u64 = req.param("id")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    HttpResponse::json(&User {
        id,
        name: "Alice".to_string(),
    })
}

async fn list_users(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&vec![
        User { id: 1, name: "Alice".to_string() },
        User { id: 2, name: "Bob".to_string() },
    ])
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .wrap(Logger::default())
        .wrap(Cors::permissive())
        .route("/users", get(list_users))
        .route("/users/:id", get(get_user))
        .run("0.0.0.0:3000")
        .await
}
```

## Routing

### Method Helpers

```rust
use armature_core::micro::*;

App::new()
    .route("/", get(index))
    .route("/users", get(list).post(create))
    .route("/users/:id", get(show).put(update).delete(destroy))
    .route("/any-method", any(catch_all))
```

Available helpers:
- `get(handler)` - GET requests
- `post(handler)` - POST requests
- `put(handler)` - PUT requests
- `delete(handler)` - DELETE requests
- `patch(handler)` - PATCH requests
- `head(handler)` - HEAD requests
- `options(handler)` - OPTIONS requests
- `any(handler)` - All methods

### Chaining Methods

Handle multiple methods on the same path:

```rust
.route("/resource",
    get(read_resource)
        .post(create_resource)
        .put(update_resource)
        .delete(delete_resource)
)
```

### Path Parameters

```rust
async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Extract :id from /users/:id
    let id = req.param("id").unwrap();

    // Multiple params: /users/:user_id/posts/:post_id
    let user_id = req.param("user_id").unwrap();
    let post_id = req.param("post_id").unwrap();

    Ok(HttpResponse::ok())
}

App::new()
    .route("/users/:id", get(get_user))
    .route("/users/:user_id/posts/:post_id", get(get_post))
```

### Query Parameters

```rust
async fn search(req: HttpRequest) -> Result<HttpResponse, Error> {
    // GET /search?q=rust&page=1
    let query = req.query("q").unwrap_or(&"".to_string());
    let page = req.query("page")
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(1);

    HttpResponse::json(&SearchResults { query, page })
}
```

## Middleware

### Adding Middleware

Middleware wraps handlers and can modify requests/responses:

```rust
App::new()
    .wrap(Logger::default())      // Outermost - runs first
    .wrap(Cors::permissive())     // Runs second
    .wrap(Compress::default())    // Innermost - runs last
    .route("/", get(handler))
```

### Custom Middleware

```rust
use armature_core::micro::*;
use std::pin::Pin;
use std::future::Future;

struct Timing;

impl Middleware for Timing {
    fn call(
        &self,
        req: HttpRequest,
        next: Next,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>> {
        Box::pin(async move {
            let start = std::time::Instant::now();

            // Call next handler in chain
            let mut response = next(req).await?;

            // Add timing header
            response.headers.insert(
                "X-Response-Time".to_string(),
                format!("{}ms", start.elapsed().as_millis()),
            );

            Ok(response)
        })
    }
}

App::new()
    .wrap(Timing)
    .route("/", get(handler))
```

### Middleware Order

Middleware executes in the order added (first added = outermost):

```
Request → Logger → Cors → Compress → Handler
                                         ↓
Response ← Logger ← Cors ← Compress ← Response
```

## State Management

### Sharing State

Use `Data<T>` to share state across handlers:

```rust
use armature_core::micro::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct AppState {
    request_count: AtomicU64,
    db_pool: Pool,
}

async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Access state from request extensions
    // (State is automatically injected)
    Ok(HttpResponse::ok())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let state = AppState {
        request_count: AtomicU64::new(0),
        db_pool: create_pool().await,
    };

    App::new()
        .data(state)  // Register state
        .route("/", get(handler))
        .run("0.0.0.0:8080")
        .await
}
```

### Multiple State Types

```rust
#[derive(Clone)]
struct DbPool { /* ... */ }

#[derive(Clone)]
struct Config {
    api_key: String,
}

App::new()
    .data(DbPool::new())
    .data(Config { api_key: "secret".into() })
    .route("/", get(handler))
```

## Scopes

Group routes under a common prefix:

### Basic Scopes

```rust
App::new()
    .service(
        scope("/api/v1")
            .route("/users", get(list_users).post(create_user))
            .route("/users/:id", get(get_user).delete(delete_user))
            .route("/posts", get(list_posts))
    )
    .route("/health", get(health_check))
```

Routes created:
- `GET /api/v1/users`
- `POST /api/v1/users`
- `GET /api/v1/users/:id`
- `DELETE /api/v1/users/:id`
- `GET /api/v1/posts`
- `GET /health`

### Nested Scopes

```rust
App::new()
    .service(
        scope("/api")
            .service(
                scope("/v1")
                    .route("/users", get(v1_users))
            )
            .service(
                scope("/v2")
                    .route("/users", get(v2_users))
            )
    )
```

Routes:
- `GET /api/v1/users`
- `GET /api/v2/users`

### Scoped Middleware

```rust
App::new()
    .wrap(Logger::default())  // Global middleware
    .service(
        scope("/api")
            .wrap(auth_middleware)  // Only for /api/*
            .route("/users", get(users))
    )
    .route("/public", get(public_page))  // No auth required
```

## Built-in Middleware

### Logger

Logs requests with timing information:

```rust
use armature_core::micro::{Logger, LogFormat};

// Default format
App::new().wrap(Logger::default())

// Custom format
App::new().wrap(Logger::new(LogFormat::Combined))
```

Output:
```
INFO Request completed method=GET path=/users status=200 duration_ms=5
```

### CORS

Cross-Origin Resource Sharing configuration:

```rust
use armature_core::micro::Cors;

// Permissive (allow all)
App::new().wrap(Cors::permissive())

// Custom configuration
App::new().wrap(
    Cors::default()
        .allowed_origins(["https://example.com", "https://app.example.com"])
        .allowed_methods(["GET", "POST", "PUT", "DELETE"])
        .allowed_headers(["Content-Type", "Authorization"])
        .allow_credentials(true)
        .max_age(3600)
)
```

### Compress

Adds compression headers:

```rust
use armature_core::micro::{Compress, CompressionLevel};

App::new().wrap(Compress::default())
App::new().wrap(Compress::new(CompressionLevel::Best))
```

## Configuration

### Default Service (404 Handler)

```rust
async fn not_found(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&serde_json::json!({
        "error": "Not Found",
        "message": "The requested resource does not exist"
    }))
    .map(|r| r.status(404))
}

App::new()
    .route("/", get(index))
    .default_service(not_found)
```

### Building vs Running

```rust
// Build without starting server
let app = App::new()
    .route("/", get(handler))
    .build();

// Handle requests manually (useful for testing)
let response = app.handle(request).await?;

// Or run the server
App::new()
    .route("/", get(handler))
    .run("0.0.0.0:8080")
    .await?;
```

## Best Practices

### 1. Use Scopes for API Versioning

```rust
App::new()
    .service(scope("/api/v1").route("/users", get(v1::users)))
    .service(scope("/api/v2").route("/users", get(v2::users)))
```

### 2. Register Middleware in Correct Order

```rust
App::new()
    .wrap(Logger::default())      // Log all requests (even errors)
    .wrap(Cors::permissive())     // Handle CORS before auth
    .wrap(auth_middleware)        // Auth after CORS preflight
```

### 3. Use Typed State

```rust
// ✅ Good - typed state
#[derive(Clone)]
struct AppState { db: Pool }

// ❌ Avoid - untyped data
.data(HashMap::new())
```

### 4. Handle Errors Properly

```rust
async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id = req.param("id")
        .ok_or_else(|| Error::validation("Missing id parameter"))?;

    let id: u64 = id.parse()
        .map_err(|_| Error::validation("Invalid id format"))?;

    // ...
}
```

### 5. Keep Handlers Small

```rust
// ✅ Good - delegate to service layer
async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let body: CreateUserRequest = req.json()?;
    let user = user_service::create(body).await?;
    HttpResponse::json(&user)
}

// ❌ Avoid - business logic in handler
async fn create_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    // 100+ lines of database calls, validation, etc.
}
```

## When to Use

### Use Micro-Framework When

- ✅ Building microservices
- ✅ Simple REST APIs
- ✅ Quick prototypes
- ✅ Learning Armature
- ✅ Performance-critical services
- ✅ Don't need dependency injection
- ✅ Prefer explicit over implicit

### Use Full Framework When

- ✅ Large enterprise applications
- ✅ Need dependency injection
- ✅ Want decorator-based controllers
- ✅ Complex middleware requirements
- ✅ GraphQL subscriptions
- ✅ Automatic OpenAPI generation

### Comparison

| Aspect | Micro-Framework | Full Framework |
|--------|-----------------|----------------|
| Setup | `App::new()` | `Application::bootstrap(Module)` |
| Routing | `get(handler)` | `@Get()` decorator |
| DI | Manual `Data<T>` | `@Injectable` auto-wiring |
| Middleware | `wrap(mw)` | `@UseGuards`, `@UsePipes` |
| Best for | Microservices | Enterprise apps |

## API Reference

### App

```rust
impl App {
    fn new() -> Self;
    fn data<T: Clone + Send + Sync + 'static>(self, data: T) -> Self;
    fn wrap<M: Middleware + 'static>(self, middleware: M) -> Self;
    fn route(self, path: &str, route: RouteBuilder) -> Self;
    fn service(self, scope: Scope) -> Self;
    fn default_service<H>(self, handler: H) -> Self;
    fn build(self) -> BuiltApp;
    async fn run(self, addr: impl ToSocketAddrs) -> std::io::Result<()>;
}
```

### RouteBuilder

```rust
fn get<H>(handler: H) -> RouteBuilder;
fn post<H>(handler: H) -> RouteBuilder;
fn put<H>(handler: H) -> RouteBuilder;
fn delete<H>(handler: H) -> RouteBuilder;
fn patch<H>(handler: H) -> RouteBuilder;
fn head<H>(handler: H) -> RouteBuilder;
fn options<H>(handler: H) -> RouteBuilder;
fn any<H>(handler: H) -> RouteBuilder;
```

### Scope

```rust
fn scope(prefix: impl Into<String>) -> Scope;

impl Scope {
    fn route(self, path: &str, route: RouteBuilder) -> Self;
    fn wrap<M: Middleware + 'static>(self, middleware: M) -> Self;
    fn service(self, inner: Scope) -> Self;
}
```

### Data

```rust
impl<T> Data<T> {
    fn new(data: T) -> Self;
    fn get_ref(&self) -> &T;
    fn into_inner(self) -> Arc<T>;
}

impl<T> Deref for Data<T> {
    type Target = T;
}
```

## Summary

The micro-framework provides a lightweight, Actix-style API for building web applications:

**Key Points:**
- Use `App::new()` to create applications
- Register routes with `get()`, `post()`, etc.
- Share state with `.data()` and `Data<T>`
- Add middleware with `.wrap()`
- Group routes with `scope()`
- Run with `.run("addr").await`

**Performance:**
- Empty app creation: ~25ns
- Route matching: ~600-900ns
- State access: <1ns

**Best For:**
- Microservices
- Simple APIs
- Learning Armature
- Performance-critical code

For complex applications needing DI and decorators, use the full framework mode instead.

