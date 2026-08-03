# Changelog — `armature-queue`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** in-flight jobs are reclaimed. `dequeue` wrote a claim timestamp that nothing ever read back, so a crashed or SIGKILLed worker stranded its job in no queue at all — never retried, never dead-lettered. `WorkerConfig::visibility_timeout` drives a reaper, and pop-and-claim is now one atomic script.
- **Breaking:** jobs scheduled beyond the retention window run. Their body expired before the due time, so promotion silently skipped them and left a permanent entry in the delayed set that inflated `backlog_size()` until `max_size` tripped.
- `StopOutcome` counts only job-processing tasks; the reaper is cancelled up front so the numbers still describe in-flight jobs.
