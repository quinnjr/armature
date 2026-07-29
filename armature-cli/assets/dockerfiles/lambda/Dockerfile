# =============================================================================
# Armature AWS Lambda - Container Image Deployment
# =============================================================================
# Features:
# - AWS Lambda container image support
# - Multi-stage build for minimal size
# - cargo-lambda for optimal Lambda builds
# - x86_64 and ARM64 (Graviton2) support
# - Lambda Runtime Interface Client (RIC) included
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Builder - Build with cargo-lambda
# -----------------------------------------------------------------------------
FROM rust:1.94-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    openssl-dev \
    openssl-libs-static \
    pkgconfig \
    curl \
    gcc

# Install cargo-lambda, pinned to a specific release (not "latest") to avoid
# pulling an unvetted moving target on every future `docker build`.
# TODO: update this pin periodically. Integrity is verified via the
# CARGO_LAMBDA_SHA256 build-arg below when a checksum is supplied; look up
# the current release's SHA256 on cargo-lambda's GitHub releases page and
# pass it with `--build-arg CARGO_LAMBDA_SHA256=<hash>`.
ARG CARGO_LAMBDA_VERSION=v1.8.3
ARG CARGO_LAMBDA_SHA256=""
RUN curl -sSL -o cargo-lambda.tar.gz "https://github.com/cargo-lambda/cargo-lambda/releases/download/${CARGO_LAMBDA_VERSION}/cargo-lambda-x86_64-unknown-linux-musl.tar.gz" \
    && if [ -n "$CARGO_LAMBDA_SHA256" ]; then \
         echo "${CARGO_LAMBDA_SHA256}  cargo-lambda.tar.gz" | sha256sum -c -; \
       else \
         echo "WARNING: CARGO_LAMBDA_SHA256 not set — skipping integrity check" >&2; \
       fi \
    && tar xz -C /usr/local/bin -f cargo-lambda.tar.gz \
    && rm cargo-lambda.tar.gz

# Copy dependency files first (for caching)
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
# Intentionally allowed to fail: this dummy build only exists to cache
# dependency compilation in a Docker layer; a failure here just means no
# cache was warmed, not a real problem.
RUN cargo lambda build --release --target x86_64-unknown-linux-musl || true

# Copy source and build
COPY src ./src
RUN cargo lambda build --release --target x86_64-unknown-linux-musl

# -----------------------------------------------------------------------------
# Stage 2: Runtime - AWS Lambda base image
# -----------------------------------------------------------------------------
FROM public.ecr.aws/lambda/provided:al2023 AS runtime

# Note: no non-root USER is set here, unlike the Cloud Run Dockerfile.
# The AWS Lambda "provided" base image's bootstrap/init process and Lambda
# Runtime Interface Client are designed to run as root inside the managed
# Lambda execution environment; switching to a non-root user is not a
# supported/documented configuration for this base image and risks breaking
# the runtime's ability to start. Isolation here is instead provided by
# AWS Lambda's own Firecracker microVM sandboxing, not by an in-container
# non-root user.

# Copy the bootstrap binary
COPY --from=builder /app/target/lambda/*/bootstrap ${LAMBDA_RUNTIME_DIR}/bootstrap

# Set the handler
CMD ["bootstrap"]

# =============================================================================
# Alternative: ARM64 (Graviton2) Build
# =============================================================================
# To build for ARM64, use:
#   docker build --platform linux/arm64 -f Dockerfile.lambda-arm64 .
#
# Or use cargo-lambda with:
#   cargo lambda build --release --arm64
# =============================================================================

# =============================================================================
# Deployment Instructions
# =============================================================================
# 1. Build the image:
#    docker build -t my-lambda-app -f Dockerfile .
#
# 2. Tag for ECR:
#    docker tag my-lambda-app:latest 123456789.dkr.ecr.us-east-1.amazonaws.com/my-lambda-app:latest
#
# 3. Push to ECR:
#    aws ecr get-login-password --region us-east-1 | docker login --username AWS --password-stdin 123456789.dkr.ecr.us-east-1.amazonaws.com
#    docker push 123456789.dkr.ecr.us-east-1.amazonaws.com/my-lambda-app:latest
#
# 4. Create/Update Lambda function:
#    aws lambda create-function \
#      --function-name my-function \
#      --package-type Image \
#      --code ImageUri=123456789.dkr.ecr.us-east-1.amazonaws.com/my-lambda-app:latest \
#      --role arn:aws:iam::123456789:role/lambda-role
# =============================================================================

