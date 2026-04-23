# Memory Profiling Guide

Comprehensive guide to memory profiling and leak detection in Armature applications.

## Overview

Memory leaks in long-running server applications can cause degraded performance and eventually crash the server. This guide covers tools and techniques for detecting and fixing memory leaks.

## Features

- ✅ DHAT heap profiler integration
- ✅ Valgrind support for detailed leak detection
- ✅ Heaptrack for allocation tracking
- ✅ Massif for memory usage over time
- ✅ Memory benchmarks for allocation patterns
- ✅ Custom allocation tracking utilities

## Quick Start

### Using DHAT (Recommended for Rust)

DHAT is a Rust-native heap profiler that provides detailed allocation information:

```bash
# Build with memory profiling enabled
cargo build --example memory_profile_server --release --features memory-profiling

# Run the server
./target/release/examples/memory_profile_server

# Generate load (in another terminal)
curl http://localhost:3000/health
# ... more requests ...

# Press Ctrl+C to stop and generate report
# Output: dhat-heap.json
```

View the report online at [dhat-viewer](https://nnethercote.github.io/dh_view/dh_view.html).

### Using the Profiling Script

The project includes a comprehensive profiling script:

```bash
# DHAT profiling (default)
./scripts/memory-profile.sh dhat 30

# Valgrind leak detection
./scripts/memory-profile.sh valgrind 30

# Massif heap profiler
./scripts/memory-profile.sh massif 30

# Heaptrack (best for detailed analysis)
./scripts/memory-profile.sh heaptrack 30
```

## Tools

### DHAT

DHAT (Dynamic Heap Analysis Tool) is built specifically for Rust and provides:

- Total bytes allocated
- Peak memory usage
- Allocation hotspots
- Short-lived allocations (potential optimization targets)

**Setup:**

The project is already configured for DHAT. Just use the `memory-profiling` feature:

```rust
// This is already in examples/memory_profile_server.rs
#[cfg(feature = "memory-profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "memory-profiling")]
    let _profiler = dhat::Profiler::new_heap();

    // Your application code
}
```

**Interpreting Results:**

Key metrics to look for:
- **Total bytes**: Overall memory allocated
- **Max bytes at once**: Peak memory usage
- **Blocks allocated**: Number of allocations
- **Short-lived**: Allocations freed quickly (optimization opportunities)

### Valgrind

Valgrind provides comprehensive leak detection but is slower:

```bash
# Full leak check
valgrind --leak-check=full \
         --show-leak-kinds=all \
         --track-origins=yes \
         ./target/release/examples/benchmark_server
```

**Leak Categories:**
- **Definitely lost**: Memory that was never freed
- **Indirectly lost**: Lost due to pointer chains
- **Possibly lost**: Ambiguous (may or may not be a leak)
- **Still reachable**: Memory still accessible at exit (not a leak)

### Massif

Massif tracks heap usage over time:

```bash
valgrind --tool=massif \
         --massif-out-file=massif.out \
         ./target/release/examples/benchmark_server

# View results
ms_print massif.out
```

### Heaptrack

Heaptrack provides the most detailed analysis with a GUI:

```bash
# Record
heaptrack ./target/release/examples/benchmark_server

# Analyze
heaptrack_gui heaptrack.benchmark_server.*.zst
```

## Memory Benchmarks

Run the memory allocation benchmarks:

```bash
# All memory benchmarks
cargo bench --bench memory_benchmarks

# Specific benchmark group
cargo bench --bench memory_benchmarks string_allocations
cargo bench --bench memory_benchmarks vec_allocations
cargo bench --bench memory_benchmarks hashmap_allocations
cargo bench --bench memory_benchmarks smart_pointers
cargo bench --bench memory_benchmarks request_response
cargo bench --bench memory_benchmarks object_pool
cargo bench --bench memory_benchmarks leak_patterns
cargo bench --bench memory_benchmarks allocation_sizes
cargo bench --bench memory_benchmarks drop_timing
```

## Common Memory Leak Patterns

### 1. Unbounded Caches

❌ **Problem:**

```rust
// Cache that grows forever
let mut cache: HashMap<String, Vec<u8>> = HashMap::new();

loop {
    let key = generate_unique_key();
    cache.insert(key, data);  // Never removes old entries!
}
```

✅ **Solution:**

```rust
use std::collections::HashMap;

const MAX_CACHE_SIZE: usize = 1000;

fn cache_insert(cache: &mut HashMap<String, Vec<u8>>, key: String, value: Vec<u8>) {
    // Evict old entries when cache is full
    if cache.len() >= MAX_CACHE_SIZE {
        if let Some(oldest_key) = cache.keys().next().cloned() {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(key, value);
}
```

Or use a proper LRU cache:

```rust
use lru::LruCache;

let mut cache: LruCache<String, Vec<u8>> = LruCache::new(1000.try_into().unwrap());
```

### 2. Forgotten Event Handlers

❌ **Problem:**

```rust
// Subscriptions that are never cleaned up
let mut handlers: Vec<Box<dyn Fn()>> = Vec::new();

loop {
    handlers.push(Box::new(|| println!("handler")));
    // handlers vector grows forever!
}
```

✅ **Solution:**

```rust
// Use weak references or explicit cleanup
use std::sync::Weak;

struct EventEmitter {
    handlers: Vec<Weak<dyn Fn()>>,
}

impl EventEmitter {
    fn cleanup(&mut self) {
        self.handlers.retain(|h| h.upgrade().is_some());
    }
}
```

### 3. Circular References

❌ **Problem:**

```rust
use std::sync::Arc;
use std::cell::RefCell;

struct Node {
    next: RefCell<Option<Arc<Node>>>,
}

// Creates circular reference - neither node is ever freed
let a = Arc::new(Node { next: RefCell::new(None) });
let b = Arc::new(Node { next: RefCell::new(Some(a.clone())) });
*a.next.borrow_mut() = Some(b.clone());
```

✅ **Solution:**

```rust
use std::sync::{Arc, Weak};
use std::cell::RefCell;

struct Node {
    next: RefCell<Option<Weak<Node>>>,  // Use Weak to break cycle
}
```

### 4. Growing Buffers

❌ **Problem:**

```rust
// Buffer that never shrinks
let mut buffer = Vec::new();

loop {
    buffer.extend(large_data);
    buffer.clear();  // Clears contents but keeps capacity!
}
```

✅ **Solution:**

```rust
// Periodically shrink oversized buffers
const MAX_BUFFER_SIZE: usize = 1024 * 1024;  // 1MB

if buffer.capacity() > MAX_BUFFER_SIZE {
    buffer.shrink_to_fit();
}
```

### 5. Thread-Local Storage Leaks

❌ **Problem:**

```rust
thread_local! {
    static DATA: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

// Adding to thread-local without cleanup
DATA.with(|d| d.borrow_mut().push(String::from("data")));
```

✅ **Solution:**

```rust
// Clear thread-local storage periodically
thread_local! {
    static DATA: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

fn reset_thread_local() {
    DATA.with(|d| d.borrow_mut().clear());
}
```

## Armature-Specific Patterns

### Request Arena

Armature uses arena allocation for request-scoped data:

```rust
use armature_core::arena::{with_arena, reset_arena};

// Allocations in arena are batched
with_arena(|arena| {
    let data = arena.alloc_str("temporary data");
    // Process request...
});

// Reset arena after request to free all allocations at once
reset_arena();
```

### Object Pools

Use object pools for frequently allocated objects:

```rust
use armature_core::memory_opt::ObjectPool;

let pool: ObjectPool<HttpRequest> = ObjectPool::new(100);

// Get from pool (reuses existing object)
let req = pool.get();

// Process request...

// Return to pool for reuse
pool.put(req);
```

### Memory Statistics

Monitor memory usage with built-in statistics:

```rust
use armature_core::memory_opt::memory_stats;

let stats = memory_stats();
println!("Headers inline: {}", stats.headers_inline());
println!("Headers heap: {}", stats.headers_heap());
println!("Pool hit ratio: {:.2}%", stats.pool_hit_ratio() * 100.0);
```

## CI Integration

Add memory checks to your CI pipeline:

```yaml
# .github/workflows/memory.yml
name: Memory Checks

on: [push, pull_request]

jobs:
  memory-check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Valgrind
        run: sudo apt-get install -y valgrind

      - name: Build release
        run: cargo build --release --example benchmark_server

      - name: Check for leaks
        run: |
          timeout 30s valgrind --leak-check=full \
            --error-exitcode=1 \
            ./target/release/examples/benchmark_server &
          sleep 5
          curl http://localhost:3000/health
          kill %1
```

## Best Practices

1. **Bound all caches**: Never use unbounded collections for caching
2. **Use arenas for request-scoped data**: Batch deallocations
3. **Profile regularly**: Run memory benchmarks before releases
4. **Monitor production**: Track memory usage over time
5. **Use Weak references**: Break circular reference cycles
6. **Shrink oversized buffers**: Don't let temporary buffers keep memory
7. **Clean up subscriptions**: Remove event handlers when done

## Troubleshooting

### High Memory Usage

1. Run DHAT to identify allocation hotspots
2. Check for unbounded caches
3. Look for large allocations that could use streaming

### Gradual Memory Growth

1. Use Massif to track memory over time
2. Check for growing collections
3. Look for thread-local storage that isn't cleaned

### Memory Spikes

1. Profile with Heaptrack for detailed timeline
2. Identify large temporary allocations
3. Consider object pooling or streaming

## Summary

| Tool | Best For | Speed |
|------|----------|-------|
| DHAT | Rust-specific profiling | Fast |
| Valgrind | Leak detection | Slow |
| Massif | Memory over time | Slow |
| Heaptrack | Detailed GUI analysis | Medium |

**Key Commands:**

```bash
# Quick DHAT profile
./scripts/memory-profile.sh dhat 30

# Memory benchmarks
cargo bench --bench memory_benchmarks

# Detailed leak check
./scripts/memory-profile.sh valgrind 60

# View DHAT results
# Open https://nnethercote.github.io/dh_view/dh_view.html
```

