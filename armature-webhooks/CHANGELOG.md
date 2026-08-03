# Changelog — `armature-webhooks`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** the shared delivery body is `Bytes`, so it is genuinely shared rather than copied per endpoint.
- Fan-out concurrency is bounded, and a receiver's response body is capped instead of buffered without limit on every attempt.
