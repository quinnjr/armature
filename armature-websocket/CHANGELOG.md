# Changelog — `armature-websocket`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- **Breaking:** `From<Message> for tungstenite::Message` becomes `TryFrom`. A hand-built Text frame with a non-UTF-8 payload was lossily converted, silently corrupting it with replacement characters.
- Room broadcast iterates the member map directly and passes payloads through without a per-recipient copy.

### Changed

- **Breaking:** `From<Message> for tungstenite::Message` is now `TryFrom`, with the new `InvalidTextPayload` error. A text message whose payload is not valid UTF-8 was converted with `from_utf8_lossy`, silently replacing invalid bytes with U+FFFD; it is now rejected and the single message is dropped with an error log rather than corrupting the stream.
- Binary/Ping/Pong payloads pass through as `Bytes` instead of `to_vec()` — one full memcpy per recipient removed from every broadcast.
- Room broadcast iterates the membership map directly (the new `Room::for_each_member`) instead of first materializing a `Vec` of cloned connection ids.
