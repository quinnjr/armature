# armature-cloudrun

Google Cloud Run deployment utilities for the Armature framework.

## Features

- **Container Ready** - Optimized for Cloud Run containers
- **Health Checks** - Built-in health endpoints
- **Graceful Shutdown** - Handle SIGTERM properly
- **Port Configuration** - Respect PORT environment variable

## Installation

```toml
[dependencies]
armature-cloudrun = "0.1"
```

## Quick Start

```rust,ignore
use armature::prelude::*;
use armature_cloudrun::{CloudRunConfig, init_tracing};

#[controller("/")]
struct HelloController;

#[controller_impl]
impl HelloController {
    #[get("/")]
    async fn hello() -> &'static str {
        "Hello from Cloud Run!"
    }
}

#[module(controllers: [HelloController])]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Stackdriver-formatted structured logging for Cloud Logging.
    init_tracing();

    // Reads PORT (and K_SERVICE/K_REVISION) from the Cloud Run environment.
    let config = CloudRunConfig::from_env();

    Application::create::<AppModule>()
        .listen(&config.bind_address())
        .await?;

    Ok(())
}
```

## Health checks

`HealthCheck` computes a `HealthCheckResult` from any checkers you register and
can serve it over HTTP. Run it on a sidecar port, or call `handle_request`
from your own hyper server:

```rust,ignore
use armature_cloudrun::{FnHealthChecker, HealthCheck};

let health = HealthCheck::new();
health
    .register(FnHealthChecker::new("db", || async { Ok(()) }))
    .await;

// GET /health|/healthz|/readyz -> full readiness (200/503 + JSON)
// GET /livez                   -> liveness
tokio::spawn(health.clone().serve("0.0.0.0:8081".parse().unwrap()));
```

## Graceful shutdown

Cloud Run sends `SIGTERM` when scaling down. `wait_for_shutdown()` resolves on
`SIGTERM`/`SIGINT` so you can drain in-flight work (e.g. flip
`HealthCheck::mark_unhealthy()` first):

```rust,ignore
use armature_cloudrun::wait_for_shutdown;

wait_for_shutdown().await;
```

## Dockerfile

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/my-app /app/my-app
CMD ["/app/my-app"]
```

## License

MIT OR Apache-2.0

