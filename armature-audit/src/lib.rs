//! Audit logging and compliance for Armature
//!
//! This crate provides comprehensive audit logging for security, compliance,
//! and operational tracking.
//!
//! # Features
//!
//! - **Audit Events** - Structured [`AuditEvent`] logging with status/severity
//! - **Request/Response Logging** - [`AuditMiddleware`] logs HTTP payloads and
//!   records the caller's real principal from the bearer token's subject claim
//! - **Data Masking** - PII/sensitive data masking via [`mask_json`],
//!   [`mask_string`], and [`mask_value`]
//! - **Retention Policies** - [`RetentionManager`] deletes logs past their age
//! - **Backends** - [`FileBackend`] (JSON-lines file), [`MemoryBackend`],
//!   [`StdoutBackend`], and [`MultiBackend`] fan-out
//!
//! Events serialize to JSON; there is no bundled database or query backend.
//!
//! # Quick Start
//!
//! ```no_run
//! use armature_audit::*;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create audit logger
//! let audit = AuditLogger::builder()
//!     .backend(FileBackend::new("audit.log"))
//!     .build();
//!
//! // Log an event
//! audit.log(AuditEvent::new("user.login")
//!     .user("alice")
//!     .resource("system")
//!     .action("authenticate")
//!     .status(AuditStatus::Success)).await?;
//! # Ok(())
//! # }
//! ```

pub mod backend;
pub mod event;
pub mod logger;
pub mod masking;
pub mod middleware;
pub mod retention;

pub use backend::*;
pub use event::*;
pub use logger::*;
pub use masking::*;
pub use middleware::*;
pub use retention::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Just ensure all exports are accessible
        let _ = AuditEvent::new("test");
    }
}
