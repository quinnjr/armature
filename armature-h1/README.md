# armature-h1

A zero-allocation, thread-per-core HTTP/1.1 server for the Armature framework.

`#![forbid(unsafe_code)]`. No `armature-*` dependency — this crate stands alone.

## What it is

`armature-h1` owns the whole server side of HTTP/1.1: bind, accept, TLS, parse,
dispatch, write, keep-alive. It is built around one measured observation — for a
framework request path, the parser is roughly 5% of the distance to a top-tier
server, and the *type and dispatch shape* is the other 95%.

So the design target is not "fast parsing". It is **zero heap allocations per
request in the steady state**, which the test suite asserts as an exact number
rather than a claim:

| Request shape | Allocations per request |
|---|---|
| keep-alive `GET` | **0** |
| `GET` with 7 browser headers | **0** |
| `POST` with `Content-Length` | **0** |
| `POST` with `Transfer-Encoding: chunked` | **0** |
| after 500 requests on one connection | **0** (flat) |

The 7-header row matching the 1-header row is the load-bearing evidence: header
count does not drive allocation, because every header value is a `Bytes` slice of
the connection's read buffer rather than a `String`.

See `tests/alloc_regression.rs`. If you change this crate and that test fails,
the change regressed the entire premise.

## Example

```rust,no_run
use armature_h1::{Config, Request, Response, Server};
use std::cell::Cell;
use std::rc::Rc;

fn main() -> std::io::Result<()> {
    let server = Server::bind(Config::new("127.0.0.1:8080".parse().unwrap()))?;
    println!("listening on http://{}", server.local_addr());

    server.serve(|| {
        // One counter per worker thread. No atomic, no lock — nothing migrates
        // cores, so a plain `Cell` is correct. This is the point of the model.
        let served = Rc::new(Cell::new(0u64));
        move |_req: Request| {
            let served = served.clone();
            async move {
                served.set(served.get() + 1);
                Response::text("Hello, world!")
            }
        }
    })
}
```

Run it: `cargo run -p armature-h1 --release --example hello`

## The thread-per-core model, and its cost

N pinned OS threads, each running a `current_thread` runtime. On Unix each thread
owns its own `SO_REUSEPORT` listener, so the kernel load-balances accepts and a
connection never migrates cores. Windows, Solaris, and illumos fall back to
duplicated handles on one shared listener.

Because nothing migrates, per-core state needs no synchronization: the `Date`
cache, route caches, and your own service state are all plain
`Rc`/`Cell`/`RefCell`. Handler futures are **not** required to be `Send`.

**The cost, stated plainly:** a handler that blocks its core stalls every other
connection on that core. A work-stealing runtime would have hidden that; this one
will not. Do not block in a handler — move genuinely blocking work to a shared
pool.

Note the bound asymmetry in `Server::serve`: the service *factory* is `Send`,
because it crosses thread boundaries once at startup; the service it produces is
not, because it never does.

## Deliberate divergences

**Strict CRLF.** RFC 9112 §2.2 permits recognizing a bare LF as a line
terminator. This crate rejects it, along with bare CR and obs-fold, with 400 and
a close. Request smuggling *is* two implementations disagreeing about where a
message ends; leniency that differs from a peer's leniency is the vector, so the
strict check runs first and independently of the parser.

**Every framing rejection closes the connection.** The stream is never
resynchronized after a malformed or ambiguous request, because the bytes
following one are exactly what an attacker wants read as a fresh request.

**An unread request body closes the connection.** If your handler returns without
reading the body, reuse would mean hunting for a request line inside a message
body. Draining instead would preserve keep-alive but hands an attacker a way to
make the server read bytes it has no use for. If you want keep-alive on a request
with a body, read the body.

**No `Trailers` event for an empty trailer section.** Boxing an empty header list
to announce that it is empty put an allocation on the hot path for no
information.

## Buffer pinning

Request data is a view into the connection's read buffer. That buffer is
allocated once per connection and reused for every request on it; there is **no**
cross-connection buffer pool, and deliberately so — a buffer that outlives its
connection is a buffer a handler may still hold a `Bytes` slice of, and the
bookkeeping to detect that costs more than the allocation it saves.

The consequence for you: if a handler stores a header value or body chunk in
long-lived state, it pins that whole buffer for as long as it holds the slice.
This is bounded — one buffer per live connection, sized to one typical head — but
it is not free. Use `ByteStr::into_owned`-style copies for data you intend to
keep past the response.

## Protocol support

Handled: keep-alive, pipelining, chunked request bodies, trailers, `Expect:
100-continue` (sent lazily, on the first body read — a handler that rejects a
request without reading its body causes no interim response), `HEAD`, `CONNECT`,
`Upgrade` with raw-socket handoff, origin/absolute/asterisk-form targets,
HTTP/1.0 semantics.

**Caveat on `Upgrade`.** The handoff is available on `Connection::serve`, which
returns `Ok(Some(Upgraded))` — the transport plus the bytes the peer already sent
past the head — when a handler answers a request carrying `Connection: upgrade`
with status 101. `Server` has no upgrade-consumer hook, unlike the pluggable
HTTP/2 fallback, so under `Server::serve` an upgraded connection is **closed**.
Drive `Connection` yourself if you need the socket.

Rejected, per RFC 9110/9111/9112, each with a named test:

| Condition | Response |
|---|---|
| `Content-Length` and `Transfer-Encoding` both present | 400, close |
| duplicate or conflicting `Content-Length` | 400, close |
| `chunked` not the final transfer coding | 400, close |
| an unsupported transfer coding | 501, close |
| `Transfer-Encoding` on an HTTP/1.0 request | 400, close |
| obs-fold, bare CR, or bare LF in the head | 400, close |
| a byte in the request target outside RFC 3986's character set | 400, close |
| a target matching none of RFC 9112 §3.2's four forms | 400, close |
| a `#` fragment in the request target (RFC 9110 §7.1) | 400, close |
| whitespace before a field-name colon | 400, close |
| missing or multiple `Host` on HTTP/1.1 | 400, close |
| a framing field in a trailer section | 400, close |
| head over byte or field-count limit | 431, close |
| body over limit | 413, close |
| header or body deadline exceeded | 408, close |
| idle deadline exceeded between requests | silent close |
| write deadline exceeded, at any point in a streamed body | close |

On the response side, a handler-supplied header or trailer whose value contains
CR, LF, or NUL — or whose custom name is not an RFC 9110 token — is **dropped**
rather than written, and the writer frames the response itself as if the field
had never been supplied. A reflected `Location` or an echoed correlation id is
otherwise one bad input away from response splitting, which is request smuggling
run backwards.

## Measuring it

The zero-allocation claim is a test, not a paragraph:

```sh
cargo test -p armature-h1 --release --test alloc_regression -- --nocapture
```

A counting global allocator warms the pools, snapshots the counter, serves 100
more keep-alive requests, and asserts the delta is exactly zero — for plain GETs,
a browser-sized GET, a `Content-Length` POST, and a chunked POST. Exactly zero
rather than a threshold, because a threshold is a slow leak waiting to be
tolerated. One test also compares an early window against one 500 requests in, so
a per-request leak into a growing structure fails even if the absolute number
looks small.

Per-stage cost:

```sh
cargo bench -p armature-h1            # parse, write, and end-to-end over duplex
```

`benches/e2e.rs` runs over `tokio::io::duplex`, not a socket: a loopback TCP
benchmark measures the kernel more than it measures this crate. It also pays a
cross-thread wakeup per request, since `Connection` is `!Send` and the client half
has to be driven from another thread — read `benches/parse.rs` and
`benches/write.rs` for per-stage numbers without that overhead.

Against hyper over a real socket:

```sh
scripts/bench-h1.sh                   # needs `cargo install oha`
DURATION=30s CONNECTIONS=200 scripts/bench-h1.sh
```

It prints every version and command line it used, builds both servers before
either runs, and compares against a bare-hyper server sending the same body. Read
the p99, not the throughput headline: thread-per-core's cost is in the tail (see
above), which is exactly what a requests/sec number hides.

Fuzzing:

```sh
cd armature-h1 && cargo +nightly fuzz run framing_differential
```

Three targets: `parse_head` (no panic, and every `Bytes` in a parsed head points
into the input rather than a copy), `chunked` (semantics invariant under how the
input is split across reads), and `framing_differential` — the same bytes to
`framing::decide` and to hyper, failing when both accept and disagree on body
length, which is request smuggling by definition. That target is what found the
two request-target validations in the table above. CI runs each for 60 seconds on
pull requests touching this crate.

## Status

HTTP/1.1 only, and deliberately so. HTTP/2 remains served by `hyper` in
`armature-core`, reached via ALPN `h2` or the h2c prior-knowledge preface. HTTP/3
and gRPC are untouched.

See `docs/superpowers/specs/2026-07-29-armature-h1-design.md` for the full design
and `docs/superpowers/plans/2026-07-29-armature-h1-crate.md` for the build plan.
