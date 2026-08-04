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

Each crate owns its own fuzz targets in `<crate>/fuzz`, so `cargo fuzz` is run
from the crate directory rather than from a single workspace-wide fuzz crate:

```bash
cd armature-core
cargo +nightly fuzz run routing
```

### Run for a Limited Time

```bash
cargo +nightly fuzz run routing -- -max_total_time=60
```

### List What a Crate Has

```bash
cd armature-h1
cargo +nightly fuzz list
```

---

## Available Fuzz Targets

| Crate | Target | Covers |
|-------|--------|--------|
| `armature-core` | `http_request` | Request construction and accessors |
| `armature-core` | `http_response` | Response building |
| `armature-core` | `routing` | Route registration *and* matching, patterns chosen by the fuzzer |
| `armature-core` | `json` | JSON round-tripping |
| `armature-core` | `url_parsing` | Request-line and URI splitting |
| `armature-core` | `headers` | Header-name validation and parsing |
| `armature-core` | `query_params` | Query-string parsing and percent-decoding |
| `armature-core` | `path_params` | Path-parameter extraction against fixed routes |
| `armature-h1` | `parse_head` | Message-head parsing |
| `armature-h1` | `chunked` | Chunked decoding, including split-invariance |
| `armature-h1` | `framing_differential` | Framing decisions compared against hyper |
| `armature-i18n` | `accept_language` | `Accept-Language` negotiation |
| `armature-i18n` | `locale_tag` | Locale-tag parsing, asserting round-trip idempotence |
| `armature-webhooks` | `signature_verify` | HMAC verification: no forgery accepted, no genuine signature rejected |
| `armature-config` | `config_parse` | JSON/TOML/`.env` parsing, each driven with the others' bytes |
| `armature-jwt` | `token_verify` | Token verification: no unissued token accepted |
| `armature-validation` | `validators` | Validators cross-checked against independent implementations |

A target is worth adding where a crate parses or authenticates something it did
not produce. Most crates do neither and have no fuzz directory; an empty
harness would only suggest coverage that does not exist.

---

## Running Fuzz Tests

### Basic Usage

```bash
cd armature-core

# Run specific target
cargo +nightly fuzz run routing

# Run with more parallelism
cargo +nightly fuzz run routing -- -jobs=4 -workers=4

# Run with coverage report
cargo +nightly fuzz coverage routing
```

### Common Options

```bash
# Limit memory usage (MB)
cargo +nightly fuzz run routing -- -rss_limit_mb=2048

# Limit input size (bytes)
cargo +nightly fuzz run routing -- -max_len=4096

# Set random seed for reproducibility
cargo +nightly fuzz run routing -- -seed=12345

# Run for limited iterations
cargo +nightly fuzz run routing -- -runs=10000

# Run for limited time (seconds)
cargo +nightly fuzz run routing -- -max_total_time=300
```

### Running All Targets

```bash
#!/bin/bash
# Run every fuzz target in the repo for 60 seconds each.
set -euo pipefail

for manifest in */fuzz/Cargo.toml; do
    crate="$(dirname "$(dirname "$manifest")")"
    (cd "$crate" && cargo +nightly fuzz list) | while read -r target; do
        echo "== $crate/$target"
        (cd "$crate" && cargo +nightly fuzz run "$target" -- -max_total_time=60)
    done
done
```

---

## Corpus Management

### Seed Corpus

Create initial test cases in `<crate>/fuzz/corpus/<target>/`:

```bash
mkdir -p armature-h1/fuzz/corpus/parse_head

# Add seed files
printf 'GET /api/users HTTP/1.1\r\n\r\n' > armature-h1/fuzz/corpus/parse_head/simple_get
printf 'POST /api/users HTTP/1.1\r\nContent-Length: 15\r\n\r\n{"name":"test"}' > armature-h1/fuzz/corpus/parse_head/post_json
```

### Minimizing Corpus

After fuzzing, minimize the corpus to remove redundant inputs:

```bash
cargo +nightly fuzz cmin parse_head
```

### Sharing Corpus

The corpus is deliberately not version controlled — the root `.gitignore`
excludes `**/fuzz/corpus/`, `**/fuzz/artifacts/` and `**/fuzz/coverage/`,
because a corpus grows without bound and a crash artifact worth keeping belongs
in a test rather than in a directory of opaque binary blobs. Every CI run
therefore starts from an empty corpus and rediscovers coverage from scratch,
which is what the 60-second smoke budget is sized for. A corpus you build
locally is yours to keep locally; promote anything interesting it finds into a
regression test in the owning crate.

---

## CI Integration

### GitHub Actions

Fuzzing already runs in CI: the `fuzz-smoke` job in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) fans out one matrix
entry per `{ crate, target }` pair, lints that crate's `fuzz` workspace with
`cargo clippy --all-targets -- -D warnings`, runs the target for 60 seconds and
uploads `<crate>/fuzz/artifacts/` on failure so the crashing input survives the
runner.

Sixty seconds is a regression gate, not a campaign. On pull requests each entry
runs only when the diff touches the code compiled into it — the owning crate,
the shared `armature-core`/`armature-log` roots, the workspace manifest, or the
workflow itself — and runs unconditionally on pushes, on the nightly schedule,
and on PRs into `main` or `release/**`.

Adding a target means adding a line to that matrix; see
[Writing New Fuzz Targets](#writing-new-fuzz-targets).

### OSS-Fuzz Integration

Armature is compatible with [OSS-Fuzz](https://github.com/google/oss-fuzz). See the OSS-Fuzz documentation for continuous fuzzing on Google's infrastructure.

---

## Writing New Fuzz Targets

### 1. Add Target to `<crate>/fuzz/Cargo.toml`

The bin name is the target name `cargo fuzz run` takes, and it must match the
file stem — no `fuzz_` prefix; every existing target is named bare.

```toml
[[bin]]
name = "new_target"
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

### 3. Register It in CI

A target nothing runs is a target nothing catches. Add one line to the
`fuzz-smoke` matrix in `.github/workflows/ci.yml`:

```yaml
          - { crate: armature-core, target: new_target }
```

### 4. Add It to the Target Table

Add a row to [Available Fuzz Targets](#available-fuzz-targets) above, saying
what the target covers — the table is how anyone finds out the surface is
already fuzzed before writing a second harness for it.

### 5. Using Arbitrary

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
2. Email quinn.josephr@proton.me with:
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

# Run one target (from the crate that owns it)
cd armature-core && cargo +nightly fuzz run routing

# Coverage
cargo +nightly fuzz coverage routing

# Minimize corpus (from armature-h1, which owns parse_head)
cargo +nightly fuzz cmin parse_head
```

To run every target in the repo for 60 seconds each, use the script under
[Running All Targets](#running-all-targets) — it walks `*/fuzz/Cargo.toml` and
asks each crate for its own target list.

### Directory Structure

Fuzzing lives beside the code it exercises, one `fuzz/` per crate:

```
armature-core/
├── src/
└── fuzz/
    ├── Cargo.toml       # Its own [workspace]; nightly-only, so the root
    │                    # workspace must not reach it
    ├── fuzz_targets/
    │   ├── routing.rs
    │   ├── headers.rs
    │   └── ...
    ├── corpus/          # Seed and discovered inputs (gitignored)
    │   └── routing/
    └── artifacts/       # Crash reproductions (gitignored)
```

Each fuzz crate declares its own `[workspace]` because `libfuzzer-sys` links a
sanitizer runtime and builds only on nightly — without that, a plain
`cargo build` at the repo root would try to compile it and fail.

---

**Happy fuzzing!** 🐛🔍

