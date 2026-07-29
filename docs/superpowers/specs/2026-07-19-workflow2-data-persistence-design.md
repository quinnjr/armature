# Workflow 2 — Data & Persistence

**Date:** 2026-07-19
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 2 of 9)
**Crates:** armature-diesel, armature-seaorm, armature-eventsourcing, armature-cqrs, armature-redis, armature-tenancy, armature-opensearch
**Findings:** 8 Critical · 21 Warning · 11 Info (40 total; see `.superpowers/sdd/wf2-findings.md`)

## Problem

The seven data/persistence crates advertise behavior their code does not implement. Several are
correctness or security defects, not merely doc drift:

- **armature-tenancy middleware** — in optional mode a failed tenant resolution passes the request
  through unchanged, so a client-supplied `__tenant_id` header **survives and is trusted** →
  tenant-isolation bypass (cross-tenant data access).
- **armature-eventsourcing `AggregateRepository::save`** — passes `aggregate.version()` as the
  expected version while the store checks against the stored event count; because `apply_event`
  increments version, the normal apply-then-save flow raises a **false `VersionConflict`**, so
  optimistic concurrency is broken for real usage.
- **armature-diesel `transaction_with_isolation`** — issues `SET TRANSACTION ISOLATION LEVEL`
  **before** the transaction opens (a no-op outside a tx block), so the requested isolation is
  silently never applied — a correctness/consistency hazard.
- **armature-opensearch `execute_with_meta`** — reads aggregations from `"aggs"` but OpenSearch
  returns them under `"aggregations"`, so `SearchResult.aggregations` is **always `None`**.
- **armature-redis cluster support** — advertised and configurable, but the pool always builds a
  single-node manager, so "cluster mode" silently connects standalone to the first URL.
- **armature-opensearch AWS auth** — advertises AWS OpenSearch Service SigV4 auth (with
  aws-config/aws-credential deps) but never signs a request.
- Numerous **partial** config knobs across all seven crates (pool/connection/TLS/timeout/pagination
  options accepted but read nowhere) and stale README/doc examples that do not compile.

## Goal

Make every advertised unit in these seven crates conformant. Implement the 8 Critical and 21
Warning findings for real; reconcile the 11 Info (mostly stale docs and missing tests). When done,
every corresponding `TODO.md` checkbox is ticked and the datastore integrations do what their name,
docs, types, and tests claim. Verify with `armature-testkit` testcontainers (Postgres/Redis/
OpenSearch) and pure-logic unit tests — no live cloud credentials.

Non-goals: new datastore features beyond what is advertised; changing DI/module conventions;
implementing datastore backends the crate never claimed.

## Approach

One workflow → one PR to `develop`. Tasks ordered **Critical → Warning → Info**, grouped so related
fixes in a crate land together, and **security/correctness-first** (tenancy bypass, eventsourcing
version bug, diesel isolation lead). Reuse over re-implementation:

- **diesel-async** `build_transaction().isolation_level(...)` for the isolation fix rather than
  hand-rolled `SET TRANSACTION` ordering.
- **redis** `redis::cluster::ClusterClient` for real cluster support.
- **aws-config / aws-credential-types + aws-sigv4** (already deps) for OpenSearch SigV4 signing.
- **armature-core request extensions / request-local storage** for tenant context (replacing the
  spoofable `__tenant_*` headers).

### Verification (via `armature-testkit`, `containers` feature, gated by `docker_available()`)

- **Postgres container** — diesel (PG pool + isolation), seaorm (connect options, pagination),
  eventsourcing/cqrs (if a PG store is used), tenancy (PG-backed store). Assert real behavior
  against a live Postgres: e.g. two concurrent transactions observe the configured isolation;
  optimistic-concurrency conflict is raised only for a genuine stale write.
- **Redis container** — redis pool (cluster path via a note if a cluster harness is infeasible; at
  minimum unit-assert the manager selection), connection_url/db, command_timeout, CLIENT SETNAME;
  tenancy cache clear-by-prefix.
- **OpenSearch container** — opensearch client (aggregations round-trip, status-code error surface,
  TLS/transport options); **StubServer** for the SigV4 request-shape assertion (assert the
  `Authorization: AWS4-HMAC-SHA256 ...` header is produced for a known key/region/date).
- **Pure-logic (no container)** — eventsourcing version math, cqrs projection reset + bus error
  paths, seaorm pagination metadata + keyset prev_cursor, redis connection_url string building,
  tenancy incoming-header stripping + `count()` totals.
- Every implemented unit gets a regression test that **fails against the current code**.

### Conventions

- rustls-only default; heavy/native DB drivers stay behind their existing feature flags; per-crate
  minimal `tokio` features. `resolver = "3"` (workspace) keeps MSRV 1.89 resolution honest.
- `armature-testkit` added as a `dev-dependency` (with the `containers` feature) to the crates that
  need a live datastore; container tests self-skip via `docker_available()` when Docker is absent.
- New runtime deps kept minimal and behind existing features (e.g. aws-sigv4 only under the AWS/
  opensearch auth path; redis cluster under a `cluster` feature if not already present).
- **testkit has no MySQL container.** For diesel's MySQL pool knobs, either add a `MysqlContainer`
  to testkit (preferred if cheap) or unit-assert the built manager/connection options; decided in
  the plan per finding.

## Work breakdown (Critical/security-first)

### armature-tenancy (0C/5W/1I) — do FIRST (security)
- **W (sec)** strip/normalize incoming `__tenant_*` headers; store resolved tenant in request
  extensions (not spoofable headers); optional-failure path clears them. Close the isolation bypass.
- **W** `create` persists `display_name`; `count()` counts the filtered set (not the page);
  `clear_tenant` clears by prefix via a new CacheProvider scan/delete method.
- **I** provisioning-failure cleanup frees the slug (or documents the retention).

### armature-eventsourcing (2C/1W/1I)
- **C** `save` / `save_with_snapshot` pass the pre-new-events **base** version to the store; add a
  repo test that applies events before saving and asserts a genuine conflict is raised only on a
  real stale write.
- **W/I** reconcile snapshot docs; add the optimistic-concurrency regression test.

### armature-diesel (2C/3W/1I)
- **C** `transaction_with_isolation` applies the level on the BEGIN (diesel-async build_transaction);
  Postgres-container test proves the level is actually in effect.
- **C** pool constructors consume the advertised knobs (connect_timeout/min_idle/max_lifetime/
  idle_timeout/application_name/ssl_mode); drop or document any the backend can't honor.
- **W** `TransactionGuard` rollback-on-drop; honor `test_on_checkout`; rewrite README.
- **I** `PoolStatus::utilization` saturating (no underflow panic).

### armature-redis (1C/3W/1I)
- **C** real cluster manager when `cluster` set (or remove the knobs + claim if a cluster harness is
  out of scope — decided in plan, leaning toward implement).
- **W** fix `connection_url` db-append; apply `command_timeout`; issue `CLIENT SETNAME`.
- **I** honor `tls` for directly-constructed configs.

### armature-opensearch (2C/4W/1I)
- **C** aggregations from `"aggregations"`; SigV4 signing when `aws_region` set (StubServer asserts
  the signed header).
- **W** apply TLS/compression/connect_timeout/max_retries to the transport; check `status_code()` and
  surface 4xx/5xx as errors; wire or remove the bulk-streaming surface.
- **I** rewrite README.

### armature-seaorm (0C/4W/4I)
- **W** apply sqlx_log_level/statement_cache_capacity; read `DATABASE_IDLE_TIMEOUT`; emit
  `prev_cursor` for backward keyset pagination; fix the transaction doc.
- **I** consume `TransactionOptions` isolation/deferrable; reconcile crate/README docs; add the
  pagination-metadata test.

### armature-cqrs (1C/1W/2I)
- **C** rewrite README to the real `CommandBus`/`QueryBus` surface.
- **W** `rebuild` resets projection state before replay (no double-apply).
- **I** add rebuild + bus-error-path tests.

## Success criteria

- All 8 Critical and 21 Warning findings implemented with regression tests that failed against the
  old code; the 11 Info reconciled (doc/test). The tenancy isolation bypass, eventsourcing version
  bug, and diesel isolation no-op are proven closed.
- `cargo test` for all seven crates green (container tests pass when Docker present, self-skip
  otherwise); strict `clippy --workspace --features full-with-saml -D warnings`, `cargo audit`, and
  MSRV 1.89 all clean (the CI gates fixed in WF1).
- No datastore control silently degrades (no trusted-spoofable-tenant-header, no
  isolation-that-isn't, no success-on-error-status).

## Risks

- **Cluster/SigV4 integration** are the heaviest items; if a faithful test harness is infeasible in
  CI, unit-assert the request/manager construction and gate live tests `#[ignore]` — but do not ship
  a "cluster"/"AWS auth" surface that silently no-ops.
- **testcontainers flakiness / Docker-in-CI** — all container tests gated behind `containers` +
  `docker_available()`; the default `cargo test` stays credential- and Docker-free.
- **Tenant-context storage** depends on armature-core request-local/extensions API; if that API is
  thin, prefer the smallest addition there over leaving the spoofable header path.
