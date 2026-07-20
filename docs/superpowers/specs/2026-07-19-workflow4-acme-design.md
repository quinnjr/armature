# Workflow 4 — Certificates (ACME)

**Date:** 2026-07-19
**Roadmap:** `docs/superpowers/specs/2026-07-18-conformance-completion-roadmap-design.md` (Workflow 4 of 9)
**Crate:** armature-acme
**Findings:** 8 Critical · 2 Warning · 1 Info (11 total; see `.superpowers/sdd/wf4-findings.md`)

## Problem

`armature-acme` advertises a full ACME (RFC 8555) certificate client — the flagship `AcmeClient::order_certificate()` is shown in the crate Quick Start and README — but the entire protocol flow is a **hollow shell**. Every method (`register_account`, `create_order`, `get_challenges`, `notify_challenge_ready`, `finalize_order`, `order_certificate`, `should_renew`) returns a placeholder: empty strings, empty vectors, or the raw directory URL. A caller following the documented Quick Start **silently writes empty `cert.pem`/`key.pem` files** and believes it obtained a certificate. Auto-renewal never fires. The advertised config knobs (`challenge_type`, `accept_tos`, EAB `eab_kid`/`eab_hmac_key`, `contact_email`, `account_dir`) are read nowhere.

The crate already has all the scaffolding: the wire types (`Directory`, `Account`/`AccountCreate`, `Order`/`Identifier`/`OrderCreate`, `Authorization`/`Challenge`, `Http01Challenge`), the config surface, and every crypto dependency needed (`ring` for ES256 signing, `rcgen` for CSR generation, `base64`, `rustls`, `reqwest`+rustls). Only the **protocol flow logic** is missing.

## Goal

Implement the real RFC 8555 flow so every advertised unit does what its name/docs/types claim, verified **end-to-end against a real ACME CA** (Pebble, via `armature-testkit::PebbleCa`, docker-gated) plus pure-unit tests for the crypto primitives. When done, `order_certificate()` obtains a genuine certificate (or returns a real error), `save_certificate` never writes empty PEM, `should_renew` honors `renew_before_days`, and the config knobs are consumed. Non-goals: DNS-01/TLS-ALPN-01 automation beyond exposing the challenge data the types already model (HTTP-01 is the primary path; DNS-01 key-authorization value is computed but serving is the caller's job); a built-in challenge web server; certificate revocation UX beyond what the directory already exposes.

## Approach

One crate, one cohesive implementation (the flow is inherently sequential and coupled — not parallelizable across files). One strong implementer builds it; one adversarial reviewer verifies the crypto/JWS/flow before the gate. Reuse the existing types and deps; do not restructure the module layout.

### RFC 8555 flow to implement (in `client.rs`, with helpers in `account.rs`/a new `jws.rs` if cleaner)

1. **Account key + JWS** (the core primitive):
   - Generate an **ECDSA P-256** account key via `ring::signature::EcdsaKeyPair` (ES256). Persist it as PKCS#8 in `config.account_dir/account_key.pem` (and the resolved account URL in `account_dir/account_url.txt` or a small JSON), created with `0700`/`0600` perms where possible. On `register_account`, **load an existing key if present**, else generate + persist.
   - Implement ACME **JWS** (flattened JSON: `{protected, payload, signature}`). Protected header: `{alg:"ES256", nonce, url, jwk|kid}` — `jwk` for `newAccount`/EAB, `kid` (account URL) for every subsequent request. Payload is base64url(JSON) (empty string `""` for POST-as-GET). ES256 signature is the raw 64-byte `r||s` (ring produces fixed-length ECDSA), base64url-encoded.
   - **Nonce management:** `HEAD`/`GET directory.new_nonce` for the first nonce; capture the `Replay-Nonce` response header after **every** signed POST and reuse it. On a `badNonce` error, refetch and retry once.
   - **JWK** = the P-256 public key as `{crv:"P-256", kty:"EC", x, y}` (base64url, un-padded, fixed 32-byte coords). **Thumbprint** (RFC 7638) = base64url(SHA-256 of the canonical JWK `{"crv":...,"kty":...,"x":...,"y":...}` with sorted keys, no whitespace).

2. **`register_account`** — POST-JWS(`jwk`) to `directory.new_account` with `{termsOfServiceAgreed: config.accept_tos, contact: config.contact_email mapped to "mailto:"}`. If `eab_kid`/`eab_hmac_key` set, include `externalAccountBinding` = a JWS(HS256, kid=eab_kid, key=base64url-decoded eab_hmac_key) over the account JWK. Store `account_url` from the `Location` header; persist. Idempotent (`onlyReturnExisting`-friendly): loading an existing key + re-POSTing returns the same account. **Fail closed** if `accept_tos` is false and the CA requires TOS.

3. **`create_order`** — POST-JWS(`kid`) to `directory.new_order` with `{identifiers:[{type:"dns", value:domain} for domain in config.domains]}`. Return the order URL from `Location`.

4. **`get_challenges(order_url)`** — POST-as-GET the order → `authorizations[]`; POST-as-GET each authorization → pick the challenge matching `config.challenge_type` (default HTTP-01). Compute `key_authorization = token + "." + base64url(jwk_thumbprint)`. Return `Vec<Http01Challenge{token, key_authorization, url}>` (and expose the DNS-01 value `base64url(sha256(key_authorization))` for DNS challenge type). Respect `config.challenge_type`.

5. **`notify_challenge_ready(challenge_url)`** — POST-JWS(`kid`) `{}` to the challenge URL, then **poll** the authorization (POST-as-GET) with bounded backoff until `valid` (Ok) or `invalid` (Err with the challenge `error`), up to a timeout.

6. **`finalize_order(order_url)`** — generate a certificate keypair + **CSR** with SAN entries for all `config.domains` via `rcgen` (`CertificateParams` + `serialize_request`), DER-encode; POST-JWS(`kid`) `{csr: base64url(der)}` to the order's `finalize` URL; poll the order (POST-as-GET) until `valid`; POST-as-GET the order's `certificate` URL to download the PEM chain. Return `(certificate_pem_chain, cert_private_key_pem)` — the key PEM is the rcgen keypair's private key.

7. **`order_certificate`** — orchestrate register → create_order → get_challenges → notify_challenge_ready (per challenge) → poll → finalize. Because HTTP-01 requires the caller to serve the token, `order_certificate` works end-to-end only when validation is satisfied out-of-band (as in the Pebble `PEBBLE_VA_ALWAYS_VALID` test) or the caller pre-serves; document this precisely, or accept an optional challenge-solver callback (implementer's choice — do NOT silently no-op). It must return a **real error** rather than `("", "")` when validation/finalize fails.

8. **`save_certificate`** — reject empty/whitespace or non-PEM cert/key input with an error (`AcmeError`), so an empty result can never be written as a valid cert.

9. **`should_renew(cert_path)`** — parse the leaf certificate's `notAfter` (via `rustls-pemfile` + an X.509 parse; if no X.509 parser dep exists, add a minimal one such as `x509-parser` behind the existing crypto stack, or parse the DER validity) and return `true` when `now + renew_before_days >= notAfter`. Missing file → `true`.

### Config wiring (Warning)
Consume `challenge_type` (challenge selection), `accept_tos` (newAccount TOS flag), `eab_kid`/`eab_hmac_key` (EAB signing), `contact_email` (mailto contacts), `account_dir` (key persistence). Add a way to trust a **custom CA root** (`AcmeConfig::with_ca_certificate(pem)` and/or a documented `danger_accept_invalid_certs` for test CAs) so the client can talk to Pebble/private ACME — thread it into the `reqwest`/rustls client in `AcmeClient::new`.

### Verification (via `armature-testkit::PebbleCa`, `docker_available()`-gated)
- **End-to-end (Pebble):** trust Pebble's `/roots/0` (or accept-invalid for the test), `AcmeClient::new(config with Pebble directory_url)` → `register_account` → `order_certificate` (Pebble auto-validates via `PEBBLE_VA_ALWAYS_VALID`) → assert a **non-empty PEM certificate chain that parses** and a matching private key; assert `save_certificate` writes real files. Also exercise the manual flow (`get_challenges` → `notify_challenge_ready` → `finalize_order`) once.
- **Pure-unit (no Docker):** ES256 JWS round-trip (sign → verify with ring); RFC 7638 JWK thumbprint against a known vector; `key_authorization` format; CSR contains the SANs; `should_renew` boundary (cert expiring in < / > `renew_before_days`); EAB HS256 MAC shape; `save_certificate` rejects empty PEM.
- Every implemented unit gets a regression test that fails against the current hollow code.

## Success criteria
- All 8 Critical + 2 Warning implemented; the 1 Info (tests) satisfied. `order_certificate` obtains a real Pebble certificate; `save_certificate` never writes empty PEM; `should_renew` honors `renew_before_days`; config knobs consumed; README rewritten to the real API.
- `cargo test -p armature-acme` green (Pebble e2e runs with Docker, self-skips otherwise); strict `clippy --features full-with-saml -D warnings`, `cargo audit`, MSRV 1.89 clean.
- No silent success: an ACME failure surfaces as `AcmeError`, never as empty PEM.

## Risks
- **Crypto correctness** (ES256 raw-signature encoding, JWK thumbprint canonicalization, nonce handling) is the highest-risk surface — covered by pure-unit vectors + the adversarial review before gate.
- **New dep for X.509 notAfter parsing** — prefer an existing/lightweight parser (`x509-parser`) added behind the crate's existing crypto stack; MSRV-check it.
- **Pebble/Docker-in-CI** — the e2e test is `containers`+`docker_available()`-gated; default `cargo test` stays Docker-free.
- **Breaking API** — any signature change (e.g. adding a solver to `order_certificate`, `with_ca_certificate`, `save_certificate` now fallible on empty) is a 0.1.x → 0.2.0 bump + CHANGELOG.
