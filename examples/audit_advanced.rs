#![allow(clippy::needless_question_mark)]
//! Advanced Audit Example
//!
//! Demonstrates advanced audit features including:
//! - Multiple backends
//! - Custom masking rules
//! - Retention policies
//! - Compliance logging
//!
//! Run with:
//! ```bash
//! cargo run --example audit_advanced
//! ```

use armature_audit::*;
use armature_core::handler::from_legacy_handler;
use armature_core::*;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let _guard = LogConfig::default().init();

    info!("Advanced Audit Example");
    info!("======================");

    // Create multiple backends
    info!("\nConfiguring audit backends...");
    let file_backend = FileBackend::new("audit.log");
    let memory_backend = Arc::new(MemoryBackend::new());
    let stdout_backend = StdoutBackend::new();

    // Combine backends
    let multi_backend = MultiBackend::new()
        .with_backend(Box::new(file_backend))
        .with_backend(Box::new(MemoryBackend::new()))
        .with_backend(Box::new(stdout_backend));

    info!("✓ Multiple backends configured:");
    info!("  - File (audit.log)");
    info!("  - Memory (for querying)");
    info!("  - Stdout (for development)");

    // Custom masking configuration
    info!("\nConfiguring data masking...");
    let masking_config = MaskingConfig::new()
        .add_field("credit_card")
        .add_field("ssn")
        .mask_emails(true)
        .mask_phones(true)
        .mask_char('*')
        .show_last_chars(4);

    info!("✓ Masking configured for:");
    info!("  - Passwords, tokens, API keys");
    info!("  - Credit cards");
    info!("  - SSNs");
    info!("  - Email addresses");
    info!("  - Phone numbers");

    // Create audit logger
    let audit_logger = Arc::new(
        AuditLogger::builder()
            .backend(multi_backend)
            .masking_config(masking_config)
            .build(),
    );

    // Setup retention policy
    info!("\nConfiguring retention policy...");
    let retention_policy =
        RetentionPolicy::days(90).cleanup_interval(std::time::Duration::from_secs(3600)); // Hourly

    let retention_manager = Arc::new(RetentionManager::new(
        memory_backend.clone(),
        retention_policy,
    ));

    info!("✓ Retention policy: 90 days, cleanup hourly");

    // Start retention manager
    retention_manager.clone().start().await;
    info!("✓ Retention manager started");

    // Create router
    let mut router = Router::new();

    // Clone for handlers
    let audit_for_payment = audit_logger.clone();
    let audit_for_gdpr = audit_logger.clone();
    let memory_for_query = memory_backend.clone();

    // Compliance-sensitive endpoint
    router.add_route(Route {
        method: HttpMethod::POST,
        path: "/api/payment".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let logger = audit_for_payment.clone();
            Box::pin(async move {
                let body_str = String::from_utf8_lossy(&req.body);

                // Log PCI-DSS compliance event
                logger
                    .log(
                        AuditEvent::new("payment.processed")
                            .user("customer_12345")
                            .ip("192.168.1.100")
                            .action("process_payment")
                            .resource("payment")
                            .status(AuditStatus::Success)
                            .severity(AuditSeverity::Critical)
                            .metadata("amount", serde_json::json!(99.99))
                            .metadata("currency", serde_json::json!("USD"))
                            .metadata("compliance", serde_json::json!("PCI-DSS"))
                            .request_body(body_str.to_string()),
                    )
                    .await
                    .ok();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "status": "success",
                    "transaction_id": "txn_abc123"
                }))?)
            })
        })),
        constraints: None,
    });

    // GDPR data access endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/user/:id/data".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let logger = audit_for_gdpr.clone();
            Box::pin(async move {
                let user_id = req.param("id").unwrap();

                // Log GDPR data access
                logger
                    .log(
                        AuditEvent::new("gdpr.data_access")
                            .user("admin")
                            .action("data_export")
                            .resource("user_data")
                            .resource_id(user_id)
                            .status(AuditStatus::Success)
                            .severity(AuditSeverity::Warning)
                            .metadata("compliance", serde_json::json!("GDPR"))
                            .metadata("purpose", serde_json::json!("user request")),
                    )
                    .await
                    .ok();

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "user_id": user_id,
                    "data": {
                        "name": "John Doe",
                        "email": "john@example.com"
                    }
                }))?)
            })
        })),
        constraints: None,
    });

    // Audit query endpoint
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/audit/recent".to_string(),
        handler: from_legacy_handler(Arc::new(move |_req: HttpRequest| {
            let backend = memory_for_query.clone();
            Box::pin(async move {
                // Query recent audit events
                let events = backend
                    .read(10)
                    .await
                    .map_err(|_| Error::Internal("Failed to read audit logs".to_string()))?;

                Ok(HttpResponse::ok().with_json(&serde_json::json!({
                    "count": events.len(),
                    "events": events
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
                    "message": "Advanced Audit Example",
                    "features": {
                        "multiple_backends": "File + Memory + Stdout",
                        "data_masking": "PII, credit cards, SSN, emails",
                        "retention": "90 days with automatic cleanup",
                        "compliance": "PCI-DSS, GDPR"
                    },
                    "endpoints": {
                        "POST /api/payment": "Payment processing (PCI-DSS)",
                        "GET /api/user/:id/data": "User data export (GDPR)",
                        "GET /api/audit/recent": "Query recent audit logs"
                    }
                }))?)
            })
        })),
        constraints: None,
    });

    // Build application
    let container = Container::new();
    let app = Application::new(container, router);

    info!("\n✓ Server started on http://localhost:3000");
    info!("\nCompliance features:");
    info!("  ✓ PCI-DSS payment logging");
    info!("  ✓ GDPR data access tracking");
    info!("  ✓ 90-day retention policy");
    info!("  ✓ Sensitive data masking");
    info!("\nPress Ctrl+C to stop");

    app.listen(3000).await?;

    // Cleanup
    retention_manager.stop().await;

    Ok(())
}
