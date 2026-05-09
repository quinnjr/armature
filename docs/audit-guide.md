# Audit & Compliance Guide

Comprehensive guide to audit logging and compliance in Armature.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Quick Start](#quick-start)
- [Audit Events](#audit-events)
- [Storage Backends](#storage-backends)
- [Data Masking](#data-masking)
- [Request Logging Middleware](#request-logging-middleware)
- [Retention Policies](#retention-policies)
- [Compliance](#compliance)
- [Best Practices](#best-practices)
- [Examples](#examples)
- [Summary](#summary)

---

## Overview

Armature's audit module provides comprehensive audit logging for security, compliance, and operational tracking. It's designed for enterprise applications that need SOC2, PCI-DSS, GDPR, or HIPAA compliance.

**Key Differences from Application Logging:**

| Application Logging | Audit Logging |
|---------------------|---------------|
| Debugging & monitoring | Compliance & security |
| Can be lost/rotated | Immutable records |
| General events | Who did what, when |
| Verbose | Structured |
| Optional | Required for compliance |

---

## Features

- ✅ **Structured Audit Events** - Who, what, when, where tracking
- ✅ **Automatic HTTP Logging** - Request/response middleware
- ✅ **Data Masking** - PII, passwords, credit cards, SSN
- ✅ **Multiple Backends** - File, memory, stdout, extensible
- ✅ **Retention Policies** - Automatic cleanup with configurable TTL
- ✅ **Compliance Ready** - PCI-DSS, GDPR, SOC2, HIPAA
- ✅ **Async & Thread-safe** - Production-ready performance

---

## Quick Start

### 1. Add Dependency

```toml
[dependencies]
armature-audit = "0.1"
```

### 2. Create Audit Logger

```rust
use armature_audit::*;
use std::sync::Arc;

let logger = Arc::new(
    AuditLogger::builder()
        .backend(FileBackend::new("audit.log"))
        .build()
);
```

### 3. Log Events

```rust
logger.log(AuditEvent::new("user.login")
    .user("alice")
    .ip("192.168.1.100")
    .action("authenticate")
    .status(AuditStatus::Success)).await?;
```

### 4. Add Middleware (Optional)

```rust
use armature_core::*;

let audit_middleware = Arc::new(AuditMiddleware::new(logger));

let app = Application::new()
    .middleware(audit_middleware)
    .build();
```

---

## Audit Events

### Event Structure

```rust
pub struct AuditEvent {
    id: String,                    // Unique event ID (UUID)
    timestamp: DateTime<Utc>,      // When it occurred
    event_type: String,            // Event type (e.g., "user.login")
    user_id: Option<String>,       // Who performed the action
    ip_address: Option<String>,    // From where
    user_agent: Option<String>,    // User's browser/client
    resource_type: Option<String>, // What was accessed
    resource_id: Option<String>,   // Specific resource
    action: String,                // What was done
    status: AuditStatus,           // Success/Failure/Denied/Error
    severity: AuditSeverity,       // Info/Warning/Error/Critical
    method: Option<String>,        // HTTP method
    path: Option<String>,          // Request path
    status_code: Option<u16>,      // HTTP status
    metadata: HashMap<...>,        // Custom fields
    error: Option<String>,         // Error message
    request_body: Option<String>,  // Request payload (masked)
    response_body: Option<String>, // Response payload (masked)
    duration_ms: Option<u64>,      // Duration
}
```

### Creating Events

```rust
use armature_audit::*;

// Basic event
let event = AuditEvent::new("user.login");

// Complete event
let event = AuditEvent::new("resource.update")
    .user("alice")
    .ip("192.168.1.100")
    .user_agent("Mozilla/5.0...")
    .resource("document")
    .resource_id("doc_123")
    .action("update")
    .status(AuditStatus::Success)
    .severity(AuditSeverity::Info)
    .method("PUT")
    .path("/api/documents/123")
    .status_code(200)
    .metadata("fields_changed", serde_json::json!(["title", "content"]))
    .duration_ms(150);
```

### Event Types

Use dot notation for hierarchy:

```rust
// Authentication
"user.login"
"user.logout"
"user.login.failed"

// Resource operations
"resource.create"
"resource.read"
"resource.update"
"resource.delete"

// Administrative
"admin.user.created"
"admin.role.assigned"
"admin.config.changed"

// Compliance
"gdpr.data_access"
"gdpr.data_export"
"pci.payment.processed"
```

### Status Values

```rust
pub enum AuditStatus {
    Success,  // Operation succeeded
    Failure,  // Operation failed
    Denied,   // Operation denied (authorization)
    Error,    // Operation resulted in error
}
```

### Severity Levels

```rust
pub enum AuditSeverity {
    Info,      // Informational
    Warning,   // Warrants attention
    Error,     // Error condition
    Critical,  // Critical security/compliance event
}
```

---

## Storage Backends

### FileBackend

Writes events to a file (one JSON object per line).

```rust
use armature_audit::*;

let backend = FileBackend::new("audit.log");

let logger = AuditLogger::builder()
    .backend(backend)
    .build();
```

**Format:** JSON Lines (JSONL)
```json
{"id":"...","timestamp":"...","event_type":"user.login",...}
{"id":"...","timestamp":"...","event_type":"resource.update",...}
```

### MemoryBackend

Stores events in memory (for testing and querying).

```rust
let backend = MemoryBackend::new();

// Query recent events
let events = backend.read(100).await?;

// Clear events
backend.clear().await;
```

### StdoutBackend

Prints events to stdout (for development).

```rust
let backend = StdoutBackend::new();
```

### MultiBackend

Write to multiple backends simultaneously.

```rust
let multi = MultiBackend::new()
    .add(Box::new(FileBackend::new("audit.log")))
    .add(Box::new(MemoryBackend::new()))
    .add(Box::new(StdoutBackend::new()));

let logger = AuditLogger::builder()
    .backend(multi)
    .build();
```

### Custom Backend

Implement the `AuditBackend` trait:

```rust
use armature_audit::*;
use async_trait::async_trait;

struct DatabaseBackend {
    // database connection
}

#[async_trait]
impl AuditBackend for DatabaseBackend {
    async fn write(&self, event: &AuditEvent) -> Result<(), AuditBackendError> {
        // Write to database
        Ok(())
    }

    async fn flush(&self) -> Result<(), AuditBackendError> {
        Ok(())
    }
}
```

---

## Data Masking

### Default Masking

Automatically masks common sensitive fields:

```rust
password, secret, token, api_key, credit_card, cvv, ssn, private_key
```

### Masking Configuration

```rust
use armature_audit::*;

let config = MaskingConfig::new()
    .add_field("custom_field")
    .mask_emails(true)
    .mask_phones(true)
    .mask_ssn(true)
    .mask_credit_cards(true)
    .mask_char('*')
    .show_last_chars(4);

let logger = AuditLogger::builder()
    .masking_config(config)
    .build();
```

### What Gets Masked

**Passwords & Tokens:**
```json
// Before
{"username": "alice", "password": "secret123"}

// After
{"username": "alice", "password": "******123"}
```

**Email Addresses:**
```
Before: Email: user@example.com
After:  Email: [EMAIL]
```

**Phone Numbers:**
```
Before: Phone: 123-456-7890
After:  Phone: [PHONE]
```

**Credit Cards:**
```
Before: Card: 4532-1234-5678-9010
After:  Card: [CARD]
```

**SSN:**
```
Before: SSN: 123-45-6789
After:  SSN: [SSN]
```

### Nested JSON Masking

```rust
let data = serde_json::json!({
    "user": {
        "name": "Alice",
        "credentials": {
            "password": "secret123",
            "api_key": "key_abc123"
        }
    }
});

let masked = mask_json(&data, &config);
// credentials.password and credentials.api_key are masked
```

---

## Request Logging Middleware

### Basic Usage

```rust
use armature_audit::*;
use armature_core::*;
use std::sync::Arc;

let logger = Arc::new(AuditLogger::builder()
    .backend(FileBackend::new("audit.log"))
    .build());

let audit_middleware = Arc::new(AuditMiddleware::new(logger));

let app = Application::new()
    .middleware(audit_middleware)
    .build();
```

### Configuration

```rust
let audit_middleware = Arc::new(
    AuditMiddleware::new(logger)
        .log_request_body(true)      // Log request bodies
        .log_response_body(true)     // Log response bodies
        .max_body_size(10_000)       // Max 10KB body size
);
```

### What Gets Logged

For each HTTP request:
- Event type: `http.request`
- HTTP method (GET, POST, etc.)
- Request path
- Status code
- User ID (from Authorization header)
- IP address (from X-Forwarded-For or X-Real-IP)
- User agent
- Request body (masked)
- Response body (masked)
- Duration in milliseconds

### Example Audit Log Entry

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "timestamp": "2024-12-13T10:30:45.123Z",
  "event_type": "http.request",
  "user_id": "authenticated_user",
  "ip_address": "192.168.1.100",
  "user_agent": "Mozilla/5.0...",
  "action": "http_request",
  "status": "success",
  "severity": "info",
  "method": "POST",
  "path": "/api/users",
  "status_code": 201,
  "request_body": "{\"username\":\"alice\",\"password\":\"******123\"}",
  "response_body": "{\"id\":123,\"username\":\"alice\"}",
  "duration_ms": 45
}
```

---

## Retention Policies

### Configuration

```rust
use armature_audit::*;
use chrono::Duration;

// Keep logs for 90 days
let policy = RetentionPolicy::days(90);

// Keep logs for 30 days, cleanup hourly
let policy = RetentionPolicy::days(30)
    .cleanup_interval(std::time::Duration::from_secs(3600));

// Keep logs for 7 days
let policy = RetentionPolicy::days(7);

// Keep logs for 24 hours
let policy = RetentionPolicy::hours(24);
```

### Retention Manager

```rust
use std::sync::Arc;

let backend = Arc::new(MemoryBackend::new());
let policy = RetentionPolicy::days(90);

let manager = Arc::new(RetentionManager::new(backend, policy));

// Start automatic cleanup
manager.clone().start().await;

// Manual cleanup
let deleted = manager.cleanup().await?;
println!("Deleted {} old logs", deleted);

// Stop cleanup
manager.stop().await;
```

### Common Retention Periods

| Use Case | Retention Period |
|----------|------------------|
| Development | 7 days |
| Standard apps | 30-90 days |
| SOC 2 | 1 year |
| PCI-DSS | 1 year (3 months online) |
| HIPAA | 6 years |
| GDPR | As needed, with deletion capability |

---

## Compliance

### PCI-DSS Compliance

For payment card data:

```rust
use armature_audit::*;

// Mask credit card data
let config = MaskingConfig::new()
    .add_field("credit_card")
    .add_field("cvv")
    .mask_credit_cards(true);

// Log payment transactions
logger.log(AuditEvent::new("payment.processed")
    .user(user_id)
    .resource("payment")
    .status(AuditStatus::Success)
    .severity(AuditSeverity::Critical)
    .metadata("amount", serde_json::json!(99.99))
    .metadata("compliance", serde_json::json!("PCI-DSS"))).await?;

// Retention: 1 year minimum
let policy = RetentionPolicy::days(365);
```

### GDPR Compliance

For personal data:

```rust
// Mask PII
let config = MaskingConfig::new()
    .mask_emails(true)
    .add_field("ssn")
    .add_field("date_of_birth");

// Log data access
logger.log(AuditEvent::new("gdpr.data_access")
    .user(admin_id)
    .resource("user_data")
    .resource_id(user_id)
    .action("data_export")
    .metadata("purpose", serde_json::json!("user request"))
    .metadata("compliance", serde_json::json!("GDPR"))).await?;

// Support deletion (right to be forgotten)
backend.delete_user_events(user_id).await?;
```

### SOC 2 Compliance

For security and availability:

```rust
// Log all administrative actions
logger.log(AuditEvent::new("admin.user.created")
    .user(admin_id)
    .resource("user")
    .resource_id(new_user_id)
    .action("create")
    .severity(AuditSeverity::Warning)
    .metadata("role", serde_json::json!("admin"))).await?;

// Log configuration changes
logger.log(AuditEvent::new("config.changed")
    .user(admin_id)
    .resource("system_config")
    .action("update")
    .severity(AuditSeverity::Critical)
    .metadata("setting", serde_json::json!("max_upload_size"))
    .metadata("old_value", serde_json::json!(10))
    .metadata("new_value", serde_json::json!(100))).await?;
```

### HIPAA Compliance

For healthcare data:

```rust
// Mask PHI
let config = MaskingConfig::new()
    .add_field("ssn")
    .add_field("medical_record_number")
    .add_field("patient_id")
    .mask_emails(true);

// Log PHI access
logger.log(AuditEvent::new("phi.accessed")
    .user(doctor_id)
    .resource("medical_record")
    .resource_id(patient_id)
    .action("view")
    .severity(AuditSeverity::Warning)
    .metadata("compliance", serde_json::json!("HIPAA"))
    .metadata("purpose", serde_json::json!("treatment"))).await?;

// Retention: 6 years minimum
let policy = RetentionPolicy::days(365 * 6);
```

---

## Best Practices

### 1. Log Meaningful Events

```rust
// ✅ Good - specific and actionable
logger.log(AuditEvent::new("user.password.reset")
    .user(user_id)
    .action("password_reset")
    .status(AuditStatus::Success)
    .metadata("method", serde_json::json!("email_link"))).await?;

// ❌ Bad - too generic
logger.log(AuditEvent::new("event")
    .action("action")).await?;
```

### 2. Use Appropriate Severity

```rust
// Info - Normal operations
logger.log(AuditEvent::new("user.profile.viewed")
    .severity(AuditSeverity::Info)).await?;

// Warning - Attention needed
logger.log(AuditEvent::new("user.login.failed")
    .severity(AuditSeverity::Warning)).await?;

// Error - System errors
logger.log(AuditEvent::new("api.error")
    .severity(AuditSeverity::Error)).await?;

// Critical - Security/compliance events
logger.log(AuditEvent::new("admin.role.assigned")
    .severity(AuditSeverity::Critical)).await?;
```

### 3. Always Log Security Events

```rust
// ✅ Always log:
- Login attempts (success and failure)
- Logout events
- Permission changes
- Administrative actions
- Data access/export
- Configuration changes
- Payment transactions
- PHI/PII access
```

### 4. Include Context

```rust
// ✅ Good - includes context
logger.log(AuditEvent::new("document.deleted")
    .user("alice")
    .resource("document")
    .resource_id("doc_123")
    .metadata("document_type", serde_json::json!("contract"))
    .metadata("reason", serde_json::json!("user request"))).await?;

// ❌ Bad - no context
logger.log(AuditEvent::new("deleted")).await?;
```

### 5. Handle Failures Gracefully

```rust
// ✅ Good - don't fail the request if audit fails
if let Err(e) = logger.log(event).await {
    tracing::error!("Failed to log audit event: {}", e);
}

// ❌ Bad - failing request
logger.log(event).await?; // Request fails if audit fails!
```

---

## Examples

### Example 1: Basic Audit Logging

```rust
use armature_audit::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create logger
    let logger = Arc::new(
        AuditLogger::builder()
            .backend(FileBackend::new("audit.log"))
            .build()
    );

    // Log event
    logger.log(AuditEvent::new("user.login")
        .user("alice")
        .ip("192.168.1.100")
        .status(AuditStatus::Success)).await?;

    Ok(())
}
```

### Example 2: Request/Response Logging

```rust
use armature_core::*;
use armature_audit::*;
use std::sync::Arc;

let logger = Arc::new(AuditLogger::builder()
    .backend(FileBackend::new("audit.log"))
    .build());

let audit_middleware = Arc::new(
    AuditMiddleware::new(logger)
        .log_request_body(true)
        .log_response_body(true)
);

let app = Application::new()
    .middleware(audit_middleware)
    .build();
```

### Example 3: Multiple Backends

```rust
let multi = MultiBackend::new()
    .add(Box::new(FileBackend::new("audit.log")))
    .add(Box::new(MemoryBackend::new()))
    .add(Box::new(StdoutBackend::new()));

let logger = AuditLogger::builder()
    .backend(multi)
    .build();
```

### Example 4: Custom Masking

```rust
let config = MaskingConfig::new()
    .add_field("credit_card")
    .add_field("ssn")
    .add_field("medical_id")
    .mask_emails(true)
    .mask_phones(true)
    .show_last_chars(4);

let logger = AuditLogger::builder()
    .masking_config(config)
    .build();
```

### Example 5: Retention Policy

```rust
use chrono::Duration;

let backend = Arc::new(MemoryBackend::new());
let policy = RetentionPolicy::days(90);
let manager = Arc::new(RetentionManager::new(backend, policy));

// Start automatic cleanup
manager.clone().start().await;

// Stop when done
manager.stop().await;
```

---

## Summary

**Key Points:**

1. **Audit logs are for compliance** - not debugging
2. **Use structured events** - who, what, when, where
3. **Mask sensitive data** - PII, passwords, credit cards
4. **Choose appropriate retention** - based on compliance needs
5. **Use multiple backends** - for redundancy
6. **Log security events** - always
7. **Don't fail requests** - if audit logging fails

**Quick Reference:**

```rust
// Create logger
let logger = Arc::new(AuditLogger::builder()
    .backend(FileBackend::new("audit.log"))
    .build());

// Log event
logger.log(AuditEvent::new("user.action")
    .user("alice")
    .status(AuditStatus::Success)).await?;

// Add middleware
let app = Application::new()
    .middleware(Arc::new(AuditMiddleware::new(logger)))
    .build();

// Setup retention
let manager = Arc::new(RetentionManager::new(
    backend,
    RetentionPolicy::days(90)
));
manager.clone().start().await;
```

**Compliance Checklist:**

- ✅ Track who did what, when
- ✅ Mask sensitive data (PII, passwords, etc.)
- ✅ Immutable audit trail
- ✅ Appropriate retention periods
- ✅ Secure storage
- ✅ Query capability
- ✅ Deletion capability (GDPR)

**Resources:**
- [PCI-DSS Requirements](https://www.pcisecuritystandards.org/)
- [GDPR Guidelines](https://gdpr.eu/)
- [SOC 2 Framework](https://www.aicpa.org/soc2)
- [HIPAA Security Rule](https://www.hhs.gov/hipaa/)

