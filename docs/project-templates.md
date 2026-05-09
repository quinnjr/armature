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
| `api-full` | Auth, validation, Docker | Production APIs |
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
# Edit .env with your JWT_SECRET
cargo run
# Server at http://localhost:3000
```

For production with Docker:

```bash
docker-compose up -d
```

#### graphql-api

```bash
cp -r templates/graphql-api my-graphql
cd my-graphql
cp .env.example .env
cargo run
# GraphQL Playground at http://localhost:3000/graphql
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

Production-ready API with authentication and validation.

**Features:**
- JWT authentication
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
- `GET /api/users` - List users (authenticated)
- `GET /api/users/:id` - Get user (authenticated)
- `DELETE /api/users/:id` - Delete user (authenticated)

### graphql-api

Production-ready GraphQL API with queries, mutations, and subscriptions.

**Features:**
- GraphQL Playground/GraphiQL
- Query and Mutation resolvers
- Subscription support
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
- `GET /graphql` - GraphQL Playground
- `POST /graphql` - GraphQL endpoint
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
armature-framework = { version = "0.1", features = ["ratelimit"] }
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
armature-framework = { version = "0.1", features = ["cache"] }
```

### Adding Validation

```toml
# Cargo.toml
[dependencies]
armature-framework = { version = "0.1", features = ["validation"] }
```

### Adding OpenAPI Docs

```toml
# Cargo.toml
[dependencies]
armature-framework = { version = "0.1", features = ["openapi"] }
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

Use multi-stage builds:

```dockerfile
# Build stage
FROM rust:1.85 AS builder
# ... build ...

# Runtime stage
FROM debian:bookworm-slim
# ... minimal runtime ...
```

### 6. Security

- Use strong JWT secrets (32+ random bytes)
- Hash passwords with bcrypt/argon2
- Validate all input
- Use HTTPS in production
- Implement rate limiting

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
| Production REST API | `api-full` |
| GraphQL API | `graphql-api` |
| Background jobs | `microservice` |

All templates follow Armature best practices and can be customized as needed.

