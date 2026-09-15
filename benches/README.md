# Armature Benchmark Suite

Performance benchmarks for the Armature framework, plus the cross-framework
comparison harness.

## Where the benchmarks live

Every criterion benchmark is owned by the crate it measures, so it moves with
that crate's repository and runs from that crate's `Cargo.toml`. This directory
holds only what is genuinely cross-cutting: the framework-comparison servers,
the TechEmpower harness, the HTTP load-test runner, and two pattern benchmarks
that exercise no Armature code at all.

### Per-crate benchmarks

| Crate | Bench targets | Measures |
|-------|---------------|----------|
| `armature-core` | `core`, `router`, `arena`, `body`, `json`, `micro`, `pipeline`, `resilience`, `simd_parser`, `internal_overhead` | Request/response construction, routing, arena allocation, body handling, JSON, SIMD parsing, resilience primitives, DI and handler dispatch overhead |
| `armature-h1` | `parse`, `write`, `e2e` | HTTP/1.1 parsing, serialization and end-to-end round-trip |
| `armature-jwt` | `jwt` | Token signing and verification across algorithms |
| `armature-auth` | `auth` | Password hashing, API keys, guards, OAuth2, session IDs |
| `armature-validation` | `validation` | Email/URL/string/numeric validators, pattern matching |
| `armature-cache` | `cache` | Cache keys, in-memory and tiered stores, TTL, concurrent access |
| `armature-cron` | `cron` | Cron expression parsing and presets |
| `armature-queue` | `queue` | Job and config construction, priorities, payload serialization |
| `armature-ratelimit` | `ratelimit` | Token bucket, sliding window, concurrent and hot-key workloads |
| `armature-storage` | `storage` | File validation, metadata, local storage, uploaded-file handling |
| `armature-http-client` | `http_client` | Client config, retry, circuit breaker, request building |

Target names say what they measure (`jwt`, `cache`, `simd_parser`). The two
benches that stayed in the root package keep their historical `_benchmarks`
suffix — that inconsistency is deliberate and bounded to those two.

Run one with `-p`:

```bash
cargo bench -p armature-core --bench internal_overhead
cargo bench -p armature-jwt  --bench jwt
cargo bench -p armature-cache --bench cache
```

Filter within a target by passing a name after `--`:

```bash
cargo bench -p armature-core --bench internal_overhead -- routing
cargo bench -p armature-core --bench json -- large
```

### Root-package benchmarks

Two benchmarks stay in the root package because they measure patterns rather
than any Armature crate:

| Bench target | Measures |
|--------------|----------|
| `database_benchmarks` | TechEmpower-shaped access patterns against an in-memory mock pool — no real driver or I/O |
| `memory_benchmarks` | Allocation *timing* for string/vec/map/pointer shapes and pooling — wall-clock only, not leak detection |

Neither imports an `armature_*` symbol — they are `criterion` + `crossbeam` +
`std` only — so they build under the root's default feature set:

```bash
cargo bench -p armature-framework --bench database_benchmarks
cargo bench -p armature-framework --bench memory_benchmarks
```

### Everything at once

`scripts/run-benchmarks.sh` wraps the per-crate invocations into suites:

```bash
./scripts/run-benchmarks.sh --all            # every suite
./scripts/run-benchmarks.sh --core --open    # armature-core, then open the report
./scripts/run-benchmarks.sh --h1             # armature-h1 parse/write/e2e
./scripts/run-benchmarks.sh --security       # jwt + auth
./scripts/run-benchmarks.sh --validation     # armature-validation
./scripts/run-benchmarks.sh --data           # cache, cron, queue, ratelimit, storage, http-client
./scripts/run-benchmarks.sh --patterns       # root database/memory pattern benches
./scripts/run-benchmarks.sh --all --baseline v0.3.0
```

## Framework Comparison

The per-crate benchmarks measure Armature's own internals; none of them compare
against another framework. Cross-framework numbers come from two trees, both
out-of-workspace (each declares its own `[workspace]`) and both compile-checked
by CI's `out-of-workspace-checks` job:

- `benches/comparison_servers/` — the Rust baselines (actix, axum, warp, rocket,
  a raw hyper h1 server) plus the Node.js servers, driven by the
  `http-benchmark` runner below.
- `benchmarks/comparison/` — the non-Rust competitors that need their own
  toolchains (Go fiber/gin, ASP.NET Core, Spring Boot, NestJS), with its own
  `run_benchmarks.sh` and checked-in `results/`.

### HTTP Benchmarks

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
| Rate Limit Check | < 100ns | Counter increment |
| File Validation | < 1μs | Size + MIME check |
| API Key Generate | < 5μs | Random bytes + encoding |

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
./scripts/run-benchmarks.sh --all --baseline main

# Test changes
git checkout feature-branch
./scripts/run-benchmarks.sh --all --compare main
```

## Adding New Benchmarks

Add the benchmark to the crate it measures — not to the root package.

### 1. Create the benchmark file in that crate

`armature-<crate>/benches/my_bench.rs`:

```rust
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_my_feature(c: &mut Criterion) {
    c.bench_function("my_feature", |b| {
        b.iter(|| my_function(black_box(input)))
    });
}

criterion_group!(benches, bench_my_feature);
criterion_main!(benches);
```

### 2. Declare it in that crate's `Cargo.toml`

Crates with benchmarks set `autobenches = false`, so a new file is not picked up
until it is declared:

```toml
[[bench]]
name = "my_bench"
harness = false
```

Add `criterion` to that crate's `[dev-dependencies]` if it isn't there yet.

### 3. Run

```bash
cargo bench -p armature-<crate> --bench my_bench
```

### 4. Wire it into CI if it should gate regressions

Lint coverage is automatic: `.github/workflows/ci.yml` derives the crate list
from `cargo metadata`, one invocation per crate, so a new benchmark-owning
crate is picked up with no edit.

Everything else is explicit, and these are all the places:

| File | What to add |
|---|---|
| `scripts/run-benchmarks.sh` | a suite entry (`"icon\|label\|package\|bench"`) |
| `.github/workflows/benchmark.yml` | a step, if it should be baseline-tracked; and `-p <crate>` in the weekly trend job |
| `scripts/benchmark-compare.sh` | `-p <crate>` in `BENCH_PACKAGES` |
| `benches/README.md` | a row in the per-crate table above |

`scripts/pgo-build.sh` is deliberately *not* on that list — its workload is a
curated hot-path subset, not everything.

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
cargo flamegraph -p armature-core --bench core

# Memory profiling with DHAT
./scripts/memory-profile.sh dhat 30

# Memory profiling with Valgrind
./scripts/memory-profile.sh valgrind 30

# Memory profiling with Heaptrack
./scripts/memory-profile.sh heaptrack 30

# Cachegrind
valgrind --tool=cachegrind target/release/deps/core-*
```

### Memory Leak Detection

`memory_benchmarks` times allocation shapes; it does not detect leaks. Use the
memory profiling server for that:

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
cargo bench -p armature-core
```

If a new file under `benches/` is ignored, it is missing a `[[bench]]` entry —
the benchmark-owning crates set `autobenches = false` on purpose.

### Inconsistent Results

- Close other applications
- Disable CPU scaling: `sudo cpupower frequency-set --governor performance`
- Run multiple iterations: `cargo bench -p armature-core -- --sample-size 1000`

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
# Run every suite
./scripts/run-benchmarks.sh --all

# Run internal overhead benchmarks
cargo bench -p armature-core --bench internal_overhead

# HTTP benchmarks
cargo run --release --example benchmark_server
oha -z 10s -c 50 http://localhost:3000/

# Full comparison
cargo run --release --bin http-benchmark -- --all

# Generate HTML report
./scripts/run-benchmarks.sh --all && open target/criterion/report/index.html
```

**Performance Expectations:**
- Sub-microsecond for core operations
- Sub-10μs for security operations
- Competitive with other Rust frameworks
- Excellent developer experience trade-off
