# Changelog — `armature-security`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `allow_credentials(true)` now clears `allow_any_origin` (and vice versa), and the preflight/apply paths refuse to emit an origin at all if the combination is somehow reached. Previously the two could be set together and any attacker origin was reflected alongside `Access-Control-Allow-Credentials: true` — the doc forbade the pairing, nothing enforced it, and `permissive().allow_credentials(true)` reached it in two calls.
- Request-signature verification no longer skips on a `starts_with` prefix of the raw target. `should_skip` matches whole segments of `path_only()` and rejects dot segments, so `/health../admin` and `/healthcheck-admin` no longer bypass verification.
- A signature timestamped in the future is rejected rather than treated as infinitely fresh; `saturating_sub` had clamped the age to zero.
- A disallowed preflight method returns `Forbidden` instead of a 204 with the header quietly omitted, and `Vary` is appended rather than overwritten.

### Changed — `0.1.3` → `0.1.4`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- Request-signature verification reads the method through `method_str()`; signatures are unchanged.
