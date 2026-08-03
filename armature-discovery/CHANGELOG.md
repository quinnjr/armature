# Changelog — `armature-discovery`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** etcd registrations carry a lease with a keep-alive, and both keys are written in one transaction. Without a lease a crashed instance stayed discoverable forever — the opposite of finding healthy instances — and a partial write left a service discoverable but not deregisterable.
- The default health probe reuses one HTTP client instead of building a fresh connection pool and TLS config per probe.
