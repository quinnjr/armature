# Changelog — `armature-storage`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The fully-buffered API's size ceiling is documented: every object is materialized in memory in both directions, and single-request `PutObject` caps S3 objects at 5 GiB.
