# Workflow 5 — Cloud Platforms (design)

**Date:** 2026-07-21
**Status:** Approved for planning
**Source:** `TODO.md` — the six cloud-platform crates (`armature-aws`, `-azure`, `-gcp`, `-lambda`, `-cloudrun`, `-azure-functions`). 43 findings: **6 Critical · 19 Warning · 18 Info**.
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 5 row).

## Problem

The cloud crates are thin conformance wrappers over vendor SDKs (`aws-sdk-*`,
`azure_*`, `google-cloud-*`, `lambda_runtime`), and the audit found the same
recurring gaps the roadmap predicted for this group:

1. **Advertised-but-unwired services.** Builders expose `enable_servicebus()` /
   `enable_firestore()` / `enable_secret_manager()` / `enable_cloud_run()` /
   `enable_cloud_functions()` that flip a name into an `enabled_services` set,
   but the corresponding `initialize_enabled_services` arm falls through a
   `_ => {}` catch-all — no field, no client, no accessor. Enabling the service
   is a silent no-op. (azure ServiceBus, gcp ×4 — all **Critical**.)

2. **Credential sources accepted then ignored.** `CredentialsSource` enums
   advertise distinct variants (AWS `Environment`/`IamRole`/`WebIdentity`,
   Azure `StorageAccountKey`/`ConnectionString`/`Environment`, GCP
   `ServiceAccountJson`/`AccessToken`/`MetadataServer`) but the build path
   ignores the selection and falls back to the default credential chain / ADC /
   `DeveloperToolsCredential`. Selecting a non-default source does nothing.

3. **Config knobs read nowhere.** `max_request_size`, `timeout_seconds`,
   `emulator_host`, `cosmos_database`, `service_configs`, project-id,
   Cloud Run resource-limit env vars — settable, defaulted, documented, and
   consulted by no code path.

4. **Runtime data never populated.** Lambda `claims()`/`path_parameters`/
   `stage_variables` always return empty maps; the `impl_request_handler!`
   macro drops headers/query/context; Cloud Run `service_url` fabricates a
   wrong-but-plausible URL; `InstanceMetadata::fetch` swallows every error into
   all-`None`.

5. **READMEs / doc examples that do not compile.** Every crate's README Quick
   Start documents types and methods that never existed (`S3Client::new`,
   `BlobClient::new`, `StorageClient::new`, `CloudRunApp`,
   `LambdaRuntime::api_gateway`, …); the real surface is the
   `*Config`/`*Services` builder pattern shown correctly in `lib.rs`.

6. **No tests at all** in `armature-aws`, `-azure-functions`, `-cloudrun`,
   `-gcp`, `-lambda` — every load-bearing conversion/branching claim is
   unverified.

## Goal

Make every advertised unit in the six crates real, per finding, following the
roadmap's decision to **implement rather than honesty-triage** — *except* where
the vendor SDK has genuinely removed the capability, in which case the honest
fix (and the fix the finding itself prescribes) is to remove the dead
variant/knob/claim. Each `reconcile: code` finding gets the behavior plus a
regression test that fails against the current hollow code; each
`reconcile: claim` finding gets the doc/README corrected to the real API.

### Implement-vs-remove decision rule

The implementing agent decides per credential/service finding by reading the
**actual pinned SDK version** in the crate's `Cargo.toml`:

- **Implementable against the pinned SDK → implement it.** Azure Service Bus
  (`azure_messaging_servicebus 0.21` is already a declared dep + re-export),
  the four GCP `enable_*` services (`google-cloud-*` deps + cfg features
  already declared), AWS `Environment`/`WebIdentity`/`IamRole` providers
  (`aws-config` exposes `EnvironmentVariableCredentialsProvider`, web-identity,
  and IMDS providers), GCP explicit-credential threading, emulator-host
  endpoint override.
- **Removed by the SDK → reconcile: claim (remove the variant + doc).** Azure
  `azure_identity 1.0` dropped `EnvironmentCredential` and shared-key
  credentials; those findings (`StorageAccountKey`, `Environment`,
  connection-string via identity) are explicitly `reconcile: claim`. The agent
  removes the variant/builder/README claim and routes remaining callers to the
  supported `DeveloperToolsCredential`/DefaultAzureCredential chain. If the
  `azure_storage_*` SDK itself supports connection-string or shared-key at the
  storage layer (independent of `azure_identity`), prefer implementing it
  there; the agent must check before deleting.

Non-goals: standing up new cloud services beyond what is advertised; a real
Cloud Run / Azure Functions gateway server where only client scaffolding is
claimed (soften the doc instead); changing feature-flag or rustls-only
conventions.

## Verification strategy

These crates are **not built in default CI** and pull vendor SDKs whose MSRV
exceeds the workspace's 1.89 (`aws-types`/`aws-smithy-*` require 1.94) — so the
workspace MSRV gate legitimately excludes them, and the per-crate gate is
`cargo check/clippy/test -p <crate> --all-features` on stable.

- **Offline, deterministic, credential-free by default.** The bulk of the
  high-value findings — Lambda claims/path-params/stage-vars/macro,
  Azure Functions request parsing / size / timeout, Cloud Run config-from-env /
  metadata error contract / URL, all credential-**source selection** branching,
  all config-knob threading — are pure request/config/credential-construction
  logic testable with **no live service**: assert the constructed
  provider/endpoint/parsed-struct, or drive the runtime with a synthetic
  event/`hyper::Request`. Every such finding gets a unit test.
- **Client-init findings** (azure ServiceBus, gcp `enable_*`) are verified by
  construction: `enable_x()` → `Services::new(config)` yields a working
  `x()` accessor (against an emulator endpoint / dummy credentials where the
  SDK builds a client without a network round-trip), gated behind
  `skip_if_no_docker!` + LocalStack/Azurite/emulator only where a live call is
  unavoidable. No live vendor credentials in the default run; live-only paths
  stay `#[ignore]`d.
- **README / doc-example findings** are verified by making the example a real
  compiled doctest (or by matching it line-for-line to the `lib.rs` example
  that already compiles), so the corrected example cannot silently drift again.

## The six crates and their finding clusters

| Crate | Crit | Warn | Info | Dominant clusters |
|---|---:|---:|---:|---|
| `armature-aws` | 0 | 2 | 4 | credential-source variants; README; accessor RwLock; `s3` stub mod; from_env; no tests |
| `armature-azure` | 2 | 4 | 3 | ServiceBus init (**C**); connection-string auth (**C**); shared-key/env credential removal; README+lib doc; cosmos_database/service_configs unread |
| `armature-gcp` | 3 | 2 | 3 | `enable_*` ×4 init (**C**); credentials threading (**C**); README (**C**); emulator_host; project_id; eager-not-lazy doc; service_configs |
| `armature-lambda` | 1 | 3 | 2 | authorizer claims (**C**); path_parameters; stage_variables; `impl_request_handler!` macro; README; no tests |
| `armature-cloudrun` | 0 | 4 | 3 | README `CloudRunApp`; health-endpoint claim; `service_url` fabrication; metadata fetch error contract; from_env vars; sequential fetch; tracing correlation |
| `armature-azure-functions` | 0 | 4 | 3 | trigger claims; `bindings` module; max_request_size; timeout_seconds; request-logging level; from_json envelope; no tests |

## Conventions (inherited)

- Heavy SDKs stay behind existing feature flags; default build lean and
  rustls-only.
- Per-crate `tokio` feature subsets; strict pre-commit gate
  (`fmt`, `clippy --workspace --all-targets --features full-with-saml -D
  warnings`).
- One PR to `develop`, one CHANGELOG entry; tick the `TODO.md` boxes and update
  totals; version-bump each crate whose public surface changes (0.x → minor for
  breaking, patch for additive/behavior-only), refresh root pins.

## Success criteria

- Every Critical and Warning finding implemented (with a failing-against-old
  regression test) or reconciled by doc change; Info likewise or explicitly
  deferred with rationale in the CHANGELOG.
- `cargo test -p <crate> --all-features` green for all six; strict workspace
  clippy + fmt green; default `cargo test --workspace` stays credential-free.
- `TODO.md` cloud-crate boxes ticked; totals decremented; `develop`-bound PR.

## Risks

- **SDK surface drift.** The vendor SDKs are pinned at specific (some pre-1.0)
  versions; the implement-vs-remove rule is decided against the pinned version,
  not the latest docs.
- **MSRV / CI gap.** These crates already sit outside the MSRV-1.89 gate and the
  default CI matrix (pre-existing, tracked in TODO's CI-coverage finding). WF5
  does not close that gap; it is flagged in the CHANGELOG as a known follow-up.
- **Emulator coverage.** LocalStack/Azurite/GCP-emulator tests are Docker-gated
  and self-skip; the credential-free unit tests carry the real conformance
  burden so coverage does not depend on Docker.
