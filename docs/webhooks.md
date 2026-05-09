# Webhooks Module

Webhook orchestration for sending and receiving HTTP callbacks in the Armature framework.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Sending Webhooks](#sending-webhooks)
- [Receiving Webhooks](#receiving-webhooks)
- [Endpoint Registry](#endpoint-registry)
- [Signature Security](#signature-security)
- [Retry Policies](#retry-policies)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The `armature-webhooks` module provides comprehensive webhook support including:

- **Outgoing Webhooks**: Send HTTP callbacks to registered endpoints
- **Incoming Webhooks**: Receive and verify webhooks from external services
- **Signature Verification**: HMAC-SHA256 signing and verification
- **Automatic Retries**: Configurable retry policies with exponential backoff
- **Event System**: Subscribe endpoints to specific event types

## Features

- ✅ Send webhooks with automatic HMAC-SHA256 signatures
- ✅ Receive and verify incoming webhooks
- ✅ Event-based endpoint subscriptions with wildcards
- ✅ Configurable retry policies (fixed, exponential backoff)
- ✅ Delivery tracking and status monitoring
- ✅ Endpoint registry with failure tracking
- ✅ Timestamp validation to prevent replay attacks
- ✅ Custom headers per endpoint

## Installation

Add the webhooks feature to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["webhooks"] }
```

Or use the crate directly:

```toml
[dependencies]
armature-webhooks = "0.1"
```

## Sending Webhooks

### Basic Usage

```rust
use armature_webhooks::{WebhookClient, WebhookConfig, WebhookPayload};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = WebhookClient::new(WebhookConfig::default());

    let payload = WebhookPayload::new("user.created")
        .with_data(serde_json::json!({
            "user_id": "usr_123",
            "email": "user@example.com"
        }));

    // Send to a specific URL
    let delivery = client.send("https://example.com/webhook", payload).await?;

    println!("Delivery status: {:?}", delivery.status);
    Ok(())
}
```

### With Signing Secret

```rust
let delivery = client
    .send_with_secret(
        "https://example.com/webhook",
        payload,
        Some("your-signing-secret"),
    )
    .await?;
```

### Dispatching to Multiple Endpoints

```rust
use std::sync::Arc;

let registry = Arc::new(WebhookRegistry::new());

// Register endpoints
registry.register(
    WebhookEndpoint::builder("https://api1.example.com/webhook")
        .events(vec!["user.*"])
        .build()
);

registry.register(
    WebhookEndpoint::builder("https://api2.example.com/webhook")
        .events(vec!["user.created", "order.*"])
        .build()
);

let client = WebhookClient::with_registry(WebhookConfig::default(), registry);

// Dispatch to all subscribed endpoints
let payload = WebhookPayload::new("user.created")
    .with_data(serde_json::json!({"user_id": "123"}));

let deliveries = client.dispatch(payload).await?;
println!("Sent to {} endpoints", deliveries.len());
```

## Receiving Webhooks

### Verify and Parse

```rust
use armature_webhooks::WebhookReceiver;

let receiver = WebhookReceiver::new("your-signing-secret");

// In your HTTP handler
fn handle_webhook(body: &[u8], signature_header: &str) -> Result<(), WebhookError> {
    // Verify and parse in one step
    let payload = receiver.receive(body, signature_header)?;

    println!("Received event: {}", payload.event);
    println!("Data: {:?}", payload.data);

    Ok(())
}
```

### Verify from HTTP Headers

```rust
use std::collections::HashMap;

let mut headers = HashMap::new();
headers.insert("X-Webhook-Signature".to_string(), signature.to_string());

let is_valid = receiver.verify_from_headers(&body, &headers)?;
```

### Event Handlers

```rust
let handler = receiver.handler("user.*", |payload| {
    println!("Received user event: {}", payload.event);
    // Process the webhook
    Ok(())
});

// Handle incoming request
let handled = handler.handle(&body, &signature)?;
```

## Endpoint Registry

### Creating and Managing Endpoints

```rust
use armature_webhooks::{WebhookRegistry, WebhookEndpoint};

let registry = WebhookRegistry::new();

// Create an endpoint with builder
let endpoint = WebhookEndpoint::builder("https://api.example.com/webhooks")
    .events(vec!["user.created", "user.updated", "order.*"])
    .description("Main API webhook")
    .header("X-Custom-Header", "custom-value")
    .build();

let endpoint_id = registry.register(endpoint);

// Query endpoints
let user_endpoints = registry.get_endpoints_for_event("user.created");
let all_endpoints = registry.get_all();

// Manage endpoints
registry.enable(&endpoint_id)?;
registry.disable(&endpoint_id)?;
registry.unregister(&endpoint_id);
```

### Event Wildcards

```rust
// Subscribe to all user events
let endpoint = WebhookEndpoint::builder("https://example.com")
    .events(vec!["user.*"])
    .build();

// Subscribe to all events
let endpoint = WebhookEndpoint::builder("https://example.com")
    .all_events()
    .build();

// Subscribe to specific events only
let endpoint = WebhookEndpoint::builder("https://example.com")
    .events(vec!["user.created", "order.completed"])
    .build();
```

### Failure Tracking

```rust
// Record delivery results
registry.record_success(&endpoint_id)?;
registry.record_failure(&endpoint_id)?;

// Find problematic endpoints
let failing = registry.get_failing_endpoints(5); // ≥5 consecutive failures

// Auto-disable failing endpoints
if endpoint.failure_count >= 10 {
    registry.disable(&endpoint.id)?;
}
```

## Signature Security

### How Signatures Work

Webhooks are signed using HMAC-SHA256:

1. A timestamp is included to prevent replay attacks
2. The signature covers both timestamp and payload
3. Format: `t=1234567890,v1=<hex-encoded-signature>`

### Signature Format

```
X-Webhook-Signature: t=1234567890,v1=abc123def456...
```

Where:
- `t` = Unix timestamp when signature was created
- `v1` = HMAC-SHA256 signature (hex encoded)

### Timestamp Validation

```rust
// Default: 5 minutes tolerance
let receiver = WebhookReceiver::new("secret");

// Custom tolerance
let receiver = WebhookReceiver::new("secret")
    .with_tolerance(60); // 60 seconds
```

### Secret Rotation

```rust
let mut endpoint = registry.get(&endpoint_id).unwrap();

// Rotate to a new secret
let new_secret = endpoint.rotate_secret();
println!("New secret: {}", new_secret);

// Update in registry
registry.update(&endpoint_id, endpoint)?;
```

## Retry Policies

### Available Policies

```rust
use armature_webhooks::RetryPolicy;
use std::time::Duration;

// No retries
let policy = RetryPolicy::none();

// Fixed delay between retries
let policy = RetryPolicy::fixed(3, Duration::from_secs(5));
// Retries: 5s, 5s, 5s

// Exponential backoff (default)
let policy = RetryPolicy::exponential(5);
// Retries: 1s, 2s, 4s, 8s, 16s (capped at max_delay)

// Custom configuration
let policy = RetryPolicy {
    max_attempts: 5,
    initial_delay: Duration::from_secs(2),
    max_delay: Duration::from_secs(120),
    backoff_multiplier: 3.0,
    jitter: true,
};
```

### Configuring the Client

```rust
let config = WebhookConfig::builder()
    .retry_policy(RetryPolicy::exponential(5))
    .timeout_secs(30)
    .build();

let client = WebhookClient::new(config);
```

### Retryable Status Codes

The client automatically retries on these HTTP status codes:
- `408` - Request Timeout
- `429` - Too Many Requests
- `500` - Internal Server Error
- `502` - Bad Gateway
- `503` - Service Unavailable
- `504` - Gateway Timeout

## Best Practices

### 1. Always Verify Signatures

```rust
// ❌ Bad - no verification
let payload: WebhookPayload = serde_json::from_slice(&body)?;

// ✅ Good - verify first
let payload = receiver.receive(&body, &signature)?;
```

### 2. Use Idempotency

```rust
// Store the webhook ID to prevent duplicate processing
let payload = receiver.receive(&body, &signature)?;

if already_processed(&payload.id) {
    return Ok(()); // Skip duplicate
}

process_webhook(&payload)?;
mark_processed(&payload.id);
```

### 3. Return Quickly

```rust
// ❌ Bad - slow processing blocks response
let payload = receiver.receive(&body, &signature)?;
heavy_processing(&payload)?; // Takes 30 seconds

// ✅ Good - queue for async processing
let payload = receiver.receive(&body, &signature)?;
queue.enqueue(payload)?;
Ok(()) // Return 200 immediately
```

### 4. Handle Failures Gracefully

```rust
let delivery = client.send(url, payload).await?;

match delivery.status {
    WebhookDeliveryStatus::Succeeded => {
        log::info!("Webhook delivered: {}", delivery.id);
    }
    WebhookDeliveryStatus::PermanentlyFailed => {
        log::error!("Webhook failed after {} attempts", delivery.attempts);
        // Alert, disable endpoint, etc.
    }
    _ => {
        // Pending or in-progress
    }
}
```

## Common Pitfalls

- ❌ **Don't** trust unverified webhooks
- ❌ **Don't** process webhooks synchronously if they take time
- ❌ **Don't** ignore timestamp validation (replay attacks)
- ✅ **Do** implement idempotency
- ✅ **Do** return 200 quickly and process asynchronously
- ✅ **Do** monitor and alert on failing endpoints

## API Reference

### WebhookClient

```rust
impl WebhookClient {
    fn new(config: WebhookConfig) -> Self;
    fn with_registry(config: WebhookConfig, registry: Arc<WebhookRegistry>) -> Self;
    async fn send(&self, url: &str, payload: WebhookPayload) -> Result<WebhookDelivery>;
    async fn send_with_secret(&self, url: &str, payload: WebhookPayload, secret: Option<&str>) -> Result<WebhookDelivery>;
    async fn send_to_endpoint(&self, endpoint: &WebhookEndpoint, payload: WebhookPayload) -> Result<WebhookDelivery>;
    async fn dispatch(&self, payload: WebhookPayload) -> Result<Vec<WebhookDelivery>>;
}
```

### WebhookReceiver

```rust
impl WebhookReceiver {
    fn new(secret: impl Into<String>) -> Self;
    fn with_tolerance(self, seconds: u64) -> Self;
    fn verify(&self, payload: &[u8], signature: &str) -> Result<bool>;
    fn receive(&self, payload: &[u8], signature: &str) -> Result<WebhookPayload>;
}
```

### WebhookPayload

```rust
impl WebhookPayload {
    fn new(event: impl Into<String>) -> Self;
    fn with_data(self, data: serde_json::Value) -> Self;
    fn with_metadata(self, metadata: serde_json::Value) -> Self;
    fn to_bytes(&self) -> Result<Vec<u8>>;
    fn to_json(&self) -> Result<String>;
}
```

### WebhookEndpoint

```rust
impl WebhookEndpoint {
    fn new(url: impl Into<String>) -> Self;
    fn builder(url: impl Into<String>) -> WebhookEndpointBuilder;
    fn is_subscribed_to(&self, event: &str) -> bool;
    fn rotate_secret(&mut self) -> String;
}
```

### WebhookRegistry

```rust
impl WebhookRegistry {
    fn new() -> Self;
    fn register(&self, endpoint: WebhookEndpoint) -> String;
    fn unregister(&self, id: &str) -> Option<WebhookEndpoint>;
    fn get(&self, id: &str) -> Option<WebhookEndpoint>;
    fn get_endpoints_for_event(&self, event: &str) -> Vec<WebhookEndpoint>;
    fn enable(&self, id: &str) -> Result<()>;
    fn disable(&self, id: &str) -> Result<()>;
    fn record_success(&self, id: &str) -> Result<()>;
    fn record_failure(&self, id: &str) -> Result<()>;
}
```

## Summary

**Key Points:**

1. Use `WebhookClient` to send outgoing webhooks
2. Use `WebhookReceiver` to verify incoming webhooks
3. Use `WebhookRegistry` to manage multiple endpoints
4. Always verify signatures before processing
5. Implement idempotency for reliable processing
6. Configure appropriate retry policies

**Quick Start:**

```rust
use armature_webhooks::{WebhookClient, WebhookConfig, WebhookPayload};

// Send a webhook
let client = WebhookClient::new(WebhookConfig::default());
let payload = WebhookPayload::new("user.created")
    .with_data(json!({"user_id": "123"}));
client.send("https://example.com/webhook", payload).await?;

// Receive a webhook
let receiver = WebhookReceiver::new("secret");
let payload = receiver.receive(&body, &signature)?;
```

