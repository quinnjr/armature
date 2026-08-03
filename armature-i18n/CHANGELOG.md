# Changelog — `armature-i18n`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** the fallback chain is evaluated per key, not per bundle, so a key present in `fr.json` but missing from `fr-CA.json` no longer skips `fr` entirely; `has` follows the same chain `t` does.
- Plural and bundle lookups no longer allocate per probe.
