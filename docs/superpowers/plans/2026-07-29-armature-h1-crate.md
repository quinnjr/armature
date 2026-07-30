# armature-h1 Crate Implementation Plan (Plan 1 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `armature-h1`, a standalone thread-per-core HTTP/1.1 server crate with zero heap allocations on the steady-state keep-alive path, full RFC 9112 framing correctness, and benchmarks proving it against the current hyper path — with no `armature-*` dependency.

**Architecture:** N pinned OS threads, each running a `current_thread` tokio runtime with its own `SO_REUSEPORT` listener. Request heads parse via `httparse` into `Bytes` slices of a per-core pooled `BytesMut`, so header values and bodies are refcount bumps rather than `String` allocations. Framing decisions (`Content-Length` vs `Transfer-Encoding`) live in one pure, exhaustively-tested function; every rejection closes the connection rather than resynchronizing.

**Tech Stack:** Rust edition 2024, MSRV 1.94.1, tokio (current-thread), httparse, memchr, bytes, smallvec, socket2, core_affinity, httpdate, thiserror, tracing. `rustls` + `tokio-rustls` behind a `tls` feature. `#![forbid(unsafe_code)]`.

**Spec:** `docs/superpowers/specs/2026-07-29-armature-h1-design.md`

## Global Constraints

- Crate lives at `armature-h1/`, registered in the root `Cargo.toml` `[workspace] members` list and in `.github/workflows/ci.yml`'s per-crate matrix.
- `edition.workspace = true`, `rust-version.workspace = true`, `version = "0.1.0"`, plus `authors`/`license`/`repository`/`homepage` from `[workspace.package]`.
- `#![forbid(unsafe_code)]` at the top of `lib.rs`. No exceptions; vectorization comes from `httparse` and `memchr`.
- **No `armature-*` dependency.** `armature-core` will depend on this crate, never the reverse.
- Targeted specs: RFC 9110, RFC 9111, RFC 9112. Where RFC 7230 differs, 9112 wins.
- Line endings in the message head are **strict CRLF**. Bare CR and bare LF are both rejected with 400 + close. RFC 9112 §2.2 permits accepting bare LF; we decline, because a reject-and-close policy is the request-smuggling defense and leniency is where smuggling lives.
- Every framing rejection closes the connection. Never attempt to resynchronize a stream whose framing is in doubt.
- Handler/service futures are **not** `Send`. Nothing in this crate may require `Send` on a service future.
- Pre-commit gate (runs automatically): `cargo fmt -- --check` then `cargo clippy --workspace --all-targets --features full-with-saml -- -D warnings`. Both must pass before any commit lands.
- Per-task test command: `cargo test -p armature-h1`. Full gate before the final commit: `cargo test -p armature-h1 --all-features`.

## File Structure

| File | Responsibility |
|---|---|
| `armature-h1/Cargo.toml` | Deps, features (`tls`, `default = []`) |
| `armature-h1/src/lib.rs` | `forbid(unsafe_code)`, module wiring, public re-exports |
| `armature-h1/src/bytestr.rs` | `ByteStr` — `Bytes` with a UTF-8 invariant |
| `armature-h1/src/method.rs` | `Method`, `Version` |
| `armature-h1/src/header.rs` | `HeaderId` interning, `HeaderVec` |
| `armature-h1/src/head.rs` | `Head` struct + header accessors, path/query split |
| `armature-h1/src/limits.rs` | `Limits` with defaults |
| `armature-h1/src/parse.rs` | CRLF/obs-fold prescan, `parse_head` via httparse |
| `armature-h1/src/framing.rs` | `BodyKind`, `decide()` — the security core |
| `armature-h1/src/chunked.rs` | Chunked decoder + trailer parsing |
| `armature-h1/src/pool.rs` | Per-core `BufPool` over `BytesMut::try_reclaim` |
| `armature-h1/src/write.rs` | Status line, header serialization, `DateCache`, chunked encoder |
| `armature-h1/src/deadline.rs` | `ConnDeadline` — one reusable coarsened `Sleep` per connection |
| `armature-h1/src/service.rs` | `Request`, `Response`, `Body`, `H1Service`, `Transport`, `Upgraded` |
| `armature-h1/src/conn.rs` | Connection state machine: keep-alive, pipelining, 100-continue, HEAD, upgrade |
| `armature-h1/src/server.rs` | `bind`, `SO_REUSEPORT` sharding, core pinning, graceful shutdown |
| `armature-h1/src/tls.rs` | `tls` feature: rustls acceptor, ALPN + h2c dispatch to `H2Fallback` |
| `armature-h1/tests/rfc9112/*.rs` | Conformance over real socket pairs |
| `armature-h1/fuzz/` | Parser fuzz + differential-vs-hyper framing fuzz |
| `armature-h1/benches/` | criterion micro-benches |
| `armature-h1/tests/alloc_regression.rs` | Counting allocator: zero allocs for a keep-alive GET |

Tasks 1–11 are pure, synchronous, unit-testable units with no I/O. Task 12 onward introduces sockets. This ordering is deliberate: the highest-risk code (parsing, framing) is fully tested before any async complexity exists to hide bugs behind.

---

### Task 1: Crate scaffold and `ByteStr`

**Files:**
- Create: `armature-h1/Cargo.toml`, `armature-h1/src/lib.rs`, `armature-h1/src/bytestr.rs`
- Modify: `Cargo.toml` (workspace members), `.github/workflows/ci.yml` (per-crate matrix)
- Test: inline `#[cfg(test)] mod tests` in `armature-h1/src/bytestr.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `ByteStr` with `from_utf8(Bytes) -> Result<Self, Utf8Error>`, `from_static(&'static str) -> Self`, `as_str(&self) -> &str`, `as_bytes(&self) -> &[u8]`, `into_bytes(self) -> Bytes`, `Deref<Target = str>`, `Clone`, `Debug`, `Display`, `PartialEq<str>`, `PartialEq<&str>`, `Eq`, `Hash`.

- [ ] **Step 1: Create the crate manifest**

`armature-h1/Cargo.toml`:

```toml
[package]
name = "armature-h1"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Zero-allocation thread-per-core HTTP/1.1 server for the Armature framework"
keywords = ["http", "server", "http1", "performance", "async"]
categories = ["web-programming::http-server", "asynchronous", "network-programming"]

[dependencies]
bytes = "1.12"
httparse = "1.10"
memchr = "2.8"
smallvec = { version = "1.15", features = ["union", "const_generics"] }
httpdate = "1.0"
thiserror = "2.0"
tracing = "0.1"
tokio = { version = "1.52", features = ["rt", "net", "io-util", "time", "sync", "macros"] }
socket2 = { version = "0.6", features = ["all"] }
core_affinity = "0.8"

rustls = { version = "0.23", features = ["ring"], optional = true }
tokio-rustls = { version = "0.26", optional = true }

[features]
default = []
tls = ["dep:rustls", "dep:tokio-rustls"]

[dev-dependencies]
tokio = { version = "1.52", features = ["rt", "net", "io-util", "time", "sync", "macros", "test-util"] }
```

- [ ] **Step 2: Register the crate in the workspace and CI**

In the root `Cargo.toml`, add `"armature-h1",` to `[workspace] members` immediately after `"armature-core",`.

In `.github/workflows/ci.yml`, add a row to the per-crate matrix (the one containing `- crate: armature-core`) so the `tls` feature is actually exercised — without this the feature never builds in CI:

```yaml
          # armature-h1 gates its rustls acceptor and ALPN/h2c dispatch behind
          # the `tls` feature, which no root feature reaches. `default = []`,
          # so minimal is skipped as identical to the plain default build.
          - crate: armature-h1
            minimal: ""
```

- [ ] **Step 3: Write the failing test**

`armature-h1/src/bytestr.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_utf8_accepts_valid() {
        let s = ByteStr::from_utf8(Bytes::from_static(b"/index.html")).unwrap();
        assert_eq!(s.as_str(), "/index.html");
        assert_eq!(&s, "/index.html");
        assert_eq!(s.len(), 11);
    }

    #[test]
    fn from_utf8_rejects_invalid() {
        assert!(ByteStr::from_utf8(Bytes::from_static(&[0xff, 0xfe])).is_err());
    }

    #[test]
    fn from_static_is_cheap_and_correct() {
        let s = ByteStr::from_static("GET");
        assert_eq!(s.as_str(), "GET");
    }

    /// The load-bearing property: a ByteStr carved out of a larger buffer
    /// shares the allocation rather than copying.
    #[test]
    fn shares_the_parent_allocation() {
        let parent = Bytes::from_static(b"GET /a/b HTTP/1.1");
        let target = ByteStr::from_utf8(parent.slice(4..8)).unwrap();
        assert_eq!(target.as_str(), "/a/b");
        assert_eq!(parent.len(), 17, "parent must be untouched");
    }

    #[test]
    fn deref_gives_str_methods() {
        let s = ByteStr::from_static("/a/b?x=1");
        assert!(s.starts_with("/a"));
        assert_eq!(s.split('?').next(), Some("/a"));
    }

    #[test]
    fn empty_is_valid() {
        let s = ByteStr::from_utf8(Bytes::new()).unwrap();
        assert!(s.is_empty());
    }
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p armature-h1 bytestr`
Expected: FAIL — `cannot find type ByteStr in this scope`.

- [ ] **Step 5: Implement `ByteStr`**

Prepend to `armature-h1/src/bytestr.rs`:

```rust
//! A `Bytes` handle carrying a UTF-8 invariant.
//!
//! This is the crate's string type. It exists so that a request target or
//! header value can be a slice of the connection's read buffer — a refcount
//! bump — rather than a freshly allocated `String`.

use bytes::Bytes;
use std::fmt;
use std::ops::Deref;
use std::str::Utf8Error;

/// An immutable UTF-8 string backed by [`Bytes`].
///
/// Cloning is a refcount increment. Slicing a `ByteStr` out of a larger
/// buffer does not copy.
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct ByteStr(Bytes);

impl ByteStr {
    /// Wrap `bytes`, validating UTF-8.
    #[inline]
    pub fn from_utf8(bytes: Bytes) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;
        Ok(Self(bytes))
    }

    /// Wrap a static string without allocating.
    #[inline]
    pub fn from_static(s: &'static str) -> Self {
        Self(Bytes::from_static(s.as_bytes()))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        // Invariant established in `from_utf8`; `from_static` is UTF-8 by type.
        // Checked rather than unchecked because the crate forbids unsafe code,
        // and this validation is amortized away by the branch predictor.
        std::str::from_utf8(&self.0).unwrap_or("")
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[inline]
    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Deref for ByteStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for ByteStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for ByteStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl PartialEq<str> for ByteStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl PartialEq<&str> for ByteStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}
```

> **Note on `as_str`:** the checked `from_utf8` here is deliberate. The unchecked
> variant requires `unsafe`, which this crate forbids, and the validation is a
> length-proportional scan that the optimizer hoists out of hot loops. If a
> benchmark in Task 17 shows this specific call on the critical path, revisit it
> then with data — not now on speculation.

- [ ] **Step 6: Create `lib.rs`**

`armature-h1/src/lib.rs`:

```rust
//! Zero-allocation thread-per-core HTTP/1.1 server.
//!
//! See `docs/superpowers/specs/2026-07-29-armature-h1-design.md`.
//!
//! # Design
//!
//! Request heads are parsed into [`Bytes`](bytes::Bytes) slices of a per-core
//! pooled read buffer, so header values and bodies cost a refcount increment
//! rather than an allocation. Framing decisions live in one pure function
//! ([`framing::decide`]) and every rejection closes the connection rather than
//! resynchronizing the stream.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod bytestr;

pub use bytestr::ByteStr;
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 6 tests.

- [ ] **Step 8: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add armature-h1 Cargo.toml .github/workflows/ci.yml
git commit -m "feat(h1): crate scaffold and ByteStr

New armature-h1 crate: thread-per-core HTTP/1.1 server, forbid(unsafe_code).
ByteStr is Bytes plus a UTF-8 invariant, so request targets and header values
are slices of the connection read buffer rather than String allocations.

Registered in the workspace and in CI's per-crate matrix so the tls feature
is actually exercised."
```

---

### Task 2: `Method` and `Version`

**Files:**
- Create: `armature-h1/src/method.rs`
- Modify: `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/method.rs`

**Interfaces:**
- Consumes: `ByteStr` from Task 1.
- Produces:
  - `enum Method { Get, Head, Post, Put, Delete, Connect, Options, Trace, Patch, Query, Other(ByteStr) }`
  - `Method::from_bytes(token: &[u8]) -> Option<Method>` — `None` means "not well-known"; the caller builds `Other` from the buffer so this function needs no `Bytes`.
  - `Method::as_str(&self) -> &str`, `Method::is_safe(&self) -> bool`, `Method::expects_response_body(&self) -> bool` (false for `Head`).
  - `enum Version { Http10, Http11 }` with `as_bytes(&self) -> &'static [u8]`, `Version::from_httparse(u8) -> Option<Version>`.

`Query` is included because Armature supports the HTTP QUERY method end-to-end, including body-keyed response caching; omitting it here would silently downgrade that to `Other`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_methods_parse() {
        assert_eq!(Method::from_bytes(b"GET"), Some(Method::Get));
        assert_eq!(Method::from_bytes(b"HEAD"), Some(Method::Head));
        assert_eq!(Method::from_bytes(b"POST"), Some(Method::Post));
        assert_eq!(Method::from_bytes(b"PUT"), Some(Method::Put));
        assert_eq!(Method::from_bytes(b"DELETE"), Some(Method::Delete));
        assert_eq!(Method::from_bytes(b"CONNECT"), Some(Method::Connect));
        assert_eq!(Method::from_bytes(b"OPTIONS"), Some(Method::Options));
        assert_eq!(Method::from_bytes(b"TRACE"), Some(Method::Trace));
        assert_eq!(Method::from_bytes(b"PATCH"), Some(Method::Patch));
        assert_eq!(Method::from_bytes(b"QUERY"), Some(Method::Query));
    }

    /// Methods are case-sensitive per RFC 9110 section 9.1.
    #[test]
    fn methods_are_case_sensitive() {
        assert_eq!(Method::from_bytes(b"get"), None);
        assert_eq!(Method::from_bytes(b"Get"), None);
    }

    #[test]
    fn unknown_method_is_not_well_known() {
        assert_eq!(Method::from_bytes(b"PROPFIND"), None);
        assert_eq!(Method::from_bytes(b""), None);
        assert_eq!(Method::from_bytes(b"GETX"), None);
        assert_eq!(Method::from_bytes(b"GE"), None);
    }

    #[test]
    fn as_str_round_trips() {
        for m in [
            Method::Get, Method::Head, Method::Post, Method::Put,
            Method::Delete, Method::Connect, Method::Options,
            Method::Trace, Method::Patch, Method::Query,
        ] {
            assert_eq!(Method::from_bytes(m.as_str().as_bytes()), Some(m.clone()));
        }
        assert_eq!(Method::Other(ByteStr::from_static("PROPFIND")).as_str(), "PROPFIND");
    }

    #[test]
    fn head_expects_no_response_body() {
        assert!(!Method::Head.expects_response_body());
        assert!(Method::Get.expects_response_body());
    }

    #[test]
    fn safe_methods_classified() {
        assert!(Method::Get.is_safe());
        assert!(Method::Head.is_safe());
        assert!(Method::Options.is_safe());
        assert!(Method::Trace.is_safe());
        assert!(Method::Query.is_safe());
        assert!(!Method::Post.is_safe());
        assert!(!Method::Delete.is_safe());
        assert!(!Method::Other(ByteStr::from_static("PROPFIND")).is_safe());
    }

    #[test]
    fn versions_map_from_httparse() {
        assert_eq!(Version::from_httparse(0), Some(Version::Http10));
        assert_eq!(Version::from_httparse(1), Some(Version::Http11));
        assert_eq!(Version::from_httparse(2), None);
        assert_eq!(Version::Http11.as_bytes(), b"HTTP/1.1");
        assert_eq!(Version::Http10.as_bytes(), b"HTTP/1.0");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p armature-h1 method`
Expected: FAIL — `cannot find type Method in this scope`.

- [ ] **Step 3: Implement `Method` and `Version`**

Prepend to `armature-h1/src/method.rs`:

```rust
//! Request method and protocol version.

use crate::ByteStr;

/// An HTTP request method.
///
/// Well-known methods are unit variants, so dispatch is a discriminant
/// comparison rather than a string comparison. Unrecognized methods carry a
/// [`ByteStr`] slice of the read buffer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `HEAD`
    Head,
    /// `POST`
    Post,
    /// `PUT`
    Put,
    /// `DELETE`
    Delete,
    /// `CONNECT`
    Connect,
    /// `OPTIONS`
    Options,
    /// `TRACE`
    Trace,
    /// `PATCH`
    Patch,
    /// `QUERY` (RFC 9110-style safe method with a body).
    Query,
    /// Any other valid method token.
    Other(ByteStr),
}

impl Method {
    /// Match a method token against the well-known set.
    ///
    /// Returns `None` when the token is not well-known; the caller then builds
    /// [`Method::Other`] from the read buffer. Keeping `Bytes` out of this
    /// signature is what makes the function trivially unit-testable.
    ///
    /// Methods are case-sensitive (RFC 9110 section 9.1), so this compares
    /// exactly. Dispatching on length first means most calls do one integer
    /// comparison and one 3-or-4-byte memcmp.
    #[inline]
    pub fn from_bytes(token: &[u8]) -> Option<Method> {
        match token.len() {
            3 => match token {
                b"GET" => Some(Method::Get),
                b"PUT" => Some(Method::Put),
                _ => None,
            },
            4 => match token {
                b"HEAD" => Some(Method::Head),
                b"POST" => Some(Method::Post),
                _ => None,
            },
            5 => match token {
                b"PATCH" => Some(Method::Patch),
                b"TRACE" => Some(Method::Trace),
                b"QUERY" => Some(Method::Query),
                _ => None,
            },
            6 => match token {
                b"DELETE" => Some(Method::Delete),
                _ => None,
            },
            7 => match token {
                b"CONNECT" => Some(Method::Connect),
                b"OPTIONS" => Some(Method::Options),
                _ => None,
            },
            _ => None,
        }
    }

    /// The method token as a string.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
            Method::Connect => "CONNECT",
            Method::Options => "OPTIONS",
            Method::Trace => "TRACE",
            Method::Patch => "PATCH",
            Method::Query => "QUERY",
            Method::Other(s) => s.as_str(),
        }
    }

    /// Whether this method is safe per RFC 9110 section 9.2.1.
    ///
    /// Unrecognized methods are conservatively treated as unsafe.
    #[inline]
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            Method::Get | Method::Head | Method::Options | Method::Trace | Method::Query
        )
    }

    /// Whether a response to this method may carry a body.
    ///
    /// `HEAD` responses carry headers only, including the `Content-Length`
    /// the equivalent `GET` would have produced (RFC 9112 section 6.3).
    #[inline]
    pub fn expects_response_body(&self) -> bool {
        !matches!(self, Method::Head)
    }
}

/// The HTTP/1 protocol version of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Version {
    /// `HTTP/1.0` — connections close by default.
    Http10,
    /// `HTTP/1.1` — connections persist by default.
    Http11,
}

impl Version {
    /// Map `httparse`'s minor-version byte.
    ///
    /// Anything other than 0 or 1 is not HTTP/1.x and must be answered with
    /// 505 rather than guessed at.
    #[inline]
    pub fn from_httparse(minor: u8) -> Option<Version> {
        match minor {
            0 => Some(Version::Http10),
            1 => Some(Version::Http11),
            _ => None,
        }
    }

    /// The version token for a status line.
    #[inline]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            Version::Http10 => b"HTTP/1.0",
            Version::Http11 => b"HTTP/1.1",
        }
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Add to `armature-h1/src/lib.rs`, after `mod bytestr;`:

```rust
mod method;
```

and extend the re-exports:

```rust
pub use method::{Method, Version};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 13 tests.

- [ ] **Step 6: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add armature-h1/src/method.rs armature-h1/src/lib.rs
git commit -m "feat(h1): Method and Version enums

Well-known methods are unit variants dispatched on token length, so routing
compares a discriminant instead of a string. QUERY is included because
Armature supports it end-to-end. Unknown methods carry a ByteStr slice of the
read buffer.

Methods are case-sensitive per RFC 9110 9.1; versions other than HTTP/1.0 and
1.1 map to None so the caller answers 505 rather than guessing."
```

---

### Task 3: `HeaderId` interning and `HeaderVec`

**Files:**
- Create: `armature-h1/src/header.rs`
- Modify: `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/header.rs`

**Interfaces:**
- Consumes: `ByteStr` from Task 1.
- Produces:
  - `enum HeaderId { Host, ContentLength, ContentType, TransferEncoding, Connection, Expect, Upgrade, Date, Server, Accept, AcceptEncoding, AcceptLanguage, Authorization, CacheControl, Cookie, SetCookie, Etag, IfNoneMatch, IfModifiedSince, LastModified, Location, Referer, UserAgent, Vary, Allow, Trailer, Te, ContentEncoding, ContentLanguage, Range, IfMatch, IfUnmodifiedSince, Origin, Other(ByteStr) }`
  - `HeaderId::from_bytes(name: &[u8]) -> Option<HeaderId>` — case-insensitive; `None` means not well-known.
  - `HeaderId::as_str(&self) -> &str` — canonical casing for well-known names.
  - `HeaderId::is_hop_by_hop(&self) -> bool` — `Connection`, `TransferEncoding`, `Te`, `Trailer`, `Upgrade`.
  - `HeaderId::forbidden_in_trailers(&self) -> bool` — framing and routing headers that RFC 9110 §6.5.1 forbids in a trailer section.
  - `type HeaderVec = SmallVec<[(HeaderId, Bytes); 16]>`
  - `fn get<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a Bytes>`
  - `fn get_str<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a str>` — `None` if the value is not UTF-8.
  - `fn all<'a>(v: &'a HeaderVec, id: &'a HeaderId) -> impl Iterator<Item = &'a Bytes> + 'a`
  - `fn count(v: &HeaderVec, id: &HeaderId) -> usize`

Interning is the second independent win in this design: `armature-core`'s current `HeaderMap` performs a case-insensitive string comparison per lookup, which this reduces to a discriminant comparison.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn well_known_names_intern_case_insensitively() {
        assert_eq!(HeaderId::from_bytes(b"host"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"Host"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"HOST"), Some(HeaderId::Host));
        assert_eq!(HeaderId::from_bytes(b"hOsT"), Some(HeaderId::Host));
        assert_eq!(
            HeaderId::from_bytes(b"content-length"),
            Some(HeaderId::ContentLength)
        );
        assert_eq!(
            HeaderId::from_bytes(b"Transfer-Encoding"),
            Some(HeaderId::TransferEncoding)
        );
        assert_eq!(HeaderId::from_bytes(b"TE"), Some(HeaderId::Te));
    }

    #[test]
    fn unknown_names_are_not_well_known() {
        assert_eq!(HeaderId::from_bytes(b"x-request-id"), None);
        assert_eq!(HeaderId::from_bytes(b""), None);
        // A prefix of a known name must not match it.
        assert_eq!(HeaderId::from_bytes(b"hos"), None);
        assert_eq!(HeaderId::from_bytes(b"hostx"), None);
    }

    #[test]
    fn as_str_uses_canonical_casing_and_round_trips() {
        assert_eq!(HeaderId::Host.as_str(), "host");
        assert_eq!(HeaderId::ContentLength.as_str(), "content-length");
        assert_eq!(HeaderId::Te.as_str(), "te");
        for id in HeaderId::WELL_KNOWN {
            assert_eq!(
                HeaderId::from_bytes(id.as_str().as_bytes()).as_ref(),
                Some(id),
                "{} failed to round-trip",
                id.as_str()
            );
        }
    }

    #[test]
    fn hop_by_hop_classified() {
        assert!(HeaderId::Connection.is_hop_by_hop());
        assert!(HeaderId::TransferEncoding.is_hop_by_hop());
        assert!(HeaderId::Te.is_hop_by_hop());
        assert!(HeaderId::Trailer.is_hop_by_hop());
        assert!(HeaderId::Upgrade.is_hop_by_hop());
        assert!(!HeaderId::ContentType.is_hop_by_hop());
        assert!(!HeaderId::Host.is_hop_by_hop());
    }

    /// RFC 9110 section 6.5.1: a trailer section must not carry framing,
    /// routing, or request-modifier fields. Accepting Transfer-Encoding or
    /// Content-Length in a trailer is a smuggling vector.
    #[test]
    fn framing_headers_forbidden_in_trailers() {
        assert!(HeaderId::TransferEncoding.forbidden_in_trailers());
        assert!(HeaderId::ContentLength.forbidden_in_trailers());
        assert!(HeaderId::Host.forbidden_in_trailers());
        assert!(HeaderId::Connection.forbidden_in_trailers());
        assert!(HeaderId::Expect.forbidden_in_trailers());
        assert!(HeaderId::Te.forbidden_in_trailers());
        assert!(HeaderId::Trailer.forbidden_in_trailers());
        assert!(HeaderId::Upgrade.forbidden_in_trailers());
        assert!(!HeaderId::Etag.forbidden_in_trailers());
        assert!(!HeaderId::ContentType.forbidden_in_trailers());
    }

    fn vec_of(pairs: &[(HeaderId, &'static str)]) -> HeaderVec {
        pairs
            .iter()
            .map(|(id, v)| (id.clone(), Bytes::from_static(v.as_bytes())))
            .collect()
    }

    #[test]
    fn get_returns_first_match() {
        let v = vec_of(&[
            (HeaderId::Host, "a.example"),
            (HeaderId::ContentLength, "5"),
            (HeaderId::Host, "b.example"),
        ]);
        assert_eq!(get_str(&v, &HeaderId::Host), Some("a.example"));
        assert_eq!(get_str(&v, &HeaderId::ContentLength), Some("5"));
        assert_eq!(get(&v, &HeaderId::ContentType), None);
    }

    #[test]
    fn count_and_all_see_every_occurrence() {
        let v = vec_of(&[
            (HeaderId::Host, "a.example"),
            (HeaderId::Host, "b.example"),
            (HeaderId::ContentLength, "5"),
        ]);
        assert_eq!(count(&v, &HeaderId::Host), 2);
        assert_eq!(count(&v, &HeaderId::ContentLength), 1);
        assert_eq!(count(&v, &HeaderId::Date), 0);
        let hosts: Vec<_> = all(&v, &HeaderId::Host).collect();
        assert_eq!(hosts.len(), 2);
        assert_eq!(&hosts[1][..], b"b.example");
    }

    #[test]
    fn custom_names_compare_by_value() {
        let x = HeaderId::Other(ByteStr::from_static("x-request-id"));
        let v = vec_of(&[]);
        let mut v = v;
        v.push((x.clone(), Bytes::from_static(b"abc")));
        assert_eq!(get_str(&v, &x), Some("abc"));
        assert_eq!(
            get(&v, &HeaderId::Other(ByteStr::from_static("x-other"))),
            None
        );
    }

    #[test]
    fn get_str_rejects_non_utf8_values() {
        let mut v = HeaderVec::new();
        v.push((HeaderId::ContentType, Bytes::from_static(&[0xff, 0xfe])));
        assert_eq!(get_str(&v, &HeaderId::ContentType), None);
        assert!(get(&v, &HeaderId::ContentType).is_some());
    }

    /// Sixteen inline slots covers the overwhelming majority of real requests,
    /// so the header list itself never allocates.
    #[test]
    fn typical_request_stays_inline() {
        let mut v = HeaderVec::new();
        for _ in 0..16 {
            v.push((HeaderId::Accept, Bytes::from_static(b"*/*")));
        }
        assert!(!v.spilled(), "16 headers must stay on the stack");
        v.push((HeaderId::Accept, Bytes::from_static(b"*/*")));
        assert!(v.spilled(), "17 headers is expected to spill");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p armature-h1 header`
Expected: FAIL — `cannot find type HeaderId in this scope`.

- [ ] **Step 3: Implement `HeaderId`**

Prepend to `armature-h1/src/header.rs`:

```rust
//! Header name interning and the header list type.
//!
//! Well-known field names collapse to a discriminant, so a lookup is an
//! integer comparison rather than the case-insensitive string comparison a
//! conventional header map performs.

use crate::ByteStr;
use bytes::Bytes;
use smallvec::SmallVec;

/// An interned HTTP field name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum HeaderId {
    /// `host`
    Host,
    /// `content-length`
    ContentLength,
    /// `content-type`
    ContentType,
    /// `transfer-encoding`
    TransferEncoding,
    /// `connection`
    Connection,
    /// `expect`
    Expect,
    /// `upgrade`
    Upgrade,
    /// `date`
    Date,
    /// `server`
    Server,
    /// `accept`
    Accept,
    /// `accept-encoding`
    AcceptEncoding,
    /// `accept-language`
    AcceptLanguage,
    /// `authorization`
    Authorization,
    /// `cache-control`
    CacheControl,
    /// `cookie`
    Cookie,
    /// `set-cookie`
    SetCookie,
    /// `etag`
    Etag,
    /// `if-none-match`
    IfNoneMatch,
    /// `if-modified-since`
    IfModifiedSince,
    /// `last-modified`
    LastModified,
    /// `location`
    Location,
    /// `referer`
    Referer,
    /// `user-agent`
    UserAgent,
    /// `vary`
    Vary,
    /// `allow`
    Allow,
    /// `trailer`
    Trailer,
    /// `te`
    Te,
    /// `content-encoding`
    ContentEncoding,
    /// `content-language`
    ContentLanguage,
    /// `range`
    Range,
    /// `if-match`
    IfMatch,
    /// `if-unmodified-since`
    IfUnmodifiedSince,
    /// `origin`
    Origin,
    /// Any other valid field name, lowercased at parse time.
    Other(ByteStr),
}

impl HeaderId {
    /// Every well-known variant, for round-trip testing and diagnostics.
    pub const WELL_KNOWN: &'static [HeaderId] = &[
        HeaderId::Host,
        HeaderId::ContentLength,
        HeaderId::ContentType,
        HeaderId::TransferEncoding,
        HeaderId::Connection,
        HeaderId::Expect,
        HeaderId::Upgrade,
        HeaderId::Date,
        HeaderId::Server,
        HeaderId::Accept,
        HeaderId::AcceptEncoding,
        HeaderId::AcceptLanguage,
        HeaderId::Authorization,
        HeaderId::CacheControl,
        HeaderId::Cookie,
        HeaderId::SetCookie,
        HeaderId::Etag,
        HeaderId::IfNoneMatch,
        HeaderId::IfModifiedSince,
        HeaderId::LastModified,
        HeaderId::Location,
        HeaderId::Referer,
        HeaderId::UserAgent,
        HeaderId::Vary,
        HeaderId::Allow,
        HeaderId::Trailer,
        HeaderId::Te,
        HeaderId::ContentEncoding,
        HeaderId::ContentLanguage,
        HeaderId::Range,
        HeaderId::IfMatch,
        HeaderId::IfUnmodifiedSince,
        HeaderId::Origin,
    ];

    /// Match a field name against the well-known set, case-insensitively.
    ///
    /// Returns `None` when the name is not well-known; the caller then builds
    /// [`HeaderId::Other`] from the read buffer.
    ///
    /// Dispatching on length first bounds each call to one integer comparison
    /// plus a short case-insensitive compare. Field names are case-insensitive
    /// per RFC 9110 section 5.1.
    #[inline]
    pub fn from_bytes(name: &[u8]) -> Option<HeaderId> {
        // `eq_ignore_ascii_case` on a fixed-length slice compiles to a small
        // branch-free comparison; the length dispatch keeps the candidate set
        // tiny so this stays cheap.
        macro_rules! m {
            ($($lit:literal => $variant:expr),+ $(,)?) => {
                $(if name.eq_ignore_ascii_case($lit) { return Some($variant); })+
                None
            };
        }

        match name.len() {
            2 => m!(b"te" => HeaderId::Te),
            4 => m!(b"host" => HeaderId::Host, b"date" => HeaderId::Date, b"vary" => HeaderId::Vary),
            5 => m!(b"allow" => HeaderId::Allow, b"range" => HeaderId::Range),
            6 => m!(
                b"accept" => HeaderId::Accept,
                b"expect" => HeaderId::Expect,
                b"cookie" => HeaderId::Cookie,
                b"server" => HeaderId::Server,
                b"origin" => HeaderId::Origin,
            ),
            7 => m!(
                b"upgrade" => HeaderId::Upgrade,
                b"trailer" => HeaderId::Trailer,
                b"referer" => HeaderId::Referer,
            ),
            8 => m!(b"if-match" => HeaderId::IfMatch, b"location" => HeaderId::Location),
            10 => m!(
                b"connection" => HeaderId::Connection,
                b"set-cookie" => HeaderId::SetCookie,
                b"user-agent" => HeaderId::UserAgent,
            ),
            12 => m!(b"content-type" => HeaderId::ContentType),
            13 => m!(
                b"authorization" => HeaderId::Authorization,
                b"cache-control" => HeaderId::CacheControl,
                b"if-none-match" => HeaderId::IfNoneMatch,
                b"last-modified" => HeaderId::LastModified,
            ),
            14 => m!(b"content-length" => HeaderId::ContentLength),
            15 => m!(
                b"accept-encoding" => HeaderId::AcceptEncoding,
                b"accept-language" => HeaderId::AcceptLanguage,
            ),
            16 => m!(
                b"content-encoding" => HeaderId::ContentEncoding,
                b"content-language" => HeaderId::ContentLanguage,
            ),
            17 => m!(
                b"transfer-encoding" => HeaderId::TransferEncoding,
                b"if-modified-since" => HeaderId::IfModifiedSince,
            ),
            19 => m!(b"if-unmodified-since" => HeaderId::IfUnmodifiedSince),
            4_usize.. | _ => None,
        }
    }

    /// The canonical lowercase field name.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            HeaderId::Host => "host",
            HeaderId::ContentLength => "content-length",
            HeaderId::ContentType => "content-type",
            HeaderId::TransferEncoding => "transfer-encoding",
            HeaderId::Connection => "connection",
            HeaderId::Expect => "expect",
            HeaderId::Upgrade => "upgrade",
            HeaderId::Date => "date",
            HeaderId::Server => "server",
            HeaderId::Accept => "accept",
            HeaderId::AcceptEncoding => "accept-encoding",
            HeaderId::AcceptLanguage => "accept-language",
            HeaderId::Authorization => "authorization",
            HeaderId::CacheControl => "cache-control",
            HeaderId::Cookie => "cookie",
            HeaderId::SetCookie => "set-cookie",
            HeaderId::Etag => "etag",
            HeaderId::IfNoneMatch => "if-none-match",
            HeaderId::IfModifiedSince => "if-modified-since",
            HeaderId::LastModified => "last-modified",
            HeaderId::Location => "location",
            HeaderId::Referer => "referer",
            HeaderId::UserAgent => "user-agent",
            HeaderId::Vary => "vary",
            HeaderId::Allow => "allow",
            HeaderId::Trailer => "trailer",
            HeaderId::Te => "te",
            HeaderId::ContentEncoding => "content-encoding",
            HeaderId::ContentLanguage => "content-language",
            HeaderId::Range => "range",
            HeaderId::IfMatch => "if-match",
            HeaderId::IfUnmodifiedSince => "if-unmodified-since",
            HeaderId::Origin => "origin",
            HeaderId::Other(s) => s.as_str(),
        }
    }

    /// Whether this field is hop-by-hop and must not be forwarded.
    #[inline]
    pub fn is_hop_by_hop(&self) -> bool {
        matches!(
            self,
            HeaderId::Connection
                | HeaderId::TransferEncoding
                | HeaderId::Te
                | HeaderId::Trailer
                | HeaderId::Upgrade
        )
    }

    /// Whether RFC 9110 section 6.5.1 forbids this field in a trailer section.
    ///
    /// Framing fields are the critical entries: a `Transfer-Encoding` or
    /// `Content-Length` accepted from a trailer is a request-smuggling vector,
    /// because framing was already decided before the trailer was read.
    #[inline]
    pub fn forbidden_in_trailers(&self) -> bool {
        matches!(
            self,
            HeaderId::TransferEncoding
                | HeaderId::ContentLength
                | HeaderId::Host
                | HeaderId::Connection
                | HeaderId::Expect
                | HeaderId::Te
                | HeaderId::Trailer
                | HeaderId::Upgrade
                | HeaderId::CacheControl
                | HeaderId::Authorization
                | HeaderId::SetCookie
        )
    }
}
```

Note the `4_usize.. | _ => None` arm above is a placeholder for the exhaustiveness
requirement — replace it with a plain `_ => None` arm and confirm `cargo clippy`
raises no `unreachable_patterns` warning.

- [ ] **Step 4: Implement `HeaderVec` and its accessors**

Append to `armature-h1/src/header.rs`, before the test module:

```rust
/// A request or response header list.
///
/// Sixteen inline slots covers the overwhelming majority of real requests, so
/// the list itself does not allocate. Values are [`Bytes`] slices of the
/// connection read buffer.
pub type HeaderVec = SmallVec<[(HeaderId, Bytes); 16]>;

/// The first value for `id`, or `None`.
///
/// A linear scan over at most a few dozen entries beats hashing a field name,
/// and it never allocates.
#[inline]
pub fn get<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a Bytes> {
    v.iter().find(|(k, _)| k == id).map(|(_, val)| val)
}

/// The first value for `id` as a string, or `None` if absent or not UTF-8.
#[inline]
pub fn get_str<'a>(v: &'a HeaderVec, id: &HeaderId) -> Option<&'a str> {
    get(v, id).and_then(|b| std::str::from_utf8(b).ok())
}

/// Every value for `id`, in wire order.
#[inline]
pub fn all<'a>(v: &'a HeaderVec, id: &'a HeaderId) -> impl Iterator<Item = &'a Bytes> + 'a {
    v.iter().filter(move |(k, _)| k == id).map(|(_, val)| val)
}

/// How many times `id` appears.
///
/// Framing correctness depends on this: exactly one `Host` is required on
/// HTTP/1.1, and duplicate `Content-Length` fields must be rejected.
#[inline]
pub fn count(v: &HeaderVec, id: &HeaderId) -> usize {
    v.iter().filter(|(k, _)| k == id).count()
}
```

- [ ] **Step 5: Wire into `lib.rs`**

Add `mod header;` after `mod bytestr;` and extend the re-exports:

```rust
pub use header::{HeaderId, HeaderVec};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 23 tests.

- [ ] **Step 7: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings. If `unreachable_patterns` fires, fix the match arm noted in Step 3.

- [ ] **Step 8: Commit**

```bash
git add armature-h1/src/header.rs armature-h1/src/lib.rs
git commit -m "feat(h1): HeaderId interning and HeaderVec

Well-known field names collapse to a discriminant, turning each header lookup
from a case-insensitive string comparison into an integer comparison. Values
are Bytes slices of the read buffer; 16 inline slots keep the list itself
allocation-free for typical requests.

forbidden_in_trailers encodes RFC 9110 6.5.1 — accepting Transfer-Encoding or
Content-Length from a trailer section is a smuggling vector, since framing was
decided before the trailer was read."
```

---

### Task 4: `Limits`

**Files:**
- Create: `armature-h1/src/limits.rs`
- Modify: `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/limits.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `struct Limits { max_head_bytes: usize, max_headers: usize, max_body_bytes: u64, max_pipeline_depth: usize, header_timeout: Duration, body_timeout: Duration, idle_timeout: Duration, write_timeout: Duration }` with `Default` and `Clone`.
  - Defaults: `max_head_bytes: 16 * 1024`, `max_headers: 96`, `max_body_bytes: 2 * 1024 * 1024`, `max_pipeline_depth: 8`, `header_timeout: 10s`, `body_timeout: 30s`, `idle_timeout: 75s`, `write_timeout: 30s`.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_values() {
        let l = Limits::default();
        assert_eq!(l.max_head_bytes, 16 * 1024);
        assert_eq!(l.max_headers, 96);
        assert_eq!(l.max_body_bytes, 2 * 1024 * 1024);
        assert_eq!(l.max_pipeline_depth, 8);
        assert_eq!(l.header_timeout, Duration::from_secs(10));
        assert_eq!(l.body_timeout, Duration::from_secs(30));
        assert_eq!(l.idle_timeout, Duration::from_secs(75));
        assert_eq!(l.write_timeout, Duration::from_secs(30));
    }

    /// `max_headers` must not exceed the fixed httparse scratch array in
    /// parse.rs, or heads that fit the limit would fail to parse.
    #[test]
    fn max_headers_fits_the_parser_scratch_array() {
        assert!(Limits::default().max_headers <= crate::limits::MAX_HEADERS_CEILING);
    }

    #[test]
    fn builder_overrides_apply() {
        let l = Limits {
            max_body_bytes: 64,
            ..Default::default()
        };
        assert_eq!(l.max_body_bytes, 64);
        assert_eq!(l.max_headers, 96, "unrelated fields keep their defaults");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p armature-h1 limits`
Expected: FAIL — `cannot find type Limits in this scope`.

- [ ] **Step 3: Implement `Limits`**

Prepend to `armature-h1/src/limits.rs`:

```rust
//! Resource limits and deadlines.
//!
//! These are the slowloris and resource-exhaustion defenses. Every field has a
//! finite default; there is deliberately no "unlimited" setting for the head,
//! because an unbounded head read is a trivial memory-exhaustion vector.

use std::time::Duration;

/// The largest `max_headers` the parser's fixed scratch array can serve.
///
/// `parse::parse_head` allocates its `httparse::Header` array on the stack at
/// this size, so `Limits::max_headers` must never exceed it.
pub const MAX_HEADERS_CEILING: usize = 128;

/// Per-connection resource limits and deadlines.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum bytes in the request line plus header section. Exceeding this
    /// yields 431 and closes.
    pub max_head_bytes: usize,
    /// Maximum header field count. Exceeding this yields 431 and closes.
    /// Must be `<= MAX_HEADERS_CEILING`.
    pub max_headers: usize,
    /// Maximum request body bytes. Exceeding this yields 413 and closes.
    pub max_body_bytes: u64,
    /// Maximum requests parsed ahead of the one being served. Reaching this
    /// stops reading (backpressure) rather than producing a response.
    pub max_pipeline_depth: usize,
    /// Deadline for the complete head to arrive once the first byte does.
    pub header_timeout: Duration,
    /// Deadline for the complete body to arrive once the head is parsed.
    pub body_timeout: Duration,
    /// Deadline for the next request to begin on an idle keep-alive
    /// connection.
    pub idle_timeout: Duration,
    /// Deadline for a response write to complete.
    pub write_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_head_bytes: 16 * 1024,
            max_headers: 96,
            max_body_bytes: 2 * 1024 * 1024,
            max_pipeline_depth: 8,
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(75),
            write_timeout: Duration::from_secs(30),
        }
    }
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Add `mod limits;` and `pub use limits::Limits;`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 26 tests.

- [ ] **Step 6: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add armature-h1/src/limits.rs armature-h1/src/lib.rs
git commit -m "feat(h1): Limits with finite defaults

Head size, header count, body size, pipeline depth, and four deadlines. There
is deliberately no unlimited setting for the head: an unbounded head read is a
trivial memory-exhaustion vector.

MAX_HEADERS_CEILING ties max_headers to the parser's fixed stack scratch array
so a head that fits the configured limit can always be parsed."
```

---

### Task 5: The strict-CRLF prescan

**Files:**
- Create: `armature-h1/src/parse.rs`
- Modify: `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/parse.rs`

**Interfaces:**
- Consumes: `Limits`, `MAX_HEADERS_CEILING` from Task 4.
- Produces:
  - `enum ParseError { BareCr, BareLf, ObsFold, WhitespaceBeforeColon, InvalidHeaderName, InvalidRequestLine, InvalidMethod, NonUtf8Target, UnsupportedVersion, HeadTooLarge, TooManyHeaders }`
  - `ParseError::status(&self) -> u16` — 400 for malformed, 431 for oversized, 505 for a non-HTTP/1.x version.
  - `fn find_head_end(buf: &[u8]) -> Option<usize>` — index just past the terminating `\r\n\r\n`, or `None` if the head is incomplete.
  - `fn prescan(head: &[u8]) -> Result<(), ParseError>` — validates strict CRLF and rejects obs-fold, over the head region only.

Why a prescan rather than trusting the parser: `httparse` is deliberately lenient about line endings, accepting a bare LF as a terminator. RFC 9112 §2.2 permits that, and this crate declines it. Divergence between what we accept and what an upstream or downstream peer accepts is precisely where request smuggling lives, so the strict check runs first and independently, and its result is not conditional on parser internals.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const OK: &[u8] = b"GET / HTTP/1.1\r\nHost: a\r\n\r\n";

    #[test]
    fn find_head_end_locates_the_terminator() {
        assert_eq!(find_head_end(OK), Some(OK.len()));
        // Body bytes after the terminator do not move the boundary.
        let with_body = b"GET / HTTP/1.1\r\nHost: a\r\n\r\nBODY";
        assert_eq!(find_head_end(with_body), Some(with_body.len() - 4));
    }

    #[test]
    fn find_head_end_reports_incomplete() {
        assert_eq!(find_head_end(b""), None);
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\n"), None);
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: a\r\n"), None);
        // A lone trailing CR is not yet a terminator.
        assert_eq!(find_head_end(b"GET / HTTP/1.1\r\nHost: a\r\n\r"), None);
    }

    #[test]
    fn prescan_accepts_strict_crlf() {
        assert!(prescan(OK).is_ok());
        assert!(prescan(b"GET / HTTP/1.1\r\nHost: a\r\nAccept: */*\r\n\r\n").is_ok());
        assert!(prescan(b"GET / HTTP/1.1\r\n\r\n").is_ok());
    }

    /// RFC 9112 section 2.2 permits recognizing a bare LF. We decline: leniency
    /// that differs from a peer's leniency is a smuggling vector.
    #[test]
    fn prescan_rejects_bare_lf() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\nHost: a\r\n\r\n"),
            Err(ParseError::BareLf)
        );
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\n\r\n"),
            Err(ParseError::BareLf)
        );
    }

    #[test]
    fn prescan_rejects_bare_cr() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\rHost: a\r\n\r\n"),
            Err(ParseError::BareCr)
        );
        // A CR embedded in a field value is equally rejected.
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\rb\r\n\r\n"),
            Err(ParseError::BareCr)
        );
    }

    /// RFC 9112 section 5.2: obs-fold must be rejected in a request.
    #[test]
    fn prescan_rejects_obs_fold() {
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\r\n b\r\n\r\n"),
            Err(ParseError::ObsFold)
        );
        assert_eq!(
            prescan(b"GET / HTTP/1.1\r\nHost: a\r\n\tb\r\n\r\n"),
            Err(ParseError::ObsFold)
        );
    }

    /// The blank line terminating the head is CRLF followed by CRLF, which
    /// must not itself be mistaken for a fold.
    #[test]
    fn prescan_does_not_mistake_the_terminator_for_a_fold() {
        assert!(prescan(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n").is_ok());
    }

    #[test]
    fn status_codes_map_correctly() {
        assert_eq!(ParseError::BareCr.status(), 400);
        assert_eq!(ParseError::BareLf.status(), 400);
        assert_eq!(ParseError::ObsFold.status(), 400);
        assert_eq!(ParseError::WhitespaceBeforeColon.status(), 400);
        assert_eq!(ParseError::InvalidRequestLine.status(), 400);
        assert_eq!(ParseError::NonUtf8Target.status(), 400);
        assert_eq!(ParseError::HeadTooLarge.status(), 431);
        assert_eq!(ParseError::TooManyHeaders.status(), 431);
        assert_eq!(ParseError::UnsupportedVersion.status(), 505);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p armature-h1 parse`
Expected: FAIL — `cannot find function find_head_end in this scope`.

- [ ] **Step 3: Implement `ParseError`, `find_head_end`, and `prescan`**

Prepend to `armature-h1/src/parse.rs`:

```rust
//! Request head parsing.
//!
//! Parsing runs in two independent passes. [`prescan`] enforces strict CRLF
//! framing and rejects obs-fold; only then does `httparse` tokenize the head.
//!
//! The prescan exists because `httparse` is deliberately lenient about line
//! endings and will accept a bare LF as a line terminator. RFC 9112 section 2.2
//! permits that, and this crate declines it: when our leniency differs from an
//! upstream or downstream peer's leniency, the difference is a request-smuggling
//! vector. Running the strict check first and independently means the decision
//! does not depend on parser internals.

use crate::limits::MAX_HEADERS_CEILING;
use memchr::memmem;

/// A malformed or oversized request head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// A CR not followed by LF.
    #[error("bare CR in message head")]
    BareCr,
    /// An LF not preceded by CR.
    #[error("bare LF in message head")]
    BareLf,
    /// Obsolete line folding (RFC 9112 section 5.2).
    #[error("obs-fold in message head")]
    ObsFold,
    /// Whitespace between a field name and its colon.
    #[error("whitespace before colon in field name")]
    WhitespaceBeforeColon,
    /// A field name containing characters outside the token set.
    #[error("invalid field name")]
    InvalidHeaderName,
    /// A request line that does not parse.
    #[error("invalid request line")]
    InvalidRequestLine,
    /// A method token containing characters outside the token set.
    #[error("invalid method token")]
    InvalidMethod,
    /// A request target that is not valid UTF-8.
    #[error("request target is not valid UTF-8")]
    NonUtf8Target,
    /// A version other than HTTP/1.0 or HTTP/1.1.
    #[error("unsupported HTTP version")]
    UnsupportedVersion,
    /// The head exceeded `Limits::max_head_bytes`.
    #[error("message head too large")]
    HeadTooLarge,
    /// The head carried more fields than `Limits::max_headers`.
    #[error("too many header fields")]
    TooManyHeaders,
}

impl ParseError {
    /// The status code to answer with before closing the connection.
    #[inline]
    pub fn status(&self) -> u16 {
        match self {
            ParseError::HeadTooLarge | ParseError::TooManyHeaders => 431,
            ParseError::UnsupportedVersion => 505,
            _ => 400,
        }
    }
}

/// The index just past the `\r\n\r\n` terminating the head, or `None` if the
/// head is not yet complete.
///
/// Uses a SIMD-accelerated substring search, so scanning a partial head is
/// cheap enough to repeat on every read without buffering heuristics.
#[inline]
pub fn find_head_end(buf: &[u8]) -> Option<usize> {
    memmem::find(buf, b"\r\n\r\n").map(|i| i + 4)
}

/// Validate that `head` uses strict CRLF line endings and contains no obs-fold.
///
/// `head` must be exactly the head region, terminator included, as returned by
/// [`find_head_end`].
pub fn prescan(head: &[u8]) -> Result<(), ParseError> {
    // Every CR must be followed by LF, and every LF preceded by CR. Walking the
    // CR positions and the LF positions separately lets memchr vectorize both
    // scans, and the two checks together are equivalent to "line endings are
    // exactly CRLF".
    for i in memchr::memchr_iter(b'\r', head) {
        if head.get(i + 1) != Some(&b'\n') {
            return Err(ParseError::BareCr);
        }
    }
    for i in memchr::memchr_iter(b'\n', head) {
        if i == 0 || head[i - 1] != b'\r' {
            return Err(ParseError::BareLf);
        }
    }

    // obs-fold: a CRLF followed by SP or HTAB continues the previous field.
    // The final CRLF CRLF terminator is not a fold, and cannot be mistaken for
    // one, because the byte after the third CR is LF rather than SP or HTAB.
    for i in memmem::find_iter(head, b"\r\n") {
        match head.get(i + 2) {
            Some(b' ') | Some(b'\t') => return Err(ParseError::ObsFold),
            _ => {}
        }
    }

    Ok(())
}

// Compile-time assertion that the parser scratch array matches the ceiling
// `Limits` validates against.
const _: () = assert!(MAX_HEADERS_CEILING == 128);
```

- [ ] **Step 4: Wire into `lib.rs`**

Add `pub mod parse;` (public, because the conformance tests in Task 15 and the fuzz targets in Task 16 exercise `prescan` and `parse_head` directly) and `pub use parse::ParseError;`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 34 tests.

- [ ] **Step 6: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add armature-h1/src/parse.rs armature-h1/src/lib.rs
git commit -m "feat(h1): strict-CRLF prescan and ParseError

httparse accepts a bare LF as a line terminator, which RFC 9112 2.2 permits
and this crate declines. The prescan runs first and independently of the
parser: rejecting bare CR, bare LF, and obs-fold before tokenization means the
decision never depends on parser internals, and our leniency cannot drift from
a peer's.

Both scans are memchr-vectorized, so revalidating a partial head on every read
is cheap enough to need no buffering heuristics."
```

---

### Task 6: `Head` and `parse_head`

**Files:**
- Create: `armature-h1/src/head.rs`
- Modify: `armature-h1/src/parse.rs`, `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/parse.rs`

**Interfaces:**
- Consumes: `ByteStr`, `Method`, `Version`, `HeaderId`, `HeaderVec`, `Limits`, `ParseError`, `find_head_end`, `prescan`.
- Produces:
  - `struct Head { method: Method, target: ByteStr, version: Version, headers: HeaderVec }` — all fields public.
  - `Head::get(&self, id: &HeaderId) -> Option<&Bytes>`, `get_str`, `all`, `count` — thin forwards to the Task 3 free functions.
  - `Head::path(&self) -> &str` — target up to `?`.
  - `Head::query(&self) -> Option<&str>` — target after the first `?`, `None` if absent.
  - `Head::is_keep_alive(&self) -> bool` — `Connection` token analysis against the version default.
  - `Head::connection_has_token(&self, token: &str) -> bool` — case-insensitive comma-list membership, used for `close`, `keep-alive`, and `upgrade`.
  - `fn parse_head(buf: &Bytes, limits: &Limits) -> Result<Option<(Head, usize)>, ParseError>` — `Ok(None)` means more bytes are needed; the `usize` is the head length in bytes.

The load-bearing property, asserted by test: every `Bytes` in the returned `Head` shares `buf`'s allocation. `Bytes::slice_ref` performs that projection and is safe — it panics only if handed a slice that is not a subslice, which cannot occur here because `httparse` borrows from the same buffer.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `armature-h1/src/parse.rs`:

```rust
    use crate::{HeaderId, Limits, Method, Version};
    use bytes::Bytes;

    fn parse(raw: &'static [u8]) -> Result<Option<(crate::Head, usize)>, ParseError> {
        parse_head(&Bytes::from_static(raw), &Limits::default())
    }

    #[test]
    fn parses_a_minimal_request() {
        let (head, n) = parse(b"GET / HTTP/1.1\r\nHost: a.example\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(n, 35);
        assert_eq!(head.method, Method::Get);
        assert_eq!(head.target.as_str(), "/");
        assert_eq!(head.version, Version::Http11);
        assert_eq!(head.headers.len(), 1);
        assert_eq!(head.get_str(&HeaderId::Host), Some("a.example"));
    }

    #[test]
    fn reports_incomplete_heads() {
        assert_eq!(parse(b"GET / HTTP/1.1\r\n").unwrap(), None);
        assert_eq!(parse(b"GE").unwrap(), None);
        assert_eq!(parse(b"").unwrap(), None);
    }

    /// The whole point of the design: nothing in the parsed head is copied.
    #[test]
    fn header_values_share_the_read_buffer() {
        let buf = Bytes::from_static(b"GET /a?b=1 HTTP/1.1\r\nHost: a.example\r\n\r\n");
        let (head, _) = parse_head(&buf, &Limits::default()).unwrap().unwrap();
        let host = head.get(&HeaderId::Host).unwrap();
        // A slice of the same allocation has the same base pointer arithmetic:
        // its address must fall inside the parent buffer's range.
        let base = buf.as_ptr() as usize;
        let addr = host.as_ptr() as usize;
        assert!(
            addr >= base && addr < base + buf.len(),
            "header value must point into the read buffer, not a copy"
        );
        let target = head.target.as_bytes().as_ptr() as usize;
        assert!(target >= base && target < base + buf.len());
    }

    #[test]
    fn splits_path_and_query() {
        let (head, _) = parse(b"GET /a/b?x=1&y=2 HTTP/1.1\r\nHost: a\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.path(), "/a/b");
        assert_eq!(head.query(), Some("x=1&y=2"));

        let (head, _) = parse(b"GET /a/b HTTP/1.1\r\nHost: a\r\n\r\n").unwrap().unwrap();
        assert_eq!(head.path(), "/a/b");
        assert_eq!(head.query(), None);

        // An empty query is present but empty, distinct from absent.
        let (head, _) = parse(b"GET /a? HTTP/1.1\r\nHost: a\r\n\r\n").unwrap().unwrap();
        assert_eq!(head.path(), "/a");
        assert_eq!(head.query(), Some(""));
    }

    #[test]
    fn unknown_method_and_header_become_other_variants() {
        let (head, _) = parse(b"PROPFIND / HTTP/1.1\r\nHost: a\r\nX-Req-Id: z\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(head.method.as_str(), "PROPFIND");
        assert!(matches!(head.method, Method::Other(_)));
        let id = HeaderId::Other(crate::ByteStr::from_static("x-req-id"));
        assert_eq!(head.get_str(&id), Some("z"));
    }

    /// Field names are case-insensitive, so a custom name must be lowercased at
    /// parse time or lookups would depend on the sender's capitalization.
    #[test]
    fn custom_header_names_are_lowercased() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\nX-Req-Id: z\r\n\r\n")
            .unwrap()
            .unwrap();
        match &head.headers[1].0 {
            HeaderId::Other(name) => assert_eq!(name.as_str(), "x-req-id"),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn http_10_is_accepted_and_http_12_is_not() {
        let (head, _) = parse(b"GET / HTTP/1.0\r\n\r\n").unwrap().unwrap();
        assert_eq!(head.version, Version::Http10);
        assert_eq!(
            parse(b"GET / HTTP/1.2\r\nHost: a\r\n\r\n"),
            Err(ParseError::UnsupportedVersion)
        );
    }

    #[test]
    fn rejects_whitespace_before_colon() {
        assert_eq!(
            parse(b"GET / HTTP/1.1\r\nHost : a\r\n\r\n"),
            Err(ParseError::WhitespaceBeforeColon)
        );
    }

    #[test]
    fn rejects_malformed_request_lines() {
        assert!(parse(b"GET\r\nHost: a\r\n\r\n").is_err());
        assert!(parse(b"GET /\r\nHost: a\r\n\r\n").is_err());
        assert!(parse(b"/ HTTP/1.1\r\nHost: a\r\n\r\n").is_err());
    }

    #[test]
    fn prescan_runs_before_tokenization() {
        // A bare LF must be rejected as BareLf, not silently accepted by
        // httparse's lenient line-ending handling.
        assert_eq!(
            parse(b"GET / HTTP/1.1\nHost: a\r\n\r\n"),
            Err(ParseError::BareLf)
        );
    }

    #[test]
    fn enforces_head_size_limit() {
        let limits = Limits {
            max_head_bytes: 40,
            ..Default::default()
        };
        let long = Bytes::from_static(
            b"GET /aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HTTP/1.1\r\nHost: a\r\n\r\n",
        );
        assert_eq!(
            parse_head(&long, &limits),
            Err(ParseError::HeadTooLarge)
        );
    }

    #[test]
    fn enforces_header_count_limit() {
        let limits = Limits {
            max_headers: 2,
            ..Default::default()
        };
        let raw = Bytes::from_static(b"GET / HTTP/1.1\r\nHost: a\r\nA: 1\r\nB: 2\r\n\r\n");
        assert_eq!(parse_head(&raw, &limits), Err(ParseError::TooManyHeaders));
    }

    #[test]
    fn keep_alive_defaults_follow_the_version() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n").unwrap().unwrap();
        assert!(head.is_keep_alive(), "HTTP/1.1 persists by default");

        let (head, _) = parse(b"GET / HTTP/1.0\r\n\r\n").unwrap().unwrap();
        assert!(!head.is_keep_alive(), "HTTP/1.0 closes by default");
    }

    #[test]
    fn connection_tokens_override_the_default() {
        let (head, _) = parse(b"GET / HTTP/1.1\r\nHost: a\r\nConnection: close\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(!head.is_keep_alive());

        let (head, _) = parse(b"GET / HTTP/1.0\r\nConnection: keep-alive\r\n\r\n")
            .unwrap()
            .unwrap();
        assert!(head.is_keep_alive());
    }

    #[test]
    fn connection_token_matching_is_case_insensitive_and_list_aware() {
        let (head, _) = parse(
            b"GET / HTTP/1.1\r\nHost: a\r\nConnection: Keep-Alive, Upgrade\r\n\r\n",
        )
        .unwrap()
        .unwrap();
        assert!(head.connection_has_token("upgrade"));
        assert!(head.connection_has_token("keep-alive"));
        assert!(!head.connection_has_token("close"));
        assert!(head.is_keep_alive());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armature-h1 parse`
Expected: FAIL — `cannot find function parse_head in this scope`.

- [ ] **Step 3: Implement `Head`**

Create `armature-h1/src/head.rs`:

```rust
//! The parsed request head.

use crate::header::{self, HeaderId, HeaderVec};
use crate::{ByteStr, Method, Version};
use bytes::Bytes;

/// A parsed request line and header section.
///
/// Every [`Bytes`] within shares the connection's read buffer allocation, so
/// constructing a `Head` copies nothing.
#[derive(Clone, Debug)]
pub struct Head {
    /// The request method.
    pub method: Method,
    /// The request target, exactly as received.
    pub target: ByteStr,
    /// The protocol version.
    pub version: Version,
    /// The header fields, in wire order.
    pub headers: HeaderVec,
}

impl Head {
    /// The first value for `id`.
    #[inline]
    pub fn get(&self, id: &HeaderId) -> Option<&Bytes> {
        header::get(&self.headers, id)
    }

    /// The first value for `id` as a string, or `None` if absent or not UTF-8.
    #[inline]
    pub fn get_str(&self, id: &HeaderId) -> Option<&str> {
        header::get_str(&self.headers, id)
    }

    /// Every value for `id`, in wire order.
    #[inline]
    pub fn all<'a>(&'a self, id: &'a HeaderId) -> impl Iterator<Item = &'a Bytes> + 'a {
        header::all(&self.headers, id)
    }

    /// How many times `id` appears.
    #[inline]
    pub fn count(&self, id: &HeaderId) -> usize {
        header::count(&self.headers, id)
    }

    /// The target up to, but excluding, the first `?`.
    #[inline]
    pub fn path(&self) -> &str {
        let t = self.target.as_str();
        match t.as_bytes().iter().position(|&b| b == b'?') {
            Some(i) => &t[..i],
            None => t,
        }
    }

    /// The target after the first `?`, or `None` when there is no `?`.
    ///
    /// A trailing `?` yields `Some("")`, which is distinct from `None`. This is
    /// deliberate: it lets a caller distinguish "no query" from "empty query"
    /// without re-examining the target.
    #[inline]
    pub fn query(&self) -> Option<&str> {
        let t = self.target.as_str();
        t.as_bytes()
            .iter()
            .position(|&b| b == b'?')
            .map(|i| &t[i + 1..])
    }

    /// Whether `Connection` carries `token`, compared case-insensitively
    /// against each comma-separated element.
    pub fn connection_has_token(&self, token: &str) -> bool {
        self.all(&HeaderId::Connection).any(|v| {
            std::str::from_utf8(v)
                .map(|s| s.split(',').any(|t| t.trim().eq_ignore_ascii_case(token)))
                .unwrap_or(false)
        })
    }

    /// Whether the connection should persist after this request.
    ///
    /// HTTP/1.1 persists unless `Connection: close`; HTTP/1.0 closes unless
    /// `Connection: keep-alive`.
    #[inline]
    pub fn is_keep_alive(&self) -> bool {
        match self.version {
            Version::Http11 => !self.connection_has_token("close"),
            Version::Http10 => self.connection_has_token("keep-alive"),
        }
    }
}
```

- [ ] **Step 4: Implement `parse_head`**

Append to `armature-h1/src/parse.rs`, before the test module:

```rust
use crate::head::Head;
use crate::header::{HeaderId, HeaderVec};
use crate::{ByteStr, Limits, Method, Version};
use bytes::Bytes;

/// Parse a request head out of `buf`.
///
/// Returns `Ok(None)` when the head is not yet complete, and `Ok(Some((head,
/// n)))` where `n` is the head length in bytes, so the caller knows where the
/// body begins.
///
/// Every `Bytes` in the returned `Head` is a `slice_ref` projection of `buf`,
/// so no field content is copied.
pub fn parse_head(buf: &Bytes, limits: &Limits) -> Result<Option<(Head, usize)>, ParseError> {
    debug_assert!(limits.max_headers <= MAX_HEADERS_CEILING);

    let Some(head_len) = find_head_end(buf) else {
        // Incomplete. Refuse to keep buffering an unterminated head forever.
        if buf.len() > limits.max_head_bytes {
            return Err(ParseError::HeadTooLarge);
        }
        return Ok(None);
    };

    if head_len > limits.max_head_bytes {
        return Err(ParseError::HeadTooLarge);
    }

    let region = &buf[..head_len];
    prescan(region)?;

    let mut scratch = [httparse::EMPTY_HEADER; MAX_HEADERS_CEILING];
    let mut req = httparse::Request::new(&mut scratch);
    let parsed = req.parse(region).map_err(map_httparse)?;
    let httparse::Status::Complete(consumed) = parsed else {
        // `find_head_end` located a terminator, so httparse must have completed.
        // A Partial here means the head contained a NUL or similar that httparse
        // treats as needing more input; reject rather than loop.
        return Err(ParseError::InvalidRequestLine);
    };
    debug_assert_eq!(consumed, head_len);

    let version = req
        .version
        .and_then(Version::from_httparse)
        .ok_or(ParseError::UnsupportedVersion)?;

    let method_token = req.method.ok_or(ParseError::InvalidRequestLine)?;
    let method = match Method::from_bytes(method_token.as_bytes()) {
        Some(m) => m,
        None => {
            if method_token.is_empty() {
                return Err(ParseError::InvalidMethod);
            }
            Method::Other(
                ByteStr::from_utf8(buf.slice_ref(method_token.as_bytes()))
                    .map_err(|_| ParseError::InvalidMethod)?,
            )
        }
    };

    let target_token = req.path.ok_or(ParseError::InvalidRequestLine)?;
    let target = ByteStr::from_utf8(buf.slice_ref(target_token.as_bytes()))
        .map_err(|_| ParseError::NonUtf8Target)?;

    let field_count = req.headers.len();
    if field_count > limits.max_headers {
        return Err(ParseError::TooManyHeaders);
    }

    let mut headers = HeaderVec::with_capacity(field_count);
    for h in req.headers.iter() {
        let name = h.name.as_bytes();
        let id = match HeaderId::from_bytes(name) {
            Some(id) => id,
            None => {
                // Field names are case-insensitive, so a custom name must be
                // normalized here or lookups would depend on the sender's
                // capitalization. Already-lowercase names — the common case —
                // are projected without copying; only mixed-case names allocate.
                let lowered = if name.iter().any(|b| b.is_ascii_uppercase()) {
                    Bytes::from(name.to_ascii_lowercase())
                } else {
                    buf.slice_ref(name)
                };
                HeaderId::Other(
                    ByteStr::from_utf8(lowered).map_err(|_| ParseError::InvalidHeaderName)?,
                )
            }
        };
        headers.push((id, buf.slice_ref(h.value)));
    }

    Ok(Some((
        Head {
            method,
            target,
            version,
            headers,
        },
        head_len,
    )))
}

/// Map an `httparse` error onto our error set.
///
/// `httparse` reports a field name containing a space as `HeaderName`, which is
/// the whitespace-before-colon case RFC 9112 section 5.1 requires rejecting.
fn map_httparse(e: httparse::Error) -> ParseError {
    match e {
        httparse::Error::HeaderName => ParseError::WhitespaceBeforeColon,
        httparse::Error::Version => ParseError::UnsupportedVersion,
        httparse::Error::Token => ParseError::InvalidMethod,
        httparse::Error::TooManyHeaders => ParseError::TooManyHeaders,
        _ => ParseError::InvalidRequestLine,
    }
}
```

> **Note on `map_httparse`:** `httparse::Error::HeaderName` covers more than the
> whitespace-before-colon case, but every cause is a malformed field name
> answered with 400, so collapsing them costs no correctness. The distinct
> variant exists to make the RFC 9112 §5.1 test read clearly.

- [ ] **Step 5: Wire into `lib.rs`**

Add `mod head;` and extend the re-exports:

```rust
pub use head::Head;
pub use parse::parse_head;
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 49 tests.

If `enforces_header_count_limit` fails because `httparse` reported `TooManyHeaders`
first, that is acceptable: both paths produce `ParseError::TooManyHeaders`. Confirm
the assertion still holds.

- [ ] **Step 7: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add armature-h1/src/head.rs armature-h1/src/parse.rs armature-h1/src/lib.rs
git commit -m "feat(h1): Head and parse_head

Projects httparse's borrowed tokens into Bytes slices of the read buffer via
slice_ref, so a parsed head copies nothing. A pointer-range assertion in the
tests holds that property in place — it is the premise of the whole design.

Custom field names are lowercased at parse time, since field names are
case-insensitive and lookups must not depend on the sender's capitalization;
already-lowercase names, the common case, still project without copying.

Version, method, and target validation reject rather than guess: HTTP/1.2 is
505, a non-UTF-8 target is 400."
```

---

### Task 7: `framing::decide` — the security core

**Files:**
- Create: `armature-h1/src/framing.rs`
- Modify: `armature-h1/src/lib.rs`
- Test: inline in `armature-h1/src/framing.rs`

**Interfaces:**
- Consumes: `Head`, `HeaderId`, `Limits`, `Version`.
- Produces:
  - `enum BodyKind { None, Length(u64), Chunked }`
  - `enum FramingError { LengthAndTransferEncoding, DuplicateContentLength, InvalidContentLength, ChunkedNotFinal, UnsupportedTransferEncoding, MissingHost, MultipleHost, BodyTooLarge }`
  - `FramingError::status(&self) -> u16` — 400, except `BodyTooLarge` → 413 and `UnsupportedTransferEncoding` → 501.
  - `fn decide(head: &Head, limits: &Limits) -> Result<BodyKind, FramingError>`

This is the single most security-critical function in the crate. It is deliberately pure and synchronous — no I/O, no state — so it can be exhaustively tested and differentially fuzzed against hyper in Task 16. Every error closes the connection.

The rule order matters and is fixed: `Host` validity, then `Content-Length`-with-`Transfer-Encoding` conflict, then `Transfer-Encoding` analysis, then `Content-Length` analysis, then the size limit. Checking the conflict before either individual analysis means a request carrying both a valid `Content-Length` and a valid `Transfer-Encoding` is rejected rather than silently resolved in favor of one — which is exactly the ambiguity smuggling exploits.

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Limits, parse_head};
    use bytes::Bytes;

    /// Build a head from a raw request, asserting it parses.
    fn head(raw: &'static [u8]) -> crate::Head {
        parse_head(&Bytes::from_static(raw), &Limits::default())
            .expect("must parse")
            .expect("must be complete")
            .0
    }

    fn decide_raw(raw: &'static [u8]) -> Result<BodyKind, FramingError> {
        decide(&head(raw), &Limits::default())
    }

    // ---- positive cases ----

    #[test]
    fn no_framing_headers_means_no_body() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\nHost: a\r\n\r\n"),
            Ok(BodyKind::None)
        );
    }

    #[test]
    fn content_length_gives_a_fixed_body() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\n"),
            Ok(BodyKind::Length(5))
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\n\r\n"),
            Ok(BodyKind::Length(0))
        );
    }

    #[test]
    fn transfer_encoding_chunked_gives_a_chunked_body() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Ok(BodyKind::Chunked)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: CHUNKED\r\n\r\n"),
            Ok(BodyKind::Chunked),
            "transfer codings are case-insensitive"
        );
    }

    /// RFC 9112 section 6.3: duplicate Content-Length fields with identical
    /// values may be treated as one.
    #[test]
    fn identical_duplicate_content_length_is_accepted() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\n"
            ),
            Ok(BodyKind::Length(5))
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5, 5\r\n\r\n"),
            Ok(BodyKind::Length(5)),
            "a comma list of identical values is the same case"
        );
    }

    #[test]
    fn http_10_needs_no_host() {
        assert_eq!(decide_raw(b"GET / HTTP/1.0\r\n\r\n"), Ok(BodyKind::None));
    }

    // ---- the rejection table from the spec ----

    /// RFC 9112 section 6.1: both present is unresolvable ambiguity.
    #[test]
    fn rejects_content_length_with_transfer_encoding() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
        // Order on the wire must not change the outcome.
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
    }

    #[test]
    fn rejects_conflicting_content_length() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"
            ),
            Err(FramingError::DuplicateContentLength)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5, 6\r\n\r\n"),
            Err(FramingError::DuplicateContentLength)
        );
    }

    #[test]
    fn rejects_malformed_content_length() {
        for raw in [
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: abc\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: \r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: +5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: -5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5x\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 0x5\r\n\r\n"[..],
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5 5\r\n\r\n"[..],
            // u64 overflow must not wrap.
            &b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 99999999999999999999999\r\n\r\n"[..],
        ] {
            let h = parse_head(&Bytes::copy_from_slice(raw), &Limits::default())
                .unwrap()
                .unwrap()
                .0;
            assert_eq!(
                decide(&h, &Limits::default()),
                Err(FramingError::InvalidContentLength),
                "should have rejected: {}",
                String::from_utf8_lossy(raw)
            );
        }
    }

    /// RFC 9112 section 6.1: chunked must be the final coding, or the message
    /// length is undetermined.
    #[test]
    fn rejects_chunked_not_final() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked, gzip\r\n\r\n"
            ),
            Err(FramingError::ChunkedNotFinal)
        );
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n\r\n"
            ),
            Err(FramingError::ChunkedNotFinal),
            "codings accumulate across repeated fields"
        );
    }

    /// Two chunked codings would mean two framing layers; reject rather than
    /// pick one.
    #[test]
    fn rejects_repeated_chunked() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked, chunked\r\n\r\n"
            ),
            Err(FramingError::ChunkedNotFinal)
        );
    }

    #[test]
    fn rejects_unsupported_transfer_coding() {
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: gzip\r\n\r\n"),
            Err(FramingError::UnsupportedTransferEncoding)
        );
        assert_eq!(
            decide_raw(b"POST / HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: identity\r\n\r\n"),
            Err(FramingError::UnsupportedTransferEncoding)
        );
    }

    #[test]
    fn rejects_missing_or_multiple_host_on_http_11() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\n\r\n"),
            Err(FramingError::MissingHost)
        );
        assert_eq!(
            decide_raw(b"GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"),
            Err(FramingError::MultipleHost)
        );
    }

    /// Multiple Host fields are ambiguous regardless of version.
    #[test]
    fn rejects_multiple_host_on_http_10_too() {
        assert_eq!(
            decide_raw(b"GET / HTTP/1.0\r\nHost: a\r\nHost: b\r\n\r\n"),
            Err(FramingError::MultipleHost)
        );
    }

    #[test]
    fn rejects_oversized_declared_body() {
        let limits = Limits {
            max_body_bytes: 4,
            ..Default::default()
        };
        let h = head(b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\n\r\n");
        assert_eq!(decide(&h, &limits), Err(FramingError::BodyTooLarge));
    }

    // ---- ordering: the conflict check must win ----

    /// A request carrying both a *valid* Content-Length and a *valid*
    /// Transfer-Encoding must be rejected as a conflict, never resolved in
    /// favor of one. This ordering is the smuggling defense.
    #[test]
    fn conflict_check_precedes_individual_analysis() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding),
            "must not report ChunkedNotFinal or a Length body"
        );
    }

    /// An invalid Content-Length alongside a Transfer-Encoding is still first
    /// and foremost a conflict.
    #[test]
    fn conflict_reported_even_when_content_length_is_garbage() {
        assert_eq!(
            decide_raw(
                b"POST / HTTP/1.1\r\nHost: a\r\nContent-Length: abc\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(FramingError::LengthAndTransferEncoding)
        );
    }

    #[test]
    fn status_codes_map_correctly() {
        assert_eq!(FramingError::LengthAndTransferEncoding.status(), 400);
        assert_eq!(FramingError::DuplicateContentLength.status(), 400);
        assert_eq!(FramingError::InvalidContentLength.status(), 400);
        assert_eq!(FramingError::ChunkedNotFinal.status(), 400);
        assert_eq!(FramingError::MissingHost.status(), 400);
        assert_eq!(FramingError::MultipleHost.status(), 400);
        assert_eq!(FramingError::UnsupportedTransferEncoding.status(), 501);
        assert_eq!(FramingError::BodyTooLarge.status(), 413);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p armature-h1 framing`
Expected: FAIL — `cannot find function decide in this scope`.

- [ ] **Step 3: Implement `framing`**

Prepend to `armature-h1/src/framing.rs`:

```rust
//! Message body framing: the decision of how long the request body is.
//!
//! This is the most security-critical code in the crate. Request smuggling is,
//! in essence, two HTTP implementations disagreeing about where one message
//! ends and the next begins — so this module resolves nothing ambiguously.
//! Every unclear case is an error, and every error closes the connection.
//!
//! The function is deliberately pure and synchronous so it can be exhaustively
//! unit-tested and differentially fuzzed against another implementation.

use crate::header::HeaderId;
use crate::{Head, Limits, Version};

/// How to read the request body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// No body. A request without `Content-Length` or `Transfer-Encoding` has
    /// no body — unlike a response, whose length may run to end-of-stream.
    None,
    /// A body of exactly this many bytes.
    Length(u64),
    /// A chunked body, terminated by a zero-length chunk.
    Chunked,
}

/// An unresolvable or unacceptable framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FramingError {
    /// Both `Content-Length` and `Transfer-Encoding` were present.
    #[error("both Content-Length and Transfer-Encoding present")]
    LengthAndTransferEncoding,
    /// `Content-Length` appeared more than once with differing values.
    #[error("conflicting Content-Length values")]
    DuplicateContentLength,
    /// `Content-Length` was not a bare decimal integer.
    #[error("malformed Content-Length")]
    InvalidContentLength,
    /// `chunked` was present but was not the final transfer coding.
    #[error("chunked is not the final transfer coding")]
    ChunkedNotFinal,
    /// A transfer coding other than `chunked` was requested.
    #[error("unsupported transfer coding")]
    UnsupportedTransferEncoding,
    /// HTTP/1.1 requires exactly one `Host`.
    #[error("missing Host")]
    MissingHost,
    /// More than one `Host` is ambiguous at any version.
    #[error("multiple Host fields")]
    MultipleHost,
    /// The declared body exceeded `Limits::max_body_bytes`.
    #[error("declared body too large")]
    BodyTooLarge,
}

impl FramingError {
    /// The status code to answer with before closing the connection.
    #[inline]
    pub fn status(&self) -> u16 {
        match self {
            FramingError::BodyTooLarge => 413,
            FramingError::UnsupportedTransferEncoding => 501,
            _ => 400,
        }
    }
}

/// Decide how to frame the request body.
///
/// Rule order is fixed and load-bearing:
///
/// 1. `Host` validity
/// 2. `Content-Length`-with-`Transfer-Encoding` conflict
/// 3. `Transfer-Encoding` analysis
/// 4. `Content-Length` analysis
/// 5. Declared-size limit
///
/// Step 2 precedes 3 and 4 so that a request carrying a *valid* value of each is
/// rejected outright rather than silently resolved in favor of one. That silent
/// resolution — and disagreement between peers about which one wins — is the
/// smuggling vector.
pub fn decide(head: &Head, limits: &Limits) -> Result<BodyKind, FramingError> {
    // 1. Host. More than one is ambiguous at any version; HTTP/1.1 additionally
    //    requires at least one (RFC 9112 section 3.2).
    match head.count(&HeaderId::Host) {
        0 if head.version == Version::Http11 => return Err(FramingError::MissingHost),
        n if n > 1 => return Err(FramingError::MultipleHost),
        _ => {}
    }

    let has_len = head.count(&HeaderId::ContentLength) > 0;
    let has_te = head.count(&HeaderId::TransferEncoding) > 0;

    // 2. Conflict. Checked before either field is analyzed.
    if has_len && has_te {
        return Err(FramingError::LengthAndTransferEncoding);
    }

    // 3. Transfer-Encoding. Codings accumulate across repeated fields in wire
    //    order, exactly as if they had been sent as one comma list.
    if has_te {
        let mut codings: smallvec::SmallVec<[&str; 4]> = smallvec::SmallVec::new();
        for value in head.all(&HeaderId::TransferEncoding) {
            let s = std::str::from_utf8(value).map_err(|_| FramingError::ChunkedNotFinal)?;
            for coding in s.split(',') {
                let coding = coding.trim();
                if !coding.is_empty() {
                    codings.push(coding);
                }
            }
        }

        let chunked_count = codings
            .iter()
            .filter(|c| c.eq_ignore_ascii_case("chunked"))
            .count();

        return match chunked_count {
            // A single chunked coding, and it must be last.
            1 if codings
                .last()
                .is_some_and(|c| c.eq_ignore_ascii_case("chunked")) =>
            {
                if codings.len() == 1 {
                    Ok(BodyKind::Chunked)
                } else {
                    // e.g. `gzip, chunked`: chunked frames the message, but an
                    // inner coding we cannot decode remains. 501 rather than
                    // silently handing the handler compressed bytes.
                    Err(FramingError::UnsupportedTransferEncoding)
                }
            }
            // Present but not final, or present more than once: the message
            // length is undetermined.
            n if n >= 1 => Err(FramingError::ChunkedNotFinal),
            // No chunked coding at all: some coding we do not implement.
            _ => Err(FramingError::UnsupportedTransferEncoding),
        };
    }

    // 4. Content-Length. Every value across every field, and every element of
    //    every comma list, must parse identically (RFC 9112 section 6.3).
    if has_len {
        let mut agreed: Option<u64> = None;
        for value in head.all(&HeaderId::ContentLength) {
            let s = std::str::from_utf8(value).map_err(|_| FramingError::InvalidContentLength)?;
            for element in s.split(',') {
                let n = parse_content_length(element.trim())?;
                match agreed {
                    None => agreed = Some(n),
                    Some(prev) if prev == n => {}
                    Some(_) => return Err(FramingError::DuplicateContentLength),
                }
            }
        }
        let len = agreed.ok_or(FramingError::InvalidContentLength)?;

        // 5. Declared-size limit, before a single body byte is read.
        if len > limits.max_body_bytes {
            return Err(FramingError::BodyTooLarge);
        }
        return Ok(BodyKind::Length(len));
    }

    // Neither field: a request has no body.
    Ok(BodyKind::None)
}

/// Parse one `Content-Length` element as a bare decimal integer.
///
/// Rejects the empty string, signs, whitespace, non-digits, and overflow.
/// `str::parse::<u64>` would accept a leading `+`, which RFC 9112 does not, so
/// digits are checked explicitly.
#[inline]
fn parse_content_length(s: &str) -> Result<u64, FramingError> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(|b| b.is_ascii_digit()) {
        return Err(FramingError::InvalidContentLength);
    }
    s.parse::<u64>()
        .map_err(|_| FramingError::InvalidContentLength)
}
```

- [ ] **Step 4: Wire into `lib.rs`**

Add `pub mod framing;` (public so the differential fuzz target in Task 16 can call `decide` directly) and:

```rust
pub use framing::{BodyKind, FramingError};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p armature-h1`
Expected: PASS — 66 tests.

- [ ] **Step 6: Verify the gate passes**

Run: `cargo fmt -p armature-h1 && cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add armature-h1/src/framing.rs armature-h1/src/lib.rs
git commit -m "feat(h1): framing::decide, the security core

Pure synchronous resolution of Content-Length vs Transfer-Encoding, with the
full RFC 9112 6.1/6.3 rejection table and 24 tests.

Rule order is load-bearing and documented: the CL-and-TE conflict check runs
before either field is analyzed, so a request carrying a valid value of each is
rejected outright rather than resolved in favor of one. That silent resolution,
and peers disagreeing about which field wins, is request smuggling.

Kept pure and I/O-free so Task 16 can differentially fuzz it against hyper."
```

---

## Tasks 8–18: format note

Tasks 1–7 carry full code because they are the crate's high-risk core: the type
layer whose shape every later task depends on, and the parsing and framing logic
where a subtle error is a security defect. Tasks 8–18 below carry **exact
interface contracts and complete test manifests** instead of full code bodies.
Each named test states the property it holds; the implementer writes the test
from that statement and then the code that satisfies it. Every signature, type,
constant, and error variant a later task depends on is spelled out, so no task
has to guess at a neighbor's API.

Each task follows the same step rhythm as Tasks 1–7: write the tests → run them
and watch them fail → implement → run them and watch them pass → `cargo fmt` and
`cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings` →
commit.

---

### Task 8: `chunked::ChunkedDecoder`

**Files:** Create `armature-h1/src/chunked.rs`; modify `lib.rs`.

**Interfaces:**
- Consumes: `HeaderId`, `HeaderVec`, `ByteStr`, `Limits`.
- Produces:
  - `enum ChunkEvent { Data(Bytes), Trailers(HeaderVec), End }`
  - `enum ChunkedError { BadSize, SizeOverflow, MissingCrlf, BodyTooLarge, BadTrailer, ForbiddenTrailer, BadExtension, TrailerTooLarge }` with `status(&self) -> u16` (all 400 except `BodyTooLarge` → 413).
  - `struct ChunkedDecoder` with `new(limits: &Limits) -> Self` and
    `poll(&mut self, buf: &mut Bytes) -> Result<Option<ChunkEvent>, ChunkedError>`.
    `Ok(None)` means more input is needed. `poll` consumes from the front of
    `buf` via `Bytes::advance`, and `Data` events are `slice`s of `buf` — the
    decoder copies nothing.
  - `ChunkedDecoder::decoded_len(&self) -> u64` — running total, checked against
    `Limits::max_body_bytes` as data arrives, not merely as declared.

**Internal states:** `Size` → `Extension` → `Data` → `DataCrlf` → (`Size` |
`Trailers`) → `Done`. Chunk size is hex, at most 16 digits, overflow-checked.
Extensions are skipped to the line end without interpretation. A zero-size chunk
transitions to `Trailers`.

**Test manifest** (each a `#[test]`):

| Test | Property |
|---|---|
| `decodes_a_single_chunk` | `5\r\nhello\r\n0\r\n\r\n` yields `Data("hello")`, `Trailers(empty)`, `End` |
| `decodes_multiple_chunks` | two chunks arrive as two `Data` events in order |
| `data_events_share_the_input_buffer` | pointer-range assertion: `Data` payloads point into `buf`, proving no copy |
| `handles_split_across_reads` | feeding one byte at a time yields the same event sequence; `poll` returns `Ok(None)` mid-chunk |
| `hex_size_is_case_insensitive` | `1F\r\n…` and `1f\r\n…` both give a 31-byte chunk |
| `skips_chunk_extensions` | `5;name=value\r\nhello\r\n` decodes normally |
| `parses_trailers` | `0\r\nEtag: x\r\n\r\n` yields `Trailers` containing `etag: x` |
| `rejects_non_hex_size` | `zz\r\n` → `BadSize` |
| `rejects_empty_size` | `\r\n` → `BadSize` |
| `rejects_size_overflow` | 17 hex digits → `SizeOverflow` |
| `rejects_missing_crlf_after_data` | `5\r\nhelloXX` → `MissingCrlf` |
| `rejects_bare_lf_in_framing` | `5\nhello\r\n` → `MissingCrlf`, matching the strict-CRLF policy of Task 5 |
| `enforces_running_body_limit` | with `max_body_bytes: 4`, a 5-byte chunk → `BodyTooLarge` even though nothing was declared up front |
| `enforces_body_limit_across_chunks` | three 2-byte chunks under `max_body_bytes: 4` → `BodyTooLarge` on the third |
| `rejects_forbidden_trailer_fields` | a `Transfer-Encoding` or `Content-Length` trailer → `ForbiddenTrailer`, exercising `HeaderId::forbidden_in_trailers` from Task 3 |
| `rejects_obs_fold_in_trailers` | `0\r\nEtag: a\r\n b\r\n\r\n` → `BadTrailer` |
| `enforces_trailer_size_limit` | a trailer section past `max_head_bytes` → `TrailerTooLarge` |
| `end_is_terminal` | `poll` after `End` returns `Ok(Some(End))` and never consumes further input |

The forbidden-trailer test is the security-relevant one: framing was already
decided before the trailer section was read, so honoring a framing field from a
trailer is a smuggling vector (RFC 9110 §6.5.1).

---

### Task 9: `pool::BufPool`

**Files:** Create `armature-h1/src/pool.rs`; modify `lib.rs`.

**Interfaces:**
- Produces:
  - `struct BufPool { /* private */ }` — deliberately **not** `Sync`; one lives per worker thread.
  - `BufPool::new(buf_cap: usize, max_free: usize) -> Self`
  - `take(&mut self) -> BytesMut` — a cleared buffer with at least `buf_cap` capacity.
  - `give(&mut self, buf: BytesMut)` — returns a buffer for reuse.
  - `misses(&self) -> u64`, `free_len(&self) -> usize`

**Implementation note:** `give` must call `BytesMut::try_reclaim(buf_cap)` and
only retain the buffer when it returns `true`. A buffer whose `Bytes` slices are
still alive in a handler cannot be reused without corrupting them; `try_reclaim`
is the exact API for that check (it succeeds only when the buffer is uniquely
owned). When it fails, drop the buffer and increment `misses`, which is how
handler-side buffer pinning becomes observable rather than silent.

**Test manifest:**

| Test | Property |
|---|---|
| `take_returns_cleared_buffer_with_capacity` | length 0, capacity ≥ `buf_cap` |
| `give_then_take_reuses_the_allocation` | pointer equality across a give/take round trip |
| `retains_at_most_max_free` | giving `max_free + 3` buffers keeps `free_len() == max_free` |
| `refuses_to_reuse_a_pinned_buffer` | hold a frozen `Bytes` slice, `give` the parent, assert `misses() == 1` and `free_len() == 0` |
| `reuses_once_the_slice_is_dropped` | drop the slice first, then `give`; assert `misses() == 0` and the buffer is retained |
| `take_on_empty_pool_allocates` | `free_len() == 0` still yields a usable buffer |

---

### Task 10: `write` — response serialization

**Files:** Create `armature-h1/src/write.rs`; modify `lib.rs`.

**Interfaces:**
- Produces:
  - `struct DateCache { /* private */ }` with `new() -> Self` and
    `get(&mut self, now: SystemTime) -> &[u8]` — a 29-byte IMF-fixdate,
    reformatted only when the whole second changes. `now` is a parameter rather
    than read internally so the cache is testable without a clock.
  - `enum OutBody { None, Fixed(Bytes), Chunked }`
  - `struct ResponseHead { pub status: u16, pub headers: HeaderVec }`
  - `fn reason_phrase(status: u16) -> &'static str` — the RFC 9110 registry, `""` for unregistered codes.
  - `fn write_u64(out: &mut BytesMut, v: u64)` — decimal, no allocation, no `format!`.
  - `fn write_head(out: &mut BytesMut, version: Version, resp: &ResponseHead, body: &OutBody, date: &[u8], keep_alive: bool)`
  - `fn write_chunk(out: &mut BytesMut, data: &[u8])`
  - `fn write_last_chunk(out: &mut BytesMut, trailers: &HeaderVec)`

**Implementation notes:** `write_head` emits `Date` and `Connection` itself, and
must not emit a second copy of either if the handler already supplied one — a
duplicate `Content-Length` on a response is the mirror image of the request
smuggling this crate rejects. `OutBody::Fixed` emits `Content-Length`;
`OutBody::Chunked` emits `Transfer-Encoding: chunked`; `OutBody::None` emits
`Content-Length: 0` except on 204 and 304, which must carry neither framing
field.

**Test manifest:**

| Test | Property |
|---|---|
| `writes_a_minimal_200` | exact byte-for-byte output compared against a literal |
| `write_u64_matches_format` | 0, 1, 9, 10, 99, 100, `u64::MAX` all match `v.to_string()` |
| `reason_phrases_are_correct` | 200/201/204/301/304/400/404/408/413/431/500/501/505 spot-checked; 599 gives `""` |
| `fixed_body_emits_content_length` | `Content-Length: 5` present, no `Transfer-Encoding` |
| `chunked_body_emits_transfer_encoding` | `Transfer-Encoding: chunked` present, no `Content-Length` |
| `empty_body_emits_zero_length` | `Content-Length: 0` |
| `no_framing_fields_on_204_or_304` | neither `Content-Length` nor `Transfer-Encoding` appears |
| `does_not_duplicate_handler_supplied_date` | one `Date` field only |
| `does_not_duplicate_handler_supplied_content_length` | one `Content-Length` only |
| `emits_connection_close_when_not_keep_alive` | `Connection: close` present |
| `omits_connection_header_when_keep_alive_on_http11` | absent, since persistence is the HTTP/1.1 default |
| `emits_connection_keep_alive_on_http10` | present, since closing is the HTTP/1.0 default |
| `date_cache_reformats_only_on_second_change` | two `get` calls in the same second return the identical slice; a call one second later differs |
| `date_format_is_imf_fixdate` | 29 bytes matching `Sun, 06 Nov 1994 08:49:37 GMT` shape |
| `write_chunk_frames_correctly` | `5\r\nhello\r\n` |
| `write_last_chunk_without_trailers` | `0\r\n\r\n` |
| `write_last_chunk_with_trailers` | `0\r\nEtag: x\r\n\r\n` |

---

### Task 11: `deadline::ConnDeadline`

**Files:** Create `armature-h1/src/deadline.rs`; modify `lib.rs`.

**Interfaces:**
- Produces:
  - `struct ConnDeadline { /* private */ }`
  - `ConnDeadline::new(tick: Duration) -> Self`
  - `arm(&mut self, after: Duration)` — reset to `now + after`, rounded up to the next `tick` boundary.
  - `disarm(&mut self)` — reset far into the future.
  - `async fn expired(&mut self)` — completes when the armed deadline passes.

**Implementation note:** one `Pin<Box<tokio::time::Sleep>>` per connection,
reused across every phase via `Sleep::reset`. This is the spec's per-request
timer allocation removed; a bespoke timing wheel is not built, because tokio's
timer driver already *is* a hierarchical wheel and duplicating it would add code
without adding capability. Coarsening to a 100 ms tick is what keeps the wheel's
slot churn low.

**Test manifest** (all under `#[tokio::test(start_paused = true)]`, using
`tokio::time::advance` — no real sleeping, so the suite stays fast and
deterministic):

| Test | Property |
|---|---|
| `expires_after_the_armed_duration` | advancing past the deadline completes `expired()` |
| `does_not_expire_early` | `expired()` is still pending just before the deadline |
| `rearming_extends_the_deadline` | arm, advance halfway, re-arm, assert the original deadline passes without expiry |
| `disarm_prevents_expiry` | advancing an hour after `disarm` leaves `expired()` pending |
| `coarsens_up_to_the_tick_boundary` | arming for 10 ms with a 100 ms tick expires at 100 ms, not 10 ms |
| `reuses_one_timer_across_many_arms` | 1000 arm/expire cycles complete, and the struct is never reconstructed |

---

### Task 12: `service` — `Request`, `Response`, `Body`, `H1Service`

**Files:** Create `armature-h1/src/service.rs`; modify `lib.rs`.

**Interfaces:**
- Produces:
  - `trait Transport: AsyncRead + AsyncWrite + Unpin {}` with a blanket impl for every `T: AsyncRead + AsyncWrite + Unpin`, so `Upgraded` need not be generic and `Response` need not carry a type parameter.
  - `struct Upgraded { pub io: Box<dyn Transport>, pub buffered: Bytes }` — `buffered` carries bytes already read past the head, which the upgrade consumer must process before reading the socket. Dropping them is a silent data-loss bug, so the field is public and documented as mandatory.
  - `struct Body { /* private */ }` with `poll_chunk(&mut self, cx) -> Poll<Option<Result<Bytes, BodyError>>>`, `async fn chunk(&mut self) -> Option<Result<Bytes, BodyError>>`, `async fn collect(&mut self, cap: u64) -> Result<Bytes, BodyError>`, `is_end(&self) -> bool`, `trailers(&self) -> Option<&HeaderVec>`, and `Body::empty()`.
  - `enum BodyError { Chunked(ChunkedError), Io(std::io::Error), Incomplete, TooLarge }`
  - `struct Request { pub head: Head, pub body: Body }`
  - `enum ResponseBody { Empty, Full(Bytes), Stream(Pin<Box<dyn Stream<Item = Result<Bytes, BodyError>>>>) }` — note the absence of `Send`.
  - `struct Response { pub status: u16, pub headers: HeaderVec, pub body: ResponseBody, pub upgrade: Option<Box<dyn FnOnce(Upgraded)>> }` with `Response::new(status)`, `ok()`, `text(&str)`, `json(Bytes)`, `status_only(u16)`, and `header(self, HeaderId, Bytes) -> Self`.
  - `trait H1Service { type Future: Future<Output = Response>; fn call(&self, req: Request) -> Self::Future; }` — **no `Send` bound anywhere**, which is the point of the thread-per-core model.

**Implementation note:** `Body::collect` must enforce its `cap` argument as it
accumulates rather than trusting the declared length, so a lying `Content-Length`
cannot drive unbounded buffering.

**Test manifest:**

| Test | Property |
|---|---|
| `empty_body_ends_immediately` | `chunk()` is `None`, `is_end()` is true |
| `fixed_body_yields_declared_bytes` | a `Length(5)` body over a mock transport yields exactly `"hello"` |
| `fixed_body_short_read_is_incomplete` | a transport closing early gives `BodyError::Incomplete`, never a truncated success |
| `chunked_body_yields_chunks_and_trailers` | chunk sequence then `trailers()` populated |
| `collect_enforces_its_cap` | a body larger than `cap` gives `BodyError::TooLarge` |
| `collect_ignores_a_lying_content_length` | declared 5, sends 500 → `TooLarge`/`Incomplete`, never 500 bytes buffered |
| `response_builders_set_expected_fields` | `text`, `json`, `status_only`, `header` |
| `service_future_need_not_be_send` | a service whose future captures an `Rc<Cell<u32>>` compiles and runs — a compile-time assertion that the `Send` bound is genuinely absent |

That last test is the one that keeps the design honest: if anyone reintroduces a
`Send` bound anywhere in the chain, it stops compiling.

---

### Task 13: `conn` — the connection state machine

**Files:** Create `armature-h1/src/conn.rs`; modify `lib.rs`.

**Interfaces:**
- Consumes: everything from Tasks 1–12.
- Produces:
  - `struct ConnConfig { pub limits: Limits, pub tick: Duration, pub server_name: Option<Bytes> }`
  - `struct Connection<IO, S> { /* private */ }` with `new(io: IO, service: S, cfg: Rc<ConnConfig>, pool: Rc<RefCell<BufPool>>, date: Rc<RefCell<DateCache>>) -> Self` and `async fn serve(self) -> Result<Option<Upgraded>, std::io::Error>` — `Ok(Some(_))` means the connection was upgraded and the caller must hand it to the upgrade consumer.
  - `enum Disposition { KeepAlive, Close, Upgrade }` (crate-internal, but named here because the tests assert on it).

**Behavior the tests pin down:**
- Serve loop: read → `find_head_end` → `parse_head` → `framing::decide` → build `Request` → `service.call` → write → decide disposition.
- On any `ParseError` or `FramingError`: write the mapped status with `Connection: close` and an empty body, then close. Never resynchronize.
- `Expect: 100-continue`: the interim `100 Continue` is written when the handler first polls the body, not when the head is parsed. A handler that returns without reading the body must produce **no** interim response — that is the whole value of lazy 100-continue, and it gets its own test.
- `HEAD`: response headers are written including the `Content-Length` the equivalent `GET` would carry, and the body is suppressed.
- Pipelining: requests already in the buffer are served in order; responses coalesce into one write when more are pending; reaching `max_pipeline_depth` stops reading rather than responding.
- Upgrade: a `Response` with `upgrade: Some(_)` and status 101 causes the loop to stop, returning `Upgraded { io, buffered }` with every unconsumed byte.
- Deadlines: `header_timeout` armed on first byte, `body_timeout` on head parse, `idle_timeout` between requests, `write_timeout` around writes. Expiry writes 408 where a response is still possible and closes.

**Test manifest** (over `tokio::io::duplex`, so real async reads and partial writes are exercised):

| Test | Property |
|---|---|
| `serves_a_single_request` | one request in, one well-formed response out |
| `keeps_the_connection_alive` | two sequential requests on one connection both answered |
| `closes_on_connection_close` | connection closes after the response |
| `closes_after_http_10_without_keep_alive` | closes by default |
| `serves_pipelined_requests_in_order` | three requests written at once produce three responses in order |
| `coalesces_pipelined_responses_into_one_write` | a write-counting transport records fewer writes than responses |
| `stops_reading_at_max_pipeline_depth` | with depth 2, the third request is not parsed until the first completes |
| `sends_100_continue_when_body_is_read` | interim response precedes the final one |
| `omits_100_continue_when_body_is_ignored` | a handler returning 401 without reading the body produces exactly one response |
| `head_response_has_headers_but_no_body` | `Content-Length: 5` present, zero body bytes on the wire |
| `parse_error_yields_400_and_closes` | bare LF request → `400`, `Connection: close`, socket closed |
| `framing_error_yields_mapped_status_and_closes` | CL+TE → `400` and close; oversized → `413` and close |
| `does_not_resynchronize_after_a_framing_error` | bytes following a rejected request are never interpreted as a second request |
| `header_timeout_yields_408` | a partial head that stalls gets 408 |
| `body_timeout_yields_408` | a stalled body gets 408 |
| `idle_timeout_closes_silently` | no response is written on idle expiry |
| `returns_upgraded_with_buffered_bytes` | bytes sent after the upgrade request are present in `Upgraded::buffered` |
| `handler_panic_does_not_poison_the_connection` | a panicking handler yields 500 and closes cleanly rather than aborting the worker |

---

### Task 14: `server` — bind, shard, pin, shut down

**Files:** Create `armature-h1/src/server.rs`, `armature-h1/examples/hello.rs`; modify `lib.rs`.

**Interfaces:**
- Produces:
  - `struct TcpConfig { pub nodelay: bool, pub backlog: i32, pub reuse_port: bool }` — defaults `nodelay: true`, `backlog: 1024`, `reuse_port: true`.
  - `struct Config { pub addr: SocketAddr, pub workers: usize, pub limits: Limits, pub tcp: TcpConfig, pub tick: Duration, pub pin_cores: bool, pub server_name: Option<Bytes>, pub shutdown_grace: Duration }` with `Config::new(addr)` defaulting `workers` to the available parallelism.
  - `struct Server { /* private */ }` with `Server::bind(cfg: Config) -> io::Result<Server>`, `handle(&self) -> ServerHandle`, `local_addr(&self) -> SocketAddr`, and
    `serve<F, S>(self, make: F) -> io::Result<()> where F: Fn() -> S + Clone + Send + 'static, S: H1Service + 'static`.
  - `struct ServerHandle { /* private */ }` — `Clone + Send`, with `shutdown(&self)` and `async fn wait_idle(&self)`.

**Implementation notes:**
- The `make` factory is `Send` but the `S` it produces is not: the factory crosses thread boundaries once at startup, the service never does. This is what lets per-core service state be non-atomic.
- `local_addr` must report the concrete bound port so tests can bind port 0.
- Unix: one `socket2::Socket` per worker with `SO_REUSEPORT` and `SO_REUSEADDR`, each converted to a `tokio::net::TcpListener`, so the kernel load-balances accepts and no connection migrates cores.
- Windows: `SO_REUSEPORT` does not exist, so fall back to one shared `Arc<TcpListener>` accepted from by every worker. `TcpListener::accept` takes `&self`, so this needs no lock. Gate with `#[cfg]` and assert the fallback in a test that runs on all platforms.
- Core pinning via `core_affinity::set_for_current`, skipped when `pin_cores` is false or the core count is below `workers`.
- Shutdown: a `tokio::sync::watch` channel — `Send`, so it crosses into each worker's runtime — stops accepts, then drains in-flight connections for `shutdown_grace` before dropping them.

**Test manifest:**

| Test | Property |
|---|---|
| `binds_and_serves_on_an_ephemeral_port` | `local_addr().port() != 0`, and a real client request round-trips |
| `serves_concurrent_connections` | 64 concurrent clients all get correct responses |
| `distributes_across_workers` | with 4 workers and per-core counters, more than one worker records traffic |
| `per_core_state_is_isolated` | a service holding `Cell<u32>` counts only its own core's requests, proving the factory ran per worker |
| `shutdown_stops_accepting` | after `shutdown()`, new connections are refused |
| `shutdown_drains_in_flight_requests` | a request in progress at shutdown still receives its response |
| `shutdown_is_idempotent` | calling it twice does not panic |
| `wait_idle_resolves_after_drain` | resolves once connections are gone |
| `single_worker_config_works` | `workers: 1` serves correctly, the degenerate case |
| `example_compiles` | `cargo build -p armature-h1 --example hello` succeeds |

`examples/hello.rs` is a complete runnable server returning `"Hello, world!"`,
serving as the crate's smoke test and doc example.

---

### Task 15: `tls` — rustls acceptor, ALPN, and h2c dispatch

**Files:** Create `armature-h1/src/tls.rs`; modify `Cargo.toml` (already has the feature), `lib.rs`, `server.rs`.

**Interfaces:**
- Produces (all behind `#[cfg(feature = "tls")]` except `H2Fallback`):
  - `trait H2Fallback { fn handle(&self, io: Box<dyn Transport>, buffered: Bytes) -> Pin<Box<dyn Future<Output = ()>>>; }` — **not** feature-gated, because plaintext h2c dispatch needs it without TLS.
  - `struct TlsConfig { pub cert_chain: Vec<Vec<u8>>, pub key_der: Vec<u8>, pub alpn: Vec<Vec<u8>> }` defaulting `alpn` to `[b"http/1.1"]`.
  - `Config::with_tls(self, TlsConfig) -> Self` and `Config::with_h2_fallback(self, Rc<dyn H2Fallback>) -> Self`.
  - `const H2C_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";`
  - `fn is_h2c_preface(buf: &[u8]) -> Preface` where `enum Preface { Http1, Http2, NeedMore }` — a prefix shorter than the preface but consistent with it is `NeedMore`, so dispatch never guesses on a partial first read.

**Implementation notes:** the rustls `ServerConfig` is built once per process and
shared as an `Arc` across workers — read-only after construction, so no
per-request atomic traffic. When ALPN negotiates `h2`, the `TlsStream` goes to
`H2Fallback` untouched. When no fallback is configured and a peer insists on h2,
close rather than mis-serve it as HTTP/1.

**Test manifest:**

| Test | Property |
|---|---|
| `is_h2c_preface_detects_http1` | `GET / HTTP/1.1\r\n` → `Http1` |
| `is_h2c_preface_detects_the_preface` | the exact preface → `Http2` |
| `is_h2c_preface_needs_more_on_a_short_prefix` | `PRI * HTTP/2` → `NeedMore`, never a guess |
| `is_h2c_preface_rejects_a_divergent_prefix` | `PRX` → `Http1` |
| `h2c_preface_dispatches_to_the_fallback` | a recording fallback receives the connection with the preface intact in `buffered` |
| `h2c_without_a_fallback_closes` | no response bytes, socket closed |
| `alpn_h2_dispatches_to_the_fallback` | a rustls client offering `h2` reaches the fallback |
| `alpn_http11_serves_normally` | a client offering `http/1.1` gets an HTTP/1.1 response |
| `tls_request_round_trips` | full rustls handshake plus request and response over loopback with a self-signed cert generated in the test |
| `tls_config_is_shared_not_cloned_per_connection` | `Arc::strong_count` does not grow per connection |

---

### Task 16: RFC 9112 conformance suite

**Files:** Create `armature-h1/tests/rfc9112/mod.rs` plus `framing.rs`, `syntax.rs`, `limits.rs`, `semantics.rs`, and `armature-h1/tests/rfc9112.rs` as the entry point.

These run against a **real server on a real socket** via `Server::bind` on port
0, not against the parser directly. Tasks 5–7 already unit-test the pure
functions; this suite exists to prove the wiring — that a rejection reached by
`framing::decide` actually produces the right bytes on the wire and actually
closes the connection. A raw `TcpStream` is used rather than an HTTP client,
because a client library would normalize away the very malformations under test.

**Interfaces:** a shared helper `fn raw_exchange(req: &[u8]) -> (Vec<u8>, bool)`
returning the response bytes and whether the server closed.

**Test manifest** — one test per row of the spec's rejection table, plus the
positive cases:

*`framing.rs`* — `content_length_and_transfer_encoding_400_close`,
`conflicting_content_length_400_close`,
`comma_list_content_length_conflict_400_close`,
`chunked_not_final_400_close`, `unsupported_transfer_coding_501_close`,
`missing_host_400_close`, `multiple_host_400_close`,
`identical_duplicate_content_length_accepted`, `chunked_request_round_trips`,
`chunked_with_trailers_round_trips`, `forbidden_trailer_400_close`.

*`syntax.rs`* — `bare_cr_400_close`, `bare_lf_400_close`, `obs_fold_400_close`,
`whitespace_before_colon_400_close`, `bad_request_line_400_close`,
`http_12_505_close`, `absolute_form_target_accepted`,
`asterisk_form_options_accepted`, `non_utf8_target_400_close`.

*`limits.rs`* — `oversized_head_431_close`, `too_many_headers_431_close`,
`oversized_declared_body_413_close`, `oversized_chunked_body_413_close`,
`header_timeout_408_close`, `body_timeout_408_close`,
`idle_timeout_closes_without_response`,
`pipeline_depth_applies_backpressure`.

*`semantics.rs`* — `keep_alive_serves_sequential_requests`,
`pipelined_requests_answered_in_order`,
`expect_100_continue_interim_then_final`,
`expect_100_continue_omitted_when_body_unread`,
`head_returns_headers_without_body`,
`connect_method_reaches_the_handler`,
`upgrade_hands_off_buffered_bytes`, `http_10_closes_by_default`,
`http_10_keep_alive_honored`, `no_framing_headers_on_204`,
`no_framing_headers_on_304`.

Every `_close` test asserts **both** the status code and that the socket was
closed — a correct status on a connection left open is still a smuggling
vector, so asserting only the status would let the real defect through.

---

### Task 17: Fuzzing

**Files:** Create `armature-h1/fuzz/Cargo.toml`, `fuzz/fuzz_targets/parse_head.rs`, `fuzz/fuzz_targets/chunked.rs`, `fuzz/fuzz_targets/framing_differential.rs`; modify `.github/workflows/ci.yml`.

**Targets:**
1. `parse_head` — arbitrary bytes into `parse_head`. Properties: never panics; on
   `Ok(Some((head, n)))`, `n <= input.len()` and every `Bytes` in `head` lies
   within the input's address range; `Ok(None)` only when `find_head_end` is
   `None` or the head is within limits.
2. `chunked` — arbitrary bytes into `ChunkedDecoder`, fed in arbitrary split
   points derived from the input. Properties: never panics; `decoded_len` never
   exceeds `max_body_bytes`; feeding the same input in different split patterns
   produces the identical event sequence. That last property is the one that
   catches state-machine bugs a single-shot fuzz run cannot.
3. `framing_differential` — the load-bearing one. Feed the same bytes to
   `framing::decide` and to hyper's HTTP/1 server over a `tokio::io::duplex`,
   then compare the **accept/reject decision and the resulting body length**. A
   divergence is a smuggling vector by definition, so this catches the class of
   defect that code review reliably misses. Divergences are recorded as
   reproducible corpus entries rather than merely failing.

`hyper` is a dev-dependency of the fuzz crate only, never of `armature-h1`
itself.

**CI:** a `fuzz-smoke` job running each target for 60 seconds on pull requests
that touch `armature-h1/`, so a regression in the parser or framing logic fails
the build rather than waiting for a scheduled long run.

---

### Task 18: Benchmarks, the allocation regression test, and docs

**Files:** Create `armature-h1/benches/parse.rs`, `benches/write.rs`, `benches/e2e.rs`, `armature-h1/tests/alloc_regression.rs`, `armature-h1/README.md`, `scripts/bench-h1.sh`; modify `armature-h1/Cargo.toml`, `lib.rs`.

**The allocation regression test is the load-bearing deliverable of this task**,
and arguably of the whole plan. Without it the zero-allocation property is an
assertion in a design document that decays on the first careless commit.

- A counting global allocator (`struct Counting; impl GlobalAlloc`) wrapping the
  system allocator with atomic counters. This is the one place `unsafe` is
  unavoidable — `GlobalAlloc` is an unsafe trait — so it lives in
  `tests/alloc_regression.rs`, a test target, **not** in the crate. `lib.rs`
  keeps `#![forbid(unsafe_code)]` intact.
- Test `steady_state_keepalive_get_allocates_nothing`: warm the pools with 100
  requests, snapshot the counter, serve 100 more keep-alive `GET`s, assert the
  delta is zero.
- Test `chunked_request_allocates_nothing_in_steady_state`: the same for a
  chunked body.
- Test `pipelined_requests_allocate_nothing_in_steady_state`: the same for four
  pipelined requests.
- Each asserts an exact zero rather than a threshold. A threshold is a slow leak
  waiting to be tolerated.

**Benchmarks** (criterion): `parse_head` on a small GET, a browser-sized GET, and
a 64-header request; `HeaderId::from_bytes` for a well-known and a custom name;
`framing::decide`; `write_head`; `write_u64` against `format!`; and an end-to-end
`serve` over `duplex`.

**`scripts/bench-h1.sh`** runs `oha` against the `hello` example and against an
equivalent hyper server, printing both results side by side, so the comparison in
the spec is reproducible on demand rather than a claim. It must print the exact
command lines and versions it used.

**`README.md`** covers what the crate is, the thread-per-core model and its
head-of-line-blocking tradeoff, the strict-CRLF divergence from RFC 9112 §2.2 and
why, the buffer-pinning caveat with `h1_pool_miss`, and a complete `hello`
example.

**Final gate for the whole plan:**

```bash
cargo test -p armature-h1 --all-features
cargo test -p armature-h1 --all-features --release
cargo clippy -p armature-h1 --all-targets --all-features -- -D warnings
cargo fmt -p armature-h1 -- --check
cargo build -p armature-h1 --example hello
```

---

## Self-Review

**Spec coverage.** Section 1 (type layer) → Tasks 1–3, 6. Section 3 (server
model) → Tasks 11, 14. Section 3 (protocol dispatch) → Task 15. Section 4 (crate
layout) → the File Structure table; `driver.rs` was dropped in favor of a plain
`IO: AsyncRead + AsyncWrite` generic on `Connection` plus a `Listener` seam in
`server.rs`, since a bespoke `Driver` trait added an indirection layer with no
second implementation to justify it. Section 5 (protocol correctness) → Tasks
7, 8, 13, 16; every row of the rejection table has a named test. Section 6
(testing) → Tasks 16, 17, 18; the timing wheel became one reusable `Sleep` per
connection (Task 11), which meets the stated goal — no per-request timer
allocation — without reimplementing tokio's own wheel.

**Deviations from the spec, all deliberate and recorded above:** no `driver.rs`;
no bespoke timing wheel; strict CRLF rather than RFC 9112's permitted bare-LF
leniency; `unsafe` appears in exactly one test target for the counting allocator,
leaving the crate itself `forbid(unsafe_code)`.

**Out of scope for this plan, covered by Plans 2–5:** every `armature-core`
change (B1–B8), the serve-path swap, and the `armature-websocket` upgrade
adapter. This plan ends with a standalone crate that has no `armature-*`
dependency and is benchmarked against hyper — deliberately, so the premise is
proven before the breaking change is paid for.

**Type consistency check.** `Limits` is threaded by reference into `parse_head`,
`framing::decide`, and `ChunkedDecoder::new`. `HeaderVec` is the one header list
type across `Head`, trailers, and `Response`. `HeaderId` accessors are free
functions in `header.rs`, forwarded as methods on `Head`. `Transport` is the
single object-safe IO trait, used by both `Upgraded` and `H2Fallback`.
`MAX_HEADERS_CEILING` is defined in `limits.rs` and consumed in `parse.rs`, with
a `const` assertion tying them together.

