# Changelog — `armature-lambda`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- The captured method and body are read through the request's new accessors.
- **Breaking:** `impl_request_handler!` is renamed to `impl_lambda_handler!`. The old name expanded to `$crate::runtime::RequestHandler`, a path in a private module that never resolved outside this crate, and to a bare `async_trait::async_trait` that only resolved in crates depending on `async-trait` under that exact name. The macro now names both through `$crate`, and `RequestHandler` and `async_trait` are re-exported at the crate root.
- **Breaking:** `LambdaRequest::headers` and `LambdaResponse::headers` are `Vec<(String, String)>` instead of `HashMap<String, String>`, so repeated field names survive in both directions — most importantly a handler can now emit more than one `Set-Cookie`. `LambdaResponse::header` appends; the new `set_header` replaces. New readers: `header_values` on both types and `header_value` on the response. A request header whose value is not UTF-8 is still dropped, but now logs a warning instead of vanishing silently.

### Documentation

- The crate docs, `RequestHandler`/`LambdaRuntime` rustdoc and the README no longer claim that an Armature `Application` becomes a handler on its own. No `HttpRequest`/`HttpResponse` conversion exists here; `impl_lambda_handler!` targets a user-supplied inherent `handle_request` method, and its required shape is now documented.
