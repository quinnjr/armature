# Changelog — `armature-eventsourcing`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `with_snapshots` requires `A: Serialize` and plain `save` now writes snapshots. The configured frequency was previously unhonored — `create_snapshot` was a stub that only logged — so the naming-obvious combination snapshotted nothing.
- The version invariant `load_events`/`save_events` depend on is documented on `Aggregate` and asserted in debug builds.
- The crate no longer advertises persistent storage: it ships a pluggable trait and an in-memory implementation for testing.
