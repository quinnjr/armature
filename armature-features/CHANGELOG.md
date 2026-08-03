# Changelog — `armature-features`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The per-thread regex cache is bounded. Patterns can arrive from a remote flag service, and the cache was never evicted, so cost grew with distinct patterns times threads.
