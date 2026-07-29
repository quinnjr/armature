//! Armature API - Full Template
//!
//! A production-ready REST API with authentication and validation.
//!
//! Run with: cargo run
//! Health check: http://localhost:3000/health

mod config;
mod controllers;
mod middleware;
mod models;
mod services;

use armature::prelude::*;
use tracing::{error, info};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::controllers::{AuthController, HealthController, UserController};
use crate::models::UserRole;
use crate::services::{get_auth_service, get_user_service, init_auth_service, init_user_service};

// =============================================================================
// Application Module
// =============================================================================

#[module(
    controllers: [HealthController, AuthController, UserController]
)]
#[derive(Default)]
struct AppModule;

// =============================================================================
// Main
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load environment variables
    dotenvy::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .init();

    info!("Starting Armature API (Full Template)");

    // Load configuration
    let config = AppConfig::from_env();

    info!(host = %config.host, port = %config.port, "Configuration loaded");

    // Initialize process-wide services (see src/services/*.rs)
    if let Err(e) = init_auth_service(config.jwt_secret.clone(), config.jwt_expiry_hours) {
        error!(error = %e, "failed to initialize AuthService (invalid JWT configuration)");
        std::process::exit(1);
    }
    init_user_service();

    // Seed a default admin account, opt-in only (off by default) — this is
    // a development convenience, not something that should run
    // unconditionally in every environment including production.
    //
    // Set `SEED_ADMIN=true` to enable it. The password is either taken from
    // `SEED_ADMIN_PASSWORD` (if the operator wants a known password) or
    // generated randomly; either way it is never printed in plaintext.
    let seed_admin = std::env::var("SEED_ADMIN")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if seed_admin {
        let seed_password = std::env::var("SEED_ADMIN_PASSWORD")
            .unwrap_or_else(|_| Uuid::new_v4().simple().to_string());

        let admin_password_hash =
            get_auth_service()
                .hash_password(&seed_password)
                .unwrap_or_else(|e| {
                    error!(error = %e, "failed to hash seed admin password");
                    std::process::exit(1);
                });
        get_user_service().create_with_role(
            "admin@example.com".to_string(),
            admin_password_hash,
            "Admin User".to_string(),
            UserRole::Admin,
        );

        eprintln!(
            "Seed admin account created: admin@example.com — password was not printed; \
             set SEED_ADMIN_PASSWORD to choose it explicitly, or check that env var to recall it."
        );
    }

    println!();
    println!("Available endpoints:");
    println!("  GET    /health            - Health check");
    println!("  GET    /health/live       - Liveness probe");
    println!("  GET    /health/ready      - Readiness probe");
    println!("  POST   /api/auth/login    - Login");
    println!("  POST   /api/auth/register - Register");
    println!("  GET    /api/users         - List users (auth required)");
    println!("  GET    /api/users/:id     - Get user (auth required)");
    println!("  DELETE /api/users/:id     - Delete user (auth required)");
    println!();

    // Create application
    let app = Application::create::<AppModule>().await;

    // Start server.
    // `Application::listen` always binds all interfaces on the given port;
    // `config.host` / `HOST` is logged above for operator visibility but
    // isn't a bind address in this version of armature-core.
    info!(host = %config.host, port = %config.port, "Server starting");

    app.listen(config.port).await?;

    Ok(())
}
