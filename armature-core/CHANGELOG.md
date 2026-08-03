# Changelog — `armature-core`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changes at or before `0.6.0` are recorded in the workspace
[`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- Catch-all routes match zero remaining segments again, and a trailing slash no
  longer changes the outcome. `matchit` requires a catch-all to consume at least
  one segment and treats a trailing slash as significant, so `Router` had begun
  answering differently from `OptimizedRouter` — an invariant AGENTS.md states
  and `route_cache`'s precedence test exists to enforce. A differential test now
  drives a pattern/target matrix through both routers and compares which handler
  answered.
- Route-parameter names are interned once at registration. Every captured
  parameter on every request took a process-global mutex, and the comment
  claiming the compiled router already interned at registration was false.
- `interceptor::cache_key` folded the query in twice — once raw via the target,
  once sorted — defeating the canonicalization it documents, so `?a=1&b=2` and
  `?b=2&a=1` were separate entries. `CacheKey::from_request` could also collide
  `?a=1%26b%3D2` with `?a=1&b=2` and serve one request another's body.
- `clone_response` shares the cached body instead of copying it on every hit.
- `HeaderMap` by-name lookups no longer allocate a `HeaderId::Other` per call for
  a custom name, which had made them strictly worse than the `HashMap` they
  replaced.
- Static assets resolve against `path_only()`, so a cache-busting `?v=2` no
  longer 404s, and the serve path no longer makes blocking `canonicalize`,
  `exists` and `is_dir` syscalls on the async executor.
- `param_intern` is hard-capped. `push_param` and `from_parts` let
  request-derived names reach an interner whose own documentation forbids
  exactly that.
- `RouteConstraints::validate` rejects a non-UTF-8 parameter rather than
  skipping the constraint, and `simd_parser` reports truncated input instead of
  fabricating a `GET /`.

### Breaking — `0.7.0` → `0.8.0`

The request and response types are now backed by `Bytes` rather than owned
`String`s and `Vec<u8>`s, and the work the serve path used to do eagerly is done
on demand or not at all.

- `HttpRequest.method` is a `Method` (was `String`). Constructors take
  `impl Into<Method>`, so `HttpRequest::new("GET", …)` and
  `HttpRequest::new("GET".to_string(), …)` both still compile. `method_str()`
  gives a `&str` and `req.method == "GET"` still works. Method tokens are now
  matched case-sensitively per RFC 9110 §9.1, and routing rejects CONNECT and
  TRACE rather than mapping them onto a routable method.
- `HttpRequest.path` is a `ByteStr` (was `String`). It derefs to `str`, so most
  uses are unaffected; `path_str()` is the explicit accessor. It now holds the
  *raw target*, query string included — `path_only()` is what routing matches on.
- `HttpRequest.body` and `HttpResponse.body` are `Bytes` (were `Vec<u8>`), and
  the private `body_bytes` shadow field is gone, so the two can no longer
  disagree about which holds the body. `body_slice()` returns `&[u8]`;
  `body_bytes()`, `set_body_bytes()`, `has_bytes_body()` and `body_ref()` remain
  as forwarders. `HttpResponse::with_capacity`'s capacity argument is now
  ignored: `Bytes` is handed a finished buffer rather than grown in place.
- `HttpRequest.query_params` is removed. `query()` returns a lazily parsed
  `QueryView<'_>`, `query_param(name)` replaces the old `query(name)`,
  `query_string()` gives the raw query and `push_query_param` appends an encoded
  pair. Repeated keys are all preserved in client order rather than collapsing
  to the last. `from_parts` still accepts a `query_params` argument and ignores
  it.
- `HttpRequest.path_params` is `RouteParams = SmallVec<[(&'static str, Bytes); 4]>`.
  `param(name)` returns `Option<&str>` (was `Option<&String>`), `param_bytes`
  returns the raw span, and `push_param`/`set_params` replace map insertion.
  `RouteParamsExt` adds `get_str`/`get_bytes` for by-name lookup.
- `HeaderMap` stores `(HeaderId, Bytes)`. `get` returns `Option<&str>` and
  yields `None` for a value that is not UTF-8 — `get_bytes` returns those.
  `remove` returns `Option<Bytes>`. Custom header names are lowercased at insert,
  so `to_hash_map()`, `keys()` and `iter()` report canonical names.
- `Router::match_route` returns `Option<(BoxedHandler, RouteParams)>` and
  `RouteConstraints::validate` takes `&RouteParams`.
- `zero_cost::Method` and `zero_cost::RequestPath` wrap `Method`/`ByteStr`;
  `extractors::RawBody` wraps `Bytes`; `extractors::Method` is renamed
  `MethodExtractor` to avoid colliding with the re-exported `Method`.

### Added

- `armature-core` re-exports `Method`, `ByteStr`, `HeaderId` and `header_id` from
  the new `armature-h1` crate, plus `bytes::Bytes`, so downstream crates can name
  the body type without taking their own `bytes` dependency.
- `query` module (`QueryView`) and `param_intern` module (leak-once interning of
  route parameter names, bounded by the route table rather than by traffic).

### Performance

- Dispatch goes through one `matchit` tree per routable method instead of a
  linear scan with a per-candidate method string comparison. Registration order
  still decides precedence: patterns `matchit` rejects fall back to the scan, and
  a static route an earlier same-method route already answers is left out of the
  tree, so `matchit`'s static-over-parameter preference cannot override
  first-registered-wins.
- `Extensions` is a `SmallVec` with eight inline slots instead of a `HashMap`.
- The query string is not parsed or percent-decoded unless a handler reads it.
- The serve path no longer copies the request body out of hyper's `Bytes`, and
  header values are inserted without a `String` allocation per value.
- Serving a response-cache hit clones the stored `Bytes` instead of copying it.

### Migration

`tokio::spawn` inside handlers is unaffected by this release; the `Send`-bound
change is a later one.

### Removed

- **Breaking — `0.6.0` → `0.7.0`:** removed the `tower_compat` module and the
  `tower`/`tower-service` dependencies. The module existed solely for Tower
  interop — `ArmatureService`, `HyperServiceAdapter`, `ServiceFactory` and
  `ArmatureLayerService` implemented `tower_service::Service`, `ArmatureLayer`
  implemented `tower::Layer`, and `tower_stats()`/`TowerStats` counted
  conversions — which pulled the whole `tower` façade (plus its unused `util`
  feature) into every `armature-core` build for a single trait impl. The
  `http`-crate conversion traits that lived alongside it (`IntoHttpRequest`,
  `FromHttpRequest`, `IntoHttpResponse`, `HttpResponseFromHttp`, `HeaderMapExt`,
  `ArmatureHeaderMapExt`) are removed with it; they had no consumers outside the
  module. No in-tree crate, example, or template referenced any of it.
