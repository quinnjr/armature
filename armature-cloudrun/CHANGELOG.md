# Changelog — `armature-cloudrun`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- Health checkers run concurrently under a per-check timeout. They were awaited in sequence with no timeout, so readiness latency was the sum rather than the max and one hung checker stalled `/readyz` indefinitely — surfacing on Cloud Run as a probe timeout rather than an unhealthy result.
- The crate's `tokio` features are declared rather than borrowed from workspace feature unification.
