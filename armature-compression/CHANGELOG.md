# Changelog — `armature-compression`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** a configured algorithm the client does not accept now yields no compression; `Accept-Encoding` was not consulted at all unless the algorithm was `Auto`, so a client advertising only `br` received gzip it may not decode.
- `q=0` is honoured as "not acceptable" per RFC 9110 §12.5.3, along with `*` and `identity;q=0`; q-values were parsed off and then ignored.

### Fixed

- `Accept-Encoding` q-values are honoured. They were stripped and then ignored, so `Accept-Encoding: br;q=0, gzip` selected Brotli — RFC 9110 §12.5.3 defines `q=0` as "not acceptable". A `*` wildcard and `identity;q=0` are now handled, and among acceptable codings the highest q wins (ties broken by the server's br > zstd > gzip preference).
- A configured (non-`Auto`) algorithm is intersected with what the client accepts. `Accept-Encoding` was not consulted at all in that mode, so a client advertising only `br` could be sent gzip it may be unable to decode; the response now goes out uncompressed instead.

### Added

- `CompressionAlgorithm::is_accepted_by`.

### Changed — `0.1.3` → `0.1.4`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- Response bodies are cleared as `Bytes`.
