# Changelog — `armature-log`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The colored Pretty format brackets the target, matching the documented layout — colors are on by default in `preset_development()`, so the documented form was the one nobody saw.

### Fixed

- The colored Pretty formatter emits the brackets around the target (`[my_app]`), matching the documented format. They were dropped only on the colored path — which `preset_development()` enables by default, so the documented shape was the one almost nobody saw.
