# Workflow 6 — Delivery Providers: Implementation Plan

**Spec:** `docs/superpowers/specs/2026-07-20-workflow6-delivery-providers-design.md`
**Findings:** `.superpowers/sdd/wf6-findings.md` (55: 7C/33W/15I)
**Branch:** `feature/wf6-delivery-providers` → PR to `develop` (HELD for user audit window)

## Execution model

Five disjoint crates → one edit-only implementation agent per crate, in parallel (the WF3 pattern).
No agent touches CHANGELOG, crate `version`, or git; the coordinator does the central gate,
semver bumps, CHANGELOG, TODO tick, and commit. Priority order (governs model tier + review depth):
**payments (security) → mail → files → storage → push.**

## Tasks

### T1 — armature-payments (3C/11W/1I) [opus — security]
1. **PayPal `verify_webhook`**: POST `/v1/notifications/verify-webhook-signature` with `webhook_id`
   + transmission headers; reject on non-SUCCESS with `InvalidWebhookSignature`.
2. **Braintree `verify_webhook`**: verify `bt_signature` (HMAC-SHA1 over payload, key pair),
   **constant-time** compare.
3. **Braintree `create_payment_method`**: stop sending `"fake-valid-nonce"` — accept a client nonce
   or return an explicit unsupported error.
4. Stripe: status-check every non-charge method; `delete_customer` checks status; populate
   `price_id`/`quantity` from items; handle or reject `PaymentMethod`/`Bank` sources.
5. PayPal: real `update_subscription` (revise) or unsupported error; stop fabricating customer IDs
   and billing periods; honor partial `capture` amounts (both providers).
6. Consume `ProcessorConfig` (retry/idempotency/logging) and `ChargeRequest.idempotency_key`/
   `statement_descriptor`. Braintree `list_payment_methods` returns real methods.
7. Tests: forged webhook rejected + valid accepted (both providers, StubServer); Stripe signature
   verification incl. constant-time; provider error mapping.

### T2 — armature-mail (2C/7W/2I) [opus]
1. Implement `TeraEngine` + `MiniJinjaEngine` (deps declared) or remove feature/re-export/claim.
2. SES raw-MIME (`EmailContent::raw`) so attachments send; error instead of silent drop.
3. Emit custom headers + `priority` (X-Priority/Importance) in `to_lettre` and provider payloads.
4. Inline attachments: Content-ID + `Content-Disposition: inline` so `cid:` resolves.
5. `job_timeout` wraps sends; `RedisBackend::pop` uses MGET/pipeline; `enqueue_batch` pipelines.
6. Fix `MailerQueueExt::queue` doc; surface address-parse errors instead of dropping recipients.
7. Tests: MIME assembly (attachment/inline/headers/priority), RedisContainer queue behavior.

### T3 — armature-files (2C/5W/3I) [sonnet]
1. `OutputFormat::Original` round-trips → unblocks `MultiSizeBuilder::generate()`.
2. `TextWatermark` renders real glyphs (ab_glyph/imageproc) **or** is renamed to a redaction box.
3. Convert errors on unsupported target/input (no silent passthrough); WebP honors quality;
   `AutoOrient` applies EXIF orientation; PDF horizontal line emits a stroke operator.
4. `MultiSizeBuilder` decodes the source once. Wire or remove `TextAlign`/`Avif`.
5. Tests: Original passthrough, WebP quality differs, AutoOrient rotates, watermark renders,
   PDF line present, MultiSize/convert paths.

### T4 — armature-storage (0C/6W/6I) [sonnet]
1. Apply `public_access` (predefined ACL), S3 `region`, `storage_class`, GCS `endpoint`.
2. Azure `temporary_url` → `Ok(None)` until SAS signing exists (no permanent unsigned URL).
3. Enforce `MultipartConstraints` (sizes/counts/field allowlist) and re-export it.
4. Consume or drop `project_id` + the presigned/signed/SAS duration knobs.
5. Allowed-types/extensions reject untyped/extensionless uploads. StubServer tests for S3/GCS/Azure.

### T5 — armature-push (0C/4W/3I) [sonnet]
**Verify WF3 state first** (transport/SSRF/tests/size fix already landed).
1. README → real API; drop or implement "Topics".
2. Map `Notification.urgency` → Web Push `Urgency` header; honor per-notification APNS `topic`.
3. Bounded-concurrency `send_batch`/`send_to_tokens`.
4. Consume or remove `WebPushConfig::public_key`; confirm the `PayloadTooLarge` size fix; add
   FCM/APNS status→error contract tests.

## Verification (central, after all tasks)
1. `cargo fmt --all`.
2. Strict `clippy --workspace --all-targets --features full-with-saml -- -D warnings -A collapsible_if
   -A result_large_err -A dead_code -A useless_vec -A unwrap_or_default`.
3. `cargo test` for the 5 crates + any consumers (Docker present → container/stub tests run).
4. `cargo audit`; MSRV `cargo +1.89 check` (vet any new dep: rasterizer, sha1).
5. Semver: bump each crate with a breaking/behavior change to its next 0.x minor; update internal +
   root pins; CHANGELOG (Added/Changed-breaking/Fixed/**Security** for the webhook bypasses).
6. Tick the 55 `TODO.md` boxes; the five summary rows → 0|0|0; update the header total.

## Then
Adversarial review of the payments webhook verification (security-critical) → fix → gate green →
**PR to `develop` HELD** for the user's `/simplify` `/optimize` `/audit` `/code-review` window.
Note CodeQL is now a required check, so the PR must be CodeQL-clean to merge.
