# Changelog — `armature-metrics`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The Prometheus `path` label is taken from `path_only()`. The raw target went in, so `/login?token=…` became a label value and every unique query burned a cardinality slot.
- `Summary::quantile` reuses a scratch buffer instead of cloning the whole observation window per call.

### Fixed

- The Prometheus path label is taken from `HttpRequest::path_only`, not the raw request target. `/login?token=SECRET` was published as a label value on `/metrics`, and every distinct query string burned a slot against the cardinality cap.

### Changed

- `Summary::quantile` and `Collector::collect` reuse a scratch buffer instead of allocating a copy of the whole observation window (up to `max_size` values) on every call.

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- Test fixtures build request bodies as `Bytes`.
