# Changelog — `armature-session`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `RedisSessionStore::save` returns an error when the computed TTL is not positive, instead of skipping the write and returning `Ok(())` — `create()` could hand back a session that was never persisted.
