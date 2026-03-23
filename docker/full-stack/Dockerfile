# Armature Full-Stack Dockerfile
# Production-optimized multi-stage build

# ==================================================
# Stage 1: Build
# ==================================================
FROM rust:1.83-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig \
    git

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy all crate manifests
COPY armature-core/Cargo.toml armature-core/
COPY armature-proc-macro/Cargo.toml armature-proc-macro/
COPY armature-log/Cargo.toml armature-log/
COPY armature-auth/Cargo.toml armature-auth/
COPY armature-cache/Cargo.toml armature-cache/
COPY armature-redis/Cargo.toml armature-redis/
COPY armature-validation/Cargo.toml armature-validation/

# Create dummy source files for dependency caching
RUN for crate in armature-*; do \
        mkdir -p "$crate/src" && \
        if [ "$crate" = "armature-proc-macro" ]; then \
            echo "fn main() {}" > "$crate/src/lib.rs"; \
        else \
            echo "pub fn main() {}" > "$crate/src/lib.rs"; \
        fi; \
    done

# Build dependencies only
RUN cargo build --release --workspace 2>/dev/null || true

# Copy actual source code
COPY . .

# Touch all source files to invalidate crate caches
RUN find . -name "*.rs" -exec touch {} \;

# Build the application
RUN cargo build --release --example full_example && \
    strip /app/target/release/examples/full_example

# ==================================================
# Stage 2: Runtime
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

