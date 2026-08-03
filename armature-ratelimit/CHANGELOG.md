# Changelog — `armature-ratelimit`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** an unkeyed request is now governed by an explicit `UnkeyedRequestPolicy` and counted in `unkeyed_allow_count`. The default middleware could never extract a key, so it allowed every request with only a log line and no counter — an inert security control.
- **Breaking:** `KeyExtractor::Custom` is removed. It carried a description string and no function, so selecting it silently disabled rate limiting through the unkeyed path; `KeyExtractorFn`/`KeyExtractorBuilder` are the working mechanism.
- **Breaking:** `MemoryStore::new()` defaults to a bounded `max_keys` and a shorter idle TTL. The documented protection against a key-rotating client did not hold under the previous unbounded default.
- Eviction at the cap is amortized over a batch instead of a full scan per new key, so the attacker the cap targets can no longer convert memory pressure into CPU pressure.
- Rate-limit keys use `path_only()`, closing a bucket-minting bypass via query strings.

### Changed — `0.2.1` → `0.2.2`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
