# Workflow 1 — Auth & Security

**Date:** 2026-07-19
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 1 of 9)
**Crates:** armature-auth, armature-jwt, armature-security, armature-siem, armature-mcp
**Findings:** 10 Critical · 17 Warning · 8 Info (from the conformance audit / `TODO.md`)

## Problem

The five auth/security crates advertise security controls their code does not implement.
Several are exploitable, not merely incomplete:

- **armature-mcp `authenticate_jwt`** — documented "JWT with signature verification" but only
  base64url-decodes the payload and discards the signature. **Complete auth bypass** — any
  forged `sub`/`scope`/`exp` passes.
- **armature-auth SAML `validate_response`** — accepts any attacker base64 XML with a
  `<NameID>` as a valid SSO assertion. No signature/issuer/audience/expiry checks. **SSO auth
  bypass.**
- **armature-security `RequestSigner`** — "HMAC-SHA256" is actually `Sha256(secret || msg)`, a
  length-extension-forgeable prefix hash, not a MAC.
- **armature-siem `TcpTransport::send`** — a "TLS" syslog channel silently sends security events
  over **plaintext**.
- **armature-siem Sentinel `HttpTransport`** — sends `config.token` as the Authorization header
  instead of the required Azure `SharedKey` HMAC signature, so every send fails auth.
- **armature-jwt `generate_token_pair`/`refresh_token`** — the refresh token is byte-identical to
  the access token with the same `exp`, so refresh can never succeed after expiry.

## Goal

Make every advertised unit in these five crates conformant. Implement the 10 Critical and 17
Warning findings for real; reconcile the 8 Info findings (mostly stale docs and missing tests).
When done, every corresponding `TODO.md` checkbox is ticked and the security controls actually
work. Verify with `armature-testkit` (stub servers, no live credentials).

Non-goals: new auth features beyond what is advertised; changing the framework's DI/guard
conventions.

## Approach

One workflow → one PR to `develop`. Tasks are ordered **Critical → Warning → Info**, and grouped
so related fixes in a crate land together. Key reuse (avoid re-implementing crypto/JWT):

- **MCP + JWT verification** delegate to `armature-jwt`'s `JwtManager` (already real after the
  core session) rather than hand-rolled decode paths.
- **HMAC** (security `RequestSigner`, SIEM Sentinel SharedKey) uses the `hmac` + `sha2` crates.
- **SAML signature verification** uses the `samael` crate (SAML2 with xmlsec) — the single
  largest piece; if `samael` cannot be integrated cleanly, the fallback is to verify the
  enveloped XML signature via `xmlsec`/`openssl` primitives, but `samael` is preferred.

### Verification (via `armature-testkit`)

- **Pure-logic** (no network): JWT `exp` handling, HMAC correctness (test vectors), CEF slot
  assignment, SAML assertion parsing/validation against fixed signed/unsigned XML fixtures, MCP
  JWT signature accept/reject. Unit tests in the default suite.
- **HTTP integrations** (stub server): Microsoft Graph `/me` mapping, OIDC `id_token` extraction,
  SIEM HTTP/Sentinel/Elastic sends (assert the right headers/signature/body/retry), SAML IdP
  metadata fetch. Use `armature_testkit::StubServer`.
- **TLS transport** (SIEM): verify a real TLS handshake against a local rustls server (a small
  in-test TLS listener) — proves the channel is encrypted, not plaintext.
- Every implemented unit gets a regression test that **fails against the current code**.

### Conventions

- rustls-only; per-crate minimal `tokio` features; strict pre-commit clippy must pass.
- `armature-testkit` is added as a `dev-dependency` to each of the five crates.
- New runtime deps (`hmac`, `samael`, `flate2` for SIEM gzip) are added minimally and, where
  heavy/optional (samael/SAML), kept behind the crate's existing feature flags.

## Work breakdown (Critical-first)

### armature-jwt (2C, 2W, 2I) — do first; MCP depends on it
- **C** `generate_token_pair`: give the refresh token a distinct `exp = now + refresh_expires_in`.
- **C** `refresh_token`: re-issue with fresh access/refresh `exp` (strip incoming `exp`).
- **W** `TokenPair` metadata / `with_refresh_expiration` now agree with real lifetimes (fixed by
  the two Criticals; add asserting tests).
- **I** README API drift; add the refresh-flow test (decode both tokens, assert refresh `exp` >
  access `exp`, round-trip refresh).

### armature-mcp (2C, 1W, 2I)
- **C** `authenticate_jwt`: verify the signature via `armature-jwt` `JwtManager` (HS*/RS*/ES* per
  `JwtAuth.secret`/`public_key`/`algorithm`); reject on mismatch. Consumes the `JwtAuth` builder
  fields (the W finding).
- **C** `#[mcp]` attribute: re-export `pub use armature_proc_macro::mcp;` and fix `mcp_impl` to emit
  a fn-pointer handler matching `ToolHandlerFnPtr`; add a trybuild case.
- **I** cursor pagination (implement or drop from the surface); add JWT signature-rejection tests.

### armature-security (1C, 2W, 3I)
- **C** `RequestSigner`: real `Hmac::<Sha256>` (add `hmac`).
- **W** CSP `report_only` emits `Content-Security-Policy-Report-Only`; README rewritten to the real
  API (SecurityMiddleware/CorsConfig/RequestSigningMiddleware).
- **I** doc/default reconciliations: `FrameGuard::AllowFrom` (legacy note), Expect-CT (deprecated,
  drop from defaults), default `X-XSS-Protection` → `0` (Helmet parity).

### armature-siem (3C, 5W)
- **C** `TcpTransport::send`: real TLS via tokio-rustls when `tls` is set (honor `tls_verify`/
  `ca_cert_path`), else error — never downgrade to plaintext.
- **C** Sentinel `HttpTransport`: implement the Azure SharedKey signing scheme (canonical string,
  HMAC-SHA256 over base64-decoded key, `Authorization`/`x-ms-date`/`Log-Type` headers).
- **C** `send`/`send_immediate`: retry loop honoring `max_retries`/`retry_delay` (exp backoff,
  respect `RateLimited`).
- **W** time-based batch flush (`batch_flush_interval` background task); gzip compression
  (`flate2`); `ca_cert_path` custom root; Elastic `cloud_id` decode; CEF distinct cs/cn slots.

### armature-auth (2C, 7W, 1I) — largest; SAML is the big piece
- **C** SAML `validate_response`: real signature verification (samael/xmlsec) + issuer/audience/
  `NotOnOrAfter` enforcement + `allow_unsigned_assertions`.
- **C** SAML `create_auth_request`: resolve the IdP SSO URL from `idp_metadata` (parse metadata),
  drop the hardcoded `idp.example.com`.
- **W** SAML config knobs (`allow_unsigned_assertions`, `required_attributes`, `sp_certificate`/
  `sp_private_key` signing), `extract_attributes` real parsing; Microsoft Entra Graph `id→sub`
  mapping; OIDC `id_token` extraction; API-key rate limiting; `AuthGuard::extract_user` (read the
  verified `UserContext`/`RequestRoles`); `LocalStrategy`/`JwtStrategy` `AuthStrategy` impls.
- **I** lib.rs SAML doc example signature fix.

## Success criteria

- All 10 Critical and 17 Warning findings implemented with regression tests that failed against the
  old code; the 8 Info reconciled (doc/test).
- `cargo test` for all five crates green; SAML/TLS/HMAC verified against fixtures/stubs; strict
  clippy + fmt clean.
- No security control silently degrades (no plaintext-for-TLS, no unverified-signature-accepted).

## Risks

- **SAML signature verification** is the hardest item (XML canonicalization + xmlsec via samael).
  If samael integration is blocked, escalate rather than shipping a weakened check — a SAML
  validator that doesn't verify signatures is worse than an explicit "unsupported" error.
- **Sentinel SharedKey** must match Azure's exact canonicalization; test against a stub asserting
  the computed `Authorization` header for a known key/date/body vector.
