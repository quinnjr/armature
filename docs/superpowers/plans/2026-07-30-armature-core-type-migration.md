# `armature-core` Type Migration Implementation Plan (Plan 2 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate `armature-core`'s request/response types onto the `Bytes`-backed
types `armature-h1` already provides — B1, B2, B7, B8 of the design spec — so a
request carries slices of a pooled buffer instead of six freshly allocated
collections, without breaking the public API's shape.

**Architecture:** `armature-core` takes a dependency on `armature-h1` and adopts
its `Method`, `ByteStr`, and `HeaderId` as the canonical types. `HttpRequest`'s
`String`/`Vec<u8>`/`HashMap` fields become `Method`/`ByteStr`/`Bytes`/`SmallVec`,
with the constructors kept generic (`impl Into<…>`) so existing call sites compile
unchanged, and `method_str()`/`body_slice()`/`get_bytes()` covering code that
genuinely needs the old representation. Query parsing becomes lazy and memoized;
path params become spans against the target buffer; the router becomes an array of
`matchit` trees indexed by method. No serve-path change and no `Send`-bound change
happens here — those are Plans 3 and 4.

**Tech Stack:** Rust 2024, `armature-h1` (this workspace), `bytes` 1.12,
`smallvec` 1.15, `matchit` 0.9 (already a declared dependency of `armature-core`,
currently unused), `criterion` 0.8.

## Global Constraints

- MSRV `1.94.1`; edition 2024. Do not raise either.
- `armature-core` version `0.5.0` → `0.6.0`. Semver-major, deliberately.
- The gate for every task, run from the repo root, is the real CI gate — strict,
  whole-workspace, no `-A` escapes:
  ```bash
  cargo fmt --all -- --check
  cargo clippy --all-targets --features full -- -D warnings
  cargo test --features full
  ```
  Plus, for tasks touching `armature-h1`:
  ```bash
  cargo test -p armature-h1 --all-features
  cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings
  ```
  The local default toolchain is nightly; pin stable for the gate with
  `rustup override set stable` in the worktree if it is not already pinned.
- `armature-h1` keeps `#![forbid(unsafe_code)]`. `armature-core` does not forbid
  unsafe, but no task in this plan introduces any.
- **`Send`/`Sync` bounds do not change in this plan.** `HttpRequest` and
  `HttpResponse` must remain `Send + Sync`, and `Extensions` keeps
  `Arc<dyn Any + Send + Sync>`. B5 drops those bounds and is Plan 3. Any use of
  `Cell`/`RefCell`/`Rc`/`OnceCell` in a type reachable from `HttpRequest` breaks
  extractors, which hold `&HttpRequest` across an `await` in a `Send` future —
  `&T: Send` requires `T: Sync`. Use `OnceLock` where memoization is needed and
  leave a comment pointing at Plan 3.
- Changelog entries go in each crate's own `CHANGELOG.md`, never the root one. The
  root `CHANGELOG.md` gets only the cross-crate migration note in Task 10.
- Route pattern syntax visible to users does not change: `:id` and `*rest` stay.
  Translation to `matchit`'s `{id}`/`{*rest}` happens inside the router.
- Every task ends green and lands as its own commit. Conventional Commits are
  enforced by a commit hook: `feat|fix|docs|style|refactor|perf|test|build|ci|chore`.

## Prerequisites

This plan builds on Plan 1 (`armature-h1`), which is **not yet merged** — it is
open as PR #276 against `develop`. Work on a branch off `feat/armature-h1`:

```bash
git worktree add -b worktree-feat+armature-core-types \
  .claude/worktrees/feat+armature-core-types worktree-feat+armature-h1
```

(Already created if you are reading this file inside that worktree.)

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `armature-h1/src/method.rs` | gains `From<&str>`/`From<String>` for `Method`, and `Method::as_str` coverage for `Other` | 1 |
| `armature-h1/src/bytestr.rs` | gains `From<&str>`/`From<String>`/`From<Bytes>` and `into_owned` | 1 |
| `armature-h1/src/header.rs` | gains `header::intern(&str) -> HeaderId` | 1 |
| `armature-core/Cargo.toml` | adds the `armature-h1` path dependency; version 0.6.0 | 1, 10 |
| `armature-core/src/lib.rs` | re-exports `ByteStr`, `HeaderId`, `Method` | 1 |
| `armature-core/src/traits.rs` | `HttpMethod` ↔ `Method` conversions | 1 |
| `armature-core/src/headers.rs` | `HeaderMap` stores `(HeaderId, Bytes)`; `&str` facade; `HeaderValueInput` trait | 2 |
| `armature-core/src/http.rs` | `HttpRequest.method`/`path`/`body`, `HttpResponse.body`, `Query`, path-param spans | 3, 4, 5, 6 |
| `armature-core/src/query.rs` | **new** — `Query<'a>` lazy view and the percent-decoding parser | 5 |
| `armature-core/src/param_intern.rs` | **new** — leak-once interner producing `&'static str` param names | 6 |
| `armature-core/src/extensions.rs` | `SmallVec<[(TypeId, Arc<dyn Any + Send + Sync>); 8]>` | 7 |
| `armature-core/src/routing.rs` | method-indexed `matchit` trees, pattern translation, linear fallback | 8 |
| `armature-core/src/application.rs` | request construction without eager query parsing | 9 |
| `armature-core/src/simd_parser.rs` | `parse_query_string_decoded` retained for compat, marked non-hot | 5 |
| `armature-core/tests/alloc_core.rs` | **new** — counting-allocator assertions for the migrated types | 11 |
| `armature-core/benches/router.rs` | **new** — method-indexed dispatch vs the old linear scan | 11 |

---

### Task 1: Shared types — `armature-core` depends on `armature-h1`

Everything downstream needs `Method`, `ByteStr`, and `HeaderId` to exist in
`armature-core`'s namespace with `From` impls that make `impl Into<…>`
constructors work. The `From` impls must live in `armature-h1` because of the
orphan rule — `Method` and `ByteStr` are defined there.

**Files:**
- Modify: `armature-h1/src/method.rs`
- Modify: `armature-h1/src/bytestr.rs`
- Modify: `armature-h1/src/header.rs`
- Modify: `armature-core/Cargo.toml`
- Modify: `armature-core/src/lib.rs`
- Modify: `armature-core/src/traits.rs:100-140`
- Test: inline `#[cfg(test)]` modules in the three `armature-h1` files, and
  `armature-core/src/traits.rs`

**Interfaces:**
- Produces:
  - `impl From<&str> for Method`, `impl From<String> for Method` — unknown tokens
    become `Method::Other(ByteStr)`, preserving case (methods are case-sensitive,
    RFC 9110 §9.1).
  - `impl From<&str> for ByteStr` (copies), `impl From<String> for ByteStr`
    (takes the allocation), `impl From<Bytes> for ByteStr` — **fallible input,
    infallible impl**: non-UTF-8 `Bytes` becomes `ByteStr::default()` rather than
    panicking. `ByteStr::from_utf8` stays the checked constructor.
  - `ByteStr::into_owned(&self) -> String`
  - `armature_h1::header::intern(name: &str) -> HeaderId`
  - `impl From<HttpMethod> for Method`, `impl TryFrom<&Method> for HttpMethod`
    in `armature-core`.

- [ ] **Step 1: Write the failing tests in `armature-h1`**

Append to `armature-h1/src/method.rs`'s test module:

```rust
    #[test]
    fn from_str_maps_well_known_and_preserves_unknown_case() {
        assert_eq!(Method::from("GET"), Method::Get);
        assert_eq!(Method::from("QUERY"), Method::Query);
        // Methods are case-sensitive (RFC 9110 section 9.1): a lowercase token is
        // not GET, it is a different method token entirely.
        assert_eq!(
            Method::from("get"),
            Method::Other(ByteStr::from_static("get"))
        );
        assert_eq!(
            Method::from("PURGE".to_string()),
            Method::Other(ByteStr::from_static("PURGE"))
        );
    }
```

Append to `armature-h1/src/bytestr.rs`'s test module:

```rust
    #[test]
    fn from_string_takes_the_allocation_and_from_str_copies() {
        let owned = String::from("/a/b");
        let s = ByteStr::from(owned);
        assert_eq!(s.as_str(), "/a/b");

        let borrowed = ByteStr::from("/c");
        assert_eq!(borrowed.as_str(), "/c");
        assert_eq!(borrowed.into_owned(), "/c".to_string());
    }

    #[test]
    fn from_non_utf8_bytes_is_empty_rather_than_panicking() {
        // Infallible by signature, so invalid input has to go somewhere. Empty is
        // the only answer that cannot be mistaken for real data; callers that need
        // to know use `from_utf8`.
        let s = ByteStr::from(Bytes::from_static(&[0xff, 0xfe]));
        assert!(s.is_empty());
        assert!(ByteStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).is_err());
    }
```

Append to `armature-h1/src/header.rs`'s test module:

```rust
    #[test]
    fn intern_prefers_well_known_and_lowercases_the_rest() {
        assert_eq!(intern("Content-Length"), HeaderId::ContentLength);
        assert_eq!(intern("content-length"), HeaderId::ContentLength);
        // A custom name is lowercased once here so every later comparison is a
        // plain byte compare rather than a case-insensitive one.
        assert_eq!(
            intern("X-Tenant-Id"),
            HeaderId::Other(ByteStr::from_static("x-tenant-id"))
        );
        // Already lowercase: no allocation beyond the copy into Bytes.
        assert_eq!(
            intern("x-req-id"),
            HeaderId::Other(ByteStr::from_static("x-req-id"))
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armature-h1 --all-features`
Expected: FAIL — `Method::from`, `ByteStr::from`, `intern` do not exist.

- [ ] **Step 3: Implement in `armature-h1`**

In `armature-h1/src/method.rs`, after `impl Method`:

```rust
impl From<&str> for Method {
    /// Parse a method token, falling back to [`Method::Other`].
    ///
    /// Infallible on purpose: this exists so `armature-core`'s constructors can
    /// take `impl Into<Method>` and keep every existing `HttpRequest::new("GET")`
    /// call site compiling. An invalid token is not rejected here — it is carried
    /// as `Other` and rejected by routing, which is where a 405 belongs.
    #[inline]
    fn from(token: &str) -> Self {
        Method::from_bytes(token.as_bytes())
            .unwrap_or_else(|| Method::Other(ByteStr::from(token)))
    }
}

impl From<String> for Method {
    #[inline]
    fn from(token: String) -> Self {
        Method::from_bytes(token.as_bytes())
            .unwrap_or_else(|| Method::Other(ByteStr::from(token)))
    }
}
```

In `armature-h1/src/bytestr.rs`, after the existing impls:

```rust
impl From<&str> for ByteStr {
    /// Copies. A borrowed `&str` has no `Bytes` to share.
    #[inline]
    fn from(s: &str) -> Self {
        Self(Bytes::copy_from_slice(s.as_bytes()))
    }
}

impl From<String> for ByteStr {
    /// Takes the existing allocation rather than copying it.
    #[inline]
    fn from(s: String) -> Self {
        Self(Bytes::from(s.into_bytes()))
    }
}

impl From<Bytes> for ByteStr {
    /// Non-UTF-8 input yields an empty string.
    ///
    /// The `From` contract is infallible, and the alternatives are worse: a panic
    /// puts a remote client in control of process liveness, and an unchecked
    /// conversion would break `#![forbid(unsafe_code)]`. Use
    /// [`ByteStr::from_utf8`] when the distinction matters.
    #[inline]
    fn from(bytes: Bytes) -> Self {
        Self::from_utf8(bytes).unwrap_or_default()
    }
}

impl ByteStr {
    /// Copy the contents into an owned `String`.
    ///
    /// The escape hatch for handlers that retain request data past the response:
    /// holding a `ByteStr` pins the whole pooled buffer it slices (see
    /// `armature-h1`'s README on buffer pinning), and this breaks that link.
    #[inline]
    pub fn into_owned(&self) -> String {
        self.as_str().to_owned()
    }
}
```

In `armature-h1/src/header.rs`, as a free function beside `get`/`all`/`count`:

```rust
/// Intern a field name: a well-known variant, or a lowercased `Other`.
///
/// Lowercasing here means every later comparison is a plain byte compare instead
/// of a case-insensitive one. It allocates only for a name that is not
/// well-known, which is the uncommon case.
#[inline]
pub fn intern(name: &str) -> HeaderId {
    if let Some(id) = HeaderId::from_bytes(name.as_bytes()) {
        return id;
    }
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        return HeaderId::Other(ByteStr::from(name.to_ascii_lowercase()));
    }
    HeaderId::Other(ByteStr::from(name))
}
```

- [ ] **Step 4: Run the `armature-h1` tests to verify they pass**

Run: `cargo test -p armature-h1 --all-features`
Expected: PASS.

- [ ] **Step 5: Write the failing conversion test in `armature-core`**

Append to `armature-core/src/traits.rs`'s test module (create one if absent):

```rust
#[cfg(test)]
mod method_conversion_tests {
    use super::HttpMethod;
    use crate::Method;

    #[test]
    fn every_http_method_round_trips_through_method() {
        for m in [
            HttpMethod::GET,
            HttpMethod::POST,
            HttpMethod::PUT,
            HttpMethod::DELETE,
            HttpMethod::PATCH,
            HttpMethod::HEAD,
            HttpMethod::OPTIONS,
            HttpMethod::QUERY,
        ] {
            let converted = Method::from(m.clone());
            assert_eq!(
                HttpMethod::try_from(&converted).ok(),
                Some(m.clone()),
                "{m:?} did not round-trip"
            );
        }
    }

    #[test]
    fn methods_with_no_http_method_counterpart_fail_conversion() {
        // CONNECT, TRACE, and Other exist in armature-h1 because the wire has
        // them; HttpMethod is the *routable* set, which is deliberately smaller.
        // A router that silently mapped CONNECT onto GET would be a security bug.
        assert!(HttpMethod::try_from(&Method::Connect).is_err());
        assert!(HttpMethod::try_from(&Method::Trace).is_err());
        assert!(HttpMethod::try_from(&Method::from("PURGE")).is_err());
    }
}
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test -p armature-core --features full method_conversion`
Expected: FAIL — `armature-h1` is not a dependency and `crate::Method` does not resolve.

- [ ] **Step 7: Add the dependency, the re-exports, and the conversions**

In `armature-core/Cargo.toml`, in `[dependencies]`:

```toml
# The HTTP/1.1 layer. armature-core adopts its Bytes-backed types (Method,
# ByteStr, HeaderId) as canonical rather than defining a parallel set; the
# serve-path swap onto it is Plan 4, not this one.
armature-h1 = { path = "../armature-h1", version = "0.1" }
```

In `armature-core/src/lib.rs`, beside the other re-exports:

```rust
/// The `Bytes`-backed string, method, and header-name types, re-exported from
/// `armature-h1` so downstream crates need not depend on it directly.
pub use armature_h1::{ByteStr, HeaderId, Method};
pub use armature_h1::header as header_id;
```

In `armature-core/src/traits.rs`, after `impl HttpMethod`:

```rust
impl From<HttpMethod> for crate::Method {
    #[inline]
    fn from(m: HttpMethod) -> Self {
        match m {
            HttpMethod::GET => crate::Method::Get,
            HttpMethod::POST => crate::Method::Post,
            HttpMethod::PUT => crate::Method::Put,
            HttpMethod::DELETE => crate::Method::Delete,
            HttpMethod::PATCH => crate::Method::Patch,
            HttpMethod::HEAD => crate::Method::Head,
            HttpMethod::OPTIONS => crate::Method::Options,
            HttpMethod::QUERY => crate::Method::Query,
            // `HttpMethod` is #[non_exhaustive]; a variant added later without a
            // mapping here must not silently become GET.
            other => crate::Method::Other(crate::ByteStr::from(other.as_str())),
        }
    }
}

/// The method is not routable through `HttpMethod`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnroutableMethod(pub String);

impl std::fmt::Display for UnroutableMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "method `{}` has no HttpMethod counterpart", self.0)
    }
}

impl std::error::Error for UnroutableMethod {}

impl TryFrom<&crate::Method> for HttpMethod {
    type Error = UnroutableMethod;

    #[inline]
    fn try_from(m: &crate::Method) -> Result<Self, Self::Error> {
        match m {
            crate::Method::Get => Ok(HttpMethod::GET),
            crate::Method::Post => Ok(HttpMethod::POST),
            crate::Method::Put => Ok(HttpMethod::PUT),
            crate::Method::Delete => Ok(HttpMethod::DELETE),
            crate::Method::Patch => Ok(HttpMethod::PATCH),
            crate::Method::Head => Ok(HttpMethod::HEAD),
            crate::Method::Options => Ok(HttpMethod::OPTIONS),
            crate::Method::Query => Ok(HttpMethod::QUERY),
            crate::Method::Connect => Err(UnroutableMethod("CONNECT".into())),
            crate::Method::Trace => Err(UnroutableMethod("TRACE".into())),
            crate::Method::Other(t) => Err(UnroutableMethod(t.into_owned())),
        }
    }
}
```

- [ ] **Step 8: Run the test to verify it passes**

Run: `cargo test -p armature-core --features full method_conversion`
Expected: PASS.

- [ ] **Step 9: Run the full gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
cargo test -p armature-h1 --all-features
```
Expected: all green. Nothing else changed yet, so this is a cheap confirmation
that adding the dependency did not create a cycle or a feature-unification
surprise.

- [ ] **Step 10: Commit**

```bash
git add armature-h1/src/method.rs armature-h1/src/bytestr.rs armature-h1/src/header.rs \
        armature-core/Cargo.toml armature-core/src/lib.rs armature-core/src/traits.rs
git commit -m "feat(core): adopt armature-h1's Method, ByteStr, and HeaderId"
```

---

### Task 2: `HeaderMap` stores `(HeaderId, Bytes)` behind a `&str` facade

**Files:**
- Modify: `armature-core/src/headers.rs` (whole file; `Header` struct at :40-60,
  `HeaderMap` at :85-300)
- Test: `armature-core/src/headers.rs` test module

**Interfaces:**
- Consumes: `HeaderId`, `ByteStr`, `header_id::intern` from Task 1.
- Produces:
  - `HeaderMap::get(&self, name: &str) -> Option<&str>` (was `Option<&String>`)
  - `HeaderMap::get_bytes(&self, name: &str) -> Option<&Bytes>` — new; the only
    accessor that sees a non-UTF-8 value
  - `HeaderMap::get_id(&self, id: &HeaderId) -> Option<&Bytes>` — new; no
    interning cost on the hot path
  - `HeaderMap::insert(&mut self, name: impl AsRef<str>, value: impl HeaderValueInput) -> Option<Bytes>`
  - `HeaderMap::iter() -> impl Iterator<Item = (&str, &str)>`
  - `HeaderMap::remove(&mut self, name: &str) -> Option<Bytes>`
  - `pub trait HeaderValueInput { fn into_value(self) -> Bytes; }` implemented for
    `&str`, `String`, `&'static str` via specialization-free overlap avoidance
    (see the implementation note), `Bytes`, `Vec<u8>`, `&[u8]`.

- [ ] **Step 1: Write the failing tests**

In `armature-core/src/headers.rs`'s test module:

```rust
    #[test]
    fn get_returns_str_and_well_known_names_are_interned() {
        let mut h = HeaderMap::new();
        h.insert("Content-Type", "application/json");
        h.insert("X-Tenant-Id", "acme".to_string());

        // Case-insensitive lookup survives the move to HeaderId.
        assert_eq!(h.get("content-type"), Some("application/json"));
        assert_eq!(h.get("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(h.get("x-tenant-id"), Some("acme"));
        assert_eq!(h.get("absent"), None);

        // Well-known names cost no allocation and compare as an integer.
        assert_eq!(h.get_id(&HeaderId::ContentType).map(|b| &b[..]), Some(&b"application/json"[..]));
    }

    #[test]
    fn non_utf8_value_is_invisible_to_get_but_reachable_as_bytes() {
        let mut h = HeaderMap::new();
        h.insert("x-raw", bytes::Bytes::from_static(&[0xff, 0x00]));
        // `get` promises a `&str`, and there isn't one. Returning None beats
        // returning lossy text that a caller would go on to trust.
        assert_eq!(h.get("x-raw"), None);
        assert_eq!(h.get_bytes("x-raw").map(|b| b.len()), Some(2));
    }

    #[test]
    fn insert_replaces_and_returns_the_previous_value() {
        let mut h = HeaderMap::new();
        assert_eq!(h.insert("host", "a.example"), None);
        assert_eq!(h.insert("Host", "b.example").as_deref(), Some(&b"a.example"[..]));
        assert_eq!(h.get("host"), Some("b.example"));
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn append_keeps_both_values_and_get_all_sees_them() {
        let mut h = HeaderMap::new();
        h.append("set-cookie", "a=1");
        h.append("Set-Cookie", "b=2");
        assert_eq!(h.get_all("set-cookie"), vec!["a=1", "b=2"]);
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn iter_yields_lowercased_names_for_custom_headers() {
        let mut h = HeaderMap::new();
        h.insert("X-A", "1");
        let pairs: Vec<_> = h.iter().collect();
        // Interning lowercases custom names once, at insert. A caller comparing
        // `name == "x-a"` must not have to guess which case survived.
        assert_eq!(pairs, vec![("x-a", "1")]);
    }

    #[test]
    fn stays_inline_for_a_typical_request() {
        let mut h = HeaderMap::new();
        for i in 0..INLINE_HEADERS {
            h.insert(format!("x-{i}"), "v");
        }
        assert!(h.is_inline(), "a typical header set must not spill to the heap");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full headers`
Expected: FAIL — `get` returns `Option<&String>`, `get_id`/`get_bytes` do not exist.

- [ ] **Step 3: Rewrite the storage and the facade**

Replace the `Header` struct and the `HeaderMap` accessors in
`armature-core/src/headers.rs`:

```rust
use armature_h1::{header as header_id, ByteStr, HeaderId};
use bytes::Bytes;

/// One header field: an interned name and a `Bytes` value.
///
/// The value is `Bytes` rather than `String` so it can be a slice of the
/// connection's read buffer once Plan 4 wires the serve path through
/// `armature-h1`. Until then it is a copy, and the type is what makes the later
/// change a non-event.
#[derive(Clone, Debug)]
pub struct Header {
    pub id: HeaderId,
    pub value: Bytes,
}

impl Header {
    #[inline]
    pub fn new(name: impl AsRef<str>, value: impl HeaderValueInput) -> Self {
        Self {
            id: header_id::intern(name.as_ref()),
            value: value.into_value(),
        }
    }

    /// The field name, lowercased for custom names.
    #[inline]
    pub fn name(&self) -> &str {
        self.id.as_str()
    }

    /// The value as UTF-8, or `None` if it is not.
    #[inline]
    pub fn value_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.value).ok()
    }
}

/// Anything that can become a header value.
///
/// This exists so the 228 existing `insert(name, value)` call sites keep
/// compiling across `&str`, `String`, and `Bytes` alike. A plain
/// `impl Into<Bytes>` bound would not: `Bytes: From<&'static str>` but not
/// `From<&'a str>`, so every borrowed-`&str` call site would break.
pub trait HeaderValueInput {
    fn into_value(self) -> Bytes;
}

impl HeaderValueInput for Bytes {
    #[inline]
    fn into_value(self) -> Bytes {
        self
    }
}

impl HeaderValueInput for &str {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::copy_from_slice(self.as_bytes())
    }
}

impl HeaderValueInput for String {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::from(self.into_bytes())
    }
}

impl HeaderValueInput for &[u8] {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::copy_from_slice(self)
    }
}

impl HeaderValueInput for Vec<u8> {
    #[inline]
    fn into_value(self) -> Bytes {
        Bytes::from(self)
    }
}

impl HeaderValueInput for ByteStr {
    #[inline]
    fn into_value(self) -> Bytes {
        self.into_bytes()
    }
}
```

Then the accessors:

```rust
impl HeaderMap {
    /// The value of `name` as UTF-8, case-insensitively.
    ///
    /// Returns `None` for a value that is not valid UTF-8; use
    /// [`HeaderMap::get_bytes`] to see those.
    #[inline]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.get_bytes(name).and_then(|v| std::str::from_utf8(v).ok())
    }

    /// The raw value of `name`, case-insensitively.
    #[inline]
    pub fn get_bytes(&self, name: &str) -> Option<&Bytes> {
        // Interning first turns the per-entry comparison into an integer compare
        // for well-known names, which is the common case on the hot path.
        let id = header_id::intern(name);
        self.get_id(&id)
    }

    /// The raw value for an already-interned name. No interning cost.
    #[inline]
    pub fn get_id(&self, id: &HeaderId) -> Option<&Bytes> {
        self.inner.iter().find(|h| &h.id == id).map(|h| &h.value)
    }

    /// Case-insensitive lookup. Identical to [`HeaderMap::get`]; kept because
    /// call sites use both names.
    #[inline]
    pub fn get_ignore_case(&self, name: &str) -> Option<&str> {
        self.get(name)
    }

    #[inline]
    pub fn contains(&self, name: &str) -> bool {
        self.get_bytes(name).is_some()
    }

    #[inline]
    pub fn contains_key(&self, name: &str) -> bool {
        self.contains(name)
    }

    /// Insert, replacing any existing value for `name`.
    #[inline]
    pub fn insert(
        &mut self,
        name: impl AsRef<str>,
        value: impl HeaderValueInput,
    ) -> Option<Bytes> {
        let id = header_id::intern(name.as_ref());
        let value = value.into_value();
        if let Some(existing) = self.inner.iter_mut().find(|h| h.id == id) {
            return Some(std::mem::replace(&mut existing.value, value));
        }
        self.inner.push(Header { id, value });
        None
    }

    /// Append without replacing, for fields that may repeat (`Set-Cookie`).
    #[inline]
    pub fn append(&mut self, name: impl AsRef<str>, value: impl HeaderValueInput) {
        self.inner.push(Header {
            id: header_id::intern(name.as_ref()),
            value: value.into_value(),
        });
    }

    #[inline]
    pub fn remove(&mut self, name: &str) -> Option<Bytes> {
        let id = header_id::intern(name);
        let pos = self.inner.iter().position(|h| h.id == id)?;
        Some(self.inner.remove(pos).value)
    }

    #[inline]
    pub fn remove_all(&mut self, name: &str) -> usize {
        let id = header_id::intern(name);
        let before = self.inner.len();
        self.inner.retain(|h| h.id != id);
        before - self.inner.len()
    }

    /// Every field as `(name, value)`, skipping non-UTF-8 values.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.inner
            .iter()
            .filter_map(|h| h.value_str().map(|v| (h.name(), v)))
    }

    #[inline]
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.inner.iter().map(|h| h.name())
    }

    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.names()
    }

    #[inline]
    pub fn values(&self) -> impl Iterator<Item = &str> {
        self.inner.iter().filter_map(|h| h.value_str())
    }

    #[inline]
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        let id = header_id::intern(name);
        self.inner
            .iter()
            .filter(|h| h.id == id)
            .filter_map(|h| h.value_str())
            .collect()
    }

    #[inline]
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }

    #[inline]
    pub fn from_hash_map(map: HashMap<String, String>) -> Self {
        let mut out = Self::with_capacity(map.len());
        for (k, v) in map {
            out.insert(k, v);
        }
        out
    }

    #[inline]
    pub fn content_type(&self) -> Option<&str> {
        self.get_id(&HeaderId::ContentType)
            .and_then(|v| std::str::from_utf8(v).ok())
    }

    #[inline]
    pub fn content_length(&self) -> Option<usize> {
        self.get_id(&HeaderId::ContentLength)
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|v| v.parse().ok())
    }
}
```

Update `extend` to take `impl AsRef<str>` keys and `impl HeaderValueInput`
values, and delete `Header::name_eq` (its case-insensitive compare is now
`HeaderId` equality). Keep `INLINE_HEADERS`, `new`, `with_capacity`,
`is_inline`, `len`, `is_empty`, and `clear` unchanged.

- [ ] **Step 4: Run the header tests**

Run: `cargo test -p armature-core --features full headers`
Expected: PASS.

- [ ] **Step 5: Fix `armature-core`'s own call sites**

`get` now yields `&str`, not `&String`. Find and fix:

```bash
cargo build -p armature-core --features full 2>&1 | grep -E '^(error|  -->)' | head -60
```

The mechanical patterns, and what each becomes:

| Old | New |
|---|---|
| `h.get("x").map(String::as_str)` | `h.get("x")` |
| `h.get("x").cloned()` | `h.get("x").map(str::to_owned)` |
| `h.get("x") == Some(&"v".to_string())` | `h.get("x") == Some("v")` |
| `if let Some(v) = h.get("x") { v.as_str() }` | `if let Some(v) = h.get("x") { v }` |
| `h.remove("x").unwrap_or_default()` (wanted `String`) | `h.remove("x").map(\|b\| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()` |

Do **not** paper over a type error with `.to_string()` where the old code had a
`&String` only incidentally — that reintroduces the allocation this task removes.

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
```
Expected: green. Failures outside `armature-core` are expected only if another
crate touches `HeaderMap` directly; fix those here rather than deferring — the
sweep in Task 10 is for `HttpRequest` fields, not for this.

- [ ] **Step 7: Commit**

```bash
git add armature-core/src/headers.rs armature-core/src
git commit -m "refactor(core): store headers as (HeaderId, Bytes) behind a &str facade"
```

---

### Task 3: `HttpRequest.method: String → Method`

**Files:**
- Modify: `armature-core/src/http.rs:16-140`
- Test: `armature-core/src/http.rs` test module

**Interfaces:**
- Consumes: `Method` and its `From<&str>`/`From<String>` impls (Task 1).
- Produces:
  - `HttpRequest.method: Method`
  - `HttpRequest::new(method: impl Into<Method>, path: impl Into<ByteStr>)` —
    signature widened in this task for the method parameter, in Task 4 for the path
  - `HttpRequest::method_str(&self) -> &str`
  - `impl PartialEq<str> for Method` and `impl PartialEq<&str> for Method` in
    `armature-h1` — so the ~41 `req.method == "GET"` comparisons keep compiling

- [ ] **Step 1: Write the failing tests**

In `armature-core/src/http.rs`'s test module:

```rust
    #[test]
    fn new_accepts_str_and_string_and_method() {
        // All three forms must compile: 315 call sites pass a String today, and
        // new code should be able to pass a Method directly.
        let a = HttpRequest::new("GET", "/a");
        let b = HttpRequest::new("POST".to_string(), "/b");
        let c = HttpRequest::new(Method::Put, "/c");
        assert_eq!(a.method, Method::Get);
        assert_eq!(b.method, Method::Post);
        assert_eq!(c.method, Method::Put);
    }

    #[test]
    fn method_compares_against_str_and_reports_itself_as_str() {
        let req = HttpRequest::new("DELETE", "/x");
        assert!(req.method == "DELETE");
        assert!(req.method != "GET");
        assert_eq!(req.method_str(), "DELETE");

        // An unknown token survives intact rather than being coerced.
        let odd = HttpRequest::new("PURGE", "/x");
        assert_eq!(odd.method_str(), "PURGE");
        assert!(odd.method == "PURGE");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full http::tests`
Expected: FAIL — `method` is a `String`; `method_str` does not exist.

- [ ] **Step 3: Add the `Method` string comparisons to `armature-h1`**

In `armature-h1/src/method.rs`:

`Method::as_str` already exists and already covers `Other`, so this adds only the
comparisons and `Display`:

```rust
impl PartialEq<str> for Method {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Method {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
```

Add `use std::fmt;` if the file does not already have it.

- [ ] **Step 4: Change the field and the constructors**

In `armature-core/src/http.rs`:

```rust
pub struct HttpRequest {
    /// The request method.
    ///
    /// Was a `String`. An unrecognized token is carried as `Method::Other`
    /// rather than rejected here; routing answers it with 405.
    pub method: Method,
    // ...
}

impl HttpRequest {
    /// Create a request.
    ///
    /// Generic in the method so every existing `HttpRequest::new("GET".to_string(), …)`
    /// call site compiles unchanged.
    #[inline]
    pub fn new(method: impl Into<Method>, path: String) -> Self {
        Self {
            method: method.into(),
            path,
            // ... unchanged
        }
    }

    /// The method as a string, for logging and for code that compares tokens.
    #[inline]
    pub fn method_str(&self) -> &str {
        self.method.as_str()
    }
}
```

Apply the same widening to `with_extensions_capacity` and `with_bytes_body`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p armature-core --features full http::tests`
Expected: PASS.

- [ ] **Step 6: Fix `armature-core`'s 25 struct-literal constructions and comparisons**

```bash
cargo build -p armature-core --features full 2>&1 | grep -E '^error' -A3 | head -80
```

Patterns:

| Old | New |
|---|---|
| `HttpRequest { method: "GET".to_string(), .. }` | `HttpRequest { method: Method::Get, .. }` |
| `req.method.as_str()` | `req.method_str()` |
| `req.method.clone()` into a `String` field | `req.method_str().to_owned()` |
| `match req.method.as_str() { "GET" => …` | `match req.method { Method::Get => …` |
| `route.method.as_str() != request.method` | `route.method.as_str() != request.method_str()` |

- [ ] **Step 7: Run the gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
cargo test -p armature-h1 --all-features
```
Expected: green.

- [ ] **Step 8: Commit**

```bash
git add armature-h1/src/method.rs armature-core/src
git commit -m "refactor(core)!: HttpRequest.method is a Method rather than a String"
```

---

### Task 4: `path: String → ByteStr` and bodies to `Bytes`

**Files:**
- Modify: `armature-core/src/http.rs:16-140` (request) and `:390-460` (response)
- Test: `armature-core/src/http.rs` test module

**Interfaces:**
- Consumes: `ByteStr` (Task 1), `Method` on the request (Task 3).
- Produces:
  - `HttpRequest.path: ByteStr`, `HttpRequest.body: Bytes`
  - `HttpResponse.body: Bytes`
  - `HttpRequest::path_str(&self) -> &str`, `HttpRequest::body_slice(&self) -> &[u8]`
  - `HttpResponse::body_slice(&self) -> &[u8]`
  - The private `body_bytes: Option<Bytes>` shadow field is **deleted** from both
    types — with `body: Bytes` there is nothing left for it to optimize.
    `body_bytes()`, `set_body_bytes()`, `has_bytes_body()`, and `body_ref()` are
    kept as thin forwarders so their call sites survive.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn path_is_a_bytestr_and_still_compares_and_prints_as_a_str() {
        let req = HttpRequest::new("GET", "/users/42?a=1");
        assert_eq!(req.path_str(), "/users/42?a=1");
        assert!(req.path == "/users/42?a=1");
        assert_eq!(format!("{}", req.path), "/users/42?a=1");
    }

    #[test]
    fn request_body_is_bytes_and_the_shadow_field_is_gone() {
        let mut req = HttpRequest::new("POST", "/x");
        req.set_body(b"hello".to_vec());
        assert_eq!(req.body_slice(), b"hello");
        // The old two-field arrangement could disagree with itself; one field
        // cannot.
        assert_eq!(req.body_bytes(), bytes::Bytes::from_static(b"hello"));
        assert!(req.has_bytes_body());

        req.set_body_bytes(bytes::Bytes::from_static(b"world"));
        assert_eq!(req.body_slice(), b"world");
        assert_eq!(req.body_ref(), b"world");
    }

    #[test]
    fn cloning_a_body_does_not_copy_it() {
        let big = bytes::Bytes::from(vec![7u8; 64 * 1024]);
        let mut req = HttpRequest::new("POST", "/x");
        req.set_body_bytes(big.clone());
        let copy = req.clone();
        // Same allocation, reached from two requests: the whole point of Bytes.
        assert_eq!(copy.body.as_ptr(), req.body.as_ptr());
    }

    #[test]
    fn response_body_is_bytes() {
        let mut resp = HttpResponse::new(200);
        resp.body = bytes::Bytes::from_static(b"{}");
        assert_eq!(resp.body_slice(), b"{}");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full http::tests`
Expected: FAIL — `path_str`, `body_slice` missing; `body.as_ptr()` does not exist on `Vec` in the sense tested.

- [ ] **Step 3: Change the fields**

```rust
pub struct HttpRequest {
    pub method: Method,
    /// The raw request target, query string included.
    ///
    /// Was a `String`. A `ByteStr` so it can be a slice of the connection read
    /// buffer once Plan 4 lands; `Deref<Target = str>` keeps `&req.path` working
    /// wherever a `&str` is wanted.
    pub path: ByteStr,
    pub headers: HeaderMap,
    /// The request body.
    ///
    /// Was a `Vec<u8>` shadowed by an optional `Bytes`. One field, always
    /// authoritative.
    pub body: Bytes,
    // path_params / query_params replaced in Tasks 5 and 6
    pub extensions: Extensions,
}

impl HttpRequest {
    #[inline]
    pub fn new(method: impl Into<Method>, path: impl Into<ByteStr>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            path_params: Default::default(),
            extensions: Extensions::new(),
        }
    }

    /// The request target as a string.
    #[inline]
    pub fn path_str(&self) -> &str {
        self.path.as_str()
    }

    /// The body as a byte slice.
    #[inline]
    pub fn body_slice(&self) -> &[u8] {
        &self.body
    }

    /// The body as `Bytes`. A refcount bump, not a copy.
    #[inline]
    pub fn body_bytes(&self) -> Bytes {
        self.body.clone()
    }

    /// Kept for call-site compatibility; `body` is always `Bytes` now.
    #[inline]
    pub fn has_bytes_body(&self) -> bool {
        !self.body.is_empty()
    }

    #[inline]
    pub fn body_ref(&self) -> &[u8] {
        &self.body
    }

    #[inline]
    pub fn set_body_bytes(&mut self, bytes: Bytes) {
        self.body = bytes;
    }

    /// Takes a `Vec` and hands over its allocation without copying.
    #[inline]
    pub fn set_body(&mut self, body: Vec<u8>) {
        self.body = Bytes::from(body);
    }
}
```

Do the same for `HttpResponse`: `body: Bytes`, delete `body_bytes:
Option<Bytes>`, and add `body_slice()`. Keep `status`, `headers: LazyHeaders`,
and `cookies` untouched — `LazyHeaders` is a `String`-keyed map for *response*
headers and is out of scope for this plan.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p armature-core --features full http::tests`
Expected: PASS.

- [ ] **Step 5: Fix `armature-core`'s call sites**

```bash
cargo build -p armature-core --features full 2>&1 | grep -E '^error' -A3 | head -100
```

| Old | New |
|---|---|
| `req.path.clone()` into a `String` | `req.path_str().to_owned()`, or clone the `ByteStr` if the target can hold one |
| `req.path.split('?')` | unchanged — `Deref<Target = str>` |
| `&req.path[..]` | unchanged |
| `req.body.clone()` into `Vec<u8>` | `req.body.to_vec()` |
| `req.body.len()`, `req.body.is_empty()` | unchanged |
| `resp.body = vec` | `resp.body = Bytes::from(vec)` |
| `resp.body.extend_from_slice(x)` | build a `BytesMut`, or `resp.body = Bytes::from([&resp.body[..], x].concat())` — flag any of these for Plan 3's `write_into`, do not micro-optimize here |
| `String::from_utf8(req.body.clone())` | `String::from_utf8(req.body.to_vec())` |

- [ ] **Step 6: Run the gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
```
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add armature-core/src
git commit -m "refactor(core)!: path is a ByteStr and bodies are Bytes"
```

---

### Task 5: Lazy, memoized query parsing

`application.rs:1545` percent-decodes the entire query string into a
`HashMap<String, String>` on every request that has one. Most handlers never read
it. This task makes that work happen on first access, or never.

**Files:**
- Create: `armature-core/src/query.rs`
- Modify: `armature-core/src/http.rs` (remove `query_params`, add the cache and
  `query()`/`query_param()`)
- Modify: `armature-core/src/lib.rs` (declare and re-export)
- Test: `armature-core/src/query.rs` test module

**Interfaces:**
- Consumes: `ByteStr`, `HttpRequest.path` (Task 4).
- Produces:
  - `pub struct Query<'a>` with `get(&self, key: &str) -> Option<&str>`,
    `get_all(&self, key: &str) -> impl Iterator<Item = &str>`,
    `iter() -> impl Iterator<Item = (&str, &str)>`, `len()`, `is_empty()`,
    `to_hash_map() -> HashMap<String, String>`
  - `HttpRequest::query(&self) -> Query<'_>` — **renamed from the old
    `query(&self, name: &str) -> Option<&String>`**
  - `HttpRequest::query_param(&self, name: &str) -> Option<&str>` — the
    replacement for the old `query(name)`; 57 call sites move to it
  - `HttpRequest::query_string(&self) -> Option<&str>`

- [ ] **Step 1: Write the failing tests**

Create `armature-core/src/query.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use crate::HttpRequest;

    #[test]
    fn parses_on_first_access_and_decodes() {
        let req = HttpRequest::new("GET", "/s?q=hello%20world&page=2");
        let q = req.query();
        assert_eq!(q.get("q"), Some("hello world"));
        assert_eq!(q.get("page"), Some("2"));
        assert_eq!(q.get("absent"), None);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn plus_is_a_space_and_percent_escapes_decode() {
        let req = HttpRequest::new("GET", "/s?a=x+y&b=%2Fpath&c=%E2%9C%93");
        let q = req.query();
        assert_eq!(q.get("a"), Some("x y"));
        assert_eq!(q.get("b"), Some("/path"));
        assert_eq!(q.get("c"), Some("✓"));
    }

    #[test]
    fn repeated_keys_are_all_reachable_and_get_returns_the_first() {
        let req = HttpRequest::new("GET", "/s?tag=a&tag=b");
        let q = req.query();
        assert_eq!(q.get("tag"), Some("a"));
        assert_eq!(q.get_all("tag").collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn no_query_string_is_an_empty_view_not_a_panic() {
        let req = HttpRequest::new("GET", "/s");
        assert!(req.query().is_empty());
        assert_eq!(req.query_string(), None);
        assert_eq!(req.query_param("x"), None);
    }

    #[test]
    fn malformed_input_degrades_rather_than_failing() {
        // A bare key, an empty value, a stray '=', and a truncated escape. None of
        // these is worth rejecting a request over, and all of them appear in real
        // traffic.
        let req = HttpRequest::new("GET", "/s?flag&empty=&=novalue&bad=%zz&trunc=%2");
        let q = req.query();
        assert_eq!(q.get("flag"), Some(""));
        assert_eq!(q.get("empty"), Some(""));
        // An undecodable escape is preserved verbatim rather than dropped, so a
        // handler sees what the client actually sent.
        assert_eq!(q.get("bad"), Some("%zz"));
        assert_eq!(q.get("trunc"), Some("%2"));
    }

    #[test]
    fn the_view_is_memoized_across_calls() {
        let req = HttpRequest::new("GET", "/s?a=1");
        let first = req.query().get("a").map(str::to_owned);
        let second = req.query().get("a").map(str::to_owned);
        assert_eq!(first, second);
        // Same backing storage both times: the second call must not re-parse.
        let p1 = req.query().iter().next().map(|(k, _)| k.as_ptr());
        let p2 = req.query().iter().next().map(|(k, _)| k.as_ptr());
        assert_eq!(p1, p2);
    }

    #[test]
    fn cloning_a_request_does_not_carry_a_stale_cache() {
        let req = HttpRequest::new("GET", "/s?a=1");
        assert_eq!(req.query().get("a"), Some("1"));
        let mut clone = req.clone();
        clone.path = crate::ByteStr::from("/s?a=2");
        // The clone's path changed, so its cache must not answer for the old one.
        assert_eq!(clone.query().get("a"), Some("2"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full query`
Expected: FAIL — the module is not declared and `query()` takes an argument.

- [ ] **Step 3: Implement `Query` and the parser**

In `armature-core/src/query.rs`, above the test module:

```rust
//! Lazy query-string parsing.
//!
//! The old path parsed *and* percent-decoded the whole query into a
//! `HashMap<String, String>` on every request that had one, whether or not any
//! handler read it. This parses on first access and memoizes, so a handler that
//! ignores the query pays nothing beyond carrying the raw bytes it already had.

use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashMap;

/// Parsed query pairs. Eight inline slots covers essentially all real queries.
pub type QueryPairs = SmallVec<[(String, String); 8]>;

/// A parsed view over a request's query string.
///
/// Borrowed from the request, so it cannot outlive it — which is what lets the
/// values be slices of the request's own memory rather than copies.
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    pairs: &'a [(String, String)],
}

impl<'a> Query<'a> {
    #[inline]
    pub(crate) fn new(pairs: &'a [(String, String)]) -> Self {
        Self { pairs }
    }

    /// The first value for `key`.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Every value for `key`, in the order the client sent them.
    #[inline]
    pub fn get_all(&self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.pairs
            .iter()
            .filter(move |(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// An owned copy, for the call sites that genuinely need one.
    ///
    /// This is the allocation the lazy path exists to avoid — reach for it only
    /// when a `HashMap` is actually required.
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        self.pairs.iter().cloned().collect()
    }
}

/// Parse `query` into key/value pairs, percent-decoding both sides.
///
/// Malformed input degrades rather than erroring: a bare key gets an empty
/// value, and an escape that does not decode is preserved verbatim so the
/// handler sees what the client sent. Rejecting a request over a stray `%` would
/// break clients for no security gain — nothing downstream trusts these bytes.
pub(crate) fn parse(query: &str) -> QueryPairs {
    let mut out = QueryPairs::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        if raw_key.is_empty() {
            continue;
        }
        out.push((
            decode(raw_key).into_owned(),
            decode(raw_value).into_owned(),
        ));
    }
    out
}

/// Percent- and plus-decode one component.
///
/// Returns `Cow::Borrowed` when there is nothing to decode, which is the common
/// case, so the copy happens only for values that need it.
fn decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') && !s.contains('+') {
        return Cow::Borrowed(s);
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                // `get` does the bounds check, so a truncated escape at the end
                // of the input falls into the `None` arm rather than panicking.
                match bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        // Not a valid escape. Keep it as written.
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }

    match String::from_utf8(out) {
        Ok(decoded) => Cow::Owned(decoded),
        // Decoded to non-UTF-8: hand back the raw form rather than lossy text.
        Err(_) => Cow::Borrowed(s),
    }
}
```

- [ ] **Step 4: Wire the cache into `HttpRequest`**

In `armature-core/src/http.rs`:

```rust
use crate::query::{parse as parse_query, Query, QueryPairs};
use std::sync::OnceLock;

/// The memoized query pairs.
///
/// `OnceLock` rather than `OnceCell` because `HttpRequest` must stay `Sync` in
/// this plan: extractors hold `&HttpRequest` across an `await` inside a `Send`
/// future, and `&T: Send` requires `T: Sync`. Plan 3 drops the `Send` bound from
/// handlers, and this can become an `OnceCell` then.
#[derive(Debug, Default)]
pub struct QueryCache(OnceLock<QueryPairs>);

impl Clone for QueryCache {
    /// A clone starts cold.
    ///
    /// Carrying the parsed pairs across a clone would be wrong, not merely
    /// wasteful: `path` is a public field, so a caller can clone a request and
    /// then change its target. A cold cache cannot answer for the wrong path.
    fn clone(&self) -> Self {
        Self(OnceLock::new())
    }
}

pub struct HttpRequest {
    pub method: Method,
    pub path: ByteStr,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub path_params: PathParams, // Task 6
    pub extensions: Extensions,
    /// Parsed lazily by [`HttpRequest::query`].
    query_cache: QueryCache,
}

impl HttpRequest {
    /// The raw query string, without the `?`.
    #[inline]
    pub fn query_string(&self) -> Option<&str> {
        self.path.as_str().split_once('?').map(|(_, q)| q)
    }

    /// A parsed view of the query string.
    ///
    /// Parses on the first call and memoizes; a handler that never calls this
    /// pays nothing. Note the shape change: this used to take a name and return
    /// one value — that accessor is now [`HttpRequest::query_param`].
    #[inline]
    pub fn query(&self) -> Query<'_> {
        let pairs = self.query_cache.0.get_or_init(|| match self.query_string() {
            Some(q) => parse_query(q),
            None => QueryPairs::new(),
        });
        Query::new(pairs)
    }

    /// The first query value for `name`.
    #[inline]
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query().get(name)
    }
}
```

Delete the `query_params` field and the old `query(&self, name)` method. Add
`pub mod query;` and `pub use query::Query;` to `armature-core/src/lib.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p armature-core --features full query`
Expected: PASS.

- [ ] **Step 6: Migrate `armature-core`'s 46 `query_params` uses and 15 `query(name)` calls**

```bash
grep -rn 'query_params\|\.query(' --include='*.rs' armature-core/src | grep -v 'fn query' | head -60
```

| Old | New |
|---|---|
| `req.query_params.get("page")` | `req.query_param("page")` |
| `req.query_params.get("p").cloned()` | `req.query_param("p").map(str::to_owned)` |
| `req.query_params.is_empty()` | `req.query().is_empty()` |
| `req.query_params.len()` | `req.query().len()` |
| `for (k, v) in &req.query_params` | `for (k, v) in req.query().iter()` |
| `req.query_params = map` | delete — the query comes from `path` now. If a test built a request by setting this field, build it with the query in the path instead: `HttpRequest::new("GET", "/x?a=1")` |
| `req.query("page")` | `req.query_param("page")` |
| `req.query_params.clone()` | `req.query().to_hash_map()` |

- [ ] **Step 7: Run the gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
```
Expected: green. Crates outside `armature-core` will fail here — that is Task 10.
If the failure count outside `armature-core` is large enough to obscure real
errors, scope the gate to `cargo test -p armature-core --features full` for this
task and let Task 10 restore the whole-workspace gate.

- [ ] **Step 8: Commit**

```bash
git add armature-core/src/query.rs armature-core/src/http.rs armature-core/src/lib.rs armature-core/src
git commit -m "perf(core)!: parse the query string lazily instead of on every request"
```

---

### Task 6: Span-based path parameters

**Files:**
- Create: `armature-core/src/param_intern.rs`
- Modify: `armature-core/src/http.rs`
- Modify: `armature-core/src/lib.rs`
- Test: `armature-core/src/param_intern.rs` and `armature-core/src/http.rs` test modules

**Interfaces:**
- Consumes: `Bytes`, `HttpRequest` (Task 4).
- Produces:
  - `pub type PathParams = SmallVec<[(&'static str, Bytes); 4]>`
  - `HttpRequest.path_params: PathParams`
  - `HttpRequest::param(&self, name: &str) -> Option<&str>` (was `Option<&String>`)
  - `HttpRequest::param_bytes(&self, name: &str) -> Option<&Bytes>`
  - `HttpRequest::set_params(&mut self, params: PathParams)`
  - `armature_core::param_intern::intern(name: &str) -> &'static str`

- [ ] **Step 1: Write the failing tests**

`armature-core/src/param_intern.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::intern;

    #[test]
    fn interning_the_same_name_twice_yields_the_same_pointer() {
        let a = intern("user_id");
        let b = intern("user_id");
        // Pointer equality, not just string equality: the point of interning is
        // that a route's param names are allocated once at startup, never per
        // request.
        assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
        assert_eq!(a, "user_id");
    }

    #[test]
    fn distinct_names_are_distinct() {
        assert_eq!(intern("a"), "a");
        assert_eq!(intern("b"), "b");
        assert!(!std::ptr::eq(intern("a").as_ptr(), intern("b").as_ptr()));
    }
}
```

`armature-core/src/http.rs`:

```rust
    #[test]
    fn params_read_back_as_str_and_bytes() {
        let mut req = HttpRequest::new("GET", "/users/42/posts/7");
        let mut params = PathParams::new();
        params.push((crate::param_intern::intern("user_id"), bytes::Bytes::from_static(b"42")));
        params.push((crate::param_intern::intern("post_id"), bytes::Bytes::from_static(b"7")));
        req.set_params(params);

        assert_eq!(req.param("user_id"), Some("42"));
        assert_eq!(req.param("post_id"), Some("7"));
        assert_eq!(req.param("nope"), None);
        assert_eq!(req.param_bytes("user_id").map(|b| b.len()), Some(2));
        assert_eq!(req.param("user_id").and_then(|v| v.parse::<u32>().ok()), Some(42));
    }

    #[test]
    fn four_params_stay_inline() {
        let mut params = PathParams::new();
        for name in ["a", "b", "c", "d"] {
            params.push((crate::param_intern::intern(name), bytes::Bytes::from_static(b"x")));
        }
        assert!(!params.spilled(), "four params must not allocate");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full param`
Expected: FAIL — `param_intern` does not exist; `path_params` is a `HashMap`.

- [ ] **Step 3: Implement the interner**

`armature-core/src/param_intern.rs`:

```rust
//! Leak-once interning for route parameter names.
//!
//! A request's path params are `(&'static str, Bytes)`: the name comes from the
//! compiled route pattern and the value is a slice of the request target. The
//! name has to outlive the request without being cloned per request, and route
//! registration happens at startup, so leaking one `Box<str>` per distinct
//! parameter name is the whole cost — bounded by the number of route parameters
//! an application declares, not by traffic.
//!
//! This is deliberately not a general-purpose interner. Do not call it with
//! request-derived strings: that would let a client grow the process without
//! bound.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

fn table() -> &'static Mutex<HashSet<&'static str>> {
    static TABLE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Intern a route parameter name, returning a `&'static str`.
///
/// Call this at route-registration time only.
pub fn intern(name: &str) -> &'static str {
    let mut table = table().lock().expect("param intern table poisoned");
    if let Some(existing) = table.get(name) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    table.insert(leaked);
    leaked
}
```

- [ ] **Step 4: Change the field and accessors**

In `armature-core/src/http.rs`:

```rust
/// Route parameters captured from the request target.
///
/// Names are `&'static str` from the compiled route pattern (see
/// [`crate::param_intern`]), values are slices of the target — so a matched
/// route costs no allocation for either half. Four inline slots covers the
/// overwhelming majority of routes.
pub type PathParams = SmallVec<[(&'static str, Bytes); 4]>;

impl HttpRequest {
    /// A captured route parameter, as UTF-8.
    #[inline]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.param_bytes(name)
            .and_then(|v| std::str::from_utf8(v).ok())
    }

    /// A captured route parameter, raw.
    #[inline]
    pub fn param_bytes(&self, name: &str) -> Option<&Bytes> {
        self.path_params
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v)
    }

    /// Replace the captured parameters. Called by the router.
    #[inline]
    pub fn set_params(&mut self, params: PathParams) {
        self.path_params = params;
    }
}
```

Add `pub mod param_intern;` and `pub use http::PathParams;` to `lib.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p armature-core --features full param`
Expected: PASS.

- [ ] **Step 6: Migrate `armature-core`'s 24 `path_params` uses and 6 `param()` calls**

| Old | New |
|---|---|
| `req.path_params.get("id")` | `req.param("id")` |
| `req.path_params.insert(k, v)` | build a `PathParams` and `req.set_params(..)` |
| `req.path_params = map` (from `match_path`) | Task 8 supplies a `PathParams` directly |
| `req.param("id").unwrap().parse()` | unchanged — `&str` parses the same |
| `req.param("id").cloned()` | `req.param("id").map(str::to_owned)` |
| `constraints.validate(&params)` (takes `&HashMap`) | change `RouteConstraints::validate` to take `&PathParams`; it only reads names and values |

- [ ] **Step 7: Run the gate**

Run `cargo test -p armature-core --features full` (workspace-wide green returns in Task 10).
Expected: `armature-core` green.

- [ ] **Step 8: Commit**

```bash
git add armature-core/src/param_intern.rs armature-core/src/http.rs armature-core/src/lib.rs armature-core/src
git commit -m "perf(core)!: path params are interned-name spans rather than a HashMap"
```

---

### Task 7: `Extensions` as a `SmallVec`

**Files:**
- Modify: `armature-core/src/extensions.rs`
- Test: `armature-core/src/extensions.rs` test module

**Interfaces:**
- Produces: identical public API — `new`, `with_capacity`, `insert`, `insert_arc`,
  `get`, `get_arc`, `contains`, `remove`, `clear`, `len`, `is_empty`, `extend` —
  backed by `SmallVec<[(TypeId, Arc<dyn Any + Send + Sync>); 8]>`.
- **The `Send + Sync` bounds stay.** The spec's `Rc<dyn Any>` follows B5, which is
  Plan 3. Changing it here breaks every `Send` future holding a request.

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn eight_extensions_stay_inline() {
        let mut ext = Extensions::new();
        ext.insert(1u8);
        ext.insert(2u16);
        ext.insert(3u32);
        ext.insert(4u64);
        ext.insert(5i8);
        ext.insert(6i16);
        ext.insert(7i32);
        ext.insert(8i64);
        assert_eq!(ext.len(), 8);
        assert!(!ext.spilled(), "eight extensions must not allocate a table");
        assert_eq!(ext.get::<u32>(), Some(&3u32));
    }

    #[test]
    fn insert_replaces_the_same_type() {
        let mut ext = Extensions::new();
        ext.insert(1u32);
        ext.insert(2u32);
        assert_eq!(ext.len(), 1);
        assert_eq!(ext.get::<u32>(), Some(&2u32));
    }

    #[test]
    fn extend_overwrites_colliding_types_and_keeps_the_rest() {
        let mut a = Extensions::new();
        a.insert(1u32);
        a.insert("keep");
        let mut b = Extensions::new();
        b.insert(2u32);
        a.extend(b);
        assert_eq!(a.get::<u32>(), Some(&2u32));
        assert_eq!(a.get::<&str>(), Some(&"keep"));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full extensions`
Expected: FAIL — `spilled` does not exist on a `HashMap`-backed `Extensions`.

- [ ] **Step 3: Swap the storage**

```rust
use smallvec::SmallVec;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Eight inline slots. A linear scan comparing `TypeId`s — one `u128` compare
/// each — beats hashing one, and it never allocates for a realistic request.
type Slots = SmallVec<[(TypeId, Arc<dyn Any + Send + Sync>); 8]>;

#[derive(Clone, Default)]
pub struct Extensions {
    slots: Slots,
}

impl Extensions {
    #[inline]
    pub fn new() -> Self {
        Self { slots: Slots::new() }
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self { slots: Slots::with_capacity(capacity) }
    }

    /// Whether the storage has spilled to the heap.
    #[inline]
    pub fn spilled(&self) -> bool {
        self.slots.spilled()
    }

    #[inline]
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        self.insert_arc(Arc::new(value));
    }

    #[inline]
    pub fn insert_arc<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        let id = TypeId::of::<T>();
        let erased: Arc<dyn Any + Send + Sync> = value;
        if let Some(slot) = self.slots.iter_mut().find(|(k, _)| *k == id) {
            slot.1 = erased;
            return;
        }
        self.slots.push((id, erased));
    }

    #[inline]
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        self.slots
            .iter()
            .find(|(k, _)| *k == id)
            .and_then(|(_, v)| v.downcast_ref::<T>())
    }

    #[inline]
    pub fn get_arc<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let id = TypeId::of::<T>();
        self.slots
            .iter()
            .find(|(k, _)| *k == id)
            .and_then(|(_, v)| v.clone().downcast::<T>().ok())
    }

    #[inline]
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
        let id = TypeId::of::<T>();
        self.slots.iter().any(|(k, _)| *k == id)
    }

    #[inline]
    pub fn remove<T: Send + Sync + 'static>(&mut self) -> bool {
        let id = TypeId::of::<T>();
        match self.slots.iter().position(|(k, _)| *k == id) {
            Some(i) => {
                self.slots.remove(i);
                true
            }
            None => false,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Merge `other` in, its entries winning on a type collision.
    pub fn extend(&mut self, other: Extensions) {
        for (id, value) in other.slots {
            if let Some(slot) = self.slots.iter_mut().find(|(k, _)| *k == id) {
                slot.1 = value;
            } else {
                self.slots.push((id, value));
            }
        }
    }
}
```

Keep the module docs, adjusting the "Memory" bullet to say
`SmallVec` rather than `HashMap`. Remove the now-unused `HashMap` import if
nothing else in the file uses it.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p armature-core --features full extensions`
Expected: PASS.

- [ ] **Step 5: Run the gate**

Run `cargo test -p armature-core --features full` and
`cargo clippy -p armature-core --all-targets --features full -- -D warnings`.
Expected: green — the public API did not change, so no call site should move.

- [ ] **Step 6: Commit**

```bash
git add armature-core/src/extensions.rs
git commit -m "perf(core): back Extensions with a SmallVec instead of a HashMap"
```

---

### Task 8: Method-indexed `matchit` router

Today `Router::route` scans a `Vec<Route>` linearly, comparing a method *string*
per candidate and building a `HashMap` per match. `matchit` has been a declared
dependency of `armature-core` since before this plan and is not used anywhere.

**Files:**
- Modify: `armature-core/src/routing.rs`
- Test: `armature-core/src/routing.rs` test module, plus
  `armature-core/tests/routing_tests.rs`

**Interfaces:**
- Consumes: `Method` (Task 1), `PathParams` and `param_intern` (Task 6),
  `HttpRequest.path`/`method` (Tasks 3–4).
- Produces:
  - `Router::route(&self, req: HttpRequest) -> Result<HttpResponse, Error>` —
    unchanged signature, new internals
  - `Router::match_route(&self, method: &str, path: &str) -> Option<(BoxedHandler, PathParams)>`
    — **return type changes** from `HashMap<String, String>`
  - `pub(crate) fn translate_pattern(pattern: &str) -> String` — `:id` → `{id}`,
    `*rest` → `{*rest}`
  - Private `MethodIndex` built lazily and invalidated by `add_route`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn translates_armature_patterns_to_matchit_syntax() {
        assert_eq!(translate_pattern("/users"), "/users");
        assert_eq!(translate_pattern("/users/:id"), "/users/{id}");
        assert_eq!(
            translate_pattern("/users/:user_id/posts/:post_id"),
            "/users/{user_id}/posts/{post_id}"
        );
        assert_eq!(translate_pattern("/files/*path"), "/files/{*path}");
        assert_eq!(translate_pattern("/users/:id/files/*path"), "/users/{id}/files/{*path}");
        // Already-braced patterns pass through, so a user who wrote matchit
        // syntax directly is not broken by the translation.
        assert_eq!(translate_pattern("/users/{id}"), "/users/{id}");
    }

    #[tokio::test]
    async fn dispatches_by_method_without_comparing_strings() {
        let mut router = Router::new();
        router.get("/r", || async { HttpResponse::new(200) });
        router.post("/r", || async { HttpResponse::new(201) });

        let get = router.route(HttpRequest::new("GET", "/r")).await.unwrap();
        let post = router.route(HttpRequest::new("POST", "/r")).await.unwrap();
        assert_eq!(get.status, 200);
        assert_eq!(post.status, 201);
    }

    #[tokio::test]
    async fn captures_params_as_spans_of_the_target() {
        let mut router = Router::new();
        router.get("/users/:id/posts/:post", |req: HttpRequest| async move {
            let mut r = HttpResponse::new(200);
            r.body = bytes::Bytes::from(format!(
                "{}:{}",
                req.param("id").unwrap_or("-"),
                req.param("post").unwrap_or("-")
            ));
            r
        });

        let resp = router
            .route(HttpRequest::new("GET", "/users/42/posts/7"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"42:7");
    }

    #[tokio::test]
    async fn catch_all_still_matches() {
        let mut router = Router::new();
        router.get("/files/*path", |req: HttpRequest| async move {
            let mut r = HttpResponse::new(200);
            r.body = bytes::Bytes::from(req.param("path").unwrap_or("-").to_owned());
            r
        });
        let resp = router
            .route(HttpRequest::new("GET", "/files/a/b/c.txt"))
            .await
            .unwrap();
        assert_eq!(resp.body_slice(), b"a/b/c.txt");
    }

    #[tokio::test]
    async fn a_query_string_does_not_participate_in_matching() {
        let mut router = Router::new();
        router.get("/s", || async { HttpResponse::new(200) });
        let resp = router.route(HttpRequest::new("GET", "/s?a=1&b=2")).await.unwrap();
        assert_eq!(resp.status, 200);
    }

    #[tokio::test]
    async fn duplicate_routes_keep_first_registered_wins() {
        // matchit rejects conflicting inserts; the old linear scan accepted them
        // and took the first. Existing applications rely on that, so a conflict
        // must fall back to a scan rather than panicking or reordering.
        let mut router = Router::new();
        router.get("/dup", || async { HttpResponse::new(200) });
        router.get("/dup", || async { HttpResponse::new(500) });
        let resp = router.route(HttpRequest::new("GET", "/dup")).await.unwrap();
        assert_eq!(resp.status, 200, "the first registration must win");
    }

    #[tokio::test]
    async fn routes_added_after_the_first_dispatch_are_visible() {
        let mut router = Router::new();
        router.get("/a", || async { HttpResponse::new(200) });
        assert_eq!(router.route(HttpRequest::new("GET", "/a")).await.unwrap().status, 200);

        // The index is built lazily; adding a route has to invalidate it.
        router.get("/b", || async { HttpResponse::new(201) });
        assert_eq!(router.route(HttpRequest::new("GET", "/b")).await.unwrap().status, 201);
    }

    #[tokio::test]
    async fn an_unroutable_method_is_405_not_a_panic() {
        let mut router = Router::new();
        router.get("/a", || async { HttpResponse::new(200) });
        let resp = router.route(HttpRequest::new("PURGE", "/a")).await;
        // Whatever the existing no-match behavior is (404/405), an Other method
        // must reach it rather than unwrapping a missing tree.
        assert!(resp.is_ok());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p armature-core --features full routing`
Expected: FAIL — `translate_pattern` does not exist; param capture returns a `HashMap`.

- [ ] **Step 3: Implement the pattern translation**

```rust
/// Rewrite an armature route pattern into `matchit` syntax.
///
/// `:id` becomes `{id}` and `*rest` becomes `{*rest}`. User-facing pattern
/// syntax does not change — this is the seam that keeps it from having to.
/// Segments that are already braced pass through untouched.
pub(crate) fn translate_pattern(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    for (i, segment) in pattern.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        match segment.as_bytes().first() {
            Some(b':') => {
                out.push('{');
                out.push_str(&segment[1..]);
                out.push('}');
            }
            Some(b'*') => {
                out.push_str("{*");
                out.push_str(&segment[1..]);
                out.push('}');
            }
            _ => out.push_str(segment),
        }
    }
    out
}
```

- [ ] **Step 4: Implement the method-indexed index**

```rust
use crate::param_intern;
use crate::{Method, PathParams};
use std::sync::OnceLock;

/// The number of method-indexed trees: the routable methods of `HttpMethod`.
const METHOD_SLOTS: usize = 8;

/// Index of `method` into the tree array.
#[inline]
fn method_slot(method: &Method) -> Option<usize> {
    Some(match method {
        Method::Get => 0,
        Method::Post => 1,
        Method::Put => 2,
        Method::Delete => 3,
        Method::Patch => 4,
        Method::Head => 5,
        Method::Options => 6,
        Method::Query => 7,
        // CONNECT, TRACE, and unknown tokens are not routable; the caller
        // answers them the same way it answers a path that does not match.
        _ => return None,
    })
}

/// One `matchit` tree per routable method, plus a linear fallback.
///
/// The fallback exists because `matchit` rejects conflicting patterns while the
/// linear scan this replaces accepted them and took the first match. Silently
/// dropping the loser would change the behavior of applications that register
/// overlapping routes, so a rejected insert lands here and is scanned in
/// registration order.
struct MethodIndex {
    trees: [Option<matchit::Router<usize>>; METHOD_SLOTS],
    fallback: Vec<usize>,
}

impl MethodIndex {
    fn build(routes: &[Route]) -> Self {
        let mut trees: [Option<matchit::Router<usize>>; METHOD_SLOTS] = Default::default();
        let mut fallback = Vec::new();

        for (idx, route) in routes.iter().enumerate() {
            let method = Method::from(route.method.clone());
            let Some(slot) = method_slot(&method) else {
                fallback.push(idx);
                continue;
            };
            let tree = trees[slot].get_or_insert_with(matchit::Router::new);
            if tree.insert(translate_pattern(&route.path), idx).is_err() {
                // A conflict, an unsupported pattern, or a duplicate. Preserve
                // first-registered-wins by scanning instead.
                fallback.push(idx);
            }
        }

        Self { trees, fallback }
    }

    /// The lowest-registration-index route matching `method` and `path`.
    fn find(&self, routes: &[Route], method: &Method, path: &str) -> Option<(usize, PathParams)> {
        let mut best: Option<(usize, PathParams)> = None;

        if let Some(slot) = method_slot(method)
            && let Some(tree) = self.trees[slot].as_ref()
            && let Ok(m) = tree.at(path)
        {
            let mut params = PathParams::new();
            for (name, value) in m.params.iter() {
                params.push((
                    param_intern::intern(name),
                    Bytes::copy_from_slice(value.as_bytes()),
                ));
            }
            best = Some((*m.value, params));
        }

        // A fallback route registered earlier than the tree match has to win, or
        // adding a conflicting route later would silently change which handler
        // serves an existing path.
        for &idx in &self.fallback {
            if best.as_ref().is_some_and(|(b, _)| *b < idx) {
                break;
            }
            let route = &routes[idx];
            if Method::from(route.method.clone()) != *method {
                continue;
            }
            let parts: SmallVec<[&str; 8]> = split_segments(path).collect();
            if let Some(params) = match_path_spans(&route.path, &parts) {
                best = Some((idx, params));
                break;
            }
        }

        best
    }
}
```

- [ ] **Step 5: Replace `match_path`'s `HashMap` with `PathParams`**

Rename `match_path` to `match_path_spans` and change its return type. The
matching logic is unchanged; only the accumulator differs:

```rust
/// Match `pattern` against pre-split `path_parts`, capturing spans.
///
/// The fallback matcher, used only for routes `matchit` would not accept. The
/// tree path does not come through here.
fn match_path_spans(pattern: &str, path_parts: &[&str]) -> Option<PathParams> {
    // ... the existing two-pass validation, unchanged ...

    let mut params = PathParams::new();
    // In the capture pass, replace
    //   params.insert(name.to_string(), value.to_string());
    // with
    //   params.push((param_intern::intern(name), Bytes::copy_from_slice(value.as_bytes())));
    // and for the catch-all segment, join the remaining parts as today but build
    // the joined string once and wrap it with Bytes::from.
    Some(params)
}
```

- [ ] **Step 6: Rewire `Router`**

```rust
pub struct Router {
    pub routes: Vec<Route>,
    /// Built on first dispatch, invalidated by `add_route`.
    ///
    /// Lazy rather than built in `add_route` because routes are registered in
    /// bulk at startup and rebuilding the trees per insertion would be O(n²) for
    /// no benefit.
    index: OnceLock<MethodIndex>,
}

impl Router {
    #[inline]
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            index: OnceLock::new(),
        }
    }

    #[inline]
    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
        // `&mut self` is what makes this sound: no dispatch can be reading the
        // index while it is replaced.
        self.index = OnceLock::new();
    }

    #[inline]
    fn index(&self) -> &MethodIndex {
        self.index.get_or_init(|| MethodIndex::build(&self.routes))
    }

    /// Match without dispatching.
    #[inline]
    pub fn match_route(&self, method: &str, path: &str) -> Option<(BoxedHandler, PathParams)> {
        let path = path.split('?').next().unwrap_or(path);
        let method = Method::from(method);
        self.index()
            .find(&self.routes, &method, path)
            .map(|(idx, params)| (self.routes[idx].handler.clone(), params))
    }

    pub async fn route(&self, mut request: HttpRequest) -> Result<HttpResponse, Error> {
        let path = request
            .path
            .as_str()
            .split('?')
            .next()
            .unwrap_or(request.path.as_str());

        // The query string is no longer parsed here — `req.query()` does it on
        // demand (Task 5).
        let matched = self.index().find(&self.routes, &request.method, path);

        if let Some((idx, params)) = matched {
            let route = &self.routes[idx];
            if let Some(constraints) = &route.constraints {
                constraints.validate(&params)?;
            }
            request.set_params(params);
            return route.handler.call(request).await;
        }

        // ... existing no-match branch, unchanged ...
    }
}
```

`Router` must keep deriving/implementing whatever it did before. `OnceLock` is
not `Clone`; if `Router` was `Clone`, implement `Clone` by hand and give the
clone a fresh `OnceLock`, exactly as `QueryCache` does in Task 5.

Every `add_route` sibling (`get`, `post`, `put`, `delete`, `patch`, `options`,
`head`, `query`) pushes directly to `self.routes` today. Change each to call
`self.add_route(..)` so the invalidation cannot be forgotten — that is what the
`routes_added_after_the_first_dispatch_are_visible` test checks.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p armature-core --features full routing`
Expected: PASS, including `armature-core/tests/routing_tests.rs`,
`route_constraints_tests.rs`, and `route_groups_tests.rs`.

- [ ] **Step 8: Run the gate**

Run `cargo test -p armature-core --features full` and
`cargo clippy -p armature-core --all-targets --features full -- -D warnings`.
Expected: green.

- [ ] **Step 9: Commit**

```bash
git add armature-core/src/routing.rs armature-core/src/route_constraint.rs
git commit -m "perf(core): dispatch through method-indexed matchit trees"
```

---

### Task 9: Build requests without eager work

**Files:**
- Modify: `armature-core/src/application.rs:1512-1600`
- Modify: `armature-core/src/micro.rs` and `armature-core/src/worker.rs` wherever
  they build an `HttpRequest`
- Test: `armature-core/tests/integration_test.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8.
- Produces: no new API. The serve path stops allocating two `HashMap`s and stops
  percent-decoding queries no handler reads.

- [ ] **Step 1: Write the failing test**

In `armature-core/tests/integration_test.rs`:

```rust
#[tokio::test]
async fn a_request_with_a_query_string_routes_and_reads_it_lazily() {
    use armature_core::{HttpRequest, HttpResponse, Router};

    let mut router = Router::new();
    router.get("/s", |req: HttpRequest| async move {
        let mut r = HttpResponse::new(200);
        // The query is read here, inside the handler — not parsed before dispatch.
        r.body = bytes::Bytes::from(req.query_param("q").unwrap_or("none").to_owned());
        r
    });

    let resp = router
        .route(HttpRequest::new("GET", "/s?q=hello%20world"))
        .await
        .unwrap();
    assert_eq!(resp.body_slice(), b"hello world");
}
```

- [ ] **Step 2: Run to verify it fails or passes for the wrong reason**

Run: `cargo test -p armature-core --features full --test integration_test`
Expected: this may already pass from Task 8. If it does, that is fine — the point
of the step is that it must be green *before* `application.rs` changes, so a
regression there is attributable.

- [ ] **Step 3: Rewrite the request construction**

In `armature-core/src/application.rs`'s `handle_request`:

```rust
    // The target is taken whole, query string included: `req.query()` slices it
    // on demand and `Router::route` splits at the '?' without allocating.
    let method = Method::from_bytes(req.method().as_str().as_bytes())
        .unwrap_or_else(|| Method::Other(ByteStr::from(req.method().as_str())));
    let target = match req.uri().query() {
        Some(q) => {
            let path = req.uri().path();
            let mut s = String::with_capacity(path.len() + 1 + q.len());
            s.push_str(path);
            s.push('?');
            s.push_str(q);
            ByteStr::from(s)
        }
        None => ByteStr::from(req.uri().path()),
    };

    // `method` is still needed as a string for the CORS check and the logs below.
    let method_token = method.as_str().to_owned();
    trace!(method = %method_token, path = %target, "Incoming request");

    // ... the existing OPTIONS/CORS short-circuit, using `method_token` ...

    let mut armature_req = HttpRequest::new(method, target);

    // No eager query parsing. This is the line the task exists to delete:
    //   armature_req.query_params = crate::simd_parser::parse_query_string_decoded(q);

    for (name, value) in req.headers() {
        // One copy per header value, because hyper's HeaderValue owns its own
        // buffer and cannot be projected into our Bytes. Plan 4 removes it by
        // parsing with armature-h1 directly; until then this is what it was
        // before, minus a String allocation per name.
        armature_req
            .headers
            .insert(name.as_str(), bytes::Bytes::copy_from_slice(value.as_bytes()));
    }
```

The `content-length` fast-path rejection below it reads
`armature_req.headers.get("content-length")`, which still returns `Option<&str>`
and needs no change. Where the body is buffered into `armature_req`, replace
`armature_req.body = collected.to_vec()` with
`armature_req.set_body_bytes(collected)` — hyper hands back `Bytes` already, so
this deletes a copy of the entire request body.

Do the same in `micro.rs` and `worker.rs`. Find them with:

```bash
grep -rn 'HttpRequest::new\|query_params\|\.body = ' --include='*.rs' \
  armature-core/src/application.rs armature-core/src/micro.rs armature-core/src/worker.rs
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p armature-core --features full`
Expected: PASS.

- [ ] **Step 5: Delete the eager parser's hot-path claim**

`simd_parser::parse_query_string_decoded` is now called by nothing on the serve
path. Keep the function — it is public API — but update its doc comment to say it
allocates a `HashMap` and that `HttpRequest::query()` is the request-path
accessor. Do not delete it in this plan; that is a separate semver decision.

- [ ] **Step 6: Run the gate**

Run `cargo test -p armature-core --features full` and
`cargo clippy -p armature-core --all-targets --features full -- -D warnings`.
Expected: green.

- [ ] **Step 7: Commit**

```bash
git add armature-core/src/application.rs armature-core/src/micro.rs \
        armature-core/src/worker.rs armature-core/src/simd_parser.rs \
        armature-core/tests/integration_test.rs
git commit -m "perf(core): stop eagerly parsing queries and copying bodies on the serve path"
```

---

### Task 10: Workspace sweep, version bump, migration note

The previous tasks kept `armature-core` green while leaving the rest of the
workspace broken. This task makes the whole workspace green again and is the one
that must not be split — a half-swept workspace does not compile.

**Files:**
- Modify: every crate that touches the changed API. Enumerate, do not guess:
  ```bash
  cargo build --workspace --features full 2>&1 \
    | grep -oE '^error(\[E[0-9]+\])?: .*' | sort | uniq -c | sort -rn | head -30
  cargo build --workspace --features full 2>&1 \
    | grep -oE '\-\-> [a-z0-9/_-]+\.rs' | sed 's#--> ##' | cut -d/ -f1 | sort | uniq -c | sort -rn
  ```
- Modify: `armature-core/Cargo.toml` (version 0.6.0), and every crate that
  declares `armature-core = { version = "0.5" }`
- Modify: `armature-core/CHANGELOG.md`, plus the `CHANGELOG.md` of each crate that
  needed source changes
- Modify: root `CHANGELOG.md` — the cross-crate migration note only
- Modify: `armature-h1/CHANGELOG.md` — create it; the crate gained public API in
  Task 1

**Interfaces:**
- Consumes: everything above. Produces no new API.

- [ ] **Step 1: Enumerate the blast radius and write it down**

Run the two commands above and record the per-crate error counts in the commit
message. The starting estimate from a pre-migration survey, for orientation only
— trust the compiler, not this table:

| Pattern | Workspace occurrences |
|---|---|
| `HttpRequest::new(...)` | 315 |
| `req.body` | 85 |
| `req.path` | 92 |
| `req.method` | 41 |
| `.query_params` | 80 |
| `.path_params` | 64 |
| `.query(` | 57 |
| `.param(` | 46 |
| `headers.get(` | 222 |
| `headers.insert(` | 228 |

- [ ] **Step 2: Sweep, in dependency order**

Work outward from `armature-core`: `armature-app`, `armature-macros`, then the
leaf crates, then `examples/`, `templates/`, and `benches/`. The substitutions are
the tables in Tasks 2–6. Two rules:

1. **Never fix a type error by adding an allocation.** `.to_string()` on a
   `&str` that the old code held as a `&String` puts back exactly what this plan
   removed. If a call site genuinely needs an owned value, it needed one before
   too — check.
2. **Never fix a test by weakening it.** A test that set `req.query_params`
   directly should now build the request with the query in its path, not assert
   less.

Fan this out if you like, but only as **edit-only work on disjoint file sets**,
with one coordinator doing the single `cargo fmt` and gate run at the end.
Concurrent formatters and concurrent `cargo` invocations on one target directory
clobber each other.

- [ ] **Step 3: Bump the version and the dependents**

```bash
sed -i 's/^version = "0.5.0"/version = "0.6.0"/' armature-core/Cargo.toml
grep -rln 'armature-core = { version = "0.5"' --include=Cargo.toml . \
  | xargs sed -i 's/armature-core = { version = "0.5"/armature-core = { version = "0.6"/'
```

Then check the templates, which pin the *framework* version rather than
`armature-core`, and any `Cargo.toml` using a different spelling
(`version = "0.5.0"`, `path` plus `version`, workspace inheritance):

```bash
grep -rn 'armature-core' --include=Cargo.toml . | grep -v '0\.6'
```

- [ ] **Step 4: Write the changelogs**

`armature-core/CHANGELOG.md`, under a new `## 0.6.0` heading:

```markdown
### Breaking

- `HttpRequest.method` is now `Method` (was `String`). Constructors take
  `impl Into<Method>`, so `HttpRequest::new("GET", …)` and
  `HttpRequest::new("GET".to_string(), …)` both still compile. Use `method_str()`
  where a `&str` is needed; `req.method == "GET"` still works.
- `HttpRequest.path` is now `ByteStr` (was `String`). It derefs to `str`, so most
  uses are unaffected; `path_str()` is explicit.
- `HttpRequest.body` and `HttpResponse.body` are now `Bytes` (were `Vec<u8>`).
  `body_slice()` returns `&[u8]`. The private `body_bytes` shadow field is gone,
  so the two can no longer disagree.
- `HttpRequest.query_params` is removed. `req.query()` returns a lazily parsed
  `Query<'_>` view and `req.query_param(name)` replaces the old
  `req.query(name)`. The query string is no longer parsed or percent-decoded
  unless a handler reads it.
- `HttpRequest.path_params` is now `SmallVec<[(&'static str, Bytes); 4]>`.
  `param(name)` returns `Option<&str>` (was `Option<&String>`); `param_bytes`
  returns the raw span.
- `HeaderMap` stores `(HeaderId, Bytes)`. `get` returns `Option<&str>` (was
  `Option<&String>`) and yields `None` for a value that is not UTF-8 — use
  `get_bytes` for those. `remove` returns `Option<Bytes>`. `iter`, `keys`,
  `names`, and `values` yield `&str`. Custom header names are lowercased at
  insert.
- `Router::match_route` returns `Option<(BoxedHandler, PathParams)>` (was
  `HashMap<String, String>`).
- `RouteConstraints::validate` takes `&PathParams`.

### Performance

- Method dispatch goes through one `matchit` tree per method instead of a linear
  scan with a per-candidate method string comparison. Routes that `matchit`
  rejects (conflicting or duplicate patterns) fall back to the old scan, so
  first-registered-wins still holds.
- `Extensions` is a `SmallVec` of eight inline slots instead of a `HashMap`.
- The serve path no longer copies the request body out of hyper's `Bytes`.

### Migration

`tokio::spawn` inside handlers is unaffected by this release. The `Send`-bound
change is 0.7 (Plan 3).
```

Root `CHANGELOG.md` gets a short pointer to `armature-core/CHANGELOG.md`'s 0.6.0
section and nothing more — per-crate changelogs are where the detail lives.

Create `armature-h1/CHANGELOG.md` with a `## 0.1.0` section listing the crate's
initial surface plus the `From` impls and `header::intern` added in Task 1.

- [ ] **Step 5: Run the full gate, workspace-wide**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo clippy --all-targets --features full-with-saml -- -D warnings
cargo test --features full
cargo test --doc --features full
cargo test -p armature-h1 --all-features
```

Doc tests matter more than usual here: the changed types appear in dozens of
`///` examples, and a doc test is the only thing that compiles them.

- [ ] **Step 6: Check the crates CI does not build with real features**

CI builds 24 of 62 members with `--all-features`; the rest get default features
only, so a break in one of them can pass CI. Before committing, run the
`test-members` matrix crates plus the ones this sweep touched:

```bash
for c in $(grep -oE '^\s+- crate: (armature-[a-z0-9-]+)' .github/workflows/ci.yml \
           | awk '{print $3}'); do
  echo "== $c"; cargo test -p "$c" --all-features 2>&1 | tail -3
done
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor!: migrate the workspace onto armature-core 0.6's Bytes-backed types"
```

---

### Task 11: Prove the allocations are gone

Without this, "kills ≈25 allocations" is a sentence in a spec.

**Files:**
- Create: `armature-core/tests/alloc_core.rs`
- Create: `armature-core/benches/router.rs`
- Modify: `armature-core/Cargo.toml` (`[[bench]]`, criterion dev-dependency)

**Interfaces:**
- Consumes: everything above.
- Produces: `alloc_core.rs` asserting per-request allocation *counts* against a
  named budget, and a router benchmark comparing method-indexed dispatch against a
  linear scan.

- [ ] **Step 1: Write the failing test**

`armature-core/tests/alloc_core.rs`:

```rust
//! Allocation budget for the migrated request path.
//!
//! Unlike `armature-h1`'s `alloc_regression.rs`, this asserts a *budget* rather
//! than zero. `armature-core` still allocates on this path by construction —
//! hyper's types, `Arc`-based DI, boxed middleware futures — and Plans 3 and 4
//! are what remove those. What must hold now is that the migrated types no longer
//! contribute, so the budget is set at the measured value and comes down as the
//! later plans land. A number that only ever goes down is worth having; a
//! threshold nobody revisits is not, so each constant below says what it covers.

use armature_core::{HttpRequest, HttpResponse, Router};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn count<T>(f: impl FnOnce() -> T) -> u64 {
    ALLOCS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let out = f();
    COUNTING.store(false, Ordering::SeqCst);
    drop(out);
    ALLOCS.load(Ordering::SeqCst)
}

/// Constructing a request from static strings: the `ByteStr` copies of the
/// method token and target, and nothing else. `HeaderMap`, `PathParams`, and
/// `Extensions` are all inline, and `QueryCache` is cold.
const BUDGET_CONSTRUCT: u64 = 2;

#[test]
fn constructing_a_request_costs_only_its_target() {
    let n = count(|| HttpRequest::new("GET", "/users/42"));
    println!("construct: {n} allocations");
    assert!(
        n <= BUDGET_CONSTRUCT,
        "constructing a request cost {n} allocations, budget is {BUDGET_CONSTRUCT}"
    );
}

/// Reading a query the handler ignores must cost nothing at all.
#[test]
fn an_unread_query_string_costs_nothing() {
    let req = HttpRequest::new("GET", "/s?a=1&b=2&c=hello%20world");
    let n = count(|| req.headers.len());
    println!("unread query: {n} allocations");
    assert_eq!(
        n, 0,
        "a query string no handler reads must not be parsed or decoded"
    );
}

/// Six typical request headers, all with well-known names.
///
/// One `Bytes` copy per value and zero for the names — `HeaderId` interning
/// resolves all six to enum variants, and the `SmallVec` holds them inline.
const BUDGET_SIX_HEADERS: u64 = 6;

#[test]
fn well_known_header_names_cost_no_allocation() {
    let mut req = HttpRequest::new("GET", "/");
    let n = count(|| {
        req.headers.insert("host", "a.example");
        req.headers.insert("accept", "*/*");
        req.headers.insert("accept-encoding", "gzip");
        req.headers.insert("user-agent", "curl/8");
        req.headers.insert("connection", "keep-alive");
        req.headers.insert("content-length", "0");
    });
    println!("six headers: {n} allocations");
    assert!(
        n <= BUDGET_SIX_HEADERS,
        "six headers cost {n} allocations, budget is {BUDGET_SIX_HEADERS}: \
         interning a well-known name must not allocate"
    );
}

#[test]
fn cloning_a_request_does_not_copy_its_body_or_target() {
    let mut req = HttpRequest::new("POST", "/upload");
    req.set_body_bytes(bytes::Bytes::from(vec![0u8; 1024 * 1024]));
    let n = count(|| req.clone());
    println!("clone: {n} allocations");
    // A megabyte body and a target, cloned for the price of refcounts.
    assert!(n <= 1, "cloning a request cost {n} allocations");
}

#[tokio::test]
async fn dispatch_allocates_only_for_captured_params() {
    let mut router = Router::new();
    router.get("/users/:id", |_req: HttpRequest| async { HttpResponse::new(200) });
    // Warm the lazily built index outside the count.
    let _ = router.route(HttpRequest::new("GET", "/users/1")).await;

    let n = count(|| {
        futures::executor::block_on(router.route(HttpRequest::new("GET", "/users/42")))
    });
    println!("dispatch: {n} allocations");
    // Budget, not zero: the boxed handler future is B4/B3 work in Plan 3. What
    // this pins down is that dispatch no longer builds a HashMap per match.
    assert!(n <= 8, "dispatch cost {n} allocations");
}
```

- [ ] **Step 2: Run it and record the real numbers**

Run: `cargo test -p armature-core --features full --test alloc_core -- --nocapture --test-threads=1`

`--test-threads=1` is not optional: the counter is a process-wide static, and
parallel tests would count each other's allocations.

Expected: FAIL on at least one budget, with the printed numbers showing the
actual cost. Set each `BUDGET_*` to the measured value **only after checking the
number is explicable** — if `well_known_header_names_cost_no_allocation` reports
12 rather than 6, something is allocating per name and the interning path needs
fixing, not the budget raising.

- [ ] **Step 3: Write the router benchmark**

`armature-core/benches/router.rs`:

```rust
//! Router dispatch: method-indexed trees against the linear scan they replaced.
//!
//! The interesting axis is route-table size. A linear scan is competitive at
//! four routes and hopeless at four hundred; a tree is flat. Both are measured so
//! the claim is a curve rather than one number.

use armature_core::{HttpRequest, HttpResponse, Router};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

fn router_with(n: usize) -> Router {
    let mut r = Router::new();
    for i in 0..n {
        // Leaked so the pattern can be a `&'static str`-shaped `String` at
        // registration time; this is a benchmark harness, not a serve path.
        let path: &'static str = Box::leak(format!("/route{i}/:id").into_boxed_str());
        r.get(path, |_req: HttpRequest| async { HttpResponse::new(200) });
    }
    r
}

fn bench_match(c: &mut Criterion) {
    let mut g = c.benchmark_group("router/match_route");
    for n in [4usize, 32, 128, 512] {
        let router = router_with(n);
        // Match the *last* registered route: the worst case for a linear scan and
        // the same case as any other for a tree.
        let target = format!("/route{}/42", n - 1);
        g.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let m = router.match_route("GET", black_box(&target));
                assert!(m.is_some());
                m
            })
        });
    }
    g.finish();
}

fn bench_method_miss(c: &mut Criterion) {
    // A method with no routes registered: the tree array short-circuits, where a
    // scan compared every route's method string.
    let router = router_with(128);
    c.bench_function("router/method_miss", |b| {
        b.iter(|| router.match_route("DELETE", black_box("/route0/1")))
    });
}

criterion_group!(benches, bench_match, bench_method_miss);
criterion_main!(benches);
```

In `armature-core/Cargo.toml`:

```toml
[dev-dependencies]
criterion = { version = "0.8", features = ["html_reports"] }
futures = "0.3"

[[bench]]
name = "router"
harness = false
```

- [ ] **Step 4: Run the benchmark**

Run: `cargo bench -p armature-core --bench router -- --warm-up-time 0.5 --measurement-time 2`
Expected: completes; `match_route` is roughly flat across 4→512 routes. If it is
not flat, the fallback list is absorbing routes that should be in the trees —
check `translate_pattern` against the generated patterns.

- [ ] **Step 5: Run the full gate**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --features full -- -D warnings
cargo test --features full
cargo test -p armature-core --features full --test alloc_core -- --test-threads=1
```
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add armature-core/tests/alloc_core.rs armature-core/benches/router.rs armature-core/Cargo.toml
git commit -m "test(core): allocation budget and router dispatch benchmark"
```

---

## Self-Review

**Spec coverage.**

| Spec item | Task |
|---|---|
| B1 — `method: Method` | 3 |
| B1 — `path: ByteStr` | 4 |
| B1 — `body: Bytes` on request and response | 4 |
| B1 — `HeaderMap` stores `(HeaderId, Bytes)`, keeps its `&str` facade | 2 |
| B1 — `method_str()` / `body_slice()` escape hatches | 3, 4 |
| B1 — generic constructors so existing call sites compile | 1, 3, 4 |
| B2 — lazy, memoized query view | 5 |
| B2 — span-based path params with `&'static str` names | 6 |
| B2 — delete the eager percent-decode at `application.rs:1545` | 9 |
| B7 — method-indexed `matchit` trees | 8 |
| B8 — `Extensions` as a `SmallVec` | 7 |
| Workspace-wide mechanical fixups | 10 |
| Semver bump 0.5 → 0.6 and migration note | 10 |
| Allocation claims verified | 11 |

**Deliberate deviations from the spec, and why.**

- **B8 keeps `Arc<dyn Any + Send + Sync>`; it does not become `Rc<dyn Any>`.** The
  spec says the `Send + Sync` bound "drops out, following B5" — and B5 is Plan 3.
  Doing it here would break every extractor, which holds `&HttpRequest` across an
  `await` inside a `Send` future. The `SmallVec` half of B8, which is the part
  that removes the hashing and the allocation, lands here in full.
- **`HttpRequest` keeps a `OnceLock`, not a `OnceCell`.** Same reason. Noted in
  the code so Plan 3 can swap it.
- **The spec's "205 existing call sites" and "roughly 30" for B2 are both low.** A
  fresh count is in Task 10: 315 `HttpRequest::new`, 80 `query_params`, 64
  `path_params`, 222 `headers.get`. The plan does not depend on the numbers, but
  an implementer sizing Task 10 against the spec's figures would be surprised.
- **`Router` gains a linear fallback for patterns `matchit` rejects.** The spec
  calls B7 "near-non-breaking — only `Router` internals change", which is only
  true with this fallback: `matchit` errors on conflicting inserts, while the
  current linear scan accepts duplicates and takes the first. Dropping the loser
  would silently change which handler serves a live path.
- **`LazyHeaders` (response headers) is untouched.** B1 names
  `HttpResponse.body`, not its header map. Migrating `LazyHeaders` to
  `(HeaderId, Bytes)` is worth doing, but it belongs with B6's `write_into` in
  Plan 3, where the response write path is already being rebuilt.
- **`matchit` was already a declared dependency and entirely unused.** B7 is
  therefore the first code to use it; no dependency is added.

**Placeholder scan.** No `TODO`, `TBD`, "similar to Task N", or "add error
handling" steps. Every code step carries the code. Two steps deliberately hand
over a *measured* value rather than a literal — Task 11's `BUDGET_*` constants and
Task 10's error enumeration — and both say how to obtain it and how to tell a
legitimate number from a regression being papered over.

**Type consistency.**

- `Method` is `armature_h1::Method` throughout, re-exported as
  `armature_core::Method`. `HttpMethod` remains the routing enum, converted at the
  boundary in `MethodIndex::build`.
- `Method::as_str()` is the name used in Tasks 1, 3, 8, and 9. Task 3 Step 3
  flags that `armature-h1` may already have `as_str` for this and says to keep one
  name — resolve it there, then use that name in the later tasks.
- `PathParams` is `SmallVec<[(&'static str, Bytes); 4]>`, defined in
  `http.rs` (Task 6), consumed by `routing.rs` (Task 8) and
  `route_constraint.rs` (Task 6 Step 6).
- `QueryPairs` is `SmallVec<[(String, String); 8]>` and lives in `query.rs`;
  `Query<'a>` borrows `&'a [(String, String)]` from the `OnceLock`. The pairs are
  `String` rather than `ByteStr` because percent-decoding produces owned bytes for
  any value that needs it, and a `ByteStr` that sometimes borrows the target and
  sometimes owns a decoded copy would be the same type doing two jobs.
- `HeaderValueInput` is the insert-side trait in `headers.rs` (Task 2), used by
  `application.rs` (Task 9).
- `header_id::intern(&str) -> HeaderId` (Task 1) is header *names*;
  `param_intern::intern(&str) -> &'static str` (Task 6) is route parameter names.
  Two interners, two jobs, deliberately different return types — do not merge
  them.

**Out of scope, covered by later plans.** B3 (arena middleware futures), B4
(`MaybeReady`), B5 (`!Send` handlers, `spawn`/`spawn_shared`, the DI slab), B6
(`write_into`) — Plan 3. The serve-path swap onto `armature-h1` and the
zero-allocation end-to-end regression test — Plan 4. The `armature-websocket`
upgrade adapter — Plan 5.
