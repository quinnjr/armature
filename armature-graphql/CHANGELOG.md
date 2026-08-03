# Changelog — `armature-graphql`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The subscription cap decrements on server-side completion and bounds its tracking set even when unlimited; one-shot queries arrive as `subscribe` and so counted against the cap without ever being released.
