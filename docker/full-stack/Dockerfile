# Armature Full-Stack Dockerfile
# Production-optimized multi-stage build

# The chef/planner boilerplate below (Stages 1-2) is intentionally duplicated
# across all 4 Armature Dockerfiles rather than extracted into a shared base
# image, since this repo builds each with plain `docker build` (no buildx
# bake infra). Keep RUST_VERSION / CARGO_CHEF_VERSION in sync across:
#   docker/full-stack/Dockerfile
#   docker/microservices/Dockerfile.service
#   docker/microservices/Dockerfile.worker
#   docker/observability/Dockerfile.api
ARG RUST_VERSION=1.94.1
# Pinned to a specific cargo-chef release (not "latest") for reproducible
# builds. Update periodically: https://github.com/LukeMathWalker/cargo-chef/releases
ARG CARGO_CHEF_VERSION=0.1.68

# ==================================================
# Stage 1: Chef - base image with cargo-chef installed
# ==================================================
FROM lukemathwalker/cargo-chef:${CARGO_CHEF_VERSION}-rust-${RUST_VERSION}-alpine AS chef
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
FROM alpine:3.19 AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    curl \
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
    CMD curl -f http://localhost:3000/health || exit 1

# Use tini as init system
ENTRYPOINT ["/sbin/tini", "--"]

# Run the application
CMD ["/app/server"]
