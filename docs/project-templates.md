# Project Templates Guide

Armature provides starter templates to help you quickly bootstrap new projects.
Each template is designed for a specific use case and includes best practices.

## Table of Contents

- [Overview](#overview)
- [Available Templates](#available-templates)
- [Using Templates](#using-templates)
- [Template Details](#template-details)
- [Customization](#customization)
- [Best Practices](#best-practices)

## Overview

Templates are located in the `templates/` directory:

```
templates/
├── README.md
├── api-minimal/          # Bare-bones REST API
├── api-full/             # Full-featured API
├── graphql-api/          # GraphQL API server
└── microservice/         # Queue-connected worker
```

## Available Templates

| Template | Description | Best For |
|----------|-------------|----------|
| `api-minimal` | Single-file REST API | Learning, prototyping |
| `api-full` | Auth, validation, Docker | Authenticated REST APIs |
| `graphql-api` | GraphQL API with queries, mutations, subscriptions | GraphQL APIs |
| `microservice` | Job queue worker | Background processing |

## Using Templates

### Quick Start

```bash
# Copy a template
cp -r templates/api-minimal my-project
cd my-project

# Update project name in Cargo.toml
# Configure .env from .env.example

# Run
cargo run
```

### Template-Specific Setup

#### api-minimal

```bash
cp -r templates/api-minimal my-api
cd my-api
cp .env.example .env
cargo run
# Server at http://localhost:3000
```

#### api-full

```bash
cp -r templates/api-full my-api
cd my-api
cp .env.example .env
# Set JWT_SECRET (required, at least 32 bytes): openssl rand -hex 32
cargo run
# Server at http://localhost:3000
```

The app refuses to start without a valid `JWT_SECRET`; there is no fallback
secret.

With Docker Compose, also set `POSTGRES_PASSWORD` and `REDIS_PASSWORD` in
`.env`. Compose refuses to start while any of them is unset, and published
ports are bound to `127.0.0.1`:

```bash
docker compose up -d
```

#### graphql-api

```bash
cp -r templates/graphql-api my-graphql
cd my-graphql
cp .env.example .env
cargo run
# GraphQL endpoint at http://localhost:3000/graphql
# GraphiQL at http://localhost:3000/playground only with GRAPHQL_PLAYGROUND=true
```

#### microservice

```bash
cp -r templates/microservice my-worker
cd my-worker
cp .env.example .env
# Configure REDIS_URL
cargo run
```

## Template Details

### api-minimal

The simplest starting point for learning Armature.

**Features:**
- Single `main.rs` file
- Basic CRUD operations
- Health check endpoint
- In-memory data store

**Structure:**
```
api-minimal/
├── Cargo.toml
├── .env.example
└── src/
    └── main.rs
```

**Endpoints:**
- `GET /health` - Health check
- `GET /api/users` - List users
- `GET /api/users/:id` - Get user
- `POST /api/users` - Create user
- `DELETE /api/users/:id` - Delete user

### api-full

REST API with authentication, per-user authorization and validation.

**Features:**
- JWT authentication (`JWT_SECRET` required, refresh tokens rejected as
  bearer tokens via `verify_access`)
- Per-user authorization: listing users is admin-only; reading, updating and
  deleting a user is allowed for that user or an admin
- Request validation
- Structured logging
- Docker support
- Health checks (liveness/readiness)
- Error handling

**Structure:**
```
api-full/
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── .env.example
└── src/
    ├── main.rs
    ├── config.rs
    ├── models.rs
    ├── middleware.rs
    ├── controllers/
    │   ├── mod.rs
    │   ├── auth.rs
    │   ├── health.rs
    │   └── user.rs
    └── services/
        ├── mod.rs
        ├── auth.rs
        └── user.rs
```

**Endpoints:**
- `GET /health` - Full health check
- `GET /health/live` - Liveness probe
- `GET /health/ready` - Readiness probe
- `POST /api/auth/login` - Login
- `POST /api/auth/register` - Register
- `GET /api/users` - List users (admin only)
- `GET /api/users/:id` - Get user (self or admin)
- `PATCH /api/users/:id` - Update user (self or admin)
- `DELETE /api/users/:id` - Delete user (self or admin)

### graphql-api

GraphQL API with queries and mutations. Subscriptions are not wired up (the
schema uses `EmptySubscription`).

**Features:**
- GraphiQL playground, opt-in with `GRAPHQL_PLAYGROUND=true` (off by default)
- Query and Mutation resolvers
- Type-safe schema
- Pagination support
- Authentication integration
- Structured logging

**Structure:**
```
graphql-api/
├── Cargo.toml
├── .env.example
└── src/
    ├── main.rs
    ├── config.rs
    ├── context.rs
    ├── schema/
    │   ├── mod.rs
    │   ├── query.rs
    │   ├── mutation.rs
    │   ├── subscription.rs
    │   └── types.rs
    └── services/
        ├── mod.rs
        ├── auth.rs
        ├── user.rs
        └── book.rs
```

**Endpoints:**
- `POST /graphql` - GraphQL endpoint
- `GET /graphql/schema` - Schema SDL (404 unless `GRAPHQL_INTROSPECTION=true`; introspection queries are also disabled until then)
- `GET /playground` - GraphiQL (404 unless `GRAPHQL_PLAYGROUND=true`)
- `GET /health` - Health check
- `GET /health/live` - Liveness probe
- `GET /health/ready` - Readiness probe

**Example Queries:**
```graphql
# List all users
query {
  users {
    items { id name email role }
    total
    hasMore
  }
}

# Get a specific book with author
query {
  book(id: "1") {
    id
    title
    author { name email }
  }
}

# Create a user
mutation {
  createUser(input: { name: "Alice", email: "alice@example.com" }) {
    id
    name
  }
}

# Search books
query {
  searchBooks(query: "Rust") {
    id
    title
    publishedYear
  }
}
```

### microservice

Background job processor with health monitoring.

**Features:**
- Job queue processing
- Retry with backoff
- Prometheus metrics
- Graceful shutdown
- Docker support

**Structure:**
```
microservice/
├── Cargo.toml
├── Dockerfile
├── .env.example
└── src/
    ├── main.rs
    ├── config.rs
    ├── handlers.rs
    └── jobs.rs
```

**Endpoints:**
- `GET /health` - Service health with job stats
- `GET /health/live` - Liveness probe
- `GET /health/ready` - Readiness probe
- `GET /metrics` - Prometheus metrics

**Job Types:**
- `send_email` - Email sending
- `send_notification` - Push/SMS/Slack notifications
- `process_data` - Data processing

## Customization

### Adding a Database

```toml
# Cargo.toml
[dependencies]
sqlx = { version = "0.7", features = ["runtime-tokio", "postgres"] }
```

```rust
// src/main.rs
let pool = PgPoolOptions::new()
    .max_connections(5)
    .connect(&env::var("DATABASE_URL")?)
    .await?;
```

### Adding Rate Limiting

```toml
# Cargo.toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["ratelimit"] }
```

```rust
use armature_ratelimit::{RateLimiter, Algorithm};

let limiter = RateLimiter::builder()
    .token_bucket(100, 10.0)
    .build()
    .await?;
```

### Adding Caching

```toml
# Cargo.toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["cache"] }
```

### Adding Validation

```toml
# Cargo.toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["validation"] }
```

### Adding OpenAPI Docs

```toml
# Cargo.toml
[dependencies]
armature = { package = "armature-framework", version = "0.9", features = ["openapi"] }
```

## Best Practices

### 1. Configuration

Always use environment variables for configuration:

```rust
let config = AppConfig::from_env();
```

Never commit secrets to version control. Use `.env.example` as a template.

### 2. Logging

Use structured logging with tracing:

```rust
use tracing::{info, debug, error};

info!(user_id = %user.id, "User logged in");
```

### 3. Error Handling

Return consistent error responses:

```rust
#[derive(Serialize)]
struct ApiError {
    code: String,
    message: String,
}

HttpResponse::bad_request().json(ApiError {
    code: "VALIDATION_ERROR".into(),
    message: "Invalid input".into(),
})
```

### 4. Health Checks

Always include health endpoints for container orchestration:

- `/health` - Full health check
- `/health/live` - Is the process alive?
- `/health/ready` - Can it accept traffic?

### 5. Docker Best Practices

Use multi-stage builds with base images pinned by tag **and** digest, and run
as a non-root user. The template Dockerfiles use `cargo-chef` on Rust 1.99.1
(at or above the 1.94.1 MSRV) and a minimal runtime image without `curl`:

```dockerfile
# Build stage (pin the digest: docker buildx imagetools inspect <image>:<tag>)
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-alpine3.24@sha256:<digest> AS chef
# ... cargo chef cook, cargo build --release --locked ...

# Runtime stage
FROM alpine:3.24.2@sha256:<digest> AS runtime
# ... copy the binary, USER a non-root numeric UID ...
```

See the [Docker guide](docker-guide.md).

### 6. Security

- Use strong JWT secrets (32+ random bytes); `JwtConfig::new` rejects shorter,
  placeholder or low-entropy secrets
- Register authentication middleware with `Application::use_middleware` so
  guards see a verified identity; built-in guards fail closed without one
- Authorize per resource (self-or-admin), not only "is authenticated"
- Hash passwords with Argon2id (`armature_auth::PasswordHasher`)
- Validate all input
- Use HTTPS in production
- Implement rate limiting
- Keep `.env` out of version control; ship `.env.example` with empty secrets

## Creating New Templates

To add a new template:

1. Create directory under `templates/`
2. Include minimum files:
   - `Cargo.toml`
   - `src/main.rs`
   - `.env.example`
3. Follow existing patterns
4. Update `templates/README.md`
5. Add to this guide

## Summary

Templates provide a quick start for common project types:

| Need | Use |
|------|-----|
| Learning Armature | `api-minimal` |
| Authenticated REST API | `api-full` |
| GraphQL API | `graphql-api` |
| Background jobs | `microservice` |

All templates follow Armature best practices and can be customized as needed.

