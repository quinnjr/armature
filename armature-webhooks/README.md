# armature-webhooks

Webhook handling for the Armature framework.

## Features

- **Signature Verification** - HMAC-SHA256/SHA512 webhook signature signing and verification
- **Retry Support** - Automatic delivery retries with configurable backoff
- **Event Registry** - Subscribe endpoints to specific event types and dispatch to all of them

## Installation

```toml
[dependencies]
armature-webhooks = "0.1"
```

## Quick Start

### Receiving webhooks

```rust
use armature_webhooks::WebhookReceiver;

let receiver = WebhookReceiver::new("your-webhook-secret");

// Create a handler that only processes "order.*" events
let handler = receiver.handler("order.*", |event| {
    match event.event.as_str() {
        "order.created" => {
            // process_order(event.data)
        }
        _ => {}
    }
    Ok(())
});

// In your request handling code, verify + dispatch a raw request body
// and its `X-Webhook-Signature` header value:
let handled = handler.handle(payload_bytes, signature_header)?;
```

### Sending webhooks

```rust
use armature_webhooks::{WebhookClient, WebhookConfig, WebhookPayload};

let client = WebhookClient::new(WebhookConfig::default());

let payload = WebhookPayload::new("user.created")
    .with_data(serde_json::json!({
        "user_id": "123",
        "email": "user@example.com"
    }));

// Sign and send with a specific secret
client.send_with_secret("https://example.com/webhook", payload, Some("endpoint-secret")).await?;
```

## License

MIT OR Apache-2.0
