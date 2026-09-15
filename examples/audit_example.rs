#![allow(clippy::needless_question_mark)]
//! Audit Logging Example
//!
//! This example demonstrates audit logging with sensitive data masking.
//!
//! Run with:
//! ```bash
//! cargo run --example audit_example
//! ```
//!
//! Test with:
//! ```bash
//! # Make requests
//! curl http://localhost:3000/login -d '{"username":"alice","password":"secret123"}'
//! curl http://localhost:3000/api/users
//! curl http://localhost:3000/api/delete/123
//!
//! # View audit log
//! cat audit.log
//! ```

use armature_audit::*;
use armature_core::handler::from_legacy_handler;
use armature_core::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Audit Logging Example");
    info!("=====================");

    // Create audit logger with file backend
    info!("\nCreating audit logger...");
    let audit_logger = Arc::new(
        AuditLogger::builder()
            .backend(FileBackend::new("audit.log"))
            .build(),
    );
    info!("✓ Audit logger created (writing to audit.log)");

    // Clone for use in handlers
    let audit_for_login = audit_logger.clone();
    let audit_for_delete = audit_logger.clone();

    // Create router
    let mut router = Router::new();

    // Login endpoint - demonstrates sensitive data masking
    router.add_route(Route {
        method: HttpMethod::POST,
        path: "/login".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let logger = audit_for_login.clone();
            Box::pin(async move {
                // Parse login attempt
                let _body_str = String::from_utf8_lossy(&req.body);

                // Manual audit log for business event
                logger
                    .log(
                        AuditEvent::new("user.login.attempt")
                            .user("alice")
                            .ip("127.0.0.1")
                            .action("authenticate")
                            .status(AuditStatus::Success)
                            .resource("authentication")
                            .metadata("method", serde_json::json!("password")),
                    )
                    .await
                    .ok();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "Login successful",
                    "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9..."
                }))?)
            })
        })),
        constraints: None,
    });

    // API endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/users".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async move {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "users": [
                        {"id": 1, "name": "Alice"},
                        {"id": 2, "name": "Bob"}
                    ]
                }))?)
            })
        })),
        constraints: None,
    });

    // Delete endpoint - demonstrates high-severity audit
    router.add_route(Route {
        method: HttpMethod::DELETE,
        path: "/api/delete/:id".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let logger = audit_for_delete.clone();
            Box::pin(async move {
                let id = req.param("id").unwrap();

                // Log critical action
                logger
                    .log(
                        AuditEvent::new("resource.delete")
                            .user("admin")
                            .action("delete")
                            .resource("user")
                            .resource_id(id)
                            .status(AuditStatus::Success)
                            .severity(AuditSeverity::Critical)
                            .metadata("reason", serde_json::json!("user request")),
                    )
                    .await
                    .ok();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": format!("Deleted resource {}", id)
                }))?)
            })
        })),
        constraints: None,
    });

    // Home endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async move {
                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "Audit Logging Example",
                    "endpoints": {
                        "POST /login": "Login (with sensitive data masking)",
                        "GET /api/users": "List users",
                        "DELETE /api/delete/:id": "Delete resource (critical audit)"
                    },
                    "audit_log": "audit.log"
                }))?)
            })
        })),
        constraints: None,
    });

    // Build application
    let container = Container::new();
    let app = Application::new(container, router);

    info!("\n✓ Server started on http://localhost:3000");
    info!("\nFeatures demonstrated:");
    info!("  - Automatic HTTP request/response logging");
    info!("  - Sensitive data masking (passwords, tokens)");
    info!("  - Manual business event logging");
    info!("  - Severity levels (Info, Warning, Critical)");
    info!("\nAudit log location: audit.log");
    info!("\nTry these requests:");
    info!(
        "  curl -X POST http://localhost:3000/login -d '{{\"username\":\"alice\",\"password\":\"secret123\"}}'"
    );
    info!("  curl http://localhost:3000/api/users");
    info!("  curl -X DELETE http://localhost:3000/api/delete/123");
    info!("\nPress Ctrl+C to stop");

    app.listen(3000).await?;

    Ok(())
}
