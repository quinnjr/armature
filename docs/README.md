# Armature Documentation

Welcome to the Armature framework documentation! Armature is a batteries-included, enterprise-grade web framework for Rust, inspired by NestJS and Angular.

## Framework Overview

Armature provides everything you need to build production-ready APIs:

- ✅ **99% Feature Complete** - Enterprise-ready with 150+ features implemented
- 🚀 **Actix-Competitive Performance** - 112 optimizations, SIMD JSON, zero-alloc responses
- 🔒 **Type-Safe** - Catch errors at compile time
- 💉 **Dependency Injection** - Automatic service injection
- 📦 **Modular Architecture** - Organize code into reusable modules
- 🔐 **Built-in Security** - JWT, OAuth2, SAML, 2FA, rate limiting
- 📊 **Observability** - OpenTelemetry, Prometheus, structured logging
- ☁️ **Cloud Native** - AWS, GCP, Azure SDKs with serverless support

## Getting Started

Start with the main [README](../README.md) in the project root for a quick introduction and setup guide.

```bash
# Install the CLI
cargo install armature-cli

# Create a new project
armature new my-api
cd my-api

# Start the dev server
armature dev
```

## Documentation Index

### Core Guides

| Guide | Description |
|-------|-------------|
| [Dependency Injection](di-guide.md) | Service injection, module system, best practices |
| [Configuration](config-guide.md) | Environment variables, type-safe config, validation |
| [Lifecycle Hooks](lifecycle-hooks.md) | OnInit, OnDestroy, module lifecycle |
| [Project Templates](project-templates.md) | Starter templates and scaffolding |
| [Macros Overview](macro-overview.md) | Decorator macros and code generation |

### Authentication & Security

| Guide | Description |
|-------|-------------|
| [Authentication](auth-guide.md) | Password hashing, JWT, guards, RBAC |
| [OAuth2 Providers](oauth2-providers-guide.md) | Google, Microsoft, GitHub, Discord, and more |
| [Security Best Practices](security-guide.md) | CORS, CSP, HSTS, request signing |
| [Advanced Security](security-advanced-guide.md) | 2FA, WebAuthn, API keys |
| [Session Management](session-guide.md) | Redis-backed sessions, cookies |
| [Rate Limiting](rate-limiting-guide.md) | Token bucket, sliding window algorithms |

### Routing & Controllers

| Guide | Description |
|-------|-------------|
| [Route Groups](route-groups-guide.md) | Organizing routes with shared middleware |
| [Route Constraints](route-constraints-guide.md) | Parameter validation at route level |
| [Guards](use-guard-guide.md) | Authorization and access control |
| [Middleware](use-middleware-guide.md) | Request/response middleware |
| [Guards & Interceptors](guards-interceptors.md) | Cross-cutting concerns |
| [Request Extractors](request-extractors.md) | Body, Query, Path, Header extractors |

### API Features

| Guide | Description |
|-------|-------------|
| [API Versioning](api-versioning-guide.md) | URL, header, and query-based versioning |
| [Content Negotiation](content-negotiation-guide.md) | Accept header handling |
| [Pagination & Filtering](pagination-filtering-guide.md) | Offset/cursor pagination, sorting |
| [Response Caching](response-caching-guide.md) | Cache-Control, ETags |
| [ETags & Conditional Requests](etag-conditional-requests-guide.md) | If-Match, If-None-Match |
| [Request Timeouts](request-timeouts-guide.md) | Configurable timeouts |
| [Streaming Responses](streaming-responses-guide.md) | Chunked transfer, large files |

### GraphQL & OpenAPI

| Guide | Description |
|-------|-------------|
| [GraphQL Guide](graphql-guide.md) | Schema-first and code-first GraphQL |
| [GraphQL Configuration](graphql-configuration.md) | Advanced GraphQL options |
| [OpenAPI/Swagger](openapi-guide.md) | Auto-generated API documentation |

### Real-Time Communication

| Guide | Description |
|-------|-------------|
| [WebSocket & SSE](websocket-sse-guide.md) | Real-time bidirectional communication |
| [Webhooks](webhooks.md) | Webhook sending and receiving |

### Background Processing

| Guide | Description |
|-------|-------------|
| [Job Queues](queue-guide.md) | Redis-backed background jobs |
| [Cron Jobs](cron-guide.md) | Scheduled tasks |
| [Graceful Shutdown](graceful-shutdown-guide.md) | Connection draining, cleanup hooks |

### Caching

| Guide | Description |
|-------|-------------|
| [Caching Strategies](cache-improvements-guide.md) | Multi-tier caching, tag invalidation |
| [Redis Integration](redis-guide.md) | Centralized Redis client |

### Observability

| Guide | Description |
|-------|-------------|
| [Structured Logging](logging-guide.md) | JSON logging, pretty printing, env config |
| [Debug Logging](debug-logging-guide.md) | Development logging |
| [OpenTelemetry](opentelemetry-guide.md) | Distributed tracing and metrics |
| [Prometheus Metrics](metrics-guide.md) | Custom metrics, /metrics endpoint |
| [Health Checks](health-check-guide.md) | Liveness, readiness, startup probes |
| [Error Correlation](error-correlation-guide.md) | Request ID tracking |
| [Audit Logging](audit-guide.md) | Who did what, when |

### Database

| Guide | Description |
|-------|-------------|
| [Diesel Integration](diesel-guide.md) | Async Diesel with connection pooling |
| [SeaORM Integration](seaorm-guide.md) | SeaORM with active record pattern |

### Data & Search

| Guide | Description |
|-------|-------------|
| [OpenSearch](opensearch-guide.md) | Full-text search, indexing, aggregations |

### Internationalization

| Guide | Description |
|-------|-------------|
| [i18n Guide](i18n-guide.md) | Translations, locale detection, pluralization, formatting |

### Serialization & AI

| Guide | Description |
|-------|-------------|
| [TOON Format](toon-guide.md) | Token-optimized serialization for LLM applications |

### Cloud Providers

| Guide | Description |
|-------|-------------|
| [Cloud Providers](cloud-providers-guide.md) | AWS, GCP, Azure SDK integration |

Armature provides first-class integrations with major cloud providers:

| Crate | Provider | Services |
|-------|----------|----------|
| **armature-aws** | Amazon Web Services | S3, DynamoDB, SQS, SNS, SES, Lambda, KMS, Cognito |
| **armature-gcp** | Google Cloud Platform | Storage, Pub/Sub, Firestore, Spanner, BigQuery |
| **armature-azure** | Microsoft Azure | Blob, Queue, Cosmos, Service Bus, Key Vault |

**Key Features:**
- 🔌 **Dynamic Loading** - Only compile services you need via feature flags
- 💉 **DI Integration** - Register once, inject everywhere
- ⚡ **Lazy Initialization** - Services created on first access
- 🔧 **Environment Config** - Reads from standard cloud environment variables
- 🧪 **Emulator Support** - LocalStack, GCP emulators, Azurite

### Networking & HTTP

| Guide | Description |
|-------|-------------|
| [HTTPS & TLS](https-guide.md) | TLS configuration |
| [ACME Certificates](acme-certificates.md) | Let's Encrypt auto-renewal |
| [Compression](compression.md) | Gzip, Brotli compression |
| [HTTP Status & Errors](http-status-errors.md) | Error handling |
| [Error Transformation](error-transformation-guide.md) | Custom error formatting |

### Testing

| Guide | Description |
|-------|-------------|
| [Testing Guide](testing-guide.md) | Unit, integration, e2e testing |
| [Test Coverage](testing-coverage.md) | Coverage reporting |
| [Testing Best Practices](testing-documentation.md) | Testing patterns |
| [Documentation Testing](documentation-testing.md) | Doc example testing |
| [Fuzzing Guide](fuzzing-guide.md) | Fuzz testing with cargo-fuzz |

### Architecture

| Guide | Description |
|-------|-------------|
| [Stateless Architecture](stateless-architecture.md) | Building scalable services |
| [Server Integration](server-integration.md) | Hyper, custom servers |
| [Macros Deep Dive](macros-guide.md) | Understanding Armature macros |

### Performance & Benchmarks

| Guide | Description |
|-------|-------------|
| [Performance Guide](performance-guide.md) | Optimization techniques, best practices, profiling |
| [vs Node.js](armature-vs-nodejs-benchmark.md) | Performance comparison with Express, NestJS |
| [vs Next.js](armature-vs-nextjs-benchmark.md) | Performance comparison with Next.js |

### Development & Contributing

| Guide | Description |
|-------|-------------|
| [Publishing to crates.io](publishing-guide.md) | Publishing workspace crates |

## Quick Example

```rust
use armature_framework::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct User { id: u32, name: String }

// Injectable service
#[injectable]
#[derive(Default, Clone)]
struct UserService;

impl UserService {
    fn find_by_id(&self, id: u32) -> Option<User> {
        Some(User { id, name: "Alice".into() })
    }
}

// Controller with routes
#[controller("/api/users")]
#[derive(Default, Clone)]
struct UserController;

impl UserController {
    #[get("")]
    async fn list() -> Result<Json<Vec<User>>, Error> {
        Ok(Json(vec![User { id: 1, name: "Alice".into() }]))
    }

    #[get("/:id")]
    async fn get_user(req: HttpRequest) -> Result<Json<User>, Error> {
        let id: u32 = req.param("id").unwrap().parse().unwrap();
        Ok(Json(User { id, name: "Alice".into() }))
    }
}

// Module wires everything together
#[module(
    providers: [UserService],
    controllers: [UserController]
)]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() {
    let app = Application::create::<AppModule>().await;
    app.listen(3000).await.unwrap();
}
```

## Examples

See the [examples directory](../examples/) for working code samples:

| Example | Description |
|---------|-------------|
| `crud_api.rs` | Complete REST API with CRUD operations |
| `auth_api.rs` | JWT authentication flow |
| `realtime_api.rs` | WebSocket/SSE real-time communication |
| `dependency_injection.rs` | DI patterns and best practices |
| `websocket_chat.rs` | WebSocket chat room |
| `server_sent_events.rs` | SSE streaming |

## Key Concepts

### Dependency Injection

Armature provides automatic service injection based on field types:

```rust
#[injectable]
#[derive(Default, Clone)]
struct UserService { }

#[controller("/users")]
#[derive(Default, Clone)]
struct UserController {
    user_service: UserService,  // Auto-injected!
}
```

### Module System

Organize your application into modules:

```rust
#[module(
    providers: [UserService, EmailService],
    controllers: [UserController],
    imports: [AuthModule, CacheModule]
)]
#[derive(Default)]
struct AppModule;
```

### Cloud Integration

Multi-cloud support with DI:

```rust
#[module_impl]
impl CloudModule {
    #[provider(singleton)]
    async fn aws() -> Arc<AwsServices> {
        AwsServices::new(AwsConfig::from_env().enable_s3().build()).await.unwrap()
    }

    #[provider(singleton)]
    async fn redis() -> Arc<RedisService> {
        Arc::new(RedisService::new(RedisConfig::from_env().build()).await.unwrap())
    }
}
```

## Documentation Conventions

- **Lowercase with hyphens**: `my-feature-guide.md`
- **Descriptive names**: `oauth2-providers-guide.md` not `oauth.md`
- **.md extension** for all Markdown files

## Version

This documentation is for Armature version 0.1.0.

---

For the latest updates, visit the [GitHub repository](https://github.com/pegasusheavy/armature).
