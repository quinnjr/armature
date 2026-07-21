# Workflow 9 — Core & Tooling: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every advertised unit in the ten core/tooling crates conformant (16C/48W/23I = 87 findings), each fix landing with a regression test that fails against current code.

**Architecture:** Ten edit-only implementation agents, one per crate (pairwise edit-disjoint), dispatched in three parallel waves; the coordinator owns the central gate, semver bumps, CHANGELOG, TODO tick, and commits. The single cross-crate dependency (app needs an address-accepting listen) is satisfied by putting core in Wave 1.

**Tech Stack:** Rust workspace; trybuild (armature-mcp precedent) + `#[should_panic]` expansion tests for macro crates; assert_cmd/predicates/tempfile (already CLI dev-deps) for CLI; flate2 (already an armature-core dep) for real compression; std `IsTerminal`; ferron's already-declared `notify`.

**Spec:** `docs/superpowers/specs/2026-07-21-workflow9-core-tooling-design.md`
**Findings:** `.superpowers/sdd/wf9-findings.md` (87: 16C/48W/23I; file:line for every item)
**Branch:** `feature/wf9-core-tooling` (worktree off `origin/develop`) → PR to `develop` (HELD for user audit window)

## Global Constraints

- rustls-only; no OpenSSL/native-tls; per-crate minimal `tokio` feature subsets (never `full`).
- Agents never touch CHANGELOG, crate `version` fields, `TODO.md`, or git — coordinator only.
- Never leave a stub masquerading as a feature: implement, or remove flag+dep+claim together.
- Every Critical/Warning fix ships a regression test that fails against the old code; `reconcile: claim`/`test` items are doc/test-only.
- Default `cargo test --workspace` stays credential-free and Docker-free (Docker-needing tests self-skip; `ARMATURE_REQUIRE_DOCKER=1` forces them at gate time).
- Breaking changes are allowed but must be listed by the agent in its report for CHANGELOG Breaking entries + 0.x minor bumps.

## Dispatch

| Wave | Tasks (parallel) |
|---|---|
| 1 | T1 macros-utils [opus] · T2 proc-macro [opus] · T3 macros [sonnet] · T4 core [opus] |
| 2 | T5 testing [sonnet] · T6 ferron [sonnet] · T7 log [sonnet] |
| 3 | T8 app [sonnet] · T9 rhai [sonnet] · T10 cli [opus] |

## Tasks

### T1 — armature-macros-utils (6C/7W/1I) [opus]
1. - [ ] `assert_json!`/`assert_status!`: real comparisons (parse body → `assert_eq!` on `serde_json::Value`; status incl. the documented `ok` alias and two-arg forms).
2. - [ ] `test_request!`: parse method/path/optional body+headers; build the corresponding `HttpRequest`.
3. - [ ] `derive_model`: emit field-wise `Debug` + `Clone` impls; `new()` bounded `where Self: Default`; **retract** the Serialize/Deserialize claim (derive macros cannot attach serde derives) — doc says "Debug, Clone; add serde derives yourself".
4. - [ ] `derive_api_model`: parse `#[api(skip)]` and exclude those fields from `to_json` (filtered map).
5. - [ ] `derive_resource`: parse `#[resource(table = "…", primary_key)]` for real (`table_name()` returns configured/snake-case name, expose `primary_key()`); **narrow** the CRUD claim to table metadata.
6. - [ ] `bail!`/`ensure!`/`validate!`/`validate_required!`: parse the documented multi-arg forms (optional error-kind ident, format-args message, `Punctuated` field lists); surface caller messages, name the missing field.
7. - [ ] `redirect!`/`json!`/`html!`/`text!`: optional leading status (numeric or `permanent`/`temporary`/`ok` aliases).
8. - [ ] `validate_email!`: `static LazyLock<Regex>` (compile once); document the caller-side `regex` dependency.
9. - [ ] Tests: new `tests/` with trybuild compile-pass for every documented form + behavioral tests (`#[should_panic]` proves assert macros fail on mismatch; skip-marked field absent from JSON; each bail/ensure form maps to the right Error variant).

### T2 — armature-proc-macro (2C/4W/2I) [opus]
1. - [ ] `mod`-declare + export the existing `catch_attr.rs`/`guard_attr.rs`/`middleware_attr.rs`: `#[catch]`, `#[guard]`, `#[use_guard]`, `#[middleware]`, `#[use_middleware]` entry points in lib.rs (fix any bit-rot in those files so they compile).
2. - [ ] `#[cache]`: key incorporates actual argument values (format captured param idents into the key); parse `ttl`/`key`/`tag` attributes and thread into key/ttl/tags so tag invalidation works.
3. - [ ] `Controller::routes()`/`__collect_routes`: populate from the `#[routes]`-generated handler metadata so it returns the same list the registrar registers (if truly circular, make `routes()` delegate to the registrar path and document it — no silent `vec![]`).
4. - [ ] `Param` derive: implement the documented multi-field struct mode (named fields each extracted from the same-named path param via FromStr) alongside the single-value mode.
5. - [ ] `Query` derive: deserialize from the decoded pairs without lossy re-encoding (percent-encode k/v via `url::form_urlencoded::Serializer` before `from_str`, or deserialize pair-wise) — `&`/`=`/`%` in values round-trip.
6. - [ ] Info: route-path normalization — normalize `:id` → matchit syntax as documented, or fix the doc to "validated verbatim"; `timeout`/`body_limit` wrappers use the handler's real request param ident (not hardcoded `req`).
7. - [ ] Tests: trybuild compile-pass for each exported attribute + `#[should_panic]`-free behavioral tests (cache keys differ per args; query with `&` in value round-trips; multi-field Param extracts).

### T3 — armature-macros (1C/2W/3I) [sonnet]
1. - [ ] `routes!`: rewrite onto `armature_core::Route::new(HttpMethod::…, path, handler)` (drop the broken struct literal); compile-pass test proving expansion builds.
2. - [ ] README: **remove** the "Validation Macros" feature bullet + `validate_required!`/`validate_email!`/`validate!` examples (they live in armature-macros-utils; add a pointer line). Module doc: replace `error_json!` with an existing macro (`not_found!`).
3. - [ ] `paginated_response!`: bind `$data` once (`let __data = $data;` — len computed before serialization; no double-eval/use-after-move).
4. - [ ] `header!` doc: fix the `.unwrap_or` example to a type-correct form.
5. - [ ] Replace the empty `test_macros_compile` with a real suite: every exported macro invoked and asserted (status/body of responses, pagination fields, header hit/miss).

### T4 — armature-core (0C/4W/4I) [opus]
1. - [ ] **First (unblocks T8):** additive address-accepting listen — `pub async fn listen_on(self, addr: impl Into<SocketAddr>) -> Result<(), Error>` beside `listen` (which becomes `listen_on((Ipv4Addr::UNSPECIFIED, port))`).
2. - [ ] `CacheInterceptor`: real store (`Arc<RwLock<HashMap<String, (Instant, HttpResponse)>>>`): lookup before `next`, return fresh (< ttl_seconds) hits, insert after; eviction of expired entries on insert.
3. - [ ] `Http3Config`: `configure_quinn` applies `stream_receive_window`/`receive_window`; enable 0-RTT on the quinn server config when `enable_0rtt` (count `zero_rtt_accepted` on accept).
4. - [ ] `BatchConfig`: **remove** `adaptive_batching`/`min_batch_size`/`parse_timeout_ms` fields + setters + preset references (breaking; report for CHANGELOG).
5. - [ ] `micro::Compress`: real gzip via the existing `flate2` dep when Accept-Encoding permits, honoring `CompressionLevel`, setting `Content-Encoding` (+ keep Vary). `CompressionMiddleware` (Info): same compressor honoring `min_size`, name made true.
6. - [ ] Info: `simd_parser` docs → say scalar (memchr paths stay "SIMD"); `AuthenticationGuard` doc → "presence/format check only; use armature-auth for validation"; `ResponseCache::evict_oldest` → insertion-order `VecDeque` eviction (no O(n) scans on the hot path).
7. - [ ] Tests: interceptor caches within ttl and misses after; h3 transport params reflect config; compress round-trips (gzip decode) and respects Accept-Encoding absence; eviction order.

### T5 — armature-testing (3C/7W/1I) [sonnet]
1. - [ ] `TestApp::get`/`TestContainer::get`: delegate to the stored `armature_core::Container` (adapt `Result<Arc<T>>` → the documented return); `TestApp::new` passes the caller's container into `Application::new` (registered providers reach handlers).
2. - [ ] `TestAppBuilder::add_module`: register the module's providers/controllers for real (same wiring Application uses); if module wiring genuinely cannot work here, **remove** the method (breaking; report).
3. - [ ] `IntegrationTestBuilder`: add `run_test(body)` executor that awaits before_each hooks → body → after_each hooks.
4. - [ ] `LoadTestRunner::run`: honor `rate_limit` (pace workers); distribute `total_requests % concurrency` remainder; guard concurrency > total (exact request count always).
5. - [ ] `DockerContainer`: `start` polls readiness up to `wait_timeout_secs` (not fixed 2s); `Drop` stops synchronously via `std::process::Command` (`docker stop`) — no nested runtime, safe inside `#[tokio::test]`.
6. - [ ] `ContractManager::list`: store consumer/provider inside the JSON and read back (round-trips hyphenated names into `load()`); `verify_interaction` (Info): doc narrowed to response-side verification.
7. - [ ] Tests: registered provider retrievable + reaches a handler via DI; hooks run in order; load counts exact (100/3, 5>2 cases); Drop-in-async doesn't panic (docker-gated self-skip); hyphenated contract round-trip.

### T6 — armature-ferron (0C/8W/2I) [sonnet]
1. - [ ] `to_kdl()`: emit backend `timeout`/`headers`; single-backend path (`FerronConfigBuilder::backend`) stores the full `Backend` so weight/timeout/headers/max_connections/backup reach the KDL; emit `http_port`/`https_port`; emit `error_log`; gate access-log on `logging`; emit `add_prefix` rewrite.
2. - [ ] `FerronManager`: spawn a `notify` watcher on config_path populating `watch_handle`; reload on change when `auto_reload`.
3. - [ ] `HealthState::check_backend`: Unhealthy only at `consecutive_failures >= unhealthy_threshold` (else Degraded); mirror for recovery via `healthy_threshold`.
4. - [ ] `FerronProcess`: `is_running` probes with signal `None` (signal 0) + `try_wait()` zombie detection; log forwarders open each file once, `.await` writes (no per-line reopen, no `block_on` in async).
5. - [ ] Drop unused `handlebars` + `kdl` deps from Cargo.toml (hand-rolled emission stays).
6. - [ ] Tests: `to_kdl` asserts every previously-dropped option (both backend paths); health transitions at exactly the thresholds; `is_running` true for live child, false after exit.

### T7 — armature-log (0C/2W/3I) [sonnet]
1. - [ ] Auto-init: `std::sync::Once`-guarded init inside the `log()` entry point (forces `CONFIG` Lazy + atomic sync) so env vars work without explicit `init()` — doc claim becomes true.
2. - [ ] `atty::is`: real `std::io::IsTerminal` check honoring the passed `Stream`; NO_COLOR/TERM stay as overrides.
3. - [ ] `config()` performs the same full atomic init as `init()` (no partial state depending on call order).
4. - [ ] Info: "zero-cost when disabled" doc → "runtime-gated"; output-format tests for JSON/Pretty/Compact renderers (assert emitted shape/fields).

### T8 — armature-app (1C/2W/3I) [sonnet] — after T4.1
1. - [ ] Register the four dispatch closures as `call` (keep `invoke` as alias) so documented `ctx.call(...)` works.
2. - [ ] `build_router`: prepend each module's `guards` to contained controllers' guard lists (module guards enforced ahead of controller guards).
3. - [ ] `run()`: parse host into `IpAddr`, bind via core's new `listen_on` (127.0.0.1 actually binds 127.0.0.1).
4. - [ ] Info: Quick Start uses `create_module`; module setters error on wrong-typed elements instead of `filter_map` silent drop; bootstrap/shutdown hook errors logged, bootstrap failure aborts startup.
5. - [ ] Tests: rhai script calling `ctx.call` resolves; module-guard blocks an unauthorized request; host knob reaches the bound addr (bind 127.0.0.1 ephemeral, connect).

### T9 — armature-rhai (1C/3W/2I) [sonnet]
1. - [ ] Matcher returns captured `:param`/catch-all values; `find_route` inserts them into `request.path_params` before dispatch — `request.param("id")` returns the value (Quick Start works).
2. - [ ] `call_after`: build `ResponseBinding` from the real response (status/headers/body) so scripts inspect/amend the actual outgoing response.
3. - [ ] `script_handler`: never panics — `block_in_place`+`Handle::block_on` inside a multi-thread runtime; dedicated thread + mini runtime when none/current-thread; tests cover all three contexts.
4. - [ ] Catch-all matches on segment boundary (`path == prefix || path.starts_with(prefix + "/")`) — `/apix` rejected.
5. - [ ] Info: Quick Start rewritten to the real builder API (`max_operations`/`scripts_dir`/`hot_reload`; no phantom `serve`); wire `is_stale`/`evict_stale` into `compile_file`'s hot-reload path (recompile only when stale) + test.

### T10 — armature-cli (2C/9W/2I) [opus]
1. - [ ] `new::run`: extend signature (database, features, docker, ci) and emit the corresponding Cargo deps/features, Dockerfile, `.github/workflows/ci.yml`; thread from **both** the wizard (db_idx, selected_features, include_docker, include_ci) and the `--database`/`--docker`/`--ci` flags.
2. - [ ] Implement graphql/grpc/lambda/cloudrun template branches in `create_project_structure` (template-specific deps + starter module/main); **validate the template before any file is written** (bad template → error, no half-created dir).
3. - [ ] `doctor`: track failures; report missing essentials and exit non-zero (no unconditional all-clear). `validate`: return Err (non-zero exit) when has_errors.
4. - [ ] `generate`: `--fields` parsed (`name:string,email:string`) into dto/entity/scaffold structs; guard/job templates vary by `--guard-type`/`--job-type`; `--auth` adds the auth guard.
5. - [ ] `routes`: apply `--path` filter, render `--format` table/json/yaml/markdown distinctly, parse `#[middleware]`/`#[guard]` attributes into RouteInfo (statistics real, `--middleware` shows them).
6. - [ ] `openapi generate_client`: honor `with_logging` (request/response logging wrapper), `with_retry` (retry wrapper), `base_url` (default in constructor) in both TS and Rust generators; consume or drop `async_client`.
7. - [ ] `mock --watch`: watch the spec file, reload MockState on change. `repl`: inject the Armature prelude into evcxr (`:dep` + `use armature::prelude::*;` via stdin) — doc claim true.
8. - [ ] **Retract db-migrations:** remove the `Db` subcommand + `DbCommands` + the README "Database Migrations" feature bullet (breaking; report for CHANGELOG).
9. - [ ] Tests (assert_cmd + tempfile): every template incl. the four new ones produces its advertised tree; `--database postgres --docker --ci` emits deps/Dockerfile/workflow; invalid template writes nothing; doctor/validate exit codes; `--fields` lands in the DTO; routes formats differ; generated-project cargo-check as `#[ignore]`d smoke test.

## Verification (central, coordinator, after all waves)
1. `cargo fmt --all`.
2. Strict `clippy --workspace --all-targets --features full-with-saml -- -D warnings -A collapsible_if -A result_large_err -A dead_code -A useless_vec -A unwrap_or_default`.
3. `cargo test` for the 10 crates + consumers; full default `cargo test --workspace`; Docker present → `ARMATURE_REQUIRE_DOCKER=1` for gated tests.
4. `cargo audit`; MSRV `cargo +1.89 check` (no new deps expected beyond dropping two).
5. Confirm each of the ten crates' new test suites executes in CI's workspace run (cf. the known CI feature-gap: 38/62 member crates never build with real features — these ten must not join them).
6. Semver: 0.x minor bump for every crate with breaking/behavior change; update internal + root pins; CHANGELOG (Added/Changed-breaking/Fixed; call out CLI `db` removal, core batch-knob removal, testing signature corrections).
7. Tick the 87 `TODO.md` boxes; ten summary rows → `N/N` conformant; update the header totals (233 → 146).

## Then
Adversarial review of the macro-layer expansions (compile-time surface; highest blast radius) → fix → gate green → **PR to `develop` HELD** for the user's `/simplify` `/code-review` `/optimize` audit window. CodeQL is a required check — PR must be CodeQL-clean. Note the WF8 coordination point: if WF8's PR added `#[derive(Validate)]` to armature-proc-macro first, rebase `lib.rs`'s mod list.
