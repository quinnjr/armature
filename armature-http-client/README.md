# armature-http-client

HTTP client for the Armature framework.

## Features

- **Retry Logic** - Automatic retries with backoff
- **Circuit Breaker** - Fail fast on repeated failures
- **Timeouts** - Request and connection timeouts
- **Connection Pooling** - Efficient connection reuse
- **Interceptors** - Request/response middleware

## Installation

```toml
[dependencies]
armature-http-client = "0.1"
```

## Quick Start

```rust
use armature_http_client::{HttpClient, HttpClientConfig, RetryConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = HttpClientConfig::builder()
        .timeout(Duration::from_secs(30))
        .retry(RetryConfig::exponential(3, Duration::from_millis(100)))
        .build();

    let client = HttpClient::new(config);

    // GET request
    let response = client
        .get("https://api.example.com/users")
        .send()
        .await?;

    // POST with JSON
    let user = client
        .post("https://api.example.com/users")
        .json(&serde_json::json!({ "name": "John" }))?
        .send()
        .await?;

    Ok(())
}
```

## Circuit Breaker

```rust
use armature_http_client::{HttpClient, HttpClientConfig, CircuitBreakerConfig};
use std::time::Duration;

let config = HttpClientConfig::builder()
    .circuit_breaker(CircuitBreakerConfig {
        failure_threshold: 5,
        success_threshold: 2,
        reset_timeout: Duration::from_secs(30),
        half_open_requests: 3,
        failure_window: Duration::from_secs(60),
    })
    .build();

let client = HttpClient::new(config);
```

## License

MIT OR Apache-2.0

