# Changelog — `armature-opensearch`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `bulk_index`/`bulk_delete` chunk their requests, so they are no longer atomic — a large call previously exceeded `http.max_content_length` and was rejected wholesale.
- Bulk error counting no longer looks only under the `index` action key, so failures under another action are not reported as successes.
