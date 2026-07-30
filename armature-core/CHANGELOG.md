# Changelog — `armature-core`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Changes at or before `0.6.0` are recorded in the workspace
[`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Removed

- **Breaking — `0.6.0` → `0.7.0`:** removed the `tower_compat` module and the
  `tower`/`tower-service` dependencies. The module existed solely for Tower
  interop — `ArmatureService`, `HyperServiceAdapter`, `ServiceFactory` and
  `ArmatureLayerService` implemented `tower_service::Service`, `ArmatureLayer`
  implemented `tower::Layer`, and `tower_stats()`/`TowerStats` counted
  conversions — which pulled the whole `tower` façade (plus its unused `util`
  feature) into every `armature-core` build for a single trait impl. The
  `http`-crate conversion traits that lived alongside it (`IntoHttpRequest`,
  `FromHttpRequest`, `IntoHttpResponse`, `HttpResponseFromHttp`, `HeaderMapExt`,
  `ArmatureHeaderMapExt`) are removed with it; they had no consumers outside the
  module. No in-tree crate, example, or template referenced any of it.
