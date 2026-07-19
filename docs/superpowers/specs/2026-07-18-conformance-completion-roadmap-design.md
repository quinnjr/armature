# Conformance Completion Roadmap

**Date:** 2026-07-18
**Status:** Approved decomposition; Workflow 0 to be planned next
**Source:** `TODO.md` (per-crate conformance & efficiency audit — 470 findings across 62 crates: 82 Critical, 255 Warning, 133 Info; 161 hollow/divergent units)

## Problem

A conformance audit of every workspace crate found that Armature advertises a large
feature set its code does not implement. 161 units are **hollow** (validate/acquire,
then return success without doing the work) or **divergent** (do something other than
the claim). Examples: `armature-acme`'s entire certificate flow returns `("", "")` while
the README Quick Start writes empty `cert.pem`/`key.pem`; cloud, messaging, provider,
and data crates carry stub methods returning canned `Ok`/defaults. The framework's
public promises are not trustworthy.

## Goal

Make every advertised unit real. For the 161 hollow/divergent units and the remaining
Warning/Info findings, **implement the missing behavior** (decision: implement
everything, not honesty-triage). Findings whose fix is doc-only (`reconcile: claim`) or
test-only (`reconcile: test`) are handled in the same passes. When complete, every
`TODO.md` checkbox is ticked and the code does what its name, docs, types, tests, and
contracts claim.

Non-goals: new features beyond what is already advertised; rearchitecting `armature-core`
(it audited effectively clean after the July 2026 hardening pass); changing the
NestJS/Angular-style conventions.

## Approach

Decompose into **9 domain workflows** plus a **test-foundation prerequisite
(Workflow 0)**. Each workflow is an independent spec → plan → execute → PR cycle that
implements its crate group's findings **Critical → Warning → Info**, verified with local
stub servers and testcontainers.

### Verification strategy (decided)

Deterministic, offline, CI-safe. No live third-party credentials in the default test run.
- **Protocol/HTTP integrations** (ACME, cloud REST, OAuth, webhooks, mail APIs, payment
  gateways): assert request shape and script responses against a local hyper stub server.
- **Datastores** (Redis, Postgres, OpenSearch): `testcontainers` behind a feature/env gate.
- **AWS-shaped services:** LocalStack; **Azure blob/queue:** Azurite, where it pays off.
- **ACME:** a Pebble-based harness.
- Live-service tests remain `#[ignore]`d. Every implemented unit gets a regression test
  that **fails against the current hollow code**.

### Conventions (all workflows)

- Heavy/native integrations stay behind their existing feature flags; the default build
  stays lean and **rustls-only** (no OpenSSL/native-tls).
- Per-crate `tokio` feature subsets (no `features = ["full"]`), per `AGENTS.md`.
- One PR per workflow to `develop`, each with a `CHANGELOG` entry.
- `TODO.md` checkboxes are ticked as findings land; it is the burn-down tracker.
- Strict pre-commit gate (`cargo fmt`, `clippy --workspace --all-targets --features
  full-with-saml -D warnings`) must pass.

## The workflows

| # | Workflow | Crates | Crit | Warn | Info |
|---|---|---|---:|---:|---:|
| 0 | **Test Foundation** (prereq) | new `armature-testkit` dev-crate | — | — | — |
| 1 | Auth & Security | auth, jwt, security, siem, mcp | 10 | 17 | 8 |
| 2 | Data & Persistence | diesel, seaorm, eventsourcing, cqrs, redis, tenancy, opensearch | 8 | 21 | 11 |
| 3 | Messaging & Scheduling | messaging, queue, events, webhooks, cron, distributed, discovery | 10 | 25 | 16 |
| 4 | Certificates (ACME) | acme | 8 | 2 | 1 |
| 5 | Cloud Platforms | aws, azure, gcp, lambda, cloudrun, azure-functions | 6 | 19 | 18 |
| 6 | Delivery Providers | mail, push, payments, storage, files | 7 | 33 | 15 |
| 7 | Web & API | graphql, graphql-client, grpc, openapi, websocket, http-client | 7 | 25 | 13 |
| 8 | Platform Services | cache, ratelimit, compression, metrics, opentelemetry, audit, analytics, config, features, i18n, validation, toon, collab, admin | 10 | 65 | 28 |
| 9 | Core & Tooling | core, proc-macro, macros, macros-utils, testing, app, cli, ferron, rhai | 16 | 48 | 23 |

**Execution order:** 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9. The test foundation unblocks
everyone; Auth and Data are the most depended-on and highest criticality-density; ACME is
a dramatic self-contained early win; Platform and Core/Tooling are largest and run once
the patterns are proven. Order is adjustable per workflow readiness.

### Anatomy of a domain workflow

1. **Spec** — turn the crate group's audit findings into concrete requirements.
2. **Plan** (`writing-plans`) — one task per finding or tight cluster, Critical first.
3. **Execute** — subagent fan-out; each task implements the behavior plus the failing
   regression test.
4. **Verify** — `cargo test` + strict clippy + the domain's stub/container tests green.
5. **PR** — one PR to `develop` with a `CHANGELOG` entry; tick the `TODO.md` boxes.

## Workflow 0 — `armature-testkit` (planned next)

The shared verification substrate every later workflow depends on. A **dev-oriented crate**
(`publish = false`) providing reusable, deterministic test harnesses so integration logic
can be proven without live credentials.

**Requirements:**

1. **HTTP stub server** — spin up a local hyper server on an ephemeral port; register
   scripted responses matched by method + path (+ optional body/header assertions);
   record received requests so a test can assert the client sent the right request
   (headers, body, auth). Returns the bound `SocketAddr`/base URL. Clean async shutdown
   on drop. This covers ACME, cloud REST, OAuth userinfo/introspection, webhooks, mail
   APIs, and payment gateways.
2. **Testcontainer helpers** — thin async wrappers over `testcontainers` for Redis and
   Postgres (and OpenSearch), gated behind a `containers` feature and/or an env flag
   (e.g. `ARMATURE_TESTCONTAINERS=1`) so the default `cargo test` never requires Docker.
   Each returns a ready connection URL.
3. **ACME/Pebble harness** — start a Pebble test-CA container (behind the same gate),
   exposing its directory URL and trust anchor, so the ACME flow can be exercised
   end-to-end offline.
4. **LocalStack / Azurite helpers** (optional, same gate) — for AWS-shaped and Azure
   blob/queue services, added when Workflow 5 needs them (can be deferred to that
   workflow if it keeps Workflow 0 tight).
5. **Ergonomics** — helpers return RAII guards that shut down/stop containers on drop;
   a `skip_if_no_docker!()`-style macro so gated tests self-skip cleanly with a message
   rather than failing when Docker is absent.

**Verification of Workflow 0 itself:** the stub-server harness has its own unit tests
(request recording + response scripting round-trip) that run in the default suite; the
container helpers have `#[ignore]`d/gated smoke tests.

**Boundaries:** `armature-testkit` depends only on `tokio`, `hyper`/`hyper-util`,
`testcontainers`, and `bytes` — no dependency on the crates it will test, to avoid cycles.
It is added to the workspace `members` and excluded from published output.

## Success criteria

- Every Critical and Warning finding in `TODO.md` is either implemented (with a passing
  regression test that failed against the old code) or, for `reconcile: claim`/`test`
  findings, reconciled by doc/test change.
- The default `cargo test --workspace` stays green and credential-free; gated container
  tests pass when Docker is available.
- Strict clippy and fmt gates pass on every workflow PR.
- `TODO.md` reaches zero open Critical/Warning boxes.

## Risks

- **Scale.** ~470 findings is a large program; the domain slicing and per-workflow PRs
  keep each increment reviewable.
- **External API drift.** Stub servers encode assumptions about vendor APIs; where the
  official SDK exists (AWS/Azure/GCP), prefer implementing against the SDK and stub only
  our glue.
- **Docker in CI.** Container tests must be gated so the default pipeline never depends
  on Docker; CI can opt into a `containers` job.
