# Changelog — `armature-mcp`

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Earlier changes are recorded in the workspace [`CHANGELOG.md`](../CHANGELOG.md).

## [Unreleased]

### Changed — `0.1.5` → `0.1.6`

- Migrated onto `armature-core` `0.8`'s `Bytes`-backed request and response types. No behavior change beyond what that migration implies; see [`armature-core/CHANGELOG.md`](../armature-core/CHANGELOG.md).

### Fixed

- **JSON-RPC notifications are no longer answered.** Per JSON-RPC 2.0 §4.1 a request without an `id` must not receive a response. `notifications/initialized` — which every MCP client sends immediately after `initialize` — previously came back as `-32601 Method not found`. Notifications are now dispatched for their side effects only, and `POST /mcp` answers `204 No Content`. `notifications/initialized` and `notifications/cancelled` are recognised no-ops.
- **The advertised `prompts` capability is now actually served.** `initialize` announced `capabilities.prompts` whenever `enable_prompts` was set, but there were no `prompts/list` / `prompts/get` handlers. Both methods now exist, backed by a new `McpPromptRegistry` (compile-time `register_mcp_prompt!` inventory) plus dynamic `McpPromptProvider`s registered with `McpService::with_prompt_provider`.
- A JSON-RPC batch (top-level JSON array) is rejected with `-32600 Invalid Request` instead of the misleading `-32700 Parse error`. Batching remains unsupported and is now documented as such.
- A malformed `input_schema` on a registered tool is still replaced with a permissive `{"type":"object"}`, but now logs at `error` level and trips a `debug_assert!` rather than silently shipping the bug to clients.

### Changed

- **Breaking (API):** `McpService::handle_request` now returns `Option<JsonRpcResponse>`, and `handle_json` / `handle_json_unauthenticated` return `Option<String>`; `None` means "notification — send no body".
- Added `McpService::handle_bytes`, which parses the JSON-RPC payload directly from a byte slice. `POST /mcp` uses it, removing the per-request body copy and intermediate `String`.
- `GET /mcp` now reports `prompts_count` alongside `tools_count` and `resources_count`.
