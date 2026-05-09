# Fuzzing Guide

Guide to fuzz testing Armature for security vulnerabilities and robustness.

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Available Fuzz Targets](#available-fuzz-targets)
- [Running Fuzz Tests](#running-fuzz-tests)
- [Corpus Management](#corpus-management)
- [CI Integration](#ci-integration)
- [Writing New Fuzz Targets](#writing-new-fuzz-targets)
- [Best Practices](#best-practices)

---

## Overview

Armature includes comprehensive fuzz testing using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) with libFuzzer. Fuzzing helps discover:

- **Panics**: Unexpected crashes from malformed input
- **Hangs**: Infinite loops or excessive computation
- **Memory issues**: Buffer overflows, use-after-free
- **Logic errors**: Incorrect behavior with edge cases

---

## Quick Start

### Install cargo-fuzz

```bash
cargo install cargo-fuzz
```

### Run a Fuzz Target

```bash
cd fuzz
cargo +nightly fuzz run fuzz_http_request
```

### Run for a Limited Time

```bash
cargo +nightly fuzz run fuzz_http_request -- -max_total_time=60
```

---

## Available Fuzz Targets

| Target | Description | Priority |
|--------|-------------|----------|
| `fuzz_http_request` | HTTP request parsing | Critical |
| `fuzz_http_response` | HTTP response building | High |
| `fuzz_routing` | Route matching | Critical |
| `fuzz_json` | JSON parsing/serialization | High |
| `fuzz_url_parsing` | URL/path parsing | High |
| `fuzz_headers` | HTTP header parsing | High |
| `fuzz_query_params` | Query string parsing | Medium |
| `fuzz_path_params` | Path parameter extraction | Medium |

---

## Running Fuzz Tests

### Basic Usage

```bash
cd fuzz

# Run specific target
cargo +nightly fuzz run fuzz_http_request

# Run with more parallelism
cargo +nightly fuzz run fuzz_http_request -- -jobs=4 -workers=4

# Run with coverage report
cargo +nightly fuzz coverage fuzz_http_request
```

### Common Options

```bash
# Limit memory usage (MB)
cargo +nightly fuzz run fuzz_http_request -- -rss_limit_mb=2048

# Limit input size (bytes)
cargo +nightly fuzz run fuzz_http_request -- -max_len=4096

# Set random seed for reproducibility
cargo +nightly fuzz run fuzz_http_request -- -seed=12345

# Run for limited iterations
cargo +nightly fuzz run fuzz_http_request -- -runs=10000

# Run for limited time (seconds)
cargo +nightly fuzz run fuzz_http_request -- -max_total_time=300
```

### Running All Targets

```bash
#!/bin/bash
# Run all fuzz targets for 60 seconds each

TARGETS=(
    fuzz_http_request
    fuzz_http_response
    fuzz_routing
    fuzz_json
    fuzz_url_parsing
    fuzz_headers
    fuzz_query_params
    fuzz_path_params
)

for target in "${TARGETS[@]}"; do
    echo "Fuzzing $target..."
    cargo +nightly fuzz run "$target" -- -max_total_time=60
done
```

---

## Corpus Management

### Seed Corpus

Create initial test cases in `armature-fuzz/corpus/<target>/`:

```bash
mkdir -p armature-fuzz/corpus/fuzz_http_request

# Add seed files
echo 'GET /api/users HTTP/1.1' > armature-fuzz/corpus/fuzz_http_request/simple_get
echo 'POST /api/users HTTP/1.1\nContent-Type: application/json\n\n{"name":"test"}' > armature-fuzz/corpus/fuzz_http_request/post_json
```

### Minimizing Corpus

After fuzzing, minimize the corpus to remove redundant inputs:

```bash
cargo +nightly fuzz cmin fuzz_http_request
```

### Sharing Corpus

The corpus directory can be committed to version control:

```bash
git add armature-fuzz/corpus/
git commit -m "Add fuzz corpus"
```

---

## CI Integration

### GitHub Actions

```yaml
name: Fuzz Tests

on:
  schedule:
    - cron: '0 0 * * 0'  # Weekly
  workflow_dispatch:

jobs:
  fuzz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust nightly
        uses: dtolnay/rust-action@nightly

      - name: Install cargo-fuzz
        run: cargo install cargo-fuzz

      - name: Run fuzz tests
        run: |
          cd fuzz
          for target in fuzz_http_request fuzz_routing fuzz_json; do
            cargo +nightly fuzz run "$target" -- -max_total_time=300
          done

      - name: Upload crash artifacts
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: fuzz-crashes
          path: armature-fuzz/artifacts/
```

### OSS-Fuzz Integration

Armature is compatible with [OSS-Fuzz](https://github.com/google/oss-fuzz). See the OSS-Fuzz documentation for continuous fuzzing on Google's infrastructure.

---

## Writing New Fuzz Targets

### 1. Add Target to Cargo.toml

```toml
[[bin]]
name = "fuzz_new_target"
path = "fuzz_targets/new_target.rs"
test = false
doc = false
bench = false
```

### 2. Create the Fuzz Target

```rust
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

/// Input structure for fuzzing
#[derive(Debug, Arbitrary)]
struct FuzzInput {
    field1: String,
    field2: Vec<u8>,
    field3: Option<u32>,
}

fuzz_target!(|data: FuzzInput| {
    // Limit input sizes to prevent OOM
    if data.field1.len() > 10000 || data.field2.len() > 100000 {
        return;
    }

    // Call the code under test
    // Should NOT panic for any valid Arbitrary input
    let result = your_function(&data.field1, &data.field2);

    // Optionally verify invariants
    if let Ok(output) = result {
        assert!(output.len() <= data.field1.len() * 2);
    }
});
```

### 3. Using Arbitrary

The `Arbitrary` derive macro generates random test inputs:

```rust
use arbitrary::Arbitrary;

#[derive(Debug, Arbitrary)]
struct ComplexInput {
    // Primitives
    number: u32,
    text: String,
    bytes: Vec<u8>,

    // Optionals
    maybe: Option<String>,

    // Enums
    choice: Choice,

    // Nested
    nested: Box<NestedInput>,
}

#[derive(Debug, Arbitrary)]
enum Choice {
    A,
    B(String),
    C { value: i32 },
}
```

---

## Best Practices

### 1. Limit Input Size

```rust
fuzz_target!(|data: FuzzInput| {
    // Prevent OOM/timeouts
    if data.bytes.len() > 1_000_000 {
        return;
    }
    // ...
});
```

### 2. Handle Errors Gracefully

```rust
fuzz_target!(|data: FuzzInput| {
    // Code should handle all inputs without panicking
    // Errors are expected and OK
    let _ = parse_input(&data.raw);

    // DON'T use unwrap() - this will cause false positives
    // BAD: let result = parse_input(&data.raw).unwrap();
});
```

### 3. Test Invariants

```rust
fuzz_target!(|data: FuzzInput| {
    // Verify round-trip
    if let Ok(parsed) = parse(&data.raw) {
        let serialized = serialize(&parsed);
        let reparsed = parse(&serialized);
        assert_eq!(parsed, reparsed.unwrap());
    }
});
```

### 4. Focus on Attack Surfaces

Prioritize fuzzing:
- Input parsers (HTTP, JSON, URLs)
- Routing/path matching
- Authentication/authorization
- Serialization/deserialization
- Memory-intensive operations

### 5. Regular Fuzzing

- Run fuzz tests weekly in CI
- Fuzz after major changes to parsing code
- Keep corpus updated with interesting inputs

---

## Reporting Vulnerabilities

If fuzzing discovers a security vulnerability:

1. **Do not** create a public GitHub issue
2. Email security@pegasusheavy.com with:
   - Description of the issue
   - Reproduction steps (crash input)
   - Potential impact assessment
3. We will respond within 48 hours

---

## Summary

### Quick Commands

```bash
# Install
cargo install cargo-fuzz

# Run
cd fuzz && cargo +nightly fuzz run fuzz_http_request

# Run all (60s each)
for t in fuzz_*; do cargo +nightly fuzz run "$t" -- -max_total_time=60; done

# Coverage
cargo +nightly fuzz coverage fuzz_http_request

# Minimize corpus
cargo +nightly fuzz cmin fuzz_http_request
```

### Directory Structure

```
armature-fuzz/
├── Cargo.toml           # Fuzz crate manifest
├── fuzz_targets/        # Fuzz target source files
│   ├── http_request.rs
│   ├── routing.rs
│   └── ...
├── corpus/              # Seed inputs (version controlled)
│   └── fuzz_http_request/
└── artifacts/           # Crash reproductions (gitignored)
```

---

**Happy fuzzing!** 🐛🔍

