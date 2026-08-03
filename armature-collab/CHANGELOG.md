# Changelog — `armature-collab`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- `LwwMap::remove` records a tombstone for a key the replica has not observed. A causally later removal delivered before its add was silently lost, breaking convergence.
- `OperationBuffer::ready` uses a dependants index rather than repeated linear passes with mid-vector removal, and the unbounded `applied` set is documented with an opt-in limit.
