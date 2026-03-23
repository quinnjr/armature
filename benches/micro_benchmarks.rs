//! Benchmarks for the micro-framework API
//!
//! Measures overhead of the micro-framework compared to direct router usage.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse, Router};

// Test handlers
async fn simple_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
    Ok(HttpResponse::ok())
}

async fn json_handler(_req: HttpRequest) -> Result<HttpResponse, Error> {
    HttpResponse::json(&serde_json::json!({
        "message": "Hello, World!",
        "status": "ok"
    }))
}

async fn param_handler(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id = req.param("id").cloned().unwrap_or_default();
    HttpResponse::json(&serde_json::json!({ "id": id }))
}

fn bench_app_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro_app_creation");

    group.bench_function("empty_app", |b| b.iter(|| black_box(App::new().build())));

    group.bench_function("app_with_5_routes", |b| {
        b.iter(|| {
            black_box(
                App::new()
                    .route("/", get(simple_handler))
                    .route("/users", get(simple_handler).post(simple_handler))
                    .route("/users/:id", get(param_handler))
                    .route("/health", get(simple_handler))
                    .build(),
            )
        })
    });

    group.bench_function("app_with_20_routes", |b| {
        b.iter(|| {
            let mut app = App::new();
            for i in 0..20 {
                app = app.route(&format!("/route{}", i), get(simple_handler));
            }
            black_box(app.build())
        })
    });

    group.bench_function("app_with_scope", |b| {
        b.iter(|| {
            black_box(
                App::new()
                    .service(
                        scope("/api/v1")
                            .route("/users", get(simple_handler).post(simple_handler))
                            .route(
                                "/users/:id",
                                get(param_handler)
                                    .put(simple_handler)
                                    .delete(simple_handler),
                            )
                            .route("/posts", get(simple_handler))
                            .route("/posts/:id", get(simple_handler)),
                    )
                    .build(),
            )
        })
    });

    group.bench_function("app_with_middleware", |b| {
        b.iter(|| {
            black_box(
                App::new()
                    .wrap(Logger::default())
                    .wrap(Cors::permissive())
                    .wrap(Compress::default())
                    .route("/", get(simple_handler))
                    .build(),
            )
        })
    });

    group.finish();
}

fn bench_routing(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("micro_routing");

    // Direct router (baseline)
    let mut direct_router = Router::new();
    direct_router.get("/", simple_handler);
    direct_router.get("/users", simple_handler);
    direct_router.get("/users/:id", param_handler);
    direct_router.get("/health", simple_handler);

    // Micro app
    let micro_app = App::new()
        .route("/", get(simple_handler))
        .route("/users", get(simple_handler))
        .route("/users/:id", get(param_handler))
        .route("/health", get(simple_handler))
        .build();

    group.bench_function("direct_router_static", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/health".to_string());
            black_box(direct_router.route(req).await.unwrap())
        })
    });

    group.bench_function("micro_app_static", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/health".to_string());
            black_box(micro_app.handle(req).await.unwrap())
        })
    });

    group.bench_function("direct_router_param", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/users/123".to_string());
            black_box(direct_router.route(req).await.unwrap())
        })
    });

    group.bench_function("micro_app_param", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/users/123".to_string());
            black_box(micro_app.handle(req).await.unwrap())
        })
    });

    group.finish();
}

fn bench_middleware_overhead(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("micro_middleware");

    // No middleware
    let app_no_mw = App::new().route("/", get(simple_handler)).build();

    // 1 middleware
    let app_1_mw = App::new()
        .wrap(Logger::default())
        .route("/", get(simple_handler))
        .build();

    // 3 middleware
    let app_3_mw = App::new()
        .wrap(Logger::default())
        .wrap(Cors::permissive())
        .wrap(Compress::default())
        .route("/", get(simple_handler))
        .build();

    group.bench_function("no_middleware", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/".to_string());
            black_box(app_no_mw.handle(req).await.unwrap())
        })
    });

    group.bench_function("1_middleware", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/".to_string());
            black_box(app_1_mw.handle(req).await.unwrap())
        })
    });

    group.bench_function("3_middleware", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/".to_string());
            black_box(app_3_mw.handle(req).await.unwrap())
        })
    });

    group.finish();
}

fn bench_state_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro_state");

    #[derive(Clone)]
    struct AppConfig {
        name: String,
        version: String,
        debug: bool,
    }

    group.bench_function("data_creation", |b| {
        b.iter(|| {
            black_box(Data::new(AppConfig {
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                debug: true,
            }))
        })
    });

    let data = Data::new(AppConfig {
        name: "test".to_string(),
        version: "1.0.0".to_string(),
        debug: true,
    });

    group.bench_function("data_access", |b| {
        b.iter(|| {
            black_box(&data.name);
            black_box(&data.version);
            black_box(data.debug);
        })
    });

    group.bench_function("data_clone", |b| b.iter(|| black_box(data.clone())));

    group.finish();
}

fn bench_route_builder(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro_route_builder");

    group.bench_function("single_method", |b| {
        b.iter(|| black_box(get(simple_handler)))
    });

    group.bench_function("multiple_methods", |b| {
        b.iter(|| {
            black_box(
                get(simple_handler)
                    .post(simple_handler)
                    .put(simple_handler)
                    .delete(simple_handler),
            )
        })
    });

    group.bench_function("any_method", |b| b.iter(|| black_box(any(simple_handler))));

    group.finish();
}

fn bench_scope_building(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro_scope");

    group.bench_function("empty_scope", |b| b.iter(|| black_box(scope("/api"))));

    group.bench_function("scope_with_routes", |b| {
        b.iter(|| {
            black_box(
                scope("/api")
                    .route("/users", get(simple_handler))
                    .route("/posts", get(simple_handler))
                    .route("/comments", get(simple_handler)),
            )
        })
    });

    group.bench_function("nested_scopes", |b| {
        b.iter(|| {
            black_box(
                scope("/api")
                    .service(scope("/v1").route("/users", get(simple_handler)))
                    .service(scope("/v2").route("/users", get(simple_handler))),
            )
        })
    });

    group.finish();
}

fn bench_json_response(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("micro_json");

    let app = App::new().route("/json", get(json_handler)).build();

    group.bench_function("json_handler", |b| {
        b.to_async(&rt).iter(|| async {
            let req = HttpRequest::new("GET".to_string(), "/json".to_string());
            black_box(app.handle(req).await.unwrap())
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_app_creation,
    bench_routing,
    bench_middleware_overhead,
    bench_state_access,
    bench_route_builder,
    bench_scope_building,
    bench_json_response,
);

criterion_main!(benches);
