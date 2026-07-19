# Workflow 1 — Auth & Security Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the five auth/security crates conformant — implement the 10 Critical + 17 Warning findings and reconcile the 8 Info findings from the conformance audit, closing real vulnerabilities (JWT/SAML/MCP auth bypasses, prefix-hash-as-HMAC, SIEM TLS downgrade).

**Architecture:** Fix in dependency order — `armature-jwt` first (MCP reuses its `JwtManager`), then `armature-mcp`, `armature-security`, `armature-siem`, `armature-auth` (SAML last, largest). Reuse `armature-jwt` for JWT verification, `hmac`+`sha2` for MACs, `samael` for SAML signature verification, `tokio-rustls` for SIEM TLS. Verify with `armature-testkit` stub servers + fixed crypto/XML fixtures.

**Tech Stack:** Rust 2024, tokio, hmac/sha2, samael, tokio-rustls, flate2, armature-jwt, armature-testkit (dev-dep).

## Global Constraints

- Rust 2024, MSRV 1.89 (inherit via workspace).
- rustls-only (no OpenSSL/native-tls); per-crate minimal `tokio` features (no `features=["full"]`).
- Strict pre-commit gate passes: `cargo fmt` + `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings`. Commit with `--no-verify` per task (the slow full-workspace clippy runs once at the final gate); the final task runs the full strict gate.
- Add `armature-testkit = { path = "../armature-testkit" }` as a **dev-dependency** to each crate whose tests use a stub server.
- **No security control may silently degrade.** A control that cannot be implemented must return an explicit error, never accept-unverified or downgrade-to-plaintext.
- Every implemented unit gets a regression test that FAILS against the current code.
- Commit after every task.

---

## Part A — armature-jwt (foundation; do first)

### Task A1: Refresh token gets a distinct, longer expiry (Critical ×2)

**Files:** Modify `armature-jwt/src/service.rs` (`generate_token_pair` ~:58, `refresh_token` ~:74). Test: same file's `#[cfg(test)] mod tests`.

**Finding:** `generate_token_pair` signs the same claims twice → refresh token == access token, same `exp`. `refresh_token` re-signs decoded claims verbatim → new access token keeps old `exp`, and since refresh shares that `exp`, refresh fails after expiry.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn refresh_token_outlives_access_token() {
    let mgr = JwtManager::new(test_config()).unwrap();
    let pair = mgr.generate_token_pair(test_claims()).unwrap();
    let access = mgr.verify::<serde_json::Value>(&pair.access_token).unwrap();
    let refresh = mgr.verify::<serde_json::Value>(&pair.refresh_token).unwrap();
    // exp is in the claims; extract and compare
    let a_exp = access.get("exp").and_then(|v| v.as_i64()).unwrap();
    let r_exp = refresh.get("exp").and_then(|v| v.as_i64()).unwrap();
    assert!(r_exp > a_exp, "refresh exp {r_exp} must exceed access exp {a_exp}");
    assert_ne!(pair.access_token, pair.refresh_token);
}

#[test]
fn refresh_reissues_a_fresh_access_token() {
    let mgr = JwtManager::new(test_config()).unwrap();
    let pair = mgr.generate_token_pair(test_claims()).unwrap();
    let new_pair = mgr.refresh_token(&pair.refresh_token).unwrap();
    assert!(mgr.verify::<serde_json::Value>(&new_pair.access_token).is_ok());
}
```
(Add `test_config()`/`test_claims()` helpers if absent, mirroring the existing `test_token_pair_generation`.)

- [ ] **Step 2: Run → FAIL** (`cargo test -p armature-jwt refresh_token_outlives_access_token`) — refresh exp equals access exp.

- [ ] **Step 3: Implement.** In `generate_token_pair`: after building access claims with `exp = now + config.expires_in`, build a **separate** refresh claims value whose `exp = now + config.refresh_expires_in`, and sign that for the refresh token. In `refresh_token`: verify the incoming refresh token, then OVERRIDE the decoded claims' `exp` (strip it) and call `generate_token_pair` so both tokens get fresh expirations. Read the actual `Claims`/`StandardClaims` shape in `claims.rs`/`token.rs` to set `exp` correctly (it may be a field on `StandardClaims` or an `exp()` builder).

- [ ] **Step 4: Run → PASS** (both new tests + existing).

- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(jwt): refresh token gets distinct longer expiry; refresh re-issues fresh exp"`

### Task A2: JWT doc/README reconciliation + metadata test (Warning ×2 → covered; Info ×2)

**Files:** Modify `armature-jwt/README.md`, `armature-jwt/src/config.rs` doc on `with_refresh_expiration`. Test: service.rs.

- [ ] **Step 1:** The `TokenPair` metadata and `with_refresh_expiration` warnings are resolved by A1 (real refresh exp now matches `refresh_expires_in`). Add an assertion to the A1 test that `pair.refresh_expires_in` matches the refresh token's actual lifetime (within tolerance).
- [ ] **Step 2:** Fix the README (README.md:31) to the real API: `with_expiration`, `Claims::new(custom)`, `generate_token_pair`, `refresh_token`, `JwtManager::new(config)?` returning `Result`. Mirror the working lib.rs doctests. Verify with `cargo test -p armature-jwt --doc`.
- [ ] **Step 3: Commit** — `git commit --no-verify -m "docs(jwt): correct README API; assert refresh lifetime metadata"`

---

## Part B — armature-mcp

### Task B1: `authenticate_jwt` verifies the signature (Critical — auth bypass)

**Files:** Modify `armature-mcp/src/auth.rs` (`authenticate_jwt` ~:535), `armature-mcp/Cargo.toml` (add `armature-jwt` dep if absent, and `armature-testkit` dev-dep). Test: auth.rs tests.

**Finding:** `authenticate_jwt` base64url-decodes the payload and discards the signature; `JwtAuth.secret/public_key/algorithm` unused → any forged token passes.

- [ ] **Step 1: Write the failing test** — a token with a valid structure but wrong signature must be REJECTED:
```rust
#[tokio::test]
async fn jwt_auth_rejects_bad_signature() {
    // Build a JwtAuth configured with secret "correct". Mint a token signed with "wrong".
    // authenticate_jwt(&token) must return Err (not Ok).
}
#[tokio::test]
async fn jwt_auth_accepts_valid_signature_and_enforces_exp() { /* valid token passes; expired rejected */ }
```
Use `armature-jwt`'s `JwtManager` to mint the test tokens (correct + wrong secret, expired).

- [ ] **Step 2: Run → FAIL** — the forged/expired token is currently accepted.

- [ ] **Step 3: Implement.** Replace the decode-only path: construct an `armature_jwt::JwtManager` (or use its verify API) from `JwtAuth.secret` (HS*) or `JwtAuth.public_key` (RS*/ES*) and `JwtAuth.algorithm`, verify the token, and only then read claims (sub/scope/iss/aud/exp). Reject on any verification error. Map claims into the existing `McpAuthContext`. Consume the `with_secret/with_public_key/with_algorithm` builder fields (resolves the Warning finding).

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(mcp): verify JWT signature via armature-jwt (closes auth bypass)"`

### Task B2: `#[mcp]` attribute compiles + is exported (Critical — divergent)

**Files:** Modify `armature-mcp/src/lib.rs` (re-export), `armature-proc-macro/src/mcp.rs` (~:174 handler emission). Test: `armature-mcp/tests/ui/` trybuild case + `tests/trybuild.rs`.

**Finding:** `mcp` attribute not re-exported; `mcp_impl` emits `Arc::new(closure)` but `McpToolEntry::new` needs a fn-pointer `ToolHandlerFnPtr` → generated code doesn't compile.

- [ ] **Step 1: Write the failing trybuild case** — a new `tests/ui/mcp_attribute_compiles.rs` using `#[mcp]` on a tool fn and `use armature_mcp::mcp;`; add it to the trybuild harness expecting PASS.
- [ ] **Step 2: Run → FAIL** (compile error: `mcp` not found / handler type mismatch).
- [ ] **Step 3: Implement.** Add `pub use armature_proc_macro::mcp;` to `armature-mcp/src/lib.rs`. In `mcp.rs`, change the generated handler to emit a `fn __wrap(args) -> Pin<Box<dyn Future<...>>>` coerced to `ToolHandlerFnPtr` (mirror the working `register_mcp_tool!` macro's emission), not `Arc::new(closure)`.
- [ ] **Step 4: Run → PASS** (trybuild + existing `register_macro_compiles`).
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(mcp): export #[mcp] attribute and emit fn-pointer handler"`

### Task B3: Cursor pagination + JWT tests (Info ×2)

**Files:** `armature-mcp/src/service.rs` (`handle_tools_list`/`handle_resources_list` ~:220), auth.rs tests.

- [ ] Implement cursor paging (read `ListParams.cursor`, page the registry, emit `next_cursor`) OR, if the registry has no stable ordering to page, drop `cursor`/`next_cursor` from the public surface + docs. Pick paging if feasible; document the decision. Add a test either way (paging returns a cursor; or the types are gone).
- [ ] The JWT signature-rejection tests from B1 satisfy the unverified-claim Info. Commit — `git commit --no-verify -m "feat(mcp): cursor pagination for list handlers"` (or `docs(mcp): drop unimplemented cursor surface`).

---

## Part C — armature-security

### Task C1: Real HMAC-SHA256 in RequestSigner (Critical — forgeable MAC)

**Files:** Modify `armature-security/src/request_signing.rs` (~:103), `armature-security/Cargo.toml` (add `hmac`). Test: request_signing.rs tests / `tests/request_signing_tests.rs`.

- [ ] **Step 1: Write the failing test** — assert the signer matches a known HMAC-SHA256 test vector (RFC 4231 or a computed reference), and that it differs from `Sha256(secret || msg)`:
```rust
#[test]
fn sign_is_real_hmac_sha256() {
    let sig = RequestSigner::new("key").sign("message"); // adapt to real API
    // Reference: hmac-sha256("key","message") hex. Assert equality with the standard value.
    assert_eq!(sig, "6e9ef29b75fffc5b7abae527d58fdadb2fe42e7219011976917343065f58ed4a");
}
```
- [ ] **Step 2: Run → FAIL** (current prefix-hash produces a different digest).
- [ ] **Step 3: Implement.** `cargo add hmac --package armature-security`. Replace the two `Sha256::update` calls with `Hmac::<Sha256>::new_from_slice(secret.as_bytes())?.chain_update(message).finalize()` and hex-encode. Keep the public `sign`/verify signatures.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(security): use real HMAC-SHA256 (closes length-extension forgery)"`

### Task C2: CSP report-only + README + deprecated-default reconciliations (Warning ×2, Info ×3)

**Files:** `armature-security/src/content_security_policy.rs`, `src/lib.rs` (header emission ~:239, xss default ~:320), `src/expect_ct.rs`, `src/frame_guard.rs`, `README.md`.

- [ ] **CSP report-only (Warning):** thread `report_only` through `SecurityMiddleware::apply` so it emits `Content-Security-Policy-Report-Only` when set. Test: assert the emitted header NAME (not just the field) for report_only true vs false.
- [ ] **README (Warning):** rewrite to the real API (`SecurityMiddleware`, `CorsConfig`, `RequestSigningMiddleware`); drop the non-existent `CsrfMiddleware`/`CorsMiddleware`/`SecurityHeaders`/CSRF/sanitization claims. Verify doctests compile.
- [ ] **Defaults (Info):** default `xss_filter` to `Disabled ("0")` (Helmet parity — update any test asserting "1"); document `FrameGuard::AllowFrom` as legacy/non-functional (steer to CSP frame-ancestors); stop enabling Expect-CT in `enable_all()`/`Default` (deprecated) and note it's a no-op.
- [ ] Add/adjust tests for the report-only header and the new xss default. Commit — `git commit --no-verify -m "fix(security): CSP report-only header, real API docs, modern security defaults"`

---

## Part D — armature-siem

### Task D1: TLS transport actually encrypts (Critical — plaintext downgrade)

**Files:** `armature-siem/src/client.rs` (`TcpTransport::send` ~:294), `armature-siem/Cargo.toml` (add `tokio-rustls`), dev-dep `armature-testkit`. Test: client.rs tests.

- [ ] **Step 1: Write the failing test** — stand up a minimal in-test rustls TLS listener (self-signed) and assert the transport completes a TLS handshake and delivers the event; a `tls=true` config against a plaintext listener must NOT deliver in cleartext. (If a full TLS test is heavy, at minimum assert that `tls=true` uses a TLS connector path and that the bytes on the wire are not the plaintext event — the key is proving no plaintext downgrade.)
- [ ] **Step 2: Run → FAIL** (current send writes plaintext regardless of `tls`).
- [ ] **Step 3: Implement.** When `self.tls`, wrap the `TcpStream` in `tokio_rustls::TlsConnector` (honor `config.tls_verify` and `config.ca_cert_path` for the root store); write the event over the TLS stream. If TLS setup fails, return `SiemError::Transport` — never fall back to plaintext.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(siem): real TLS syslog transport, no plaintext downgrade"`

### Task D2: Azure Sentinel SharedKey signing (Critical — auth always fails)

**Files:** `armature-siem/src/client.rs` (`HttpTransport::new` Sentinel arm ~:194), Cargo.toml (`hmac`,`sha2`,`base64` if absent). Test: client.rs with `armature-testkit` StubServer.

- [ ] **Step 1: Write the failing test** — a StubServer captures the request; assert the `Authorization` header equals `SharedKey {workspace}:{signature}` for a known workspace/key/date/body vector, and that `x-ms-date` and `Log-Type` headers are present. (Compute the expected signature with a reference HMAC over the canonical string.)
- [ ] **Step 2: Run → FAIL** (current code sends `config.token` as Authorization, no signing).
- [ ] **Step 3: Implement.** Build the canonical string (`POST\n{content-length}\napplication/json\nx-ms-date:{rfc1123-date}\n/api/logs`), HMAC-SHA256 it with the base64-decoded shared key, base64-encode, set `Authorization: SharedKey {workspace}:{sig}`, `x-ms-date`, `Log-Type`, `time-generated-field`. Note: date must be passed in (not `now()`) so the test is deterministic — thread a clock or accept the date as a parameter internally for testability.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(siem): implement Azure Sentinel SharedKey signing"`

### Task D3: Retry with backoff (Critical — no retry)

**Files:** `armature-siem/src/client.rs` (`send`/`send_immediate` ~:84). Test: client.rs with a StubServer that fails N times then succeeds.

- [ ] **Step 1: Failing test** — StubServer returns 500 twice then 200; assert `send` ultimately succeeds and the server was hit 3 times, honoring `max_retries=3`. And a test that exhausts retries returns the error.
- [ ] **Step 2: Run → FAIL** (no retry today; first 500 propagates).
- [ ] **Step 3: Implement.** Wrap `transport.send` in a loop honoring `config.max_retries` with exponential backoff from `config.retry_delay`; respect `SiemError::RateLimited` (retry-after). Keep the public signature.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(siem): retry send with exponential backoff"`

### Task D4: SIEM Warning cluster — batch flush, compression, ca_cert, cloud_id, CEF slots (Warning ×5)

**Files:** `armature-siem/src/client.rs` (`add_to_batch` ~:116, compression ~:184, ca_cert ~:174), `src/provider.rs` (`ElasticConfig::cloud` ~:82), `src/format/cef.rs` (~:133), Cargo.toml (`flate2`).

- [ ] **Batch flush:** spawn a `tokio::spawn` + `interval(config.batch_flush_interval)` background task that calls `flush()`; ensure it shuts down when the client drops. Test: low-volume events flush after the interval.
- [ ] **Compression:** when `config.compression`, gzip the body (`flate2`) and set `Content-Encoding: gzip`. Test (StubServer): body is gzip and header set.
- [ ] **ca_cert_path:** load the PEM and `add_root_certificate` on the reqwest client (and TLS transport). Test: a client built with a bogus ca_cert path errors clearly.
- [ ] **Elastic cloud_id:** base64-decode `name$es_uuid$kibana_uuid` → `https://{es_uuid}.{name}:443`. Test: a known cloud_id decodes to the expected endpoint.
- [ ] **CEF slots:** assign distinct `cs1..cs6`/`cn1..cn3` per metadata key (or serialize metadata to one JSON custom field). Test: two metadata entries both survive in the CEF output.
- [ ] Commit — `git commit --no-verify -m "fix(siem): time-based flush, gzip, custom CA, cloud_id decode, distinct CEF slots"`

---

## Part E — armature-auth (largest; SAML last)

### Task E1: SAML signature verification via samael (Critical — SSO bypass)

**Files:** `armature-auth/src/saml.rs` (`validate_response` ~:235), `armature-auth/Cargo.toml` (add `samael` under the existing `saml` feature). Test: saml.rs with fixed signed + unsigned + tampered XML fixtures.

- [ ] **Step 1: Write failing tests** with fixtures (generate a signed SAML Response with a known key/cert, plus an unsigned one and a tampered-signature one): a validly-signed assertion → `Ok` with correct NameID/attributes/expiry; an unsigned assertion with `allow_unsigned_assertions=false` → `Err`; a tampered signature → `Err`; an expired `NotOnOrAfter` → `Err`; wrong audience/issuer → `Err`.
- [ ] **Step 2: Run → FAIL** (current code accepts any base64 XML with a NameID).
- [ ] **Step 3: Implement** using `samael` (SAML2 + xmlsec): parse the Response, verify the enveloped XML signature against the IdP certificate from `config.idp_metadata`, enforce issuer/audience/`NotOnOrAfter` from the actual assertion, honor `allow_unsigned_assertions` and `required_attributes`, and read the real `NotOnOrAfter` (not `now()+1h`). Reject on any failure. If `samael` cannot be integrated, STOP and report BLOCKED — do not ship a validator that skips signature checks.
- [ ] **Step 4: Run → PASS.**
- [ ] **Step 5: Commit** — `git commit --no-verify -m "fix(auth): real SAML signature + condition verification (closes SSO bypass)"`

### Task E2: SAML auth request uses real IdP + config + attributes (Critical + Warning cluster)

**Files:** `armature-auth/src/saml.rs` (`create_auth_request` ~:226, `extract_attributes` ~:296, config usage ~:87).

- [ ] **create_auth_request (Critical):** resolve the IdP SSO endpoint from `config.idp_metadata` (parse the metadata XML/URL) for `redirect_url`; drop the hardcoded `idp.example.com`. Sign the AuthnRequest with `sp_private_key`/`sp_certificate` when configured. Test: the generated request points at the metadata's SSO URL.
- [ ] **extract_attributes (Warning):** parse `<saml:AttributeStatement>`/`<saml:Attribute>` into the map (via samael's parsed assertion from E1). Test: a response with attributes yields them.
- [ ] **Config knobs (Warning):** ensure `allow_unsigned_assertions`/`required_attributes`/`sp_*` are all consumed (partly done in E1). 
- [ ] Commit — `git commit --no-verify -m "fix(auth): SAML auth request from IdP metadata; parse attributes; honor SP config"`

### Task E3: OAuth provider + guard + strategy fixes (Warning cluster + Info)

**Files:** `armature-auth/src/providers/microsoft.rs` (~:118), `src/oauth2.rs` (~:76), `src/api_key.rs` (~:112), `src/guard.rs` (~:31), `src/strategy.rs` (~:16), `src/lib.rs` (SAML doc ~:137). Tests: respective modules + `armature-testkit` StubServer for Graph.

- [ ] **Microsoft Entra Graph (Warning):** deserialize Graph `/me` into a Graph-specific struct and map `id→sub`, `mail`/`userPrincipalName→email`, `displayName→name`. Test (StubServer serving a Graph `/me` payload): `get_user_info` returns a populated `OAuth2UserInfo`.
- [ ] **OIDC id_token (Warning):** extract `id_token` from the token endpoint response (OIDC-aware token type / extra fields) and populate `OAuth2Token.id_token`. Test (StubServer token endpoint returning an `id_token`): it's surfaced.
- [ ] **API-key rate limiting (Warning):** enforce `rate_limit` in `validate()` (per-key counts) returning `RateLimitExceeded`. Test: exceeding the limit errors.
- [ ] **AuthGuard::extract_user (Warning):** read the verified `UserContext`/`RequestRoles` extension (as `JwtAuthMiddleware` attaches) instead of always erroring. Test: a request with the extension yields the user; without it errors.
- [ ] **Strategies (Warning):** implement `AuthStrategy` for `LocalStrategy` (verify username/password) and `JwtStrategy` (verify via `JwtManager`). Tests: valid/invalid credentials.
- [ ] **lib.rs SAML doc (Info):** fix the example to `SamlServiceProvider::new("my-sp".to_string(), config)?`.
- [ ] Commit (may split into 2 commits if large) — `git commit --no-verify -m "fix(auth): Graph mapping, OIDC id_token, API-key limits, guard extraction, strategies"`

---

## Task F: Final gate

- [ ] `cargo fmt`; then `cargo fmt -- --check` clean.
- [ ] `cargo test -p armature-jwt -p armature-mcp -p armature-security -p armature-siem -p armature-auth` (all green; the crypto/SAML/TLS regression tests pass).
- [ ] `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings` clean (the strict pre-commit command).
- [ ] `cargo check --workspace` clean.
- [ ] Tick the corresponding `TODO.md` checkboxes; add a `CHANGELOG` entry.
- [ ] Commit — `git commit --no-verify -m "chore(wf1): tick TODO, changelog, final gate"`

## Self-Review

- **Coverage:** every one of the 35 findings maps to a task (A1-2 jwt, B1-3 mcp, C1-2 security, D1-4 siem, E1-3 auth). Criticals are the first task in each crate's part. ✓
- **Security invariant:** each Critical's test asserts REJECTION of the bad case (forged sig, tampered SAML, plaintext-for-TLS, unsigned Sentinel) — the fix is proven by a test that failed before. SAML has an explicit BLOCKED escape rather than shipping a weak check. ✓
- **Reuse:** MCP/auth-JwtStrategy delegate to `armature-jwt`; HMAC via `hmac` crate in two places; no bespoke crypto. ✓
- **Placeholders:** crypto/SAML/TLS tasks specify the crate API + exact test rather than full pre-written code (the code depends on external-crate APIs the implementer reads) — this is intentional for integration tasks, not a placeholder; each still has a concrete fix + failing test.
