//! Body Handling Benchmarks
//!
//! Compares zero-copy Bytes-based body handling vs Vec<u8> copying.
//!
//! Run with: cargo bench --bench body_benchmarks

use armature_core::body::{RequestBody, ResponseBody};
use armature_core::{HttpRequest, HttpResponse};
use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

// ============================================================================
// Request Body Creation Benchmarks
// ============================================================================

fn bench_request_body_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_body_create");

    // Create body from slice - copies data
    let data = vec![0u8; 1024];
    group.throughput(Throughput::Bytes(1024));

    group.bench_function("from_slice_1kb", |b| {
        b.iter(|| {
            let body = RequestBody::from_slice(black_box(&data));
            black_box(body.len())
        })
    });

    // Create body from Vec - zero-copy
    group.bench_function("from_vec_1kb", |b| {
        b.iter(|| {
            let vec = black_box(vec![0u8; 1024]);
            let body = RequestBody::from_vec(vec);
            black_box(body.len())
        })
    });

    // Create body from Bytes - zero-copy
    let bytes = Bytes::from(vec![0u8; 1024]);
    group.bench_function("from_bytes_1kb", |b| {
        b.iter(|| {
            let body = RequestBody::from_bytes(black_box(bytes.clone()));
            black_box(body.len())
        })
    });

    // Create body from static - zero-copy
    group.bench_function("from_static_1kb", |b| {
        static DATA: [u8; 1024] = [0u8; 1024];
        b.iter(|| {
            let body = RequestBody::from_static(black_box(&DATA));
            black_box(body.len())
        })
    });

    group.finish();
}

// ============================================================================
// Response Body Creation Benchmarks
// ============================================================================

fn bench_response_body_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("response_body_create");

    group.throughput(Throughput::Bytes(1024));

    // From Vec - zero-copy conversion
    group.bench_function("from_vec_1kb", |b| {
        b.iter(|| {
            let vec = black_box(vec![0u8; 1024]);
            let body = ResponseBody::from_vec(vec);
            black_box(body.len())
        })
    });

    // From Bytes - zero-copy
    let bytes = Bytes::from(vec![0u8; 1024]);
    group.bench_function("from_bytes_1kb", |b| {
        b.iter(|| {
            let body = ResponseBody::from_bytes(black_box(bytes.clone()));
            black_box(body.len())
        })
    });

    // From JSON
    #[derive(serde::Serialize)]
    struct TestData {
        id: u64,
        name: String,
        values: Vec<i32>,
    }

    let data = TestData {
        id: 12345,
        name: "Test User".to_string(),
        values: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    };

    group.bench_function("from_json", |b| {
        b.iter(|| {
            let body = ResponseBody::from_json(black_box(&data)).unwrap();
            black_box(body.len())
        })
    });

    group.finish();
}

// ============================================================================
// HttpRequest Body Handling Benchmarks
// ============================================================================

fn bench_http_request_body(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_request_body");

    // Old way: set body via Vec<u8> assignment
    group.bench_function("legacy_vec_assignment", |b| {
        b.iter(|| {
            let mut req = HttpRequest::new("POST", "/api".to_string());
            req.body = black_box(armature_core::Bytes::from(vec![0u8; 1024]));
            black_box(req.body.len())
        })
    });

    // New way: set body via Bytes
    let bytes = Bytes::from(vec![0u8; 1024]);
    group.bench_function("bytes_assignment", |b| {
        b.iter(|| {
            let mut req = HttpRequest::new("POST", "/api".to_string());
            req.set_body_bytes(black_box(bytes.clone()));
            black_box(req.body_ref().len())
        })
    });

    // New way: constructor with Bytes
    group.bench_function("constructor_with_bytes", |b| {
        b.iter(|| {
            let req =
                HttpRequest::with_bytes_body("POST", "/api".to_string(), black_box(bytes.clone()));
            black_box(req.body_ref().len())
        })
    });

    group.finish();
}

// ============================================================================
// HttpResponse Body Handling Benchmarks
// ============================================================================

fn bench_http_response_body(c: &mut Criterion) {
    let mut group = c.benchmark_group("http_response_body");

    // Legacy: with_body using Vec<u8>
    group.bench_function("legacy_with_body", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_body(black_box(vec![0u8; 1024]));
            black_box(resp.body.len())
        })
    });

    // New: with_bytes_body using Bytes
    let bytes = Bytes::from(vec![0u8; 1024]);
    group.bench_function("with_bytes_body", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_bytes_body(black_box(bytes.clone()));
            black_box(resp.body_ref().len())
        })
    });

    // JSON response - most common case
    #[derive(serde::Serialize)]
    struct ApiResponse {
        success: bool,
        data: Vec<i32>,
    }

    let data = ApiResponse {
        success: true,
        data: vec![1, 2, 3, 4, 5],
    };

    group.bench_function("with_json", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_json(black_box(&data)).unwrap();
            black_box(resp.body_ref().len())
        })
    });

    // Static response
    group.bench_function("with_static_body", |b| {
        static BODY: &[u8] = b"Static response body content";
        b.iter(|| {
            let resp = HttpResponse::ok().with_static_body(black_box(BODY));
            black_box(resp.body_ref().len())
        })
    });

    group.finish();
}

// ============================================================================
// Hyper Body Passthrough Benchmarks
// ============================================================================

fn bench_hyper_passthrough(c: &mut Criterion) {
    let mut group = c.benchmark_group("hyper_passthrough");

    // Simulate the old pattern: Vec<u8> -> Bytes (copies)
    group.bench_function("legacy_vec_to_bytes", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_body(black_box(vec![0u8; 1024]));
            // Simulates: Full::new(bytes::Bytes::from(response.body))
            let body_bytes = resp.body;
            black_box(body_bytes.len())
        })
    });

    // New pattern: Bytes directly (zero-copy)
    let bytes = Bytes::from(vec![0u8; 1024]);
    group.bench_function("zero_copy_bytes", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_bytes_body(black_box(bytes.clone()));
            // Simulates: Full::new(response.into_body_bytes())
            let body_bytes = resp.into_body_bytes();
            black_box(body_bytes.len())
        })
    });

    // Full simulation: JSON response to Hyper body
    #[derive(serde::Serialize)]
    struct JsonResponse {
        message: String,
        code: u32,
    }

    let data = JsonResponse {
        message: "Success".to_string(),
        code: 200,
    };

    group.bench_function("json_to_hyper_body", |b| {
        b.iter(|| {
            let resp = HttpResponse::ok().with_json(black_box(&data)).unwrap();
            let body_bytes = resp.into_body_bytes();
            black_box(body_bytes.len())
        })
    });

    group.finish();
}

// ============================================================================
// Clone Performance Benchmarks
// ============================================================================

fn bench_body_cloning(c: &mut Criterion) {
    let mut group = c.benchmark_group("body_clone");

    // Vec<u8> clone - copies all data
    let vec_data = vec![0u8; 4096];
    group.bench_function("vec_clone_4kb", |b| {
        b.iter(|| {
            let cloned = black_box(&vec_data).clone();
            black_box(cloned.len())
        })
    });

    // Bytes clone - just ref count increment
    let bytes_data = Bytes::from(vec![0u8; 4096]);
    group.bench_function("bytes_clone_4kb", |b| {
        b.iter(|| {
            let cloned = black_box(&bytes_data).clone();
            black_box(cloned.len())
        })
    });

    // RequestBody clone
    let request_body = RequestBody::from_vec(vec![0u8; 4096]);
    group.bench_function("request_body_clone_4kb", |b| {
        b.iter(|| {
            let cloned = black_box(&request_body).clone();
            black_box(cloned.len())
        })
    });

    // ResponseBody clone
    let response_body = ResponseBody::from_vec(vec![0u8; 4096]);
    group.bench_function("response_body_clone_4kb", |b| {
        b.iter(|| {
            let cloned = black_box(&response_body).clone();
            black_box(cloned.len())
        })
    });

    group.finish();
}

criterion_group!(
    body_benches,
    bench_request_body_creation,
    bench_response_body_creation,
    bench_http_request_body,
    bench_http_response_body,
    bench_hyper_passthrough,
    bench_body_cloning,
);

criterion_main!(body_benches);
