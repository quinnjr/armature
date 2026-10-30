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
- [Delivery Egress Policy](#delivery-egress-policy)
- [Retry Policies](#retry-policies)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Summary](#summary)

## Overview

The `armature-webhooks` module provides comprehensive webhook support including:

- **Outgoing Webhooks**: Send HTTP callbacks to registered endpoints
- **Incoming Webhooks**: Receive and verify webhooks from external services
- **Signature Verification**: HMAC-SHA256/SHA512 signing and verification with a derived key
- **SSRF Protection**: Deliveries only reach public `https` endpoints by default
- **Automatic Retries**: Configurable retry policies with exponential backoff
- **Event System**: Subscribe endpoints to specific event types

## Features

- ✅ Send webhooks with automatic HMAC signatures (`t=<ts>,v2=<hex>`)
- ✅ Receive and verify incoming webhooks, with a replay cache
- ✅ Opt-in GitHub-style (`X-Hub-Signature-256`) receiver mode
- ✅ Egress policy: https only, no loopback/private/metadata addresses, no redirects
- ✅ Event-based endpoint subscriptions with wildcards
- ✅ Configurable retry policies (fixed, exponential backoff)
- ✅ Delivery tracking and status monitoring
- ✅ Endpoint registry with failure tracking
- ✅ Timestamp validation to prevent replay attacks
- ✅ Custom headers per endpoint (security-sensitive headers are refused)

## Installation

Add the webhooks feature to your `Cargo.toml`:

```toml
[dependencies]
armature = { package = "armature-framework", version = "0.8", features = ["webhooks"] }
```

Or use the crate directly:

```toml
[dependencies]
armature-webhooks = "0.7"
```

### Signing Secrets

Every signing secret must be at least 32 bytes (`armature_webhooks::MIN_SECRET_LEN`).
`WebhookSignature::new`, `WebhookReceiver::new`/`from_config`/`github` return
`Err(WebhookError::ConfigError(..))` for a shorter (or empty) secret, instead of
silently signing with a brute-forceable key. Load the secret from your secret store or
environment; generate one with `openssl rand -hex 32`. Endpoints created with
`WebhookEndpoint::new`/`builder` get a generated secret: `whsec_` + 32 bytes from the OS
CSPRNG (64 hex characters).

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

    // Send to a specific URL. It must pass the egress policy: https, and a
    // public address (see "Delivery Egress Policy").
    let delivery = client.send("https://example.com/webhook", payload).await?;

    println!("Delivery status: {:?}", delivery.status);
    Ok(())
}
```

### With Signing Secret

```rust
// At least 32 bytes; never hardcode it.
let secret = std::env::var("WEBHOOK_SECRET")?;

let delivery = client
    .send_with_secret("https://example.com/webhook", payload, Some(&secret))
    .await?;
```

### Dispatching to Multiple Endpoints

```rust
use std::sync::Arc;

let registry = Arc::new(WebhookRegistry::new());

// Register endpoints. `build()` validates the URL and headers against the
// strict egress policy, and `register()` validates again against the
// registry's policy; both return `Result`.
registry.register(
    WebhookEndpoint::builder("https://api1.example.com/webhook")
        .events(vec!["user.*"])
        .build()?,
)?;

registry.register(
    WebhookEndpoint::builder("https://api2.example.com/webhook")
        .events(vec!["user.created", "order.*"])
        .build()?,
)?;

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
use armature_webhooks::{WebhookError, WebhookReceiver};

// Errors if the secret is shorter than 32 bytes.
let receiver = WebhookReceiver::new(std::env::var("WEBHOOK_SECRET")?)?;

// In your HTTP handler
fn handle_webhook(
    receiver: &WebhookReceiver,
    body: &[u8],
    signature_header: &str,
) -> Result<(), WebhookError> {
    // Verify and parse in one step
    let payload = receiver.receive(body, signature_header)?;

    println!("Received event: {}", payload.event);
    println!("Data: {:?}", payload.data);

    Ok(())
}
```

A receiver remembers every signature it accepted (and the delivery id, when the sender
sends one) for the timestamp tolerance window, so a captured request replayed within the
window is rejected with `WebhookError::ReplayDetected`. The cache is per process and
holds 100,000 entries by default (`with_replay_cache_capacity`); when it is full the
receiver fails closed with `WebhookError::ReplayCacheFull`.

### Verify from HTTP Headers

```rust
use std::collections::HashMap;

let mut headers = HashMap::new();
headers.insert("X-Webhook-Signature".to_string(), signature.to_string());
// Optional: the delivery id is added to the replay cache too.
headers.insert("X-Webhook-Id".to_string(), delivery_id.to_string());

let is_valid = receiver.verify_from_headers(&body, &headers)?;
// Or verify and parse in one step:
let payload = receiver.receive_from_request(&body, &headers)?;
```

Header names are matched case-insensitively. An Armature receiver reads only
`X-Webhook-Signature`/`X-Webhook-Id`.

### Receiving GitHub-Style Webhooks

GitHub (and providers that copy it) sign with `X-Hub-Signature-256: sha256=<hex>`, a MAC
over the body only, with no timestamp. This scheme is **not** accepted by a normal
receiver; build a dedicated receiver for it:

```rust
use armature_webhooks::{SigningAlgorithm, WebhookReceiver};
use std::time::Duration;

// Use a secret dedicated to this provider (at least 32 bytes).
let github = WebhookReceiver::github(std::env::var("GITHUB_WEBHOOK_SECRET")?)?
    // Remember deliveries for 24h (the default) to reject replays.
    .with_github_replay_window(Duration::from_secs(24 * 60 * 60));

// Reads only X-Hub-Signature-256 and X-GitHub-Delivery.
let payload = github.receive_from_request(&body, &headers)?;

// HMAC-SHA512: expects `sha512=` in X-Hub-Signature-512.
let github_512 = WebhookReceiver::github(secret)?
    .with_algorithm(SigningAlgorithm::HmacSha512);
```

Because the scheme has no timestamp, the replay cache is its only replay protection: a
request replayed after the window, or against another replica, still verifies. Prefer the
Armature scheme whenever you control both sides.

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

// Create an endpoint with builder. `build()` returns `Result`: it rejects
// URLs refused by the egress policy and denied custom headers.
let endpoint = WebhookEndpoint::builder("https://api.example.com/webhooks")
    .events(vec!["user.created", "user.updated", "order.*"])
    .description("Main API webhook")
    .header("X-Custom-Header", "custom-value")
    .build()?;

let endpoint_id = registry.register(endpoint)?;

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
    .build()?;

// Subscribe to all events
let endpoint = WebhookEndpoint::builder("https://example.com")
    .all_events()
    .build()?;

// Subscribe to specific events only
let endpoint = WebhookEndpoint::builder("https://example.com")
    .events(vec!["user.created", "order.completed"])
    .build()?;
```

### Custom Headers

Custom endpoint headers may not override security-relevant ones. `Host`, `Authorization`,
`Cookie`, `Proxy-*`, `Metadata*`, `X-aws-ec2-metadata-token*`, `X-Google-Metadata-Request`,
hop-by-hop and framing headers, `Content-*`, `X-Webhook-*`, `X-Hub-Signature*`,
`Forwarded` and `X-Forwarded-*` are refused (`WebhookError::InvalidEndpoint`) by
`build()`, by `WebhookRegistry::register`/`update`, and again before every delivery. Use
`armature_webhooks::is_denied_header(name)` to check a name up front.

### Failure Tracking

```rust
// Record delivery results
registry.record_success(&endpoint_id)?;
registry.record_failure(&endpoint_id)?;

// Find problematic endpoints
let failing = registry.get_failing_endpoints(5); // ≥5 consecutive failures

// Auto-disable failing endpoints
for endpoint in registry.get_failing_endpoints(10) {
    registry.disable(&endpoint.id)?;
}
```

## Signature Security

### How Signatures Work

Webhooks are signed with HMAC-SHA256 (or HMAC-SHA512, via `SigningAlgorithm`):

1. A timestamp is included to prevent replay attacks
2. The signature covers both timestamp and payload: `"<t>.<body>"`
3. The MAC key is **derived** from the shared secret, not the secret itself
4. Format: `t=1234567890,v2=<hex-encoded-signature>`

### Signature Format

```
X-Webhook-Signature: t=1234567890,v2=abc123def456...
```

Where:
- `t` = Unix timestamp when signature was created
- `v2` = `hex(HMAC(K, t + "." + body))`, with `K = HMAC(key = "armature-webhooks/v2", msg = secret)`

Both HMACs use the configured hash. The derived key gives domain separation from the
GitHub scheme (`sha256=hex(HMAC(secret, body))`): under the old `v1` format both keyed
HMAC with the raw secret, so a valid `v1` signature over a body was also a valid GitHub
signature over `"<t>.<body>"`. A non-Armature receiver verifies a `v2` signature like
this:

```python
import hashlib, hmac
k = hmac.new(b"armature-webhooks/v2", secret.encode(), hashlib.sha256).digest()
expected = hmac.new(k, f"{t}.".encode() + body, hashlib.sha256).hexdigest()
ok = hmac.compare_digest(expected, v2)  # and check |now - t| <= tolerance
```

**Migration from `v1`:** `v1` and bare `sha256=` signatures are no longer accepted by
`WebhookSignature::verify` / `WebhookReceiver::new`. Upgrade senders and receivers
together; use `WebhookReceiver::github` (or `WebhookSignature::verify_github`) for
providers that only offer the GitHub scheme.

### Timestamp Validation

```rust
// Default: 5 minutes tolerance
let receiver = WebhookReceiver::new(secret.clone())?;

// Custom tolerance
let receiver = WebhookReceiver::new(secret)?
    .with_tolerance(60); // 60 seconds

// Or take tolerance and algorithm from a WebhookConfig
let receiver = WebhookReceiver::from_config(secret, &config)?;
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

`rotate_secret` generates a fresh `whsec_` secret from the OS CSPRNG. `Debug` output of
`WebhookSignature`, `WebhookReceiver` and `WebhookEndpoint` never prints the secret or
custom header values.

## Delivery Egress Policy

Webhook URLs are usually supplied by your users, so every delivery goes through an
`EgressPolicy` (re-exported from `armature-http-client`) to stop server-side request
forgery:

- Only `https://` URLs are allowed.
- Loopback, private (RFC 1918, ULA), link-local (including cloud metadata such as
  `169.254.169.254`), and other special-purpose addresses are refused, as are
  `localhost`, `.local` and `.internal` names.
- The check runs on the URL and again on **every DNS answer** when a connection is
  opened (re-resolved per new connection, and the connection is pinned to the vetted
  addresses), so DNS rebinding does not bypass it.
- Redirects are never followed and system proxies are never used.
- A refusal (`WebhookError::EgressRefused`) fails the delivery permanently, without
  retries.

The strict policy is the default. Relax it only for local development:

```rust
use armature_webhooks::{EgressPolicy, WebhookConfig, WebhookEndpoint, WebhookRegistry};

// Development only: allow a stub receiver on localhost over http.
let dev_policy = EgressPolicy::default()
    .allow_insecure_loopback(true)
    .allow_http(true);

let config = WebhookConfig::builder()
    .egress_policy(dev_policy.clone())
    .build();

let registry = WebhookRegistry::with_egress_policy(dev_policy.clone());
let endpoint = WebhookEndpoint::builder("http://127.0.0.1:8080/hook")
    .build_with_policy(&dev_policy)?;
registry.register(endpoint)?;
```

Add extra blocked name suffixes with `EgressPolicy::deny_host_suffix(".corp")`.

### Response Bodies

Receivers' response bodies are **not** stored on `WebhookDelivery` by default, because
the receiver chooses them and showing them back (for example in a delivery log) would
turn the sender into a proxy. Opt in with `.store_response_bodies(true)`; the stored body
is capped at `max_response_body_size` (8 KiB by default).

### TLS Verification

Certificates are always verified. `WebhookConfig::verify_ssl` and the builder's
`verify_ssl(false)` exist only with the `dangerous-insecure-tls` cargo feature of
`armature-webhooks`, and disabling verification logs a warning. Never enable that feature
in a production build: it lets anyone on the network path impersonate every receiver.

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
- ❌ **Don't** use secrets shorter than 32 bytes or hardcode them
- ❌ **Don't** relax the egress policy (`allow_insecure_loopback`, `allow_http`) in production
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

### WebhookConfigBuilder

```rust
impl WebhookConfigBuilder {
    fn timeout_secs(self, secs: u64) -> Self;
    fn retry_policy(self, policy: RetryPolicy) -> Self;
    fn egress_policy(self, policy: EgressPolicy) -> Self;
    fn store_response_bodies(self, store: bool) -> Self;
    fn timestamp_tolerance(self, seconds: u64) -> Self;
    fn signing_algorithm(self, algorithm: SigningAlgorithm) -> Self;
    fn max_concurrent_deliveries(self, max: usize) -> Self;
    fn max_response_body_size(self, size: usize) -> Self;
    #[cfg(feature = "dangerous-insecure-tls")]
    fn verify_ssl(self, verify: bool) -> Self;
    fn build(self) -> WebhookConfig;
}
```

### WebhookReceiver

```rust
impl WebhookReceiver {
    fn new(secret: impl Into<String>) -> Result<Self>;
    fn from_config(secret: impl Into<String>, config: &WebhookConfig) -> Result<Self>;
    fn github(secret: impl Into<String>) -> Result<Self>;
    fn with_algorithm(self, algorithm: SigningAlgorithm) -> Self;
    fn with_tolerance(self, seconds: u64) -> Self;
    fn with_github_replay_window(self, window: Duration) -> Self;
    fn with_replay_cache_capacity(self, capacity: usize) -> Self;
    fn verify(&self, payload: &[u8], signature: &str) -> Result<bool>;
    fn receive(&self, payload: &[u8], signature: &str) -> Result<WebhookPayload>;
    fn verify_from_headers(&self, payload: &[u8], headers: &HashMap<String, String>) -> Result<bool>;
    fn receive_from_request(&self, payload: &[u8], headers: &HashMap<String, String>) -> Result<WebhookPayload>;
}
```

### WebhookSignature

```rust
impl WebhookSignature {
    fn new(secret: impl Into<String>) -> Result<Self>; // >= 32 bytes
    fn with_algorithm(self, algorithm: SigningAlgorithm) -> Self;
    fn sign(&self, payload: &[u8]) -> String;            // "t=..,v2=.."
    fn verify(&self, payload: &[u8], signature: &str, tolerance_secs: u64) -> Result<bool>;
    fn verify_github(&self, payload: &[u8], signature: &str) -> Result<bool>;
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
    fn validate(&self, policy: &EgressPolicy) -> Result<()>;
    fn is_subscribed_to(&self, event: &str) -> bool;
    fn rotate_secret(&mut self) -> String;
}

impl WebhookEndpointBuilder {
    fn build(self) -> Result<WebhookEndpoint>; // strict egress policy
    fn build_with_policy(self, policy: &EgressPolicy) -> Result<WebhookEndpoint>;
}
```

### WebhookRegistry

```rust
impl WebhookRegistry {
    fn new() -> Self;
    fn with_egress_policy(policy: EgressPolicy) -> Self;
    fn register(&self, endpoint: WebhookEndpoint) -> Result<String>;
    fn update(&self, id: &str, endpoint: WebhookEndpoint) -> Result<()>;
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

1. Use `WebhookClient` to send outgoing webhooks; deliveries only reach public https endpoints
2. Use `WebhookReceiver` to verify incoming webhooks (`WebhookReceiver::github` for GitHub-style)
3. Use `WebhookRegistry` to manage multiple endpoints
4. Always verify signatures before processing; secrets are at least 32 bytes
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
let receiver = WebhookReceiver::new(std::env::var("WEBHOOK_SECRET")?)?;
let payload = receiver.receive(&body, &signature)?;
```

