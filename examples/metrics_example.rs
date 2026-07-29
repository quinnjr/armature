#![allow(clippy::needless_question_mark)]
//! Prometheus Metrics Example
//!
//! This example demonstrates how to use Armature's metrics module
//! to collect and expose Prometheus metrics.
//!
//! Run with:
//! ```bash
//! cargo run --example metrics_example
//! ```
//!
//! Test with:
//! ```bash
//! # Make some requests
//! curl http://localhost:3000/
//! curl http://localhost:3000/api/users
//! curl http://localhost:3000/api/posts
//!
//! # View metrics
//! curl http://localhost:3000/metrics
//! ```

use armature_core::handler::handler;
use armature_core::*;
use armature_metrics::*;
use armature_proc_macro::use_middleware;
use std::sync::LazyLock;

// Custom business metrics.
//
// These are registered once, on first access, and shared by every handler
// below. Handlers wrapped with `#[use_middleware(...)]` are plain free
// functions (the attribute macro only accepts `ItemFn`, not closures), so
// they can't capture local variables the way the old inline-closure routes
// did -- these `LazyLock` statics take the place of that captured state.
static PAGE_VIEWS: LazyLock<Counter> = LazyLock::new(|| {
    CounterBuilder::new("page_views_total", "Total page views")
        .register()
        .expect("failed to register page_views_total")
});

static ACTIVE_USERS: LazyLock<Gauge> = LazyLock::new(|| {
    GaugeBuilder::new("active_users", "Number of active users")
        .register()
        .expect("failed to register active_users")
});

static API_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    HistogramBuilder::new("api_latency_seconds", "API call latency")
        .latency_buckets()
        .register()
        .expect("failed to register api_latency_seconds")
});

// Route handlers.
//
// Each handler is wrapped in `RequestMetricsMiddleware` via the real
// `#[use_middleware(...)]` attribute macro, so every request actually flows
// through it -- unlike the previous version of this example, which built a
// `RequestMetricsMiddleware` and immediately discarded it. That is what
// makes the automatic HTTP-level metrics (`http_requests_total`,
// `http_request_duration_seconds`, `http_requests_in_flight`,
// `http_request_size_bytes`, `http_response_size_bytes`) genuinely populate
// once the server is hit.

/// Home endpoint.
#[use_middleware(RequestMetricsMiddleware::new())]
async fn home(_req: HttpRequest) -> Result<HttpResponse, Error> {
    // Increment page views counter
    PAGE_VIEWS.inc();

    Ok(HttpResponse::ok().with_json(&serde_json::json!({
        "message": "Metrics Example API",
        "endpoints": {
            "/": "Home",
            "/api/users": "List users",
            "/api/posts": "List posts",
            "/metrics": "Prometheus metrics"
        }
    }))?)
}

/// Users endpoint.
#[use_middleware(RequestMetricsMiddleware::new())]
async fn list_users(_req: HttpRequest) -> Result<HttpResponse, Error> {
    let start = std::time::Instant::now();

    // Simulate some work
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Update active users gauge
    ACTIVE_USERS.inc();

    // Record API latency
    let duration = start.elapsed().as_secs_f64();
    API_LATENCY.observe(duration);

    // Decrease active users after response
    ACTIVE_USERS.dec();

    Ok(HttpResponse::ok().with_json(&serde_json::json!({
        "users": [
            {"id": 1, "name": "Alice"},
            {"id": 2, "name": "Bob"},
            {"id": 3, "name": "Charlie"}
        ]
    }))?)
}

/// Posts endpoint.
#[use_middleware(RequestMetricsMiddleware::new())]
async fn list_posts(_req: HttpRequest) -> Result<HttpResponse, Error> {
    // Simulate some work
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

    Ok(HttpResponse::ok().with_json(&serde_json::json!({
        "posts": [
            {"id": 1, "title": "First Post"},
            {"id": 2, "title": "Second Post"}
        ]
    }))?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Prometheus Metrics Example");
    info!("===========================");

    // Register custom business metrics up front so they show up in
    // `/metrics` (at zero) even before the first request comes in.
    info!("\nCreating custom metrics...");
    LazyLock::force(&PAGE_VIEWS);
    LazyLock::force(&ACTIVE_USERS);
    LazyLock::force(&API_LATENCY);
    info!("✓ Registered custom metrics");

    // Create router
    let mut router = Router::new();

    // Home endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: handler(home),
        constraints: None,
    });

    // Users endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/users".to_string(),
        handler: handler(list_users),
        constraints: None,
    });

    // Posts endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/posts".to_string(),
        handler: handler(list_posts),
        constraints: None,
    });

    // Metrics endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/metrics".to_string(),
        handler: create_metrics_handler(),
        constraints: None,
    });

    info!(
        "\n✓ RequestMetricsMiddleware is wired onto every route above via #[use_middleware(...)]"
    );

    // Build application
    let container = Container::new();
    let app = Application::new(container, router);

    info!("\n✓ Server started on http://localhost:3000");
    info!("\nAvailable endpoints:");
    info!("  GET http://localhost:3000/");
    info!("  GET http://localhost:3000/api/users");
    info!("  GET http://localhost:3000/api/posts");
    info!("  GET http://localhost:3000/metrics  ← Prometheus metrics");
    info!("\nMetrics being collected:");
    info!("  - page_views_total (Counter)");
    info!("  - active_users (Gauge)");
    info!("  - api_latency_seconds (Histogram)");
    info!("  - http_requests_total (Counter)");
    info!("  - http_request_duration_seconds (Histogram)");
    info!("  - http_requests_in_flight (Gauge)");
    info!("  - http_request_size_bytes (Histogram)");
    info!("  - http_response_size_bytes (Histogram)");
    info!("\nPress Ctrl+C to stop");

    app.listen(3000).await?;

    Ok(())
}
