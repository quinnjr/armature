# =============================================================================
# Armature Google Cloud Run - Optimized Container Image
# =============================================================================
# Features:
# - Multi-stage build for minimal size (~15-25MB)
# - Alpine Linux with musl for static binary
# - Non-root user for security
# - Cloud Run optimizations (PORT env, health checks)
# - Graceful shutdown handling (SIGTERM)
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Chef - Prepare dependency recipe
# -----------------------------------------------------------------------------
# Base images are pinned by exact tag AND digest: cargo-chef 0.1.78 on Rust
# 1.99.1 (>= the 1.94.1 MSRV), Alpine 3.24 and distroless static Debian 13.
# Dependabot's "docker" ecosystem proposes digest bumps.
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

# Name of the compiled binary (your crate's [[bin]] name, usually the
# package name). Override with: docker build --build-arg BIN_NAME=my-app
ARG BIN_NAME=app

# Install build dependencies
RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconfig

# Build dependencies (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

# Build application
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release --target x86_64-unknown-linux-musl --bin ${BIN_NAME}

# -----------------------------------------------------------------------------
# Stage 4a: Debug runtime (Alpine, has a shell). Used by docker-compose.yml
# for local development via `target: runtime-debug`. It is declared BEFORE
# the production stage on purpose: a plain `docker build` builds the LAST
# stage, which must stay the distroless one.
# -----------------------------------------------------------------------------
FROM alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS runtime-debug

ARG BIN_NAME=app

WORKDIR /app

# No curl: the health check uses BusyBox wget from the base image.
RUN apk add --no-cache ca-certificates \
    && adduser -D -u 65532 nonroot

COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/${BIN_NAME} /app/server

USER nonroot

ENV PORT=8080
EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -q --spider "http://127.0.0.1:${PORT}/health" || exit 1

CMD ["/app/server"]

# -----------------------------------------------------------------------------
# Stage 4b: Runtime - Minimal Distroless image (default build target)
# -----------------------------------------------------------------------------
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3 AS runtime

# Redeclare: ARGs do not cross FROM boundaries
ARG BIN_NAME=app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/${BIN_NAME} /app/server

# Cloud Run uses PORT env variable (default 8080)
ENV PORT=8080

# Expose port
EXPOSE 8080

# Run as non-root (distroless nonroot tag already sets this)
USER nonroot:nonroot

# Run
ENTRYPOINT ["/app/server"]

# =============================================================================
# Deployment Instructions
# =============================================================================
# 1. Build the image (BIN_NAME defaults to "app"; set it to your crate's
#    binary name if different):
#    docker build --build-arg BIN_NAME=my-app -t gcr.io/PROJECT_ID/armature-app -f Dockerfile .
#
# 2. Push to Container Registry:
#    docker push gcr.io/PROJECT_ID/armature-app
#
# 3. Deploy to Cloud Run:
#    gcloud run deploy armature-app \
#      --image gcr.io/PROJECT_ID/armature-app \
#      --platform managed \
#      --region us-central1 \
#      --allow-unauthenticated \
#      --memory 256Mi \
#      --cpu 1 \
#      --min-instances 0 \
#      --max-instances 10 \
#      --timeout 300s \
#      --concurrency 80
#
# 4. Or use Artifact Registry (recommended):
#    gcloud builds submit --tag us-central1-docker.pkg.dev/PROJECT_ID/armature/app
#    gcloud run deploy armature-app \
#      --image us-central1-docker.pkg.dev/PROJECT_ID/armature/app \
#      --platform managed \
#      --region us-central1
# =============================================================================

