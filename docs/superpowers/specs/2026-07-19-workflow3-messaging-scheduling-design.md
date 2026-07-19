# Workflow 3 — Messaging & Scheduling

**Date:** 2026-07-19
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 3 of 9)
**Crates:** armature-messaging, armature-queue, armature-events, armature-webhooks, armature-cron, armature-distributed, armature-discovery
**Findings:** 10 Critical · 25 Warning · 16 Info (51 total; see `.superpowers/sdd/wf3-findings.md`)

## Problem

The seven messaging/scheduling crates advertise behavior their code does not deliver. Several are
correctness or data-loss defects rather than doc drift:

- **armature-queue worker** — three separate defects on the job-processing path: a documented
  registration method (`register_cpu_intensive_handler`) `block_on`s a lock inside async code and
  **panics** in exactly its documented usage; `register_handler` defers the insert to a
  fire-and-forget `tokio::spawn`, so `register_handler(); start();` **races** and jobs spuriously
  fail "No handler"; and `process_batch` **orphans** a type-mismatched job that was already popped
  and moved into the processing set — silent **data loss**.
- **armature-cron scheduler** — `add_job` inserts in a detached task and returns `Ok` before the job
  is registered (never surfacing `JobAlreadyExists`); `start` holds the global `jobs` write lock
  across each `job.execute().await`, **serializing the entire scheduler** and defeating the
  advertised async execution / `max_concurrent_jobs`; and `CronExpression::matches` ignores its
  `time` argument, so the exported predicate almost never answers correctly.
- **armature-discovery etcd** — `register` writes under `{prefix}/{id}` while `discover` scans
  `{prefix}/{name}/`, so the register→discover round trip **can never return** what this client
  wrote; `list_services` is a hollow `Ok(vec![])`.
- **armature-webhooks** — README advertises Idempotency + Stripe/GitHub/Slack provider support and a
  Quick Start API that **do not exist**; `truncate_string` byte-slices untrusted response bodies and
  **panics** on a multibyte boundary; `signing_algorithm`/`timestamp_tolerance` config knobs are
  read nowhere; the default `send` path emits **unsigned** webhooks despite an "automatic signing"
  claim.
- Numerous **partial** integration knobs across messaging (Kafka manual-ack never commits, NATS
  JWT/JetStream/reconnect dropped, RabbitMQ vhost/confirms/pool unreachable, custom headers lost on
  a NATS round trip), events (`continue_on_error(false)` doesn't stop), distributed (locks have no
  TTL renewal despite the "automatic renewal" claim), plus stale READMEs that do not compile.

## Goal

Make every advertised unit in these seven crates conformant. Implement the 10 Critical and 25
Warning findings for real; reconcile the 16 Info (stale docs, missing tests, small correctness/
efficiency items). When done, every corresponding `TODO.md` checkbox is ticked and the messaging/
scheduling integrations do what their name, docs, types, and tests claim. Verify with
`armature-testkit` (Redis testcontainer + `StubServer` for HTTP backends) and pure-logic unit tests
— no live cloud/broker credentials required in CI.

Non-goals: new broker backends or features beyond what is advertised; changing DI/module
conventions; implementing a real Kubernetes discovery backend (the README claim is dropped, not
built) or the events crate's event-sourcing/replay (that lives in armature-eventsourcing — the
claim is removed).

## Approach

One workflow → one PR to `develop`. Tasks ordered **Critical → Warning → Info**, grouped by crate so
related fixes land together, **correctness/data-loss-first** (queue worker, cron scheduler, etcd key
scheme, webhook panic/spoof). Reuse over re-implementation:

- **tokio** `Semaphore` for cron `max_concurrent_jobs`; short-lived lock + clone-out for the
  scheduler lock-across-await bug.
- Existing broker client crates (rdkafka commit APIs, async-nats `ConnectOptions`/`jetstream`,
  lapin channel/vhost) for the messaging config wiring.
- `str::is_char_boundary`/`char_indices` for the webhook + any multibyte truncation.
- `armature-core` request extensions where a handler needs request-local state.
- **armature-testkit** `RedisContainer` + `StubServer` (both from WF0) for the live-path tests.

### Verification (via `armature-testkit`, gated by `docker_available()`)

- **Redis container** — queue (priority ordering, retry-with-backoff, dead-letter routing,
  delayed-job promotion; the worker register→start→process happy path with no orphaned jobs),
  distributed (lock mutual-exclusion via SET NX, token-guarded release, single-leader election).
- **StubServer (HTTP)** — discovery (consul `?passing=true` filtering; etcd register→discover→
  get_service round trip against a stubbed KV API), webhooks (signed delivery header shape, unsigned
  default, dispatch fan-out), messaging MqBridge where an HTTP endpoint models the bridge.
- **Pure-logic (no container)** — cron `matches` + missed-job/next-run math + `JobAlreadyExists`;
  events `continue_on_error` sequential-stop ordering + `HandlersFailed`; webhooks
  `truncate_string` multibyte, header-case matching, HMAC-SHA256-vs-512 selection; messaging NATS
  header round-trip + Kafka `enable_auto_commit` derivation; discovery etcd composite-key builder.
- Every implemented unit gets a regression test that **fails against the current code**.

### Conventions

- rustls-only default; broker/native deps stay behind their existing feature flags; per-crate
  minimal `tokio` features. Workspace `resolver = "3"` keeps MSRV 1.89 honest.
- `armature-testkit` added as a `dev-dependency` (with `containers`) only to crates that need a live
  datastore; container tests self-skip via `docker_available()`.
- No control silently degrades: no lock that "renews" but doesn't, no manual-ack that never commits,
  no TLS/plaintext downgrade, no success-on-error-status, no unsigned "signed" webhook.
- Breaking API changes (e.g. `register_handler`/`add_job` becoming `async`) get a CHANGELOG
  Breaking entry and the affected crate's semver minor bump (0.x → 0.(x+1).0), matching WF2.

## Work breakdown (correctness/data-loss-first)

### armature-queue (3C/2W/4I) — do FIRST (data loss + panics)
- **C** worker: `register_handler`/`register_cpu_intensive_handler` register synchronously (async +
  `.await`, no `block_on`); `process_batch` re-enqueues/fails a type-mismatched job instead of
  orphaning it. Redis-container test: register→start→enqueue→complete with zero orphaned jobs.
- **W/I** README to real API; `enqueue_in`/`enqueue_at` convenience; `process_batch` success-count
  log; `max_size` counts delayed+processing; `dequeue` skips the promotion Lua when idle; container
  tests for retry/DL/delayed/priority.

### armature-cron (3C/4W/2I)
- **C** `matches` computes correctly (`schedule.after(time-1s).next() == Some(time)`); `add_job`
  registers inline and returns `JobAlreadyExists`; `start` clones state under a short lock and runs
  the job future without holding the map lock.
- **W** enforce `max_concurrent_jobs` via `Semaphore`; implement or drop `run_missed_jobs`; fix
  lib.rs docs + README (6-field cron, `add_job(|ctx|)`); drop Timezone claim.

### armature-discovery (2C/2W/2I)
- **C** etcd composite key `{prefix}/{name}/{id}` across register/get_service/deregister so discover
  matches; real `list_services` via `/v3/kv/range`.
- **W** consul `?passing=true`/Checks filtering; README to real API, drop k8s. **I** `health_check`
  surfaces `HealthCheckFailed` instead of `Ok(false)`; round-trip test.

### armature-webhooks (2C/4W/2I)
- **C** rewrite README (remove idempotency/provider claims or implement; real Quick Start API).
- **W** thread `signing_algorithm` (dispatch SHA512), source `timestamp_tolerance` from config,
  correct the `send` doc/behavior (unsigned unless secret), `truncate_string` on a char boundary.
- **I** concurrent `dispatch`; case-insensitive `verify_from_headers`.

### armature-messaging (0C/7W/2I)
- **W** Kafka manual-ack commits offset; combine `enable_auto_commit` with ack_mode; MqBridge routes
  per-call topic + caches the publisher; NATS applies JWT/NKey/JetStream/reconnect and preserves
  custom headers on receive; RabbitMQ `connect_with_config` honors vhost/confirms/pool.
- **I** `publish_with_options` applies (or documents) options; remove the orphaned broker.rs/
  traits.rs/message.rs (or wire them) — decided in plan (lean toward remove: they never compiled).

### armature-events (0C/3W/2I)
- **W** `continue_on_error(false)` actually stops (sequential await) or the doc is corrected;
  README/lib.rs docs describe the real trait-based handler API (drop event-sourcing/replay/decorator
  claims). **I** README Quick Start compiles; `HandlersFailed`/sequential path tested.

### armature-distributed (0C/3W/2I)
- **W** add a LockGuard TTL-renewal watchdog (or drop the "automatic lock renewal" claim — decided in
  plan, lean toward implementing the watchdog); README Quick Start + Features to the real
  locks+leader surface (drop rate-limit/cache/circuit-breaker claims).
- **I** `LockGuard::drop` best-effort documented + safe off-runtime; lock/leader semantics tests.

## Success criteria

- All 10 Critical and 25 Warning findings implemented with regression tests that failed against the
  old code; the 16 Info reconciled. The queue worker races/data-loss, cron scheduler serialization,
  and etcd key mismatch are proven closed.
- `cargo test` for all seven crates green (container tests pass when Docker present, self-skip
  otherwise); strict `clippy --workspace --features full-with-saml -D warnings`, `cargo audit`, and
  MSRV 1.89 all clean.
- No control silently degrades (no register race, no orphaned jobs, no lock that doesn't renew, no
  manual-ack that never commits, no unsigned "signed" webhook, no success-on-error-status).

## Risks

- **Broker integration** (Kafka/NATS/RabbitMQ) is the heaviest surface; where a faithful CI test
  harness is infeasible, unit-assert the config→client-option mapping and gate live tests
  `#[ignore]` — but do not ship an integration knob that silently no-ops.
- **async signature changes** on queue/cron registration are breaking; bump semver + CHANGELOG.
- **testcontainers/Docker-in-CI** — all container tests behind `containers` + `docker_available()`;
  default `cargo test` stays credential- and Docker-free.
