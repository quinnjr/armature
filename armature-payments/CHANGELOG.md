# Changelog — `armature-payments`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `Money::from_decimal`/`from_float` are deprecated and now panic rather than returning zero. An amount that did not round-trip through `i64` silently became a charge of nothing, reported as success; `try_from_decimal`/`try_from_float` are the fallible replacements.
- `Price::is_on_sale`/`discount_percent` check currency before comparing minor units.
