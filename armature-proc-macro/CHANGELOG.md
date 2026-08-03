# Changelog — `armature-proc-macro`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** attribute macros reject what they used to discard. `#[body_limit(512kb)]` meant 512 *bytes* (rustc lexes it as a suffixed integer and the suffix was dropped), `#[timeout(hours = 2)]` meant two seconds, and unknown `#[module]`/`#[catch]` keys registered nothing — all silently.
- The parameter extractors work through `#[routes]`. `#[body]`, `#[param("id")]`, `#[query("page")]` and `#[header]` were documented but unreachable: route attributes were stripped before the extraction codegen could run, so a handler written as documented failed to compile.
- A handler carrying several route attributes registers all of them; every one after the first was dropped without a diagnostic.

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- The `Query` derive and the `#[query]` route-parameter extractor deserialize the raw query string instead of re-encoding already-decoded pairs, so a value containing a literal `&`, `=` or `%` round-trips as sent.
