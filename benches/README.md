# Armature Benchmark Suite

Comprehensive performance benchmarks for all major components of the Armature framework,
including comparisons with other popular Rust web frameworks.

## Overview

The benchmark suite measures performance across **16 categories**:

### Core Framework Benchmarks
1. **Core Benchmarks** - HTTP request/response, routing, middleware, status codes
2. **Security Benchmarks** - JWT operations (sign, verify, algorithms)
3. **Validation Benchmarks** - Form validation, email, URL, patterns
4. **Data Benchmarks** - Queue jobs, cron expressions
5. **Framework Comparison** - Comparison with Actix-web, Axum, Warp, Rocket
6. **Memory Benchmarks** - Allocation patterns, leak detection, object pools

### Infrastructure Benchmarks
6. **Resilience Benchmarks** - Circuit breaker, retry, bulkhead, timeout, fallback
7. **HTTP Client Benchmarks** - Client config, retry, circuit breaker, request building
8. **Storage Benchmarks** - File validation, multipart, local/S3 storage
9. **Cache Benchmarks** - Memory cache, TTL, serialization, concurrent access
10. **Redis Benchmarks** - Config, keys, serialization, commands, pub/sub

### Application Benchmarks
11. **Auth Benchmarks** - Password hashing, API keys, guards, OAuth2, session IDs
12. **Mail Benchmarks** - Email building, attachments, templates, SMTP config
13. **Session Benchmarks** - Session ID generation, data management, cookie parsing
14. **Rate Limit Benchmarks** - Token bucket, sliding window, concurrent access

## Quick Start

```bash
# Run all benchmarks
cargo bench

# Run framework comparison benchmarks
cargo bench --bench framework_comparison

# Run HTTP benchmark server
cargo run --release --example benchmark_server

# Run comparison tool (requires oha or wrk)
cargo run --release --bin http-benchmark -- --framework armature
```

## Running Benchmarks

### Run All Benchmarks

```bash
cargo bench
```

### Run Specific Benchmark Suite

```bash
# === Core Framework Benchmarks ===
# Core HTTP and routing
cargo bench --bench core_benchmarks

# Security (JWT)
cargo bench --bench security_benchmarks

# Validation
cargo bench --bench validation_benchmarks

# Data processing (queue, cron)
cargo bench --bench data_benchmarks

# Framework comparison (micro-benchmarks)
cargo bench --bench framework_comparison

# Memory allocation patterns and leak detection
cargo bench --bench memory_benchmarks

# === Infrastructure Benchmarks ===
# Resilience patterns (circuit breaker, retry, bulkhead)
cargo bench --bench resilience_benchmarks

# HTTP Client operations
cargo bench --bench http_client_benchmarks

# File storage and validation
cargo bench --bench storage_benchmarks

# Caching operations
cargo bench --bench cache_benchmarks

# Redis operations
cargo bench --bench redis_benchmarks

# === Application Benchmarks ===
# Authentication operations
cargo bench --bench auth_benchmarks

# Email operations
cargo bench --bench mail_benchmarks

# Session management
cargo bench --bench session_benchmarks

# Rate limiting
cargo bench --bench ratelimit_benchmarks
```

### Run Specific Benchmark

```bash
# Run only JWT benchmarks
cargo bench --bench security_benchmarks jwt

# Run only routing benchmarks
cargo bench --bench framework_comparison routing

# Run only JSON operations
cargo bench --bench framework_comparison json_operations
```

## Framework Comparison

### Micro-Benchmarks

The `framework_comparison` benchmark measures internal operations:

- **Request Creation** - Building HttpRequest objects
- **Response Creation** - Building HttpResponse with JSON
- **JSON Operations** - Serialize/deserialize performance
- **Routing** - Route matching with 10-500 routes
- **Middleware** - Middleware creation overhead
- **DI Resolution** - Dependency injection container performance
- **Handler Invocation** - Async handler execution

```bash
cargo bench --bench framework_comparison
```

### HTTP Benchmarks

For real HTTP performance, use the benchmark runner:

```bash
# Start Armature benchmark server
cargo run --release --example benchmark_server

# In another terminal, run benchmarks
cargo run --release --bin http-benchmark -- --framework armature

# Compare with other frameworks (start their servers first)
cargo run --release --bin http-benchmark -- --all
```

### Comparison Servers

Start comparison servers for each framework:

```bash
# Armature (port 3000)
cargo run --release --example benchmark_server

# Actix-web (port 3001)
cd benches/comparison_servers/actix_server && cargo run --release

# Axum (port 3002)
cd benches/comparison_servers/axum_server && cargo run --release

# Warp (port 3003)
cd benches/comparison_servers/warp_server && cargo run --release

# Rocket (port 3004)
cd benches/comparison_servers/rocket_server && cargo run --release

# Node.js Frameworks (for comparison)

# Express (port 3006)
cd benches/comparison_servers/express_server && npm install && npm start

# Koa (port 3007)
cd benches/comparison_servers/koa_server && npm install && npm start

# NestJS (port 3008)
cd benches/comparison_servers/nestjs_server && npm install && npm run benchmark

# Next.js (port 3005)
cd benches/comparison_servers/nextjs_api && npm install && npm run benchmark
```

### Benchmark with oha (Recommended)

```bash
# Install oha
cargo install oha

# Plaintext
oha -z 10s -c 50 http://localhost:3000/

# JSON
oha -z 10s -c 50 http://localhost:3000/json

# Path parameters
oha -z 10s -c 50 http://localhost:3000/users/123

# POST with body
oha -z 10s -c 50 -m POST -d '{"name":"test"}' -H "Content-Type: application/json" http://localhost:3000/api/users
```

### Benchmark with wrk

```bash
# Install wrk
# Ubuntu: apt install wrk
# macOS: brew install wrk

# Basic benchmark
wrk -t4 -c50 -d10s http://localhost:3000/

# With latency stats
wrk -t4 -c50 -d10s --latency http://localhost:3000/json
```

## Benchmark Results

Results are saved in `target/criterion/` with:
- HTML reports for visualization
- Statistical analysis (mean, std dev, outliers)
- Historical comparison (if run multiple times)
- Performance graphs

View HTML reports:

```bash
open target/criterion/report/index.html
```

## Benchmark Categories

### Core Benchmarks (`core_benchmarks.rs`)

- **HTTP Request Creation** - Creating HttpRequest instances
- **HTTP Response Creation** - ok(), with_json(), with_body()
- **JSON Parsing** - Deserializing request bodies
- **Form Parsing** - URL-encoded form data
- **Middleware Chain** - Processing with 1, 5, 10, 20 middleware
- **Routing** - Route matching with 100 routes
- **Status Codes** - Status code lookups and checks
- **Error Handling** - Error creation and status mapping

### Security Benchmarks (`security_benchmarks.rs`)

- Token signing (HS256, HS384, HS512)
- Token verification
- Algorithm comparison

### Validation Benchmarks (`validation_benchmarks.rs`)

- **Email Validation** - Valid and invalid emails
- **URL Validation** - Various URL formats
- **String Validators** - MinLength, MaxLength, IsAlpha, etc.
- **Numeric Validators** - Min, Max, InRange, IsPositive
- **Pattern Matching** - Regex validation

### Data Benchmarks (`data_benchmarks.rs`)

- **Queue Jobs** - Job creation, serialization
- **Cron Expressions** - Parsing, next execution

### Framework Comparison (`framework_comparison.rs`)

- **Request/Response** - Object creation overhead
- **JSON** - Serialization with small/medium/large payloads
- **Routing** - Route matching with 10/50/100/500 routes
- **DI** - Container operations, service resolution
- **Handlers** - Async handler invocation patterns
- **Full Cycle** - Complete request handling

### Memory Benchmarks (`memory_benchmarks.rs`)

- **String Allocations** - Small, medium, large strings, formatting, concatenation
- **Vec Allocations** - With/without capacity, clone, nested vectors
- **HashMap Allocations** - With/without capacity, string keys, clone
- **Smart Pointers** - Box, Arc, Rc allocation and cloning
- **Request/Response** - Simulated HTTP object allocation patterns
- **Object Pool** - Pool vs direct allocation comparison
- **Leak Patterns** - Bounded vs unbounded cache, weak references
- **Allocation Sizes** - Various sizes from 64B to 64KB
- **Drop Timing** - Deallocation performance for various structures

### Resilience Benchmarks (`resilience_benchmarks.rs`)

- **Circuit Breaker** - Creation, state checks, recording success/failure
- **Retry** - Config creation, backoff calculation
- **Bulkhead** - Creation, permit acquisition, stats
- **Timeout** - Creation, wrap operations
- **Fallback** - Single and chained fallbacks
- **Combined Patterns** - Full resilience stack overhead

### HTTP Client Benchmarks (`http_client_benchmarks.rs`)

- **Configuration** - Default, builder, full config
- **Retry Config** - None, default, backoff strategies
- **Circuit Breaker** - State checks, recording
- **Request Building** - GET, POST, headers
- **Response Processing** - Status checks, URL parsing

### Storage Benchmarks (`storage_benchmarks.rs`)

- **File Validation** - Size, MIME type, extension checks
- **File Metadata** - Filename sanitization, key generation, checksums
- **Local Storage** - Config, storage creation
- **Uploaded File** - Clone, extension, MIME parsing
- **Bytes Operations** - Creation, slicing

### Cache Benchmarks (`cache_benchmarks.rs`)

- **Cache Keys** - Simple, formatted, hashed keys
- **Memory Cache** - Create, set, get (hit/miss), delete, exists
- **TTL Management** - Set with various TTLs
- **Serialization** - JSON serialize/deserialize for cache values
- **Concurrent Access** - Mixed read/write workloads

### Auth Benchmarks (`auth_benchmarks.rs`)

- **Password Hashing** - Hash short/long passwords, verify
- **API Key** - Generate, parse, compare
- **Auth Guards** - Role/permission checking
- **User Context** - Creation, cloning
- **OAuth2** - Token creation, serialization
- **Session ID** - UUID generation, random bytes, hex encoding

### Redis Benchmarks (`redis_benchmarks.rs`)

- **Configuration** - Default, builder, full config, URL parsing
- **Key Generation** - Simple, formatted, complex, batch keys
- **Value Serialization** - Small/medium/large JSON
- **Command Building** - GET, SET, HSET, MGET, pipeline
- **Pub/Sub** - Channel names, message serialization
- **Lua Scripts** - Simple and complex scripts

### Mail Benchmarks (`mail_benchmarks.rs`)

- **Email Address** - Creation, parsing, validation
- **Email Building** - Simple, HTML, multiple recipients, headers
- **Attachments** - Small/medium, MIME type detection
- **Templates** - Engine creation, registration, rendering
- **SMTP Config** - Basic, from environment
- **Email Serialization** - Simple/complex emails

### Session Benchmarks (`session_benchmarks.rs`)

- **Session ID** - UUID, random bytes, validation
- **Configuration** - Default, custom, cookie building
- **Session Data** - Create, get, insert, remove, contains
- **Serialization** - Simple/complex session data
- **Cookie Parsing** - Extract session ID, parse all cookies
- **Memory Store** - Create, get (hit/miss), delete

### Rate Limit Benchmarks (`ratelimit_benchmarks.rs`)

- **Configuration** - Basic, per-route, builder
- **Key Generation** - IP, user, route, complex keys
- **Memory Limiter** - Create, check, remaining, reset
- **Sliding Window** - Timestamp, window calculation
- **Token Bucket** - Creation, consumption, refill
- **Response Headers** - Rate limit headers, retry-after
- **Concurrent Access** - Mixed workload, hot keys

## Performance Targets

### Target Latencies (p50)

| Operation | Target | Notes |
|-----------|--------|-------|
| HTTP Request Creation | < 100ns | Minimal allocation |
| JSON Parsing (small) | < 1μs | Typical API payload |
| JWT Sign | < 10μs | HS256 algorithm |
| JWT Verify | < 20μs | Includes signature check |
| Email Validation | < 500ns | Regex check |
| Route Match (100 routes) | < 1μs | Prefix tree |
| DI Resolution | < 50ns | DashMap lookup |
| Circuit Breaker Check | < 50ns | State lookup |
| Bulkhead Acquire | < 100ns | Semaphore acquire |
| Cache Get (memory) | < 500ns | DashMap lookup |
| Session ID Generate | < 1μs | UUID v4 |
| Rate Limit Check | < 100ns | Counter increment |
| File Validation | < 1μs | Size + MIME check |
| API Key Generate | < 5μs | Random bytes + encoding |
| Email Build (simple) | < 500ns | String allocation |

### Throughput Targets

| Operation | Target | Notes |
|-----------|--------|-------|
| HTTP Requests (plaintext) | > 200K/s | Single core |
| HTTP Requests (JSON) | > 150K/s | With serialization |
| JWT Operations | > 50K/s | Sign + verify |
| Validations | > 1M/s | Simple validators |

## Expected HTTP Performance

Typical performance on modern hardware (varies by configuration):

### Rust Frameworks

| Framework | Plaintext (req/s) | JSON (req/s) | Relative |
|-----------|------------------|--------------|----------|
| Actix-web | 400K-600K | 300K-450K | 100% |
| Axum | 350K-500K | 280K-400K | ~85% |
| Warp | 300K-450K | 250K-350K | ~75% |
| Armature | 250K-400K | 200K-300K | ~65% |
| Rocket | 200K-350K | 150K-250K | ~55% |

### Node.js Frameworks (for comparison)

| Framework | Plaintext (req/s) | JSON (req/s) | Relative |
|-----------|------------------|--------------|----------|
| Express | 25K-50K | 20K-45K | ~8% |
| Koa | 30K-55K | 25K-50K | ~10% |
| NestJS | 20K-45K | 18K-40K | ~7% |
| Next.js | 15K-40K | 12K-35K | ~5% |

**Note:** Armature prioritizes developer experience, type safety, and features
(DI, validation, middleware, etc.) alongside raw performance.

**Rust vs Node.js:** Rust frameworks typically achieve 10-15x higher throughput than
Node.js frameworks. Node.js frameworks are included for real-world comparison when evaluating
Armature as a backend for JavaScript/TypeScript frontends.

See [Armature vs Next.js Benchmark Guide](../docs/guides/armature-vs-nextjs-benchmark.md) for detailed comparison.

## Interpreting Results

### Key Metrics

- **Mean** - Average time per operation
- **Std Dev** - Consistency of performance
- **Median** - 50th percentile (p50)
- **Outliers** - Operations outside normal range
- **Throughput** - Operations per second

### Performance Regression

Criterion automatically detects:
- ✅ **Improvement** - Green, faster than baseline
- ⚠️ **Regression** - Yellow/Red, slower than baseline
- 📊 **No change** - Within noise threshold

### Comparing Results

```bash
# Run baseline
git checkout main
cargo bench -- --save-baseline main

# Test changes
git checkout feature-branch
cargo bench -- --baseline main
```

## Adding New Benchmarks

### 1. Create Benchmark File

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_my_feature(c: &mut Criterion) {
    c.bench_function("my_feature", |b| {
        b.iter(|| {
            my_function(black_box(input))
        })
    });
}

criterion_group!(benches, bench_my_feature);
criterion_main!(benches);
```

### 2. Add to `Cargo.toml`

```toml
[[bench]]
name = "my_benchmarks"
harness = false
```

### 3. Run

```bash
cargo bench --bench my_benchmarks
```

## Best Practices

### DO

✅ Use `black_box()` to prevent compiler optimizations
✅ Benchmark realistic workloads
✅ Measure multiple input sizes
✅ Run benchmarks on consistent hardware
✅ Check for regressions before merging
✅ Use `--release` for HTTP benchmarks

### DON'T

❌ Benchmark trivial operations
❌ Include setup in benchmark loop
❌ Run benchmarks with debug builds
❌ Compare results across different machines
❌ Ignore performance regressions

## Profiling

For detailed profiling:

```bash
# CPU profiling with flamegraph
cargo flamegraph --bench core_benchmarks

# Memory profiling with DHAT
./scripts/memory-profile.sh dhat 30

# Memory profiling with Valgrind
./scripts/memory-profile.sh valgrind 30

# Memory profiling with Heaptrack
./scripts/memory-profile.sh heaptrack 30

# Cachegrind
valgrind --tool=cachegrind target/release/deps/core_benchmarks-*
```

### Memory Leak Detection

Use the memory profiling server for leak detection:

```bash
# Build with memory profiling
cargo build --example memory_profile_server --release --features memory-profiling

# Run and generate load
./target/release/examples/memory_profile_server &
curl http://localhost:3000/health  # Generate requests
kill %1  # Generates DHAT report

# View report at: https://nnethercote.github.io/dh_view/dh_view.html
```

See `docs/memory-profiling-guide.md` for comprehensive documentation.

## Troubleshooting

### Benchmarks Won't Run

```bash
cargo clean
cargo bench
```

### Inconsistent Results

- Close other applications
- Disable CPU scaling: `sudo cpupower frequency-set --governor performance`
- Run multiple iterations: `cargo bench -- --sample-size 1000`

### HTTP Benchmark Issues

- Ensure server is running: `curl http://localhost:3000/health`
- Check for port conflicts: `lsof -i :3000`
- Verify tool installation: `oha --version` or `wrk --version`

## Resources

- [Criterion.rs User Guide](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [TechEmpower Benchmarks](https://www.techempower.com/benchmarks/)
- [oha - HTTP load generator](https://github.com/hatoo/oha)

## Summary

**Quick Commands:**

```bash
# Run all benchmarks
cargo bench

# Run framework comparison
cargo bench --bench framework_comparison

# HTTP benchmarks
cargo run --release --example benchmark_server
oha -z 10s -c 50 http://localhost:3000/

# Full comparison
cargo run --release --bin http-benchmark -- --all

# Generate HTML report
cargo bench && open target/criterion/report/index.html
```

**Performance Expectations:**
- Sub-microsecond for core operations
- Sub-10μs for security operations
- Competitive with other Rust frameworks
- Excellent developer experience trade-off
