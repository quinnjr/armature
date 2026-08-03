# Changelog — `armature-ferron`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- Backend health checks run concurrently, so one backend hitting its timeout no longer delays every subsequent check or pushes the loop past its tick interval.

### Changed

- Background health checks run all backends concurrently via `JoinSet`. Serially, one backend hitting `config.timeout` delayed every check behind it and could push a sweep past the tick interval.
