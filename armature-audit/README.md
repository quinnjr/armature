# armature-audit

Audit logging and compliance for the Armature framework.

## Features

- **Structured audit events** — `AuditEvent` with actor, action, resource,
  status, severity, HTTP context, and metadata.
- **Request/response middleware** — `AuditMiddleware` logs HTTP requests and
  records the caller's real principal from the bearer token's subject claim.
- **Data masking** — automatic PII/secret masking for JSON, strings, and
  individual values (`mask_json`, `mask_string`, `mask_value`).
- **Retention** — `RetentionManager` deletes events older than a configured age.
- **Backends** — `FileBackend` (one JSON object per line), `MemoryBackend`,
  `StdoutBackend`, and `MultiBackend` (fan-out to several backends).

Events serialize to JSON. There is no bundled database backend or query builder.

## Installation

```toml
[dependencies]
armature-audit = "0.1"
```

## Quick Start

```rust,no_run
use armature_audit::*;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
// Create an audit logger backed by a JSON-lines file.
let audit = AuditLogger::builder()
    .backend(FileBackend::new("/var/log/audit.log"))
    .build();

// Log an event.
audit.log(AuditEvent::new("user.update")
    .user("user123")
    .resource("user")
    .resource_id("456")
    .action("update")
    .status(AuditStatus::Success)).await?;
# Ok(())
# }
```

## Middleware

Register `AuditMiddleware` after your authentication layer so it can read the
verified bearer token. It records each request/response and extracts the
principal from the JWT `sub` claim (configurable via `.subject_claim(..)`).

```rust,no_run
use armature_audit::*;
use std::sync::Arc;

let logger = Arc::new(AuditLogger::builder()
    .backend(FileBackend::new("/var/log/audit.log"))
    .build());

let middleware = AuditMiddleware::new(logger)
    .log_request_body(true)
    .max_body_size(8_192);
```

## Retention

```rust,no_run
use armature_audit::*;
use std::sync::Arc;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let backend = Arc::new(FileBackend::new("/var/log/audit.log"));
let manager = RetentionManager::new(backend, RetentionPolicy::days(90));

// Run once, or `Arc::new(manager).start().await` for a background sweep.
let deleted = manager.cleanup().await?;
println!("Deleted {deleted} expired audit entries");
# Ok(())
# }
```

## Data Masking

```rust
use armature_audit::*;
use serde_json::json;

let config = MaskingConfig::default();
let masked = mask_json(&json!({ "password": "secret123" }), &config);
assert_ne!(masked["password"], "secret123");
```

## License

MIT OR Apache-2.0
