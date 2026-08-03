# Changelog — `armature-testing`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Added

- `Spy` is exported. It was listed as a crate feature but `mod mock` was private, so it was unreachable. Its docs now state plainly that it is a manual recorder: `inner()` returns the wrapped value untouched and calls made through it are not intercepted.
- `Spy::calls()` and `Spy::clear()`.

### Changed

- Load-test workers accumulate latencies in per-worker buffers merged after join, instead of taking a shared `Mutex` per completed request — the measurement path no longer inflates the latencies it records.
- `median`, `p95` and `p99` all use one nearest-rank formula. `median` previously took the upper median while p95/p99 truncated, so the reported quantiles came from two different definitions.

### Changed — `0.2.1` → `0.2.2`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- `TestRequestBuilder` builds its body as `Bytes`.
