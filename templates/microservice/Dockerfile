# =============================================================================
# Armature Microservice - Optimized Multi-Stage Dockerfile (Alpine)
# =============================================================================
# Features:
# - Multi-stage build for minimal image size (~10-15MB)
# - Cargo chef for dependency caching
# - Alpine Linux for tiny footprint
# - Non-root user for security
# - Health checks built-in (BusyBox wget)
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Chef - Prepare dependency recipe
# -----------------------------------------------------------------------------
# Base images are pinned by exact tag AND digest: cargo-chef 0.1.78 on Rust
# 1.99.1 (>= this template's 1.94.1 MSRV) and Alpine 3.24, so a re-pushed tag
# cannot silently change what you build. Bump both together (Dependabot's
# "docker" ecosystem does this for you).
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-alpine3.24@sha256:2c27e67049f17e8b67d882a2cf2f5e1a1d4d71c206ec97f91edf278254688e95 AS chef
WORKDIR /app

# -----------------------------------------------------------------------------
# Stage 2: Planner - Create dependency recipe
# -----------------------------------------------------------------------------
FROM chef AS planner
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# -----------------------------------------------------------------------------
# Stage 3: Builder - Build dependencies and application
# -----------------------------------------------------------------------------
FROM chef AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig

# Build dependencies (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

# Build application
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release --target x86_64-unknown-linux-musl --bin microservice

# -----------------------------------------------------------------------------
# Stage 4: Runtime - Minimal Alpine image
# -----------------------------------------------------------------------------
FROM alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime

WORKDIR /app

# Install CA certificates for HTTPS. The health check below uses BusyBox
# wget, which is already in the base image, so no HTTP client is added.
RUN apk add --no-cache ca-certificates \
    && adduser -D -u 1000 appuser

# Copy binary from builder
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/microservice /app/service

# Set ownership
RUN chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD wget -q --spider http://127.0.0.1:8080/health/live || exit 1

# Run
CMD ["/app/service"]

