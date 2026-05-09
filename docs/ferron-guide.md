# Ferron Reverse Proxy Integration

Armature provides first-class integration with [Ferron](https://ferron.sh), a modern, high-performance reverse proxy server written in Rust. The `armature-ferron` crate enables you to generate Ferron configurations, manage proxy processes, implement health checking, and enable dynamic service discovery.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Configuration Generation](#configuration-generation)
- [Load Balancing](#load-balancing)
- [Service Discovery](#service-discovery)
- [Health Checking](#health-checking)
- [Process Management](#process-management)
- [Production Deployment](#production-deployment)
- [API Reference](#api-reference)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

## Overview

Ferron is a modern web server and reverse proxy designed for:
- **High Performance**: Written in Rust for speed and memory safety
- **Simple Configuration**: Uses KDL (KDL Document Language) for intuitive setup
- **Automatic TLS**: Built-in Let's Encrypt integration
- **Security**: Memory-safe with built-in security headers

The `armature-ferron` crate bridges Armature applications with Ferron, providing:
- Programmatic configuration generation
- Dynamic backend registration
- Health check integration
- Process lifecycle management

## Features

- ✅ **Configuration Generation** - Generate Ferron KDL configs from Rust code
- ✅ **Load Balancing** - Round-robin, least connections, IP hash, weighted
- ✅ **Service Discovery** - Dynamic backend registration and deregistration
- ✅ **Health Checking** - HTTP health checks with thresholds
- ✅ **Process Management** - Start, stop, restart, and reload Ferron
- ✅ **TLS Configuration** - Automatic and manual certificate management
- ✅ **Rate Limiting** - Per-location rate limit configuration
- ✅ **Security Headers** - Automatic security header injection

## Installation

Add `armature-ferron` to your `Cargo.toml`:

```toml
[dependencies]
armature-ferron = "0.1"
```

Or with specific features:

```toml
[dependencies]
armature-ferron = { version = "0.1", features = ["native-tls"] }
```

### System Requirements

- **Ferron**: Install from [ferron.sh](https://ferron.sh) for process management features
- **Rust**: 1.70 or later

## Quick Start

### Basic Reverse Proxy

```rust
use armature_ferron::{FerronConfig, Backend};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a simple reverse proxy configuration
    let config = FerronConfig::builder()
        .domain("api.example.com")
        .backend_url("http://localhost:3000")
        .tls_auto(true)
        .gzip(true)
        .build()?;

    // Generate the KDL configuration
    let kdl = config.to_kdl()?;
    println!("{}", kdl);

    // Output:
    // api.example.com {
    //     tls auto
    //     hsts max_age=31536000
    //     gzip level=6
    //     proxy "http://localhost:3000"
    // }

    Ok(())
}
```

### With Armature Application

```rust
use armature_framework::prelude::*;
use armature_ferron::{FerronConfig, Location, RateLimitConfig};

#[controller("/api")]
#[derive(Default, Clone)]
struct ApiController;

#[routes]
impl ApiController {
    #[get("/users")]
    async fn get_users() -> Result<HttpResponse, Error> {
        HttpResponse::json(&vec!["Alice", "Bob"])
    }

    #[get("/health")]
    async fn health() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({"status": "healthy"}))
    }
}

#[module(controllers: [ApiController])]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = 3000;

    // Generate Ferron configuration
    let ferron_config = FerronConfig::builder()
        .domain("api.example.com")
        .backend_url(format!("http://127.0.0.1:{}", port))
        .location(
            Location::new("/api")
                .proxy(format!("http://127.0.0.1:{}/api", port))
                .rate_limit(RateLimitConfig::new(100).burst(200))
        )
        .tls_auto(true)
        .header("X-Frame-Options", "DENY")
        .build()?;

    // Save configuration
    ferron_config.write_to_file("/etc/ferron/ferron.conf").await?;

    // Start Armature server
    let app = Application::create::<AppModule>().await;
    app.listen(port).await?;

    Ok(())
}
```

## Configuration Generation

### Domain Configuration

```rust
use armature_ferron::FerronConfig;

// Single domain
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .build()?;

// Multiple domains (aliases)
let config = FerronConfig::builder()
    .domains(vec!["example.com", "www.example.com"])
    .backend_url("http://localhost:3000")
    .build()?;
```

### TLS Configuration

```rust
use armature_ferron::{FerronConfig, TlsConfig};

// Automatic TLS with Let's Encrypt
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .tls_auto(true)
    .build()?;

// Manual TLS with certificates
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .tls(
        TlsConfig::manual("/path/to/cert.pem", "/path/to/key.pem")
            .hsts(true)
            .hsts_max_age(31536000)
            .min_version("1.2")
    )
    .build()?;

// Automatic TLS with email
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .tls(
        TlsConfig::auto()
            .email("admin@example.com")
    )
    .build()?;
```

### Location-Based Routing

```rust
use armature_ferron::{FerronConfig, Location, RateLimitConfig};

let config = FerronConfig::builder()
    .domain("example.com")
    .backend_url("http://localhost:3000")
    // API with rate limiting
    .location(
        Location::new("/api")
            .remove_base(true)
            .proxy("http://localhost:3000/api")
            .rate_limit(RateLimitConfig::new(100).burst(200))
    )
    // Static files
    .location(
        Location::new("/static")
            .root("/var/www/static")
    )
    // WebSocket endpoint
    .location(
        Location::new("/ws")
            .proxy("http://localhost:3000/ws")
    )
    .build()?;
```

### Proxy Routes (Simplified)

```rust
use armature_ferron::{FerronConfig, ProxyRoute};

let config = FerronConfig::builder()
    .domain("example.com")
    .backend_url("http://localhost:3000")
    .route(
        ProxyRoute::new("/api", "http://localhost:3000/api")
            .strip_prefix()
            .timeout(30)
    )
    .route(
        ProxyRoute::new("/ws", "http://localhost:3000/ws")
            .websocket()
    )
    .build()?;
```

### Security Headers

```rust
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .header("X-Frame-Options", "DENY")
    .header("X-Content-Type-Options", "nosniff")
    .header("X-XSS-Protection", "1; mode=block")
    .header("Referrer-Policy", "strict-origin-when-cross-origin")
    .header("Content-Security-Policy", "default-src 'self'")
    .build()?;
```

### Compression

```rust
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .gzip(true)
    .gzip_level(6) // 1-9, default is 6
    .build()?;
```

## Load Balancing

### Basic Load Balancing

```rust
use armature_ferron::{FerronConfig, Backend, LoadBalancer, LoadBalanceStrategy};

let config = FerronConfig::builder()
    .domain("api.example.com")
    .load_balancer(
        LoadBalancer::new()
            .strategy(LoadBalanceStrategy::RoundRobin)
            .backend(Backend::new("http://backend1:3000"))
            .backend(Backend::new("http://backend2:3000"))
            .backend(Backend::new("http://backend3:3000"))
    )
    .build()?;
```

### Load Balancing Strategies

```rust
use armature_ferron::LoadBalanceStrategy;

// Round-robin (default) - Distribute evenly
LoadBalanceStrategy::RoundRobin

// Least connections - Route to backend with fewest active connections
LoadBalanceStrategy::LeastConnections

// IP hash - Consistent routing based on client IP
LoadBalanceStrategy::IpHash

// Random - Random selection
LoadBalanceStrategy::Random

// Weighted - Based on backend weights
LoadBalanceStrategy::Weighted
```

### Weighted Backends

```rust
let config = FerronConfig::builder()
    .domain("api.example.com")
    .load_balancer(
        LoadBalancer::new()
            .strategy(LoadBalanceStrategy::Weighted)
            .backend(Backend::new("http://backend1:3000").weight(5)) // 50% traffic
            .backend(Backend::new("http://backend2:3000").weight(3)) // 30% traffic
            .backend(Backend::new("http://backend3:3000").weight(2)) // 20% traffic
    )
    .build()?;
```

### Backend Configuration

```rust
use armature_ferron::Backend;

let backend = Backend::new("http://localhost:3000")
    .weight(3)                    // Load balancing weight
    .max_connections(100)         // Maximum connections
    .timeout(30)                  // Connection timeout in seconds
    .backup()                     // Mark as backup (used when primaries fail)
    .header("X-Real-IP", "$remote_addr"); // Custom headers
```

### Health Checks for Load Balancing

```rust
let config = FerronConfig::builder()
    .domain("api.example.com")
    .load_balancer(
        LoadBalancer::new()
            .strategy(LoadBalanceStrategy::RoundRobin)
            .backend(Backend::new("http://backend1:3000"))
            .backend(Backend::new("http://backend2:3000"))
            .health_check_interval(30)      // Check every 30 seconds
            .health_check_path("/health")   // Health check endpoint
            .health_check_threshold(3)      // Failures before unhealthy
    )
    .build()?;
```

## Service Discovery

### Basic Registration

```rust
use armature_ferron::ServiceRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let registry = ServiceRegistry::new();

    // Register service instances
    let id1 = registry.register("api-service", "http://localhost:3001").await?;
    let id2 = registry.register("api-service", "http://localhost:3002").await?;
    let id3 = registry.register("api-service", "http://localhost:3003").await?;

    // Get all instances
    let instances = registry.get_instances("api-service").await;
    println!("Total instances: {}", instances.len());

    // Get URLs
    let urls = registry.get_urls("api-service").await;
    println!("Backend URLs: {:?}", urls);

    // Deregister an instance
    registry.deregister("api-service", &id1).await?;

    Ok(())
}
```

### Service Instance Configuration

```rust
use armature_ferron::{ServiceRegistry, ServiceInstance};

let registry = ServiceRegistry::new();

// Register with full configuration
let instance = ServiceInstance::new("api-service", "http://localhost:3000")
    .weight(3)
    .metadata("version", "1.0.0")
    .metadata("region", "us-west-2")
    .tag("production")
    .tag("v1");

let id = registry.register_instance(instance).await?;
```

### Health Status Management

```rust
// Mark instance as unhealthy
registry.mark_unhealthy("api-service", &instance_id).await?;

// Mark instance as healthy again
registry.mark_healthy("api-service", &instance_id).await?;

// Get only healthy instances
let healthy = registry.get_healthy_instances("api-service").await;
let healthy_urls = registry.get_healthy_urls("api-service").await;
```

### Heartbeat and Stale Removal

```rust
use std::sync::Arc;
use chrono::Duration;

let registry = Arc::new(ServiceRegistry::new());

// Send heartbeat to keep instance alive
registry.heartbeat("api-service", &instance_id).await?;

// Start background cleanup (removes stale instances)
let cleanup_handle = registry.clone().start_cleanup(
    std::time::Duration::from_secs(60),  // Check every 60 seconds
    Duration::seconds(300),               // Remove after 5 minutes of no heartbeat
);
```

### Registry Statistics

```rust
let stats = registry.stats().await;
println!("Services: {}", stats.service_count);
println!("Total instances: {}", stats.total_instances);
println!("Healthy: {}", stats.healthy_instances);
println!("Unhealthy: {}", stats.unhealthy_instances);
```

### Change Notifications

```rust
registry.on_change(|service_name| {
    println!("Service {} changed", service_name);
    // Regenerate Ferron config, notify other systems, etc.
}).await;
```

## Health Checking

### Basic Health Check

```rust
use armature_ferron::{HealthCheckConfig, HealthState, HttpHealthChecker};
use std::time::Duration;
use std::sync::Arc;

let config = HealthCheckConfig::new()
    .path("/health")
    .method("GET")
    .expected_status(vec![200])
    .timeout(Duration::from_secs(5))
    .interval(Duration::from_secs(30))
    .unhealthy_threshold(3)
    .healthy_threshold(2);

let health_state = Arc::new(HealthState::new(config));

// Check a backend
let result = health_state.check_backend("http://localhost:3000").await;
println!("Status: {:?}", result.status);
println!("Response time: {:?}ms", result.response_time_ms);
```

### Health Check Configuration

```rust
use armature_ferron::HealthCheckConfig;
use std::time::Duration;

let config = HealthCheckConfig::new()
    .path("/health")              // Health endpoint
    .method("GET")                // HTTP method (GET, HEAD, POST)
    .expected_status(vec![200, 204]) // Expected status codes
    .timeout(Duration::from_secs(5)) // Request timeout
    .interval(Duration::from_secs(30)) // Check interval
    .unhealthy_threshold(3)       // Failures before unhealthy
    .healthy_threshold(2)         // Successes before healthy
    .header("Authorization", "Bearer token"); // Custom headers
```

### Background Health Checking

```rust
use armature_ferron::HealthState;
use std::sync::Arc;

let health_state = Arc::new(HealthState::new(HealthCheckConfig::default()));

let backends = vec![
    "http://backend1:3000".to_string(),
    "http://backend2:3000".to_string(),
    "http://backend3:3000".to_string(),
];

// Start background health checking
let handle = health_state.clone()
    .start_background_checks(backends)
    .await?;

// Later: get current health status
let results = health_state.get_all_results().await;
for (url, result) in results {
    println!("{}: {:?}", url, result.status);
}

// Get only healthy backends
let healthy = health_state.get_healthy_backends().await;
```

### Health Status Types

```rust
use armature_ferron::HealthStatus;

match status {
    HealthStatus::Healthy => println!("Backend is healthy"),
    HealthStatus::Degraded => println!("Backend is degraded but functioning"),
    HealthStatus::Unhealthy => println!("Backend should not receive traffic"),
    HealthStatus::Unknown => println!("Health status not yet determined"),
}

// Check if backend can receive traffic
if status.is_available() {
    // Route traffic to this backend
}
```

## Process Management

### Basic Process Control

```rust
use armature_ferron::{FerronProcess, ProcessConfig};

let config = ProcessConfig::new("/usr/bin/ferron", "/etc/ferron/ferron.conf");

let process = FerronProcess::new(config);

// Start Ferron
process.start().await?;
println!("PID: {:?}", process.pid().await);

// Check status
let status = process.status().await;
println!("Status: {:?}", status);

// Reload configuration (SIGHUP)
process.reload().await?;

// Restart
process.restart().await?;

// Stop
process.stop().await?;
```

### Process Configuration

```rust
use armature_ferron::ProcessConfig;

let config = ProcessConfig::new("/usr/bin/ferron", "/etc/ferron/ferron.conf")
    .working_dir("/var/www")
    .arg("--verbose")
    .env("RUST_LOG", "debug")
    .pid_file("/var/run/ferron.pid")
    .stdout_log("/var/log/ferron/stdout.log")
    .stderr_log("/var/log/ferron/stderr.log")
    .auto_restart(true)
    .max_restarts(5)
    .restart_delay(2000); // 2 seconds
```

### Supervised Process

```rust
use armature_ferron::FerronProcess;
use std::sync::Arc;

let process = Arc::new(FerronProcess::new(config));

// Start with automatic restart on crash
let supervision_handle = process.clone()
    .start_with_supervision()
    .await?;

// Process will automatically restart up to max_restarts times
```

### Check Ferron Installation

```rust
use armature_ferron::process::check_ferron_installed;

match check_ferron_installed(None).await {
    Ok(version) => println!("Ferron version: {}", version),
    Err(e) => println!("Ferron not found: {}", e),
}

// Check specific path
check_ferron_installed(Some(Path::new("/usr/local/bin/ferron"))).await?;
```

## Production Deployment

### Complete Manager Setup

```rust
use armature_ferron::{
    FerronManager, FerronConfig, ServiceRegistry, HealthCheckConfig
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create configuration
    let config = FerronConfig::builder()
        .domain("api.example.com")
        .backend_url("http://localhost:3000")
        .tls_auto(true)
        .build()?;

    // Create manager with all features
    let manager = FerronManager::builder()
        .binary_path("/usr/bin/ferron")
        .config_path("/etc/ferron/ferron.conf")
        .config(config)
        .service_registry(ServiceRegistry::new())
        .health_check(HealthCheckConfig::default())
        .auto_reload(true)
        .auto_restart(true)
        .build()?;

    // Start Ferron with supervision
    let handle = Arc::new(manager)
        .start_supervised()
        .await?;

    // Register a backend
    manager.register_backend("api", "http://localhost:3001").await?;

    // Deregister when scaling down
    manager.deregister_backend("api", &instance_id).await?;

    Ok(())
}
```

### Helper Functions

```rust
use armature_ferron::manager::helpers;

// Simple reverse proxy
let config = helpers::reverse_proxy_config(
    "api.example.com",
    "http://localhost:3000"
)?;

// Load-balanced configuration
let config = helpers::load_balanced_config(
    "api.example.com",
    vec!["http://backend1:3000", "http://backend2:3000"]
)?;

// Armature-optimized configuration
let config = helpers::armature_app_config(
    "api.example.com",
    3000  // Armature app port
)?;
```

### Docker Compose Example

```yaml
version: '3.8'
services:
  ferron:
    image: ferronweb/ferron:latest
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./ferron.conf:/etc/ferron/ferron.conf
      - ferron-certs:/var/lib/ferron/certs
    depends_on:
      - app

  app:
    build: .
    expose:
      - "3000"
    environment:
      - RUST_LOG=info
    deploy:
      replicas: 3

volumes:
  ferron-certs:
```

### Kubernetes Integration

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: ferron-config
data:
  ferron.conf: |
    api.example.com {
        tls auto
        proxy "http://app-service:3000"
        lb_method "round_robin"
        lb_health_check interval=10 path="/health"
    }
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ferron
spec:
  replicas: 2
  template:
    spec:
      containers:
        - name: ferron
          image: ferronweb/ferron:latest
          ports:
            - containerPort: 80
            - containerPort: 443
          volumeMounts:
            - name: config
              mountPath: /etc/ferron
      volumes:
        - name: config
          configMap:
            name: ferron-config
```

## API Reference

### Core Types

| Type | Description |
|------|-------------|
| `FerronConfig` | Main configuration builder |
| `Backend` | Backend server configuration |
| `LoadBalancer` | Load balancing configuration |
| `Location` | Path-based routing configuration |
| `ProxyRoute` | Simplified proxy route |
| `TlsConfig` | TLS/HTTPS configuration |
| `RateLimitConfig` | Rate limiting configuration |

### Service Discovery

| Type | Description |
|------|-------------|
| `ServiceRegistry` | Service instance registry |
| `ServiceInstance` | Registered service instance |
| `RegistryStats` | Registry statistics |

### Health Checking

| Type | Description |
|------|-------------|
| `HealthState` | Health state tracker |
| `HealthCheckConfig` | Health check configuration |
| `HealthCheckResult` | Health check result |
| `HealthStatus` | Health status enum |

### Process Management

| Type | Description |
|------|-------------|
| `FerronProcess` | Process handle |
| `ProcessConfig` | Process configuration |
| `ProcessStatus` | Process status enum |
| `FerronManager` | High-level manager |

## Best Practices

### 1. Always Use Health Checks

```rust
// Configure health checks for reliability
let config = FerronConfig::builder()
    .domain("api.example.com")
    .load_balancer(
        LoadBalancer::new()
            .backend(Backend::new("http://backend1:3000"))
            .backend(Backend::new("http://backend2:3000"))
            .health_check_interval(30)
            .health_check_path("/health")
            .health_check_threshold(3)
    )
    .build()?;
```

### 2. Implement Proper Health Endpoints

```rust
#[get("/health")]
async fn health() -> Result<HttpResponse, Error> {
    // Check dependencies
    let db_ok = check_database().await;
    let cache_ok = check_cache().await;

    if db_ok && cache_ok {
        HttpResponse::json(&serde_json::json!({
            "status": "healthy",
            "checks": {
                "database": "ok",
                "cache": "ok"
            }
        }))
    } else {
        Err(Error::internal("Health check failed"))
    }
}
```

### 3. Use Security Headers

```rust
let config = FerronConfig::builder()
    .domain("api.example.com")
    .backend_url("http://localhost:3000")
    .header("X-Frame-Options", "DENY")
    .header("X-Content-Type-Options", "nosniff")
    .header("X-XSS-Protection", "1; mode=block")
    .header("Strict-Transport-Security", "max-age=31536000; includeSubDomains")
    .build()?;
```

### 4. Rate Limit API Endpoints

```rust
.location(
    Location::new("/api")
        .proxy("http://localhost:3000/api")
        .rate_limit(RateLimitConfig::new(100).burst(200).key("ip"))
)
```

### 5. Configure Backup Backends

```rust
.load_balancer(
    LoadBalancer::new()
        .backend(Backend::new("http://primary1:3000"))
        .backend(Backend::new("http://primary2:3000"))
        .backend(Backend::new("http://backup:3000").backup())
)
```

## Troubleshooting

### Common Issues

**Configuration not generating correctly:**
```rust
// Validate configuration before generating
config.validate()?;
let kdl = config.to_kdl()?;
```

**Health checks failing:**
```rust
// Check if endpoint is accessible
let result = health_state.check_backend("http://localhost:3000").await;
if let Some(error) = result.error {
    println!("Health check error: {}", error);
}
```

**Process not starting:**
```rust
// Check Ferron installation
match check_ferron_installed(None).await {
    Ok(version) => println!("Ferron {}", version),
    Err(e) => println!("Error: {}", e),
}

// Validate configuration file exists
config.validate()?;
```

### Debug Logging

```rust
// Enable debug logging
std::env::set_var("RUST_LOG", "armature_ferron=debug");
tracing_subscriber::fmt::init();
```

## Summary

The `armature-ferron` crate provides:

1. **Configuration Generation** - Programmatically generate Ferron configs
2. **Load Balancing** - Multiple strategies with weighted backends
3. **Service Discovery** - Dynamic backend registration
4. **Health Checking** - HTTP-based health monitoring
5. **Process Management** - Full lifecycle control

Key benefits:
- **Type Safety** - Catch configuration errors at compile time
- **Dynamic Updates** - Change backends without restart
- **Monitoring** - Built-in health checking
- **Automation** - Programmatic control over Ferron

For more examples, see the `examples/ferron_proxy.rs` and `examples/ferron_integration.rs` files in the Armature repository.

