//! Core type micro-benchmarks.
//!
//! Times the primitives every request touches - `HttpRequest`/`HttpResponse`
//! construction, JSON body encode/decode, header map operations, middleware
//! construction, `Route`/`Router` setup and DI container resolution - in
//! isolation, with no server, socket or runtime in the path. For end-to-end
//! request cost see `internal_overhead_benchmarks.rs`; for cross-framework HTTP
//! numbers see `benches/comparison_servers/`.
//!
//! ```bash
//! cargo bench --bench core_benchmarks
//! ```

#![allow(deprecated)]
#![allow(clippy::needless_question_mark)]

use armature_core::handler::from_legacy_handler;
use armature_core::*;
use bytes::Bytes;
use criterion::{Criterion, criterion_group, criterion_main};
use std::collections::HashMap;
use std::hint::black_box;

fn bench_http_request_creation(c: &mut Criterion) {
    c.bench_function("http_request_new", |b| {
        b.iter(|| {
            // `&'static str`, not `String`: allocating the inputs inside the
            // timed loop charges construction for two heap allocations the
            // serve path does not make.
            HttpRequest::new(black_box("GET"), black_box("/api/users"))
        })
    });
}

fn bench_http_response_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_response");

    group.bench_function("ok", |b| b.iter(HttpResponse::ok));

    group.bench_function("with_json", |b| {
        let data = serde_json::json!({"message": "Hello, World!"});
        b.iter(|| HttpResponse::ok().with_json(&data))
    });

    group.bench_function("with_body", |b| {
        let body = b"Hello, World!".to_vec();
        b.iter(|| HttpResponse::ok().with_body(black_box(body.clone())))
    });

    group.finish();
}

fn bench_json_parsing(c: &mut Criterion) {
    #[allow(dead_code)]
    #[derive(serde::Deserialize)]
    struct TestData {
        id: u64,
        name: String,
        email: String,
        active: bool,
    }

    let json_data = br#"{"id":123,"name":"John Doe","email":"john@example.com","active":true}"#;
    let mut request = HttpRequest::new("POST", "/api/test".to_string());
    request.body = Bytes::copy_from_slice(json_data);

    c.bench_function("json_parse", |b| {
        b.iter(|| {
            let _: TestData = black_box(&request).json().unwrap();
        })
    });
}

fn bench_form_parsing(c: &mut Criterion) {
    let form_data = b"name=John+Doe&email=john%40example.com&age=30&city=New+York";
    let mut request = HttpRequest::new("POST", "/api/form".to_string());
    request.body = Bytes::copy_from_slice(form_data);

    c.bench_function("form_parse_map", |b| {
        b.iter(|| {
            let _: HashMap<String, String> = black_box(&request).form_map().unwrap();
        })
    });
}

fn bench_middleware_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("middleware");

    group.bench_function("logger_creation", |b| b.iter(LoggerMiddleware::new));

    group.bench_function("cors_creation", |b| b.iter(CorsMiddleware::new));

    group.finish();
}

/// UUID v4 generation plus its hyphenated string form.
///
/// This is what a request-id middleware pays per request, but it measures
/// `Uuid`, not middleware. It used to sit in the `middleware` group above,
/// where the name implied Armature was being timed rather than the `uuid`
/// crate.
fn bench_uuid_generation(c: &mut Criterion) {
    c.bench_function("uuid_v4_to_string", |b| {
        b.iter(|| {
            let id = uuid::Uuid::new_v4().to_string();
            black_box(id);
        })
    });
}

fn bench_routing(c: &mut Criterion) {
    let mut group = c.benchmark_group("routing");

    group.bench_function("route_creation", |b| {
        use std::sync::Arc;
        b.iter(|| {
            let handler = from_legacy_handler(Arc::new(|_req: HttpRequest| {
                Box::pin(async { Ok(HttpResponse::ok()) })
            }));

            let route = Route {
                method: HttpMethod::GET,
                path: "/api/test".to_string(),
                handler,
                constraints: None,
            };
            black_box(route);
        })
    });

    // NOTE: this benchmarks bare stdlib `str::split`, not any Armature path-parsing
    // code — it's a raw baseline for comparison, not a measurement of Armature's own
    // parser. For the real path-splitting benchmark, see
    // `benches/simd_parser_benchmarks.rs`.
    group.bench_function("raw_str_split_baseline", |b| {
        b.iter(|| {
            let path = black_box("/api/users/123/profile");
            let parts: Vec<&str> = path.split('/').collect();
            black_box(parts);
        })
    });

    group.finish();
}

fn bench_status_code_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("status_codes");

    group.bench_function("from_code", |b| {
        b.iter(|| HttpStatus::from_code(black_box(404)))
    });

    group.bench_function("is_success", |b| {
        b.iter(|| black_box(HttpStatus::Ok).is_success())
    });

    group.bench_function("is_error", |b| {
        b.iter(|| black_box(HttpStatus::NotFound).is_client_error())
    });

    group.finish();
}

fn bench_error_handling(c: &mut Criterion) {
    let mut group = c.benchmark_group("error_handling");

    group.bench_function("error_creation", |b| {
        b.iter(|| Error::NotFound(black_box("Resource not found".to_string())))
    });

    group.bench_function("error_status_code", |b| {
        let err = Error::NotFound("Not found".to_string());
        b.iter(|| black_box(&err).status_code())
    });

    group.finish();
}

fn bench_container_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("container");

    group.bench_function("string_allocation", |b| {
        b.iter(|| {
            let _: String = black_box("test_value").to_string();
        })
    });

    group.bench_function("hashmap_insert", |b| {
        b.iter(|| {
            let mut map = HashMap::new();
            map.insert(black_box("key"), black_box("value"));
            black_box(map);
        })
    });

    group.finish();
}

criterion_group!(
    core_benches,
    bench_http_request_creation,
    bench_http_response_creation,
    bench_json_parsing,
    bench_form_parsing,
    bench_middleware_operations,
    bench_uuid_generation,
    bench_routing,
    bench_status_code_operations,
    bench_error_handling,
    bench_container_operations,
);

criterion_main!(core_benches);
