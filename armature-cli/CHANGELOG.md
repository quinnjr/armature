# Changelog — `armature-cli`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** unimplemented subcommands exit non-zero and are hidden. `armature deploy` reported success having deployed nothing, and `armature serve` as a container start-command exited cleanly with no server.
- Generated scaffolds compile against the current API; the pipe, exception-filter and ten test templates targeted removed types, and no non-ignored test compiled generated output.
- `armature dev -- <args>` passes arguments through to `cargo run` instead of emitting a `--` before each one and dropping them, which also made behaviour depend on whether `cargo-watch` happened to be installed.

### Changed — `0.4.0` → `0.4.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- Generated project templates use the new `HttpRequest::new` signature.
- **Breaking (scripts):** subcommands that were declared but never implemented now exit non-zero instead of printing "coming soon!" and exiting `0`. This affects `serve`, `deploy`, `upgrade`, `bench`, `lint`, `config show|set|init`, `plugin install|uninstall|new`, and `openapi validate|generate`. They are also hidden from `--help` until they do something, and are no longer listed in the crate-level docs. `armature serve` used as a container start-command and `armature deploy` in CI previously reported success having done nothing.
- Generated code templates no longer emit source that fails to compile against `armature-core`: `req.body = <Vec<u8>>` became `req.set_body(...)` (the body is `Bytes`), `req.params.get(...)`/`req.params.insert(...)` became `req.param(...)`/`req.push_param(...)`, and `HttpRequest::default()` (no such impl) became `HttpRequest::new(method, path)`.

### Fixed

- The generated exception filter put `req.path` — the raw request target — into the error response body, leaking any query string (and anything sensitive in it) back to the caller. It now uses `req.path_only()`.
- `armature dev -- <args>` no longer mangles the extra cargo arguments when `cargo-watch` is installed; they are folded into the `-x run ...` command string as the built-in watcher branch already did.
- Removed the dangling `mod watcher;` declaration left behind when the unused, substring-matching `watcher` module was deleted.
