# Scaling Guide

This guide covers strategies for scaling Armature applications to handle increased load.

## Table of Contents

- [Overview](#overview)
- [Horizontal Scaling](#horizontal-scaling)
- [Vertical Scaling](#vertical-scaling)
- [Load Balancing](#load-balancing)
- [Database Scaling](#database-scaling)
- [Caching Strategies](#caching-strategies)
- [Async Processing](#async-processing)
- [Performance Optimization](#performance-optimization)
- [Monitoring for Scale](#monitoring-for-scale)

## Overview

Armature applications can scale to handle millions of requests by:

1. **Horizontal Scaling** - Adding more instances
2. **Vertical Scaling** - Increasing instance resources
3. **Load Balancing** - Distributing traffic evenly
4. **Caching** - Reducing database load
5. **Async Processing** - Offloading heavy operations

## Horizontal Scaling

### Stateless Design

Armature encourages stateless architecture for easy horizontal scaling:

```rust
// ❌ Bad: Server-side session storage
struct UserController {
    sessions: HashMap<String, Session>, // Can't scale!
}

// ✅ Good: JWT-based stateless auth
struct UserController {
    jwt_manager: JwtManager, // Stateless, can scale!
}
```

### Using Ferron for Load Balancing

```rust
use armature_ferron::{FerronConfig, LoadBalancer, LoadBalanceStrategy, Backend};

let config = FerronConfig::builder()
    .domain("api.example.com")
    .load_balancer(
        LoadBalancer::new()
            .strategy(LoadBalanceStrategy::RoundRobin)
            .backend(Backend::new("http://app1:3000"))
            .backend(Backend::new("http://app2:3000"))
            .backend(Backend::new("http://app3:3000"))
            .backend(Backend::new("http://app4:3000"))
            .health_check_interval(10)
            .health_check_path("/health")
    )
    .build()?;
```

### Service Discovery

Dynamic backend registration for auto-scaling:

```rust
use armature_ferron::ServiceRegistry;

let registry = ServiceRegistry::new();

// Register new instances as they come online
let id = registry.register("api-service", "http://new-instance:3000").await?;

// Deregister when scaling down
registry.deregister("api-service", &id).await?;

// Get current backends for load balancer
let backends = registry.get_healthy_urls("api-service").await;
```

### Kubernetes HPA

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: armature-api
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: armature-api
  minReplicas: 3
  maxReplicas: 50
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
```

## Vertical Scaling

### Rust Performance Advantages

Armature benefits from Rust's efficiency:

- **Low memory footprint** - No garbage collector overhead
- **High CPU utilization** - Zero-cost abstractions
- **Predictable latency** - No GC pauses

### Optimal Resource Allocation

```yaml
# Kubernetes resource limits
resources:
  limits:
    cpu: "4"
    memory: "2Gi"
  requests:
    cpu: "1"
    memory: "512Mi"
```

### Tokio Runtime Tuning

```rust
#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    // Configure based on CPU cores
}
```

## Load Balancing

### Load Balancing Strategies

```rust
use armature_ferron::LoadBalanceStrategy;

// Round Robin - Equal distribution
LoadBalanceStrategy::RoundRobin

// Least Connections - Route to least busy
LoadBalanceStrategy::LeastConnections

// IP Hash - Sticky sessions without state
LoadBalanceStrategy::IpHash

// Weighted - Based on server capacity
LoadBalanceStrategy::Weighted
```

### Weighted Load Balancing

```rust
let lb = LoadBalancer::new()
    .strategy(LoadBalanceStrategy::Weighted)
    .backend(Backend::new("http://big-server:3000").weight(10))
    .backend(Backend::new("http://small-server:3000").weight(5));
```

## Database Scaling

### Connection Pooling

```rust
use sqlx::postgres::PgPoolOptions;

let pool = PgPoolOptions::new()
    .max_connections(100)  // Max connections per instance
    .min_connections(10)   // Keep 10 warm
    .acquire_timeout(Duration::from_secs(5))
    .idle_timeout(Duration::from_secs(600))
    .connect(&database_url)
    .await?;
```

### Read Replicas

```rust
// Write to primary
let primary = PgPool::connect("postgres://primary:5432/db").await?;

// Read from replicas
let replicas = vec![
    PgPool::connect("postgres://replica1:5432/db").await?,
    PgPool::connect("postgres://replica2:5432/db").await?,
];

// Route queries appropriately
async fn get_user(id: u32, replicas: &[PgPool]) -> Result<User, Error> {
    let replica = &replicas[id as usize % replicas.len()];
    sqlx::query_as("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(replica)
        .await
}
```

## Caching Strategies

### Multi-Tier Caching

```rust
use armature_cache::{TieredCache, InMemoryCache, RedisCache};

let cache = TieredCache::new()
    .l1(InMemoryCache::new(1000)) // Fast, local
    .l2(RedisCache::new(redis_url)) // Shared, persistent
    .build();

// Automatic fallthrough
let user = cache.get_or_set("user:123", || {
    fetch_user_from_db(123)
}).await?;
```

### Cache Invalidation

```rust
use armature_cache::TaggedCache;

let cache = TaggedCache::new(redis_url);

// Set with tags
cache.set_with_tags(
    "user:123",
    user,
    &["users", "user:123"],
    Duration::from_secs(3600)
).await?;

// Invalidate by tag
cache.invalidate_tag("users").await?; // Clears all user caches
```

## Async Processing

### Job Queues

Offload heavy operations to background workers:

```rust
use armature_queue::{Queue, Job};

// Enqueue job instead of processing synchronously
queue.enqueue("send_email", EmailJob {
    to: user.email,
    subject: "Welcome!",
    template: "welcome",
}).await?;

// Return immediately to client
HttpResponse::accepted()
```

### Worker Scaling

```yaml
# Scale workers independently
apiVersion: apps/v1
kind: Deployment
metadata:
  name: email-worker
spec:
  replicas: 5  # Scale workers based on queue depth
```

## Performance Optimization

### Response Compression

```rust
use armature_framework::compression::CompressionConfig;

let config = CompressionConfig::default()
    .gzip(true)
    .brotli(true)
    .min_size(1024); // Only compress > 1KB
```

### Connection Keep-Alive

```rust
// Ferron handles this automatically
let config = FerronConfig::builder()
    .keepalive(true)
    .keepalive_timeout(60)
    .build()?;
```

### Request Batching

```rust
// GraphQL DataLoader pattern
#[routes]
impl UserController {
    #[post("/users/batch")]
    async fn batch_get(req: HttpRequest) -> Result<HttpResponse, Error> {
        let ids: Vec<u32> = req.json()?;

        // Single query for all users
        let users = sqlx::query_as("SELECT * FROM users WHERE id = ANY($1)")
            .bind(&ids)
            .fetch_all(&pool)
            .await?;

        HttpResponse::json(&users)
    }
}
```

## Monitoring for Scale

### Key Metrics

```rust
use armature_framework::metrics::{counter, histogram};

// Track request rate
counter!("http_requests_total", "endpoint" => path);

// Track latency
histogram!("http_request_duration_seconds", duration);

// Track errors
counter!("http_errors_total", "status" => status.to_string());
```

### Alerting Thresholds

```yaml
# Prometheus alerting rules
groups:
  - name: scaling
    rules:
      - alert: HighRequestRate
        expr: rate(http_requests_total[5m]) > 10000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Consider scaling up"

      - alert: HighLatency
        expr: histogram_quantile(0.95, http_request_duration_seconds) > 1
        for: 5m
        labels:
          severity: critical
```

### Load Testing

```bash
# Using oha (Ohayou HTTP load generator)
oha -n 100000 -c 100 http://localhost:3000/api/users

# Using wrk
wrk -t12 -c400 -d30s http://localhost:3000/api/users
```

## Summary

Scaling checklist:

1. **Design stateless** - Enable horizontal scaling
2. **Use Ferron** - High-performance load balancing
3. **Enable service discovery** - Dynamic backend management
4. **Configure HPA** - Automatic pod scaling
5. **Pool connections** - Efficient database usage
6. **Implement caching** - Reduce database load
7. **Use job queues** - Offload heavy operations
8. **Monitor metrics** - Data-driven scaling decisions

