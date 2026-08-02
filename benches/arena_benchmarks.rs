//! Arena Allocator Benchmarks
//!
//! Compares arena allocation vs standard heap allocation for
//! per-request data structures.
//!
//! Run with: cargo bench --bench arena_benchmarks

use armature_core::HttpRequest;
use armature_core::arena::{
    ArenaMap, ArenaRequest, ArenaStr, ArenaVec, RequestScope, reset_arena, with_arena,
};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::collections::HashMap;
use std::hint::black_box;

// ============================================================================
// String Allocation Benchmarks
// ============================================================================

fn bench_string_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_alloc");

    // Standard heap allocation
    group.bench_function("heap/single", |b| {
        b.iter(|| {
            let s = black_box("hello world").to_string();
            black_box(s)
        })
    });

    // Arena allocation
    group.bench_function("arena/single", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let s = ArenaStr::from_str(arena, black_box("hello world"));
                let _ = black_box(s.len()); // Don't return arena data
            });
            reset_arena();
        })
    });

    // Multiple strings - heap
    group.bench_function("heap/multiple_10", |b| {
        b.iter(|| {
            let strings: Vec<String> = (0..10).map(|i| format!("string number {}", i)).collect();
            black_box(strings)
        })
    });

    // Multiple strings - arena
    group.bench_function("arena/multiple_10", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let mut strings = ArenaVec::with_capacity_in(10, arena);
                for i in 0..10 {
                    strings.push(ArenaStr::from_str(arena, &format!("string number {}", i)));
                }
                let _ = black_box(strings.len()); // Don't return arena data
            });
            reset_arena();
        })
    });

    group.finish();
}

// ============================================================================
// HashMap vs ArenaMap Benchmarks
// ============================================================================

fn bench_map_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("map_alloc");

    // Header map simulation - standard HashMap
    group.bench_function("hashmap/headers_16", |b| {
        b.iter(|| {
            let mut map = HashMap::with_capacity(16);
            map.insert("Content-Type".to_string(), "application/json".to_string());
            map.insert("Accept".to_string(), "application/json".to_string());
            map.insert(
                "Authorization".to_string(),
                "Bearer token123456".to_string(),
            );
            map.insert("User-Agent".to_string(), "Mozilla/5.0".to_string());
            map.insert("Accept-Language".to_string(), "en-US,en;q=0.9".to_string());
            map.insert(
                "Accept-Encoding".to_string(),
                "gzip, deflate, br".to_string(),
            );
            map.insert("Connection".to_string(), "keep-alive".to_string());
            map.insert("Host".to_string(), "api.example.com".to_string());
            black_box(map)
        })
    });

    // Header map simulation - ArenaMap
    group.bench_function("arenamap/headers_16", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let mut map = ArenaMap::with_capacity_in(arena, 16);
                map.insert(
                    ArenaStr::from_str(arena, "Content-Type"),
                    ArenaStr::from_str(arena, "application/json"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Accept"),
                    ArenaStr::from_str(arena, "application/json"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Authorization"),
                    ArenaStr::from_str(arena, "Bearer token123456"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "User-Agent"),
                    ArenaStr::from_str(arena, "Mozilla/5.0"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Accept-Language"),
                    ArenaStr::from_str(arena, "en-US,en;q=0.9"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Accept-Encoding"),
                    ArenaStr::from_str(arena, "gzip, deflate, br"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Connection"),
                    ArenaStr::from_str(arena, "keep-alive"),
                );
                map.insert(
                    ArenaStr::from_str(arena, "Host"),
                    ArenaStr::from_str(arena, "api.example.com"),
                );
                let _ = black_box(map.len()); // Don't return arena data
            });
            reset_arena();
        })
    });

    group.finish();
}

// ============================================================================
// Request Creation Benchmarks
// ============================================================================

fn bench_request_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_create");

    // Standard HttpRequest with typical data
    group.bench_function("http_request/typical", |b| {
        b.iter(|| {
            let mut req = HttpRequest::new("POST", "/api/v1/users/123".to_string());
            req.headers
                .insert("Content-Type", "application/json".to_string());
            req.headers
                .insert("Authorization", "Bearer token123".to_string());
            req.headers.insert("Accept", "application/json".to_string());
            req.headers
                .insert("User-Agent", "TestClient/1.0".to_string());
            req.path_params.insert("id".to_string(), "123".to_string());
            req.query_params
                .insert("include".to_string(), "profile".to_string());
            req.query_params
                .insert("fields".to_string(), "name,email".to_string());
            black_box(req)
        })
    });

    // ArenaRequest with typical data
    group.bench_function("arena_request/typical", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let mut req = ArenaRequest::new(arena, "POST", "/api/v1/users/123");
                req.add_header(arena, "Content-Type", "application/json");
                req.add_header(arena, "Authorization", "Bearer token123");
                req.add_header(arena, "Accept", "application/json");
                req.add_header(arena, "User-Agent", "TestClient/1.0");
                req.add_path_param(arena, "id", "123");
                req.add_query_param(arena, "include", "profile");
                req.add_query_param(arena, "fields", "name,email");
                let _ = black_box(req.method.len()); // Don't return arena data
            });
            reset_arena();
        })
    });

    // HttpRequest with many headers
    group.bench_function("http_request/many_headers", |b| {
        b.iter(|| {
            let mut req = HttpRequest::new("GET", "/api/data".to_string());
            for i in 0..20 {
                req.headers.insert(
                    format!("X-Custom-Header-{}", i),
                    format!("value-{}-with-some-content", i),
                );
            }
            black_box(req)
        })
    });

    // ArenaRequest with many headers
    group.bench_function("arena_request/many_headers", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let mut req = ArenaRequest::new(arena, "GET", "/api/data");
                for i in 0..20 {
                    req.add_header(
                        arena,
                        &format!("X-Custom-Header-{}", i),
                        &format!("value-{}-with-some-content", i),
                    );
                }
                let _ = black_box(req.headers.len()); // Don't return arena data
            });
            reset_arena();
        })
    });

    group.finish();
}

// ============================================================================
// Full Request Lifecycle Simulation
// ============================================================================

fn bench_request_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("request_lifecycle");

    // Simulate full request processing with standard allocation
    group.throughput(Throughput::Elements(1));
    group.bench_function("heap/full_cycle", |b| {
        b.iter(|| {
            // Create request
            let mut req = HttpRequest::new("POST", "/api/users".to_string());
            req.headers
                .insert("Content-Type", "application/json".to_string());
            req.headers
                .insert("Authorization", "Bearer xyz".to_string());
            req.query_params.insert("page".to_string(), "1".to_string());
            req.body = b"{\"name\":\"John\"}".to_vec();

            // "Process" request
            let _method = req.method.clone();
            let _path = req.path.clone();
            let _ct = req.headers.get("Content-Type").map(str::to_owned);

            // Request dropped here - multiple deallocations
            black_box(req)
        })
    });

    // Simulate full request processing with arena allocation
    group.bench_function("arena/full_cycle", |b| {
        b.iter(|| {
            let _scope = RequestScope::new();
            with_arena(|arena| {
                // Create request
                let body = b"{\"name\":\"John\"}";
                let mut req = ArenaRequest::with_body(arena, "POST", "/api/users", body);
                req.add_header(arena, "Content-Type", "application/json");
                req.add_header(arena, "Authorization", "Bearer xyz");
                req.add_query_param(arena, "page", "1");

                // "Process" request
                let method = req.method.as_str();
                let path = req.path.as_str();
                let ct = req.header("Content-Type");

                let _ = black_box((method.len(), path.len(), ct.map(|s| s.len())));
            })
            // Scope dropped here - single bulk deallocation
        })
    });

    // High-volume simulation: process many requests
    group.throughput(Throughput::Elements(100));
    group.bench_function("heap/100_requests", |b| {
        b.iter(|| {
            for i in 0..100 {
                let mut req = HttpRequest::new("GET", format!("/api/item/{}", i));
                req.headers.insert("Accept", "application/json".to_string());
                req.query_params.insert("v".to_string(), "1".to_string());
                black_box(&req);
            }
        })
    });

    group.bench_function("arena/100_requests", |b| {
        b.iter(|| {
            for i in 0..100 {
                let _scope = RequestScope::new();
                with_arena(|arena| {
                    let mut req = ArenaRequest::new(arena, "GET", &format!("/api/item/{}", i));
                    req.add_header(arena, "Accept", "application/json");
                    req.add_query_param(arena, "v", "1");
                    let _ = black_box(req.method.len());
                });
            }
        })
    });

    group.finish();
}

// ============================================================================
// Memory Allocation Pattern Comparison
// ============================================================================

fn bench_allocation_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc_patterns");

    // Many small allocations - heap
    group.bench_function("heap/many_small", |b| {
        b.iter(|| {
            let mut items = Vec::with_capacity(100);
            for i in 0..100 {
                items.push(format!("item_{}", i));
            }
            black_box(items)
        })
    });

    // Many small allocations - arena
    group.bench_function("arena/many_small", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let mut items = ArenaVec::with_capacity_in(100, arena);
                for i in 0..100 {
                    items.push(ArenaStr::from_str(arena, &format!("item_{}", i)));
                }
                let _ = black_box(items.len());
            });
            reset_arena();
        })
    });

    // Mixed size allocations - heap
    group.bench_function("heap/mixed_sizes", |b| {
        b.iter(|| {
            let small = "small".to_string();
            let medium = "medium string with more content here".to_string();
            let large = "a".repeat(1000);
            black_box((small, medium, large))
        })
    });

    // Mixed size allocations - arena
    group.bench_function("arena/mixed_sizes", |b| {
        b.iter(|| {
            with_arena(|arena| {
                let small = ArenaStr::from_str(arena, "small");
                let medium = ArenaStr::from_str(arena, "medium string with more content here");
                let large = ArenaStr::from_str(arena, &"a".repeat(1000));
                let _ = black_box((small.len(), medium.len(), large.len()));
            });
            reset_arena();
        })
    });

    group.finish();
}

criterion_group!(
    arena_benches,
    bench_string_allocation,
    bench_map_allocation,
    bench_request_creation,
    bench_request_lifecycle,
    bench_allocation_patterns,
);

criterion_main!(arena_benches);
