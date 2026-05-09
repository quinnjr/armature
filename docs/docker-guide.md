# Docker Guide

This guide covers containerizing Armature applications with Docker for consistent, portable deployments.

## Table of Contents

- [Overview](#overview)
- [Basic Dockerfile](#basic-dockerfile)
- [Multi-Stage Build](#multi-stage-build)
- [Docker Compose](#docker-compose)
- [Best Practices](#best-practices)
- [Common Patterns](#common-patterns)

## Overview

Docker provides consistent deployment environments for Armature applications. Benefits include:

- **Consistent environments** across development, staging, and production
- **Easy scaling** with container orchestration
- **Isolation** from host system
- **Reproducible builds** with multi-stage Dockerfiles

## Basic Dockerfile

```dockerfile
FROM rust:1.75-slim-bookworm as builder

WORKDIR /app
COPY . .

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/my-api /usr/local/bin/

EXPOSE 3000

CMD ["my-api"]
```

## Multi-Stage Build

Optimized Dockerfile with caching for faster builds:

```dockerfile
# Stage 1: Build dependencies
FROM rust:1.75-slim-bookworm as deps

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Create a dummy project to cache dependencies
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# Stage 2: Build application
FROM deps as builder

COPY src ./src
RUN touch src/main.rs && cargo build --release

# Stage 3: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false appuser

WORKDIR /app
COPY --from=builder /app/target/release/my-api /app/

# Set ownership
RUN chown -R appuser:appuser /app

USER appuser

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/health || exit 1

CMD ["./my-api"]
```

## Docker Compose

### Development Setup

```yaml
version: '3.8'

services:
  app:
    build:
      context: .
      dockerfile: Dockerfile.dev
    ports:
      - "3000:3000"
    volumes:
      - ./src:/app/src
    environment:
      - RUST_LOG=debug
      - DATABASE_URL=postgres://user:pass@db:5432/app
      - REDIS_URL=redis://redis:6379
    depends_on:
      - db
      - redis

  db:
    image: postgres:15
    environment:
      POSTGRES_USER: user
      POSTGRES_PASSWORD: pass
      POSTGRES_DB: app
    volumes:
      - pgdata:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    volumes:
      - redisdata:/data

volumes:
  pgdata:
  redisdata:
```

### Production Setup with Ferron

```yaml
version: '3.8'

services:
  ferron:
    image: ferronweb/ferron:latest
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./ferron.conf:/etc/ferron/ferron.conf:ro
      - certs:/var/lib/ferron/certs
    depends_on:
      - app

  app:
    build:
      context: .
      dockerfile: Dockerfile
    expose:
      - "3000"
    environment:
      - RUST_LOG=info
      - DATABASE_URL=postgres://user:pass@db:5432/app
      - REDIS_URL=redis://redis:6379
    deploy:
      replicas: 3
      resources:
        limits:
          cpus: '1'
          memory: 512M
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 10s

  db:
    image: postgres:15
    environment:
      POSTGRES_USER: user
      POSTGRES_PASSWORD: pass
      POSTGRES_DB: app
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U user -d app"]
      interval: 10s
      timeout: 5s
      retries: 5

  redis:
    image: redis:7-alpine
    volumes:
      - redisdata:/data
    healthcheck:
      test: ["CMD", "redis-cli", "ping"]
      interval: 10s
      timeout: 5s
      retries: 5

volumes:
  certs:
  pgdata:
  redisdata:
```

## Best Practices

### 1. Use Multi-Stage Builds

Separate build and runtime stages for smaller images:

```dockerfile
# Build stage
FROM rust:1.75 as builder
# ... build steps

# Runtime stage
FROM debian:bookworm-slim
# ... only runtime files
```

### 2. Run as Non-Root User

```dockerfile
RUN useradd -r -s /bin/false appuser
USER appuser
```

### 3. Add Health Checks

```dockerfile
HEALTHCHECK --interval=30s --timeout=5s \
    CMD curl -f http://localhost:3000/health || exit 1
```

### 4. Use .dockerignore

```dockerignore
target/
.git/
.env
*.md
tests/
docs/
```

### 5. Set Resource Limits

```yaml
deploy:
  resources:
    limits:
      cpus: '1'
      memory: 512M
    reservations:
      cpus: '0.25'
      memory: 256M
```

### 6. Use Slim Base Images

Prefer `debian:bookworm-slim` or `alpine` over full images.

### 7. Cache Dependencies

Copy `Cargo.toml` and `Cargo.lock` first, then build deps before copying source.

## Common Patterns

### Development Hot Reload

```dockerfile
# Dockerfile.dev
FROM rust:1.75

RUN cargo install cargo-watch

WORKDIR /app
COPY Cargo.toml Cargo.lock ./

CMD ["cargo", "watch", "-x", "run"]
```

### With Static Assets

```dockerfile
# Build frontend
FROM node:20 as frontend
WORKDIR /web
COPY web/package*.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

# Build backend
FROM rust:1.75 as backend
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime
FROM debian:bookworm-slim
COPY --from=backend /app/target/release/my-api /app/
COPY --from=frontend /web/dist /app/static/
CMD ["/app/my-api"]
```

### With Ferron Sidecar

```dockerfile
# ferron.Dockerfile
FROM ferronweb/ferron:latest
COPY ferron.conf /etc/ferron/ferron.conf
CMD ["ferron", "-c", "/etc/ferron/ferron.conf"]
```

## Summary

- Use **multi-stage builds** for smaller images
- Run as **non-root user** for security
- Add **health checks** for orchestration
- Use **Docker Compose** for local development
- Set **resource limits** in production
- Separate **development and production** Dockerfiles

