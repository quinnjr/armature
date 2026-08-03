# Changelog — `armature-graphql-client`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Fixed

- The WebSocket handshake requests the `graphql-transport-ws` subprotocol and asserts the negotiated value. Without it the client could not connect to this framework's own GraphQL server, nor to Apollo Server or graphql-ws.
- The client waits for a real `connection_ack` instead of proceeding unacked when the socket closes or the first frame is not text, and dropping a subscription no longer panics outside a runtime.
