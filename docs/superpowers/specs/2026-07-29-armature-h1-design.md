# armature-h1: a zero-allocation HTTP/1.1 server for Armature

**Date:** 2026-07-29
**Status:** Design approved, pending spec review
**Scope:** New crate `armature-h1` + breaking changes to `armature-core` (0.5 → 0.6)

## Summary

Replace hyper as Armature's HTTP/1.1 listener with `armature-h1`, a thread-per-core
embedded server built around a single insight: **the parser is not the bottleneck — the
API shape is.**

A plain `GET /` through five middlewares costs roughly **40 heap allocations** today.
A top-tier HTTP/1.1 server does zero. The measured inventory:

| Source | Allocations | Location |
|---|---|---|
| `method: String`, `path: String` | 2 | `http.rs:17-18` |
| `HeaderMap` — names *and* values are `String` | ~2/header (≈20 typical) | `http.rs:25` |
| `body: Vec<u8>`, copied out of hyper's `Bytes` | 1 + memcpy | `http.rs:30` |
| `query_params: HashMap<String,String>`, eagerly parsed and percent-decoded on **every** request carrying a `?`, read or not | 1 + 2/param | `application.rs:1545` |
| `path_params: HashMap<String,String>` | 1 + 2/param | `http.rs:31` |
| `Middleware::handle` is an `async fn` in a `dyn` trait → `async_trait` boxes a future **per layer, per request** | 5 | `middleware.rs:26-34` |
| `BoxedHandler::call` → `Pin<Box<dyn Future>>` | 1 | `handler.rs:137` |
| `HttpResponse.body: Vec<u8>` → copied into hyper's body | 2 + memcpy | `http.rs:397` |
| hyper's own `http::Request` + `HeaderMap` | ~6 | — |

Parsing is ~5% of that gap. The remaining 95% lives in `armature-core`'s public types and
dispatch mechanics, so this work is inseparably a new crate *and* a breaking change to
`armature-core`.

The target is zero heap allocations on the steady-state path for a keep-alive `GET` served
from pooled per-core buffers.

## Non-goals

- **HTTP/2 and HTTP/3.** `hyper`/`hyper-util` stay in the tree, demoted to HTTP/2 only.
  `http3.rs` (quinn/h3) and `armature-grpc` (h2-only) are untouched. An `armature-h2` crate
  is a separate future effort.
- **Client-side HTTP.** `armature-http-client` is out of scope.
- **Cloud adapters.** `armature-lambda`, `armature-cloudrun`, `armature-azure-functions`
  convert platform events; they do not bind a listener. Untouched except for the mechanical
  `HttpRequest` type migration.
- **Monomorphized middleware.** Full tower-style compile-time layer composition is deferred
  (see B3 below); this spec takes the arena-allocation route instead.

## Targeted specifications

**RFC 9110** (HTTP Semantics), **RFC 9111** (HTTP Caching), **RFC 9112** (HTTP/1.1
Messaging) — the 2022 set obsoleting RFC 7230–7235. No RFC 7230-era behavior; where the two
differ, 9112 wins.

---

## 1. Type layer: zero-copy via `Bytes`, not lifetimes

### The constraint

`armature-core::Handler` takes an owned request:

```rust
fn call(&self, req: HttpRequest) -> Self::Future
```

A `Head<'b>` borrowing the connection read buffer cannot cross that signature. Making it
work means lifetime-generic handler futures propagated through the handler trait, every
extractor, the middleware chain, and the proc-macro surface — across 24 crates and 205
`HttpRequest::new` call sites. Rejected as disproportionate.

### The design

The connection read buffer is a `BytesMut` drawn from a per-core pool. Every header value,
the request target, and the body become `Bytes` handles **into that same allocation** — a
refcount increment, no `memcpy`, no `String`. The result is owned and `'static`, so no
lifetime plumbing, and delivers the same zero-allocation outcome that borrowing would.

```rust
pub struct Head {
    pub method: Method,        // enum, 1 byte
    pub target: ByteStr,       // Bytes + UTF-8 invariant, slice of the read buffer
    pub headers: HeaderVec,    // SmallVec<[(HeaderId, Bytes); 16]>, inline
}

pub enum HeaderId {
    Host, ContentLength, ContentType, Accept, /* ~60 well-known */
    Other(ByteStr),
}
```

Projection from `httparse`'s borrowed slices to `Bytes` uses `Bytes::slice_ref`, which is
safe and panics if the argument is not a subslice — no pointer arithmetic of our own.

`HeaderId` interning is the second win independent of allocation: today's `HeaderMap` does a
case-insensitive string comparison per lookup, which becomes a `u8` comparison.

### Ownership and buffer reclamation

Once a request's `Bytes` are handed to a handler, that buffer segment cannot return to the
per-core pool until every handle drops. A handler stashing a header value in long-lived state
pins the whole buffer. Mitigations:

- Buffers are pool-sized to one typical request head (8 KiB default), so a pinned buffer
  costs bounded memory, not a whole connection's history.
- The pool is capped per core; on exhaustion it falls back to fresh allocation rather than
  blocking, and increments a `h1_pool_miss` counter so pinning shows up in metrics.
- `ByteStr::into_owned()` is documented as the escape hatch for handlers that retain data.

This is the same tradeoff hyper makes and is accepted.

---

## 2. Breaking changes to `armature-core`

Semver-major: `armature-core` 0.5 → 0.6. Each item below is independently testable and
lands as its own commit.

### B1 — `Bytes`-backed request and response types

`HttpRequest.method: String → Method`, `path: String → ByteStr`, `body: Vec<u8> → Bytes`.
`HeaderMap` retains its `get(&str) -> Option<&str>` facade but stores `(HeaderId, Bytes)`
internally. `HttpResponse.body: Vec<u8> → Bytes`.

Blast radius is absorbed by keeping constructors generic, so all 205 existing call sites
compile unchanged:

```rust
HttpRequest::new("GET".to_string(), "/x".to_string())  // impl Into<Method>, impl Into<ByteStr>
req.headers.get("content-type")                        // -> Option<&str>, now Bytes-backed
```

`method_str()` and `body_slice()` cover code that genuinely needs the old representations.
The 21 struct-literal constructions in `armature-core` are migrated by hand.

Kills ≈25 allocations.

### B2 — Lazy query parsing, span-based path params

`application.rs:1545` eagerly parses **and percent-decodes** the full query string into a
`HashMap<String,String>` on every request with a `?`. Most handlers never read it.

- `query_params: HashMap<String,String>` → `req.query() -> Query<'_>`, a view over the raw
  `ByteStr` that parses on first access and memoizes into a request-local `SmallVec`.
- `path_params: HashMap<String,String>` → `SmallVec<[(&'static str, Bytes); 4]>`, populated
  from the router's capture spans against the target buffer. Names are `&'static str` from
  the compiled route pattern, so they never allocate.

Kills two `HashMap`s and all eager percent-decoding. Roughly 30 call sites, mechanical.

### B3 — Arena-allocated middleware futures

`Middleware::handle` is an `async fn` in a `Send + Sync` trait object (`middleware.rs:26-34`),
so `async_trait` boxes a future per layer per request — five allocations in a typical stack.

Two possible fixes:

- **(a) Monomorphized `Layer` stack** composed at build time: zero allocations, but every
  middleware across ~10 crates is rewritten.
- **(b) Per-core bump arena** for the boxed futures: keeps `dyn` dispatch and every existing
  middleware signature, replaces `malloc`/`free` with pointer bumps and a per-request arena
  reset.

**This spec takes (b).** It captures the large majority of the win at near-zero blast
radius. (a) is a separate spec, and (b) does not foreclose it.

The arena is the existing `bumpalo` dependency, reset at the end of each request cycle
rather than per allocation.

### B4 — Unboxed ready-futures

Handlers and middleware completing synchronously — health checks, response-cache hits, 304s,
guard rejections — return through

```rust
enum MaybeReady<T> { Ready(T), Pending(Pin<Box<dyn Future<Output = T>>>) }
```

eliminating the dispatch allocation entirely on the hottest paths. `BoxedHandler::call`
returns `MaybeReady<Result<HttpResponse, Error>>`.

### B5 — `!Send` handlers and middleware

Nothing migrates cores under thread-per-core, so the `Send` bound buys nothing and costs
atomics. Dropping it from `Handler`, `Middleware`, and `IntoHandler` lets middleware use
`Cell`/`RefCell` counters instead of atomics (rate limiter, metrics, circuit breaker) and
removes `Send` proof obligations from every future.

**DI does not become `Rc`-based.** The container is `Arc`-and-`Send + Sync`-bound by the
external `dependency-injector` crate (`container.rs:130-187`); making it `Rc` means replacing
that crate. A better fix needs no fork: **each core resolves every `Injectable` once at
startup into a thread-local slab, and handlers receive `&T` rather than a cloned `Arc<T>`.**
That is strictly better than `Rc` — zero refcount traffic instead of merely non-atomic
refcount traffic. `Rc` still applies to per-request and middleware-local state.

**Migration cost, stated plainly:** `tokio::spawn` in a handler stops compiling. There are 58
such call sites across `examples/`, `templates/`, `src/`, and `armature-core/src/`. We ship

```rust
armature_core::spawn(fut)         // -> spawn_local on the current core's runtime
armature_core::spawn_shared(fut)  // -> Send + 'static, onto a shared multi-thread pool
```

so migration is a mechanical rename, and the docs explain which to pick. Cross-core shared
state becomes explicit, which is a correctness improvement as much as a cost.

hyper 1.x does **not** require `Send` service futures, so the HTTP/2 fallback keeps working
on a `LocalSet`. `armature-grpc` runs its own h2 stack and is unaffected.

### B6 — Direct-to-socket responses

`IntoResponse::write_into(&mut BytesMut)` against the connection's write buffer, so
`serde_json` serializes straight into the socket buffer instead of `Vec` → copy → hyper body
→ copy. Two allocations and two memcpys per response; the single largest win for JSON APIs.
`HttpResponse` gains a blanket `write_into` so existing handlers keep working.

### B7 — Method-indexed router

`Method` enum indexes an array of per-method `matchit` trees. No method string comparison,
better cache locality on dispatch. Near-non-breaking — only `Router` internals change.

### B8 — `Extensions` as a `SmallVec`

`HashMap<TypeId, Arc<dyn Any + Send + Sync>>` (`extensions.rs:45-48`) →
`SmallVec<[(TypeId, Rc<dyn Any>); 8]>`. A linear scan over ≤8 entries beats hashing a
`TypeId`, and it never allocates. The `Send + Sync` bound drops out, following B5.

---

## 3. Server model

N pinned OS threads. Each owns one `current_thread` tokio runtime and its **own**
`SO_REUSEPORT` listener, so the kernel load-balances accepts and no connection ever migrates
cores. Per-core and therefore lock-free and non-atomic:

- read/write `BytesMut` pools
- `Date` header cache, refreshed once per second via `httpdate`
- connection slab
- DI resolution slab (B5)
- route cache

Two mechanisms that specifically beat hyper:

- **Coarse timing wheel** (one slot ≈ 100 ms) per core carrying header, body, idle, and
  keep-alive deadlines, instead of a `tokio::time::Timeout` future allocated per request.
- **Pipelining coalescence**: when further parsed requests are already queued, responses
  accumulate in one scratch buffer and leave in a single `writev`.

An internal `Driver` trait (accept / read / write / timer) sits beneath the tokio
implementation so an `io_uring` driver can land later without restructuring. Exactly one
implementation ships.

### Protocol dispatch

`armature-h1` owns `bind` → accept → TLS → parse → dispatch → write → keep-alive. hyper is
reached as a fallback in two cases:

- TLS ALPN negotiates `h2` → the `TlsStream` is handed to a caller-supplied `H2Fallback`,
  which `armature-core` wires to hyper.
- Plaintext h2c prior-knowledge preface (`PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n`) detected on the
  first read → same fallback, with the peeked bytes forwarded.

---

## 4. Crate layout

```
armature-h1/src/
  lib.rs        #![forbid(unsafe_code)]   (unsafe confined to socket2 / core_affinity)
  server.rs     bind, SO_REUSEPORT sharding, core pinning, graceful shutdown
  driver.rs     Driver trait + tokio implementation
  tls.rs        feature "tls": rustls acceptor, ALPN dispatch to H2Fallback
  conn.rs       per-connection state machine, keep-alive, pipelining
  parse.rs      httparse + memchr -> Head; Bytes::slice_ref projection
  head.rs       Method, ByteStr, HeaderId interning, HeaderVec
  body.rs       Content-Length and chunked decode, trailers, 100-continue
  write.rs      status line and header serialization, chunked encode, writev
  upgrade.rs    Upgraded { io, buffered: Bytes } handoff
  limits.rs     max header bytes/count, body cap, pipeline depth
  timer.rs      per-core timing wheel
  pool.rs       thread-local BytesMut pools
```

Dependencies: `tokio` (rt, net, io-util, time), `httparse`, `memchr`, `bytes`, `smallvec`,
`socket2`, `core_affinity`, `httpdate`, `thiserror`, `tracing`; `rustls` + `tokio-rustls`
behind the `tls` feature. **No `armature-*` dependency** — `armature-core` depends on
`armature-h1`, never the reverse.

`#![forbid(unsafe_code)]` in this crate. `httparse` (SIMD on x86_64) and `memchr` supply the
vectorized scanning; the parser is the highest-risk code in an HTTP server, and this removes
memory-safety defects there by construction.

---

## 5. Protocol correctness

Handled: keep-alive, pipelining, chunked request and response bodies, trailers,
`Expect: 100-continue` (the interim response is sent lazily, when the handler first reads the
body), `HEAD` (headers only, `Content-Length` preserved), `CONNECT`, `Upgrade` → raw socket
handoff, origin- / absolute- / asterisk-form targets, `Connection: close`, HTTP/1.0
semantics.

Rejected per RFC 9112 §6.1 and §6.3. Each row is a named test in `tests/rfc9112/`:

| Condition | Response |
|---|---|
| `Content-Length` and `Transfer-Encoding` both present | 400, close |
| duplicate or conflicting `Content-Length` | 400, close |
| `Transfer-Encoding` where `chunked` is not final | 400, close |
| obs-fold in a header | 400, close |
| whitespace between field name and `:` | 400, close |
| bare CR or bare LF in request line or headers | 400, close |
| missing or multiple `Host` on HTTP/1.1 | 400, close |
| header block over byte limit, or header count over limit | 431, close |
| body over limit | 413, close |
| header, body, or idle deadline exceeded | 408, close |
| pipeline depth exceeded | stop reading (backpressure, no response) |

Every rejection **closes the connection** rather than attempting to resynchronize. That is
the request-smuggling defense: a connection whose framing we do not fully agree on is never
reused.

Slowloris defense: separate header-read, body-read, and idle deadlines, plus a minimum
throughput floor on body reads.

---

## 6. Testing and proof

- **Conformance** — `tests/rfc9112/`, one test per rejection row above plus positive framing
  cases, driven over a real socket pair rather than an in-memory mock, so `writev` batching
  and partial reads are exercised.
- **Fuzzing** — `cargo-fuzz` on the parser, plus a **differential target** asserting our
  framing decisions match hyper's on random input. Smuggling divergence is then caught
  mechanically rather than by review.
- **Miri** — on the parser, pools, and timing wheel.
- **Benchmarks** — criterion micro-benches (parse, header lookup, response write) and an
  end-to-end `oha`/`wrk` harness comparing `armature-h1` against the current hyper path on an
  identical handler. Checked in under `benches/` with a runner script, so the performance
  claim is reproducible rather than asserted.
- **Allocation regression test** — a counting global allocator asserting that a keep-alive
  `GET` through the full middleware stack performs **zero** heap allocations in steady state.
  This is the load-bearing test for the entire design; without it, allocations creep back in.

---

## 7. Sequencing

Each phase ends green — tests, clippy `-D warnings` workspace-wide, and `fmt`.

1. **`armature-h1` standalone.** Types, parser, connection state machine, writer, limits,
   timer wheel, conformance tests, fuzz targets, benches. Benchmarked against hyper *before*
   `armature-core` is touched, so the premise is proven before the breaking change is paid
   for.
2. **`armature-core` type migration.** B1, B2, B7, B8 — `Bytes`-backed types, lazy query,
   method-indexed router, `SmallVec` extensions, plus the workspace-wide mechanical fixups.
3. **Dispatch migration.** B3, B4, B5, B6 — arena futures, `MaybeReady`, `!Send` handlers
   with `spawn`/`spawn_shared` shims and the 58 call-site migration, direct-to-socket
   responses.
4. **Serve-path swap.** `application.rs`, `worker.rs`, `micro.rs` bind through
   `armature-h1`, with hyper retained behind `H2Fallback`. The allocation regression test
   goes green here.
5. **`armature-websocket` upgrade adapter** for `Upgraded { io, buffered }`, replacing the
   hyper upgrade path.

## Open risks

- **Pinned buffers under adversarial handlers** — bounded by pool sizing and observable via
  `h1_pool_miss`, but a handler retaining header values across many connections still
  inflates RSS. Accepted; documented.
- **`!Send` migration reach beyond this workspace** — external users of `armature-core` with
  `tokio::spawn` in handlers face the same mechanical change. Mitigated by the shims and a
  migration note in `CHANGELOG.md`, not eliminated.
- **Thread-per-core head-of-line blocking** — a handler that blocks its core stalls every
  connection on that core, where work-stealing would have hidden it. This is inherent to the
  model; the mitigation is documentation plus keeping `spawn_shared` available for genuinely
  blocking work.
- **HTTP/2 fallback path is now less-travelled.** Demoting hyper to h2-only means the h2 path
  gets less production exercise than it does today. Conformance tests for the ALPN and h2c
  dispatch branches specifically are required, not optional.
