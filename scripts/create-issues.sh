#!/bin/bash
# Create GitHub issues for high-priority performance TODOs
# Run: gh auth login (if not already authenticated)
# Then: ./scripts/create-issues.sh

set -e

REPO="pegasusheavy/armature"

echo "Creating GitHub issues for high-priority TODOs..."
echo "Repository: $REPO"
echo ""

# =============================================================================
# CREATE LABELS (if they don't exist)
# =============================================================================

echo "Creating labels (if they don't exist)..."

# Create labels - ignore errors if they already exist
gh label create "enhancement" --repo "$REPO" --color "a2eeef" --description "New feature or request" 2>/dev/null || true
gh label create "performance" --repo "$REPO" --color "d4c5f9" --description "Performance improvement" 2>/dev/null || true
gh label create "priority: critical" --repo "$REPO" --color "b60205" --description "Critical priority - blocks release" 2>/dev/null || true
gh label create "priority: high" --repo "$REPO" --color "d93f0b" --description "High priority" 2>/dev/null || true
gh label create "benchmarks" --repo "$REPO" --color "0e8a16" --description "Benchmark related" 2>/dev/null || true
gh label create "ci" --repo "$REPO" --color "1d76db" --description "CI/CD related" 2>/dev/null || true

echo "Labels ready."
echo ""

# =============================================================================
# CRITICAL (🔴) - Axum Competitive
# =============================================================================

echo "Creating Critical Priority issues..."

gh issue create --repo "$REPO" \
  --title "perf: Replace Trie router with \`matchit\` crate" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

The current trie-based router is a performance bottleneck, accounting for **~7% of CPU time** in profiling. Axum uses the \`matchit\` crate which is specifically optimized for HTTP routing patterns and significantly outperforms generic trie implementations.

### Business Impact
- **Throughput**: Potential 5-10% overall throughput improvement
- **Latency**: Reduced p99 latency for route-heavy applications
- **Competitiveness**: Parity with Axum's routing performance

### Technical Details
The \`matchit\` crate uses a radix tree optimized for URL patterns with:
- O(k) lookup where k is path segment count (not path length)
- Zero allocations during matching
- Efficient parameter extraction
- Wildcard and catch-all support

### Implementation Plan
1. Add \`matchit = \"0.8\"\` dependency
2. Create adapter layer in \`armature-core/routing.rs\`
3. Migrate route registration to matchit's \`Router::new()\`
4. Update parameter extraction to use matchit's \`Params\`
5. Remove old trie implementation
6. Benchmark with wrk/hey before and after

### Acceptance Criteria
- [ ] All existing route tests pass
- [ ] Benchmark shows measurable improvement
- [ ] No breaking changes to public API

### References
- [matchit crate](https://crates.io/crates/matchit)
- [Axum router implementation](https://github.com/tokio-rs/axum/blob/main/axum/src/routing/mod.rs)

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Compile-time route validation" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Routes are currently validated at runtime during application startup. This wastes CPU cycles on every startup and misses the opportunity to catch routing errors at compile time. Compile-time validation improves both performance and developer experience.

### Business Impact
- **Startup Time**: Faster application cold starts
- **Developer Experience**: Immediate feedback on invalid routes
- **Reliability**: Routing errors caught before deployment

### Technical Details
The \`#[get(\"/path\")]\` macro should validate:
- Valid URL pattern syntax
- No duplicate parameter names
- Valid regex constraints (if any)
- Conflicting routes detection

### Implementation Plan
1. Add route parsing logic to \`armature-proc-macro\`
2. Emit \`compile_error!()\` for invalid patterns
3. Generate static route metadata at compile time
4. Remove runtime validation from hot paths
5. Add comprehensive error messages with suggestions

### Example
\`\`\`rust
// This should fail at compile time, not runtime
#[get(\"/users/{id}/{id}\")] // Error: duplicate parameter 'id'
async fn get_user() {}
\`\`\`

### Acceptance Criteria
- [ ] Invalid routes cause compile errors
- [ ] Error messages are helpful and actionable
- [ ] No runtime validation overhead

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Inline handler dispatch via monomorphization" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Handler dispatch currently uses dynamic dispatch (trait objects), preventing the compiler from inlining handler calls. Axum achieves zero-cost abstractions through aggressive monomorphization, where each handler type generates specialized code that can be fully inlined.

### Business Impact
- **Throughput**: 10-20% improvement in request handling
- **Latency**: Reduced function call overhead
- **Binary Size**: Trade-off: larger binary for better performance

### Technical Details
Current problematic pattern:
\`\`\`rust
// BAD: Dynamic dispatch prevents inlining
type Handler = Box<dyn Fn(Request) -> Response>;
\`\`\`

Target pattern:
\`\`\`rust
// GOOD: Monomorphization enables inlining
fn handle<H: Handler>(handler: H, req: Request) -> Response {
    handler.call(req) // Can be inlined
}
\`\`\`

### Implementation Plan
1. Audit handler storage in \`armature-core\`
2. Replace \`Box<dyn Handler>\` with generic type parameters
3. Use \`impl Trait\` in return positions
4. Add \`#[inline]\` attributes to hot path functions
5. Verify inlining with \`cargo asm\` or \`perf\`

### Acceptance Criteria
- [ ] No \`Box<dyn>\` in request handling hot path
- [ ] \`cargo asm\` shows inlined handler calls
- [ ] Benchmark shows measurable improvement

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Remove runtime type checks from DI hot paths" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

The dependency injection system uses \`TypeId\` and \`Any::downcast\` for runtime type resolution. These operations occur on every request, adding measurable overhead. Compile-time type resolution can eliminate this entirely.

### Business Impact
- **Throughput**: 5-15% improvement depending on DI usage
- **Latency**: Reduced per-request overhead
- **Type Safety**: Compile-time errors instead of runtime panics

### Technical Details
Current problematic pattern:
\`\`\`rust
// BAD: Runtime type check on every request
fn get<T: 'static>(&self) -> Option<&T> {
    self.map.get(&TypeId::of::<T>())?.downcast_ref()
}
\`\`\`

Target pattern:
\`\`\`rust
// GOOD: Type resolved at compile time via generics
fn get<T>(&self) -> &T where Self: Has<T> {
    // Compile-time guaranteed to exist
}
\`\`\`

### Implementation Plan
1. Audit \`armature-core/di.rs\` for \`Any\` and \`TypeId\` usage
2. Use type-state pattern for compile-time DI resolution
3. Generate DI accessors via proc macro
4. Keep runtime fallback for dynamic use cases (feature-gated)

### Acceptance Criteria
- [ ] No \`TypeId\` lookups in request hot path
- [ ] DI errors caught at compile time
- [ ] Benchmark shows measurable improvement

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Per-request arena allocator" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Each HTTP request triggers many small allocations (headers, body parts, extracted parameters, etc.). These allocations contend on the global allocator and require individual deallocations. An arena allocator batches all request allocations and frees them in a single operation when the request completes.

### Business Impact
- **Throughput**: 15-25% improvement under high concurrency
- **Latency**: More consistent p99 latency (less allocator jitter)
- **Memory**: Reduced fragmentation

### Technical Details
Arena allocators provide:
- O(1) allocation (bump pointer)
- O(1) deallocation (reset pointer)
- Cache-friendly memory layout
- No fragmentation within request lifetime

Recommended crate: \`bumpalo\`

\`\`\`rust
// Each request gets its own arena
let arena = Bump::new();
let headers = arena.alloc_slice_copy(&parsed_headers);
let body = arena.alloc_str(&body_string);
// All freed when arena drops
\`\`\`

### Implementation Plan
1. Add \`bumpalo\` dependency
2. Create \`RequestArena\` wrapper type
3. Pass arena through request context
4. Migrate string/vec allocations to arena
5. Benchmark allocation patterns before/after

### Acceptance Criteria
- [ ] Request handling uses arena for temp allocations
- [ ] No increase in peak memory usage
- [ ] Benchmark shows throughput improvement

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Direct Hyper body passthrough" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Armature currently wraps Hyper's body types in custom abstractions, causing unnecessary allocation and copying. Axum is essentially a thin routing layer over Hyper, passing bodies through directly. We should minimize our abstraction overhead.

### Business Impact
- **Throughput**: 5-10% improvement for body-heavy workloads
- **Memory**: Reduced copying for large request/response bodies
- **Compatibility**: Better integration with Hyper ecosystem

### Technical Details
Current problematic pattern:
\`\`\`rust
// BAD: Wrapping causes allocation
let body = OurBody::from(hyper_body);
let response = Response::new(OurBody::from(response_body));
\`\`\`

Target pattern:
\`\`\`rust
// GOOD: Pass-through with zero overhead
type Body = http_body_util::combinators::BoxBody<Bytes, Error>;
\`\`\`

### Implementation Plan
1. Audit body handling in \`armature-core\`
2. Use \`http_body_util\` for body manipulation
3. Minimize \`Body\` type conversions
4. Use \`Bytes\` for zero-copy body access
5. Benchmark large file uploads/downloads

### Acceptance Criteria
- [ ] No unnecessary body copies in hot path
- [ ] Large body handling doesn't increase memory usage
- [ ] Benchmark shows improvement for body-heavy requests

## Priority
🔴 **Critical** - Required for Axum-competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: TechEmpower Benchmark Suite" \
  --label "enhancement,performance,priority: critical,benchmarks" \
  --body "## Executive Summary

TechEmpower Framework Benchmarks (TFB) is the industry standard for comparing web framework performance. Without TFB results, potential adopters cannot objectively compare Armature to competitors. Implementing the full TFB suite enables data-driven optimization and marketing.

### Business Impact
- **Adoption**: Objective performance data attracts users
- **Marketing**: Top rankings drive framework visibility
- **Development**: Identifies optimization opportunities

### Technical Details
TFB includes these test types:
1. **JSON**: Serialize a simple JSON object
2. **Plaintext**: Return \"Hello, World!\"
3. **DB Single Query**: Fetch one random row
4. **DB Multiple Queries**: Fetch N random rows
5. **Fortunes**: Template rendering with HTML escaping
6. **DB Updates**: Read-modify-write N rows
7. **Cached Queries**: Fetch from cache

### Implementation Plan
1. Create \`benches/techempower/\` directory
2. Implement each test type as separate example
3. Create Dockerfile for TFB infrastructure
4. Add database setup scripts (PostgreSQL)
5. Submit to TechEmpower repository
6. Set up local TFB runner for development

### Directory Structure
\`\`\`
benches/techempower/
├── Dockerfile
├── config.toml
├── src/
│   ├── json.rs
│   ├── plaintext.rs
│   ├── db.rs
│   ├── fortunes.rs
│   └── cached.rs
└── templates/
    └── fortunes.html
\`\`\`

### Acceptance Criteria
- [ ] All 7 TFB test types implemented
- [ ] Local benchmark runner works
- [ ] Results competitive with Axum/Actix
- [ ] PR submitted to TechEmpower repo

## Priority
🔴 **Critical** - Required for industry recognition"

# =============================================================================
# CRITICAL (🔴) - Actix Competitive
# =============================================================================

gh issue create --repo "$REPO" \
  --title "perf: HTTP/1.1 request pipelining" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

HTTP/1.1 pipelining allows clients to send multiple requests without waiting for responses. Actix-web handles this exceptionally well, processing batched requests from the socket buffer efficiently. This is critical for high-throughput scenarios with keep-alive connections.

### Business Impact
- **Throughput**: 20-40% improvement for pipelined clients
- **Latency**: Reduced round-trip overhead
- **Efficiency**: Better utilization of keep-alive connections

### Technical Details
Without pipelining:
\`\`\`
Client: [Request 1] --wait-- [Request 2] --wait-- [Request 3]
Server: [Response 1] ------- [Response 2] ------- [Response 3]
\`\`\`

With pipelining:
\`\`\`
Client: [Request 1][Request 2][Request 3]
Server: [Process all] -> [Response 1][Response 2][Response 3]
\`\`\`

### Implementation Plan
1. Modify connection handler to buffer multiple requests
2. Parse requests in batch from read buffer
3. Process requests concurrently (respecting order for responses)
4. Queue responses for ordered sending
5. Handle partial request reads correctly

### Acceptance Criteria
- [ ] Multiple requests per read() call handled
- [ ] Response ordering preserved
- [ ] Benchmark with pipelining client shows improvement

## Priority
🔴 **Critical** - Actix competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Request batching from socket buffer" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Currently, each \`read()\` from the socket triggers individual request processing. When multiple requests arrive together (common with keep-alive and pipelining), we should batch-process them to amortize syscall and parsing overhead.

### Business Impact
- **Throughput**: Significant improvement under high load
- **Syscalls**: Reduced kernel transitions
- **Efficiency**: Better CPU cache utilization

### Technical Details
\`\`\`rust
// BAD: One request per read
loop {
    let bytes = socket.read(&mut buf)?;
    let request = parse_one_request(&buf)?;
    handle(request).await;
}

// GOOD: Batch processing
loop {
    let bytes = socket.read(&mut buf)?;
    let requests = parse_all_requests(&buf)?;
    for request in requests {
        handle(request).await;
    }
}
\`\`\`

### Implementation Plan
1. Increase socket read buffer size (e.g., 16KB)
2. Implement multi-request parser
3. Return iterator of requests from single buffer
4. Process batch with appropriate concurrency

### Acceptance Criteria
- [ ] Multiple requests parsed from single read
- [ ] Benchmark shows batching benefit
- [ ] Memory usage remains reasonable

## Priority
🔴 **Critical** - Actix competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: BytesMut buffer pool" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Actix-web uses thread-local pools of pre-allocated \`BytesMut\` buffers, eliminating allocation overhead for request/response handling. This is one of their key performance advantages and explains their TechEmpower dominance.

### Business Impact
- **Throughput**: 15-30% improvement
- **Latency**: Consistent performance (no allocator jitter)
- **Memory**: Reduced fragmentation, predictable usage

### Technical Details
\`\`\`rust
thread_local! {
    static BUFFER_POOL: RefCell<Vec<BytesMut>> = RefCell::new(
        (0..32).map(|_| BytesMut::with_capacity(8192)).collect()
    );
}

fn acquire_buffer() -> BytesMut {
    BUFFER_POOL.with(|pool| {
        pool.borrow_mut().pop().unwrap_or_else(|| BytesMut::with_capacity(8192))
    })
}

fn release_buffer(mut buf: BytesMut) {
    buf.clear();
    BUFFER_POOL.with(|pool| {
        if pool.borrow().len() < 32 {
            pool.borrow_mut().push(buf);
        }
    });
}
\`\`\`

### Implementation Plan
1. Create \`armature-core/buffer.rs\` module
2. Implement thread-local buffer pool
3. Integrate with request/response handling
4. Add pool size configuration
5. Add metrics for pool utilization

### Acceptance Criteria
- [ ] Buffer allocation in hot path uses pool
- [ ] Pool size is configurable
- [ ] Benchmark shows throughput improvement

## Priority
🔴 **Critical** - Actix competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Zero-copy request body parsing" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

Request bodies are currently copied during parsing. With buffer pooling in place, we can parse directly into pooled buffers, eliminating copies entirely. This is especially impactful for large request bodies (file uploads, JSON payloads).

### Business Impact
- **Throughput**: Major improvement for body-heavy workloads
- **Memory**: 50% reduction in body handling memory
- **Latency**: Faster time-to-first-byte

### Technical Details
\`\`\`rust
// BAD: Copy into new allocation
let body: Vec<u8> = read_body(&mut stream).await?;

// GOOD: Parse directly into pooled buffer
let mut buf = acquire_buffer();
stream.read_buf(&mut buf).await?;
// buf now contains body, no copy needed
\`\`\`

### Implementation Plan
1. Ensure buffer pool is in place (#buffer-pool)
2. Modify body reading to use pooled buffers
3. Return \`Bytes\` (zero-copy slice) instead of \`Vec<u8>\`
4. Update JSON/form parsers to work with \`Bytes\`
5. Benchmark file upload performance

### Acceptance Criteria
- [ ] No body copies during request handling
- [ ] Large file upload benchmark improves
- [ ] Memory usage decreases under load

## Priority
🔴 **Critical** - Actix competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: io_uring support for Linux" \
  --label "enhancement,performance,priority: critical" \
  --body "## Executive Summary

\`io_uring\` is Linux's modern async I/O interface (5.1+), offering significantly better performance than epoll by reducing syscalls and enabling true async operations. This is increasingly important for high-performance servers.

### Business Impact
- **Throughput**: 20-50% improvement on Linux
- **Syscalls**: 50-80% reduction in kernel transitions
- **Latency**: Lower and more consistent tail latency

### Technical Details
\`epoll\` requires:
1. \`epoll_wait()\` - wait for events
2. \`read()\` / \`write()\` - perform I/O
3. \`epoll_ctl()\` - modify interest

\`io_uring\` batches all into single submission:
1. Submit read/write requests to ring
2. Kernel processes asynchronously
3. Poll completion ring for results

### Implementation Plan
1. Add feature flag: \`io-uring\`
2. Integrate \`tokio-uring\` or \`glommio\`
3. Abstract I/O layer to support both backends
4. Benchmark epoll vs io_uring
5. Document Linux version requirements

### Acceptance Criteria
- [ ] io_uring backend works on Linux 5.1+
- [ ] Graceful fallback to epoll on older kernels
- [ ] Benchmark shows significant improvement

## Priority
🔴 **Critical** - Actix competitive performance"

gh issue create --repo "$REPO" \
  --title "perf: Actix-web comparison benchmark" \
  --label "enhancement,performance,priority: critical,benchmarks" \
  --body "## Executive Summary

We need direct, reproducible benchmarks comparing Armature vs Actix-web on identical workloads. This enables data-driven optimization and validates our performance claims.

### Business Impact
- **Development**: Identify specific performance gaps
- **Marketing**: Quantify performance characteristics
- **Validation**: Verify optimization improvements

### Technical Details
Benchmark suite should include:
1. **Hello World**: Minimal overhead baseline
2. **JSON Echo**: Serialization round-trip
3. **Parameter Extraction**: Route parameter parsing
4. **Middleware Stack**: Multiple middleware layers
5. **Database Query**: Real I/O workload
6. **File Serving**: Static file performance

### Implementation Plan
1. Create \`benches/comparison/actix/\` directory
2. Implement identical endpoints in both frameworks
3. Use consistent benchmarking tool (wrk, hey, or criterion)
4. Automate benchmark runs with statistics
5. Generate comparison reports

### Benchmark Setup
\`\`\`
benches/comparison/
├── actix_server/      # Actix-web implementation
├── armature_server/   # Our implementation
├── run_benchmarks.sh  # Automated runner
└── results/           # Output data
\`\`\`

### Acceptance Criteria
- [ ] Identical functionality in both implementations
- [ ] Reproducible benchmark results
- [ ] Statistical analysis of results
- [ ] Clear performance delta measurement

## Priority
🔴 **Critical** - Required for performance validation"

# =============================================================================
# HIGH PRIORITY (🟠) - Performance Optimizations
# =============================================================================

echo "Creating High Priority issues..."

gh issue create --repo "$REPO" \
  --title "perf: Route matching cache" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Cache compiled route matching results to avoid repeated tree traversal for frequently-accessed routes. Most applications have a small set of \"hot\" routes that handle the majority of traffic.

### Business Impact
- **Throughput**: 5-10% improvement for route-heavy apps
- **Latency**: Faster routing for hot paths

### Implementation
- LRU cache for route match results
- Cache key: request path + method
- Configurable cache size

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Static route fast path" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Routes without parameters (e.g., \`/api/health\`, \`/api/users\`) can use HashMap lookup instead of tree traversal, achieving O(1) matching.

### Business Impact
- **Throughput**: Immediate improvement for static routes
- **Simplicity**: Easy win with minimal code changes

### Implementation
1. Separate static routes into HashMap at registration
2. Check HashMap before falling back to tree
3. Most health checks and API roots are static

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: SIMD JSON serialization" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

JSON serialization/deserialization is often the dominant CPU cost in API servers. SIMD-accelerated JSON parsers (\`simd-json\`, \`sonic-rs\`) offer 2-5x performance improvements.

### Business Impact
- **Throughput**: 2-5x improvement for JSON-heavy APIs
- **Latency**: Faster response times

### Implementation
1. Add optional \`simd-json\` or \`sonic-rs\` dependency
2. Feature flag: \`simd-json\`
3. Benchmark vs standard serde_json

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Zero-allocation route parameter extraction" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Route parameters (e.g., \`/users/{id}\`) should be extracted without allocating strings. Return slices into the original path instead.

### Business Impact
- **Throughput**: Reduced allocation overhead
- **Memory**: Less garbage collection pressure

### Implementation
Use \`Cow<'_, str>\` or direct slices for parameters.

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Const generic extractors" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Use const generics for extractor chains to enable compile-time optimization and eliminate runtime overhead for extractor composition.

### Implementation
\`\`\`rust
// Extractor tuple with const generic count
impl<T1, T2> FromRequest for (T1, T2)
where T1: FromRequest, T2: FromRequest
{ /* zero-cost extraction */ }
\`\`\`

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Static dispatch middleware" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Replace \`Box<dyn Middleware>\` with static dispatch using generics and Tower-style service composition.

### Business Impact
- **Throughput**: Middleware calls can be inlined
- **Compatibility**: Better Tower ecosystem integration

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: SmallVec for headers" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Most HTTP requests have 8-16 headers. Using \`SmallVec<[Header; 16]>\` avoids heap allocation for typical cases while still supporting large header sets.

### Implementation
Replace \`Vec<Header>\` with \`SmallVec<[Header; 16]>\` in request/response types.

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Tower Service compatibility" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Implement \`tower::Service\` trait for Armature handlers and middleware. This enables integration with the Tower ecosystem (timeouts, rate limiting, load balancing, etc.) and establishes performance parity patterns used by Axum.

### Business Impact
- **Ecosystem**: Access to Tower middleware
- **Compatibility**: Familiar patterns for Rust developers
- **Performance**: Tower's optimized abstractions

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Reduce task spawning overhead" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Simple handlers can be executed inline without spawning a new task. Task spawning adds ~100-500ns overhead per request that can be eliminated for synchronous handlers.

### Implementation
- Detect sync vs async handlers at compile time
- Inline sync handlers directly
- Only spawn tasks for truly async work

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Profile-Guided Optimization (PGO) build profile" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

PGO uses runtime profiling data to guide compiler optimizations, typically yielding 10-20% performance improvements for real workloads.

### Implementation
1. Add \`[profile.pgo]\` to Cargo.toml
2. Document PGO build process
3. Provide sample workload for profiling

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Vectored I/O (writev) for responses" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Use \`writev()\` syscall to send HTTP headers and body in a single kernel call instead of separate \`write()\` calls.

### Business Impact
- **Syscalls**: 50% reduction for response sending
- **Latency**: Lower time-to-first-byte

### Implementation
Gather headers + body into iovec and use vectored write.

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: CPU core affinity for workers" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Pin worker threads to specific CPU cores to improve cache locality and reduce context switching overhead. Critical for NUMA systems.

### Implementation
- Use \`core_affinity\` crate
- Optional configuration for core pinning
- Auto-detect NUMA topology

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Connection recycling" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Reset and reuse connection objects instead of allocating new ones for each connection. Reduces allocator pressure under high connection rates.

### Implementation
- Pool of pre-allocated connection contexts
- Reset state on connection close
- Return to pool for reuse

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Streaming response body support" \
  --label "enhancement,performance,priority: high" \
  --body "## Executive Summary

Allow sending response headers before body is fully generated. Critical for large responses, SSE, and reducing time-to-first-byte.

### Implementation
- Support \`impl Stream<Item = Bytes>\` as response body
- Chunked transfer encoding
- Flush headers immediately

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "ci: Automated performance regression tests" \
  --label "enhancement,ci,priority: high" \
  --body "## Executive Summary

Automatically run benchmarks on PRs and fail if performance regresses beyond a threshold. Prevents accidental performance degradation.

### Implementation
1. GitHub Actions workflow
2. Benchmark against main branch
3. Fail PR if >5% regression
4. Post results as PR comment

## Priority
🟠 **High Priority**"

gh issue create --repo "$REPO" \
  --title "perf: Axum comparison benchmark" \
  --label "enhancement,performance,priority: high,benchmarks" \
  --body "## Executive Summary

Maintain ongoing benchmark comparison with Axum to track performance parity. Should run in CI and generate reports.

### Implementation
- Side-by-side identical endpoints
- Automated benchmark runner
- Historical trend tracking

## Priority
🟠 **High Priority**"

# =============================================================================
# HIGH PRIORITY (🟠) - Enterprise Features
# =============================================================================

gh issue create --repo "$REPO" \
  --title "feat: i18n support - message translation" \
  --label "enhancement,priority: high" \
  --body "## Executive Summary

Internationalization (i18n) support for message translation enables Armature applications to serve global audiences with localized content.

### Business Impact
- **Market**: Enable applications for non-English markets
- **Enterprise**: Required for many enterprise deployments
- **DX**: Familiar patterns from other frameworks

### Implementation
1. Create \`armature-i18n\` crate
2. Support common formats (Fluent, gettext)
3. Middleware for locale detection
4. Template integration

## Priority
🟠 **High Priority** - Enterprise adoption"

gh issue create --repo "$REPO" \
  --title "feat: Locale detection from Accept-Language" \
  --label "enhancement,priority: high" \
  --body "## Executive Summary

Automatically detect user's preferred language from the \`Accept-Language\` HTTP header and make it available to handlers for localization.

### Implementation
1. Parse Accept-Language header
2. Match against supported locales
3. Provide extractor: \`Locale\`
4. Fallback to default locale

## Priority
🟠 **High Priority** - Enterprise adoption"

echo ""
echo "✅ Done! Created all high-priority issues."
echo ""
echo "View issues at: https://github.com/$REPO/issues"
