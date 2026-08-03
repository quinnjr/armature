# Changelog — `armature-opentelemetry`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** span and metric attributes use stable semantic conventions (`http.request.method`, `url.path`, `http.response.status_code`); dashboards keyed on the retired pre-1.0 names need updating.
- `http.route` no longer carries the raw target. Every distinct URL — query string included — minted a new metric time series, which is unbounded cardinality on a low-cardinality attribute.

### Fixed

- `http.route` is the query-less path rather than the raw request target. OTel defines `http.route` as a low-cardinality route template, so every distinct URL was minting its own time series.

### Changed

- **Breaking (telemetry):** span and metric attributes use the stable OTel HTTP semantic conventions instead of the retired pre-1.0 names: `http.method` → `http.request.method`, `http.target` → `url.path`, `http.status_code` → `http.response.status_code`, `http.scheme` → `url.scheme`, `http.host` → `server.address`, `http.user_agent` → `user_agent.original`, `http.response_content_length` → `http.response.body.size`. Dashboards and alerts keyed on the old names must be updated.
- Span names use the query-less path, for the same cardinality reason.

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- `http.method` and `http.target` are read through the request's new accessors; `http.target` now carries the query string, which it previously dropped.
