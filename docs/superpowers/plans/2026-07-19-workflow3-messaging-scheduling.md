# Workflow 3 — Messaging & Scheduling: Implementation Plan

**Spec:** `docs/superpowers/specs/2026-07-19-workflow3-messaging-scheduling-design.md`
**Findings:** `.superpowers/sdd/wf3-findings.md` (51: 10C/25W/16I)
**Branch:** `feature/wf3-messaging-scheduling` → PR to `develop` (HELD for user audit window)

## Execution model

One implementation agent per crate (disjoint file trees → safe parallel, edit-only), each handling
that crate's Critical → Warning → Info findings and adding a failing-first regression test per fixed
unit. A central coordinator runs fmt + strict clippy + tests + audit + MSRV, then a code-review
round, then the audit battery, then opens the PR held for the user.

Order of priority (correctness/data-loss first): **queue → cron → discovery → webhooks → messaging →
events → distributed.** Crates are disjoint so agents run in parallel; priority governs review depth
and model tier (opus for queue + cron concurrency correctness).

## Tasks (per crate)

### T1 — armature-queue (3C/2W/4I) [opus]
1. `register_handler` + `register_cpu_intensive_handler` register synchronously (async `.await` /
   `try_write`; never `block_on` in async). Breaking: these become `async`.
2. `process_batch` re-enqueues (or `fail`s) a type-mismatched job — no orphaned job in the
   processing set. Add a type-filtered dequeue if cleaner.
3. README → real `register_handler`/`WorkerConfig` API; add `Queue::enqueue_in`/`enqueue_at`.
4. `process_batch` success-count log fix; `max_size` counts delayed+processing; `dequeue` skips the
   `move_delayed_jobs` Lua when nothing is due.
5. Redis-container tests: register→start→enqueue→complete (0 orphans); retry-with-backoff;
   dead-letter routing; delayed promotion; priority ordering.

### T2 — armature-cron (3C/4W/2I) [opus]
1. `matches` → `schedule.after(&(time - 1s)).next() == Some(time)` (whole-second truncation).
2. `add_job` registers inline, returns `Err(JobAlreadyExists)` on duplicate (no detached spawn).
3. `start` clones needed state under a short-lived lock, runs `execute().await` without the map
   lock, re-acquires briefly to update status/next_run/last_run/count.
4. `max_concurrent_jobs` enforced via `tokio::sync::Semaphore`; `run_missed_jobs` implemented (fire
   past-due on startup) or field removed.
5. lib.rs docs (drop retry/hooks claims) + README (`add_job(|ctx|)`, 6-field cron) + drop Timezone
   Support claim. Add `matches` unit test.

### T3 — armature-discovery (2C/2W/2I) [sonnet]
1. etcd composite key `service_name_prefix(&name)+&id` in register/get_service/deregister so
   `discover` range-scan matches.
2. `list_services` → real `/v3/kv/range` over the whole prefix, decode ServiceInstance, distinct
   names (or explicit Unsupported).
3. consul `discover` → `?passing=true` (or filter Checks); README → real API, drop Kubernetes.
4. `health_check` returns/represents `HealthCheckFailed` on transport error (not `Ok(false)`).
5. StubServer round-trip test (register→discover→get_service) for etcd + consul passing-filter.

### T4 — armature-webhooks (2C/4W/2I) [sonnet]
1. README: remove Idempotency + Stripe/GitHub/Slack claims (or implement); Quick Start → real
   `WebhookReceiver::new(secret)` / `handler(filter, cb)` / `handle(payload, sig)`.
2. Thread `config.signing_algorithm` into `WebhookSignature::sign` (dispatch `compute_hmac_sha512`);
   source `timestamp_tolerance` from `WebhookConfig`; correct `WebhookClient::send` doc/behavior
   (unsigned unless secret, or sign from a client default).
3. `truncate_string` on a char boundary (untrusted bodies must not panic). Add a multibyte test.
4. `dispatch` concurrent fan-out; `verify_from_headers` case-insensitive lookup.

### T5 — armature-messaging (0C/7W/2I) [sonnet]
1. Kafka: Manual ack commits offset (`commit_message`/`store_offset`) on Success; combine
   `config.enable_auto_commit` with `ack_mode`.
2. NATS: apply JWT/NKey (`jwt_with_key_pair`), JetStream (`jetstream::new`), `max_reconnects`/
   reconnect tuning in `connect_with_config`; preserve non-reserved custom headers on receive
   (`nats_message_to_message` catch-all like Kafka).
3. MqBridge: route per-call `topic` (not the fixed config endpoint); cache the publisher
   (OnceCell/Mutex) instead of building one per publish.
4. RabbitMQ: `connect_with_config(RabbitMqConfig)` honoring vhost/publisher_confirms/channel_pool_size.
5. `publish_with_options` applies options where meaningful (or documents no-op); remove the orphaned
   never-compiled broker.rs/traits.rs/message.rs (or wire them — lean remove).
6. Unit tests for the header round-trip + `enable_auto_commit` derivation; live broker tests
   `#[ignore]` where no CI harness.

### T6 — armature-events (0C/3W/2I) [sonnet]
1. `EventBus::publish`: when `continue_on_error == false`, run handlers sequentially and stop on the
   first error (or reword the doc precisely — prefer implementing the stop).
2. README features (drop Event Sourcing/Replay — they live in armature-eventsourcing); lib.rs docs
   describe the trait-based `TypedEventHandler`/`subscribe` API (drop decorator claim); README Quick
   Start compiles (two type params + `TypedEventHandler::new`).
3. Tests: `HandlersFailed` Err path + sequential branch.

### T7 — armature-distributed (0C/3W/2I) [sonnet]
1. Add a `LockGuard` TTL-renewal watchdog (token-guarded `PEXPIRE` on an interval < ttl) so
   "automatic lock renewal" holds; make `LockGuard::drop` safe off-runtime + best-effort documented.
2. README Quick Start → real `RedisLock::new`/`acquire`/`LeaderElection` surface; Features list drop
   Rate Limiting/Caching/Circuit Breaker.
3. Redis-container tests: lock mutual-exclusion (SET NX), token-guarded release, single-leader.

## Verification (central, after all tasks)

1. `cargo fmt --all`.
2. Strict gate: `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings -A
   clippy::collapsible_if -A clippy::result_large_err -A dead_code -A clippy::useless_vec -A
   clippy::unwrap_or_default`.
3. `cargo test` for the 7 crates + any consumers of changed APIs (Docker present → container tests
   run).
4. `cargo audit`; MSRV `cargo +1.89 check` for the touched crates.
5. Semver: bump each crate with a breaking change (queue/cron async signatures, any removed
   surface) to its next 0.x minor; update internal dependent version pins.
6. CHANGELOG `## [Unreleased]`: Added/Changed(breaking)/Fixed/Security per crate.
7. Tick the 51 `TODO.md` boxes; update the summary table rows to 0|0|0.

## Then

Code-review round (7 dimensions) + `/simplify` `/optimize` `/audit` as the user's pre-merge audit
window. **Hold at the PR to `develop`** — do not auto-merge; ask the user.
