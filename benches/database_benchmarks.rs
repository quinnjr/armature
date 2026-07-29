//! Database-Pattern Benchmarks (Mock Pool — No Real Database I/O)
//!
//! Benchmarks the *shape* of database access patterns following TechEmpower
//! conventions, but against a `MockPool`/`AsyncMockPool` that reads from an
//! in-memory `Vec<World>` — there is no real database driver, connection, or
//! I/O involved anywhere in this file. `AsyncMockPool` approximates network
//! latency with `tokio::time::sleep`, so its numbers reflect Rust
//! in-memory-collection access plus Tokio timer/scheduling overhead, not
//! actual database round-trip cost (e.g. Postgres/sqlx over a real socket).
//! Use these to compare the *overhead of the access pattern itself*
//! (single query vs. N queries vs. fortunes vs. updates), not to estimate
//! real-world database latency or throughput.
//!
//! - **Single Query**: Fetch one row by ID
//! - **Multiple Queries**: Fetch N rows with N individual queries
//! - **Fortunes**: Fetch all rows, add one, sort, render HTML
//! - **Updates**: Fetch N rows, modify, update individually

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

// ============================================================================
// Mock Database Types
// ============================================================================

/// World row from TechEmpower benchmark
#[derive(Debug, Clone)]
pub struct World {
    pub id: i32,
    pub random_number: i32,
}

/// Fortune row from TechEmpower benchmark
#[derive(Debug, Clone)]
pub struct Fortune {
    pub id: i32,
    pub message: String,
}

/// Mock database connection pool
pub struct MockPool {
    worlds: Vec<World>,
    fortunes: Vec<Fortune>,
    query_count: AtomicU64,
}

impl MockPool {
    pub fn new() -> Self {
        // Pre-populate with 10000 worlds (TechEmpower spec)
        let worlds: Vec<World> = (1..=10000)
            .map(|id| World {
                id,
                random_number: fastrand::i32(1..=10000),
            })
            .collect();

        // Pre-populate fortunes
        let fortunes = vec![
            Fortune { id: 1, message: "fortune: No such file or directory".to_string() },
            Fortune { id: 2, message: "A computer scientist is someone who fixes things that aren't broken.".to_string() },
            Fortune { id: 3, message: "After enough decimal places, nobody gives a damn.".to_string() },
            Fortune { id: 4, message: "A bad random number generator: 1, 1, 1, 1, 1, 4.33e+67, 1, 1, 1".to_string() },
            Fortune { id: 5, message: "A computer program does what you tell it to do, not what you want it to do.".to_string() },
            Fortune { id: 6, message: "Emstronment: The conditions that affect the way an environment affects you.".to_string() },
            Fortune { id: 7, message: "Features should be discovered, not documented.".to_string() },
            Fortune { id: 8, message: "Frisbeetarianism: The belief that, when you die, your soul goes up on the roof and gets stuck.".to_string() },
            Fortune { id: 9, message: "If the code and the comments disagree, then both are probably wrong.".to_string() },
            Fortune { id: 10, message: "If you think nobody cares, try missing a couple of payments.".to_string() },
            Fortune { id: 11, message: "May you live in interesting times.".to_string() },
            Fortune { id: 12, message: "The computer revolution is over. The computers won.".to_string() },
        ];

        Self {
            worlds,
            fortunes,
            query_count: AtomicU64::new(0),
        }
    }

    /// Single query - fetch one world by ID
    #[inline]
    pub fn get_world(&self, id: i32) -> Option<World> {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        let idx = (id - 1) as usize;
        self.worlds.get(idx).cloned()
    }

    /// Multiple queries - fetch N worlds
    #[inline]
    pub fn get_worlds(&self, ids: &[i32]) -> Vec<World> {
        ids.iter().filter_map(|&id| self.get_world(id)).collect()
    }

    /// Get all fortunes
    #[inline]
    pub fn get_fortunes(&self) -> Vec<Fortune> {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        self.fortunes.clone()
    }

    /// Update a world's random number
    #[inline]
    pub fn update_world(&self, id: i32, _random_number: i32) -> bool {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        let idx = (id - 1) as usize;
        idx < self.worlds.len()
    }

    /// Batch update worlds
    #[inline]
    pub fn update_worlds(&self, updates: &[(i32, i32)]) -> usize {
        updates
            .iter()
            .filter(|(id, rn)| self.update_world(*id, *rn))
            .count()
    }

    pub fn query_count(&self) -> u64 {
        self.query_count.load(Ordering::Relaxed)
    }

    pub fn reset_count(&self) {
        self.query_count.store(0, Ordering::Relaxed);
    }
}

impl Default for MockPool {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Async Mock Pool (simulates network latency)
// ============================================================================

/// Mock async pool that approximates network latency with `tokio::time::sleep`.
/// Still backed by the in-memory `MockPool` — no real database connection,
/// driver, or I/O is involved; this only measures async scheduling overhead
/// layered on top of the in-memory access.
pub struct AsyncMockPool {
    inner: MockPool,
    latency_us: u64,
}

impl AsyncMockPool {
    pub fn new(latency_us: u64) -> Self {
        Self {
            inner: MockPool::new(),
            latency_us,
        }
    }

    /// Simulate network latency
    #[inline]
    async fn simulate_latency(&self) {
        if self.latency_us > 0 {
            tokio::time::sleep(Duration::from_micros(self.latency_us)).await;
        }
    }

    pub async fn get_world(&self, id: i32) -> Option<World> {
        self.simulate_latency().await;
        self.inner.get_world(id)
    }

    pub async fn get_worlds(&self, ids: &[i32]) -> Vec<World> {
        // Sequential queries (TechEmpower spec)
        let mut results = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(world) = self.get_world(id).await {
                results.push(world);
            }
        }
        results
    }

    pub async fn get_fortunes(&self) -> Vec<Fortune> {
        self.simulate_latency().await;
        self.inner.get_fortunes()
    }

    pub async fn update_world(&self, id: i32, random_number: i32) -> bool {
        self.simulate_latency().await;
        self.inner.update_world(id, random_number)
    }

    pub fn query_count(&self) -> u64 {
        self.inner.query_count()
    }
}

// ============================================================================
// Benchmarks
// ============================================================================

fn single_query_benchmark(c: &mut Criterion) {
    let pool = MockPool::new();

    let mut group = c.benchmark_group("db/single_query");
    group.throughput(Throughput::Elements(1));

    // Random ID lookup
    group.bench_function("random_id", |b| {
        b.iter(|| {
            let id = fastrand::i32(1..=10000);
            black_box(pool.get_world(id))
        });
    });

    // Sequential ID lookup (cache friendly)
    group.bench_function("sequential_id", |b| {
        let mut id = 1;
        b.iter(|| {
            let result = pool.get_world(id);
            id = if id >= 10000 { 1 } else { id + 1 };
            black_box(result)
        });
    });

    // First row (best case)
    group.bench_function("first_row", |b| {
        b.iter(|| black_box(pool.get_world(1)));
    });

    // Last row (potential worst case)
    group.bench_function("last_row", |b| {
        b.iter(|| black_box(pool.get_world(10000)));
    });

    group.finish();
}

fn multiple_queries_benchmark(c: &mut Criterion) {
    let pool = MockPool::new();

    let mut group = c.benchmark_group("db/multiple_queries");

    for count in [1, 5, 10, 15, 20].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        group.bench_with_input(BenchmarkId::new("random", count), count, |b, &count| {
            b.iter(|| {
                let ids: Vec<i32> = (0..count).map(|_| fastrand::i32(1..=10000)).collect();
                black_box(pool.get_worlds(&ids))
            });
        });

        group.bench_with_input(BenchmarkId::new("sequential", count), count, |b, &count| {
            b.iter(|| {
                let ids: Vec<i32> = (1..=count).collect();
                black_box(pool.get_worlds(&ids))
            });
        });
    }

    group.finish();
}

fn fortunes_benchmark(c: &mut Criterion) {
    let pool = MockPool::new();

    let mut group = c.benchmark_group("db/fortunes");

    // Fetch fortunes
    group.bench_function("fetch", |b| {
        b.iter(|| black_box(pool.get_fortunes()));
    });

    // Fetch + sort
    group.bench_function("fetch_and_sort", |b| {
        b.iter(|| {
            let mut fortunes = pool.get_fortunes();
            // Add the additional fortune (TechEmpower spec)
            fortunes.push(Fortune {
                id: 0,
                message: "Additional fortune added at request time.".to_string(),
            });
            fortunes.sort_by(|a, b| a.message.cmp(&b.message));
            black_box(fortunes)
        });
    });

    // Fetch + sort + render HTML
    group.bench_function("full_render", |b| {
        b.iter(|| {
            let mut fortunes = pool.get_fortunes();
            fortunes.push(Fortune {
                id: 0,
                message: "Additional fortune added at request time.".to_string(),
            });
            fortunes.sort_by(|a, b| a.message.cmp(&b.message));

            // Simple HTML rendering
            let mut html = String::with_capacity(4096);
            html.push_str("<!DOCTYPE html><html><head><title>Fortunes</title></head><body><table>");
            html.push_str("<tr><th>id</th><th>message</th></tr>");
            for f in &fortunes {
                html.push_str("<tr><td>");
                html.push_str(&f.id.to_string());
                html.push_str("</td><td>");
                // HTML escape the message
                for c in f.message.chars() {
                    match c {
                        '<' => html.push_str("&lt;"),
                        '>' => html.push_str("&gt;"),
                        '&' => html.push_str("&amp;"),
                        '"' => html.push_str("&quot;"),
                        '\'' => html.push_str("&#x27;"),
                        _ => html.push(c),
                    }
                }
                html.push_str("</td></tr>");
            }
            html.push_str("</table></body></html>");
            black_box(html)
        });
    });

    group.finish();
}

fn updates_benchmark(c: &mut Criterion) {
    let pool = MockPool::new();

    let mut group = c.benchmark_group("db/updates");

    for count in [1, 5, 10, 15, 20].iter() {
        group.throughput(Throughput::Elements(*count as u64));

        // Single updates (sequential)
        group.bench_with_input(BenchmarkId::new("sequential", count), count, |b, &count| {
            b.iter(|| {
                for id in 1..=count {
                    let new_random = fastrand::i32(1..=10000);
                    pool.update_world(id, new_random);
                }
            });
        });

        // Batch update
        group.bench_with_input(BenchmarkId::new("batch", count), count, |b, &count| {
            b.iter(|| {
                let updates: Vec<(i32, i32)> = (1..=count)
                    .map(|id| (id, fastrand::i32(1..=10000)))
                    .collect();
                black_box(pool.update_worlds(&updates))
            });
        });
    }

    group.finish();
}

fn async_benchmark(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("db/async");

    // Zero latency (best case)
    let pool_0 = AsyncMockPool::new(0);
    group.bench_function("single_query/0us_latency", |b| {
        b.to_async(&rt).iter(|| async {
            let id = fastrand::i32(1..=10000);
            black_box(pool_0.get_world(id).await)
        });
    });

    // 10us latency (fast local DB)
    let pool_10 = AsyncMockPool::new(10);
    group.bench_function("single_query/10us_latency", |b| {
        b.to_async(&rt).iter(|| async {
            let id = fastrand::i32(1..=10000);
            black_box(pool_10.get_world(id).await)
        });
    });

    // 100us latency (typical local DB)
    let pool_100 = AsyncMockPool::new(100);
    group.bench_function("single_query/100us_latency", |b| {
        b.to_async(&rt).iter(|| async {
            let id = fastrand::i32(1..=10000);
            black_box(pool_100.get_world(id).await)
        });
    });

    // Multiple queries with latency
    for count in [5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("multiple_queries/100us", count),
            count,
            |b, &count| {
                b.to_async(&rt).iter(|| async {
                    let ids: Vec<i32> = (0..count).map(|_| fastrand::i32(1..=10000)).collect();
                    black_box(pool_100.get_worlds(&ids).await)
                });
            },
        );
    }

    group.finish();
}

fn json_serialization_benchmark(c: &mut Criterion) {
    let pool = MockPool::new();

    let mut group = c.benchmark_group("db/json");

    // Single world to JSON
    group.bench_function("single_world", |b| {
        let world = pool.get_world(1).unwrap();
        b.iter(|| {
            // Manual JSON for speed
            let json = format!(
                r#"{{"id":{},"randomNumber":{}}}"#,
                world.id, world.random_number
            );
            black_box(json)
        });
    });

    // Multiple worlds to JSON array
    for count in [1, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("multiple_worlds", count),
            count,
            |b, &count| {
                let ids: Vec<i32> = (1..=count).collect();
                let worlds = pool.get_worlds(&ids);
                b.iter(|| {
                    let mut json = String::with_capacity(count as usize * 40);
                    json.push('[');
                    for (i, world) in worlds.iter().enumerate() {
                        if i > 0 {
                            json.push(',');
                        }
                        json.push_str(&format!(
                            r#"{{"id":{},"randomNumber":{}}}"#,
                            world.id, world.random_number
                        ));
                    }
                    json.push(']');
                    black_box(json)
                });
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .measurement_time(Duration::from_secs(5));
    targets =
        single_query_benchmark,
        multiple_queries_benchmark,
        fortunes_benchmark,
        updates_benchmark,
        async_benchmark,
        json_serialization_benchmark
}

criterion_main!(benches);
