# Armature vs Actix vs Axum

Comprehensive performance benchmarks and feature comparison between Armature's micro-framework and the leading Rust web frameworks.

## Benchmark Methodology

- **Tool:** `oha` HTTP load testing
- **Requests:** 50,000 per test
- **Concurrency:** 100 concurrent connections
- **Hardware:** Same machine, release builds
- **Rust:** 1.88 (2024 edition)

## Performance Results

### Plaintext (Hello World)

Simple "Hello, World!" response - tests raw request handling overhead.

| Framework | Requests/sec | Avg Latency | p99 Latency |
|-----------|-------------|-------------|-------------|
| **🥇 Armature** | **242,823** | **0.40ms** | **2.62ms** |
| 🥈 Actix-web | 144,069 | 0.53ms | 9.98ms |
| 🥉 Axum | 46,127 | 2.09ms | 29.58ms |

**Winner: Armature** - 1.7x faster than Actix, 5.3x faster than Axum.

### JSON Serialization

JSON response with a simple message object - tests serialization performance.

| Framework | Requests/sec | Avg Latency | p99 Latency |
|-----------|-------------|-------------|-------------|
| **🥇 Axum** | **239,594** | **0.40ms** | **1.91ms** |
| 🥈 Actix-web | 128,004 | 0.67ms | 16.95ms |
| 🥉 Armature | 35,622 | 2.65ms | 32.85ms |

**Note:** Armature's JSON serialization has optimization opportunities.

### Path Parameters (/users/:id)

Route with path parameter extraction - tests routing + extraction.

| Framework | Requests/sec | Avg Latency | p99 Latency |
|-----------|-------------|-------------|-------------|
| **🥇 Actix-web** | **183,781** | **0.44ms** | **10.00ms** |
| 🥈 Armature | 59,077 | 1.51ms | 15.79ms |
| 🥉 Axum | 38,549 | 2.47ms | 28.28ms |

## Feature Comparison

| Feature | Armature | Actix-web | Axum |
|---------|----------|-----------|------|
| HTTP/2 | ✅ | ✅ | ✅ |
| HTTP/3 (QUIC) | ✅ | ❌ | ❌ |
| Built-in DI | ✅ | ❌ | ❌ |
| Decorator Syntax | ✅ | ❌ | ❌ |
| Micro-framework Mode | ✅ | ✅ | ✅ |
| OpenAPI Generation | ✅ | 🔶 | 🔶 |
| Admin Generator | ✅ | ❌ | ❌ |
| CLI Tooling | ✅ | ❌ | ❌ |
| WebSocket | ✅ | ✅ | ✅ |
| GraphQL | ✅ | ✅ | ✅ |
| Payment Processing | ✅ | ❌ | ❌ |

✅ = Built-in | 🔶 = Via plugin | ❌ = Not available

## API Style Comparison

### Armature Micro-Framework

```rust
use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};

async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id = req.param("id").cloned().unwrap_or_default();
    HttpResponse::json(&User { id, name: "Alice".into() })
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .wrap(Logger::default())
        .route("/users/:id", get(get_user))
        .run("0.0.0.0:8080")
        .await
}
```

### Actix-web

```rust
use actix_web::{get, web, App, HttpResponse, HttpServer};

#[get("/users/{id}")]
async fn get_user(path: web::Path<String>) -> HttpResponse {
    let id = path.into_inner();
    HttpResponse::Ok().json(User { id, name: "Alice".into() })
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| App::new().service(get_user))
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
```

### Axum

```rust
use axum::{extract::Path, routing::get, Json, Router};

async fn get_user(Path(id): Path<String>) -> Json<User> {
    Json(User { id, name: "Alice".into() })
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/users/:id", get(get_user));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

## When to Choose Each Framework

### Choose Armature when:

- You need built-in DI and decorator syntax (NestJS-style)
- You want enterprise features (OpenAPI, Admin, Payments)
- You need HTTP/3 support
- You prefer batteries-included frameworks
- You're coming from TypeScript/NestJS

### Choose Actix-web when:

- Maximum raw performance is critical
- You need actor-based concurrency
- You prefer minimal abstractions
- Large community and ecosystem matter

### Choose Axum when:

- You want tight Tokio integration
- Tower middleware compatibility is important
- You prefer function-based handlers
- Type-driven API design is your style

## Run Your Own Benchmarks

```bash
# Install oha
cargo install oha

# Clone Armature
git clone https://github.com/pegasusheavy/armature
cd armature

# Build servers
cargo build --release --example micro_benchmark_server
cd benches/comparison_servers/actix_server && cargo build --release && cd -
cd benches/comparison_servers/axum_server && cargo build --release && cd -

# Run benchmarks
./scripts/benchmark-comparison.sh
```

## Summary

Armature's micro-framework delivers:

- **Competitive performance** with Actix and Axum
- **Richer feature set** with built-in DI, OpenAPI, and enterprise modules
- **Familiar API** for developers coming from NestJS/Express
- **Future-ready** with HTTP/3 support

The framework excels on plaintext throughput but has identified optimization opportunities for JSON serialization that will be addressed in upcoming releases.

