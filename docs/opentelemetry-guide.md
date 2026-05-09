# OpenTelemetry Integration Guide

Comprehensive observability for Armature applications with distributed tracing, metrics, and logging.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [Tracing](#tracing)
- [Metrics](#metrics)
- [Exporters](#exporters)
- [Middleware](#middleware)
- [Best Practices](#best-practices)
- [Examples](#examples)

## Overview

The `armature-opentelemetry` module provides comprehensive observability for your Armature applications using the OpenTelemetry standard. It automatically instruments HTTP requests, collects metrics, and enables distributed tracing across your services.

### Why OpenTelemetry?

- **Vendor-neutral**: Works with Jaeger, Zipkin, Prometheus, Grafana, and more
- **Industry standard**: CNCF graduated project with wide adoption
- **Comprehensive**: Traces, metrics, and logs in one framework
- **Distributed tracing**: Follow requests across microservices
- **Performance insights**: Identify bottlenecks and optimize

## Features

✅ **Automatic HTTP Instrumentation**
- Traces every HTTP request automatically
- Captures method, path, status, duration
- Propagates trace context across services

✅ **Distributed Tracing**
- W3C Trace Context propagation
- Parent-child span relationships
- Service mesh compatible

✅ **Metrics Collection**
- Request counts
- Request durations (histograms)
- Active requests (gauges)
- Custom business metrics

✅ **Multiple Exporters**
- OTLP (OpenTelemetry Protocol)
- Jaeger
- Zipkin
- Prometheus

✅ **Flexible Configuration**
- Environment-based config
- Code-based builder pattern
- Sampling strategies
- Resource attributes

## Quick Start

### Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["opentelemetry"] }

# Choose exporters
armature-opentelemetry = { version = "0.1", features = ["otlp", "prometheus"] }
```

### Basic Setup

```rust
use armature_framework::prelude::*;
use armature_opentelemetry::*;

#[module()]
#[derive(Default)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize telemetry
    let telemetry = TelemetryBuilder::new("my-service")
        .with_version("1.0.0")
        .with_environment("production")
        .with_otlp_endpoint("http://localhost:4317")
        .with_tracing()
        .with_metrics()
        .build()
        .await?;

    // Create application
    let app = Application::create::<AppModule>().await;

    // Run server
    app.listen(3000).await?;

    // Shutdown gracefully
    telemetry.shutdown().await?;
    Ok(())
}
```

### Running with Docker

Start Jaeger all-in-one (includes OTLP collector):

```bash
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4317:4317 \
  -p 4318:4318 \
  jaegertracing/all-in-one:latest
```

View traces at: http://localhost:16686

## Configuration

### Builder Pattern

```rust
let telemetry = TelemetryBuilder::new("my-service")
    // Service info
    .with_version("1.0.0")
    .with_namespace("production")
    .with_environment("us-west-2")

    // Enable features
    .with_tracing()
    .with_metrics()

    // Exporter config
    .with_otlp_endpoint("http://collector:4317")

    // Sampling
    .with_sampling_ratio(0.1)  // Sample 10% of traces

    // Custom attributes
    .with_attribute("team", "platform")
    .with_attribute("cluster", "k8s-prod")

    .build()
    .await?;
```

### Configuration Struct

```rust
use armature_opentelemetry::*;

let config = TelemetryConfig {
    service_name: "my-service".to_string(),
    service_version: Some("1.0.0".to_string()),
    environment: Some("production".to_string()),
    enable_tracing: true,
    enable_metrics: true,
    tracing: TracingConfig {
        exporter: TracingExporter::Otlp,
        otlp_endpoint: Some("http://localhost:4317".to_string()),
        sampling_ratio: 1.0,
        max_attributes_per_span: 128,
        max_events_per_span: 128,
    },
    metrics: MetricsConfig {
        exporter: MetricsExporter::Otlp,
        otlp_endpoint: Some("http://localhost:4317".to_string()),
        collection_interval_secs: 60,
    },
    resource_attributes: vec![
        ("team".to_string(), "platform".to_string()),
    ],
};

let telemetry = TelemetryBuilder::new("my-service")
    .with_config(config)
    .build()
    .await?;
```

### Environment Variables

```bash
export OTEL_SERVICE_NAME="my-service"
export OTEL_SERVICE_VERSION="1.0.0"
export OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4317"
export OTEL_TRACES_SAMPLER="parentbased_traceidratio"
export OTEL_TRACES_SAMPLER_ARG="0.1"
```

## Tracing

### Automatic Tracing

HTTP requests are automatically traced when using the middleware:

```rust
let app = Application::new(container, router)
    .with_middleware(telemetry.middleware());
```

Each request creates a span with:
- HTTP method
- Request path
- Status code
- Duration
- Request/response headers
- User agent

### Manual Spans

Create custom spans for specific operations:

```rust
use armature_opentelemetry::*;

async fn process_order(order_id: u64) -> Result<(), Error> {
    // Create a span
    let span = trace_span!("process_order",
        "order.id" => order_id.to_string(),
        "order.priority" => "high"
    );

    // Do work...

    // Add events
    span_event!("order_validated");
    span_event!("payment_processed", "amount" => "99.99");

    Ok(())
}
```

### Span Macros

```rust
// Create a simple span
let span = trace_span!("my_operation");

// Create with attributes
let span = trace_span!("database_query",
    "db.system" => "postgresql",
    "db.statement" => "SELECT * FROM users"
);

// Add attribute to current span
span_attribute!("user.id", user_id.to_string());

// Record event
span_event!("cache_miss");

// Record event with attributes
span_event!("item_processed",
    "item.id" => item_id.to_string(),
    "item.status" => "complete"
);
```

### Distributed Tracing

Trace context is automatically propagated via HTTP headers:

```rust
use armature_opentelemetry::HeaderInjector;

async fn call_downstream_service() {
    let client = reqwest::Client::new();
    let mut headers = HashMap::new();

    // Inject trace context into headers
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &opentelemetry::Context::current(),
            &mut HeaderInjector(&mut headers)
        );
    });

    // Make request with propagated context
    client.get("http://downstream/api")
        .headers(/* convert headers */)
        .send()
        .await?;
}
```

## Metrics

### HTTP Metrics

Automatically collected when using the middleware:

- `http.server.request.count` - Total requests (counter)
- `http.server.request.duration` - Request duration in seconds (histogram)
- `http.server.active_requests` - Currently active requests (gauge)

All metrics include labels:
- `http.method` - Request method
- `http.route` - Request path
- `http.status_code` - Response status

### Custom Metrics

```rust
use armature_opentelemetry::*;
use opentelemetry::metrics::*;

// Get meter
let meter = get_meter("my-service");

// Counter
let orders_counter = meter
    .u64_counter("orders.total")
    .with_description("Total orders processed")
    .build();

orders_counter.add(1, &[
    KeyValue::new("order.type", "premium"),
    KeyValue::new("order.status", "completed"),
]);

// Gauge
let queue_size = meter
    .i64_up_down_counter("queue.size")
    .with_description("Current queue size")
    .build();

queue_size.add(10, &[]);
queue_size.add(-2, &[]);  // Decrement

// Histogram
let processing_time = meter
    .f64_histogram("processing.duration")
    .with_description("Processing duration in seconds")
    .with_unit("s")
    .build();

processing_time.record(0.523, &[
    KeyValue::new("operation", "image_resize"),
]);
```

### Business Metrics

```rust
#[derive(Clone)]
struct OrderService {
    meter: opentelemetry::metrics::Meter,
    orders_total: Counter<u64>,
    revenue_total: Histogram<f64>,
}

impl OrderService {
    fn new() -> Self {
        let meter = get_meter("order-service");

        let orders_total = meter
            .u64_counter("orders.total")
            .build();

        let revenue_total = meter
            .f64_histogram("revenue.total")
            .with_unit("USD")
            .build();

        Self { meter, orders_total, revenue_total }
    }

    async fn create_order(&self, amount: f64) -> Result<(), Error> {
        // Process order...

        // Record metrics
        self.orders_total.add(1, &[
            KeyValue::new("region", "us-west"),
        ]);

        self.revenue_total.record(amount, &[
            KeyValue::new("currency", "USD"),
        ]);

        Ok(())
    }
}
```

## Exporters

### OTLP (Recommended)

OpenTelemetry Protocol - works with multiple backends:

```rust
let telemetry = TelemetryBuilder::new("my-service")
    .with_otlp_endpoint("http://collector:4317")
    .with_tracing()
    .with_metrics()
    .build()
    .await?;
```

Backends that support OTLP:
- **Jaeger** (v1.35+)
- **Grafana Tempo**
- **Grafana Cloud**
- **Honeycomb**
- **New Relic**
- **Datadog**
- **AWS X-Ray**

### Jaeger

Direct export to Jaeger:

```toml
[dependencies]
armature-opentelemetry = { version = "0.1", features = ["jaeger"] }
```

```rust
let config = TelemetryConfig {
    tracing: TracingConfig {
        exporter: TracingExporter::Jaeger,
        jaeger_endpoint: Some("localhost:6831".to_string()),
        ..Default::default()
    },
    ..TelemetryConfig::new("my-service")
};
```

### Zipkin

Export to Zipkin:

```toml
[dependencies]
armature-opentelemetry = { version = "0.1", features = ["zipkin"] }
```

```rust
let config = TelemetryConfig {
    tracing: TracingConfig {
        exporter: TracingExporter::Zipkin,
        zipkin_endpoint: Some("http://localhost:9411/api/v2/spans".to_string()),
        ..Default::default()
    },
    ..TelemetryConfig::new("my-service")
};
```

### Prometheus

Expose metrics for Prometheus scraping:

```toml
[dependencies]
armature-opentelemetry = { version = "0.1", features = ["prometheus"] }
```

```rust
let telemetry = TelemetryBuilder::new("my-service")
    .with_metrics()
    .build()
    .await?;

// Add metrics endpoint
#[get("/metrics")]
async fn metrics() -> Result<String, Error> {
    // Prometheus exporter provides the metrics
    Ok("metrics data".to_string())
}
```

## Middleware

### Automatic Instrumentation

The telemetry middleware automatically:

1. **Creates spans** for each request
2. **Extracts trace context** from incoming headers
3. **Injects trace context** for distributed tracing
4. **Records metrics** (count, duration, active requests)
5. **Captures errors** and sets span status

### Usage

```rust
let app = Application::new(container, router)
    .with_middleware(telemetry.middleware());
```

### Multiple Middleware

Combine with other middleware:

```rust
let app = Application::new(container, router)
    .with_middleware(CorsMiddleware::permissive())
    .with_middleware(telemetry.middleware())  // Should be early in chain
    .with_middleware(AuthMiddleware::new());
```

## Best Practices

### 1. Use Semantic Conventions

Follow OpenTelemetry semantic conventions for attribute names:

```rust
// ✅ Good - semantic convention
span_attribute!("http.method", "GET");
span_attribute!("db.system", "postgresql");
span_attribute!("messaging.system", "rabbitmq");

// ❌ Bad - custom names
span_attribute!("method", "GET");
span_attribute!("database", "postgres");
```

See: https://opentelemetry.io/docs/specs/semconv/

### 2. Sample Appropriately

Don't trace everything in production:

```rust
// Development - trace everything
.with_sampling_ratio(1.0)

// Staging - trace 50%
.with_sampling_ratio(0.5)

// Production - trace 10%
.with_sampling_ratio(0.1)

// High-traffic production - trace 1%
.with_sampling_ratio(0.01)
```

### 3. Add Context

Include relevant business context:

```rust
span_attribute!("user.id", user_id.to_string());
span_attribute!("tenant.id", tenant_id.to_string());
span_attribute!("order.id", order_id.to_string());
span_attribute!("feature.flag", "new_checkout");
```

### 4. Record Important Events

```rust
span_event!("cache_hit");
span_event!("retry_attempt", "attempt" => "2");
span_event!("threshold_exceeded", "value" => "1000");
```

### 5. Graceful Shutdown

Always shutdown telemetry to flush data:

```rust
tokio::select! {
    _ = app.listen(3000) => {},
    _ = tokio::signal::ctrl_c() => {
        println!("Shutting down...");
        telemetry.shutdown().await?;
    }
}
```

### 6. Resource Attributes

Add deployment metadata:

```rust
let telemetry = TelemetryBuilder::new("my-service")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_environment("production")
    .with_namespace("payments")
    .with_attribute("k8s.pod.name", std::env::var("POD_NAME")?)
    .with_attribute("k8s.node.name", std::env::var("NODE_NAME")?)
    .with_attribute("region", "us-west-2")
    .build()
    .await?;
```

## Examples

### Complete Application

```rust
use armature_framework::prelude::*;
use armature_framework::armature_opentelemetry::*;

#[derive(Clone)]
#[injectable]
struct UserService;

impl UserService {
    async fn create_user(&self, name: String) -> Result<u64, Error> {
        span_attribute!("user.name", name);
        span_event!("user_created");
        Ok(42)
    }
}

#[controller("/api/users")]
struct UserController {
    user_service: UserService,
}

impl UserController {
    #[post("/")]
    async fn create(
        &self,
        #[Body] body: Json<serde_json::Value>,
    ) -> Result<Json<serde_json::Value>, Error> {
        let name = body.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::BadRequest("name required".to_string()))?;

        let id = self.user_service.create_user(name.to_string()).await?;

        Ok(Json(serde_json::json!({ "id": id })))
    }
}

#[module]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = TelemetryBuilder::new("user-service")
        .with_version("1.0.0")
        .with_environment("production")
        .with_otlp_endpoint("http://localhost:4317")
        .with_tracing()
        .with_metrics()
        .build()
        .await?;

    let app = Application::create::<AppModule>().await;

    tokio::select! {
        _ = app.listen(3000) => {},
        _ = tokio::signal::ctrl_c() => {
            telemetry.shutdown().await?;
        }
    }

    Ok(())
}
```

### Kubernetes Deployment

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: otel-config
data:
  OTEL_EXPORTER_OTLP_ENDPOINT: "http://otel-collector:4317"
  OTEL_SERVICE_NAME: "user-service"
  OTEL_SERVICE_VERSION: "1.0.0"
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: user-service
spec:
  template:
    spec:
      containers:
      - name: user-service
        image: user-service:1.0.0
        envFrom:
        - configMapRef:
            name: otel-config
        env:
        - name: POD_NAME
          valueFrom:
            fieldRef:
              fieldPath: metadata.name
        - name: NODE_NAME
          valueFrom:
            fieldRef:
              fieldPath: spec.nodeName
```

## Summary

**Key Features:**
- ✅ Automatic HTTP tracing with middleware
- ✅ Built-in metrics collection
- ✅ Multiple exporter support (OTLP, Jaeger, Zipkin, Prometheus)
- ✅ Distributed tracing with context propagation
- ✅ Flexible configuration and builder API
- ✅ Production-ready with sampling strategies

**When to Use:**
- Microservices architectures
- Production debugging
- Performance monitoring
- Distributed tracing
- SLA/SLO tracking

**Next Steps:**
1. Start with OTLP + Jaeger for development
2. Add sampling for production (10% or less)
3. Include business metrics
4. Set up alerting based on metrics
5. Create dashboards for visualization

Happy observability! 🔭📊

