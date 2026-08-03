# Changelog — `armature-cache`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** an explicit "no TTL" is distinguishable from an unspecified one, so `remember_forever` stops silently inheriting `default_ttl` — on the documented configuration there was no way to store a non-expiring entry.
- Memcached expirations over 30 days are sent as absolute timestamps. The protocol reads any larger value that way, so a 40-day TTL stored an item already expired and `set_json` still returned `Ok(())`.
- The tag index no longer outlives the values it points at, and its writes are batched instead of costing three sequential round-trips per tag.
- L1 eviction is no longer a full scan under the write lock on every insert once full — which every L2 promotion went through.
- `warm_cache` bounds its concurrency instead of issuing one simultaneous factory call per key, the stampede its sibling single-flight exists to prevent.
