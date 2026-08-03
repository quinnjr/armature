# Changelog — `armature-config`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- Environment variables map `__` to the path separator `.` (`APP__DATABASE__URL` → `database.url`). Keys were only lowercased, so no environment variable could satisfy the dotted lookups the docs demonstrate.
- `FileFormat::Env` parses through `dotenvy`, the same parser `load_dotenv` uses. The hand-rolled parser it replaces understood neither `export` prefixes, escapes, inline comments nor multi-line values, and stripped every quote rather than one matched pair — so the two paths disagreed on the same file.
- `ConfigValidator::in_range` and `one_of` interpolate the bounds and the offending value; the messages read literally "must be between min and max". Both gained a `T: Display` bound.

### Changed

- `ConfigManager::has` and dotted-path `get` no longer deep-clone. Traversal was cloning the entire subtree at every level of a path, and `has` cloned a value only to discard it.
- Source precedence (dotenv → env → files, later wins, so a **file overrides an environment variable**) is documented on `ConfigServiceBuilder` and pinned by a test. This is the inverse of 12-factor; it is documented rather than reordered because reversing it would silently change what existing deployments resolve.
