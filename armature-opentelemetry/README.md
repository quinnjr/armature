# armature-opentelemetry

OpenTelemetry integration for the Armature framework.

## Features

- **Distributed Tracing** – automatic server spans per request
- **Metrics** – request count, duration, and in-flight gauges
- **Context Propagation** – W3C Trace Context (`traceparent`/`tracestate`)
- **Exporters** – OTLP over gRPC (default) and Zipkin (`zipkin` feature)
- **Middleware** – drop-in `TelemetryMiddleware` for automatic instrumentation

> The legacy Jaeger and Prometheus exporters were discontinued upstream for
> OpenTelemetry 0.32. Export to an OpenTelemetry Collector via OTLP instead and
> let the Collector fan out to Jaeger/Prometheus backends.

## Cargo features

| Feature  | Default | Description                          |
| -------- | ------- | ------------------------------------ |
| `otlp`   | yes     | OTLP/gRPC span + metric exporters    |
| `zipkin` | no      | Zipkin span exporter (reqwest/rustls)|
| `full`   | no      | `otlp` + `zipkin`                     |

## Installation

```toml
[dependencies]
armature-opentelemetry = "0.1"
```

## Quick Start

```rust,no_run
use armature_opentelemetry::TelemetryBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build and initialize telemetry (installs the tracer/meter providers
    // and the W3C Trace Context propagator).
    let telemetry = TelemetryBuilder::new("my-service")
        .with_version("1.0.0")
        .with_environment("production")
        .with_otlp_endpoint("http://localhost:4317")
        .with_tracing()
        .with_metrics()
        .build()
        .await?;

    // Mount the middleware for automatic per-request spans and metrics:
    //
    //     let app = Application::new(container, router)
    //         .with_middleware(telemetry.middleware());

    // Flush and shut down the exporters on exit.
    telemetry.shutdown().await?;
    Ok(())
}
```

## Configuration

`TelemetryBuilder` wraps a `TelemetryConfig`, which you can also build directly
and pass with `.with_config(...)`:

```rust
use armature_opentelemetry::TelemetryConfig;

let config = TelemetryConfig::new("api-service")
    .with_version("2.1.0")
    .with_namespace("backend")
    .with_environment("staging")
    .with_tracing(true)
    .with_metrics(true);

assert_eq!(config.service_name, "api-service");
```

Span limits (`max_attributes_per_span`, `max_events_per_span`) and the metrics
`collection_interval_secs` live on `config.tracing` / `config.metrics` and are
applied to the SDK providers when telemetry is initialized.

## Lower-level initialization

If you manage providers yourself, call the init functions directly:

```rust,no_run
use armature_opentelemetry::{init_tracing, init_metrics, TelemetryConfig};

# async fn run() -> Result<(), armature_opentelemetry::TelemetryError> {
let config = TelemetryConfig::new("my-service");
let tracer_provider = init_tracing(&config).await?;
let meter_provider = init_metrics(&config).await?;
# let _ = (tracer_provider, meter_provider);
# Ok(())
# }
```

`init_tracing` installs the global W3C Trace Context propagator, so
`TelemetryMiddleware` parses inbound `traceparent` headers into the parent
context for distributed traces.

## Custom spans

```rust,no_run
use armature_opentelemetry::global;
use opentelemetry::trace::Tracer;

let tracer = global::tracer("my-service");
let _span = tracer.start("process_request");
// ... do work; the span ends when dropped ...
```

## License

MIT OR Apache-2.0
