# Workflow 2 — Data & Persistence — Implementation Plan

Spec: `docs/superpowers/specs/2026-07-19-workflow2-data-persistence-design.md`
Findings: `.superpowers/sdd/wf2-findings.md`
Branch: `feature/wf2-data-persistence`. Execution: subagent-driven (implementer → reviewer per task),
security/correctness-first. Each task = failing regression test → implement → test passes → commit
`--no-verify`. Container tests use `armature-testkit` (`containers` feature) gated by `docker_available()`.

## Global Constraints
- Rust 2024, rustls-only default, per-crate tokio features, workspace `resolver = "3"` (MSRV 1.89).
- No datastore control may silently degrade. New deps minimal + behind existing features.
- Final gate must pass the WF1-fixed CI gates: fmt, clippy `--features full-with-saml -D warnings`,
  `cargo audit`, MSRV 1.89, per-crate tests.

## Part A — armature-tenancy (security first)

### Task A1: Close the tenant-isolation bypass (Critical-priority security)
**Files:** `armature-tenancy/src/middleware.rs` (~:70 handle, ~:103 get_tenant_id/optional). Test: middleware.rs.
- [ ] Failing test: a request carrying a client-supplied `__tenant_id`/`__tenant_name` header, run through `TenantMiddleware` in **optional** mode with a resolver that FAILS resolution, must NOT expose the spoofed tenant (get_tenant_id returns None / the anonymous default), and in enforcing mode must reject. Also: successful resolution stores the resolved tenant retrievably.
- [ ] Implement: at the START of `handle()`, strip/normalize any incoming `__tenant_*` headers before resolution. Store the resolved tenant in armature-core request extensions / request-local storage (read how armature-core exposes per-request extensions), and have `get_tenant_id`/`get_tenant_name` read from there — not from client-settable headers. On the optional-failure path, ensure no tenant identity leaks. Keep the existing public middleware API.
- [ ] Commit `fix(tenancy): close tenant-isolation bypass via request-local context, strip spoofable headers`.

### Task A2: TenantManager/store conformance (Warning cluster)
**Files:** `src/management.rs` (~:573 create, ~:894 count), `src/cache.rs` (~:109 clear_tenant), possibly cache trait.
- [ ] `create()` persists `request.display_name` (field on ManagedTenant/Tenant, or into metadata) matching `update()`. Test: created tenant round-trips display_name.
- [ ] `InMemoryManagedTenantStore::count()` counts the filtered set before pagination. Test: 200 matches, page limit 50 → count()==200.
- [ ] `TenantCache::clear_tenant` clears by prefix: add a scan/delete-by-prefix (or keys(pattern)) to `CacheProvider` and implement against it (Redis SCAN+UNLINK for the redis impl; prefix filter for in-memory). Test (in-memory + RedisContainer): setting N tenant keys then clear_tenant removes them.
- [ ] Commit `fix(tenancy): persist display_name, count full filtered set, implement clear_tenant by prefix`.

### Task A3: Provisioning-failure slug reclaim (Info)
**Files:** `src/management.rs` (~:599).
- [ ] On provisioning failure, either delete the record (freeing the slug) or make `create()` treat a Terminated slug as reclaimable. Test: failed-provisioning slug can be re-created. Commit `fix(tenancy): free slug on provisioning failure`.

## Part B — armature-eventsourcing

### Task B1: Fix optimistic-concurrency version base (Critical ×2)
**Files:** `src/repository.rs` (~:71 save, ~:178 save_with_snapshot, ~:62 docs). Test: repository.rs + PostgresContainer or in-memory store.
- [ ] Failing test: load/new aggregate, `apply_event` one event, then `save` — must SUCCEED (currently false VersionConflict). And a genuine stale-write (two saves at same base) must raise VersionConflict. Same for save_with_snapshot with an applied event.
- [ ] Implement: pass the pre-new-events base version to the store — `Some(aggregate.version() - events.len() as u64)` (guard underflow) or track the loaded base version. Fix both save and save_with_snapshot.
- [ ] Commit `fix(eventsourcing): pass base version for optimistic concurrency (closes false VersionConflict)`.

### Task B2: Snapshot docs + concurrency test (Warning + Info)
**Files:** `src/repository.rs` (~:94), `src/lib.rs`.
- [ ] lib.rs Snapshots docs state `save_with_snapshot` (+ Serialize/DeserializeOwned bound) is required for periodic snapshots; base `save` with snapshot_frequency set either errors or is documented no-op. Add the optimistic-concurrency regression test if not covered by B1. Commit `docs(eventsourcing): snapshot API reconciliation + concurrency test`.

## Part C — armature-diesel

### Task C1: Real transaction isolation (Critical)
**Files:** `src/transaction.rs` (~:131). Test: PostgresContainer.
- [ ] Failing test: run `transaction_with_isolation(Serializable, ...)` and assert the level is actually in effect (`SHOW transaction_isolation` inside the closure returns `serializable`). Currently returns the default.
- [ ] Implement via diesel-async `build_transaction().isolation_level(...)` (set on BEGIN), or issue `SET TRANSACTION` as the first statement inside the tx. Commit `fix(diesel): apply requested isolation level on the transaction (was a no-op)`.

### Task C2: Pool honors advertised config (Critical)
**Files:** `src/pool.rs` (~:51), `src/config.rs`. Test: PostgresContainer + unit.
- [ ] PgPool::new/MysqlPool::new apply connect_timeout, min_idle, max_lifetime, idle_timeout to the deadpool/bb8 builder; application_name/ssl_mode as libpq/connection options. For knobs a backend can't honor, remove the setter/field or doc as unsupported. Test: a pool built with connect_timeout/application_name reflects them (assert connection has application_name via `SHOW application_name` on PG; assert connect_timeout wired). MySQL: no testkit container — unit-assert the manager/opts config is populated (note in report).
- [ ] Commit `fix(diesel): apply pool config knobs (connect_timeout/min_idle/lifetimes/app_name/ssl_mode)`.

### Task C3: TransactionGuard + validation + README + utilization (Warning ×3 + Info)
**Files:** `src/transaction.rs` (~:200), `src/lib.rs` (~:14), `README.md`, `src/pool.rs` (~:230).
- [ ] `TransactionGuard` rolls back on drop when not committed (or remove it + doc). Test: drop without commit rolls back.
- [ ] Honor `test_on_checkout` with a validation query, or drop the claim + field.
- [ ] Rewrite README to real API (DieselConfig + PgPool::new + get()/transaction()).
- [ ] `PoolStatus::utilization` uses saturating_sub (no underflow panic). Test: available>size → no panic.
- [ ] Commit `fix(diesel): TransactionGuard rollback, connection validation, README, utilization`.

## Part D — armature-redis

### Task D1: Real cluster support (Critical)
**Files:** `src/pool.rs` (~:57), `src/config.rs`. Test: unit (cluster manager selected) + `#[ignore]` live cluster.
- [ ] When `config.cluster` is true, build a cluster-aware manager (`redis::cluster::ClusterClient` over `cluster_nodes`) behind a `cluster` feature if needed; single-node otherwise. If a faithful cluster test is out of CI scope, unit-assert the cluster path is taken (and gate a live test `#[ignore]`). Do NOT leave cluster as a silent single-node no-op.
- [ ] Commit `fix(redis): build a cluster-aware pool when cluster mode is configured`.

### Task D2: connection_url db + command_timeout + CLIENT SETNAME + tls (Warning ×3 + Info)
**Files:** `src/config.rs` (~:149 connection_url, ~:19 command_timeout, ~:35 connection_name/tls), `src/pool.rs`/`src/service.rs`. Test: unit + RedisContainer.
- [ ] `connection_url` appends `/{database}` correctly (parse past `scheme://[auth@]host:port`). Test: `redis://h:6379` + db 3 → `redis://h:6379/3`.
- [ ] Apply `command_timeout` via `tokio::time::timeout` around command execution (RedisContainer test: a slow/blocked command times out).
- [ ] Issue `CLIENT SETNAME` on connect when `connection_name` set (RedisContainer test: CLIENT GETNAME returns it).
- [ ] `connection_url` upgrades `redis://`→`rediss://` when `tls: true` regardless of construction path. Test: directly-constructed tls config yields rediss://.
- [ ] Commit `fix(redis): honor database in URL, command_timeout, CLIENT SETNAME, and tls scheme`.

## Part E — armature-opensearch

### Task E1: Aggregations + SigV4 AWS auth (Critical ×2)
**Files:** `src/search.rs` (~:357), `src/client.rs` (~:28), `src/config.rs`. Test: StubServer + OpenSearchContainer.
- [ ] `execute_with_meta` reads `result.get("aggregations")`. Test (StubServer canned response with an `aggregations` object): SearchResult.aggregations is Some.
- [ ] SigV4 signing when `aws_region` is set (aws-sigv4 + aws-credential-types/aws-config). StubServer test asserts the request carries `Authorization: AWS4-HMAC-SHA256 Credential=.../{region}/es/aws4_request ...` for a fixed key/region/date. If SigV4 genuinely can't integrate, remove the AWS claims + unused deps + aws_region field (do not ship a hollow AWS-auth surface).
- [ ] Commit `fix(opensearch): parse aggregations under correct key; real SigV4 AWS signing`.

### Task E2: Transport options + status-code errors + bulk (Warning ×4)
**Files:** `src/client.rs` (~:43), `src/config.rs` (~:78 tls), `src/index.rs` (~:235), `src/bulk.rs`. Test: OpenSearchContainer + StubServer.
- [ ] Apply `config.tls` (ca_cert/client_cert/danger_accept_invalid_certs) and compression/connect_timeout/max_retries to TransportBuilder (or doc inert ones removed).
- [ ] Index/client ops that only `.send().await?` now check `status_code().is_success()` and return `OpenSearchError::Internal(error.reason)` on 4xx/5xx (StubServer returns a 404/409 → error, not Ok). delete_by_query/count don't return 0 on error.
- [ ] Bulk: add a client method executing `Vec<BulkOperation<T>>` via `to_bulk_lines` (streaming with futures), or remove the futures dep + streaming claim.
- [ ] Commit `fix(opensearch): apply TLS/transport options, surface error status codes, wire bulk`.

### Task E3: README (Info)
- [ ] Rewrite README to the real API (OpenSearchConfig, QueryBuilder/BoolQueryBuilder, real client methods). Commit `docs(opensearch): correct README to the real client/query API`.

## Part F — armature-seaorm

### Task F1: Config + pagination conformance (Warning ×4)
**Files:** `src/config.rs` (~:180 to_connect_options, ~:106 from_env), `src/pagination.rs` (~:365 keyset), `src/transaction.rs` (~:34 doc). Test: unit + PostgresContainer.
- [ ] `to_connect_options` applies sqlx_log_level (→ LevelFilter → sqlx_logging_level) and statement_cache_capacity. Test: options reflect them.
- [ ] `from_env` reads `DATABASE_IDLE_TIMEOUT` (or drop from doc). Test: env var populates idle_timeout.
- [ ] `keyset_paginate` emits `prev_cursor` for `CursorDirection::Backward`. Test: backward page returns a prev_cursor.
- [ ] `begin_transaction` doc rewritten to reflect it only begins/returns a tx (no auto-commit). Commit `fix(seaorm): apply connect options + idle-timeout env + backward keyset prev_cursor; fix tx doc`.

### Task F2: Info cluster (TransactionOptions + docs + pagination test)
**Files:** `src/transaction.rs` (~:150), `src/lib.rs`, `README.md`, `src/pagination.rs` (~:86).
- [ ] Consume `TransactionOptions` isolation/deferrable (apply on the tx) or document unsupported. Reconcile crate/README docs (Migration/DI claims; compiling examples). Add the count-pagination metadata test (total_pages/has_next/has_prev). Commit `docs(seaorm): reconcile docs/README, consume tx options, add pagination-metadata test`.

## Part G — armature-cqrs

### Task G1: README + projection rebuild + tests (Critical doc + Warning + Info ×2)
**Files:** `README.md`, `src/projection.rs` (~:17 rebuild, ~:67), `src/command.rs` (~:114).
- [ ] Rewrite README to the real surface (CommandBus/QueryBus, register/execute, hand-impl Command/Query traits; no Mediator/derives).
- [ ] `Projection::rebuild` resets/clears state before replay (add a `reset()` the trait rebuild calls), so a populated projection isn't double-applied; rebuild_all inherits. Test: rebuild on a populated projection yields single-applied state.
- [ ] Tests for rebuild/rebuild_all replay-count and CommandBus/QueryBus error paths (HandlerNotFound, downcast mismatch).
- [ ] Commit `fix(cqrs): real README, projection reset-before-replay, bus/rebuild tests`.

## Part H — Final gate
- [ ] `cargo fmt`; `cargo fmt --check` clean.
- [ ] `cargo test` for all 7 crates green (container tests pass w/ Docker, self-skip otherwise).
- [ ] Strict `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings` (WF1 CI allow-list) clean.
- [ ] `cargo audit` clean; `cargo +1.89 check/test --features full-with-saml` clean (MSRV).
- [ ] Tick the 40 `TODO.md` checkboxes; update the summary table + totals; CHANGELOG entry.
- [ ] Commit `chore(wf2): tick TODO, changelog, final gate`. Then HOLD at ready-for-review (window-before-merge).
