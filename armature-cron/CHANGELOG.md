# Changelog — `armature-cron`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- `next_run` advances at dispatch rather than completion. With `prevent_overlap` disabled a running job kept its past due time and was re-dispatched every tick — a daily five-minute job fired roughly 300 times, and the cron expression stopped governing firing entirely.
