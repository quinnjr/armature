# Changelog — `armature-app`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- A Rhai before-middleware that raised an error was logged and the request **proceeded** — a failing auth or rate-limit hook admitted every request. Both before- and after-middleware errors now fail the request with 500.

### Changed

- **Breaking (scripts):** the after-middleware hook signature is `|req, res|`, receiving the full outgoing `Response` (status, headers, cookies, body) instead of only the status code. Return a `Response` to replace the outgoing one. This matches `armature_rhai::ScriptMiddleware::call_after` and the documented design.
- `RequestBinding` is built once per request and shared across guards, middleware and the handler instead of being rebuilt (headers, query and a full body copy) at every hop.
