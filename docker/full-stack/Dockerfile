# Armature Full-Stack Dockerfile
# Production-optimized multi-stage build

# The chef/planner boilerplate below (Stages 1-2) is intentionally duplicated
# across all 4 Armature Dockerfiles rather than extracted into a shared base
# image, since this repo builds each with plain `docker build` (no buildx
# bake infra). Keep the base image references in sync across:
#   docker/full-stack/Dockerfile
#   docker/microservices/Dockerfile.service
#   docker/microservices/Dockerfile.worker
#   docker/observability/Dockerfile.api
#
# Base images are pinned by exact tag AND digest (cargo-chef 0.1.78, Rust
# 1.99.1 >= the 1.94.1 MSRV, Alpine 3.24), so a re-pushed tag cannot change
# what gets built. Dependabot (docker ecosystem) proposes digest bumps.

# ==================================================
# Stage 1: Chef - base image with cargo-chef installed
# ==================================================
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-alpine3.24@sha256:2c27e67049f17e8b67d882a2cf2f5e1a1d4d71c206ec97f91edf278254688e95 AS chef
WORKDIR /app

# ==================================================
# Stage 2: Planner - compute the dependency-only recipe
# ==================================================
FROM chef AS planner

# The whole workspace (all Cargo.toml manifests + sources) is required for
# `cargo chef prepare` to resolve the dependency graph, but this stage only
# ever produces recipe.json, which is all that gets copied forward.
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ==================================================
# Stage 3: Build
# ==================================================
FROM chef AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig \
    git

# Build dependencies only. This layer is cache-keyed on recipe.json (which is
# itself derived only from Cargo.toml/Cargo.lock), so it is skipped entirely
# on rebuilds that only change application source.
COPY --from=planner /app/recipe.json recipe.json
# Scoped to the single example this image actually builds (full_example)
# instead of --examples, so the cached dependency layer isn't inflated by
# every other example in the workspace.
RUN cargo chef cook --release --example full_example --recipe-path recipe.json

# Copy actual source code
COPY . .

# Build the application
RUN cargo build --release --example full_example && \
    strip /app/target/release/examples/full_example

# ==================================================
# Stage 4: Runtime
# ==================================================
FROM alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    tini

# Create non-root user
RUN addgroup -g 1000 armature && \
    adduser -u 1000 -G armature -s /bin/sh -D armature

# Copy binary from builder
COPY --from=builder /app/target/release/examples/full_example /app/server

# Copy static assets if any
# COPY --from=builder /app/static /app/static

# Set ownership
RUN chown -R armature:armature /app

# Switch to non-root user
USER armature

# Expose ports
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q --spider http://127.0.0.1:3000/health || exit 1

# Use tini as init system
ENTRYPOINT ["/sbin/tini", "--"]

# Run the application
CMD ["/app/server"]
