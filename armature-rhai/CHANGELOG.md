# Changelog — `armature-rhai`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** the script-visible query map is first-wins, matching `QueryView::get`; it was last-wins, so `?tag=a&tag=b` read differently from Rust and Rhai.
- Routes match `path_only()`. Any request with a query string 404'd, and a parameterized route captured the query into the parameter.

### Fixed

- `ScriptRouter::find_route` matched patterns against the raw request target, so any request carrying a query string 404'd (`/users?a=1`) or captured the query into a path param (`/users/42?a=1` gave `id == "42?a=1"`). Matching is now against `HttpRequest::path_only`.
- `request.path` in scripts is the routing path, not the raw target — it no longer carries the query string.

### Changed

- The script-visible query is FIRST-wins for a repeated key, matching `armature_core::QueryView::get`. It was a `HashMap` and therefore LAST-wins, so `?tag=a&tag=b` read `"b"` in a script and `"a"` in Rust for the same request.
- New `request.query_all(name)` returns every value for a repeated key, and `request.has_param(name)` distinguishes a param captured with non-UTF-8 bytes (`param()` returns `()`) from one that was never captured.
- `RequestBinding` holds the body as `Bytes`, so cloning one no longer copies the body.

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- The script-visible `request.query` and `request.params` maps are built from the request target and the captured route spans rather than from the removed `query_params`/`path_params` maps; a path param whose captured bytes are not UTF-8 is now omitted rather than lossily converted.
