# Prometheus Metrics Guide

Comprehensive guide to collecting and exposing metrics in Armature using Prometheus.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Metric Types](#metric-types)
- [Request Metrics](#request-metrics)
- [Business Metrics](#business-metrics)
- [Labels](#labels)
- [Custom Buckets](#custom-buckets)
- [Metrics Endpoint](#metrics-endpoint)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

---

## Overview

Armature provides built-in Prometheus metrics support through the `armature-metrics` crate. This allows you to:

- Collect HTTP request metrics automatically
- Create custom business metrics
- Expose metrics via `/metrics` endpoint
- Integrate with Prometheus/Grafana

---

## Features

- ✅ **Prometheus Integration** - Native Prometheus client
- ✅ **Auto HTTP Metrics** - Request count, latency, errors
- ✅ **Multiple Metric Types** - Counter, Gauge, Histogram
- ✅ **Labels** - Multi-dimensional metrics
- ✅ **Custom Buckets** - Configurable histogram buckets
- ✅ **`/metrics` Endpoint** - Standard Prometheus endpoint
- ✅ **Business Metrics** - Easy custom metric registration

---

## Quick Start

### 1. Add Dependency

```toml
[dependencies]
armature-metrics = "0.1"
```

### 2. Create Metrics

```rust
use armature_metrics::*;

// Counter
let requests = register_counter("requests_total", "Total requests")?;
requests.inc();

// Gauge
let active_users = register_gauge("active_users", "Active users")?;
active_users.set(42.0);

// Histogram
let latency = register_histogram("request_duration_seconds", "Request duration")?;
latency.observe(0.5);
```

### 3. Add Metrics Endpoint

```rust
use armature_core::*;
use armature_metrics::*;

let mut router = Router::new();

// Add /metrics endpoint
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/metrics".to_string(),
    handler: create_metrics_handler(),
    constraints: None,
});
```

### 4. Add Request Metrics Middleware

```rust
use std::sync::Arc;

let metrics_middleware = Arc::new(RequestMetricsMiddleware::new());

let app = Application::new()
    .router(router)
    .middleware(metrics_middleware)
    .build();
```

---

## Metric Types

### Counter

Counters only increase over time. Use for counting events.

```rust
use armature_metrics::*;

// Simple counter
let counter = register_counter("page_views", "Page views")?;
counter.inc();
counter.inc_by(5.0);

// Counter with builder
let counter = CounterBuilder::new("requests_total", "Total requests")
    .register()?;
```

**Use cases:**
- Request counts
- Error counts
- Event counts

### Gauge

Gauges can increase and decrease. Use for current state.

```rust
use armature_metrics::*;

// Simple gauge
let gauge = register_gauge("temperature", "Temperature")?;
gauge.set(72.5);
gauge.inc();
gauge.dec();
gauge.add(10.0);
gauge.sub(5.0);

// Gauge with builder
let gauge = GaugeBuilder::new("active_connections", "Active connections")
    .register()?;
```

**Use cases:**
- Active connections
- Queue sizes
- Memory usage

### Histogram

Histograms sample observations and count them in buckets.

```rust
use armature_metrics::*;

// Simple histogram
let histogram = register_histogram("request_duration", "Request duration")?;
histogram.observe(0.5);

// Histogram with builder
let histogram = HistogramBuilder::new("api_latency", "API latency")
    .latency_buckets()  // Use default latency buckets
    .register()?;

// Custom buckets
let histogram = HistogramBuilder::new("response_size", "Response size")
    .buckets(vec![100.0, 1000.0, 10000.0, 100000.0])
    .register()?;
```

**Use cases:**
- Request latency
- Response sizes
- Processing times

---

## Request Metrics

The `RequestMetricsMiddleware` automatically collects HTTP metrics.

### Automatic Metrics

When you add the middleware, these metrics are collected automatically:

| Metric | Type | Description |
|--------|------|-------------|
| `http_requests_total` | Counter | Total HTTP requests |
| `http_request_duration_seconds` | Histogram | Request latency |
| `http_requests_in_flight` | Gauge | Active requests |
| `http_request_size_bytes` | Histogram | Request size |
| `http_response_size_bytes` | Histogram | Response size |

### Basic Usage

```rust
use armature_core::*;
use armature_metrics::*;
use std::sync::Arc;

let metrics_middleware = Arc::new(RequestMetricsMiddleware::new());

let app = Application::new()
    .middleware(metrics_middleware)
    .build();
```

### Without Path Labels

To reduce cardinality in high-traffic applications:

```rust
let metrics_middleware = Arc::new(RequestMetricsMiddleware::without_path());
```

This groups all paths into a single label value.

---

## Business Metrics

Create custom metrics for your business logic.

### Example: E-commerce Metrics

```rust
use armature_metrics::*;

// Order metrics
let orders = CounterVecBuilder::new("orders_total", "Total orders")
    .labels(&["status", "payment_method"])
    .register()?;

orders.with_label_values(&["completed", "credit_card"]).inc();
orders.with_label_values(&["failed", "paypal"]).inc();

// Revenue tracking
let revenue = CounterBuilder::new("revenue_dollars", "Revenue in dollars")
    .register()?;

revenue.inc_by(99.99);

// Cart size distribution
let cart_size = HistogramBuilder::new("cart_items", "Items in cart")
    .buckets(vec![1.0, 5.0, 10.0, 20.0, 50.0])
    .register()?;

cart_size.observe(3.0);
```

### Example: Database Metrics

```rust
use armature_metrics::*;

// Query duration by operation
let query_duration = HistogramVecBuilder::new(
    "db_query_duration_seconds",
    "Database query duration"
)
    .labels(&["operation", "table"])
    .buckets(vec![0.001, 0.01, 0.1, 0.5, 1.0])
    .register()?;

query_duration.with_label_values(&["SELECT", "users"]).observe(0.05);
query_duration.with_label_values(&["INSERT", "orders"]).observe(0.02);

// Connection pool metrics
let db_connections = GaugeVecBuilder::new(
    "db_connections",
    "Database connections"
)
    .labels(&["pool", "state"])
    .register()?;

db_connections.with_label_values(&["default", "active"]).set(10.0);
db_connections.with_label_values(&["default", "idle"]).set(5.0);
```

---

## Labels

Labels add dimensions to metrics for filtering and grouping.

### Adding Labels

```rust
use armature_metrics::*;

// Counter with labels
let requests = CounterVecBuilder::new("http_requests", "HTTP requests")
    .labels(&["method", "endpoint", "status"])
    .register()?;

requests.with_label_values(&["GET", "/api/users", "200"]).inc();
requests.with_label_values(&["POST", "/api/orders", "201"]).inc();
requests.with_label_values(&["GET", "/api/users", "500"]).inc();
```

### Label Best Practices

**✅ Good:**
- Use a limited set of label values
- Use labels for dimensions you need to query
- Keep cardinality manageable

```rust
// Good - limited values
let requests = CounterVecBuilder::new("requests", "Requests")
    .labels(&["method", "status_class"])  // GET/POST/PUT, 2xx/3xx/4xx/5xx
    .register()?;
```

**❌ Bad:**
- Don't use unbounded label values
- Avoid high-cardinality labels

```rust
// BAD - unbounded values
let requests = CounterVecBuilder::new("requests", "Requests")
    .labels(&["user_id", "timestamp"])  // Millions of combinations!
    .register()?;
```

---

## Custom Buckets

Histograms use buckets to count observations. Choose appropriate buckets for your use case.

### Default Buckets

```rust
use armature_metrics::*;

// Latency buckets (milliseconds to seconds)
let histogram = HistogramBuilder::new("latency", "Latency")
    .latency_buckets()  // 0.001, 0.005, 0.01, ..., 10.0
    .register()?;

// Size buckets (bytes)
let histogram = HistogramBuilder::new("size", "Size")
    .size_buckets()  // 100, 1000, 10000, ..., 100000000
    .register()?;
```

### Custom Buckets

```rust
// API response times (50ms to 5s)
let histogram = HistogramBuilder::new("api_duration", "API duration")
    .buckets(vec![0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0])
    .register()?;

// File sizes (1KB to 1GB)
let histogram = HistogramBuilder::new("file_size", "File size")
    .buckets(vec![
        1_000.0,
        10_000.0,
        100_000.0,
        1_000_000.0,
        10_000_000.0,
        100_000_000.0,
        1_000_000_000.0,
    ])
    .register()?;
```

### Bucket Guidelines

- **Too few buckets** - Loss of precision
- **Too many buckets** - Increased memory/storage
- **Sweet spot** - 10-20 buckets covering your expected range

---

## Metrics Endpoint

The `/metrics` endpoint exposes metrics in Prometheus format.

### Adding the Endpoint

```rust
use armature_core::*;
use armature_metrics::*;

let mut router = Router::new();

// Method 1: Using helper function
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/metrics".to_string(),
    handler: create_metrics_handler(),
    constraints: None,
});

// Method 2: Using handler directly
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/metrics".to_string(),
    handler: Arc::new(|req| {
        Box::pin(async move {
            metrics_handler(req).await
        })
    }),
    constraints: None,
});
```

### Prometheus Configuration

Add to your `prometheus.yml`:

```yaml
scrape_configs:
  - job_name: 'armature-app'
    static_configs:
      - targets: ['localhost:3000']
    metrics_path: '/metrics'
    scrape_interval: 15s
```

---

## Best Practices

### 1. Choose the Right Metric Type

- **Counter** - Things that only increase (requests, errors, sales)
- **Gauge** - Things that go up and down (temperature, active connections, queue size)
- **Histogram** - Distributions (latency, size, duration)

### 2. Use Meaningful Names

```rust
// ✅ Good - descriptive
let counter = register_counter("http_requests_total", "Total HTTP requests")?;
let gauge = register_gauge("active_database_connections", "Active DB connections")?;
let histogram = register_histogram("api_request_duration_seconds", "API duration")?;

// ❌ Bad - vague
let counter = register_counter("counter1", "Counter")?;
let gauge = register_gauge("active", "Active")?;
```

### 3. Follow Naming Conventions

- Use **snake_case** for metric names
- Include **units** in the name (`_seconds`, `_bytes`, `_total`)
- Be **descriptive** but concise

```rust
// Good examples
"http_requests_total"
"database_query_duration_seconds"
"response_size_bytes"
"active_connections"
"errors_total"
```

### 4. Manage Cardinality

Avoid creating millions of unique metric combinations:

```rust
// ✅ Good - bounded cardinality
let metric = CounterVecBuilder::new("requests", "Requests")
    .labels(&["method", "status_class"])  // ~20 combinations
    .register()?;

// ❌ Bad - unbounded cardinality
let metric = CounterVecBuilder::new("requests", "Requests")
    .labels(&["user_id", "session_id", "timestamp"])  // Millions!
    .register()?;
```

### 5. Use Appropriate Buckets

```rust
// ✅ Good - covers expected range
let histogram = HistogramBuilder::new("api_latency", "API latency")
    .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0])  // 10ms to 5s
    .register()?;

// ❌ Bad - too narrow
let histogram = HistogramBuilder::new("api_latency", "API latency")
    .buckets(vec![0.1, 0.2, 0.3])  // Only 100-300ms
    .register()?;
```

---

## Summary

**Key Points:**

1. **Three metric types** - Counter, Gauge, Histogram
2. **Auto HTTP metrics** - Use `RequestMetricsMiddleware`
3. **Custom business metrics** - Track domain-specific events
4. **Labels for dimensions** - But manage cardinality
5. **`/metrics` endpoint** - Standard Prometheus format
6. **Choose appropriate buckets** - Match your use case

**Quick Reference:**

```rust
// Counter
let counter = register_counter("name", "help")?;
counter.inc();

// Gauge
let gauge = register_gauge("name", "help")?;
gauge.set(42.0);

// Histogram
let histogram = register_histogram("name", "help")?;
histogram.observe(0.5);

// With labels
let metric = CounterVecBuilder::new("name", "help")
    .labels(&["label1", "label2"])
    .register()?;

metric.with_label_values(&["value1", "value2"]).inc();

// Metrics endpoint
router.add_route(Route {
    handler: create_metrics_handler(),
    ..route
});

// Request metrics middleware
let app = Application::new()
    .middleware(Arc::new(RequestMetricsMiddleware::new()))
    .build();
```

**Resources:**
- [Prometheus Documentation](https://prometheus.io/docs/)
- [Metric Types](https://prometheus.io/docs/concepts/metric_types/)
- [Best Practices](https://prometheus.io/docs/practices/naming/)

