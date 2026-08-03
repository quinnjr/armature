# Changelog — `armature-core`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changes at or before `0.6.0` are recorded in the workspace
[`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Changed

- **Behaviour — static assets:** a pre-compressed sibling (`.gz`, `.br`, …) is
  now ignored unless it can be *proven* fresh. Previously a sibling older than
  its source was served, and so was one whose freshness could not be
  established at all (the source's or the sibling's mtime being unreadable).
  Either case hands the client outdated bytes under the current `ETag` and
  `Last-Modified`, which caches then hold. When freshness is unprovable the
  server now compresses the current source on the fly, which is always correct
  and merely costs CPU. A sibling's own mtime and length also feed the `ETag`
  and the in-memory content-cache key, so rewriting only the artifact — the
  usual `gzip -k` re-run — no longer reuses the source-derived validator.
- **Behaviour — `handle_websocket`:** stream and handler errors now propagate
  through the `Result` instead of being logged and swallowed, so a caller can
  distinguish a clean client close from a protocol or handler failure. A
  received Close frame is passed to the handler for teardown and the close
  handshake is flushed before the stream is dropped, replacing the abortive TCP
  close peers previously observed (RFC 6455 §5.5.1).
- **Behaviour — `WebSocketRoom::broadcast`:** connections whose receivers have
  all been dropped are reaped instead of accumulating in the room forever.
- `From<tungstenite::Message>` maps a raw `Frame` to `Binary` rather than
  `Close`; the previous catch-all told handlers the peer was closing when it
  was not.
- `LoggerMiddleware` records `duration_ms` as a number, matching
  `RequestLoggerMiddleware`, instead of a unit-varying `Debug` string.

### Fixed

- Static asset serving no longer makes blocking `std::fs` calls on the async
  request path, and stats each inode once per request rather than up to four
  times.
- Lifecycle hook dispatch no longer holds the hook-registry read lock across
  hook `await`s, which starved concurrent registration under a write-preferring
  lock, deadlocked a hook that registered another hook, and caused
  `register_on_init_sync` to silently drop hooks registered from inside one.

### Removed

- **Breaking — `0.6.0` → `0.7.0`:** removed the `tower_compat` module and the
  `tower`/`tower-service` dependencies. The module existed solely for Tower
  interop — `ArmatureService`, `HyperServiceAdapter`, `ServiceFactory` and
  `ArmatureLayerService` implemented `tower_service::Service`, `ArmatureLayer`
  implemented `tower::Layer`, and `tower_stats()`/`TowerStats` counted
  conversions — which pulled the whole `tower` façade (plus its unused `util`
  feature) into every `armature-core` build for a single trait impl. The
  `http`-crate conversion traits that lived alongside it (`IntoHttpRequest`,
  `FromHttpRequest`, `IntoHttpResponse`, `HttpResponseFromHttp`, `HeaderMapExt`,
  `ArmatureHeaderMapExt`) are removed with it; they had no consumers outside the
  module. No in-tree crate, example, or template referenced any of it.
