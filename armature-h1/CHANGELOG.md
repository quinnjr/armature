# Changelog — `armature-h1`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Every deadline in `Limits` is now enforced against something that polls it.
  `body_timeout` was armed and never awaited, so it had no effect at all — and
  its test passed by reaching the idle timeout instead. `write_timeout` now
  covers every flush of a streamed body rather than only the final write.
- Handler-supplied header and trailer values containing CR, LF or NUL are
  rejected, and `HeaderId::Other` names are re-validated as tokens. Response
  splitting is request smuggling run backwards, and the framing checks now
  consult what was actually emitted so a dropped `Content-Length` cannot leave a
  response undelimited.
- `Transfer-Encoding` on an HTTP/1.0 request is rejected (the TE-downgrade
  smuggling vector), and `#` is rejected in the request target — RFC 9110 §7.1
  puts a fragment outside it, and no RFC 9112 §3.2 form admits one.
- `Response::upgrade`, `BufPool` and `Limits::max_pipeline_depth` are removed.
  Each was public API that did nothing: the callback was only ever dropped
  unrun, the pool had no call sites while the README documented its
  observability, and the pipeline cap was never read.

### Added — `0.1.0` (new crate)

A zero-allocation HTTP/1.1 server layer. The steady-state request path — parse,
dispatch, write — performs no heap allocations at all, which
`tests/alloc_regression.rs` asserts against a budget of zero per request.

Types:

- `Method` — well-known methods are unit variants, so dispatch is a discriminant
  comparison; an unrecognized token is carried as `Method::Other(ByteStr)`.
  Includes `QUERY` (draft-ietf-httpbis-safe-method-w-body). `From<&str>`,
  `From<String>`, `PartialEq<str>`, `PartialEq<&str>` and `Display` let it stand
  in for the `String` it replaces at a call site.
- `ByteStr` — an immutable UTF-8 string backed by `Bytes`, so a request target or
  header value can be a refcounted slice of the connection's read buffer rather
  than a fresh `String`. Derefs to `str`; `Hash` delegates to `str` so the
  `Borrow<str>` impl is sound for map lookups.
- `HeaderId` — well-known header names as an enum, with `header::intern` mapping
  a name to one (lowercasing an unrecognized name so lookups stay
  case-insensitive), and `HeaderVec` keeping 16 headers inline.
- `Version`, `Limits`, `ConnConfig`, `Connection`, `Request`, `Response`,
  `DateCache`.

Framing follows RFC 9112 §6: `framing::decide` resolves `Transfer-Encoding` and
`Content-Length` together and rejects the combinations that enable request
smuggling, including `Transfer-Encoding` on an HTTP/1.0 request (the TE-downgrade
vector). Request targets are validated against RFC 3986's character set and
RFC 9112 §3.2's four target forms; a `#` fragment is rejected, since RFC 9110
§7.1 puts it outside the request target.

The reverse direction is covered too: a handler-supplied header or trailer whose
value contains CR, LF, or NUL — or whose `HeaderId::Other` name is not an RFC
9110 token — is dropped rather than written, and the writer frames the response
as if it had never been supplied. Response splitting is request smuggling run
backwards.

Every deadline in `Limits` is enforced against something that polls it: the
header and idle deadlines against the head read, the body deadline against the
handler call itself, and the write deadline against *every* flush of a streamed
response rather than only the last.

Protocol upgrades leave through `Connection::serve`'s `Ok(Some(Upgraded))`.
`Server` has no upgrade-consumer hook and closes such connections; a service that
upgrades must drive `Connection` itself.

`Connection` is `!Send` by design — it holds `Rc`s — and is driven on a
thread-per-core runtime with `SO_REUSEPORT`.

The crate keeps `#![forbid(unsafe_code)]`; the counting allocator the regression
test needs lives in the test target, not the library.

Fuzzing: three `cargo-fuzz` targets (`parse_head`, `chunked`,
`framing_differential`). The differential target compares framing decisions
against hyper and panics only when both implementations accept a message but
disagree on its body length — which is what a smuggling primitive looks like.
