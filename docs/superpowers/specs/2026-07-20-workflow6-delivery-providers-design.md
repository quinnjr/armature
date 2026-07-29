# Workflow 6 — Delivery Providers

**Date:** 2026-07-20
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 6 of 9)
**Crates:** armature-mail, armature-push, armature-payments, armature-storage, armature-files
**Findings:** 7 Critical · 33 Warning · 15 Info (55 total; see `.superpowers/sdd/wf6-findings.md`)

**Status at merge:** this document is an accurate snapshot of round-1 scope only — it is not a map of what finally shipped. Three subsequent audit-and-fix commits on this branch (`fix(wf6): audit-battery follow-up`, `fix(wf6): second audit battery`, `fix(wf6): third audit battery`) found and closed 105 + 25 + 25 = 155 further findings across all five crates, including per-crate scoping drift (e.g. `armature-push` gained substantial new SSRF-hardening work not reflected below). See `CHANGELOG.md` for the authoritative description of what actually shipped.

## Problem

The five "delivery provider" crates advertise integrations that silently do nothing, drop data, or — in the payments crate — **accept forged input as authentic**:

- **armature-payments has two webhook-signature bypasses.** `PayPalProvider::verify_webhook` and `BraintreeProvider::verify_webhook` ignore the payload and signature and unconditionally return `Ok(())`. `PaymentProcessor::handle_webhook` calls verification *before* parsing, so **any attacker-forged PayPal or Braintree webhook is accepted as genuine** — an attacker can fabricate payment-succeeded events. `BraintreeProvider::create_payment_method` additionally discards the caller's card and sends the literal sandbox string `"fake-valid-nonce"`.
- **armature-mail silently drops data.** SES sends `EmailContent::simple` built from subject/text/html only, so **attachments vanish with no error**; custom headers and `priority` are stored but never emitted; inline attachments never get a Content-ID, so every documented `cid:` image reference fails over SMTP. Two advertised template engines (`TeraEngine`, `MiniJinjaEngine`) are empty structs with TODOs.
- **armature-files' flagship image paths error or lie.** `MultiSizeBuilder` defaults to `OutputFormat::Original`, which hits a catch-all and returns `UnsupportedFormat("Original")` — so `.with_thumbnails().generate()` **fails on every call**. `TextWatermark` renders no text at all (it fills alternating rows to make a striped box). `AutoOrient` is a no-op; WebP quality is ignored.
- **armature-storage config knobs are inert.** `public_access`, `region`, `storage_class`, `endpoint`, and every duration knob are read nowhere; `AzureBlobStorage::temporary_url` returns a **permanent unsigned URL** where a time-limited signed URL is contracted; `MultipartConstraints` enforce nothing.
- **armature-push** advertises a README API that does not exist plus an unimplemented "Topics" feature, and ignores per-notification `urgency`/`topic`.

## Goal

Make every advertised unit in these five crates conformant. Close the two webhook bypasses first, then the data-loss and always-failing paths, then the inert config knobs and stale docs. Verify with `armature-testkit` (`StubServer` for provider HTTP, `RedisContainer` for the mail queue) plus pure-unit tests — no live provider credentials in CI.

Non-goals: new provider backends; implementing Braintree/PayPal features the APIs don't expose (return an explicit unsupported error instead of fabricating data); a full PDF layout engine.

## Approach

One workflow → one PR to `develop`. The five crates are **disjoint**, so implementation fans out one edit-only agent per crate (the WF3 pattern), ordered by severity: **payments → mail → files → storage → push**. Reuse over re-implementation:

- **PayPal** webhook verification via its `/v1/notifications/verify-webhook-signature` API using the configured `webhook_id` + the request headers; **Braintree** via HMAC-SHA1 of the payload against the key pair (its documented scheme). Use the existing `hmac`/`sha1`/`sha2` stack; compare in **constant time** (the Stripe path's non-constant-time compare is fixed too).
- **lettre**'s `SinglePart`/`ContentType`/Content-ID for inline attachments and raw-MIME SES sending (`EmailContent::raw(RawMessage)`); `mail-builder`/`to_lettre()` for the MIME assembly.
- **image** crate's `DynamicImage::apply_orientation` for AutoOrient, a real WebP quality encoder path, and `imageproc`/`ab_glyph` for glyph rendering in `TextWatermark` (or rename the op to a redaction box if a font dependency is unacceptable — decided in the plan).
- AWS/GCS/Azure SDK builders for `region`, `storage_class`, `endpoint`, predefined-ACL, and SAS generation.

### Verification (via `armature-testkit`, `docker_available()`-gated where containers are used)
- **StubServer** — payments: a forged webhook (bad/absent signature) must be **rejected**, a valid one accepted; Stripe non-2xx surfaces the real provider error, not `Serialization`. Storage: S3/GCS/Azure endpoint override to assert region/storage-class/ACL/SAS request shape. Push: FCM/APNS status→error mapping, APNS per-notification topic header, Web Push Urgency header. Mail: SendGrid/Mailgun/SES request shape incl. attachments.
- **RedisContainer** — mail queue `pop` batching (MGET), `job_timeout` enforcement, `enqueue_batch` pipelining.
- **Pure-unit** — image ops (Original passthrough, WebP quality differs, AutoOrient rotates per EXIF, watermark actually renders glyphs), PDF line operator emitted, multipart constraints enforced, MIME assembly (attachments present, inline `cid:` resolvable, custom headers + priority emitted), address-parse errors surfaced.
- Every implemented unit gets a regression test that **fails against the current code**.

### Conventions
- rustls-only; heavy/native deps stay behind existing features; per-crate minimal `tokio`.
- Where a provider genuinely cannot support a contracted operation (PayPal customer creation, Azure SAS before signing is implemented), return an **explicit unsupported error** — never fabricate an ID, a date, or a success.
- Breaking changes (fallible `verify_webhook` behavior, removed fake-nonce path, `Ok(None)` from `temporary_url`) get a CHANGELOG Breaking entry + a 0.x minor bump per the project's semver rule.

## Work breakdown (security-first)

### armature-payments (3C/11W/1I) — FIRST
- **C (sec)** real PayPal + Braintree webhook verification, constant-time compare; **C** drop the hardcoded Braintree nonce.
- **W** status-checking on every Stripe method; honor `ProcessorConfig` (retry/idempotency/logging); real `update_subscription`/`list_payment_methods`; stop fabricating PayPal customers/periods; partial `capture` amounts; consume `idempotency_key`/`statement_descriptor`; handle or reject `PaymentMethod`/`Bank` sources.
- **I** test Stripe signature verification (valid + forged) with constant-time compare.

### armature-mail (2C/7W/2I)
- **C** implement `TeraEngine`/`MiniJinjaEngine` (deps already declared) or remove the feature+re-export+claim.
- **W** SES raw-MIME so attachments send; emit custom headers + priority; inline attachments get Content-ID/`inline`; `job_timeout` applied; `RedisBackend::pop` MGET batching; fix the `MailerQueueExt::queue` doc.
- **I** pipeline `enqueue_batch`; surface address-parse errors instead of dropping recipients.

### armature-files (2C/5W/3I)
- **C** `OutputFormat::Original` round-trips (unblocks `MultiSizeBuilder::generate`); `TextWatermark` renders real glyphs (or is renamed to a redaction box).
- **W** Convert errors on unsupported target/input instead of silently passing through; WebP honors quality; AutoOrient applies EXIF; PDF horizontal line emits a stroke; decode the source once in `MultiSizeBuilder`.
- **I** wire or remove `TextAlign`/`Avif`; test the MultiSize/convert paths.

### armature-storage (0C/6W/6I)
- **W** apply `public_access` ACL, `region`, `storage_class`, GCS `endpoint`; Azure `temporary_url` returns `Ok(None)` until SAS exists; enforce `MultipartConstraints`.
- **I** consume `project_id` and the duration knobs (or drop them); reject untyped/extensionless uploads under an allowlist; add StubServer tests for the three cloud backends.

### armature-push (0C/4W/3I) — verify WF3 state first
- **W** rewrite README to the real API and drop/implement "Topics"; map `urgency` to the Web Push header; honor per-notification APNS `topic`; bounded-concurrency batch sends.
- **I** consume or remove `WebPushConfig::public_key`; confirm the `PayloadTooLarge` size fix from WF3; add FCM/APNS contract tests.

## Success criteria
- All 7 Critical + 33 Warning implemented with regression tests that failed against the old code; the 15 Info reconciled. **A forged PayPal/Braintree webhook is rejected**, SES sends attachments, `MultiSizeBuilder::generate()` succeeds, and no storage knob is silently ignored.
- `cargo test` green for all five crates (container/stub tests run when Docker present, self-skip otherwise); strict `clippy --workspace --features full-with-saml -D warnings`, `cargo audit`, MSRV 1.89, and **CodeQL** (now a required check) all clean.
- No provider silently succeeds: unsupported operations return explicit errors rather than fabricated IDs, dates, or `Ok(())`.

## Risks
- **Provider API fidelity without live credentials** — signature schemes and request shapes are asserted against StubServer with recorded/synthetic fixtures; live-provider tests stay `#[ignore]`d.
- **Font dependency for text watermarking** — if adding a rasterizer is unacceptable, rename the op to match the redaction-box behavior rather than shipping a misleading name.
- **Breaking behavior changes** (verification now rejects, `temporary_url` → `None`) — semver bump + CHANGELOG; these are security/correctness fixes and are intended.
