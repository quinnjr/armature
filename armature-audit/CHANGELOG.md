# Changelog — `armature-audit`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** a client-supplied `X-Forwarded-For` is no longer recorded as fact. `trusted_proxy_depth` selects the hop (default: record no IP), and `record_unverified_ip` tags an untrusted value. The same module already refused a forgeable user id for exactly this reason.
- `FileBackend::read` streams into a bounded ring buffer instead of loading an append-only, unbounded log into memory, and counts corrupt lines over the whole scan rather than the returned window.

### Changed — `0.1.2` → `0.1.3`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).
- The audited method and path are captured as owned strings from the request's new accessors.
