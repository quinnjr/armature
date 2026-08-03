# Changelog — `armature-messaging`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `ProcessingResult::DeadLetter` on SQS no longer calls `DeleteMessage`. Deleting is precisely how a message does *not* reach a DLQ — SQS redrives after `maxReceiveCount` — so requesting dead-lettering discarded the message, indistinguishably from success. It now accelerates redelivery and requires a redrive policy on the queue.
- **Breaking:** `subscribe_with_options` errors for `AckMode::Manual` on backends that cannot honour it and for any `filter`, rather than silently downgrading. `Manual` was a synonym for `Auto` on three of four backends, with no ack handle on `Message` to make it meaningful, and `filter` was read by no backend at all.
- **Breaking:** `MessageHandler::on_deserialize_error` is removed; every backend conversion is infallible, so the framework had no site from which to call it.
- The per-message `JoinSet` is reaped as it goes; completed entries accumulated for a subscription's whole lifetime.
