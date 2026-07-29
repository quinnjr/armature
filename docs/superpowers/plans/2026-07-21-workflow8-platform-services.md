# Workflow 8 — Platform Services — Implementation Plan

**Spec:** `docs/superpowers/specs/2026-07-21-workflow8-platform-services-design.md`
**Findings:** 10 Critical · 65 Warning · 28 Info (103) across 14 disjoint crates.
**Execution:** one edit-only agent per crate (14 total), dispatched in criticality-ordered waves; coordinator runs the single central gate, `TODO.md`/`CHANGELOG` updates, and commit. Every implemented unit gets a regression test that **fails against the current code**. rustls-only; per-crate minimal `tokio`; implement-or-retract for genuinely-absent claims (build LaunchDarkly + the `Validate` derive; retract analytics Prometheus export, config Hot-Reload/Secrets/YAML/remote, audit JSON/DB backends, i18n Fluent, compression async buffering).

Each task: **implement the behavior + a failing-first regression test**. `(C)`/`(W)`/`(I)` = severity; file refs are the TODO.md `Fix` anchors.

---

## Wave 1 — request-path panics & fail-open (dispatch first)

### A1 · armature-ratelimit
1. (W) Reject or safely handle sub-second windows instead of integer divide-by-zero panic in `check_fixed_window` (`lib.rs:385`) and the identical Redis-store bug (`redis.rs:207`). Test: a 500ms window returns a decision, does not panic.
2. (W) Parse only the **first hop** of `X-Forwarded-For` (split on `,`, trim) so multi-hop proxied traffic yields a real IP; the key extractor no longer returns `None` and IP limiting stays active (`middleware.rs:344`). Test: `XFF: client, proxy1, proxy2` rate-limits on `client`.
3. (W) Read `skip_on_error`: `check` fails **closed** when `skip_on_error(false)` (`config.rs:189`). Test: store error + `skip_on_error(false)` → request denied.
4. (W) Apply `key_prefix` to the Redis store instead of the hardcoded `"ratelimit"` (`config.rs:242`). Test: configured prefix appears on the Redis key.
5. (W) Implement `remaining` for all three algorithms in both stores (`stores/memory.rs:284`, Redis). Test: `remaining` decreases with usage for sliding + fixed window.
6. (I) Compute `retry_after` from the configured window, not hardcoded 1s (`lib.rs:358`). (I) Bound/evict the standalone `algorithms::*` DashMaps (`algorithms/token_bucket.rs:50`).

### A2 · armature-metrics
1. (W) Char-boundary-safe `sanitize_path` truncation (`middleware.rs:202`). Test: a multibyte path near the limit doesn't panic.
2. (W) Register `ProcessCollector` into the local `registry` so `export_metrics` emits process cpu/memory (`lib.rs:57`). Test: exported output contains process metrics.
3. (W) Implement a real `Summary` (observe/quantiles/registration) or retract the claim from docs (`summary.rs:13`) — decide by whether the `prometheus` crate's summary is usable; prefer implement.
4. (W) Rewrite README to the real API; fix `Counter::new` shown infallible but returning `Result` (`README.md:11`).
5. (I) Test `RequestMetricsMiddleware::handle` records the counters/duration/in-flight (`middleware.rs:235`).

### A3 · armature-audit
1. (W) Char-boundary-safe `mask_value` — count/slice by chars, no mid-codepoint panic (`masking.rs:170`). Test: masking a multibyte secret doesn't panic and shows N chars.
2. (W) `extract_user_id` reflects the real principal (parse the verified `UserContext`/JWT subject, not literal `"authenticated_user"`) (`middleware.rs:61`). Test: user_id equals the token subject.
3. (W) Implement `FileBackend::delete_before` so retention deletes; stop the `Err(NotSupported)` error-loop in `RetentionManager::start` (`backend.rs:70`, `retention.rs:158`). Test: old entries deleted on cleanup.
4. (W) Buffer / reuse the file handle instead of per-event open+append+flush (`backend.rs:72`). Test: (unit) writer reused across events.
5. (I) Rewrite README + `lib.rs` feature list to the real API (no `AuditLog`/query builder/JSON-DB claims) (`README.md:22`, `lib.rs:13`).

---

## Wave 2 — hollow headline capabilities

### A4 · armature-admin
1. (C) `routes()` returns a **real mountable router** (`armature-core::Router`) whose handlers render the view structs to HTML (`lib.rs:170`). Test: mounting yields registered routes that respond.
2. (W) Back `ListView`/`DetailView`/`DashboardView` with a **pluggable data-source trait** instead of hardcoded `Vec::new()` (`views.rs:112`). Test: a stub data source populates rows.
3. (W) Honor `items_per_page`/`per_page`/`offset` and `max_items_per_page` in pagination (`views.rs:113`, `config.rs:17`).
4. (W) Enforce `require_auth` (guard dashboard/list/detail/create when true) (`config.rs:19`).
5. (W) Add `update`/`delete` handlers + wire `EditView`, or scope the "complete CRUD" claim to what ships (`views.rs:258`).
6. (I) Consume or drop the display/format knobs (`config.rs:24`) and the unused SQL helpers (`field.rs:267`).

### A5 · armature-analytics
1. (C) Implement `armature_core::Middleware` for `AnalyticsMiddleware` so recording is automatic (`middleware.rs:20`). Test: a request through the middleware increments counters.
2. (C) `Analytics::new` passes the `AnalyticsConfig` capacity knobs into `MetricsCollector::with_limits` (`lib.rs:101`). Test: a low `max_endpoints` is respected.
3. (W) Honor the remaining `AnalyticsConfig` fields (`enabled`, endpoint/rate-limit/client toggles, `throughput_window_secs`) (`config.rs:9`).
4. (W) Fix the two cap bugs so already-tracked endpoints/clients keep incrementing once the cap is hit (`collector.rs:159`, `:255`). Test: existing endpoint still increments at cap.
5. (W) Call `normalize_path` on the recording path (`middleware.rs:154`).
6. (W) Implement or **retract** Prometheus/Custom export backends, `TrackedHandler`, `AnalyticsExt` (`lib.rs:54`, `middleware.rs:134`, `:146`) — retract per spec.
7. (W) Fix `avg_utilization` and `requests_last_hour` semantics (`collector.rs:399`, `:470`).
8. (I) Uniform sampling RNG (`config.rs:147`); latency ms-truncation (`collector.rs:201`); snapshot sort cost (`collector.rs:416`).

### A6 · armature-features
1. (C) Implement `Operator::Matches` with `regex` (`flag.rs:238`). Test: a `Matches` rule fires on a matching value.
2. (C) **Build** LaunchDarkly integration against `launchdarkly-server-sdk` behind the `launchdarkly` feature — an evaluation adapter mapping contexts to LD; live test `#[ignore]`d (`lib.rs:12`).
3. (C) Rewrite README to the real `FeatureFlag`/`evaluate` API (`README.md:22`).
4. (W) Real multivariate bucketing across N variations (`flag.rs:59`). Test: users distribute across variants.
5. (I) Uniform `calculate_bucket` from more hash bytes (`flag.rs:301`); add operator tests incl. the fixed `Matches` (`flag.rs:280`).

### A7 · armature-validation
1. (C) **Build** `#[derive(Validate)]` + `#[validate(length/email/range/regex/required/custom)]` in `armature-proc-macro`; the derive emits a `Validate` impl calling existing validators, recursing into nested `#[validate]` struct fields (`README.md:24`, nested at `README.md:11`). Test: `trybuild` pass + behavior test that a derived struct validates and rejects.
2. (W) `MinLength`/`MaxLength` count **characters** not bytes (`validators.rs:47`). Test: a 3-emoji string passes `MaxLength(5)`.
3. (I) Fix the two-arg `ValidationError::new` README example (`README.md:64`); benchmark-or-drop `validate_parallel`'s speedup claim (`rules.rs:96`).

### A8 · armature-toon
1. (C) Real `to_string_pretty` formatting distinct from `to_string` (`lib.rs:94`). Test: pretty output differs and round-trips.
2. (W) Apply `with_type_hints`/`pretty`/`strict` in serialize/deserialize (`lib.rs:237`, `:267`). Test: `strict()` rejects unknown fields.
3. (W) Fix `From<serde_toon::Error>` so deserialize failures classify as a parse/deserialize error, not `SerializeError` (`error.rs:25`).

---

## Wave 3 — divergent data & CRDT correctness

### A9 · armature-opentelemetry
1. (C) Pass the **real** method/route to `record_request` instead of literals `"method"`/`"path"` (`middleware.rs:129`). Test: recorded dimensions match the request.
2. (W) Count error-path requests too (move recording out of the `Ok`-only branch) (`middleware.rs:127`).
3. (W) Install a W3C/B3 propagator (`set_text_map_propagator`) so context propagation works (`middleware.rs:74`). Test: a `traceparent` is parsed into the parent context.
4. (W) Apply span limits (`tracing_setup.rs:41`) and metrics collection interval (`metrics.rs:35`).
5. (W) Wire the log appender behind `enable_logging`/the `logging` feature, or **retract** it (`config.rs:30`).
6. (W) Rewrite README to the real `TelemetryBuilder`/`init_tracing` API (`README.md:1`).

### A10 · armature-collab
1. (W) `RgaText::operations` emits real `Delete` ops for tombstones and never exports them as `Insert '\0'` (`text.rs:353`). Test: delete replays as delete on a peer.
2. (W) Deterministic `merge` honoring the causal `after` chain with a stable node-id tie-break (`text.rs:380`). Test: two replicas of a multi-char interleave converge.
3. (W) Fix `OperationBuffer::add` to evict the **oldest** on overflow (`sync.rs:154`); give buffered ops real `deps` so the causal buffer works (`sync.rs:306`).
4. (W) Wire `SyncProtocol` into `CollabSession` so `SyncRequest` transfers state (`sync.rs:287`). Test: a joining peer receives current document state.
5. (W) Emit the 5 dormant `SessionEvent` variants on the corresponding actions (`session.rs:130`); honor the 6 unread `SessionConfig` knobs (`session.rs:38`).
6. (I) Track `SyncStats`/`ops_sent` (`sync.rs:369`, `session.rs:123`); reduce `insert_str` quadratic cost (`text.rs:209`).

---

## Wave 4 — inert knobs, docs, efficiency (may overlap earlier waves)

### A11 · armature-cache
1. (W) Apply `connection_timeout`/`operation_timeout`/`max_connections` (wrap ops in `tokio::time::timeout`, configure the connection manager/pool) (`config.rs:31`). Test (RedisContainer): a slow op times out with `CacheError::Timeout`.
2. (W) Real `TieredCache::stats` (track hits/misses/promotions) (`tiered.rs:179`); L1 expiry eviction + capacity bound (`tiered.rs:232`).
3. (W) Fix `MemcachedCache::increment` (atomic/create-at-zero semantics, no silent `delta.abs()`) (`memcached_cache.rs:174`).
4. (W) Rewrite README to the real `CacheConfig`/`InMemoryCache` API (`README.md:22`).
5. (I) Document `ttl` limitation honestly (`memcached_cache.rs:154`); coalesce `invalidate_tags` lock/round-trips (`invalidation.rs:147`).

### A12 · armature-compression
1. (W) Apply `min_chunk_size` in `compress_chunk` (buffer until threshold) (`streaming.rs:57`). Test: sub-threshold chunks aren't compressed prematurely.
2. (W) Rewrite README to the real API (no `min_size`/`exclude_types`/`CompressionLevel`) (`README.md:34`).
3. (I) Implement real async buffering in `AsyncStreamingCompressor` or **retract** the async claim (`streaming.rs:451`); make `Auto::compress` signal its no-op honestly (`algorithm.rs:133`); add an end-to-end `handle` test (`middleware.rs:157`).

### A13 · armature-i18n
1. (W) Construct/consume `TranslationSource` (Fluent/Memory) or **retract** the unused variants (`messages.rs:14`).
2. (W) Rewrite README to the real add_bundle/`t`/`Locale` signatures (`README.md:22`).
3. (W) Timezone-aware `TimeStyle::Full/Long` (`format.rs:24`); weekday in `DateStyle::Full` (`format.rs:322`); apply `min_integer_digits` (`format.rs:43`); fix `max_fraction_digits` trailing-zero trimming (`format.rs:92`).
4. (I) Locale-aware month names (`format.rs:293`); highest-score `negotiate_locale` (`locale.rs:423`); cache plural rules (`plural.rs:96`).

### A14 · armature-config
1. (C) Rewrite README Quick Start to the real `ConfigManager`/`ConfigService::builder()` API (`README.md:34`).
2. (W) Implement or **retract** Hot-Reload / Secrets / YAML / remote-source claims (`README.md:7-11`) — retract per spec (or add YAML behind a feature if cheap).
3. (W) Surface the discarded dotenv error in `build()` (`config_service.rs:121`).
4. (W) Support dot-path `get("database.host")` into loaded nested files (`lib.rs:74`). Test: nested key resolves.
5. (I) Typed getters (`get_int`/`get_bool`/`get_float`) work on env-loaded string values via parse-coercion (`lib.rs:168`); test `load_file` (`lib.rs:191`).

---

## Central gate (coordinator, after all agents)
1. `cargo fmt --all`.
2. `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings`.
3. Per-touched-crate `--no-default-features` clippy.
4. `ARMATURE_REQUIRE_DOCKER=1 cargo test --workspace --features full-with-saml`.
5. `cargo audit`; MSRV `cargo +1.89 check` on touched crates.
6. Tick `TODO.md` boxes per crate (verify each against the diff — no section-wide auto-tick); update the summary table + totals.
7. `CHANGELOG` entry (per-crate narrative + every breaking change and version bump).
8. Commit; open one PR to `develop`.
