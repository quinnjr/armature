//! Memory allocation benchmarks for detecting potential memory leaks
//!
//! These benchmarks measure allocation patterns and help identify memory issues.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use crossbeam::queue::ArrayQueue;
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;

// ============================================================================
// String Allocation Benchmarks
// ============================================================================

fn bench_string_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/string_allocations");

    // Small string (SSO eligible)
    group.bench_function("small_string_new", |b| {
        b.iter(|| {
            let s = String::from("hello");
            black_box(s)
        })
    });

    // Medium string
    group.bench_function("medium_string_new", |b| {
        b.iter(|| {
            let s = String::from("hello world, this is a medium length string");
            black_box(s)
        })
    });

    // Large string
    let large_content = "x".repeat(10000);
    group.bench_function("large_string_new", |b| {
        b.iter(|| {
            let s = large_content.clone();
            black_box(s)
        })
    });

    // String formatting (multiple allocations)
    group.bench_function("string_format", |b| {
        b.iter(|| {
            let s = format!("User {} logged in from {}", 12345, "192.168.1.1");
            black_box(s)
        })
    });

    // String concatenation
    group.bench_function("string_concat_push", |b| {
        b.iter(|| {
            let mut s = String::with_capacity(100);
            s.push_str("Hello, ");
            s.push_str("World!");
            black_box(s)
        })
    });

    // String join
    group.bench_function("string_join", |b| {
        let parts = vec!["one", "two", "three", "four", "five"];
        b.iter(|| {
            let s = parts.join(", ");
            black_box(s)
        })
    });

    group.finish();
}

// ============================================================================
// Vector Allocation Benchmarks
// ============================================================================

fn bench_vec_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/vec_allocations");

    // Vec without capacity
    group.bench_function("vec_push_no_capacity", |b| {
        b.iter(|| {
            let mut v = Vec::new();
            for i in 0..100 {
                v.push(i);
            }
            black_box(v)
        })
    });

    // Vec with capacity
    group.bench_function("vec_push_with_capacity", |b| {
        b.iter(|| {
            let mut v = Vec::with_capacity(100);
            for i in 0..100 {
                v.push(i);
            }
            black_box(v)
        })
    });

    // Vec from iterator
    group.bench_function("vec_from_iter", |b| {
        b.iter(|| {
            let v: Vec<i32> = (0..100).collect();
            black_box(v)
        })
    });

    // Vec clone
    let source: Vec<i32> = (0..1000).collect();
    group.bench_function("vec_clone_1000", |b| {
        b.iter(|| {
            let v = source.clone();
            black_box(v)
        })
    });

    // Nested vectors
    group.bench_function("vec_nested", |b| {
        b.iter(|| {
            let v: Vec<Vec<i32>> = (0..10).map(|i| (0..i * 10).collect()).collect();
            black_box(v)
        })
    });

    group.finish();
}

// ============================================================================
// HashMap Allocation Benchmarks
// ============================================================================

fn bench_hashmap_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/hashmap_allocations");

    // HashMap without capacity
    group.bench_function("hashmap_insert_no_capacity", |b| {
        b.iter(|| {
            let mut map = HashMap::new();
            for i in 0..100 {
                map.insert(i, format!("value_{}", i));
            }
            black_box(map)
        })
    });

    // HashMap with capacity
    group.bench_function("hashmap_insert_with_capacity", |b| {
        b.iter(|| {
            let mut map = HashMap::with_capacity(100);
            for i in 0..100 {
                map.insert(i, format!("value_{}", i));
            }
            black_box(map)
        })
    });

    // HashMap string keys
    group.bench_function("hashmap_string_keys", |b| {
        b.iter(|| {
            let mut map = HashMap::with_capacity(100);
            for i in 0..100 {
                map.insert(format!("key_{}", i), i);
            }
            black_box(map)
        })
    });

    // HashMap clone
    let source: HashMap<i32, String> = (0..100).map(|i| (i, format!("value_{}", i))).collect();
    group.bench_function("hashmap_clone_100", |b| {
        b.iter(|| {
            let map = source.clone();
            black_box(map)
        })
    });

    group.finish();
}

// ============================================================================
// Box and Arc Allocation Benchmarks
// ============================================================================

fn bench_smart_pointer_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/smart_pointers");

    // Box allocation
    group.bench_function("box_new", |b| {
        b.iter(|| {
            let b = Box::new(42i64);
            black_box(b)
        })
    });

    // Box with large struct
    #[derive(Clone)]
    struct LargeStruct {
        data: [u8; 1024],
    }

    group.bench_function("box_large_struct", |b| {
        b.iter(|| {
            let b = Box::new(LargeStruct { data: [0u8; 1024] });
            black_box(b)
        })
    });

    // Arc allocation
    group.bench_function("arc_new", |b| {
        b.iter(|| {
            let a = std::sync::Arc::new(42i64);
            black_box(a)
        })
    });

    // Arc clone (reference counting, no allocation)
    let arc = std::sync::Arc::new(42i64);
    group.bench_function("arc_clone", |b| {
        b.iter(|| {
            let a = arc.clone();
            black_box(a)
        })
    });

    // Rc allocation
    group.bench_function("rc_new", |b| {
        b.iter(|| {
            let r = std::rc::Rc::new(42i64);
            black_box(r)
        })
    });

    group.finish();
}

// ============================================================================
// Simulated Request/Response Allocation Benchmarks
// ============================================================================

#[derive(Clone)]
struct SimulatedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

#[derive(Clone)]
struct SimulatedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn bench_request_response_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/request_response");

    // Minimal request
    group.bench_function("request_minimal", |b| {
        b.iter(|| {
            let req = SimulatedRequest {
                method: "GET".to_string(),
                path: "/".to_string(),
                headers: vec![],
                body: None,
            };
            black_box(req)
        })
    });

    // Typical request with headers
    group.bench_function("request_with_headers", |b| {
        b.iter(|| {
            let req = SimulatedRequest {
                method: "POST".to_string(),
                path: "/api/users".to_string(),
                headers: vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Authorization".to_string(), "Bearer token123".to_string()),
                    ("Accept".to_string(), "application/json".to_string()),
                ],
                body: Some(br#"{"name":"test"}"#.to_vec()),
            };
            black_box(req)
        })
    });

    // Response with JSON body
    group.bench_function("response_json_small", |b| {
        b.iter(|| {
            let resp = SimulatedResponse {
                status: 200,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: br#"{"id":1,"name":"test"}"#.to_vec(),
            };
            black_box(resp)
        })
    });

    // Response with larger body
    let large_body = serde_json::to_vec(&serde_json::json!({
        "users": (0..100).map(|i| {
            serde_json::json!({
                "id": i,
                "name": format!("User {}", i),
                "email": format!("user{}@example.com", i)
            })
        }).collect::<Vec<_>>()
    }))
    .unwrap();

    group.bench_function("response_json_large", |b| {
        b.iter(|| {
            let resp = SimulatedResponse {
                status: 200,
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: large_body.clone(),
            };
            black_box(resp)
        })
    });

    group.finish();
}

// ============================================================================
// Memory Pool Simulation Benchmarks
// ============================================================================

/// Mutex-based object pool (baseline - has lock contention overhead)
struct MutexPool<T> {
    available: std::sync::Mutex<Vec<T>>,
}

impl<T: Default> MutexPool<T> {
    fn new(initial_size: usize) -> Self {
        let mut items = Vec::with_capacity(initial_size);
        for _ in 0..initial_size {
            items.push(T::default());
        }
        Self {
            available: std::sync::Mutex::new(items),
        }
    }

    fn get(&self) -> T {
        self.available
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(T::default)
    }

    fn put(&self, item: T) {
        self.available.lock().unwrap().push(item);
    }
}

/// Lock-free object pool using crossbeam's ArrayQueue (recommended)
struct LockFreePool<T> {
    available: Arc<ArrayQueue<T>>,
}

impl<T: Default> LockFreePool<T> {
    fn new(capacity: usize) -> Self {
        let queue = ArrayQueue::new(capacity);
        for _ in 0..capacity {
            let _ = queue.push(T::default());
        }
        Self {
            available: Arc::new(queue),
        }
    }

    fn get(&self) -> T {
        self.available.pop().unwrap_or_else(|| T::default())
    }

    fn put(&self, item: T) {
        // If full, just drop the item (bounded pool)
        let _ = self.available.push(item);
    }
}

fn bench_object_pool(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/object_pool");

    #[derive(Default, Clone)]
    struct PooledObject {
        data: Vec<u8>,
    }

    // Without pool (new allocation each time)
    group.bench_function("without_pool", |b| {
        b.iter(|| {
            let obj = PooledObject {
                data: vec![0u8; 1024],
            };
            black_box(obj)
        })
    });

    // With Mutex-based pool (has lock overhead)
    let mutex_pool: MutexPool<PooledObject> = MutexPool::new(100);
    group.bench_function("mutex_pool", |b| {
        b.iter(|| {
            let obj = mutex_pool.get();
            black_box(&obj);
            mutex_pool.put(obj);
        })
    });

    // With lock-free pool (recommended - no lock contention)
    let lockfree_pool: LockFreePool<PooledObject> = LockFreePool::new(100);
    group.bench_function("lockfree_pool", |b| {
        b.iter(|| {
            let obj = lockfree_pool.get();
            black_box(&obj);
            lockfree_pool.put(obj);
        })
    });

    group.finish();
}

// ============================================================================
// Leak Detection Pattern Benchmarks
// ============================================================================

fn bench_leak_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/leak_patterns");

    // Pattern: Growing cache without bounds
    group.bench_function("unbounded_cache_growth", |b| {
        b.iter(|| {
            let mut cache: HashMap<String, Vec<u8>> = HashMap::new();
            for i in 0..1000 {
                cache.insert(format!("key_{}", i), vec![0u8; 100]);
            }
            // Simulating cache that grows without eviction
            black_box(cache.len())
        })
    });

    // Pattern: Bounded cache with eviction
    group.bench_function("bounded_cache_with_eviction", |b| {
        b.iter(|| {
            let mut cache: HashMap<String, Vec<u8>> = HashMap::new();
            const MAX_SIZE: usize = 100;

            for i in 0..1000 {
                if cache.len() >= MAX_SIZE {
                    // Simple eviction: remove first key
                    if let Some(key) = cache.keys().next().cloned() {
                        cache.remove(&key);
                    }
                }
                cache.insert(format!("key_{}", i), vec![0u8; 100]);
            }
            black_box(cache.len())
        })
    });

    // Pattern: Circular reference prevention
    group.bench_function("weak_reference_pattern", |b| {
        use std::sync::{Arc, Weak};

        b.iter(|| {
            let strong = Arc::new(42);
            let weak: Weak<i32> = Arc::downgrade(&strong);

            // Weak reference doesn't prevent deallocation
            black_box(weak.upgrade())
        })
    });

    group.finish();
}

// ============================================================================
// Varying Size Benchmarks
// ============================================================================

fn bench_allocation_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/allocation_sizes");

    for size in [64, 256, 1024, 4096, 16384, 65536].iter() {
        group.bench_with_input(BenchmarkId::new("vec_alloc", size), size, |b, &size| {
            b.iter(|| {
                let v: Vec<u8> = vec![0u8; size];
                black_box(v)
            })
        });
    }

    for size in [64, 256, 1024, 4096, 16384, 65536].iter() {
        group.bench_with_input(BenchmarkId::new("string_alloc", size), size, |b, &size| {
            b.iter(|| {
                let s = "x".repeat(size);
                black_box(s)
            })
        });
    }

    group.finish();
}

// ============================================================================
// Drop Timing Benchmarks
// ============================================================================

fn bench_drop_timing(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/drop_timing");

    // Small vec drop
    group.bench_function("drop_small_vec", |b| {
        b.iter_batched(
            || vec![0u8; 100],
            |v| drop(black_box(v)),
            criterion::BatchSize::SmallInput,
        )
    });

    // Large vec drop
    group.bench_function("drop_large_vec", |b| {
        b.iter_batched(
            || vec![0u8; 100_000],
            |v| drop(black_box(v)),
            criterion::BatchSize::SmallInput,
        )
    });

    // Nested structure drop
    group.bench_function("drop_nested_structure", |b| {
        b.iter_batched(
            || {
                let v: Vec<Vec<String>> = (0..100)
                    .map(|i| (0..10).map(|j| format!("item_{}_{}", i, j)).collect())
                    .collect();
                v
            },
            |v| drop(black_box(v)),
            criterion::BatchSize::SmallInput,
        )
    });

    // HashMap drop
    group.bench_function("drop_hashmap", |b| {
        b.iter_batched(
            || {
                let m: HashMap<String, Vec<u8>> = (0..1000)
                    .map(|i| (format!("key_{}", i), vec![0u8; 100]))
                    .collect();
                m
            },
            |m| drop(black_box(m)),
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(
    memory_benches,
    bench_string_allocations,
    bench_vec_allocations,
    bench_hashmap_allocations,
    bench_smart_pointer_allocations,
    bench_request_response_allocations,
    bench_object_pool,
    bench_leak_patterns,
    bench_allocation_sizes,
    bench_drop_timing,
);

criterion_main!(memory_benches);
