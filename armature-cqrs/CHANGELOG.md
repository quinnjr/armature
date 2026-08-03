# Changelog — `armature-cqrs`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The command and query buses drop the handler-map guard before awaiting. Holding a `DashMap` shard's read lock across an arbitrarily long handler future blocked concurrent registration and made a re-entrant `register()` self-deadlock.
