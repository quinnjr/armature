# Changelog — `armature-events`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `subscribe` requires the handler to handle the event it is registered under. A mismatch used to compile, fail its downcast at publish time, and be swallowed by the default `continue_on_error` — so the handler silently never ran and the publisher saw success.
- **Breaking:** `publish` returns a `PublishReport` with invoked and failed handler counts.
