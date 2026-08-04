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

# Lint the per-crate benchmarks. The command above is scoped to the root
# package, which no longer owns them, so it does NOT cover these.
#
# ONE CRATE AT A TIME: cargo unifies a dependency's features across every
# package in a single `-p` selection, so a combined run lets one crate's
# criterion features satisfy a sibling whose manifest omits them. These are
# separately published crates and each must build from its own manifest alone.
for c in $(cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select([.targets[].kind[]] | index("bench")) | .name' \
    | grep -v '^armature-framework$'); do
  cargo clippy --benches -p "$c" -- -D warnings
done

# Run benchmarks (each one is owned by the crate it measures)
./scripts/run-benchmarks.sh --all
cargo bench -p armature-core --bench internal_overhead
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
├── armature-security/       # CORS, CSP, HSTS
├── armature-config/         # Type-safe config (env, .env, JSON, TOML)
├── armature-cache/          # Redis/Memcached/in-memory caching
├── armature-redis/          # Centralized Redis client
├── armature-queue/          # Background job queues
├── armature-events/         # Event bus (pub/sub)
├── armature-eventsourcing/  # Event sourcing, projections, snapshots
├── armature-cqrs/           # Command/Query Responsibility Segregation
├── armature-graphql/        # GraphQL server (schema-first and code-first)
├── armature-openapi/        # OpenAPI/Swagger generation
├── armature-opentelemetry/  # Distributed tracing (OTLP, Zipkin), metrics
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
├── benches/                 # Cross-framework comparison harness (comparison_servers/, techempower/,
│                           # http-benchmark) + the database/memory pattern benches. Per-crate
│                           # criterion benches live in each crate's own benches/.
├── tests/                   # Integration tests
└── templates/               # Project scaffolding templates (excluded from workspace)
```

## Architecture Patterns

The framework follows NestJS/Angular conventions adapted to Rust:

- **Decorators** are proc macros: `#[controller]`, `#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[patch]`, `#[options]`, `#[head]`, `#[query]`, `#[injectable]`, `#[module]`
- **HTTP methods**: `HttpMethod` includes `QUERY` (IETF safe-method-with-body). It is `#[non_exhaustive]` — always include a `_` arm when matching it.
- **Dependency injection** is field-based — add a service type as a struct field and it's auto-injected
- **Modules** group providers (services) and controllers with `#[module(...)]`
- **Application bootstrap** via `Application::create::<AppModule>().await`
- **Guards** implement the `Guard` trait for authorization. They **fail closed**: a `RoleGuard`/`PermissionGuard` requires a verified `UserContext`/`RequestRoles` extension attached by an authentication layer (use `armature_auth::JwtAuthMiddleware`, which verifies the JWT and populates them). A module's guards are scoped to that module's controllers' routes, not applied globally.
- **Middleware** implements the `Middleware` trait for request/response pipeline
- **Lifecycle hooks**: `OnModuleInit`, `OnModuleDestroy`, `OnApplicationBootstrap`, `OnApplicationShutdown`
- **Routing**: the linear `Router` is the registration target; it is compiled once into an O(1) `OptimizedRouter` for the serve path. Preserve exact routing semantics (param extraction, catch-all, constraints, unknown-method → 404) when touching either.

## Key Conventions

- **Rust 2024 edition**, MSRV 1.94.1
- **Async-first**: Built on Tokio + Hyper. All handlers are `async`
- **Feature flags**: The crate uses feature flags extensively. `full` enables everything except SAML. `full-with-saml` enables everything
- **`tokio` features**: crates declare the minimal per-crate feature subset they use (e.g. `["rt", "macros", "sync", "time"]`, plus `net`/`io-util`/`fs` as needed) — do **not** use `features = ["full"]`
- **TLS is rustls-only**: do not pull `native-tls`/OpenSSL. Add `default-features = false` to `reqwest`/`tokio-tungstenite` deps and select the rustls feature
- `HttpRequest.headers` is a `HeaderMap` (SmallVec-backed, case-insensitive); it is a drop-in for the old `HashMap<String,String>` access patterns
- Core types: `HttpRequest`, `HttpResponse`, `Router`, `Container`, `Application`, `Error`
- Error type has 30+ variants with status codes, help text, and client/server classification
- Response builder is fluent: `HttpResponse::ok().json(&data)?`
- Extractors use attribute macros: `#[body]`, `#[param("id")]`, `#[query("page")]`, `#[header("authorization")]`
- Services are singletons — created once, shared via `Arc`
- **Changelogs are per-crate**: each crate keeps its own `CHANGELOG.md` (e.g. `armature-core/CHANGELOG.md`) in [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format with an `## [Unreleased]` section at the top. Write new entries there — the root `CHANGELOG.md` is the historical record through `0.3.0` plus workspace-wide notes, not a place for new per-crate entries. A crate with no `include`/`exclude` in its `Cargo.toml` packages its `CHANGELOG.md` automatically; no manifest change needed

## Git Workflow

- **`main`** — stable release branch, target for PRs
- **`develop`** — active development branch
- Branch naming: `feature/*`, `bugfix/*`
- CI runs on push to `main`/`develop` and on all PRs
- CI checks: format, clippy, tests (Linux/macOS/Windows, stable/beta/nightly), doc tests, example builds

## Performance Notes

- Target: Actix-competitive performance (currently 242k req/sec plaintext)
- JSON serialization is a known optimization area
- Criterion benchmarks live in the crate they measure (`armature-core/benches/`, `armature-jwt/benches/`, ...), so they are always run `-p`-scoped: `cargo bench -p armature-core --bench internal_overhead`. `scripts/run-benchmarks.sh` runs them by suite. Those crates set `autobenches = false`, so a new bench file needs an explicit `[[bench]]` entry
- The root `benches/` keeps only what is not crate-specific: cross-framework HTTP comparison (`benches/comparison_servers/` + the `http-benchmark` runner, `benches/techempower/`) and the `database_benchmarks`/`memory_benchmarks` pattern benchmarks. Profiling (flamegraphs, DHAT/pprof) is not in `benches/` — it lives in `examples/profiling_server.rs`, `examples/memory_profile_server.rs`, and `scripts/memory-profile.sh`
- Do not regress performance without justification — run the relevant benchmarks before and after changes

## When Making Changes

1. Run `cargo fmt` before committing
2. Run clippy with the CI flags shown above — do not introduce new warnings
3. Run `cargo test --features full` to validate
4. If adding a new crate, add it to the workspace `members` in root `Cargo.toml`
5. If adding public API, add doc comments and a doc test; if bumping a crate's version, record the change in that crate's own `CHANGELOG.md`
6. If adding a new feature, add an example in `examples/` and a guide in `docs/`
7. Keep the NestJS/Angular decorator-style patterns consistent — don't introduce foreign paradigms
