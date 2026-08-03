# Changelog — `armature-http-client`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- An invalid per-request header name or value is warned about rather than dropped in silence — this had been silently stripping `bearer_auth` when a token contained a stray newline.
- `max_attempts = 0` no longer underflows into effectively infinite retries.
