# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### HTTP QUERY method (`armature-core`, `armature-proc-macro`, `armature-app`, `armature-testing`)
- `HttpMethod::QUERY` variant for the IETF `QUERY` method (safe, idempotent query with a request body — draft-ietf-httpbis-safe-method-w-body)
- `Router::query()`, micro API `RouteBuilder::query()` / `micro::query()`, and the `#[query("/path")]` route decorator
- `TestClient::query()` and `armature-app` builder support for `QUERY`
- Response caching for `QUERY`: `CacheKey` now includes a body hash for `QUERY` requests, and `QUERY` is a cacheable method by default, so distinct request bodies are distinct cache entries

#### Request/error hardening (`armature-core`)
- `Application::with_max_body_size()` and `DEFAULT_MAX_BODY_SIZE` (10 MB): request bodies over the limit are rejected with `413` before buffering
- Application-level guards are now stored and evaluated before routing; a guard declared by a module is scoped to that module's controllers' routes
- `Application::with_socket_tuning()` applies per-socket TCP options (opt-in; see `epoll_tuning::EpollConfig`)
- `RequestRoles` request extension and a JWT authentication middleware in `armature-auth` that verifies tokens and populates `UserContext`/`RequestRoles`

#### Real implementations of previously-stubbed modules (`armature-core`)
- `io_uring::IoUringRuntime` — real ring integration behind the (off-by-default, Linux-only) `io-uring` feature
- NUMA `bind_to_node`/`bind_to_local_node`/allocator now issue real `set_mempolicy`/`mbind` syscalls
- HMR now runs a real WebSocket notification server for browser auto-refresh
- OAuth2 `userinfo`/introspection validators (`armature-mcp`) and JWT tenant resolution (`armature-tenancy`) are implemented (previously placeholders)

#### Auth & Security features (`armature-auth`, `armature-siem`, `armature-mcp`)
- SAML `create_auth_request` resolves the IdP SSO endpoint from configured metadata (no more hardcoded `idp.example.com`) and signs the `AuthnRequest` when SP key/certificate are configured; `<AttributeStatement>` attributes are parsed from verified assertions. SP metadata (`SamlServiceProvider::get_metadata`) now includes a signing `KeyDescriptor`, `SingleLogoutService`, and `ContactPerson` when the corresponding config is set (previously silently dropped)
- `armature-auth` strategies are real: `LocalStrategy` verifies passwords via bcrypt/argon2 (`PasswordHasher`), `JwtStrategy` verifies tokens via `armature-jwt`'s `JwtManager`; `AuthGuard::extract_user` reads the verified `UserContext` extension; Microsoft Entra Graph `/me` mapping (`id`→`sub`, `mail`/`userPrincipalName`→`email`, `displayName`→`name`); OIDC `id_token` is surfaced from the token endpoint; API keys enforce per-key rate limits
- `armature-siem`: retry with exponential backoff (honoring `max_retries`/`retry_delay` and `RateLimited` retry-after), time-based batch flush, optional gzip request compression, custom CA (`ca_cert_path`) for the HTTP transport, Elastic `cloud_id` decoding, and distinct CEF custom-field slots (`cs1..cs6`/`cn1..cn3`) with JSON overflow. New `SiemConfig::workspace_id` field + builder method (the Log Analytics workspace ID used by Azure Sentinel SharedKey signing); `SiemConfig` is now `#[non_exhaustive]`
- `armature-mcp`: cursor-based pagination for `tools/list` and `resources/list`; the `#[mcp]` attribute macro is exported and emits a function-pointer tool handler. `McpService::with_tool_provider`/`with_resource_provider` register dynamic `McpToolProvider`/`McpResourceProvider` implementations alongside the compile-time `#[mcp]` inventory — their tools/resources are merged into `tools/list`/`resources/list` and dispatched by `tools/call`/`resources/read` (compile-time registry names/URIs take precedence on collision)
- `armature-jwt`: `JwtManager::verify_only` constructor (verification without a signing key) for consumers that only validate tokens

#### Data & Persistence features (`armature-diesel`, `armature-seaorm`, `armature-redis`, `armature-tenancy`, `armature-opensearch`)
- `armature-diesel`: pool constructors now apply the advertised knobs — `connect_timeout` (deadpool `create_timeout`), `min_idle`/`max_lifetime`/`idle_timeout` (bb8), `application_name`/`ssl_mode` (Postgres libpq URL params), and `test_on_checkout` (deadpool `RecyclingMethod::Verified` + bb8 `test_on_check_out`); `TransactionGuard::commit()`/`rollback()` issue real SQL
- `armature-redis`: real Redis Cluster support (`redis::cluster` manager) when `cluster` is configured; `command_timeout` enforced per command; `CLIENT SETNAME` issued on checkout when `connection_name` is set; `connection_url` honors the configured `database` and upgrades to `rediss://` when `tls` is set
- `armature-seaorm`: `DatabaseConfig::to_connect_options` applies `sqlx_log_level`; `from_env` reads `DATABASE_IDLE_TIMEOUT`; backward keyset pagination emits `prev_cursor`; new `TransactionExt::begin_transaction_with_options` consumes `TransactionOptions` (isolation/read-only via `begin_with_config`, `deferrable` via `SET TRANSACTION DEFERRABLE` on Postgres)
- `armature-tenancy`: `Tenant` gained a `display_name` field (persisted by `create`/`update`); `CacheProvider::keys_with_prefix` enables real `TenantCache::clear_tenant` (prefix scan/delete)
- `armature-opensearch`: real AWS SigV4 signing via the opensearch crate's `aws-auth` feature (new `OpenSearchConfig::aws_credentials_provider`); TLS config applied to the transport; a `bulk_execute` method for `Vec<BulkOperation>`

#### Delivery provider features (`armature-mail`, `armature-push`, `armature-payments`, `armature-storage`, `armature-files`)
- `armature-mail`: `TeraEngine` and `MiniJinjaEngine` are now real `TemplateEngine` implementations (they were empty structs); SES sends raw MIME so **attachments are actually delivered**; custom headers and `Email::priority` reach SMTP/SES/Mailgun/SendGrid; inline attachments carry a Content-ID inside `multipart/related` so `cid:` references resolve; the queue enforces `job_timeout`, batches `pop` with `MGET`, and pipelines `enqueue_batch`.
- `armature-payments`: `ProcessorConfig` retry/idempotency/logging are wired (stable idempotency key across retries; declines are never retried); every Stripe call site checks HTTP status and maps a typed provider error; subscription `price_id`/`quantity` come from the real line items; `PaymentSource::PaymentMethod` routes via PaymentIntents and `Bank` via `source`; PayPal plan changes use `revise`; Braintree sends partial capture amounts, while PayPal returns `PaymentError::Unsupported` for a partial capture (its Orders v2 capture endpoint has no `amount` field, so the previous code sent a field the API ignores); Braintree lists real vaulted payment methods.
- `armature-storage`: `public_access` applies a predefined ACL, S3 honors `region` and `storage_class`, GCS honors a custom `endpoint` (emulators) and sends `project_id` as the quota/billing project (`x-goog-user-project`) on every object operation, `MultipartConstraints` are enforced and re-exported, and the new `Storage::temporary_url_default` trait method honors each backend's configured signed-URL duration (reachable through `Arc<dyn Storage>`, unlike the per-backend inherent methods it replaces).
- `armature-files`: `OutputFormat::Original` round-trips (so `MultiSizeBuilder::generate()` works at all), `TextWatermark` renders real glyphs, `AutoOrient` applies EXIF orientation, PDF horizontal lines emit a real stroke, and `MultiSizeBuilder` decodes the source once instead of once per size.
- `armature-push`: Web Push maps `Notification::urgency` to the RFC 8030 `Urgency` header, APNS honors a per-notification `topic`, and batch sends use bounded concurrency instead of serial round-trips.

#### Real ACME certificate issuance (`armature-acme`)

The ACME client was a hollow shell — every protocol method returned a placeholder (`("", "")`, an empty `Vec`, or the raw directory URL), so the documented Quick Start silently wrote **empty** `cert.pem`/`key.pem` files and auto-renewal never fired. The full RFC 8555 flow is now implemented and verified end-to-end against a real ACME CA (Pebble, via `armature-testkit::PebbleCa`):

- **JWS/crypto core** (new `armature_acme::jws`): ECDSA P-256 account keys producing ES256 signatures (raw `r||s`), ACME flattened JWS signing with `jwk`/`kid` protected headers, `Replay-Nonce` handling with `badNonce` retry, RFC 7638 JWK thumbprints, `key_authorization`, DNS-01 TXT value derivation, and External Account Binding (HS256) for CAs like ZeroSSL.
- **`register_account`** generates or loads a persisted account key from `account_dir` and performs a real signed `newAccount` (honoring `accept_tos`, `contact_email`, and EAB); **`create_order`** POSTs a real `newOrder`; **`get_challenges`** walks the order's authorizations and returns real challenges with correct key authorizations for the configured `challenge_type`; **`notify_challenge_ready`** POSTs the challenge and polls to `valid`/`invalid`; **`finalize_order`** generates a CSR carrying every configured domain as a SAN, finalizes, polls, and downloads the certificate chain.
- **`should_renew`** parses the leaf certificate's `notAfter` and honors `renew_before_days` (missing or unparseable certificate → renew).
- `AcmeConfig` gained `with_ca_certificate()` to trust a private or test ACME CA (Pebble, Boulder, an internal server). TLS verification is always enforced — there is deliberately no option to disable certificate validation.
- Private key material (account key, issued certificate key) is created with `0600` permissions **before** any bytes are written, rather than `chmod`-ed afterwards.

#### Web & API features (`armature-graphql`, `armature-graphql-client`, `armature-grpc`, `armature-openapi`, `armature-websocket`, `armature-http-client`)
- `armature-grpc`: real `GrpcMiddleware`/`tower::Service` implementations for `Timeout`/`RateLimit`/`ConcurrencyLimit`/`LoadShedding`/`Retry` (previously inert config holders); server-side TLS (`tonic`'s `tls-ring`, rustls-only) via `GrpcServerTlsConfig`/`GrpcClientTlsConfig`; real `tonic_reflection` service registration when `enable_reflection` is set; health-check `SERVING` status is now actually reported.
- `armature-graphql-client`: subscriptions now attach their configured headers to the WebSocket handshake (previously silently dropped); `execute_request`/`batch` now genuinely retry on transport/5xx failures; an in-memory response cache honors `caching`/`cache_ttl` for query operations (never for mutations).
- `armature-graphql`: `GraphQLConfig`'s `max_depth`/`max_complexity`/`enable_introspection`/`enable_validation`/`enable_tracing` are now applied to every schema built through the crate via a new `configure()` path.
- `armature-websocket`: server and client now enforce `max_message_size`; the server sends heartbeat pings on `heartbeat_interval` and closes on missed pongs; `connection_timeout` is enforced on the read loop.
- `armature-http-client`: the interceptor system is now wired into `execute()`/`send()`; `RateLimitInterceptor` actually waits out a parsed `Retry-After` instead of only logging. (A separate, entirely unreachable `Middleware`/`MiddlewareChain` module — redundant with the interceptor system above — was later found dead and removed; see the Workflow 7 follow-up conformance pass below.)

#### Messaging & Scheduling features (`armature-messaging`, `armature-queue`, `armature-cron`, `armature-events`, `armature-distributed`, `armature-discovery`)
- `armature-messaging`: Kafka manual-ack mode now commits offsets (`commit_message`) on success and derives `enable.auto.commit` from `config.enable_auto_commit` combined with `ack_mode`; NATS `connect_with_config` applies JWT/NKey auth, JetStream, and reconnect tuning, and preserves custom message headers on receive; `MqBridgeBroker` routes per-call topic and caches its publisher; new `RabbitMqBroker::connect_with_config(RabbitMqConfig)` honoring `vhost`/`publisher_confirms`/`channel_pool_size`
- `armature-queue`: new `Queue::enqueue_in`/`enqueue_at` scheduling convenience methods, plus `requeue`/`backlog_size`/`processing_len`; `max_size` now counts delayed and in-flight jobs; `dequeue` skips the delayed-promotion script when nothing is due
- `armature-cron`: `SchedulerConfig::max_concurrent_jobs` is now enforced via a `Semaphore`, and `run_missed_jobs` fires past-due jobs on startup (both were previously read nowhere)
- `armature-distributed`: `LockGuard` now runs a token-guarded TTL-renewal watchdog so held locks no longer silently expire (matching the advertised "automatic renewal"); `Drop` is safe when the guard is dropped outside a Tokio runtime
- `armature-discovery`: `EtcdDiscovery::list_services` performs a real `/v3/kv/range` scan; `ConsulDiscovery::discover` filters to passing instances

### Changed

- **Version bumps:** `armature-redis`, `armature-diesel`, `armature-seaorm`, `armature-opensearch`, `armature-tenancy`, `armature-eventsourcing`, `armature-queue`, `armature-cron`, `armature-events`, `armature-discovery`, `armature-messaging`, `armature-webhooks`, `armature-push`, `armature-distributed`, `armature-acme`, `armature-payments`, `armature-mail`, `armature-storage`, `armature-files`, `armature-http-client`, `armature-grpc`, `armature-graphql-client`, and `armature-openapi` are bumped to `0.2.0` for the breaking/behavior/additive-API changes listed below (under Cargo's 0.x semver rules a minor bump signals a breaking change); internal and root dependents pin the new versions. `armature-graphql` is bumped to `0.3.0` (not `0.2.0`, since it was already at `0.2.0` from a prior unrelated change) for removing the public `create_merged_schema` function and adding a required `fields: &str` parameter to `FederationGateway::resolve_entities`, both breaking changes under the same 0.x convention. `armature-mcp` is bumped to `0.1.3` (its dependency requirement and `auth.rs` changed on this branch while it still carried the already-published `0.1.2`).
- **Breaking (`armature-queue`):** `Worker::register_handler` and `Worker::register_cpu_intensive_handler` are now `async fn` (were synchronous) and register the handler inline before returning — callers must `.await` them. This closes a race where `register_handler(); start();` could dequeue a job before a fire-and-forget registration task had inserted the handler, and removes a `block_on`-inside-async panic in `register_cpu_intensive_handler`.
- **Breaking (`armature-cron`):** `CronScheduler::add_job` is now `async fn` (was synchronous) and registers the job inline before returning; it now returns `Err(CronError::JobAlreadyExists)` on a duplicate name instead of silently `warn!`-ing and dropping the job. Callers must `.await` it and handle the duplicate error.
- `armature-events`: `EventBus::publish` now honors `continue_on_error(false)` — handlers run sequentially and execution **stops on the first error** (returning `HandlersFailed`), instead of the previous behavior where every handler was spawned up front and all ran to completion regardless. Code relying on the old all-handlers-run behavior with `continue_on_error(false)` should set `continue_on_error(true)`.
- **Breaking (`armature-payments`):** `PaymentProvider::verify_webhook` and `PaymentProcessor::handle_webhook` are now `async` and take a new public `&WebhookHeaders` instead of a `&str` signature (PayPal verification needs five transmission headers plus a network call). `Subscription::customer_id`/`current_period_start`/`current_period_end`/`created_at` are now `Option` (they were previously fabricated when the provider did not supply them); `CreatePaymentMethodRequest` gained `payment_method_nonce`; `StripePaymentIntent` gained public fields. **`default` features now include `paypal` and `braintree`** (previously Stripe-only, which silently skipped every PayPal/Braintree test). Behavior: forged webhooks are now rejected, `PayPal::create_customer` and `Braintree::create_payment_method` (without a client nonce) return explicit unsupported errors instead of fabricating data, and `Stripe::delete_customer` errors on non-2xx. New dependency: `sha1`.
- **Breaking (`armature-mail`):** `Email` gained a public `invalid_addresses` field and `Email::validate()` now rejects addresses the fluent builders failed to parse (they were previously dropped silently, so a send could reach fewer recipients than intended with no signal). `EmailQueueBackend` gained a `push_batch` method with a default implementation (additive for existing implementors). The SendGrid payload now carries a `headers` object.
- **Breaking (`armature-files`):** `OutputFormat::WebP` no longer carries a `quality` field and `Pipeline::to_webp()` no longer takes a quality argument — the underlying `image` encoder only supports lossless WebP, so the parameter was silently ignored; it is removed rather than faked. New dependency `ab_glyph` plus a vendored `DejaVuSans.ttf` (Bitstream Vera License, included) for real text-watermark glyph rendering.
- **Breaking (`armature-storage`):** `AzureBlobStorage::temporary_url` now returns `Ok(None)` instead of a permanent unsigned public URL (the trait contracts a *temporary signed* URL; the old value granted no access to a private container and never expired). `MultipartConstraints` is now exported, and **every** entry point enforces it: `Multipart::new`, `Multipart::from_request`, and `parse_multipart` all apply `MultipartConstraints::default()` (100 MB total / 50 MB per field / 100 fields / 10 files), so previously-accepted oversized or disallowed multipart uploads are now rejected. An earlier revision of this entry claimed enforcement while those three constructors still installed all-`None` counters — the limits only applied to callers who opted in explicitly via `with_constraints`. Callers who genuinely want no limits must now say so with the new `Multipart::unconstrained` / `Multipart::from_request_unconstrained`.
- **Breaking (`armature-storage`):** `LocalStorage` now rejects absolute keys and keys containing `..` with `StorageError::InvalidFileName` — such keys previously escaped `base_path` entirely, allowing reads and writes anywhere on the filesystem. `LocalStorage::list` is now recursive (nested keys created by `put` were previously invisible to it) and propagates errors instead of silently dropping entries. `FileValidator::allowed_types`/`images_only`/`documents_only` now sniff magic bytes, so uploads that lie about their `Content-Type` are rejected where they previously passed; `FileValidator::validate` wraps failures in a new `ValidationError::Rule { rule, source }`, so callers matching on specific variants must match on `err.kind()`. S3 `put` rejects unknown `storage_class`/`default_acl`/`server_side_encryption` values instead of forwarding them verbatim. `AzureBlobConfig::{sas_duration, access_key, connection_string}` were removed — the SDK cannot use key or connection-string auth, so the builders only ever produced a deferred constructor failure, and `sas_duration` had no reader. New dependency: `infer` (magic-byte content sniffing).
- **Breaking (`armature-push`):** `WebPushConfig::public_key` was removed — VAPID signing derives the public key from the private key, and nothing ever read the field. `jsonwebtoken` is now built with `rust_crypto` (required: without a `CryptoProvider`, FCM/APNS JWT signing panicked at runtime), and APNS no longer forces HTTP/2 prior knowledge.
- **Breaking (`armature-acme`):** `AcmeConfig` gained a public `ca_certificate` field, so exhaustive struct-literal construction must be updated (use the `lets_encrypt_*`/`zerossl` constructors plus the `with_*` builders). `save_certificate` now returns an error for empty/non-PEM input instead of writing it, and `order_certificate`/`finalize_order` return a real `AcmeError` on failure instead of `("", "")` — code that treated an empty string as success must now handle the error. `Challenge.token` is `#[serde(default)]` (CAs omit it on some challenge types). New runtime dependency: `x509-parser` (certificate expiry parsing for `should_renew`).
- `armature-push`: Web Push (VAPID) is now sent over the crate's own reqwest (rustls) client instead of `web-push`'s default `isahc`/`curl-sys` (vendored libcurl) client — `web-push` is pulled with `default-features = false`, so `features = ["web-push"]` no longer needs a libcurl C toolchain. On a non-2xx push response the returned `PushError` variant now maps by status (404/410 → `Unregistered`, 429 → `RateLimited` honoring `Retry-After`, 413 → `PayloadTooLarge`). The `apns` feature now enables `reqwest/http2` (APNS requires HTTP/2).
- `armature-discovery`: `ServiceDiscovery::health_check` now returns `Err(DiscoveryError::HealthCheckFailed)` on a transport/network failure instead of `Ok(false)`, so callers can distinguish "endpoint reported unhealthy" from "could not reach the endpoint". `discover` now returns `Ok(vec![])` (not `Err(ServiceNotFound)`) uniformly across all backends when no instances are registered — `ServiceResolver::resolve` still raises `ServiceNotFound`. etcd now maintains a secondary id-index key (`{prefix}__idx/{id}`) so `get_service`/`deregister` are point lookups instead of full-prefix scans — **existing etcd registrations from `0.1.x` must be re-registered after upgrade** to gain the index (and to move to the `{prefix}/{name}/{id}` key layout).
- `armature-distributed`: `LockGuard` gained `is_held() -> bool` and an async `lost()` future so critical sections can detect a lost lease (the renewal watchdog now marks the guard lost on renew failure/expiry — see Security).
- `armature-webhooks`: `SigningAlgorithm` is now `#[non_exhaustive]`; `verify_from_headers` now actually verifies GitHub-style `X-Hub-Signature-256` (`sha256=<hex>`, timestampless HMAC) in addition to the Stripe-style `t=,v1=` scheme.
- `armature-messaging`: new `RabbitMqBroker::connect_with_config(RabbitMqConfig)` and `NatsBroker::jetstream()`/`config()` accessors; the `nats` feature now pulls `nkeys`. In `AckMode::Manual`, Kafka now halts offset commits on the first non-`Success`/handler-error so failed messages are redelivered on restart (at-least-once; idempotent handlers required).
- **Breaking (`armature-auth`):** `AuthStrategy::authenticate`'s `credentials` parameter changed from `&dyn Any` to `&(dyn Any + Send + Sync)` (required for the `async_trait` `Send` future bound). `LocalStrategy::new()` now fails closed (returns `InvalidCredentials`) until a user store is attached via the new `LocalStrategy::with_store()`.
- **JWT refresh tokens now have a distinct, longer expiry.** `generate_token_pair`/`refresh_token` previously issued a refresh token byte-identical to the access token with the same `exp`, so refresh could never succeed after expiry; refresh tokens now expire at `now + refresh_expires_in` and `refresh_token` re-issues with fresh expirations.
- `armature-security` default hardening (Helmet parity): default `X-XSS-Protection` is now `0` (disabled), Expect-CT is no longer enabled by default (deprecated), and CSP supports report-only mode (`Content-Security-Policy-Report-Only`).
- **Breaking (`armature-diesel`):** `TransactionExt` is now `TransactionExt<Connection>` (was an associated type); `transaction`/`transaction_with_isolation` take `AsyncFnOnce` closures (the previous associated-type + HRTB form was uncallable by any real closure). `armature-diesel` gained `deadpool` (feature `rt_tokio_1`) as an explicit optional dependency so `connect_timeout` can be applied. `min_idle`/`max_lifetime`/`idle_timeout` are documented inert on deadpool (bb8-only); `application_name`/`ssl_mode` are Postgres-only. Additionally, `TransactionGuard::commit()` is now `async fn -> DieselResult<()>` (was a synchronous `fn` returning `()`) and a matching `rollback()` was added; both are now defined only under the `postgres`/`mysql` feature gates, so callers must `.await?` the result.
- **Breaking (`armature-redis`):** `RedisPool` and `RedisConnection` are now `Single`/`Cluster` enums (each variant gained fields); `RedisConnection` implements `ConnectionLike` directly instead of `Deref` — downstream code doing `&mut *conn` must drop the deref. `TransactionGuard`/pool `RecyclingMethod` unchanged. Both enums are now `#[non_exhaustive]` (a future mode such as sentinel won't break downstream `match`es); `RedisConnection::new` was removed and `RedisPool` no longer re-exposes the underlying `bb8::Pool` surface (only `get`/`state`/`is_cluster`). Cluster mode now forwards the configured `username`/`password` (percent-encoded) into each seed node and upgrades `redis://`/schemeless seed nodes to `rediss://` under `tls`.
- **Breaking (`armature-redis`):** the no-op `cluster` and `sentinel` cargo features were removed and `full` no longer includes them (`full = ["tls"]`). `cluster` was an empty feature gate that enabled nothing — cluster mode is now selected at runtime via `RedisConfig` (the `redis` crate's `cluster-async` support is always compiled in) — and `sentinel` gated no implementation. A downstream `Cargo.toml` enabling either feature on `armature-redis` will now fail to resolve; drop the feature.
- **Breaking (`armature-seaorm`):** `DatabaseConfig::statement_cache_capacity` was removed (sea-orm 1.1.x's `ConnectOptions` has no corresponding setter; JSON/TOML configs carrying the key still deserialize — it is ignored). `TransactionExt` is now a **sealed** trait — it gained a required `begin_transaction_with_options` method that cannot be given a correct default, so downstream crates can no longer implement `TransactionExt` for their own types; call it on sea-orm's `Database`/`DatabaseConnection` (the intended use).
- **Breaking (`armature-opensearch`):** setting `aws_region` without an `aws_credentials_provider` is now a hard `Validation` error instead of silently sending unsigned requests; configuring both basic auth and `aws_region` is likewise a `Validation` error (SigV4 would otherwise silently override basic auth); a non-`https://` URL is rejected when TLS or SigV4 is configured (loopback exempt). `connect_timeout`/`compression` are documented inert (not expressible via opensearch 2.4's `TransportBuilder`); the previously no-op `max_retries` field + `with_max_retries()` and the unused `futures` dep + `bulk-stream` feature were removed. `OpenSearchConfig` is now `#[non_exhaustive]`. Index/client operations — now including `bulk_index`/`bulk_delete` — surface non-2xx responses (e.g. a 429/503 bulk rejection) as `OpenSearchError::Internal` instead of returning `Ok`/`0`.
- `armature-tenancy`: the `TenantMiddleware` now stores the resolved tenant in request extensions and reads identity only from there (see Security). `TenantManager::update` now applies `display_name` to `Tenant::display_name` instead of overwriting `Tenant::name`/slug.
- **Breaking (`armature-tenancy`):** the public `Tenant` struct is now `#[non_exhaustive]`, so downstream code can no longer build it with a struct literal — construct via `Tenant::new(...)` plus the `with_*` builder methods. This also lets future fields (like the newly added `display_name`) be introduced without further breakage.

- **Guards now enforce.** `RolesGuard` (core) and `RoleGuard`/`PermissionGuard` (`armature-auth`) previously returned `Ok(true)` for any bearer-shaped request. They now **fail closed**, requiring an authenticated `RequestRoles`/`UserContext` extension (attach it via the new `armature-auth` JWT middleware). Role/permission-protected routes will reject requests until an authentication layer populates the extension.
- **Default request body limit is now 10 MB** (was unbounded). Raise it with `Application::with_max_body_size()`.
- **5xx error responses are redacted.** Server-error bodies now return a generic `"Internal Server Error"` instead of the internal error message; 4xx keep their message. All three servers (module, micro, HTTP/3) share one error-to-response mapping and now agree on status codes and body shape.
- `HttpMethod`, `Http2Config`, and `Http3Config` are now `#[non_exhaustive]`.
- `RetryableErrors::Custom` is now honored by `Retry::call` (previously ignored), and `RetryErrorPredicate` is `Arc`-based so cloning a config preserves the predicate.
- `LogConfig::timestamps(false)` is now applied (previously a no-op).
- `vectored_io::status_line()` returns `Cow<'static, [u8]>` and no longer falls back to a `200` line for unlisted status codes.
- `io_uring::BufferPool::acquire()` returns an RAII `BufferGuard` (the manual `release()` and the old tuple API were removed).
- `HttpRequest.headers` is now a SmallVec-backed `HeaderMap` (inline for typical requests) instead of `HashMap<String,String>`, removing a per-request heap allocation. The type is a drop-in for the previous access patterns; `from_parts` still accepts a `HashMap`.

### Performance

- **Optimized routing on the serve path.** All TCP transports (HTTP/1.1, h2c, HTTPS, ALPN) and HTTP/3 now dispatch through the O(1) `OptimizedRouter` (static-route hash map + compiled patterns + LRU) instead of the O(n) linear scan; `match_path` no longer re-splits the path per candidate route. Catch-all routes (`/files/*path`) now match correctly on the serve path.
- Per-request allocation reductions: alloc-free common-header interning, `micro`'s `App` clones an `Arc<Router>` instead of the whole router, zero-copy `simd-json` body parsing helper, and an empty-middleware-chain fast path.
- Route-constraint and error-sanitizer regexes are compiled once (`LazyLock`) instead of per request/error. `route_cache` uses `parking_lot` with O(1) eviction; `connection_manager` uses lock-free atomic per-connection counters.
- Static assets are served from a bounded in-memory content cache (keyed by path + mtime + encoding) instead of re-reading and re-compressing on every request.
- Redis batching: cache `mget`/`mset`, session/rate-limit/queue paths use `SCAN`/`UNLINK`/pipelines and an atomic Lua job-promotion script instead of `KEYS` and per-item round-trips; tiered cache gained single-flight loading.
- OAuth2 providers reuse a shared HTTP client instead of building one per request; compression now applies to zero-copy (`body_bytes`) responses.
- Build: workspace crates use minimal per-crate `tokio` feature sets instead of `features = ["full"]`, and the OpenSSL/native-tls stack is eliminated (rustls only), removing a native C build from the default workspace.

### Security

#### Auth & Security conformance (`armature-auth`, `armature-jwt`, `armature-security`, `armature-siem`, `armature-mcp`)

Five crates advertised security controls their code did not implement. All are now real, each with a regression test that failed against the old code:

- **SAML SSO bypass closed (`armature-auth`).** `SamlServiceProvider::validate_response` previously accepted *any* attacker-supplied base64 XML containing a `<NameID>` as a valid assertion. It now verifies the enveloped XML signature against the IdP certificate from the configured metadata (via `samael`, behind the `saml` feature), and enforces issuer, audience, `NotBefore`/`NotOnOrAfter`, and recipient/ACS — failing closed when a signature is required but no IdP certificate is configured. Session expiry now comes from the assertion's real `NotOnOrAfter`.
- **MCP JWT auth bypass closed (`armature-mcp`).** `authenticate_jwt` only base64url-decoded the payload and discarded the signature — any forged `sub`/`scope`/`exp` passed. It now verifies the signature via `armature-jwt`'s `JwtManager`, with the algorithm **pinned** from `JwtAuth.algorithm` (rejects `alg: none` and RS256→HS256 confusion).
- **Forgeable request-signing MAC fixed (`armature-security`).** `RequestSigner`'s "HMAC-SHA256" was actually `Sha256(secret || msg)`, a length-extension-forgeable prefix hash. It now uses a real `Hmac::<Sha256>` MAC with constant-time verification.
- **SIEM plaintext downgrade closed (`armature-siem`).** The "TLS" syslog transport logged a warning and then sent security events over plaintext TCP. It now performs a real TLS handshake (tokio-rustls) honoring `tls_verify`/`ca_cert_path`, and returns an error rather than ever downgrading to plaintext.
- **Azure Sentinel auth fixed (`armature-siem`).** The Sentinel transport sent the raw token as `Authorization`, so every send failed auth. It now implements the Azure Monitor SharedKey signing scheme (canonical string + HMAC-SHA256 over the base64-decoded key).
- Fixed two unbounded-growth denial-of-service vectors reachable from network input: the Prometheus request-metrics middleware no longer labels series by raw request path (bounded/opt-in), and the in-memory rate-limit store now evicts idle entries via a scheduled prune task.
- Removed a per-request `Box::leak` memory leak in `route_params`' wildcard matcher.

#### Data & Persistence conformance (`armature-tenancy`, `armature-eventsourcing`, `armature-diesel`, `armature-opensearch`, `armature-redis`, `armature-cqrs`)

- **Tenant-isolation bypass closed (`armature-tenancy`).** In optional mode a failed tenant resolution passed the request through unchanged, so a client-supplied `__tenant_id`/`__tenant_name` header survived and was trusted by `get_tenant_id`/`get_tenant_name` — a cross-tenant access bypass. The middleware now strips all incoming `__tenant*` headers before resolution and stores the resolved tenant in server-side request extensions (a client cannot set them); identity is read only from extensions.
- **Broken optimistic concurrency fixed (`armature-eventsourcing`).** `AggregateRepository::save`/`save_with_snapshot` passed the post-apply version as the expected version while the store checks against the stored event count, raising a false `VersionConflict` on the normal apply-then-save flow. They now pass the pre-new-events base version; genuine stale-write conflicts are still detected.
- **Transaction isolation actually applied (`armature-diesel`).** `transaction_with_isolation` issued `SET TRANSACTION ISOLATION LEVEL` before the transaction opened (a no-op outside a tx block); it now sets the level inside the transaction (Postgres) so the requested level takes effect.
- **OpenSearch aggregations returned (`armature-opensearch`).** `execute_with_meta` read aggregations from `"aggs"` but OpenSearch returns them under `"aggregations"`, so `SearchResult.aggregations` was always `None`; now parsed from the correct key. Silent success on 4xx/5xx responses is now surfaced as an error.
- **Redis "cluster mode" no longer silently standalone (`armature-redis`)** — see the cluster-support entry above.
- **cqrs projection `rebuild` no longer double-applies (`armature-cqrs`).** The default `rebuild` replayed events without clearing existing state; it now calls a `Projection::reset()` (which implementors must override to clear state) before replay. The README was rewritten to the real `CommandBus`/`QueryBus` API (there is no `Mediator` type or `#[derive(Command)]`).

#### Messaging & Scheduling conformance (`armature-queue`, `armature-cron`, `armature-discovery`, `armature-webhooks`, `armature-distributed`, `armature-messaging`)

Data-loss, panic, and silent-degradation defects on the messaging/scheduling paths — each now fixed with a regression test that failed against the old code:

- **Queue job loss and panics fixed (`armature-queue`).** `Worker::process_batch` orphaned a type-mismatched job that had already been popped into the `processing` set (silent data loss) — it now re-enqueues it. Handler registration was a fire-and-forget `tokio::spawn` that raced `start()` (jobs spuriously failed "no handler"), and `register_cpu_intensive_handler` `block_on`'d a lock inside async and panicked in its documented usage — both now register synchronously (`async`, see Changed).
- **Cron scheduler no longer serializes or silently drops jobs (`armature-cron`).** `start` held the global jobs write-lock across each `job.execute().await`, serializing the entire scheduler (defeating async execution / `max_concurrent_jobs`); it now clones state under a short lock and runs the job without holding it. `add_job` registered via a detached task and returned `Ok` before the job existed; it now registers inline and surfaces `JobAlreadyExists`. `CronExpression::matches` ignored its `time` argument and now answers correctly.
- **Discovery register→discover round trip fixed (`armature-discovery`).** etcd wrote keys under `{prefix}/{id}` but scanned `{prefix}/{name}/`, so a registered service could never be discovered; both now use a `{prefix}/{name}/{id}` composite key. `list_services` performed no query (always empty) and now range-scans the store. Consul `discover` no longer returns unhealthy instances as healthy.
- **Webhook delivery panic + spoofable checks fixed (`armature-webhooks`).** `truncate_string` byte-sliced untrusted webhook-target response bodies and panicked on a multibyte boundary (a delivery-task crash); it now truncates on a char boundary. `signing_algorithm` (incl. HMAC-SHA512) and `timestamp_tolerance` config knobs were read nowhere and are now honored; `verify_from_headers` is case-insensitive; the `send` doc no longer claims automatic signing when no secret is supplied.
- **Distributed locks no longer silently expire (`armature-distributed`).** A held `LockGuard` had no TTL renewal (only leadership did), so a lock could expire under a still-live guard and be acquired by another holder; a token-guarded renewal watchdog now keeps it alive (see Added).
- **Messaging delivery guarantees honored (`armature-messaging`).** Kafka manual-ack mode never committed offsets (every processed message was redelivered on restart) and now commits on success; custom message headers were dropped on a NATS round trip and are now preserved.

#### Payment webhook forgery closed (`armature-payments`)

**Two webhook-signature bypasses.** `PayPalProvider::verify_webhook` and `BraintreeProvider::verify_webhook` ignored the payload and signature and unconditionally returned `Ok(())`. Because `PaymentProcessor::handle_webhook` verifies *before* parsing, **any attacker-forged PayPal or Braintree webhook was accepted as genuine**, allowing fabricated payment events. Both now verify for real and fail closed:

- **PayPal** POSTs to `/v1/notifications/verify-webhook-signature` with the five `PayPal-*` transmission headers and the **configured** `webhook_id`, rejecting anything but `verification_status == "SUCCESS"`; transport failures, non-2xx responses, and unparseable bodies all reject. The signed body is forwarded byte-for-byte via `RawValue` (re-serialization would break PayPal's CRC over the body).
- **Braintree** verifies `bt_signature` as `public_key|hmac_sha1(payload)`, selecting the pair matching the configured public key, deriving the MAC key as `SHA1(private_key)`, and comparing in **constant time**. Empty credentials are now rejected — an empty configured public key previously made every forged webhook verify against the public constant `SHA1("")`.
- **Stripe**'s existing verification is now constant-time (was a plain `!=` string compare), enforces a signed replay window that a far-future or negative timestamp cannot bypass, and caps the number of `v1` candidates to bound HMAC work over an attacker-controlled body.
- `WebhookHeaders` deserialization now preserves the documented case-insensitivity invariant, and `parse_webhook` is documented as performing **no** authentication (callers must use `handle_webhook`).
- `BraintreeProvider::create_payment_method` no longer sends the hardcoded sandbox string `"fake-valid-nonce"` in place of the caller's card, and `PayPalProvider::create_customer` returns an explicit unsupported error instead of minting a fabricated customer ID that every later lookup rejected.

An adversarial review of the new verification found no remaining forgery path; reverting the fixes fails 10 of the 13 webhook tests.

#### Other delivery-provider fixes

- **FCM and APNS push were broken at runtime (`armature-push`).** `jsonwebtoken` was built without a `CryptoProvider`, so JWT signing **panicked** for both providers; the `rust_crypto` feature is now pinned. APNS no longer forces `http2_prior_knowledge()`, which broke HTTP/1.1 interop.
- **SES silently dropped attachments (`armature-mail`)** — see above; custom headers and priority were likewise stored and never emitted.
- **`MultiSizeBuilder::generate()` failed on every call (`armature-files`)** because its default `OutputFormat::Original` hit an error branch; `TextWatermark` drew a striped box rather than the requested text.
- **`AzureBlobStorage::temporary_url` returned a permanent unsigned URL** where a time-limited signed URL is contracted; it now returns `Ok(None)` until SAS signing exists, rather than handing out a link that grants no access and never expires. Allowed-type/extension rules now fail closed for files with no detected MIME type or extension.

#### Messaging & Scheduling hardening (audit-battery follow-up)

A multi-agent audit of the WF3 branch (code-review + audit + optimize) found several defects — some introduced by the WF3 fixes themselves — now closed with regression tests:

- **Kafka `close()` now actually stops consumers (`armature-messaging`).** `close()` previously only flipped a `connected` flag; per-subscription consumer tasks kept polling Kafka and invoking user handlers after `close()` and after the broker was dropped. Consumers are now tracked and their `active` flags cleared on `close()`/`unsubscribe` (which also bounds the previously-unbounded consumer/channel retention on RabbitMQ/AWS/NATS).
- **Kafka manual-ack no longer silently drops failed messages (`armature-messaging`).** The initial commit-on-success could advance the offset past an earlier failed message; manual-ack now halts commits on the first failure so nothing is skipped on restart.
- **RabbitMQ vhost + credential fixes (`armature-messaging`).** The compatibility `connect()` path silently overrode a URL-embedded vhost with `/` (misrouting to the default vhost); it now preserves the URL vhost. The raw AMQP connection URL (which conventionally carries `user:password@`) is no longer logged or interpolated into errors.
- **Cron job panics no longer wedge the schedule (`armature-cron`).** A panicking job left its status `Running` forever (so it never ran again) with the panic unobserved; job execution is now panic-isolated, recording a `Failed` outcome and rescheduling.
- **Web Push SSRF hardening (`armature-push`).** The new reqwest sender now blocks redirects, enforces HTTPS, rejects internal/link-local/private-IP endpoints (push subscription endpoints are client-supplied), no longer echoes the upstream response body into errors, and applies a 30s request timeout.
- **NATS credentials no longer serializable/loggable (`armature-messaging`).** `NatsConfig`'s `jwt`/`nkey_seed`/`credentials_file` are redacted in `Debug` and skipped on serialization.
- Robustness: `armature-queue` `backoff_delay` no longer overflows/panics for large attempt counts and `complete`/`fail` always drain the processing set; `armature-discovery` etcd/consul/health-check clients now have request timeouts and the etcd prefix scan uses a correct range successor; `armature-webhooks` signs over raw payload bytes and resolves signature headers deterministically.

#### Breaking changes in the delivery-provider crates (0.2.0)

The two audit-battery passes below changed a large amount of public surface. This
list is the migration guide; an earlier revision of this changelog described the
behavioral fixes in prose and labelled none of them breaking, which left a
consumer upgrading from `0.1.x` with a wall of compile errors and nothing to work
from.

**`armature-payments`**
- `StripeProvider::new`, `PayPalProvider::new`, `BraintreeProvider::new` and `ProviderClient::new` return `PaymentResult<Self>`. They previously fell back to an HTTP client with **no request and no connect timeout** when the builder failed, which is the unbounded hang the timeouts exist to prevent.
- `with_base_url` returns `PaymentResult<Self>` and rejects any non-`https` URL that is not loopback. It previously accepted any scheme, leaking `sk_live_…`/Basic credentials in cleartext — and, because PayPal's webhook verification POSTs to that base, letting an attacker-chosen host answer `{"verification_status":"SUCCESS"}`.
- `with_webhook_tolerance(Option<Duration>)` → `with_webhook_tolerance(Duration)` plus an explicit `without_webhook_tolerance()`. Disabling replay protection can no longer happen by passing `None`.
- Removed: `PaymentError::{InvalidCard, InvalidAmount, Unknown}`, `ProviderClient::base_url()`. Added: `PaymentError::Unsupported(String)`, constructed via `PaymentError::unsupported(provider, operation)` which formats both into the message.
- `sanitize_body`, `classify_status` and `retry_after_secs` are now `pub(crate)`; `lib.rs` no longer glob-re-exports `provider::*`. These were internal helpers accidentally published — `sanitize_body` in particular is a best-effort redaction heuristic whose thresholds must stay free to tune.
- `ProcessorConfig` gained `max_retry_delay_ms`; `RefundRequest` gained `idempotency_key`; `WebhookData::from_event_type` takes `raw` by value; `WebhookEventType` derives `Hash`.
- **Behavioral:** `charge` and `refund` no longer retry unless the provider reports `PaymentProvider::supports_idempotency()` *and* the request carries a key. Stripe and PayPal opt in; **Braintree does not**, because it has no general idempotency mechanism — a transient Braintree failure now surfaces instead of being retried. One visible error beats up to four real charges.
- **Behavioral:** PayPal's `get_customer`/`update_customer` return `Unsupported` rather than `CustomerNotFound`, and `create_customer`/`create_payment_method`/`attach`/`detach` return `Unsupported` rather than `Provider`.
- Dependency: `armature-core` removed (it was declared and never referenced); `hmac`/`sha2`/`hex` are now optional behind `stripe`+`braintree`, `base64` behind `paypal`+`braintree`.

**Upgrade and downgrade notes — read these two before deploying**

- **`armature-mail` queued attachments change wire format, and a *downgrade* destroys them.** `Attachment::data` now serializes as base64 rather than a JSON number array. Deserialization accepts **both** forms, so upgrading in place with a non-empty queue is safe. But 0.2.0 re-serializes every job it touches and 0.1.x cannot read base64, so a job 0.2.0 has written is unreadable by 0.1.x — whose `pop` path deletes bodies it cannot parse. **Drain the queue before rolling back.**
- **Rendered HTML changes on every `armature-mail` template engine.** The three previously escaped different character sets — Handlebars escaped `=` and `` ` ``, Tera did not; MiniJinja escaped `/`, the others did not. They now share one escape function over the OWASP union (`& < > " ' / = \``), so no engine escapes *less* than before, but output differs from 0.1.x on all three: Tera renders `'` as `&#x27;` rather than `&#39;`, and both Tera and MiniJinja newly escape characters they previously passed through. A URL in an HTML body now renders as `https:&#x2f;&#x2f;…` everywhere. Browsers decode this correctly; golden-file tests and raw-source comparisons will not match.

**`armature-mail`**
- **`EmailQueue::in_memory`, `EmailQueue::redis`, `EmailQueue::with_backend`, `MailerQueueExt::queue` and `MailerQueueExt::queue_redis` now return `Result`.** They validate that `visibility_timeout > job_timeout * 2`; the previous release documented that relationship as a requirement and never enforced it, so a plausible one-line misconfiguration systematically redelivered every slow email.
- **`EmailQueueBackend::reclaim_stale` implementors must now increment `attempts` and dead-letter on exhaustion.** Without it a job that kills its worker returns with `attempts` frozen, never reaches the dead-letter queue, and is re-sent every visibility timeout indefinitely — an unbounded stream of duplicates if the original send had in fact succeeded.
- **Behavioral:** `Email::validate` rejects ASCII control characters in `subject`, `message_id`, `in_reply_to` and `references`, not just in custom headers — these all reach the wire as header values on Mailgun/SendGrid and were a header-injection vector. A *trailing* whitespace run in `subject` is stripped rather than rejected (template-derived subjects commonly end in a newline); an embedded control character still fails the send where 0.1.x delivered.
- **Behavioral:** `Attachment::from_file` rejects files over `DEFAULT_MAX_ATTACHMENT_BYTES` (25 MB), checked via `metadata` before reading. Use `Attachment::from_file_with_limit` to raise or lower it. New: `from_file_async`, `from_file_async_with_limit`.
- `validate_header_value` is no longer exported (it is `pub(crate)`); `validate_header` remains public.
- `MailError::Provider(String)` → `MailError::Provider { status: Option<u16>, message: String }`. This is what lets `is_retryable()` tell a transient 503 from a permanent 400; previously a SendGrid 503 was dead-lettered on the first attempt. Use `MailError::provider(status, message)` to construct.
- `SendGridTransport::new` and `MailgunTransport::new` return `Result<Self>` and validate the endpoint scheme. Both previously fell back to an untimed client, and neither rejected a cleartext endpoint — which ships the API key in the clear, since `send()` attaches `Authorization: Bearer`.
- `EmailQueueBackend` gained two **required** methods: `discard(&self, job_id)` and `reclaim_stale(&self, Duration) -> Result<u64>`. Deliberately not defaulted: a no-op `reclaim_stale` would let an external backend silently keep losing jobs while appearing to implement recovery.
- `Attachment::data` is now `bytes::Bytes`; constructors take `impl Into<Bytes>`. `EmailJob::email` is `Arc<Email>`. `Email` gained `invalid_headers`.
- `#[non_exhaustive]` on `MailerConfig`, `MailgunConfig`, `SendGridConfig`, `EmailQueueConfig` — construct via `new` (or `EmailQueueConfig::default()`, which has no `new`) plus the builder methods rather than struct literals.
- `HandlebarsEngine::register_helper` gained a `Clone` bound (the helper is installed into both the escaping and non-escaping registries).
- **Behavioral:** Handlebars no longer HTML-escapes the `text` and `subject` parts. It previously escaped every part, so `Bob & Alice` reached recipients as `Bob &amp; Alice` in the plain-text body and in the `Subject:` header. All three engines now escape the `html` part only, pinned by a cross-engine conformance test.
- Dependency: `mail-builder`, `mime`, `tokio-test` removed; `bytes` added.

**`armature-files`**
- Removed: `OutputFormat::Avif` (the encoder always errored and no detection path could produce it), `OutputFormat::WebP`'s `quality` field, `Pipeline::output_format`. `ImageOp::TextWatermark` gained `font: Option<Bytes>`; `decode_image` takes `DecodeLimits`.
- **`image` is now built with `default-features = false`, so AVIF, DDS, OpenEXR, farbfeld, HDR, PNM, QOI and TGA can no longer be _decoded_.** Only bmp/gif/ico/jpeg/png/tiff/webp are enabled. Re-enable with `image/<format>` if you accept those inputs. This drops `ravif`/`rav1e`/`exr` from the tree.
- **Behavioral:** `ZipExtractor::extract_to` is now **non-clobbering** — an entry whose destination already exists (file, directory or symlink) is an error rather than an overwrite. This falls out of using `O_EXCL`, which is what stops an archive writing *through* a pre-existing symlink.
- **Behavioral:** `ImageOp::Crop` returns `InvalidDimensions` on overflowing coordinates instead of panicking in debug / wrapping in release. Image-watermark overlays are decoded under the pipeline's configured `DecodeLimits`, so a previously-accepted oversized overlay may now be rejected.
- Added: `archive::DEFAULT_MAX_ARCHIVE_BYTES`, `ZipExtractor::max_archive_bytes`, `extract_all_async`, `image::{process_image_async, convert_format_async}`, and an `embedded-font` feature (default-on).
- Dependency: `armature-core`, `armature-log`, `base64`, `sha2`, `printpdf` removed — all had zero call sites. `armature-files` drops from ~155 transitive crates to **81 with default features**, and to **20 with `--no-default-features`**. The larger share of that is the dependency removals and `image`/`imageproc` `default-features = false`, not the feature opt-out — an earlier revision of this line attributed the whole reduction to `--no-default-features`, which overstated what the flag buys a consumer who keeps `images` on.

**`armature-storage`**
- `Storage::list_page(prefix, cursor, limit)` is a new **required** trait method; `list` is now a provided method over it and errors past `LIST_MAX_ITEMS` (10 000) instead of allocating without bound. Fixing S3's silent truncation had removed the bound rather than exposing it.
- **`Storage::delete` now contracts idempotent deletion** — deleting a missing key is `Ok(())` on all four backends. `LocalStorage` previously returned `NotFound` while S3 returned `Ok(())` and GCS/Azure returned `Storage`, so `is_not_found()` was correct on one backend and wrong on three.
- Removed: `AzureBlobConfig::{sas_duration, access_key, connection_string}` — the SDK cannot use key or connection-string auth, so those builders only ever produced a deferred constructor failure.
- `FileValidator::validate` wraps failures in `ValidationError::Rule { rule, source }`; match on `err.kind()`. `allowed_types`/`images_only`/`documents_only` now sniff magic bytes, so uploads that lie about their `Content-Type` are rejected where they previously passed. S3 `put` rejects unknown `storage_class`/`default_acl`/`server_side_encryption` rather than forwarding them verbatim.
- **Security:** `LocalStorage` now physically resolves every key — each path component is `lstat`ed and rejected if it is a symlink, and the deepest existing ancestor is canonicalized and re-checked against the canonical root (again after `create_dir_all`, the one step that changes what a path resolves to). The previous release's containment check was **lexical only**: it blocked `..` and absolute keys, but a symlink already under the root escaped it entirely, giving arbitrary filesystem read and write. `list` no longer follows directory symlinks (which could loop forever and leak absolute host paths into public URLs), and `put_file` sanitizes client-supplied filenames on all four backends, not just Local.
- Added: `DEFAULT_LIST_PAGE_SIZE`, `LIST_MAX_ITEMS`, `ValidationContext`, `ValidationRule::validate_with` (defaulted). Dependency: `infer` added; `tempfile`/`serde_json` moved to dev-dependencies.

**`armature-payments` (second pass)**
- `PaymentError::Provider(String)` → `PaymentError::Provider { status: Option<u16>, message: String }`, matching `MailError` and `PushError`. Use `PaymentError::provider(status, message)`; `Display` now appends the status.
- **Behavioral:** `is_retryable()` returns `true` for a `Provider` carrying a 5xx or 408. Previously every gateway 5xx was permanently fatal — a Stripe 502 from a load balancer failed the charge outright, which is the canonical case where an idempotency key makes replay provably safe. The money-moving gate is unchanged (`supports_idempotency() && key.is_some()`), so Braintree still gets exactly one attempt; a statusless `Provider` stays non-retryable.
- **Behavioral:** a server-supplied `Retry-After` is no longer clipped to `max_retry_delay_ms` — it is bounded by the new `MAX_SERVER_RETRY_AFTER_MS` (1 hour), and `max_retry_delay_ms` now bounds only the local exponential schedule. Clipping a gateway's own backoff request to 30s re-throttles against an endpoint that just said it was overloaded. Mirrors `armature_mail::MAX_SERVER_RETRY_AFTER`.

**`armature-storage` (second pass)**
- **Behavioral:** `LocalStorage::url` can now return `Err(InvalidFileName)` and percent-encodes each path segment; it was the one `Storage` method that interpolated the raw key, so `url("../../admin")` escaped the base path and `?`/`#`/CRLF injected into the URL.
- **Behavioral:** `LocalStorage::list_page` cursors are keys rather than offsets, and `prefix` is now a byte prefix over keys as it is on S3/GCS/Azure (it previously joined the prefix onto the key root and walked that directory, so `list_page(Some("rep"))` returned nothing on Local and `["reports/a.txt"]` on S3). Cursors are opaque per the trait contract, but an offset cursor was unstable under concurrent mutation — a `put` or `delete` of a lexically earlier key silently skipped or duplicated entries across pages.
- New `[target.'cfg(unix)'.dependencies] libc` for the `O_NOFOLLOW` handles described below.

**`armature-push`**
- `PushError::Provider(String)` → `PushError::Provider { status: Option<u16>, message: String }`, mirroring `MailError`. Use `PushError::provider(status, message)`.
- **Behavioral:** `is_retryable()` now returns `true` for a `Provider` carrying a 5xx or 408. A five-minute FCM 503 window previously dropped an entire notification batch permanently.
- **Behavioral:** `should_remove_device()` no longer returns `true` for an APNs **404**. A 404 from APNs is `BadPath` — a wrong `:path`, topic or environment — not a dead device. Mapping it centrally alongside FCM's 404 meant one bad deploy could report every send as "device gone" and prune an entire iOS token table. FCM and Web Push keep 404 → `Unregistered` at their own call sites, where it is correct.
- `#[non_exhaustive]` on `ApnsEnvironment`, `FcmConfig`, `ApnsConfig`, `WebPushConfig`; `ApnsEnvironment` is no longer `Copy` and gained `Custom(String)`. `WebPushConfig::public_key` was removed. `NotificationBuilder` is `#[deprecated]` and out of the prelude; `SubscriptionKeys` is newly exported.
- All three configs gained `allow_insecure_loopback` (default `false`), `timeout` and `connect_timeout`. Non-`https` API bases are rejected at construction for FCM and APNS; Web Push validates each subscription endpoint at send time.
- Dependency: `uuid`, `base64`, `tokio-test`, `futures` removed; `futures-util` and `bytes` added.

#### Delivery-provider hardening (audit-battery follow-up)

A multi-agent audit of the WF6 branch (simplify + optimize + audit + code-review, 14 agents) found 105 defects — including two vulnerabilities introduced by the WF6 fixes themselves and one false claim in this changelog. All are now closed with regression tests.

**Money-movement correctness (`armature-payments`) — the most serious findings in this round:**

- **Retrying a charge or refund could take money more than once.** `PaymentProcessor` gained retries in WF6 and `ProcessorConfig::default()` enables them (`retry_failed: true, max_retries: 3`), but `idempotency_key` was read in exactly two places, **both Stripe** — PayPal and Braintree dropped it silently. Worse, `From<reqwest::Error>` classified *body-decode* failures as retryable `Network`, so a response the gateway had already committed was re-posted. An ambiguous timeout could therefore produce **up to four real charges**, and `RefundRequest` had no idempotency key at all, so the same applied to refunds on every provider including Stripe. Retries are now gated on a new `PaymentProvider::supports_idempotency()`; decode failures map to non-retryable `Serialization`; `RefundRequest` carries a key that is generated once and reused across attempts.
- **Wrong currency and fabricated success in capture/refund.** `BraintreeProvider::capture`/`refund` hardcoded `ChargeStatus::Succeeded` and `Currency::USD` while discarding the response's `currency_iso_code` — a €120 settlement returned `Money { 12000, USD }`, which then passed `Money::add`'s currency assertion and silently corrupted downstream totals. `PayPalProvider::capture` likewise hardcoded success, reporting `PENDING` and `DECLINED` captures as complete. Both now map the real gateway status and currency.
- **Zero-decimal currencies were rejected by the gateway.** Six call sites formatted amounts as `{:.2}` unconditionally, so `Money::new(1000, Currency::JPY)` was sent as `"1000.00"` — which PayPal rejects with `DECIMALS_NOT_SUPPORTED`. New `Money::to_gateway_string()` honors `Currency::decimals()`.
- **No HTTP timeouts anywhere in the crate.** All three providers used a bare `reqwest::Client::new()` — no request or connect timeout — and WF6 then wrapped those calls in a retry loop, so an unresponsive gateway hung `charge()` indefinitely. All three now use a shared client with 30s request / 10s connect timeouts.
- `PaymentProcessor` now honors `Retry-After` and applies exponential backoff with jitter instead of a flat delay; PayPal and Braintree map HTTP status to typed errors (previously everything collapsed to `Provider`, so `RateLimited` was never constructed and the retry policy was dead config on two of three providers); PayPal's `delete_customer` and `list_payment_methods` return explicit `Unsupported` errors instead of `Ok(())`/`Ok(vec![])`; Braintree's `charge` no longer silently discards three of four `PaymentSource` variants; `cancel_subscription` no longer silently upgrades an end-of-period request to immediate cancellation.

**Vulnerabilities introduced by the WF6 changes themselves:**

- **HTML autoescaping was disabled in the new mail template engines (`armature-mail`).** `TeraEngine` and `MiniJinjaEngine` register templates under keys like `welcome/html` — with no dot — so minijinja's extension-based autoescape callback and Tera's `.html` suffix match both fell through to **no escaping**, unlike the Handlebars engine they were modeled on. Any user-controlled render-context value could inject arbitrary HTML into transactional email sent to other users. Both engines now force HTML escaping for the `html` part, with per-engine tests asserting `<script>` is escaped there and *not* escaped in the text/subject parts.
- **`with_base_url` could defeat webhook verification (`armature-payments`).** The builder accepted any scheme on all three providers, leaking `sk_live_…` and PayPal Basic credentials over cleartext — and because `PayPalProvider::verify_webhook` posts to that override, any host it pointed at could reply `{"verification_status":"SUCCESS"}` and every forged webhook would be accepted, defeating the WF6 fix entirely. It is now fallible and rejects anything that is not https (or loopback, for tests).

**Filesystem and resource-exhaustion hardening (`armature-files`, `armature-storage`):**

- **Path traversal in `LocalStorage`.** Every `Storage` method routed through `full_path`, which used `PathBuf::push` on a caller-supplied key — so `../` escaped `base_path` and an absolute key discarded it entirely, giving arbitrary filesystem read and write. Keys that are absolute or contain `..` are now rejected with `StorageError::InvalidFileName`, and the resolved path is verified against the canonicalized root.
- **Zip Slip and zip bombs in `ZipExtractor`.** Archive entry names were joined unchecked, letting a crafted archive write outside the target directory; extraction also decompressed the entire archive into memory with no cap. Extraction now uses `enclosed_name()` with a canonicalized-root re-check (which also catches symlink escapes) and enforces `max_uncompressed_size`/`max_entries` budgets.
- **Unbounded image decode.** Neither image loader set decoder `Limits`, so a 40-byte PNG header declaring 65535×65535 forced a ~17 GB allocation. Both now apply configurable dimension and allocation limits via a new `DecodeLimits`.
- **Multipart uploads were unbounded by default.** `Multipart::new`, `from_request`, and `parse_multipart` installed all-`None` counters, so the ergonomic entry points buffered request bodies without limit — see the corrected note under Changed. All three now apply `MultipartConstraints::default()`.
- **A panic reachable from the public API.** `Position::calculate` used unchecked `u32` subtraction, so watermarking any image narrower than the rendered text — ordinary at the documented default font size — panicked in debug builds and wrapped to a garbage coordinate in release. All arithmetic is now saturating.
- CPU-bound image, PDF, and archive work now runs on `spawn_blocking` (there was previously **no** `spawn_blocking` anywhere in `armature-files`, so a Lanczos resize stalled a runtime worker and every task scheduled on it).

**Silent data loss and misclassification:**

- `Address::parse` **panicked** on input like `"a>b <x@y.com>"` (it located `<` and `>` independently, producing an inverted slice range), aborting the task rather than recording an invalid address — defeating the WF6 invalid-address fix. `validate_email` also validated a *trimmed copy* while storing the untrimmed original, so `"a@b.com\n"` passed validation and kept its newline; header names **and** values are now validated for CR/LF and control characters across all four transports.
- `MailError::is_retryable` excluded `Provider`, the variant every API transport used for all non-429 failures — so a transient SendGrid 503 was dead-lettered on the first attempt. `Provider` now carries the HTTP status and 5xx/408/429 are retryable.
- The Redis queue's `ZPOPMIN` result was read as a flat list of job IDs when it actually interleaves members and scores, so half the MGET keys were fabricated from score values (working only by accident, and failing outright under RESP3); `pop(count)` also returned up to `2 * count` jobs, diverging from the in-memory backend.
- A timed-out send was retried with a **freshly minted** `Message-ID`, so nothing downstream could deduplicate; jobs now carry a stable ID stamped at enqueue.
- `S3Storage::list` silently truncated at 1000 objects (no continuation token) while GCS and Azure paginated correctly; `LocalStorage::list` was non-recursive, so nested keys created by `put` were invisible to it, and it dropped entries whose metadata read failed.
- FCM silently discarded `Notification::badge` — including in this crate's own documented quick-start — and `PushError` variants were inconsistent across providers: 413, 404, and 401/403 collapsed into `Provider` on FCM and APNS while Web Push mapped them, and `Retry-After` was honored only by Web Push.
- The Web Push SSRF guard could be bypassed with IPv4-mapped IPv6 (`[::ffff:169.254.169.254]` reached the cloud metadata endpoint) and did not cover CGNAT (`100.64.0.0/10`).
- Private keys were printed verbatim by the derived `Debug` on `FcmConfig`, `FcmCredentials`, `ApnsConfig`, and `WebPushConfig`; all four now redact key material.

**Correctness fixes with user-visible output changes:**

- The text watermark blended `color`'s alpha into the alpha *channel* rather than compositing it, so a 25%-opacity watermark came out **fully opaque** on any format that drops alpha (JPEG, BMP, GIF).
- `Convert(OutputFormat::Original)` decoded and re-encoded — recompressing JPEGs on every pass, and compounding across `MultiSizeBuilder` runs — despite its test describing it as a format-preserving no-op. It now short-circuits to byte-identical output.
- `process_image` returned PNG bytes while leaving `mime_type` as the source format for any codec not in its mapping table (AVIF, QOI, PNM, DDS, HDR, TGA), so storage would serve a PNG under the wrong `Content-Type`.
- `PdfBuilder::orientation` did not recompute the layout cursor, so **every landscape PDF rendered blank** (content was emitted above the page top). PDF text metrics also counted UTF-8 bytes rather than glyphs, wrapping accented and CJK text 2–3× too early, and non-ASCII characters were emitted as raw bytes into a single-byte-encoded string (mojibake); text is now transcoded to WinAnsiEncoding with a hard error for unrepresentable characters.

**Test and CI integrity — the finding with the widest reach:**

- **None of these five crates' tests had ever run in CI.** The root package's `full` feature does not depend on them (they are not even root dependencies), so `cargo test --features full` never compiled them, and the coverage job's `--workspace` built each member with only its *default* features. The mail queue and Redis suites, every PDF test, the APNS and Web Push suites, and all of the S3/GCS/Azure tests were silently unexecuted. A new `test-members` CI job runs each crate with `--all-features`, and sets `ARMATURE_REQUIRE_DOCKER=1` so container-gated suites fail rather than self-skipping into a green pass.
- Several tests passed against the pre-fix code and have been rewritten to actually pin the behavior they name — the SES raw-MIME tests exercised only unchanged `to_lettre` code and re-stated the branch condition inline; the GCS signed-URL-duration assertion sat inside an `if let` that never matched without ambient credentials; the multipart "cap" test used 2 parts against a limit of 100. A constant-time-comparison test that could not detect a non-constant-time comparison was renamed to describe what it actually checks.
- Every doc example in these crates was fenced `rust,ignore` and had drifted into referencing APIs that do not exist (`ImageOp::Watermark`, `ImageOp::Quality`, `Pipeline::load`/`save`). All are now compiled by `cargo test --doc`.

#### Delivery-provider hardening (second audit battery)

A second 14-agent battery over the commit above found 25 further ship-blockers. Most were defects *introduced by* the first round's fixes — the pattern worth recording is that a security control can be described accurately in prose and be absent, or present and defeated by its own new code.

**The `armature-mail` job sweeper, added in the previous commit, had four independent defects.** It was introduced to stop jobs vanishing when a worker dies; it closed that and opened four worse failure modes. All are now fixed:
- The in-memory backend acquired its locks in the opposite order from `pop`, so a sweep interleaving with a poll **deadlocked permanently** — no error, no log, no panic. The three mutexes are now one `Mutex<InMemoryState>`, making the ordering bug unrepresentable rather than documented.
- Reclaimed jobs never incremented `attempts`, so a job that killed its worker was re-sent every visibility timeout **forever**, never reaching the dead-letter queue.
- A reclaim racing a live `complete` recorded a **successfully delivered** email as lost and dead-lettered it. `complete`/`fail`/`dead_letter`/`discard` are now gated on `ZREM :processing == 1`, so the loser of the race is a no-op. The converse is also now handled: a claimed job whose body Redis *evicted* is still recorded as lost, discriminated from the delivered case by that same reply.
- The Redis reclaim script had no `LIMIT`, so after a fleet restart it blocked Redis for the whole backlog; as a write script `SCRIPT KILL` refuses it, leaving `SHUTDOWN NOSAVE` — which discards unpersisted jobs — as the only recovery.

**`armature-storage`:** hardlinks defeated the containment check entirely (`symlink_metadata` reports a hardlink as a regular file and `canonicalize` returns the path unchanged), giving the same arbitrary read/write the symlink defense assumes an attacker is trying for. File I/O now goes through `O_NOFOLLOW` handles rather than path-based `fs::read`/`fs::write`, and `head` uses `symlink_metadata`. The `resolve` doc no longer claims a guarantee: an intermediate-directory substitution window remains on unix, and the leaf is racy on non-unix, both now stated.

**`armature-files`:** the zip entry-count guard was spoofable two ways — it took the *last* `PK\x05\x06` in the trailing 64 KiB without validating the record, and it consulted the Zip64 count only when the classic count was `0xFFFF`, while `zip` sizes its index from Zip64 whenever a locator is present. It now enumerates every candidate and takes the maximum, so a planted record can only add a count, never mask one. Extraction promotion is genuinely all-or-nothing: destinations are reserved with `O_EXCL` in a pre-flight pass and rolled back on any failure, rather than renamed entry-by-entry with earlier entries left behind.

**`armature-push`:** a `404` from APNs means `BadPath` — a wrong topic, environment or host — not a dead device, but the shared mapper treated it as `Unregistered`, whose `should_remove_device()` is `true`. One bad deploy could have pruned an entire iOS token table. FCM and Web Push keep `404 → Unregistered` at their own call sites, where it is correct. The SSRF guard now resolves `Host::Domain` and vets every answer, pinning the connection to a vetted address — previously a hostname with an `A` record of `169.254.169.254` bypassed every literal-IP check.

**`armature-payments`:** `sanitize_body` — the helper added to keep credentials out of error strings — missed JWTs (it split on `.`, so the claims and signature survived), HTTP Basic, credentials straddling its own truncation boundary, and separator-formatted card numbers. All four are closed, and the one remaining raw-body sink (`debug!` in the Stripe error path) now routes through it.

**Test and CI integrity:** twenty-seven container-gated tests across `armature-acme`, `-diesel`, `-redis`, `-seaorm`, `-distributed` and `-queue` still hand-rolled the old skip-only guard and ignored `ARMATURE_REQUIRE_DOCKER` entirely; all now delegate to the shared macro. `armature-testkit`'s own container tests are `#[ignore]`d, so the crate that owns the enforcement was the one place it could not fire — its CI row runs with `--include-ignored`. The `test-members` matrix grew from 7 to 12 of 63 members, and a new job builds `armature-payments` with each provider feature alone under `-D warnings`, since that is the shape the per-provider dependency gating exists to serve.

**`armature-testkit`:** the crate root now also exports `docker_gate`, `require_docker_env`, and `REQUIRE_DOCKER_ENV` (previously only `docker_available` was re-exported); `skip_if_no_docker!` itself now panics under `ARMATURE_REQUIRE_DOCKER=1` instead of only skipping, which is what lets a CI row's `--include-ignored` step enforce container coverage rather than silently pass.

#### Delivery-provider hardening (full-branch review)

Every prior round above reviewed one commit's diff against the one before it. This round reviewed the whole branch — all five commits, from the original spec through the second audit battery's fixes — as a single unit, specifically looking for what an incremental, commit-by-commit review cannot see: whether the code at the tip actually holds together end to end, and whether a fix from one round survived the round after it unbroken. It found 3 Critical-severity defects that no single-commit review had caught, on top of confirming 2 that a prior round had already flagged as still open. 36 findings total (5 Critical, 13 High, 12 Medium, 6 Low), all fixed with regression tests.

**The `armature-mail` job queue's claim model is redesigned around fencing tokens — the two Critical carry-overs from the previous round.**

- **A stale claim's finalize call could destroy a live one and lose mail outright.** `complete`/`discard`/`fail`/`dead_letter` decided ownership by job-id membership alone (`ZREM :processing == 1` / `HashMap::remove(id).is_some()`). That is enough to arbitrate the two-party race between a worker and the sweeper reclaiming *the same* claim, which the previous round's fix correctly closed — but not the three-party case: once a reclaimed job is *re-popped by a second worker*, the id is claimed again under a fresh claim, and the first worker's eventual, late finalize call is indistinguishable from a legitimate one by id alone. If that late call is `complete`, it silently destroys the second worker's live claim; if it's `fail`/`dead_letter`, the second worker's own later finalize then finds nothing to act on and the job vanishes from every set with no trace, despite `enqueue` having returned `Ok(job_id)`. Every claim (`pop`, and a backend's internal re-claim) now mints a fencing token — a monotonic counter in both backends (Redis: `INCR` against a `:claim_seq` key, recorded in a companion `:claim` hash; in-memory: a counter under the existing single lock) — and every release path verifies the presented token still matches the current claim before touching it, exactly the pattern this codebase already used correctly in `armature-distributed`'s lock release.
- **`RedisBackend::pop`'s claim of the main pending queue was two non-atomic round trips.** `ZPOPMIN` (remove from `:pending`) followed by a *separate* `ZADD` (add to `:processing`) meant a crash or transient Redis error between the two left the id in neither set — silently and permanently orphaned, since `reclaim_stale` only ever scans `:processing`. No concurrency or race was required, just an ordinary fault. This is now one atomic `EVAL` (`CLAIM_PENDING`), mirroring the atomic claim the retry side (`CLAIM_RETRY`) already had, and it mints the job's fencing token in the same script.
- Two related gaps closed in the same file: `EmailQueueConfig::validate()` now rejects `batch_size(0)`/`concurrency(0)` (previously the former panicked inside the worker task and the latter silently deadlocked the whole worker, including the sweep loop that lives in the same poll iteration); and `reclaim_stale` on both backends now respects `dead_letter_queue(false)` when a job exhausts its retries through repeated reclaims, discarding without a durable record the same way the normal worker-exhaustion path already did — previously it wrote to the dead-letter list unconditionally regardless of that setting.
- **`EmailQueueWorker` no longer has a fixed pool of long-lived worker loops sharing a hand-rolled channel.** A panic inside one job's send (a malformed attachment, a template bug) unwound that whole worker's loop, permanently and silently shrinking effective concurrency by one with no log line — repeated panics could decay a worker toward zero capacity as an invisible leak. Each popped job is now spawned as its own task (bounded by a `Semaphore` at `concurrency` permits), so a panic is contained by tokio's own task boundary and logged as a `JoinError` instead. The custom `worker_channel` MPMC shim this replaced is removed.
- `push_batch` is now genuinely all-or-nothing on both backends (Redis: wrapped in `MULTI`/`EXEC`; in-memory: capacity checked against the whole batch under one lock acquisition) — previously a partial failure could leave a prefix of the batch durably enqueued while returning `Err`, and a caller retrying the whole batch after that would double-enqueue the prefix that had already landed.
- **Breaking:** `EmailQueueBackend::complete`/`discard` gained a required `claim_token: u64` parameter; a custom implementor of this trait must add it. `EmailJob` gained a `claim_token: u64` field (not serialized; always minted fresh by whichever operation claims the job).

**A live, unpatched instance of an already-fixed vulnerability class was found in three storage backends (`armature-storage`).** The previous round's `LocalStorage::url` fix — percent-encoding each key segment so a crafted key like `"../../admin"` or one containing `?`/`#`/CRLF could not escape the path or inject query/fragment/header content into the URL — was never applied to `S3Storage::public_url`, `GcsStorage::public_url`, or `AzureBlobStorage::public_url`, which still formatted the raw key into a URL via plain string concatenation. All three now route through the same validate-and-percent-encode helper. **Breaking:** the three `public_url` methods now return `Result<String>` instead of an infallible `String`.

**Storage's leaf-race defense is now checked on the open file, not just the pre-open path (`armature-storage`).** The hardlink containment check (added the previous round) ran before the open, gated on `is_file()` — so a FIFO or device node at the leaf skipped it entirely (hanging `read_to_end`/the write call indefinitely, or driving unbounded memory growth against `/dev/zero`), and a hardlink substituted at the leaf *after* the check but before the open was not caught either. `open_read`/`open_write`/`copy` now `fstat` the already-open file descriptor and reject anything that is not a regular file or has `nlink() > 1`, closing both gaps at the point that actually matters. A new test proves `Storage::get`/`put`/`copy` actually route through the `O_NOFOLLOW` openers (a call counter), not just that a *pre-planted* symlink is caught by the earlier lexical check — the existing tests could not have caught a regression that silently swapped the openers back for `fs::read`/`fs::write`.

**Two Critical gaps in `armature-push`'s own test suite — one confirmed still open from the previous round, one newly found.** The DNS/SSRF regression tests self-skipped to zero assertions whenever the ambient resolver hijacked NXDOMAIN or couldn't resolve a trailing-dot hostname — on such a CI runner, the branch's headline SSRF fix ran no assertions and reported green either way. Separately, the actual pinning mechanism (`resolve_to_addrs`, which closes the DNS-rebinding window between vetting a hostname and connecting to it) had zero coverage in either direction: deleting it outright would not have failed a single existing test. Both are closed by giving `resolve_and_vet` an injectable resolver seam (crate-private `HostResolver` trait; a fixed-answer fake drives the tests instead of live DNS), which also enabled a genuine rebinding-refusal test: vet a hostname to one address, change what a live re-resolution would answer, and assert the connection still lands on the originally vetted address. Two further `armature-push` fixes: `resolve_and_vet`'s DNS lookup is now wrapped in `connect_timeout` (previously unbounded — a subscription endpoint pointed at a DNS tarpit held a shared blocking-pool thread for the resolver's full retry budget); and the pinning override's key and the actual outbound request now derive from the same parsed host string rather than two independent URL parses, closing a latent fail-open path if the two ever diverged.

**Naming collision between a strong and a weak SSRF check in the same crate (`armature-push`).** `apns.rs` and `fcm.rs` each had a function named identically to `web_push.rs`'s `validate_endpoint` — which resolves DNS and vets every answer against internal-IP-range tables — but that only checked scheme and a loopback opt-in, no DNS vetting at all (a reasonable check for APNs/FCM, whose base URL is developer-configured rather than per-request attacker input, but not one a maintainer extending it by analogy from the name would expect to be weaker). Both are renamed to `require_https_or_loopback`, with a doc pointer to `web_push::validate_endpoint` explaining the narrower guarantee.

**`armature-payments`:** `ProviderClient.api_key` is now a `secrecy::SecretString` instead of a plain `String` redacted only by a hand-written `Debug` impl, matching the `SecretString` the three provider structs already wrap it in before handing it off. **Breaking:** `ProviderClient`, `build_http_client`, and `validate_base_url` are narrowed from `pub` to `pub(crate)` — they were internal HTTP-plumbing helpers, unlike the equivalent (already-private) helpers in `armature-mail`/`armature-push`, with nothing marking them as an intended extension point for third-party `PaymentProvider` implementations.

**Documentation and CI accuracy:** the changelog's own count of container-gated test conversions was wrong twice over — it said eleven when the four named crates (`armature-acme`/`-diesel`/`-redis`/`-seaorm`) totaled nine, and omitted `armature-distributed` and `armature-queue`, which received the identical fix (eighteen more conversions between them); the true total, twenty-seven, is now stated with all six crates named. A `TODO.md` entry retracting a false "the mail queue's channel module is dead" claim cited the module's *pre-rename* symbol name (`async_channel`, renamed to `worker_channel` in the same commit that wrote the retraction), failing its own stated grep-based verification — corrected. `TODO.md`'s "still uncovered" crate list had silently dropped four real, zero-CI-coverage members (`armature-lambda`, `-cloudrun`, `-azure-functions`, `-collab`) when it replaced a longer list that named them — restored. `armature-redis`'s own newly-added test violated the just-stated rule about `#[ignore]`d container tests in the same branch that stated it; rewritten onto the enforced `RedisContainer`/`skip_if_no_docker!` pattern. Seventy-one `finding N`/`Finding #N` comments across fifteen files cited an internal audit-tracking document excluded by its own `.gitignore` — permanently unverifiable outside the machine that wrote it, with numbering colliding across at least two unrelated sequences — mechanically replaced with the self-contained descriptions most already carried alongside the citation. The WF6 spec doc, frozen since the original implementation commit, gained a short addendum noting three follow-up audit rounds addressed substantially more than its own findings count and pointing to this changelog as authoritative. `retry_after_secs` — independently reimplemented in `armature-mail`, `armature-payments`, and `armature-push` with three different signatures — is now the same shape (`fn(&HeaderMap) -> Option<u64>`) in all three, each still its own private copy (deliberately not consolidated into a shared crate, consistent with this workspace's per-provider-trait architecture).

#### Web & API conformance (Workflow 7)

Six crates (`armature-graphql`, `armature-graphql-client`, `armature-grpc`, `armature-openapi`, `armature-websocket`, `armature-http-client`) shared one pattern more than any other: config knobs and advertised features that were stored but never read, so a caller doing exactly what the documentation said got silent, undetectable non-behavior instead. 45 findings closed (7 Critical, 25 Warning, 13 Info), each with a regression test that failed against the pre-fix code.

**`armature-http-client` had two Critical correctness bugs on its most-advertised path.** `execute_with_retry` always rebuilt the request via `clone_request`, which never copied the body — so **every retried request, including the crate's own documented POST-with-json-and-retry example, sent an empty body**. Separately, `reqwest` returns 5xx/429 as `Ok(Response)`, and the retry loop's success arm always called `record_success()` regardless of status — **the circuit breaker could never open on HTTP-status failures**, defeating the cascading-failure protection it exists to provide. Both are fixed: a `RequestSpec` is now built once and turned into a fresh `reqwest::Request` per attempt (including the first), and `record_failure()`/`record_success()` are now gated on response status, not on the absence of a transport error. The interceptor/middleware system, previously exported but never invoked by `execute()`/`send()`, is now genuinely wired in. `RateLimitInterceptor` sleeps out a parsed `Retry-After` instead of only logging it. `Response::from_reqwest` and `RequestBuilder::json`/`form` now return `Result` instead of silently converting a body-read failure or a serialization failure into an invisible empty/bodyless success.

**`armature-grpc`'s entire middleware and server-auth surface was decorative.** `AuthInterceptor` used server-side authenticated nothing — its one `Interceptor` impl unconditionally ran the *client-side* `add_auth` path and never called `validate`, so a service relying on it for server-side auth rejected zero requests. The five middleware types (`Timeout`/`RateLimit`/`ConcurrencyLimit`/`LoadShedding`/`Retry`) held a config value and a getter each; the `GrpcMiddleware` trait they were meant to implement had zero implementors, so the `MiddlewareLayer<M>` scaffolding built to host them could never be constructed with any of them. The README's advertised rustls TLS support did not exist in the dependency tree at all. `enable_reflection` never registered a reflection service, and health-check registration bound and immediately dropped its `HealthReporter`, so a registered service never reported `SERVING`. All four are now real: `AuthInterceptor` is split into a client path (`add_auth`) and a server path (`server_interceptor()`, which validates and rejects); all five middleware types are real `tower::Service` implementations wired through `GrpcServerBuilder::serve_with_middleware`; server and client TLS (`tonic`'s `tls-ring`, rustls-only) are fully wired via `GrpcServerTlsConfig`/`GrpcClientTlsConfig`; reflection registers a working `tonic_reflection` service; the health reporter is retained and services are marked `SERVING`. `max_recv_message_size` is enforced (via a length-prefix-peeking wrapper service, since the crate's generic `serve<S>` has no access to codegen-only setter methods — `max_send_message_size` remains architecturally out of reach for the same reason and is documented as such); `tcp_keepalive` is applied on both client and server; client `retry_enabled`/`max_retry_attempts` now drive a real retry loop. **Breaking:** `GrpcServerConfigBuilder::build()` and `GrpcServer::bind()` are now fallible (`bind_address` previously panicked on a malformed address via an inline `.unwrap()`).

**`armature-graphql-client`'s `SubscriptionBuilder::header` silently dropped every header it was given.** `execute_subscription` received the caller's headers as a parameter literally named `_extra_headers` and connected with only the bare WebSocket URL — an auth header attached to a subscription never reached the server, a silent authentication bypass on the client's own stated intent. It now builds the WebSocket handshake request with every configured header applied. `retry_enabled`/`max_retries`, likewise read nowhere outside `config.rs`, now drive a real retry loop (transport/5xx only, never retrying a well-formed GraphQL error response); `caching`/`cache_ttl` now back a real in-memory response cache for query operations (mutations are never cached). The advertised-but-nonexistent "Type Generation" and "Apollo Federation" feature claims are removed; `batching`/`max_batch_size`/`batch_delay` — judged lower-value than retry — are removed along with the "automatic batching" claim in favor of documenting the manual `batch()` API that genuinely works. A server `Ping` on a subscription now receives a `Pong` (previously only logged).

**`armature-graphql`'s `GraphQLConfig` was entirely decorative — most consequentially, `production()`'s "introspection disabled for security" claim.** `max_depth`/`max_complexity`/`enable_introspection`/`enable_validation`/`enable_tracing` were real config fields nothing ever applied; every schema was built with a bare `Schema::build(...).finish()`, so a caller configuring `GraphQLConfig::production()` believing introspection was off shipped a fully introspectable, depth-unlimited schema. A new `configure()` path applies all five knobs and is now wired into every schema-building entry point in the crate. `create_merged_schema`, which silently ignored its resolver arguments and returned a builder that panicked on `.build()`, is removed. `resolve_entities`'s hardcoded federation query, which returned only `__typename` regardless of what fields were requested, now selects the caller's requested fields. `SubgraphConfig::with_timeout` is now applied per-request rather than being read nowhere. `FederationGateway`'s documented `.listen(4000)` server and `compose_supergraph`'s "supergraph composition" claim (it only concatenates raw SDL strings) are both corrected to describe what the crate actually does — a real HTTP gateway server and real Apollo-style composition are out of scope for this workflow.

**`armature-websocket` shipped its first test suite alongside these fixes — the crate had zero tests before this.** `max_message_size`, `heartbeat_interval`, and `connection_timeout` were config fields read by nothing on the server; the client's `max_message_size` builder option was equally inert. All three are now enforced (`WebSocketConfig` built from `max_message_size` and passed to `accept_async_with_config`/`connect_async_with_config`; a per-connection heartbeat task sends pings on `heartbeat_interval` and closes after missed pongs; `connection_timeout` bounds the read loop). `WebSocketClient::is_closed()` previously reflected only a locally-set flag — a remote-initiated close or a dead reader/writer task left it reporting an open connection on one that was, in fact, dead, with `send()` continuing to return `Ok` into nothing. The `closed` flag is now set by the reader/writer tasks themselves on any termination path, not only by local `close()`/`Drop`.

**`armature-openapi`'s documented primary usage did not compile.** The README's Quick Start was built around `#[derive(OpenApi)]`, `#[openapi(...)]`, `#[derive(ToSchema)]`, and free functions `swagger_ui()`/`openapi_spec()` — none of which exist anywhere in the crate, which has no proc-macro dependency at all. The README (and a new doctest mirroring it, so this cannot silently drift again) now describe the real `OpenApiBuilder` + `swagger_ui_response`/`spec_json_response` API. The "Auto Generation"/"Type Inference"/"Validation" feature-list bullets, which had no backing implementation, are removed. `SecurityScheme` gained an `OpenIdConnect` variant (previously undrepresentable despite OpenID being an advertised auth scheme), and the "Built-in" Swagger UI framing is corrected to note it loads its assets from a CDN rather than being self-contained.

**Breaking (Workflow 7, all crates above):** `RequestBuilder::json`/`form` (`armature-http-client`) now return `Result`; `RateLimitInterceptor` is constructed via `new()`/`with_max_wait()` rather than as a unit struct. (`Response::from_reqwest` and `HttpClient::execute` also changed shape but are `pub(crate)`, so they are not part of the public breaking surface.) `armature-grpc`'s `GrpcServerConfigBuilder::build()`/`GrpcServer::bind()` are now fallible. `armature-graphql`'s `create_merged_schema` is removed, and `FederationGateway::resolve_entities` gained a required `fields: &str` parameter (it previously hardcoded `__typename`-only selection regardless of what the caller requested — see above). `armature-graphql-client`'s `GraphQLClientConfig` loses its `batching`/`max_batch_size`/`batch_delay` fields and builders. `armature-openapi`'s `SecurityScheme` gained an `OpenIdConnect` variant and is now marked `#[non_exhaustive]` going forward so future variants don't repeat this break silently.

#### Web & API conformance follow-up (Workflow 7, post-review fix pass)

A subsequent `/simplify` + `/audit` + 7-domain `/code-review` pass over the Workflow 7 diff surfaced 34 further findings across the same six crates, all now fixed with regression tests:

**Two of the original 45 "closed" findings were falsely ticked in `TODO.md`.** `armature-http-client`'s `CircuitBreaker::record_failure` computed `Instant::now().elapsed()` on an `Instant` it had *just* created — always ~0ms — so the stored `last_failure_time` never advanced and the documented failure-window reset could never trigger; failures accumulated forever instead of expiring. `armature-grpc`'s `CompressionMiddleware` was ticked fixed on the strength of a comment admitting it was still just a config holder — nothing ever called `.accept_compressed()`/`.send_compressed()` on any service or channel, so no compression was ever actually applied. Both are now genuinely fixed (the circuit breaker stores a real base `Instant`; compression is applied via two new generic `tower::Service` wrappers mirroring `MaxRecvMessageSizeService`'s length-prefix-parsing technique, since tonic only exposes `accept_compressed`/`send_compressed` as inherent methods on codegen-only per-service types, unreachable from the crate's generic `serve<S>`/channel path).

**`armature-grpc`'s `MaxRecvMessageSizeService` had a frame-fragmentation bypass and a swallowed transport error in the same function.** `filter(|d| d.len() >= 5).unwrap_or(false)` treated a short first HTTP/2 DATA frame as "not oversized," so an attacker could fragment the 5-byte gRPC length-prefix across multiple small frames to bypass the size limit entirely; separately, `data.transpose().ok().flatten()` silently discarded a genuine mid-read transport error, forwarding a truncated body instead of propagating the failure. The service now buffers bytes across frames until the length-prefix is actually available before judging size, and surfaces read errors as `Status::internal`. `serve_with_shutdown()` never received the same health/reflection registration `serve()` got in the original pass — both now share one `register_optional_services` helper. Dead code removed: the unused `MiddlewareLayer`/`Layer` bridge, the `RetryMiddleware` `GrpcMiddleware` impl (which had trait bounds incompatible with `serve`'s actual `ServableService` requirement and could never have compiled into a working path), and `AuthInterceptor::client_interceptor`. Server-side auth token/API-key comparisons are now constant-time. Client retry (`GrpcChannel::call_with_retry`) was already correct but silently opt-in — `lib.rs`/`README.md` now say so explicitly and show the wrapping pattern, since the generated client's direct methods bypass it entirely.

**`armature-http-client`'s dead `Middleware`/`MiddlewareChain` module (see note above) is removed**, along with a double body-clone in `execute()` and a bug where an interceptor's body mutation was rebuilt away on every retry attempt after the first (only method/url/headers were copied back into the retry `RequestSpec`, not body) — now verified by a test asserting the mutated body survives every retry attempt, not just the first.

**`armature-graphql`'s `production()` preset set `enable_introspection=false` but left `max_depth`/`max_complexity` at their unlimited `0` defaults** — the DoS-protection half of its own stated purpose was inert. `production()` now sets `max_depth = 15`/`max_complexity = 1000`. `SubgraphSchemaBuilder::build` could double-register the `ApolloTracing` extension when both its own `.enable_tracing()` and an attached config's `enable_tracing` were set; now registered at most once.

**`armature-websocket`'s `connection_timeout` was neutralized by its own default config.** `heartbeat_interval` (30s) is shorter than `connection_timeout` (60s) by default, and the read-timeout future was reconstructed fresh every `select!` iteration — so the heartbeat branch always won the race before 60s of uninterrupted idle time could accumulate, meaning `connection_timeout` never fired on its own (only the separate ~120s missed-heartbeat path closed idle connections). The crate's own regression test only passed because it set `heartbeat_interval` to 3600s, avoiding the interaction. Fixed by tracking the last-read `Instant` as persistent state and racing a `sleep_until` deadline against it, independent of heartbeat cadence; a new test using realistic 1:2 heartbeat:timeout proportions confirms `connection_timeout` now closes an idle connection on its own.

**`armature-graphql-client`'s response cache had no eviction** (now bounded via `lru`, configurable through a new `max_cache_entries` field), and its `default_headers` reaching the WebSocket subscription handshake was correct but untested (now covered).

**Documentation accuracy:** `armature-grpc/src/config.rs`'s `max_send_message_size` field/builder now states in its own rustdoc (not just the changelog) that the value is presently unenforced. `armature-graphql/src/federation.rs`'s module-level "Gateway Composition" doc bullet is aligned with the already-honest function-level doc on `compose_supergraph` (SDL concatenation, not real composition). Roughly two dozen more `Finding N`-style audit-citation comments — the same unverifiable-numbered-citation anti-pattern already purged once this cycle (see the "Documentation and CI accuracy" entry above), reintroduced by this workflow's own implementation pass — are replaced with the self-contained descriptions already alongside them across `armature-grpc`, `armature-graphql`, `armature-http-client`, and `armature-graphql-client`'s source and test files.

#### Web & API conformance, second pass (Workflow 7, full 7-domain `/code-review`)

A full 7-domain `/code-review` over the entire Workflow 7 branch (both commits above) found two more real issues, plus three minor nits, all now fixed:

**`armature-grpc`'s new compression wrappers enabled a decompression-bomb DoS that bypassed `max_recv_message_size`.** The size limit only checked the *compressed* frame length on the wire; `decompress_bytes` (gzip via `GzDecoder`, zstd via `decode_all`) placed no cap on decompressed output size, so a small, highly-compressible payload could expand to gigabytes in memory once decompressed. `CompressionMiddleware` now carries a `max_decompressed_size` (default 64 MiB, configurable via a new builder method) enforced *during* decompression — via a bounded `Read::take` on the gzip path and incremental, size-checked zstd decoding instead of `decode_all` — not after. Separately, `max_recv_message_size` itself only validated the first gRPC message of a call; a client-streaming/bidi RPC could send unbounded further oversized messages after that with no check. `MaxRecvMessageSizeService` now scans every message boundary in the stream, not just the first, via a new stateful `FrameScanState` scanner shared with the buffering path.

**Minor fixes:** `CompressionMiddleware` constructed with `CompressionEncoding::None` no longer enters the buffer/transform path unnecessarily (a `None == None` comparison previously matched whenever a client sent no `grpc-encoding` header at all); compression *encode* failures (as opposed to decode failures, already handled) are now logged via `tracing::warn!` instead of silently falling back to uncompressed with no signal; the gRPC frame-header parsing constants (5-byte prefix layout) duplicated between `middleware.rs` and `server.rs` are consolidated into a shared `GRPC_FRAME_HEADER_LEN`/`read_frame_header` helper; one more leftover `Finding N` comment in `armature-graphql/src/config.rs` (missed by the prior cleanup pass) is corrected; `armature-websocket`'s `#[allow(dead_code)]` on `ConnectionInfo::set_state` now documents why it's unused (reserved for the close handshake's not-yet-wired `Closed` transition).

**Breaking:** `armature-graphql` is bumped to `0.3.0` (not `0.2.0`, since it was already at `0.2.0` from a prior unrelated change) — the earlier Workflow 7 pass removed the public `create_merged_schema` function and added a required `fields: &str` parameter to `FederationGateway::resolve_entities`, both breaking changes under this repo's 0.x convention, that had shipped without a version bump.

#### Web & API conformance, third pass (Workflow 7, second full 7-domain `/code-review`)

A second full 7-domain `/code-review` over the whole branch (run because the compression wrappers had only been exercised by their own happy-path unit tests) verified every prior fix as genuinely correct and found that the gRPC compression feature — despite the second pass's bomb fix — was still broken in three ways sharing one root cause: the wrappers buffered the *entire* body, transformed it, and rebuilt it as a trailer-less `Full` body. This (a) **dropped the HTTP/2 trailers carrying `grpc-status`/`grpc-message`, so every compressed response failed against any real tonic client** — the feature worked only in the crate's own trailer-less unit tests; (b) left aggregate memory unbounded (the per-message decompression cap didn't bound the message *count*, so many small compressed frames each expanding to the cap could still OOM); and (c) stalled streaming RPCs, since nothing was emitted until the whole stream completed. All three are fixed by replacing the buffer-and-rebuild model with an incremental `CompressionBody` `http_body::Body` adapter that transforms one gRPC frame at a time, forwards the trailers frame untouched, enforces `max_decompressed_size` against a running whole-body total, and emits each message as soon as it completes. A regression test now round-trips compression through a real `tonic_health` service and client (it fails against the pre-fix trailer-dropping code).

The same pass also: closed a **GraphQL query-injection** vector in federation `_entities` resolution (`resolve_entities` now validates each `__typename` against the GraphQL Name grammar and rejects mixed-typename batches before splicing anything into the subgraph query); fixed a **server-controlled panic** in `armature-graphql-client` (`error_for_status().unwrap_err()` panicked on an unexpected 3xx such as `304`, now a clean non-retryable error); guarded `armature-http-client` against silently dropping an interceptor-installed **streaming body on retries** (now a `StreamingBodyNotRetryable` error rather than an empty-body resend); stopped `armature-websocket` from closing a **live peer that streams data but never answers Pings** (any inbound read now resets the missed-heartbeat counter, while genuinely idle peers are still reaped); made the decompression-bomb tests **genuinely discriminating** (rebuilt as multi-GiB incremental bombs that a decompress-all implementation would OOM on, since the prior tests passed even against an unbounded implementation) and the `None`-encoding pass-through test exercise the flag=1 case the bug actually affected; added `#[non_exhaustive]` to the newly-public `CompressionEncoding` enum; and cleaned up assorted correctness/maintainability nits (`u32` frame-length truncation guard, zstd decoder window cap, `PrefetchedBody::size_hint` accounting, a stale bind-address builder error, three drifting gRPC client endpoint-config blocks unified into one helper, and several naming/doc consistency fixes).

**Version bump:** `armature-mcp` → `0.1.3` — its dependency requirement on `armature-http-client` was raised to `0.2.0` and its `auth.rs` adjusted on this branch (multibyte-safe token truncation, richer OAuth2 introspection error context) while it still carried the already-published `0.1.2`, which would fail `cargo publish` on the duplicate version.

### Fixed

- Query strings were dropped by the module/controller server — `?a=b` params never reached handlers. They are now parsed and percent-decoded on every `listen*` path.
- Six source modules (`conditional`, `content_negotiation`, `response_cache`, `error_correlation`, `error_transform`, `exception_filter`) were never declared and so never compiled — which also broke the `#[catch]` macro. They are now wired into the crate.
- Numerous correctness/soundness/security fixes in `armature-core`: use-after-free in `cow_state`, dangling reference in `cache_local`, unsound `SoaStorage`, bulkhead/circuit-breaker/streaming cancellation and race bugs, HTTP/3 responses dropping headers/cookies/body, cross-user response-cache leakage, SQL injection in pagination sort, per-request `Box::leak` in wildcard routing, multipart binary corruption, and UTF-8 mishandling in percent-decoding and content negotiation.

---

## [0.2.0] - 2026-02-02

Major release featuring Rust 2024 edition upgrade, new application builder, enhanced CLI, and HTTP handler improvements.

### Added

#### HTTP Handler Enhancements (`armature-core`, `armature-proc-macro`)
- `#[options]` proc macro attribute for custom OPTIONS route handlers
- `#[head]` proc macro attribute for HEAD request handlers
- `Router::options()` and `Router::head()` fluent methods for programmatic routing
- Full support for CORS preflight and resource metadata checks

#### Application Builder (`armature-app`)
- New `armature-app` crate with Rhai scripting support
- Declarative application configuration via Rhai scripts
- Dynamic route registration and middleware configuration
- Hot-reload support for development

#### CLI Enhancements (`armature-cli`)
- Prax ORM support for code generation
- Comprehensive code generation templates
- Improved project scaffolding

#### Messaging (`armature-messaging`)
- MQ-Bridge integration for unified messaging across brokers
- Support for RabbitMQ, Kafka, NATS, and Redis Streams

#### Security
- CodeQL security analysis workflow for automated vulnerability scanning

### Changed

- **Rust 2024 Edition** - Upgraded entire workspace to Rust 2024 edition
- **MSRV** - Minimum supported Rust version updated to 1.89
- Converted let-chains for Rust 2024 compatibility
- Various dependency updates for compatibility

### Fixed

- Fixed clippy warnings for Rust 2024 edition compatibility
- Fixed MSRV-related compilation issues

---

## [0.1.0] - 2025-12-21

Initial public release of the Armature framework - a high-performance, type-safe HTTP framework for Rust inspired by Angular and NestJS.

### Added

#### Logging (`armature-log`)
- JSON and Pretty logging formats with environment variable configuration
- `ARMATURE_DEBUG`, `ARMATURE_LOG_LEVEL`, `ARMATURE_LOG_FORMAT` controls
- `trace!`, `debug!`, `info!`, `warn!`, `error!` macros
- Optional tracing integration
- Runtime-configurable log levels and formats

#### Internationalization (`armature-i18n`)
- Message translation with Fluent syntax
- Locale detection from Accept-Language headers
- CLDR-compliant pluralization rules
- Date, number, and currency formatting

#### Database Integration
- **`armature-diesel`** - Diesel async with connection pooling
- **`armature-seaorm`** - SeaORM integration with active record pattern

#### Search (`armature-opensearch`)
- OpenSearch/Elasticsearch client
- Document management and bulk operations
- Query DSL builder

#### Serialization (`armature-toon`)
- TOON (Token-Oriented Object Notation) support for LLM-optimized serialization

#### Compression (`armature-compression`)
- Streaming compression (gzip, brotli, zstd)
- Backpressure handling
- Response compression middleware

#### Fuzzing (`armature-fuzz`)
- 8 fuzz targets for security testing
- HTTP request/response, routing, JSON, URL parsing

#### Performance Optimizations
- 65+ performance optimizations implemented
- SIMD HTTP parsing and JSON serialization
- Zero-copy request/response handling
- Arena allocators for per-request batch allocations
- HTTP/1.1 pipelining and request batching
- `io_uring` backend for Linux
- Connection pooling and keep-alive optimization
- Thread-local buffer pools
- `matchit` router for O(log n) routing
- SmallVec headers and CompactString paths

#### Publishing Tools
- `scripts/publish.sh` - Automated crates.io publishing with rate limiting
- `scripts/prepare-publish.sh` - Path-to-version dependency conversion
- `scripts/pgo-build.sh` - Profile-Guided Optimization workflow

#### Cloud Provider SDKs
- **`armature-aws`** - AWS SDK integration with feature-gated services
  - S3, DynamoDB, SQS, SNS, SES, Lambda, Secrets Manager, KMS, Cognito
  - Dynamic service loading via feature flags
  - DI container integration
  - Environment-based configuration
  - LocalStack emulator support
- **`armature-gcp`** - Google Cloud SDK integration
  - Cloud Storage, Pub/Sub, Firestore, Spanner, BigQuery
  - Feature-gated compilation
  - GCP emulator support
- **`armature-azure`** - Azure SDK integration
  - Blob Storage, Queue Storage, Cosmos DB, Service Bus, Key Vault
  - Azurite emulator support

#### Serverless Deployment
- **`armature-lambda`** - AWS Lambda runtime for Armature
  - API Gateway, ALB, and Function URL support
  - Request/response conversion
  - Cold start optimization
  - Lambda-specific Dockerfile templates
- **`armature-cloudrun`** - Google Cloud Run deployment
  - Health check utilities
  - Cloud Logging integration
  - Graceful shutdown support
  - Cloud Build configuration
- **`armature-azure-functions`** - Azure Functions custom handler
  - HTTP trigger support
  - Request/response bindings
  - Azure Container Apps deployment

#### Redis Integration
- **`armature-redis`** - Centralized Redis client crate
  - Connection pooling with bb8
  - Pub/Sub messaging support
  - Cluster, TLS, and Sentinel support
  - Shared across all crates (cache, queue, distributed, ratelimit, session)

#### HTTP Client
- **`armature-http-client`** - Production-ready HTTP client
  - Automatic retry with configurable backoff (constant, linear, exponential, jitter)
  - Circuit breaker integration
  - Request/response interceptors
  - Middleware chain support
  - Timeout policies

#### gRPC Support
- **`armature-grpc`** - gRPC server and client
  - Tonic-based implementation
  - Interceptors for auth and metrics
  - Health checking and reflection
  - Type aliases for complex signatures

#### GraphQL Client
- **`armature-graphql-client`** - GraphQL client for federation
  - Query batching
  - Subscription support via WebSocket
  - Automatic retry
  - Variables and fragments

#### Email System
- **`armature-mail`** - Comprehensive email module
  - SMTP transport with TLS/STARTTLS
  - Provider integrations: SendGrid, AWS SES, Mailgun
  - Handlebars email templates
  - Attachment support (inline and download)
  - Email queue with async sending, retries, and dead letter queue
  - Redis-backed queue storage

#### Push Notifications
- **`armature-push`** - Multi-platform push notifications
  - Web Push with VAPID
  - Firebase Cloud Messaging (FCM)
  - Apple Push Notification Service (APNS)
  - Unified push service API
  - Batch sending support
  - Device token management

#### File Storage
- **`armature-storage`** - File storage abstraction
  - Local filesystem storage
  - AWS S3 with presigned URLs and server-side encryption
  - Google Cloud Storage with signed URLs
  - Azure Blob Storage with Azurite support
  - Multipart upload handling with streaming
  - File validation (type, size, extension)

#### Resilience Patterns
- **`armature-core/resilience`** - Production resilience patterns
  - Circuit Breaker (Open/Closed/Half-Open states, sliding window)
  - Retry with Backoff (constant, linear, exponential, jitter)
  - Bulkhead (semaphore-based concurrency limiting)
  - Timeout policies
  - Fallback handlers with chains

#### CLI Enhancements
- Interactive project creation wizard
- `armature add <feature>` - Add features to existing projects
- `armature check` - Validate project configuration
- `armature routes` - List all registered routes
- `armature config:check` - Validate configuration files
- Shell completions (bash, zsh, fish, PowerShell)
- Improved colored output and progress indicators

#### Developer Experience
- Prelude modules added to all major crates for easier imports
- `Result<T>` type aliases in crates with Error types
- Convenience methods on `HttpResponse` (ok, created, no_content, bad_request, etc.)
- Convenience methods on `Container` (require, get_or_default, register_if_missing)
- Enhanced error messages with actionable suggestions
- Debug and Display implementations for all public types

#### Cookbook Examples
- `examples/crud_api.rs` - Complete REST API with CRUD operations
- `examples/auth_api.rs` - JWT authentication flow
- `examples/realtime_api.rs` - WebSocket/SSE real-time communication

#### Benchmarks
- `benches/resilience_benchmarks.rs` - Circuit breaker, retry, bulkhead, timeout
- `benches/cache_benchmarks.rs` - Cache operations and tiered caching
- `benches/auth_benchmarks.rs` - Password hashing and JWT operations
- `benches/ratelimit_benchmarks.rs` - Rate limiting algorithms
- `benches/storage_benchmarks.rs` - File validation and storage operations
- `benches/http_client_benchmarks.rs` - HTTP client patterns

#### DevOps Templates
- **Dockerfile templates** (Alpine-based, multi-stage, cargo-chef)
  - `templates/api-minimal/Dockerfile`
  - `templates/api-full/Dockerfile`
  - `templates/microservice/Dockerfile`
  - `templates/graphql-api/Dockerfile`
  - `templates/lambda/Dockerfile` (x86_64 and ARM64)
  - `templates/cloudrun/Dockerfile`
  - `templates/azure-container/Dockerfile`
- **Docker Compose** for all templates with development services
- **Kubernetes manifests** (`templates/k8s/`)
  - Deployment, Service, Ingress
  - HPA, PDB, NetworkPolicy
  - ConfigMap, Secret, ServiceAccount
  - Kustomization base
- **Helm chart** (`templates/helm/armature/`)
  - Production-ready values
  - Configurable replicas, resources, probes
  - Ingress and service configuration
- **CI/CD workflows**
  - GitHub Actions (CI, Release, Docs, PR automation)
  - Jenkins pipelines (basic, Docker agent, multibranch)

#### Documentation
- `docs/cloud-providers-guide.md` - AWS, GCP, Azure SDK usage
- `docs/redis-guide.md` - Centralized Redis integration
- `docs/dependency-injection-guide.md` - Advanced DI patterns
- Updated `docs/README.md` with comprehensive documentation index
- Angular-based docs overview component with proper routing

#### SEO & AI SEO
- Comprehensive `index.html` meta tags
  - Open Graph and Twitter Card tags
  - JSON-LD schemas (SoftwareApplication, Organization, WebSite, FAQPage, BreadcrumbList)
- `robots.txt` with 15+ AI crawler rules (GPTBot, Claude, Bingbot, etc.)
- `sitemap.xml` expanded to 35+ URLs
- `llms.txt` - AI-readable project summary (llmstxt.org standard)
- `ai.txt` - AI interaction guidelines and code generation style
- `.well-known/security.txt` - Security vulnerability reporting
- `humans.txt` - Team credits and technology stack

### Changed

- **Web app theme**: Migrated to Tailswatch oxide dark theme
- **Documentation structure**: Flattened docs/ directory (removed guides/ subfolder)
- **Mobile navigation**: Fixed menu collapse behavior
- **Comparisons page**: Refactored to emphasize Armature strengths
- **Roadmap**: Updated to show 98% feature completion
- Updated all URLs from `quinnjr.github.io` to `pegasusheavy.github.io`
- Renamed builder methods to use `with_*` pattern for better ergonomics
- Updated `tonic` to 0.14, `prost` to 0.14
- Updated `redis` to 1.0, `bb8-redis` to 0.18
- Updated `lambda_http` and `lambda_runtime` to 1.0
- Updated `web-push` to 0.11

### Fixed

- Fixed `HealthChecker` trait for object safety (dyn compatibility)
- Fixed `lambda_http` and `aws_lambda_events` API compatibility
- Fixed `web_push` crate API changes
- Fixed `tonic` gRPC framework API changes
- Fixed benchmark compilation errors
- Fixed clippy warnings across all crates
- Removed all `unsafe` code blocks (replaced with safe alternatives)
- Fixed mobile menu not closing when navigation link clicked

### Removed

- **`armature-di`** crate - Use `dependency-injector` crate directly
- Removed `unsafe impl Send/Sync` blocks (now compiler-verified)
- Removed `unsafe env::set_var` from tests

---

## Rate Limiting Module (`armature-ratelimit`)
- New `armature-ratelimit` crate for comprehensive API rate limiting
- **Algorithms**:
  - Token Bucket - smooth rate limiting with burst capacity
  - Sliding Window Log - precise rate limiting with timestamp tracking
  - Fixed Window - simple fixed time window counters
- **Storage Backends**:
  - `MemoryStore` - in-memory storage using DashMap (default)
  - `RedisStore` - Redis-backed distributed storage (optional `redis` feature)
- **Key Extraction**:
  - By IP address, user ID, API key, or custom headers
  - `KeyExtractorBuilder` for complex extraction logic
  - Per-endpoint rate limiting with `IpAndPath` extractor
- **Middleware**:
  - `RateLimitMiddleware` ready for HTTP integration
  - Standard headers: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`, `Retry-After`
  - Bypass keys for whitelisting specific clients
  - Fail-open mode for high availability
- Rate limiting example (`examples/rate_limiting.rs`)
- Comprehensive documentation (`docs/rate-limiting-guide.md`)

#### Armature CLI (`armature-cli`)
- New `armature-cli` crate for code generation and development tools
- **Commands**:
  - `armature new <name>` - Create new projects from templates (minimal, full, microservice)
  - `armature generate controller <name>` - Generate controllers with optional CRUD
  - `armature generate service <name>` - Generate injectable services
  - `armature generate module <name>` - Generate modules with controllers and providers
  - `armature generate middleware <name>` - Generate middleware
  - `armature generate guard <name>` - Generate route guards
  - `armature generate resource <name>` - Generate complete resource (controller + service + module)
  - `armature dev` - Development server with file watching and hot reloading
  - `armature build` - Production build with size reporting
  - `armature info` - Display project information
- **Features**:
  - Template-based code generation using Handlebars
  - Automatic `mod.rs` updates when generating code
  - Test file generation (optional)
  - Progress indicators and colored output
  - Uses `cargo-watch` if installed for better performance

#### Project Templates
- New `templates/` directory with starter templates:
  - **api-minimal** - Single-file REST API for learning and prototyping
  - **api-full** - Production-ready API with JWT auth, validation, Docker, health checks
  - **microservice** - Queue-connected worker with Prometheus metrics and graceful shutdown
  - **graphql-api** - GraphQL API template
- Template documentation (`docs/project-templates.md`)
- Each template includes:
  - `Cargo.toml` with appropriate dependencies
  - `.env.example` for configuration
  - `Dockerfile` and `docker-compose.yml` where applicable

#### Core Framework
- Initial release of Armature framework
- Core framework with dependency injection and decorators
- Authentication support (JWT, OAuth2, SAML, 2FA, Passwordless)
- GraphQL support
- Validation framework
- Testing utilities
- OpenAPI/Swagger integration
- Caching (Redis, Memcached)
- Job queue system
- Cron scheduling
- OpenTelemetry observability
- Security middleware (Helmet-like)
- HTTPS/TLS support
- Static asset serving with compression
- Comprehensive debug logging throughout the framework
- 30+ working examples refactored to use module/controller pattern
- Angular 21 documentation website with:
  - Tailwind CSS 4 styling with Tailswatch oxide theme
  - SPA routing with 404.html fallback for GitHub Pages
  - Vitest for unit testing
  - API documentation integration at `/api/`

### Security
- Added cargo-husky for Git hooks (pre-commit, pre-push, commit-msg)
- Branch protection via Git hooks
- Automated linting and testing on commits
- Comprehensive `.gitignore` and `.dockerignore`

## Version History

### Versioning Strategy

We follow [Semantic Versioning](https://semver.org/):

- **MAJOR** version when making incompatible API changes
- **MINOR** version when adding functionality in a backward compatible manner
- **PATCH** version when making backward compatible bug fixes

### Release Schedule

- **Major releases**: When significant breaking changes are necessary
- **Minor releases**: Every 2-3 months with new features
- **Patch releases**: As needed for bug fixes and security updates

### Upgrade Guide

See [docs/migration.md](docs/migration.md) for detailed upgrade instructions between major versions.

---

[Unreleased]: https://github.com/PegasusHeavyIndustries/armature/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/PegasusHeavyIndustries/armature/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/PegasusHeavyIndustries/armature/releases/tag/v0.1.0
