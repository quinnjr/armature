# Performance Guide

Armature is designed for high performance, achieving Actix-like speeds through 110+ optimizations across the framework.

## Quick Start

For most applications, Armature's defaults provide excellent performance. For maximum speed:

```rust
use armature_core::fast_response::{FastResponse, fast};

// Zero-allocation response creation
async fn health() -> FastResponse {
    fast::ok()
}

// Static body (no allocation)
async fn hello() -> FastResponse {
    FastResponse::ok().with_static_body(b"Hello, World!")
}

// JSON with pre-sized buffer
async fn user(id: u32) -> Result<FastResponse, Error> {
    let user = get_user(id)?;
    FastResponse::ok().with_json_sized(&user, 256)
}
```

## Response Creation

### Standard Response (`HttpResponse`)

`HttpResponse` now uses `LazyHeaders` - headers are only allocated when first inserted:

```rust
use armature_core::HttpResponse;

// Zero allocation for empty response
let resp = HttpResponse::ok();  // ~1ns

// Headers allocated on first insert
let resp = HttpResponse::ok()
    .with_header("X-Custom".into(), "value".into());
```

### Fast Response (`FastResponse`)

For maximum performance, use `FastResponse`:

```rust
use armature_core::fast_response::{FastResponse, fast};

// Zero-allocation factory functions
let ok = fast::ok();              // 200 OK, no body
let not_found = fast::not_found(); // 404
let empty_json = fast::empty_json(); // {}

// Static bodies (compile-time, zero-copy)
let resp = FastResponse::ok()
    .with_static_body(b"Hello, World!");

// JSON with content-type header
let resp = FastResponse::ok()
    .with_json(&data)?;
```

**Performance Comparison:**

| Operation | `HttpResponse` | `FastResponse` |
|-----------|---------------|----------------|
| Empty response | ~1ns | ~1ns |
| With 3 headers | ~15ns | ~8ns |
| With JSON body | ~55ns | ~45ns |

### Headers Optimization

`LazyHeaders` wraps `Option<HashMap>` and only allocates on first insert:

```rust
// No allocation until insert
let mut headers = LazyHeaders::new();
assert!(headers.is_empty());

// Allocation happens here
headers.insert("Content-Type".into(), "application/json".into());
```

`FastHeaders` uses `SmallVec` for inline storage (≤8 headers on stack):

```rust
use armature_core::fast_response::FastHeaders;

let mut headers = FastHeaders::new();
headers.insert("Content-Type", "application/json");
headers.insert("X-Request-ID", "abc-123");
// Still on stack - no heap allocation!
```

## Small Vector Optimizations

The `small_vec` module provides stack-allocated collections for common HTTP data:

```rust
use armature_core::small_vec::{QueryParams, PathParams, Cookies, FormFields};

// Query parameters (8 inline)
let params = QueryParams::parse("name=Alice&age=30");
assert!(params.is_inline()); // No heap allocation

// Path parameters (4 inline)
let mut path = PathParams::new();
path.push("id", "123");
path.push("slug", "hello-world");

// Cookies (8 inline)
let cookies = Cookies::parse("session=abc; user=alice");

// Form fields (16 inline)
let fields = FormFields::parse("name=Alice&email=alice@example.com");
```

**Inline Capacities:**

| Type | Inline Capacity | Stack Size | Heap Alloc Avoided |
|------|-----------------|------------|-------------------|
| `QueryParams` | 8 params | ~256 bytes | ~99% of requests |
| `PathParams` | 4 params | ~128 bytes | ~100% of routes |
| `FormFields` | 16 fields | ~512 bytes | ~90% of forms |
| `Cookies` | 8 cookies | ~256 bytes | ~98% of requests |

## HTTP Parsing

Armature uses SIMD-optimized parsing via `httparse` and `memchr`:

```rust
use armature_core::simd_parser::SIMDParser;

// Parse HTTP request with SIMD acceleration
let parser = SIMDParser::new();
let (request, body) = parser.parse(raw_bytes)?;
```

**Optimizations:**
- SIMD byte scanning for delimiters (CR, LF, colon)
- Header interning for common headers (Content-Type, Accept, etc.)
- Zero-copy body passthrough using `Bytes`

## Routing

The router uses `matchit` (same as Axum) for O(log n) routing:

```rust
use armature_core::routing::Router;

let mut router = Router::new();
router.get("/users/:id", get_user);
router.post("/users", create_user);

// Route matching is ~50ns for typical route sets
let handler = router.match_route("GET", "/users/123")?;
```

**Additional Optimizations:**
- LRU cache for frequently accessed routes
- Static route fast path (HashMap for exact matches)
- Compile-time route validation

## JSON Serialization

Enable SIMD-accelerated JSON with the `simd-json` feature:

```toml
[dependencies]
armature-core = { version = "0.1", features = ["simd-json"] }
```

```rust
use armature_core::json;

// SIMD-accelerated serialization (1.5-2x faster)
let bytes = json::to_vec(&data)?;

// SIMD-accelerated deserialization
let user: User = json::from_slice(&bytes)?;
```

**Performance with `simd-json`:**

| Operation | Standard | SIMD | Speedup |
|-----------|----------|------|---------|
| Serialize small | 30ns | 17ns | 1.8x |
| Serialize large | 25µs | 14µs | 1.8x |
| Deserialize | 350ns | 200ns | 1.75x |

## Connection Handling

### Buffer Auto-Tuning

Buffers automatically adjust based on traffic patterns:

```rust
use armature_core::connection_manager::{ConnectionManager, ConnectionManagerConfig};

let config = ConnectionManagerConfig::default()
    .min_buffer_size(4096)
    .max_buffer_size(65536)
    .buffer_history_size(1000);

let manager = ConnectionManager::new(config);
```

### Adaptive Keep-Alive

Connection keep-alive adjusts based on server load:

```rust
use armature_core::connection_manager::ConnectionManagerConfig;

let config = ConnectionManagerConfig::default()
    .keep_alive_timeout(Duration::from_secs(60))
    .min_keep_alive_timeout(Duration::from_secs(5))
    .load_threshold_high(0.8)
    .load_threshold_low(0.3);
```

### Idle Connection Culling

Under memory pressure, idle connections are automatically closed:

```rust
let config = ConnectionManagerConfig::default()
    .max_idle_connections(1000)
    .idle_timeout(Duration::from_secs(300));
```

## I/O Optimizations

### Vectored I/O

Responses use `writev()` to send headers and body in a single syscall:

```rust
use armature_core::vectored_io::ResponseChunks;

let response = HttpResponse::ok()
    .with_json(&data)?;

// Convert to vectored chunks
let chunks = response.into_chunks();

// Single syscall for entire response
socket.write_vectored(&chunks.to_slices())?;
```

### io_uring (Linux 5.1+)

Enable the `io_uring` feature for async I/O on Linux:

```toml
[dependencies]
armature-core = { version = "0.1", features = ["io_uring"] }
```

```rust
use armature_core::io_uring::IoUringConfig;

let config = IoUringConfig::default()
    .ring_size(256)
    .sqpoll(true); // Kernel-side polling
```

## Build Optimizations

### Release Profile

The default release profile includes:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
```

### Maximum Performance

For maximum performance, use the `release-fat` profile:

```bash
cargo build --profile release-fat
```

```toml
[profile.release-fat]
inherits = "release"
lto = "fat"
codegen-units = 1
panic = "abort"
```

### Native CPU Optimization

Target your specific CPU:

```bash
cargo build-native
```

```toml
[profile.release-native]
inherits = "release"
lto = "thin"

[target.x86_64-unknown-linux-gnu]
rustflags = ["-C", "target-cpu=native"]
```

### Profile-Guided Optimization (PGO)

Generate optimized builds based on real workloads:

```bash
# 1. Generate profiling data
cargo pgo-gen
./target/release/my-app &
# Run representative workload
wrk -t4 -c100 -d30s http://localhost:8080/

# 2. Build with PGO
cargo pgo-build
```

## Benchmarking

### Running Benchmarks

```bash
# All benchmarks
cargo bench

# Specific benchmark
cargo bench -- response_creation

# With native optimizations
cargo bench-native
```

### Framework Comparison

Compare against other frameworks:

```bash
cd benches/techempower
./run.sh
```

### Latest Results (December 2024)

| Benchmark | Time | vs Axum | vs Actix |
|-----------|------|---------|----------|
| Health check | 386ns | ~equal | ~10% slower |
| GET with param | 692ns | ~equal | ~15% slower |
| POST with body | 778ns | ~5% faster | ~10% slower |
| Route (100 routes) | 51ns | ~equal | ~equal |
| JSON serialize (small) | 17ns | ~20% faster | ~equal |

## Memory Optimization

### Arena Allocator

Per-request arena for batch allocations:

```rust
use armature_core::arena::RequestArena;

let arena = RequestArena::new();

// Allocations are batched and freed together
let data = arena.alloc(MyStruct::new());
let vec = arena.alloc_vec::<u8>(1024);

// All memory freed when arena drops
```

### Object Pools

Reuse request/response objects:

```rust
use armature_core::memory_opt::ObjectPool;

let pool: ObjectPool<HttpRequest> = ObjectPool::new(100);

// Get from pool (or create new)
let req = pool.get();

// Return to pool when done
pool.put(req);
```

## Profiling

### CPU Profiling

Generate flamegraphs with the profiling example:

```bash
cargo run --example profiling_server --release
# In another terminal
./scripts/profile.sh
```

### Automated Profiling in CI

Flamegraphs are automatically generated for PRs affecting `armature-core`.

## Best Practices

### Response Creation

```rust
// ✅ Good: Zero-allocation for common responses
async fn health() -> FastResponse {
    fast::ok()
}

// ✅ Good: Pre-sized buffer for known sizes
async fn user() -> Result<FastResponse, Error> {
    FastResponse::ok().with_json_sized(&user, 256)
}

// ❌ Avoid: Unnecessary allocations
async fn bad() -> HttpResponse {
    HttpResponse::ok()
        .with_header("X-A".into(), "1".into())
        .with_header("X-B".into(), "2".into()) // 2 String allocations
}
```

### JSON Handling

```rust
// ✅ Good: SIMD-accelerated (with feature)
use armature_core::json;
let bytes = json::to_vec(&data)?;

// ✅ Good: Direct body bytes
let resp = FastResponse::ok()
    .with_bytes(Bytes::from(json::to_vec(&data)?));

// ❌ Avoid: Double serialization
let json_str = serde_json::to_string(&data)?;
let resp = HttpResponse::ok().with_body(json_str.into_bytes());
```

### Collections

```rust
use armature_core::small_vec::{QueryParams, PathParams};

// ✅ Good: Use small_vec for typical sizes
let params = QueryParams::parse(query_string);

// ✅ Good: Check if inline
if params.is_inline() {
    // Fast path - no heap allocation
}

// ❌ Avoid: Standard Vec for small collections
let params: Vec<(String, String)> = parse_params(query_string);
```

## Summary

| Optimization | Impact | Location |
|--------------|--------|----------|
| LazyHeaders | 55% faster empty responses | `http.rs` |
| FastResponse | 2x faster response creation | `fast_response.rs` |
| SmallVec params | 99% fewer allocations | `small_vec.rs` |
| SIMD JSON | 1.8x faster serialization | `json.rs` |
| matchit router | O(log n) routing | `routing.rs` |
| Vectored I/O | 30% fewer syscalls | `vectored_io.rs` |
| io_uring | 5% I/O improvement | `io_uring.rs` |
| PGO | 10-15% overall | Build profile |

For more details, see the [benchmark documentation](armature-vs-nodejs-benchmark.md) and [profiling guide](../examples/profiling_server.rs).

