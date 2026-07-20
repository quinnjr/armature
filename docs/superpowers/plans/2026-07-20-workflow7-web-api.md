# Workflow 7 — Web & API: Implementation Plan

**Spec:** `docs/superpowers/specs/2026-07-20-workflow7-web-api-design.md`
**Findings:** `TODO.md` (45: 7C/25W/13I)
**Branch:** `feature/wf7-web-api` → PR to `develop`

## Execution model

Six disjoint crates → one edit-only implementation agent per crate, in parallel (the WF6 pattern).
No agent touches CHANGELOG, crate `version`, `TODO.md`, or git; the coordinator does the central
gate, semver bumps, CHANGELOG, TODO tick, and commit. Priority order (governs review depth):
**http-client → grpc → graphql-client → graphql → websocket → openapi.**

## Tasks

### T1 — armature-http-client (2C/4W/1I)
1. **`execute_with_retry`**: capture the serialized request body (and per-request timeout) before
   the first send, rebuild a fresh `reqwest::Request` with body+timeout for each retry attempt
   instead of relying on `clone_request`'s method/URL/headers-only copy.
2. **`CircuitBreaker`**: gate `record_failure`/`record_success` on response status
   (`status.is_server_error()` / existing `should_retry_status`), not on `reqwest::Result::Ok` —
   a 5xx/429 response must count as a failure.
3. Wire the interceptor/middleware system into `execute()`/`send()` (currently exported, never
   invoked), or downgrade the lib.rs claim to "standalone utilities" if wiring is deferred —
   decide based on how contained the wiring turns out to be.
4. `RateLimitInterceptor::intercept` sleeps for the parsed `Retry-After` (or returns a typed
   signal) instead of only logging.
5. `Response::from_reqwest` returns `Result<Self>`, propagating a body-read error instead of
   `unwrap_or_default`.
6. `RequestBuilder::json`/`form` surface a serialization failure to the caller.
7. Tests: retried request carries the original body; a 5xx/429 sequence opens the breaker; a
   body-read failure surfaces as an error, not an empty 2xx.

### T2 — armature-grpc (4C/6W/4I)
1. **`AuthInterceptor::intercept`**: split into a client path (`add_auth`) and a server path
   (`validate`), so a server-side use of it actually authenticates.
2. **Middleware**: implement `GrpcMiddleware`/Tower `Layer` for `Timeout`/`RateLimit`/
   `ConcurrencyLimit`/`LoadShedding`/`Retry` and wire `MiddlewareLayer` to apply them.
3. **TLS**: enable tonic's `tls-ring` feature, wire config-driven `ClientTlsConfig`/
   `ServerTlsConfig`, or drop the README's rustls claim if deferred — decide up front, not
   per-finding.
4. **Reflection**: when `enable_reflection`, build and register a real `tonic_reflection` service.
5. Health check: keep the `HealthReporter` and call `set_serving` for registered services.
6. Apply `max_recv_message_size`/`max_send_message_size` via the service's decoding/encoding
   limit setters; apply `tcp_keepalive` on both server and client.
7. Wire client `retry_enabled`/`max_retry_attempts` into a real retry (reuse the middleware/layer
   design from item 2 if practical).
8. Fix README/lib.rs server Quick Start examples; drop or clarify the "Code Generation" claim
   (`tonic-build` is a build-dep, no `build.rs` exists); `bind_address` returns a parse error
   instead of panicking.
9. Tests: an unauthenticated request against the server-side interceptor is rejected; a
   registered service reports `SERVING` via the health client; each middleware measurably alters
   behavior (timeout times out, rate-limit rejects over the limit).

### T3 — armature-graphql-client (1C/5W/2I)
1. **`SubscriptionBuilder::header`**: build a `tokio_tungstenite` `ClientRequestBuilder`/
   `http::Request` from the WS endpoint, apply every stored `(name, value)`, pass it to
   `connect_async` instead of the bare URL string.
2. Implement retry (`retry_enabled`/`max_retries`, retry on transport/5xx) in
   `execute_request`/`batch`.
3. Decide per the plan: implement batching (`batching`/`max_batch_size`/`batch_delay`) or drop
   those config fields/builders and reword the README — batching is lower-value than retry and
   may be better reconciled as a doc fix.
4. Drop the "Type Generation" and "Apollo Federation" feature claims from `lib.rs`/README (no
   backing code for either).
5. Respond to a server `Ping` with `Pong` in `execute_subscription`.
6. Fix the README Quick Start (`Client::new`/`gql!`/`.variable()`/`.execute()` don't exist).
7. Tests: a subscription's custom header is present in the recorded WS handshake request (local
   inspection server); retry actually retries on a 500 then succeeds; a server ping receives a
   pong.

### T4 — armature-graphql (0C/4W/3I)
1. Add a `configure(SchemaBuilder) -> SchemaBuilder` path applying `max_depth`/`max_complexity`/
   `enable_introspection`/`enable_validation`/`enable_tracing` from `GraphQLConfig`, wired into
   every schema-building entry point — `production()`'s introspection-disabled claim must become
   true.
2. Fix `create_merged_schema` (implement real merging over a `MergedObject` root, or remove the
   function rather than shipping one that panics on `.build()`).
3. Either implement `FederationGateway::listen`, or rewrite the module doc to describe only the
   query-forwarding client that exists.
4. Rename/redocument `compose_supergraph`/`ComposedSchema.sdl` to describe SDL concatenation, not
   composition (unless real composition is judged in-scope during execution).
5. `resolve_entities`'s hardcoded query selects real fields, not just `__typename`; apply
   `SubgraphConfig::with_timeout` to the per-subgraph request client.
6. Tests: `production()`'s schema rejects an introspection query and a depth-exceeding query;
   `create_merged_schema` either builds successfully or is gone; a federation-enabled subgraph's
   SDL contains the expected federation directives.

### T5 — armature-websocket (0C/4W/1I)
1. Build a `tungstenite` `WebSocketConfig` from `max_message_size`, pass via
   `accept_async_with_config` (server) / `connect_async_with_config` (client).
2. Spawn a per-connection `tokio::time::interval(heartbeat_interval)` heartbeat task on the
   server that sends `Message::ping` and tracks pong liveness.
3. Apply `connection_timeout` via `tokio::time::timeout` around the read loop.
4. Set the client's `closed` flag when reader/writer tasks actually terminate (remote close or
   send error), not only on local `close()`/`Drop`.
5. Add the crate's first tests: the close-message CAS race guard, broadcast `sent_count`
   semantics, empty-room cleanup, plus the new size/heartbeat/timeout/is_closed behavior. Use
   `tokio::time::pause`/`advance` for interval/timeout assertions — no real-time sleeps.

### T6 — armature-openapi (0C/2W/2I)
1. Rewrite the README Quick Start and Features list to the real manual-builder API
   (`OpenApiBuilder`, `swagger_ui_response`, `spec_json_response`), dropping Auto
   Generation/Type Inference/Validation/derive-macro claims — unless a small, well-scoped
   `#[derive(ToSchema)]`-style macro is judged worth adding within budget (decide during
   execution, default is doc-fix).
2. Add `SecurityScheme::OpenIdConnect` if OpenID stays advertised, or drop it from the doc list.
3. Reconcile the "built-in" Swagger UI framing against its CDN dependency (`swagger_ui_response`).
4. Tests: whatever the real API surface ends up being gets doctest/unit coverage proving the
   README examples actually compile and run.

## Verification (all tasks)

Every implemented unit gets a regression test that **fails against the current code** — write it
against a reverted version of the fix first if there's any doubt it discriminates, per the lesson
already learned twice in WF6.

## Central gate (coordinator, after all six land)

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets --features full-with-saml -D warnings` (standard
   allow-list) + per-crate + minimal-features clippy for the six touched crates
3. `cargo test` for the six crates with `ARMATURE_REQUIRE_DOCKER=1`
4. `cargo audit`, MSRV 1.89
5. CHANGELOG `[Unreleased]` entry (Added/Changed/Breaking as applicable), version bumps for
   touched crates per the 0.x-breaking-is-minor convention
6. Tick the corresponding `TODO.md` checkboxes
7. Commit, then hold for the user's audit window before push/PR/merge (per standing instruction)
