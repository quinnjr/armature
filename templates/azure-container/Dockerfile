# =============================================================================
# Armature Azure Container Apps - Optimized Container Image
# =============================================================================
# Features:
# - Multi-stage build for minimal size
# - Alpine Linux with musl for static binary
# - Azure Container Apps optimizations
# - Non-root user for security
# - Health probes configured
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Chef - Prepare dependency recipe
# -----------------------------------------------------------------------------
FROM lukemathwalker/cargo-chef:latest-rust-1.94.1-alpine AS chef
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
# Stage 4: Runtime - Minimal Alpine image
# -----------------------------------------------------------------------------
FROM alpine:3.20 AS runtime

# Redeclare: ARGs do not cross FROM boundaries
ARG BIN_NAME=app

WORKDIR /app

# Install runtime dependencies
RUN apk add --no-cache ca-certificates curl \
    && adduser -D -u 1000 appuser

# Copy binary from builder
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/${BIN_NAME} /app/server

# Set ownership
RUN chown -R appuser:appuser /app

# Switch to non-root user
USER appuser

# Azure Container Apps uses PORT env variable (default 80)
ENV PORT=80
EXPOSE 80

# Health check endpoint for Azure probes
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:${PORT}/health || exit 1

# Run
CMD ["/app/server"]

# =============================================================================
# Deployment Instructions
# =============================================================================
# 1. Build the image (BIN_NAME defaults to "app"; set it to your crate's
#    binary name if different):
#    docker build --build-arg BIN_NAME=my-app -t armature-app -f Dockerfile .
#
# 2. Tag for Azure Container Registry:
#    docker tag armature-app myregistry.azurecr.io/armature-app:latest
#
# 3. Push to ACR:
#    az acr login --name myregistry
#    docker push myregistry.azurecr.io/armature-app:latest
#
# 4. Deploy to Azure Container Apps:
#    az containerapp create \
#      --name armature-app \
#      --resource-group my-rg \
#      --environment my-env \
#      --image myregistry.azurecr.io/armature-app:latest \
#      --target-port 80 \
#      --ingress external \
#      --cpu 0.5 \
#      --memory 1.0Gi \
#      --min-replicas 0 \
#      --max-replicas 10
#
# 5. Or deploy via YAML:
#    az containerapp create \
#      --name armature-app \
#      --resource-group my-rg \
#      --yaml containerapp.yaml
# =============================================================================

