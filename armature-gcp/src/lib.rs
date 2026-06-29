//! # Armature GCP
//!
//! Google Cloud Platform services integration with dynamic loading and dependency injection.
//!
//! ## Features
//!
//! Services are loaded dynamically based on feature flags and configuration.
//! Only the services you enable are compiled and loaded.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_gcp::{GcpServices, GcpConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure which services to load
//!     let config = GcpConfig::builder()
//!         .project_id("my-project")
//!         .enable_storage()
//!         .enable_pubsub()
//!         .build();
//!
//!     // Load services
//!     let services = GcpServices::new(config).await?;
//!
//!     // Use Cloud Storage
//!     let storage = services.storage()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## With Dependency Injection
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_gcp::{GcpServices, GcpConfig};
//!
//! #[module]
//! struct GcpModule;
//!
//! #[module_impl]
//! impl GcpModule {
//!     #[provider(singleton)]
//!     async fn gcp_services() -> GcpServices {
//!         let config = GcpConfig::from_env()
//!             .enable_storage()
//!             .enable_pubsub()
//!             .build();
//!         GcpServices::new(config).await.unwrap()
//!     }
//! }
//! ```

mod config;
mod error;
mod services;

pub use config::{CredentialsSource, GcpConfig, GcpConfigBuilder};
pub use error::{GcpError, Result};
pub use services::GcpServices;

// Re-export enabled service clients
#[cfg(feature = "storage")]
pub use google_cloud_storage;

#[cfg(feature = "pubsub")]
pub use google_cloud_pubsub;

#[cfg(feature = "spanner")]
pub use google_cloud_spanner;

#[cfg(feature = "bigquery")]
pub use gcloud_bigquery;
