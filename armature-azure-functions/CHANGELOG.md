# Changelog — `armature-azure-functions`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Changed — `0.2.0` → `0.2.1`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).

### Fixed

- The handler macro forwarded only the method, path and body, so an application could never see `Authorization`, `Content-Type`, cookies, the query string, route parameters or the invocation context. It now forwards the whole `FunctionRequest`, matching `armature-lambda`.
- Query parsing dropped keys that had no `=`, kept only the last value for a repeated key, and turned a component that failed to percent-decode into an empty string. Wire order and repeats are now preserved, `?flag` keeps its key with an empty value, and an undecodable component is passed through verbatim with a `warn`-level log.
- `timeout_seconds` was applied twice — once to body ingestion and again to the handler — so a request could occupy twice the configured budget. Both stages now measure against a single deadline established when the request arrives.

### Breaking

- `impl_request_handler!` is renamed to `impl_azure_function_handler!`. The old name expanded to `$crate::runtime::RequestHandler`, a path in a private module that never resolved outside this crate, and to a bare `async_trait::async_trait` that only resolved in crates depending on `async-trait` under that exact name. The macro now names both through `$crate`, and `RequestHandler` and `async_trait` are re-exported at the crate root.
- `FunctionRequest::headers`, `FunctionRequest::query` and `FunctionResponse::headers` are `Vec<(String, String)>` instead of `HashMap<String, String>`, so repeated names survive in wire order — most importantly a handler can now emit more than one `Set-Cookie`. `FunctionResponse::header` appends; the new `set_header` replaces. New readers: `header_values` on both types, `header_value` on the response, and `query_params` on the request. This also changes the JSON shape produced by `FunctionRequest`'s serde impl and `FunctionResponse::to_json`; neither was ever the Azure custom-handler envelope, and `to_json`'s documentation now says so.
- `AzureFunctionsRuntime::handle_with_deadline` is added alongside `handle`, which now derives its deadline from the config and delegates.

### Documentation

- The crate docs, `RequestHandler`/`AzureFunctionsRuntime` rustdoc and the README no longer claim that an Armature `Application` becomes a handler on its own. No `HttpRequest`/`HttpResponse` conversion exists here; `impl_azure_function_handler!` targets a user-supplied inherent `handle_request` method, and its required shape is now documented.
