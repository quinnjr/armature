# Health Check Guide

Application health monitoring and Kubernetes probe support for Armature applications.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Health Indicators](#health-indicators)
- [Kubernetes Probes](#kubernetes-probes)
- [Custom Health Indicators](#custom-health-indicators)
- [Configuration](#configuration)
- [API Reference](#api-reference)
- [Best Practices](#best-practices)
- [Summary](#summary)

## Overview

The health check module provides comprehensive health monitoring for your Armature applications. It supports:

- Kubernetes liveness, readiness, and startup probes
- Custom health indicators for databases, caches, and external services
- Built-in indicators for memory, disk, and uptime monitoring
- JSON responses compatible with monitoring systems

## Features

- ✅ Kubernetes-compatible probe endpoints (`/health/live`, `/health/ready`, `/health`)
- ✅ Custom health indicators via trait implementation
- ✅ Built-in memory, disk, and uptime indicators
- ✅ Concurrent health check execution
- ✅ Configurable health thresholds
- ✅ JSON response format for monitoring integration
- ✅ Health status aggregation (UP, DOWN, DEGRADED, UNKNOWN)

## Quick Start

### Basic Usage

```rust
use armature_core::health::{HealthService, HealthServiceBuilder, HealthInfo, UptimeHealthIndicator};

#[tokio::main]
async fn main() {
    // Create a health service with default indicators
    let health_service = HealthServiceBuilder::new()
        .with_defaults()  // Adds memory, disk, and uptime indicators
        .with_info(HealthInfo::new("my-app").with_version("1.0.0"))
        .build();

    // Perform a full health check
    let health = health_service.check().await;

    println!("Status: {}", health.status);
    println!("Response: {}", serde_json::to_string_pretty(&health).unwrap());
}
```

### Adding Health Endpoints to Your Application

```rust
use armature_core::{HttpRequest, HttpResponse, Controller, RouteDefinition, HttpMethod};
use armature_core::health::{HealthService, HealthServiceBuilder, HealthInfo};
use std::sync::Arc;

struct HealthController {
    health_service: Arc<HealthService>,
}

impl HealthController {
    pub fn new(health_service: Arc<HealthService>) -> Self {
        Self { health_service }
    }

    /// Full health check - GET /health
    pub async fn health(&self, _req: HttpRequest) -> Result<HttpResponse, armature_core::Error> {
        let response = self.health_service.check().await;
        let status = response.status.http_status_code();

        Ok(HttpResponse::new(status)
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_vec(&response).unwrap()))
    }

    /// Liveness probe - GET /health/live
    pub async fn liveness(&self, _req: HttpRequest) -> Result<HttpResponse, armature_core::Error> {
        let response = self.health_service.check_liveness().await;
        let status = response.status.http_status_code();

        Ok(HttpResponse::new(status)
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_vec(&response).unwrap()))
    }

    /// Readiness probe - GET /health/ready
    pub async fn readiness(&self, _req: HttpRequest) -> Result<HttpResponse, armature_core::Error> {
        let response = self.health_service.check_readiness().await;
        let status = response.status.http_status_code();

        Ok(HttpResponse::new(status)
            .with_header("Content-Type".to_string(), "application/json".to_string())
            .with_body(serde_json::to_vec(&response).unwrap()))
    }
}
```

## Registering in DI Container

### Using ProviderRegistration in Modules

The recommended way to register `HealthService` in your application is through the module system:

```rust
use armature_core::{
    Module, Container, ProviderRegistration, ControllerRegistration,
    health::{HealthService, HealthServiceBuilder, HealthInfo},
};
use std::any::TypeId;

struct AppModule;

impl Module for AppModule {
    fn providers(&self) -> Vec<ProviderRegistration> {
        vec![
            ProviderRegistration {
                type_id: TypeId::of::<HealthService>(),
                type_name: "HealthService",
                register_fn: |container| {
                    let health_service = HealthServiceBuilder::new()
                        .with_defaults()
                        .with_info(HealthInfo::new("my-app").with_version("1.0.0"))
                        .build();
                    container.register(health_service);
                },
            },
        ]
    }

    fn controllers(&self) -> Vec<ControllerRegistration> {
        vec![]  // Your controllers
    }

    fn imports(&self) -> Vec<Box<dyn Module>> {
        vec![]
    }

    fn exports(&self) -> Vec<TypeId> {
        vec![TypeId::of::<HealthService>()]
    }
}
```

### Manual Container Registration

For simpler use cases or testing, you can register directly with the container:

```rust
use armature_core::{Container, health::{HealthService, HealthServiceBuilder}};

fn setup_health_service(container: &Container) {
    let health_service = HealthServiceBuilder::new()
        .with_defaults()
        .build();

    container.register(health_service);
}

// Later, resolve and use:
let health_service = container.resolve::<HealthService>().unwrap();
let health = health_service.check().await;
```

### Injecting into Controllers

Once registered, you can inject `HealthService` into your controllers:

```rust
use armature_core::health::HealthService;

#[controller("/health")]
#[derive(Default, Clone)]
struct HealthController {
    health_service: HealthService,  // Automatically injected!
}

impl HealthController {
    #[get("/")]
    async fn check(&self) -> Result<HttpResponse, Error> {
        let response = self.health_service.check().await;
        Ok(HttpResponse::new(response.status.http_status_code())
            .with_json(&response)?)
    }

    #[get("/live")]
    async fn liveness(&self) -> Result<HttpResponse, Error> {
        let response = self.health_service.check_liveness().await;
        Ok(HttpResponse::new(response.status.http_status_code())
            .with_json(&response)?)
    }
}
```

## Health Indicators

### Built-in Indicators

#### UptimeHealthIndicator

Reports application uptime. Always returns UP status.

```rust
use armature_core::health::UptimeHealthIndicator;

let indicator = UptimeHealthIndicator::new();
```

Response:
```json
{
  "name": "uptime",
  "status": "UP",
  "details": {
    "uptime_seconds": "3600",
    "uptime_human": "0d 1h 0m 0s"
  }
}
```

#### MemoryHealthIndicator

Monitors system memory usage with configurable thresholds.

```rust
use armature_core::health::MemoryHealthIndicator;

// Default thresholds: 80% degraded, 95% critical
let indicator = MemoryHealthIndicator::default();

// Custom thresholds
let indicator = MemoryHealthIndicator::with_thresholds(70.0, 90.0);
```

Response:
```json
{
  "name": "memory",
  "status": "UP",
  "details": {
    "total_mb": "16384",
    "available_mb": "8192",
    "used_mb": "8192",
    "usage_percent": "50.0"
  }
}
```

#### DiskHealthIndicator

Monitors disk space usage with configurable thresholds.

```rust
use armature_core::health::DiskHealthIndicator;

// Check root filesystem with default thresholds
let indicator = DiskHealthIndicator::new("/");

// Custom path and thresholds
let indicator = DiskHealthIndicator::new("/data")
    .with_thresholds(75.0, 90.0);
```

Response:
```json
{
  "name": "disk",
  "status": "UP",
  "details": {
    "path": "/",
    "total_gb": "500.0",
    "available_gb": "250.0",
    "used_gb": "250.0",
    "usage_percent": "50.0"
  }
}
```

## Kubernetes Probes

### Probe Types

| Endpoint | Purpose | Behavior |
|----------|---------|----------|
| `/health/live` | Liveness probe | Returns 200 if app is running |
| `/health/ready` | Readiness probe | Returns 200 if app can serve traffic |
| `/health` | Full health check | Returns all component statuses |

### Kubernetes Configuration

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
    - name: my-app
      livenessProbe:
        httpGet:
          path: /health/live
          port: 8080
        initialDelaySeconds: 10
        periodSeconds: 15
        failureThreshold: 3
      readinessProbe:
        httpGet:
          path: /health/ready
          port: 8080
        initialDelaySeconds: 5
        periodSeconds: 10
        failureThreshold: 3
      startupProbe:
        httpGet:
          path: /health/ready
          port: 8080
        initialDelaySeconds: 0
        periodSeconds: 5
        failureThreshold: 30
```

### Health Status Codes

| Status | HTTP Code | Meaning |
|--------|-----------|---------|
| UP | 200 | Component is healthy |
| DEGRADED | 200 | Component has issues but operational |
| DOWN | 503 | Component is not functioning |
| UNKNOWN | 503 | Component status cannot be determined |

## Custom Health Indicators

### Basic Implementation

```rust
use armature_core::health::{HealthIndicator, HealthCheckResult};
use async_trait::async_trait;

struct DatabaseHealthIndicator {
    connection_string: String,
}

#[async_trait]
impl HealthIndicator for DatabaseHealthIndicator {
    fn name(&self) -> &str {
        "database"
    }

    async fn check(&self) -> HealthCheckResult {
        // Perform actual database connectivity check
        match self.ping_database().await {
            Ok(latency_ms) => {
                HealthCheckResult::up("database")
                    .with_detail("type", "postgresql")
                    .with_detail("latency_ms", latency_ms.to_string())
            }
            Err(e) => {
                HealthCheckResult::down("database")
                    .with_error(e.to_string())
            }
        }
    }

    fn is_critical(&self) -> bool {
        true  // App cannot function without database
    }

    fn include_in_readiness(&self) -> bool {
        true  // Include in readiness checks
    }

    fn include_in_liveness(&self) -> bool {
        false  // Don't include in liveness (avoid cascading failures)
    }
}

impl DatabaseHealthIndicator {
    async fn ping_database(&self) -> Result<u64, String> {
        // Actual implementation would ping the database
        Ok(5)  // 5ms latency
    }
}
```

### Redis Health Indicator Example

```rust
use armature_core::health::{HealthIndicator, HealthCheckResult};
use async_trait::async_trait;
use std::time::Instant;

struct RedisHealthIndicator {
    // Redis client would go here
}

#[async_trait]
impl HealthIndicator for RedisHealthIndicator {
    fn name(&self) -> &str {
        "redis"
    }

    async fn check(&self) -> HealthCheckResult {
        let start = Instant::now();

        // Simulate Redis PING
        match self.ping().await {
            Ok(_) => {
                let duration = start.elapsed();
                HealthCheckResult::up("redis")
                    .with_detail("version", "7.0.0")
                    .with_detail("mode", "standalone")
                    .with_duration(duration)
            }
            Err(e) => {
                HealthCheckResult::down("redis")
                    .with_error(e.to_string())
                    .with_duration(start.elapsed())
            }
        }
    }

    fn is_critical(&self) -> bool {
        true
    }
}

impl RedisHealthIndicator {
    async fn ping(&self) -> Result<(), String> {
        // Actual Redis PING implementation
        Ok(())
    }
}
```

### External Service Health Indicator

```rust
use armature_core::health::{HealthIndicator, HealthCheckResult, HealthStatus};
use async_trait::async_trait;

struct ExternalApiHealthIndicator {
    name: String,
    url: String,
    timeout_ms: u64,
}

#[async_trait]
impl HealthIndicator for ExternalApiHealthIndicator {
    fn name(&self) -> &str {
        &self.name
    }

    async fn check(&self) -> HealthCheckResult {
        let start = std::time::Instant::now();

        // Simulate HTTP health check to external service
        match self.check_endpoint().await {
            Ok(status_code) => {
                let duration = start.elapsed();

                let status = if status_code >= 200 && status_code < 300 {
                    HealthStatus::Up
                } else if status_code >= 500 {
                    HealthStatus::Down
                } else {
                    HealthStatus::Degraded
                };

                HealthCheckResult {
                    name: self.name.clone(),
                    status,
                    details: [
                        ("url".to_string(), self.url.clone()),
                        ("status_code".to_string(), status_code.to_string()),
                    ].into_iter().collect(),
                    duration_ms: Some(duration.as_millis() as u64),
                    error: None,
                    timestamp: None,
                }
            }
            Err(e) => {
                HealthCheckResult::down(&self.name)
                    .with_detail("url", &self.url)
                    .with_error(e)
                    .with_duration(start.elapsed())
            }
        }
    }

    fn is_critical(&self) -> bool {
        false  // External services usually aren't critical
    }
}

impl ExternalApiHealthIndicator {
    async fn check_endpoint(&self) -> Result<u16, String> {
        // Actual HTTP request implementation
        Ok(200)
    }
}
```

## Configuration

### Health Service Builder

```rust
use armature_core::health::{
    HealthServiceBuilder,
    HealthInfo,
    MemoryHealthIndicator,
    DiskHealthIndicator,
    UptimeHealthIndicator,
};

let health_service = HealthServiceBuilder::new()
    // Add application info
    .with_info(
        HealthInfo::new("my-service")
            .with_version("2.1.0")
            .with_description("Production API service")
    )
    // Add built-in indicators
    .with_indicator(UptimeHealthIndicator::new())
    .with_indicator(MemoryHealthIndicator::with_thresholds(75.0, 90.0))
    .with_indicator(DiskHealthIndicator::new("/").with_thresholds(80.0, 95.0))
    // Add custom indicators
    .with_indicator(DatabaseHealthIndicator::new("postgres://..."))
    .with_indicator(RedisHealthIndicator::new("redis://..."))
    .build();
```

### Runtime Registration

```rust
use armature_core::health::HealthService;
use std::sync::Arc;

let health_service = Arc::new(HealthService::new());

// Register indicators at runtime
health_service.register(DatabaseHealthIndicator::new("...")).await;
health_service.register(RedisHealthIndicator::new("...")).await;

// Set application info
health_service.set_info(
    HealthInfo::new("my-app").with_version("1.0.0")
).await;
```

## API Reference

### HealthStatus

```rust
pub enum HealthStatus {
    Up,       // Component is healthy
    Down,     // Component is not functioning
    Degraded, // Component has issues but operational
    Unknown,  // Status cannot be determined
}
```

### HealthCheckResult

```rust
pub struct HealthCheckResult {
    pub name: String,
    pub status: HealthStatus,
    pub details: HashMap<String, String>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub timestamp: Option<u64>,
}

// Builder methods
HealthCheckResult::up("name")
HealthCheckResult::down("name")
HealthCheckResult::degraded("name")
HealthCheckResult::unknown("name")
    .with_detail("key", "value")
    .with_error("error message")
    .with_duration(Duration::from_millis(10))
```

### HealthIndicator Trait

```rust
#[async_trait]
pub trait HealthIndicator: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self) -> HealthCheckResult;
    fn is_critical(&self) -> bool { false }
    fn include_in_readiness(&self) -> bool { true }
    fn include_in_liveness(&self) -> bool { false }
}
```

### HealthService Methods

```rust
impl HealthService {
    pub fn new() -> Self;
    pub async fn register(&self, indicator: impl HealthIndicator + 'static);
    pub async fn set_info(&self, info: HealthInfo);
    pub async fn check(&self) -> HealthResponse;
    pub async fn check_liveness(&self) -> HealthResponse;
    pub async fn check_readiness(&self) -> HealthResponse;
    pub async fn indicator_count(&self) -> usize;
}
```

## Best Practices

### 1. Separate Liveness and Readiness Concerns

```rust
#[async_trait]
impl HealthIndicator for DatabaseHealthIndicator {
    // Liveness: Don't check external dependencies
    fn include_in_liveness(&self) -> bool {
        false  // Avoid cascading failures
    }

    // Readiness: Check all dependencies
    fn include_in_readiness(&self) -> bool {
        true
    }
}
```

### 2. Use Timeouts for External Checks

```rust
async fn check(&self) -> HealthCheckResult {
    match tokio::time::timeout(
        Duration::from_secs(5),
        self.ping_database()
    ).await {
        Ok(Ok(_)) => HealthCheckResult::up("database"),
        Ok(Err(e)) => HealthCheckResult::down("database").with_error(e.to_string()),
        Err(_) => HealthCheckResult::down("database").with_error("Timeout"),
    }
}
```

### 3. Include Meaningful Details

```rust
HealthCheckResult::up("database")
    .with_detail("type", "postgresql")
    .with_detail("version", "15.0")
    .with_detail("pool_size", "10")
    .with_detail("active_connections", "5")
    .with_detail("latency_ms", "3")
```

### 4. Handle Degraded State

```rust
async fn check(&self) -> HealthCheckResult {
    let latency = self.measure_latency().await;

    if latency > Duration::from_secs(5) {
        HealthCheckResult::degraded("api")
            .with_detail("latency_ms", latency.as_millis().to_string())
            .with_detail("threshold_ms", "5000")
    } else {
        HealthCheckResult::up("api")
            .with_detail("latency_ms", latency.as_millis().to_string())
    }
}
```

### 5. Mark Critical Dependencies

```rust
fn is_critical(&self) -> bool {
    // Only mark truly critical dependencies
    // If this fails, the app cannot function
    true
}
```

## Summary

The health check module provides:

- **Three probe types**: Full health, liveness, and readiness checks
- **Built-in indicators**: Memory, disk, and uptime monitoring
- **Custom indicators**: Easy-to-implement trait for any dependency
- **Kubernetes compatible**: Standard JSON responses and HTTP status codes
- **Concurrent execution**: All health checks run in parallel
- **Configurable thresholds**: Customize degraded/critical levels

Use health checks to:
- Enable Kubernetes automatic restarts and traffic management
- Monitor application dependencies
- Integrate with observability platforms
- Implement graceful degradation

```rust
// Quick setup with all defaults
let health = HealthServiceBuilder::new()
    .with_defaults()
    .with_info(HealthInfo::new("my-app").with_version("1.0.0"))
    .build();
```

