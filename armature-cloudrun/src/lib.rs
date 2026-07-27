// Allow dead_code while crate is under development
#![allow(dead_code)]
//! # Armature Cloud Run
//!
//! Google Cloud Run deployment utilities for Armature applications.
//!
//! Cloud Run is a fully managed compute platform that runs containers.
//! This crate provides utilities to optimize Armature applications for Cloud Run.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_cloudrun::{CloudRunConfig, init_tracing};
//!
//! #[controller("/")]
//! struct HelloController;
//!
//! #[controller_impl]
//! impl HelloController {
//!     #[get("/")]
//!     async fn hello() -> &'static str {
//!         "Hello from Cloud Run!"
//!     }
//! }
//!
//! #[module(controllers: [HelloController])]
//! struct AppModule;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize tracing for Cloud Logging
//!     init_tracing();
//!
//!     // Get port from Cloud Run environment
//!     let config = CloudRunConfig::from_env();
//!
//!     // Create and run Armature application
//!     Application::create::<AppModule>()
//!         .listen(&config.bind_address())
//!         .await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## With GCP Services
//!
//! ```rust,ignore
//! use armature_cloudrun::{CloudRunConfig, init_tracing};
//! use armature_gcp::{GcpServices, GcpConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     init_tracing();
//!
//!     // Initialize GCP services
//!     let gcp = GcpServices::new(
//!         GcpConfig::from_env()
//!             .enable_firestore()
//!             .enable_storage()
//!             .build()
//!     ).await?;
//!
//!     // Register in DI container
//!     let app = Application::create::<AppModule>();
//!     app.container().register(gcp);
//!
//!     let config = CloudRunConfig::from_env();
//!     app.listen(&config.bind_address()).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Deployment
//!
//! ```bash
//! # Build container
//! docker build -t gcr.io/PROJECT_ID/my-app .
//!
//! # Push to Container Registry
//! docker push gcr.io/PROJECT_ID/my-app
//!
//! # Deploy to Cloud Run
//! gcloud run deploy my-app \
//!     --image gcr.io/PROJECT_ID/my-app \
//!     --platform managed \
//!     --region us-central1 \
//!     --allow-unauthenticated
//! ```
//!
//! ## Cloud Run Features
//!
//! This crate helps with:
//! - **Port Configuration**: Reads PORT environment variable
//! - **Cloud Logging**: Structured JSON logging for Cloud Logging
//! - **Health Checks**: Built-in health check endpoint support
//! - **Graceful Shutdown**: Proper SIGTERM handling
//! - **Instance Metadata**: Access to Cloud Run instance info

mod config;
mod error;
mod health;
mod metadata;

pub use config::CloudRunConfig;
pub use error::{CloudRunError, Result};
pub use health::{
    CheckResult, FnHealthChecker, HealthCheck, HealthCheckResult, HealthChecker, HealthStatus,
};
pub use metadata::{InstanceMetadata, ServiceMetadata};

/// Initialize tracing for Cloud Logging.
///
/// On Cloud Run (detected via `K_SERVICE`) this installs
/// [`tracing_stackdriver::layer()`], emitting Stackdriver-structured JSON that
/// Cloud Logging parses into severity, message and structured fields;
/// elsewhere it falls back to `tracing_subscriber`'s JSON formatter.
///
/// Note: this does **not** perform automatic Cloud Trace correlation. No
/// `logging.googleapis.com/trace` / `spanId` fields are attached, because no
/// trace-context is extracted from the incoming `X-Cloud-Trace-Context` /
/// `traceparent` header here. To correlate logs with traces you must propagate
/// the trace context yourself (e.g. via an `armature-opentelemetry` layer).
pub fn init_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Use Stackdriver-compatible format for Cloud Logging
    if std::env::var("K_SERVICE").is_ok() {
        // Running on Cloud Run - use Stackdriver format
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_stackdriver::layer())
            .init();
    } else {
        // Local development - use standard format
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    }
}

/// Initialize tracing with a custom log level.
pub fn init_tracing_with_level(level: &str) {
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

    let filter = tracing_subscriber::EnvFilter::new(level);

    if std::env::var("K_SERVICE").is_ok() {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_stackdriver::layer())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    }
}

/// Check if running on Cloud Run.
pub fn is_cloud_run() -> bool {
    std::env::var("K_SERVICE").is_ok()
}

/// Get the Cloud Run service name.
pub fn service_name() -> Option<String> {
    std::env::var("K_SERVICE").ok()
}

/// Get the Cloud Run revision name.
pub fn revision_name() -> Option<String> {
    std::env::var("K_REVISION").ok()
}

/// Get the Cloud Run configuration name.
pub fn configuration_name() -> Option<String> {
    std::env::var("K_CONFIGURATION").ok()
}

/// Wait for shutdown signal (SIGTERM).
///
/// Cloud Run sends SIGTERM when scaling down or deploying new revisions.
/// This function waits for that signal to enable graceful shutdown.
pub async fn wait_for_shutdown() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("Received SIGTERM, starting graceful shutdown");
        }
        _ = sigint.recv() => {
            tracing::info!("Received SIGINT, starting graceful shutdown");
        }
    }
}
