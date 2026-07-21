# armature-metrics

Prometheus metrics and monitoring for the Armature framework.

## Features

- **Prometheus Format** - Standard `/metrics` text exposition
- **Auto Instrumentation** - HTTP request metrics middleware
- **Custom Metrics** - Counters, gauges, histograms, summaries
- **Labels** - Dimensional metrics via the `*_vec` helpers
- **Process Metrics** - CPU/memory series on Linux (via procfs)

## Installation

```toml
[dependencies]
armature-metrics = "0.1"
```

## Quick Start

Metric constructors register into the shared default registry and are
fallible (a duplicate name or invalid label returns `prometheus::Error`), so
they return a `Result`:

```rust
use armature_metrics::{register_counter, register_histogram, export_metrics};

// Register a counter (returns Result — name/label validation can fail).
let request_counter = register_counter("http_requests_total", "Total HTTP requests")?;
let response_time = register_histogram("http_response_time_seconds", "Response time")?;

// Record metrics.
request_counter.inc();
response_time.observe(0.042);

// Export everything registered in the default registry.
let body: String = export_metrics();
# Ok::<(), prometheus::Error>(())
```

Builders are available for a fluent style:

```rust
use armature_metrics::{CounterVecBuilder, HistogramVecBuilder};

let requests = CounterVecBuilder::new("http_requests_total", "Total HTTP requests")
    .labels(&["method", "status"])
    .register()?;
requests.with_label_values(&["GET", "200"]).inc();

let latency = HistogramVecBuilder::new("http_request_duration_seconds", "Latency")
    .labels(&["method"])
    .latency_buckets()
    .register()?;
latency.with_label_values(&["GET"]).observe(0.123);
# Ok::<(), prometheus::Error>(())
```

## Metric Types

- `register_counter` / `register_counter_vec`
- `register_gauge` / `register_gauge_vec`
- `register_histogram` / `register_histogram_with_buckets` / `register_histogram_vec` / `register_histogram_vec_with_buckets`
- `register_summary` / `register_summary_with_opts` — φ-quantiles over a sliding window plus cumulative count/sum

```rust
use armature_metrics::register_summary;

let latency = register_summary("db_query_seconds", "DB query latency")?;
latency.observe(0.008);
assert!(latency.quantile(0.99).is_some());
# Ok::<(), prometheus::Error>(())
```

## The `/metrics` Endpoint

`metrics_handler` (or `create_metrics_handler` for a boxed handler) serves the
default registry in Prometheus text format:

```rust
use armature_core::{Route, HttpMethod, Router};
use armature_metrics::create_metrics_handler;

let mut router = Router::new();
router.add_route(Route {
    method: HttpMethod::GET,
    path: "/metrics".to_string(),
    handler: create_metrics_handler(),
    constraints: None,
});
```

## Auto Instrumentation

`RequestMetricsMiddleware` implements `armature_core::Middleware` and records
requests automatically:

```rust
use armature_metrics::RequestMetricsMiddleware;
use std::sync::Arc;

// Safe default: no per-path label cardinality.
let middleware = Arc::new(RequestMetricsMiddleware::new());

// Opt in to bounded per-path labels (capped to avoid unbounded series).
let with_paths = Arc::new(RequestMetricsMiddleware::with_path());
```

Records:

- `http_requests_total` - request count by method, path, status
- `http_request_duration_seconds` - request duration histogram
- `http_requests_in_flight` - active requests gauge
- `http_request_size_bytes` / `http_response_size_bytes` - payload size histograms

Path labelling is **disabled by default** because the middleware sees the raw
request path, not the matched route template; enabling it (`with_path`) bounds
distinct path labels and folds the overflow into a shared `<other>` bucket.

## License

MIT OR Apache-2.0
