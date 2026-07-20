# Workflow 4 — Certificates (ACME): Implementation Plan

**Spec:** `docs/superpowers/specs/2026-07-19-workflow4-acme-design.md`
**Findings:** `.superpowers/sdd/wf4-findings.md` (11: 8C/2W/1I)
**Branch:** `feature/wf4-acme` → PR to `develop` (HELD for user audit window)

## Execution model

One crate, one cohesive implementer (the RFC 8555 flow is sequential/coupled — not fragmentable). A strong implementer builds the whole flow + tests; an adversarial reviewer verifies the JWS/crypto/flow before the central gate. No parallel edit-only fan-out.

## Tasks

### T1 — JWS/crypto core (`account.rs` + a `jws.rs` helper if cleaner)
- ES256 account key (ring `EcdsaKeyPair`), PKCS#8 persistence in `account_dir` (0600).
- ACME flattened JWS: protected `{alg,nonce,url,jwk|kid}`, base64url payload (`""` for POST-as-GET), raw 64-byte `r||s` signature.
- JWK (`{crv,kty,x,y}`) + RFC 7638 thumbprint (SHA-256 of canonical JWK) → `key_authorization`.
- Nonce: `new_nonce` seed + `Replay-Nonce` capture + one `badNonce` retry.
- Unit tests: JWS sign/verify, thumbprint vector, key_authorization format.

### T2 — Account + custom CA (`client.rs::register_account`, `config.rs`, `AcmeClient::new`)
- `register_account`: JWS(jwk) newAccount with `termsOfServiceAgreed=accept_tos`, `contact=mailto:contact_email`; EAB (HS256 over JWK) when `eab_kid`/`eab_hmac_key` set; store account URL from `Location`; load-or-generate key; idempotent.
- `AcmeConfig::with_ca_certificate(pem)` (+ documented test-only accept-invalid) threaded into the rustls reqwest client so Pebble/private ACME works.

### T3 — Order + challenges (`client.rs`)
- `create_order`: JWS(kid) newOrder `{identifiers:[dns]}` → order URL.
- `get_challenges`: POST-as-GET order → authorizations → challenges; select `config.challenge_type`; compute key_authorization; return `Vec<Http01Challenge>` (+ DNS-01 value).
- `notify_challenge_ready`: JWS(kid) `{}` to challenge URL; poll authorization to valid/invalid (bounded backoff).

### T4 — Finalize + renew + save (`client.rs`)
- `finalize_order`: rcgen CSR (SANs = domains) → JWS(kid) `{csr}` to finalize → poll order valid → download cert (POST-as-GET) → `(pem_chain, key_pem)`.
- `order_certificate`: orchestrate; real error (never `("","")`) on failure; document/solve the HTTP-01 serving requirement.
- `save_certificate`: reject empty/non-PEM input.
- `should_renew`: parse leaf `notAfter` (add `x509-parser` if needed) vs `renew_before_days`.

### T5 — README + tests
- Rewrite README to the real API (lib.rs doctests are correct).
- Pebble e2e (`armature-testkit` dev-dep, `containers`, `docker_available()` gate): full issuance → non-empty parsing cert. Pure-unit tests per T1–T4.

## Verification (central)
1. `cargo fmt -p armature-acme` (+ workspace).
2. Strict `clippy --workspace --all-targets --features full-with-saml -- -D warnings -A collapsible_if -A result_large_err -A dead_code -A useless_vec -A unwrap_or_default`.
3. `cargo test -p armature-acme` (Docker → Pebble e2e runs; self-skips otherwise); check any consumers of armature-acme (armature-app `with_acme`? — verify).
4. `cargo audit`; MSRV `cargo +1.89 check -p armature-acme` (vet any new dep — x509-parser).
5. Semver: `armature-acme` → `0.2.0` if any public signature changed (save_certificate fallible, order_certificate solver, with_ca_certificate); update pins; CHANGELOG.
6. Tick the 11 `TODO.md` boxes; summary row → 0|0|0.

## Then
Adversarial review (crypto/JWS/flow) → fix → gate green → **PR to `develop` HELD** for the user's `/simplify` `/optimize` `/audit` `/code-review` window (do NOT auto-merge; branch protection now requires green CI, so use `gh pr merge --auto --squash` only on the user's go).
