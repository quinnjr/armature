# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

*No unreleased changes.*

---

## [0.2.0] - 2026-02-02

Major release featuring Rust 2024 edition upgrade, new application builder, enhanced CLI, and HTTP handler improvements.

### Added

#### HTTP Handler Enhancements (`armature-core`, `armature-proc-macro`)
- `#[options]` proc macro attribute for custom OPTIONS route handlers
- `#[head]` proc macro attribute for HEAD request handlers
- `Router::options()` and `Router::head()` fluent methods for programmatic routing
- Full support for CORS preflight and resource metadata checks

#### Application Builder (`armature-app`)
- New `armature-app` crate with Rhai scripting support
- Declarative application configuration via Rhai scripts
- Dynamic route registration and middleware configuration
- Hot-reload support for development

#### CLI Enhancements (`armature-cli`)
- Prax ORM support for code generation
- Comprehensive code generation templates
- Improved project scaffolding

#### Messaging (`armature-messaging`)
- MQ-Bridge integration for unified messaging across brokers
- Support for RabbitMQ, Kafka, NATS, and Redis Streams

#### Security
- CodeQL security analysis workflow for automated vulnerability scanning

### Changed

- **Rust 2024 Edition** - Upgraded entire workspace to Rust 2024 edition
- **MSRV** - Minimum supported Rust version updated to 1.89
- Converted let-chains for Rust 2024 compatibility
- Various dependency updates for compatibility

### Fixed

- Fixed clippy warnings for Rust 2024 edition compatibility
- Fixed MSRV-related compilation issues

---

## [0.1.0] - 2025-12-21

Initial public release of the Armature framework - a high-performance, type-safe HTTP framework for Rust inspired by Angular and NestJS.

### Added

#### Logging (`armature-log`)
- JSON and Pretty logging formats with environment variable configuration
- `ARMATURE_DEBUG`, `ARMATURE_LOG_LEVEL`, `ARMATURE_LOG_FORMAT` controls
- `trace!`, `debug!`, `info!`, `warn!`, `error!` macros
- Optional tracing integration
- Runtime-configurable log levels and formats

#### Internationalization (`armature-i18n`)
- Message translation with Fluent syntax
- Locale detection from Accept-Language headers
- CLDR-compliant pluralization rules
- Date, number, and currency formatting

#### Database Integration
- **`armature-diesel`** - Diesel async with connection pooling
- **`armature-seaorm`** - SeaORM integration with active record pattern

#### Search (`armature-opensearch`)
- OpenSearch/Elasticsearch client
- Document management and bulk operations
- Query DSL builder

#### Serialization (`armature-toon`)
- TOON (Token-Oriented Object Notation) support for LLM-optimized serialization

#### Compression (`armature-compression`)
- Streaming compression (gzip, brotli, zstd)
- Backpressure handling
- Response compression middleware

#### Fuzzing (`armature-fuzz`)
- 8 fuzz targets for security testing
- HTTP request/response, routing, JSON, URL parsing

#### Performance Optimizations
- 65+ performance optimizations implemented
- SIMD HTTP parsing and JSON serialization
- Zero-copy request/response handling
- Arena allocators for per-request batch allocations
- HTTP/1.1 pipelining and request batching
- `io_uring` backend for Linux
- Connection pooling and keep-alive optimization
- Thread-local buffer pools
- `matchit` router for O(log n) routing
- SmallVec headers and CompactString paths

#### Publishing Tools
- `scripts/publish.sh` - Automated crates.io publishing with rate limiting
- `scripts/prepare-publish.sh` - Path-to-version dependency conversion
- `scripts/pgo-build.sh` - Profile-Guided Optimization workflow

#### Cloud Provider SDKs
- **`armature-aws`** - AWS SDK integration with feature-gated services
  - S3, DynamoDB, SQS, SNS, SES, Lambda, Secrets Manager, KMS, Cognito
  - Dynamic service loading via feature flags
  - DI container integration
  - Environment-based configuration
  - LocalStack emulator support
- **`armature-gcp`** - Google Cloud SDK integration
  - Cloud Storage, Pub/Sub, Firestore, Spanner, BigQuery
  - Feature-gated compilation
  - GCP emulator support
- **`armature-azure`** - Azure SDK integration
  - Blob Storage, Queue Storage, Cosmos DB, Service Bus, Key Vault
  - Azurite emulator support

#### Serverless Deployment
- **`armature-lambda`** - AWS Lambda runtime for Armature
  - API Gateway, ALB, and Function URL support
  - Request/response conversion
  - Cold start optimization
  - Lambda-specific Dockerfile templates
- **`armature-cloudrun`** - Google Cloud Run deployment
  - Health check utilities
  - Cloud Logging integration
  - Graceful shutdown support
  - Cloud Build configuration
- **`armature-azure-functions`** - Azure Functions custom handler
  - HTTP trigger support
  - Request/response bindings
  - Azure Container Apps deployment

#### Redis Integration
- **`armature-redis`** - Centralized Redis client crate
  - Connection pooling with bb8
  - Pub/Sub messaging support
  - Cluster, TLS, and Sentinel support
  - Shared across all crates (cache, queue, distributed, ratelimit, session)

#### HTTP Client
- **`armature-http-client`** - Production-ready HTTP client
  - Automatic retry with configurable backoff (constant, linear, exponential, jitter)
  - Circuit breaker integration
  - Request/response interceptors
  - Middleware chain support
  - Timeout policies

#### gRPC Support
- **`armature-grpc`** - gRPC server and client
  - Tonic-based implementation
  - Interceptors for auth and metrics
  - Health checking and reflection
  - Type aliases for complex signatures

#### GraphQL Client
- **`armature-graphql-client`** - GraphQL client for federation
  - Query batching
  - Subscription support via WebSocket
  - Automatic retry
  - Variables and fragments

#### Email System
- **`armature-mail`** - Comprehensive email module
  - SMTP transport with TLS/STARTTLS
  - Provider integrations: SendGrid, AWS SES, Mailgun
  - Handlebars email templates
  - Attachment support (inline and download)
  - Email queue with async sending, retries, and dead letter queue
  - Redis-backed queue storage

#### Push Notifications
- **`armature-push`** - Multi-platform push notifications
  - Web Push with VAPID
  - Firebase Cloud Messaging (FCM)
  - Apple Push Notification Service (APNS)
  - Unified push service API
  - Batch sending support
  - Device token management

#### File Storage
- **`armature-storage`** - File storage abstraction
  - Local filesystem storage
  - AWS S3 with presigned URLs and server-side encryption
  - Google Cloud Storage with signed URLs
  - Azure Blob Storage with Azurite support
  - Multipart upload handling with streaming
  - File validation (type, size, extension)

#### Resilience Patterns
- **`armature-core/resilience`** - Production resilience patterns
  - Circuit Breaker (Open/Closed/Half-Open states, sliding window)
  - Retry with Backoff (constant, linear, exponential, jitter)
  - Bulkhead (semaphore-based concurrency limiting)
  - Timeout policies
  - Fallback handlers with chains

#### CLI Enhancements
- Interactive project creation wizard
- `armature add <feature>` - Add features to existing projects
- `armature check` - Validate project configuration
- `armature routes` - List all registered routes
- `armature config:check` - Validate configuration files
- Shell completions (bash, zsh, fish, PowerShell)
- Improved colored output and progress indicators

#### Developer Experience
- Prelude modules added to all major crates for easier imports
- `Result<T>` type aliases in crates with Error types
- Convenience methods on `HttpResponse` (ok, created, no_content, bad_request, etc.)
- Convenience methods on `Container` (require, get_or_default, register_if_missing)
- Enhanced error messages with actionable suggestions
- Debug and Display implementations for all public types

#### Cookbook Examples
- `examples/crud_api.rs` - Complete REST API with CRUD operations
- `examples/auth_api.rs` - JWT authentication flow
- `examples/realtime_api.rs` - WebSocket/SSE real-time communication

#### Benchmarks
- `benches/resilience_benchmarks.rs` - Circuit breaker, retry, bulkhead, timeout
- `benches/cache_benchmarks.rs` - Cache operations and tiered caching
- `benches/auth_benchmarks.rs` - Password hashing and JWT operations
- `benches/ratelimit_benchmarks.rs` - Rate limiting algorithms
- `benches/storage_benchmarks.rs` - File validation and storage operations
- `benches/http_client_benchmarks.rs` - HTTP client patterns

#### DevOps Templates
- **Dockerfile templates** (Alpine-based, multi-stage, cargo-chef)
  - `templates/api-minimal/Dockerfile`
  - `templates/api-full/Dockerfile`
  - `templates/microservice/Dockerfile`
  - `templates/graphql-api/Dockerfile`
  - `templates/lambda/Dockerfile` (x86_64 and ARM64)
  - `templates/cloudrun/Dockerfile`
  - `templates/azure-container/Dockerfile`
- **Docker Compose** for all templates with development services
- **Kubernetes manifests** (`templates/k8s/`)
  - Deployment, Service, Ingress
  - HPA, PDB, NetworkPolicy
  - ConfigMap, Secret, ServiceAccount
  - Kustomization base
- **Helm chart** (`templates/helm/armature/`)
  - Production-ready values
  - Configurable replicas, resources, probes
  - Ingress and service configuration
- **CI/CD workflows**
  - GitHub Actions (CI, Release, Docs, PR automation)
  - Jenkins pipelines (basic, Docker agent, multibranch)

#### Documentation
- `docs/cloud-providers-guide.md` - AWS, GCP, Azure SDK usage
- `docs/redis-guide.md` - Centralized Redis integration
- `docs/dependency-injection-guide.md` - Advanced DI patterns
- Updated `docs/README.md` with comprehensive documentation index
- Angular-based docs overview component with proper routing

#### SEO & AI SEO
- Comprehensive `index.html` meta tags
  - Open Graph and Twitter Card tags
  - JSON-LD schemas (SoftwareApplication, Organization, WebSite, FAQPage, BreadcrumbList)
- `robots.txt` with 15+ AI crawler rules (GPTBot, Claude, Bingbot, etc.)
- `sitemap.xml` expanded to 35+ URLs
- `llms.txt` - AI-readable project summary (llmstxt.org standard)
- `ai.txt` - AI interaction guidelines and code generation style
- `.well-known/security.txt` - Security vulnerability reporting
- `humans.txt` - Team credits and technology stack

### Changed

- **Web app theme**: Migrated to Tailswatch oxide dark theme
- **Documentation structure**: Flattened docs/ directory (removed guides/ subfolder)
- **Mobile navigation**: Fixed menu collapse behavior
- **Comparisons page**: Refactored to emphasize Armature strengths
- **Roadmap**: Updated to show 98% feature completion
- Updated all URLs from `quinnjr.github.io` to `pegasusheavy.github.io`
- Renamed builder methods to use `with_*` pattern for better ergonomics
- Updated `tonic` to 0.14, `prost` to 0.14
- Updated `redis` to 1.0, `bb8-redis` to 0.18
- Updated `lambda_http` and `lambda_runtime` to 1.0
- Updated `web-push` to 0.11

### Fixed

- Fixed `HealthChecker` trait for object safety (dyn compatibility)
- Fixed `lambda_http` and `aws_lambda_events` API compatibility
- Fixed `web_push` crate API changes
- Fixed `tonic` gRPC framework API changes
- Fixed benchmark compilation errors
- Fixed clippy warnings across all crates
- Removed all `unsafe` code blocks (replaced with safe alternatives)
- Fixed mobile menu not closing when navigation link clicked

### Removed

- **`armature-di`** crate - Use `dependency-injector` crate directly
- Removed `unsafe impl Send/Sync` blocks (now compiler-verified)
- Removed `unsafe env::set_var` from tests

---

## Rate Limiting Module (`armature-ratelimit`)
- New `armature-ratelimit` crate for comprehensive API rate limiting
- **Algorithms**:
  - Token Bucket - smooth rate limiting with burst capacity
  - Sliding Window Log - precise rate limiting with timestamp tracking
  - Fixed Window - simple fixed time window counters
- **Storage Backends**:
  - `MemoryStore` - in-memory storage using DashMap (default)
  - `RedisStore` - Redis-backed distributed storage (optional `redis` feature)
- **Key Extraction**:
  - By IP address, user ID, API key, or custom headers
  - `KeyExtractorBuilder` for complex extraction logic
  - Per-endpoint rate limiting with `IpAndPath` extractor
- **Middleware**:
  - `RateLimitMiddleware` ready for HTTP integration
  - Standard headers: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`, `Retry-After`
  - Bypass keys for whitelisting specific clients
  - Fail-open mode for high availability
- Rate limiting example (`examples/rate_limiting.rs`)
- Comprehensive documentation (`docs/rate-limiting-guide.md`)

#### Armature CLI (`armature-cli`)
- New `armature-cli` crate for code generation and development tools
- **Commands**:
  - `armature new <name>` - Create new projects from templates (minimal, full, microservice)
  - `armature generate controller <name>` - Generate controllers with optional CRUD
  - `armature generate service <name>` - Generate injectable services
  - `armature generate module <name>` - Generate modules with controllers and providers
  - `armature generate middleware <name>` - Generate middleware
  - `armature generate guard <name>` - Generate route guards
  - `armature generate resource <name>` - Generate complete resource (controller + service + module)
  - `armature dev` - Development server with file watching and hot reloading
  - `armature build` - Production build with size reporting
  - `armature info` - Display project information
- **Features**:
  - Template-based code generation using Handlebars
  - Automatic `mod.rs` updates when generating code
  - Test file generation (optional)
  - Progress indicators and colored output
  - Uses `cargo-watch` if installed for better performance

#### Project Templates
- New `templates/` directory with starter templates:
  - **api-minimal** - Single-file REST API for learning and prototyping
  - **api-full** - Production-ready API with JWT auth, validation, Docker, health checks
  - **microservice** - Queue-connected worker with Prometheus metrics and graceful shutdown
  - **graphql-api** - GraphQL API template
- Template documentation (`docs/project-templates.md`)
- Each template includes:
  - `Cargo.toml` with appropriate dependencies
  - `.env.example` for configuration
  - `Dockerfile` and `docker-compose.yml` where applicable

#### Core Framework
- Initial release of Armature framework
- Core framework with dependency injection and decorators
- Authentication support (JWT, OAuth2, SAML, 2FA, Passwordless)
- GraphQL support
- Validation framework
- Testing utilities
- OpenAPI/Swagger integration
- Caching (Redis, Memcached)
- Job queue system
- Cron scheduling
- OpenTelemetry observability
- Security middleware (Helmet-like)
- HTTPS/TLS support
- Static asset serving with compression
- Comprehensive debug logging throughout the framework
- 30+ working examples refactored to use module/controller pattern
- Angular 21 documentation website with:
  - Tailwind CSS 4 styling with Tailswatch oxide theme
  - SPA routing with 404.html fallback for GitHub Pages
  - Vitest for unit testing
  - API documentation integration at `/api/`

### Security
- Added cargo-husky for Git hooks (pre-commit, pre-push, commit-msg)
- Branch protection via Git hooks
- Automated linting and testing on commits
- Comprehensive `.gitignore` and `.dockerignore`

## Version History

### Versioning Strategy

We follow [Semantic Versioning](https://semver.org/):

- **MAJOR** version when making incompatible API changes
- **MINOR** version when adding functionality in a backward compatible manner
- **PATCH** version when making backward compatible bug fixes

### Release Schedule

- **Major releases**: When significant breaking changes are necessary
- **Minor releases**: Every 2-3 months with new features
- **Patch releases**: As needed for bug fixes and security updates

### Upgrade Guide

See [docs/migration.md](docs/migration.md) for detailed upgrade instructions between major versions.

---

[Unreleased]: https://github.com/PegasusHeavyIndustries/armature/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/PegasusHeavyIndustries/armature/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/PegasusHeavyIndustries/armature/releases/tag/v0.1.0
