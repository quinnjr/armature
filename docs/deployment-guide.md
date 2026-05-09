# Deployment Guide

This guide covers deploying Armature applications to production environments.

## Table of Contents

- [Overview](#overview)
- [Deployment Options](#deployment-options)
- [Environment Configuration](#environment-configuration)
- [Reverse Proxy Setup](#reverse-proxy-setup)
- [Containerization](#containerization)
- [Health Checks](#health-checks)
- [Monitoring](#monitoring)
- [Scaling](#scaling)
- [Security Checklist](#security-checklist)
- [Best Practices](#best-practices)

## Overview

Armature applications are compiled to native binaries, providing excellent performance and simple deployment. The typical production setup includes:

1. **Application Binary** - Your compiled Armature application
2. **Reverse Proxy** - Ferron, Nginx, or Caddy for TLS termination and load balancing
3. **Database** - PostgreSQL, MySQL, or other database
4. **Cache** - Redis for caching and sessions
5. **Monitoring** - Prometheus, Grafana, or cloud monitoring

## Deployment Options

### Bare Metal / VM

```bash
# Build release binary
cargo build --release

# Copy to server
scp target/release/my-api user@server:/opt/my-api/

# Run with systemd
sudo systemctl enable my-api
sudo systemctl start my-api
```

### Docker

See the [Docker Guide](docker-guide.md) for detailed containerization instructions.

```bash
docker build -t my-api .
docker run -p 3000:3000 my-api
```

### Kubernetes

See the [Kubernetes Guide](kubernetes-guide.md) for orchestration details.

```bash
kubectl apply -f k8s/
```

### Serverless

Armature supports serverless deployment to:
- **AWS Lambda** with `armature-lambda`
- **Google Cloud Run** with `armature-cloudrun`
- **Azure Functions** with `armature-azure-functions`

## Environment Configuration

### Required Environment Variables

```bash
# Application
PORT=3000
HOST=0.0.0.0
RUST_LOG=info

# Database
DATABASE_URL=postgres://user:pass@localhost/mydb

# Redis (optional)
REDIS_URL=redis://localhost:6379

# Security
JWT_SECRET=your-secret-key
```

### Configuration Files

```rust
use armature_framework::config::{Config, Environment};

let config = Config::builder()
    .add_source(Environment::with_prefix("APP"))
    .build()?;
```

## Reverse Proxy Setup

### Using Ferron (Recommended)

See the [Ferron Guide](ferron-guide.md) for detailed integration.

```rust
use armature_ferron::{FerronConfig, Location, RateLimitConfig};

let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .tls_auto(true)
    .location(
        Location::new("/api")
            .proxy("http://localhost:3000/api")
            .rate_limit(RateLimitConfig::new(100))
    )
    .build()?;
```

### Using Nginx

```nginx
server {
    listen 443 ssl http2;
    server_name api.example.com;

    ssl_certificate /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

    location / {
        proxy_pass http://localhost:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### Using Caddy

```caddyfile
api.example.com {
    reverse_proxy localhost:3000

    header {
        X-Frame-Options "DENY"
        X-Content-Type-Options "nosniff"
        X-XSS-Protection "1; mode=block"
    }
}
```

## Health Checks

### Implementing Health Endpoints

```rust
#[controller("")]
#[derive(Default, Clone)]
struct HealthController;

#[routes]
impl HealthController {
    #[get("/health")]
    async fn health() -> Result<HttpResponse, Error> {
        // Check dependencies
        HttpResponse::json(&serde_json::json!({
            "status": "healthy",
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }

    #[get("/ready")]
    async fn ready() -> Result<HttpResponse, Error> {
        // Check if ready to serve traffic
        HttpResponse::json(&serde_json::json!({ "ready": true }))
    }

    #[get("/live")]
    async fn live() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_body(b"OK".to_vec())
    }
}
```

## Monitoring

### Prometheus Metrics

```rust
use armature_framework::metrics::{MetricsConfig, PrometheusExporter};

let metrics = MetricsConfig::default()
    .enable_prometheus("/metrics")
    .build()?;
```

### OpenTelemetry

```rust
use armature_framework::telemetry::{TelemetryConfig, OtlpExporter};

let telemetry = TelemetryConfig::default()
    .with_traces(OtlpExporter::new("http://jaeger:4317"))
    .build()?;
```

## Security Checklist

Before deploying to production:

- [ ] **TLS enabled** - All traffic encrypted
- [ ] **Secrets in environment** - No hardcoded credentials
- [ ] **Rate limiting configured** - Prevent abuse
- [ ] **CORS properly set** - Allow only trusted origins
- [ ] **Security headers** - X-Frame-Options, CSP, etc.
- [ ] **Input validation** - Validate all user input
- [ ] **Authentication required** - Protect sensitive endpoints
- [ ] **Logging configured** - Structured JSON logging
- [ ] **Health checks working** - /health, /ready, /live
- [ ] **Graceful shutdown** - Handle SIGTERM properly

## Best Practices

### 1. Use Release Builds

```bash
cargo build --release
```

### 2. Enable All Security Headers

```rust
.header("X-Frame-Options", "DENY")
.header("X-Content-Type-Options", "nosniff")
.header("X-XSS-Protection", "1; mode=block")
.header("Strict-Transport-Security", "max-age=31536000")
.header("Content-Security-Policy", "default-src 'self'")
```

### 3. Configure Graceful Shutdown

```rust
use armature_framework::shutdown::GracefulShutdown;

let shutdown = GracefulShutdown::new()
    .timeout(Duration::from_secs(30))
    .on_shutdown(|| async {
        // Cleanup connections
    });
```

### 4. Use Connection Pooling

```rust
let pool = PgPoolOptions::new()
    .max_connections(100)
    .min_connections(10)
    .connect(&database_url)
    .await?;
```

### 5. Set Resource Limits

```yaml
# Kubernetes
resources:
  limits:
    cpu: "2"
    memory: "2Gi"
  requests:
    cpu: "500m"
    memory: "512Mi"
```

## Summary

Key deployment considerations:

1. **Use Ferron** for reverse proxy with automatic TLS
2. **Configure health checks** for load balancer integration
3. **Enable metrics and tracing** for observability
4. **Follow security checklist** before going live
5. **Test graceful shutdown** to prevent dropped requests
6. **Use containers** for consistent deployments

