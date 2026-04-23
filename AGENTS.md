# AGENTS.md

Instructions for AI coding agents working on the Armature framework.

## Project Overview

Armature is a type-safe HTTP framework for Rust inspired by Angular and NestJS. It combines decorator syntax (via proc macros) and dependency injection with Rust's performance and safety. The codebase is a Cargo workspace with 60+ crates.

## Build & Test Commands

```bash
# Build (without SAML — use this by default)
cargo build --features full

# Build (with SAML — requires libxml2-dev, libxmlsec1-dev, libxmlsec1-openssl)
cargo build --features full-with-saml

# Run all tests
cargo test --features full

# Run doc tests only
cargo test --doc --features full

# Run a specific crate's tests
cargo test -p armature-core --features full

# Format check
cargo fmt -- --check

# Lint (allowed warnings match CI config)
cargo clippy --all-targets --features full -- -D warnings \
  -A clippy::collapsible_if \
  -A clippy::result_large_err \
  -A dead_code \
  -A clippy::useless_vec \
  -A clippy::unwrap_or_default

# Run benchmarks
cargo bench
```

**SAML is optional.** Most development uses `--features full` without SAML. Only use `full-with-saml` when working on SAML-related code and you have the system libraries installed.

## Repository Structure

```
armature-framework/          # Workspace root, Cargo.toml defines all members
├── armature-core/           # HTTP routing, middleware, DI container, Application bootstrap
├── armature-proc-macro/     # Procedural macros: #[controller], #[get], #[injectable], #[module]
├── armature-log/            # Structured logging
├── armature-auth/           # JWT, OAuth2, SAML, RBAC, guards
├── armature-jwt/            # JWT token management (HS256/RS256/ES256)
├── armature-security/       # CORS, CSP, HSTS, CSRF
├── armature-config/         # Type-safe config (env, .env, JSON, TOML)
├── armature-cache/          # Redis/Memcached/in-memory caching
├── armature-redis/          # Centralized Redis client
├── armature-queue/          # Background job queues
├── armature-events/         # Event bus (pub/sub)
├── armature-eventsourcing/  # Event sourcing, projections, snapshots
├── armature-cqrs/           # Command/Query Responsibility Segregation
├── armature-graphql/        # GraphQL server (schema-first and code-first)
├── armature-openapi/        # OpenAPI/Swagger generation
├── armature-opentelemetry/  # Distributed tracing (Jaeger, Zipkin, Prometheus)
├── armature-websocket/      # WebSocket with rooms and broadcasting
├── armature-messaging/      # RabbitMQ, Kafka, NATS
├── armature-aws/            # AWS SDK (S3, DynamoDB, SQS, SNS, Lambda, etc.)
├── armature-gcp/            # GCP SDK (Storage, Pub/Sub, Firestore, BigQuery)
├── armature-azure/          # Azure SDK (Blob, Cosmos, Service Bus, Key Vault)
├── armature-lambda/         # AWS Lambda integration
├── armature-cloudrun/       # GCP Cloud Run integration
├── armature-azure-functions/# Azure Functions integration
├── armature-cli/            # Code generation & dev server CLI
├── armature-testing/        # Testing utilities, mocks, spies
├── armature-validation/     # Validation framework
├── armature-ratelimit/      # Rate limiting (token bucket, sliding window)
├── armature-compression/    # gzip/brotli/zstd compression
├── armature-distributed/    # Distributed locks, leader election
├── armature-discovery/      # Service discovery (Consul, etcd)
├── armature-toon/           # Token-optimized serialization for LLMs
├── armature-ferron/         # Custom Rhai scripting engine
├── armature-rhai/           # Embedded Rhai scripting
├── armature-diesel/         # Diesel ORM integration
├── armature-seaorm/         # SeaORM integration
├── armature-storage/        # Cloud file/blob storage
├── armature-mail/           # Email sending
├── armature-push/           # Push notifications
├── armature-payments/       # Payment processing (Stripe, PayPal)
├── armature-admin/          # Auto-generated admin dashboard
├── armature-collab/         # Real-time collaboration (CRDTs)
├── armature-analytics/      # Analytics pipeline
├── armature-siem/           # Security info & event management
├── armature-files/          # File upload/processing
├── armature-tenancy/        # Multi-tenancy
├── armature-features/       # Feature flags
├── armature-opensearch/     # Full-text search
├── armature-i18n/           # Internationalization
├── armature-metrics/        # Prometheus metrics
├── armature-audit/          # Audit logging
├── armature-webhooks/       # Webhook handling
├── armature-cron/           # Scheduled tasks
├── armature-acme/           # Let's Encrypt certificates
├── armature-http-client/    # HTTP client
├── armature-grpc/           # gRPC integration
├── armature-graphql-client/ # GraphQL client
├── armature-app/            # Build full Armature apps in Rhai scripts (zero Rust)
├── armature-macros/         # Additional macros
├── armature-macros-utils/   # Macro utilities
├── docs/                    # 70+ guides
├── examples/                # 60+ working examples
├── benches/                 # Benchmarks (micro, comparison, profiling)
├── tests/                   # Integration tests
└── templates/               # Project scaffolding templates (excluded from workspace)
```

## Architecture Patterns

The framework follows NestJS/Angular conventions adapted to Rust:

- **Decorators** are proc macros: `#[controller]`, `#[get]`, `#[post]`, `#[injectable]`, `#[module]`
- **Dependency injection** is field-based — add a service type as a struct field and it's auto-injected
- **Modules** group providers (services) and controllers with `#[module(...)]`
- **Application bootstrap** via `Application::create::<AppModule>().await`
- **Guards** implement the `Guard` trait for authorization
- **Middleware** implements the `Middleware` trait for request/response pipeline
- **Lifecycle hooks**: `OnModuleInit`, `OnModuleDestroy`, `OnApplicationBootstrap`, `OnApplicationShutdown`

## Key Conventions

- **Rust 2024 edition**, MSRV 1.89
- **Async-first**: Built on Tokio + Hyper. All handlers are `async`
- **Feature flags**: The crate uses feature flags extensively. `full` enables everything except SAML. `full-with-saml` enables everything
- Core types: `HttpRequest`, `HttpResponse`, `Router`, `Container`, `Application`, `Error`
- Error type has 30+ variants with status codes, help text, and client/server classification
- Response builder is fluent: `HttpResponse::ok().json(&data)?`
- Extractors use attribute macros: `#[body]`, `#[param("id")]`, `#[query("page")]`, `#[header("authorization")]`
- Services are singletons — created once, shared via `Arc`

## Git Workflow

- **`main`** — stable release branch, target for PRs
- **`develop`** — active development branch
- Branch naming: `feature/*`, `bugfix/*`
- CI runs on push to `main`/`develop` and on all PRs
- CI checks: format, clippy, tests (Linux/macOS/Windows, stable/beta/nightly), doc tests, example builds

## Performance Notes

- Target: Actix-competitive performance (currently 242k req/sec plaintext)
- JSON serialization is a known optimization area
- Benchmark suite in `benches/` covers micro, comparison, and profiling scenarios
- Do not regress performance without justification — run `cargo bench` before and after changes

## When Making Changes

1. Run `cargo fmt` before committing
2. Run clippy with the CI flags shown above — do not introduce new warnings
3. Run `cargo test --features full` to validate
4. If adding a new crate, add it to the workspace `members` in root `Cargo.toml`
5. If adding public API, add doc comments and a doc test
6. If adding a new feature, add an example in `examples/` and a guide in `docs/`
7. Keep the NestJS/Angular decorator-style patterns consistent — don't introduce foreign paradigms
