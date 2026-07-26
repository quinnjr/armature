# Workflow 5 — Cloud Platforms (plan)

**Spec:** `docs/superpowers/specs/2026-07-21-workflow5-cloud-platforms-design.md`
**Scope:** 43 findings (6C / 19W / 18I) across `armature-{aws,azure,gcp,lambda,cloudrun,azure-functions}`.
**Baseline:** all six crates build clean with `--all-features` on stable (verified 2026-07-21).

## Execution model

Six **edit-only** subagents, one per crate, on **disjoint file sets** (no crate
shares source with another; lambda↔aws and azure-functions↔azure couple only
through a feature flag, not shared files). Each agent implements its crate's
findings Critical → Warning → Info, adds the failing-against-old regression
tests, and runs its own `cargo check/clippy/test -p <crate> --all-features`
before returning. Agents do **not** touch `CHANGELOG.md`, `TODO.md`, versions,
or run `git`/`fmt` — the coordinator does the single central gate + bookkeeping
+ commit, per the repo's parallel-fix discipline.

## Per-crate tasks

### Task A — `armature-azure` (2C / 4W / 3I)
- **C:** ServiceBus — add `servicebus_client` field, a `"servicebus"` arm in
  `initialize_enabled_services` building `ServiceBusClient` from
  `servicebus_namespace` + credential, a `servicebus()` accessor
  (`services.rs:35,115`). **C:** connection-string auth — check whether
  `azure_storage_blob/queue 1.0` supports connection-string/shared-key at the
  storage layer; implement if so, else remove `ConnectionString` variant +
  `connection_string()` builder + `from_env` branch + README claim
  (`services.rs:103`).
- **W:** `StorageAccountKey`/`account_key()` and `Environment` credential — the
  identity SDK dropped both; remove or re-doc to the developer-tools chain
  (`services.rs:104,85`). README + `lib.rs` doc examples → real
  `AzureConfig::builder()`/`AzureServices` + `blob_service()` accessor
  (`README.md:33`, `lib.rs:28`).
- **I:** remove "App Configuration" README bullet; wire or drop
  `cosmos_database` and `service_configs`.

### Task B — `armature-gcp` (3C / 2W / 3I)
- **C:** implement `init_firestore`/`init_secret_manager`/`init_cloud_run`/
  `init_cloud_functions` + fields + accessors behind their cfg features
  (`config.rs:166`, `services.rs`). **C:** thread `config.credentials`
  (`ServiceAccountJson`/`AccessToken`/`MetadataServer`) into each client builder
  instead of hardcoded ADC (`services.rs:109`). **C:** README → real
  `GcpConfig::builder()`/`GcpServices::new` API (`README.md:22`).
- **W:** apply `emulator_host` as endpoint override in init_* (`services.rs:111`);
  pass `project_id` into pubsub/storage builders (`services.rs:132`).
- **I:** correct "loaded lazily" doc (init is eager) or make it lazy; wire or drop
  `service_configs`; add accessor-error-contract tests.

### Task C — `armature-lambda` (1C / 3W / 2I)
- **C:** populate `authorizer_claims` — V2 from `v2.authorizer.jwt.claims`, V1
  from the proxy request-context claims map (`request.rs:79,152,169`).
- **W:** extract `path_parameters` (V1 proxy field, V2 from route/rawPath) and
  `stage_variables` (`request.rs:70,121`); `impl_request_handler!` forwards the
  full `LambdaRequest` (headers/query/context), not just method/path/body
  (`runtime.rs:168`).
- **I:** README → real `LambdaRuntime::new`/`with_config` + `Application` API
  (`README.md:39`); add the crate's first tests for the conversion contracts.

### Task D — `armature-cloudrun` (0C / 4W / 3I)
- **W:** README → real `CloudRunConfig::from_env()`/`init_tracing()`/`HealthCheck`/
  `wait_for_shutdown()` (or add a `CloudRunApp` wrapper) (`README.md:21`);
  health-check — provide a real hyper endpoint/handler OR soften the "built-in
  endpoint" claim to a status-computation helper (`health.rs:62`); `service_url`
  — read `CLOUD_RUN_URL`/metadata instead of the fabricated formatting, or drop
  (`config.rs:116`); `InstanceMetadata::fetch` — return `Err` on unreachable
  metadata server instead of silent all-`None` (`metadata.rs:24`).
- **I:** fetch the six metadata fields concurrently (`metadata.rs:42`); fix or
  re-doc the from_env vars Cloud Run does not set (`config.rs:68`); re-doc or
  verify tracing/Cloud-Trace correlation (`lib.rs:116`).

### Task E — `armature-azure-functions` (0C / 4W / 3I)
- **W:** remove Timer/Queue/Blob trigger README+lib claims (HTTP-only runtime) or
  implement envelope routing (`README.md:9`, `lib.rs:90`); re-export+wire the
  `bindings` types or delete them and soften the claim; enforce
  `max_request_size` (413) in `handle_http_request` (`config.rs:19`,
  `runtime.rs:189`); apply `timeout_seconds` via `tokio::time::timeout` → 504
  (`config.rs:21`).
- **I:** fix request-logging level mismatch (`debug!` under `info` filter)
  (`runtime.rs:100`); make `from_json` parse the real custom-handler envelope or
  re-doc + wire it (`request.rs:110`); add the crate's first tests (base_path
  stripping, base64 body round-trip, response conversion).

### Task F — `armature-aws` (0C / 2W / 4I)
- **W:** `build_sdk_config` — honor `Environment`
  (`EnvironmentVariableCredentialsProvider`), `WebIdentity`, `IamRole` (IMDS)
  distinctly, or collapse the enum to the honored variants + doc
  (`services.rs:143`); README → real `AwsConfig::builder()`/`AwsServices` +
  `s3()` accessor (`README.md:26`).
- **I:** implement or delete the placeholder `pub mod s3` (`s3.rs:2`); `from_env`
  read credential/service-enable env (or re-doc) (`config.rs:72`); accessor
  fast-path read-lock before the write-lock init (`services.rs:237`); add the
  crate's first tests (credential-source branching, is_enabled gating,
  force-path-style).

## Coordinator sequence (after agents return)

1. Central gate: `cargo fmt --all`; `clippy --workspace --all-targets --features
   full-with-saml -D warnings`; per-crate `clippy --no-default-features`;
   `cargo test -p {aws,azure,gcp,lambda,cloudrun,azure-functions} --all-features`;
   `cargo test --workspace --features full-with-saml` (Docker-required) to prove
   no cross-crate regression; `cargo audit`; MSRV note (cloud crates excluded —
   above-1.89 SDKs, pre-existing).
2. Bookkeeping: tick `TODO.md` boxes for the six crates; decrement totals
   (130 → 130 − closed); per-crate version bumps + root pins; CHANGELOG entry.
3. Commit one squashable feature commit; hold for the pre-merge review window.

## Definition of done

All 43 boxes ticked or explicitly deferred with rationale; six crates green
under `--all-features`; strict workspace clippy/fmt green; default workspace
test credential-free; PR to `develop`.
