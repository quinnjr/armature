# Error Correlation Guide

Comprehensive error correlation and distributed tracing for tracking errors across services and request chains.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Correlation Context](#correlation-context)
- [ID Generation Strategies](#id-generation-strategies)
- [Correlated Errors](#correlated-errors)
- [Error Registry](#error-registry)
- [Correlation Middleware](#correlation-middleware)
- [Distributed Tracing](#distributed-tracing)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

Error correlation is essential for debugging distributed systems. When an error occurs, you need to:

1. **Identify the request** - Which user/request triggered the error?
2. **Trace the flow** - What services were involved?
3. **Find related errors** - Are there other errors from the same flow?
4. **Understand causation** - What caused this error?

Armature's error correlation module provides all these capabilities with support for industry-standard tracing formats.

## Features

- ✅ Correlation ID generation and propagation
- ✅ Request ID tracking
- ✅ Trace/Span ID support (W3C Trace Context & Zipkin B3)
- ✅ Error chain tracking (parent-child relationships)
- ✅ Causation chain for root cause analysis
- ✅ Context propagation across services
- ✅ Multiple ID generation strategies
- ✅ Error registry for aggregation
- ✅ Middleware for automatic correlation
- ✅ OpenTelemetry-compatible format

## Quick Start

### Basic Usage

```rust
use armature_framework::prelude::*;
use armature_core::error_correlation::*;

// Create correlation context
let ctx = CorrelationContext::new()
    .with_service("my-api")
    .with_user_id("user-123");

// Create a correlated error
let error = CorrelatedError::new("Database connection failed")
    .with_context(ctx)
    .with_code("DB_CONN_ERR")
    .with_status(503)
    .caused_by("Connection timeout after 30s");

println!("{}", error.to_json());
```

### With Middleware

```rust
use armature_framework::prelude::*;
use armature_core::error_correlation::*;

let config = CorrelationConfig::new()
    .service("order-service")
    .version("1.2.0")
    .strategy(IdGenerationStrategy::UuidV7)
    .generate_traces(true);

let app = Application::new()
    .use_middleware(CorrelationMiddleware::new(config));
```

## Correlation Context

The `CorrelationContext` carries correlation information through your application.

### Creating Context

```rust
use armature_core::error_correlation::*;

// New context with generated IDs
let ctx = CorrelationContext::new();

// With specific configuration
let ctx = CorrelationContext::with_strategy(IdGenerationStrategy::UuidV7)
    .with_service("auth-service")
    .with_service_version("2.1.0")
    .with_user_id("user-123")
    .with_tenant("tenant-abc")
    .with_session("session-xyz")
    .with_baggage("region", "us-east-1");
```

### Extracting from Request

```rust
use armature_core::error_correlation::*;

async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    // Extract correlation context from headers
    let ctx = CorrelationContext::from_request(&req);

    // Or use the extension trait
    let ctx = req.correlation_context();

    // Access individual IDs
    let correlation_id = req.correlation_id();
    let request_id = req.request_id();
    let trace_id = req.trace_id();

    Ok(HttpResponse::ok())
}
```

### Creating Child Contexts

For downstream service calls, create child contexts:

```rust
use armature_core::error_correlation::*;

let parent_ctx = CorrelationContext::new()
    .with_service("api-gateway")
    .trace_id("abc123def456");

// Child context for downstream call
let child_ctx = parent_ctx.child();

// Child inherits:
// - Same correlation_id
// - Same trace_id
// - Same user_id, tenant_id, session_id
// - New request_id and span_id
// - parent_span_id set to parent's span_id
// - causation_id set to parent's request_id
```

### Propagating Context

```rust
use armature_core::error_correlation::*;

// Inject into outgoing request
let mut outgoing_req = HttpRequest::new("POST".to_string(), "/api/users".to_string());
ctx.inject_into_request(&mut outgoing_req);

// Inject into response
let mut response = HttpResponse::ok();
ctx.inject_into_response(&mut response);
```

## ID Generation Strategies

Choose the right ID generation strategy for your needs:

| Strategy | Format | Properties | Use Case |
|----------|--------|------------|----------|
| `UuidV4` | `550e8400-e29b-41d4-a716-446655440000` | Random, universal | Default, general purpose |
| `UuidV7` | `018f2c58-...` | Time-ordered, sortable | When chronological order matters |
| `Snowflake` | `0190a2b3c4d5e6f7` | 16 hex chars, time + machine + seq | High-volume distributed systems |
| `Ulid` | `01ARZ3NDEKTSV4RRFFQ69G5FAV` | 26 chars, lexicographically sortable | Database-friendly, sortable |
| `Short` | `aB3xY9kL` | 8 chars, alphanumeric | Human-readable, logs |

### Example

```rust
use armature_core::error_correlation::*;

// Generate IDs with different strategies
let uuid_v4 = IdGenerationStrategy::UuidV4.generate();
let uuid_v7 = IdGenerationStrategy::UuidV7.generate();
let snowflake = IdGenerationStrategy::Snowflake.generate();
let ulid = IdGenerationStrategy::Ulid.generate();
let short = IdGenerationStrategy::Short.generate();

println!("UUID v4:    {}", uuid_v4);    // 550e8400-e29b-41d4-a716-446655440000
println!("UUID v7:    {}", uuid_v7);    // 018f2c58-6ec9-7d7c-9d8e-000000000001
println!("Snowflake:  {}", snowflake);  // 0190a2b3c4d5e6f7
println!("ULID:       {}", ulid);       // 01ARZ3NDEKTSV4RRFFQ69G5FAV
println!("Short:      {}", short);      // aB3xY9kL
```

## Correlated Errors

`CorrelatedError` captures full context about an error occurrence.

### Creating Correlated Errors

```rust
use armature_core::error_correlation::*;

let ctx = CorrelationContext::new();

let error = CorrelatedError::new("Payment processing failed")
    .with_context(ctx)
    .with_code("PAY_001")
    .with_type("PAYMENT_ERROR")
    .with_status(502)
    .with_source_service("payment-gateway")
    .with_source_location("src/payment.rs:142")
    .caused_by("Stripe API timeout")
    .caused_by("Network congestion")
    .related_to("err-abc123")
    .with_metadata("amount", 99.99)
    .with_metadata("currency", "USD")
    .retryable(5000, 3);  // Retry after 5s, max 3 attempts
```

### From Framework Errors

```rust
use armature_core::{Error, error_correlation::*};

async fn handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    let ctx = req.correlation_context();

    // Process request...
    let result = process_order(&req).await;

    match result {
        Ok(order) => Ok(HttpResponse::json(&order)),
        Err(err) => {
            // Convert to correlated error
            let correlated = err.correlate(ctx.clone());
            // Or from request directly
            let correlated = err.correlate_with_request(&req);

            // Log and return
            log_error(&correlated);
            Err(err)
        }
    }
}
```

### Retry Information

For recoverable errors, include retry guidance:

```rust
use armature_core::error_correlation::*;

let error = CorrelatedError::new("Rate limit exceeded")
    .with_status(429)
    .with_retry_info(RetryInfo {
        retryable: true,
        retry_delay_ms: Some(60_000),  // Wait 1 minute
        max_retries: Some(5),
        current_attempt: 1,
    });

// Shorthand
let error = CorrelatedError::new("Service temporarily unavailable")
    .retryable(5000, 3);  // 5s delay, 3 max retries
```

## Error Registry

The `ErrorRegistry` stores and indexes correlated errors for querying.

### Setup

```rust
use armature_core::error_correlation::*;
use std::sync::Arc;

// Create registry with max 10,000 errors
let registry = Arc::new(ErrorRegistry::new(10_000));

// Or use default (10,000)
let registry = Arc::new(ErrorRegistry::default());
```

### Registering Errors

```rust
use armature_core::error_correlation::*;

async fn handle_error(registry: &ErrorRegistry, error: CorrelatedError) {
    registry.register(error.clone()).await;

    // Later, retrieve by ID
    if let Some(err) = registry.get(&error.error_id).await {
        println!("Found error: {}", err.message);
    }
}
```

### Querying Errors

```rust
use armature_core::error_correlation::*;

async fn analyze_errors(registry: &ErrorRegistry, correlation_id: &str) {
    // Get all errors in a correlation chain
    let errors = registry.get_by_correlation(correlation_id).await;
    println!("Found {} related errors", errors.len());

    // Get all errors in a trace
    let trace_errors = registry.get_by_trace("trace-123").await;

    // Build causation tree
    if let Some(tree) = registry.build_causation_tree("error-id").await {
        print_error_tree(&tree, 0);
    }
}

fn print_error_tree(tree: &ErrorTree, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{}{}: {}", indent, tree.error.error_id, tree.error.message);
    for child in &tree.children {
        print_error_tree(child, depth + 1);
    }
}
```

### With Middleware

```rust
use armature_core::error_correlation::*;
use std::sync::Arc;

let registry = Arc::new(ErrorRegistry::new(10_000));

let middleware = CorrelationMiddleware::new(CorrelationConfig::default())
    .with_registry(registry.clone());

// Errors are automatically registered
```

## Correlation Middleware

The `CorrelationMiddleware` automatically handles correlation for all requests.

### Configuration

```rust
use armature_core::error_correlation::*;

let config = CorrelationConfig::new()
    // Set service identity
    .service("order-service")
    .version("2.0.0")

    // ID generation strategy
    .strategy(IdGenerationStrategy::UuidV7)

    // Enable trace ID generation
    .generate_traces(true)

    // Propagate correlation headers in response
    .propagate_response(true);

let middleware = CorrelationMiddleware::new(config);
```

### Headers Handled

The middleware automatically handles these headers:

| Header | Direction | Description |
|--------|-----------|-------------|
| `X-Correlation-ID` | In/Out | Groups related requests |
| `X-Request-ID` | In/Out | Unique per request |
| `traceparent` | In/Out | W3C Trace Context |
| `tracestate` | In/Out | W3C Trace State |
| `X-B3-TraceId` | In/Out | Zipkin B3 format |
| `X-B3-SpanId` | In/Out | Zipkin B3 format |
| `X-B3-ParentSpanId` | In/Out | Zipkin B3 format |
| `X-B3-Sampled` | In/Out | Zipkin sampling |
| `X-Causation-ID` | In/Out | What caused this request |
| `X-Session-ID` | In/Out | Session tracking |

## Distributed Tracing

### W3C Trace Context

Armature supports the W3C Trace Context standard:

```rust
use armature_core::error_correlation::*;

let ctx = CorrelationContext::new()
    .trace_id("4bf92f3577b34da6a3ce929d0e0e4736")
    .span_id("00f067aa0ba902b7");

// Generate traceparent header
let traceparent = ctx.to_traceparent();
// "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
```

### Zipkin B3 Compatibility

B3 headers are automatically supported:

```rust
// Incoming request with B3 headers
let mut req = HttpRequest::new("GET".to_string(), "/api".to_string());
req.headers.insert("X-B3-TraceId".to_string(), "abc123".to_string());
req.headers.insert("X-B3-SpanId".to_string(), "def456".to_string());

let ctx = CorrelationContext::from_request(&req);
// trace_id and parent_span_id are extracted from B3 headers
```

### Integration with Tracing Systems

```rust
use armature_core::error_correlation::*;

async fn call_downstream_service(ctx: &CorrelationContext) {
    let child_ctx = ctx.child();

    let mut req = HttpRequest::new("POST".to_string(), "/api/users".to_string());
    child_ctx.inject_into_request(&mut req);

    // Request now has all tracing headers for:
    // - OpenTelemetry (traceparent)
    // - Zipkin (X-B3-*)
    // - Custom correlation (X-Correlation-ID, X-Request-ID)
}
```

## Best Practices

### 1. Always Use Correlation Middleware

```rust
// ✅ Good: Automatic correlation for all requests
let app = Application::new()
    .use_middleware(CorrelationMiddleware::default_config());
```

### 2. Propagate Context to Downstream Services

```rust
// ✅ Good: Create child context for downstream calls
async fn call_service(ctx: &CorrelationContext) {
    let child_ctx = ctx.child();
    let mut req = HttpRequest::new("GET".to_string(), "/api".to_string());
    child_ctx.inject_into_request(&mut req);
    // Make call...
}

// ❌ Bad: Creating new context loses correlation
async fn call_service_bad() {
    let ctx = CorrelationContext::new();  // Lost parent correlation!
    // ...
}
```

### 3. Include Causation Chains

```rust
// ✅ Good: Track what caused the error
let error = CorrelatedError::new("Order failed")
    .caused_by("Payment declined")
    .caused_by("Insufficient funds");
```

### 4. Add Meaningful Metadata

```rust
// ✅ Good: Include relevant context
let error = CorrelatedError::new("Validation failed")
    .with_metadata("field", "email")
    .with_metadata("value", "invalid-email")
    .with_metadata("rule", "RFC 5322");

// ❌ Bad: No context
let error = CorrelatedError::new("Validation failed");
```

### 5. Use Appropriate ID Strategy

```rust
// For high-volume systems needing time ordering
let config = CorrelationConfig::new()
    .strategy(IdGenerationStrategy::UuidV7);

// For human-readable logs
let config = CorrelationConfig::new()
    .strategy(IdGenerationStrategy::Short);
```

### 6. Register Errors for Analysis

```rust
use std::sync::Arc;

let registry = Arc::new(ErrorRegistry::new(10_000));

// Attach to middleware for automatic registration
let middleware = CorrelationMiddleware::new(CorrelationConfig::default())
    .with_registry(registry.clone());

// Query later for debugging
let errors = registry.get_by_correlation("corr-123").await;
```

## API Reference

### Types

| Type | Description |
|------|-------------|
| `CorrelationContext` | Carries correlation information |
| `CorrelatedError` | Error with full correlation info |
| `ErrorRegistry` | In-memory error storage |
| `ErrorTree` | Tree structure for causation |
| `RetryInfo` | Retry guidance for errors |
| `CorrelationConfig` | Middleware configuration |
| `CorrelationMiddleware` | Automatic correlation handling |
| `IdGenerationStrategy` | ID generation algorithms |

### Extension Traits

| Trait | For | Methods |
|-------|-----|---------|
| `CorrelatedRequest` | `HttpRequest` | `correlation_context()`, `correlation_id()`, `request_id()`, `trace_id()`, `span_id()` |
| `CorrelatedErrorExt` | `Error` | `correlate()`, `correlate_with_request()` |

### Headers Module

```rust
use armature_core::error_correlation::headers;

// Standard headers
headers::CORRELATION_ID    // "X-Correlation-ID"
headers::REQUEST_ID        // "X-Request-ID"
headers::TRACE_PARENT      // "traceparent"
headers::TRACE_STATE       // "tracestate"
headers::B3_TRACE_ID       // "X-B3-TraceId"
headers::B3_SPAN_ID        // "X-B3-SpanId"
headers::B3_PARENT_SPAN_ID // "X-B3-ParentSpanId"
headers::B3_SAMPLED        // "X-B3-Sampled"
headers::CAUSATION_ID      // "X-Causation-ID"
headers::SESSION_ID        // "X-Session-ID"
```

## Summary

**Key Points:**

1. **Correlation Context** - Use `CorrelationContext` to track requests across services
2. **Correlated Errors** - Wrap errors with `CorrelatedError` for full context
3. **Middleware** - Use `CorrelationMiddleware` for automatic handling
4. **Child Contexts** - Create child contexts for downstream calls
5. **Error Registry** - Store and query errors for debugging
6. **Tracing Support** - Compatible with OpenTelemetry and Zipkin

**Quick Reference:**

```rust
// Create context
let ctx = CorrelationContext::new().with_service("my-service");

// Create child for downstream
let child = ctx.child();

// Create correlated error
let error = CorrelatedError::new("Failed")
    .with_context(ctx)
    .caused_by("Root cause");

// Use middleware
let app = Application::new()
    .use_middleware(CorrelationMiddleware::default_config());
```


