//! # Armature Azure
//!
//! Microsoft Azure services integration with dynamic loading and dependency injection.
//!
//! ## Features
//!
//! Services are loaded dynamically based on feature flags and configuration.
//! Only the services you enable are compiled and loaded.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_azure::{AzureServices, AzureConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure which services to load
//!     let config = AzureConfig::builder()
//!         .storage_account("mystorageaccount")
//!         .enable_blob()
//!         .enable_cosmos()
//!         .build();
//!
//!     // Load services
//!     let services = AzureServices::new(config).await?;
//!
//!     // Use Blob Storage
//!     let blob = services.blob_client()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## With Dependency Injection
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_azure::{AzureServices, AzureConfig};
//!
//! #[module]
//! struct AzureModule;
//!
//! #[module_impl]
//! impl AzureModule {
//!     #[provider(singleton)]
//!     async fn azure_services() -> AzureServices {
//!         let config = AzureConfig::from_env()
//!             .enable_blob()
//!             .enable_servicebus()
//!             .build();
//!         AzureServices::new(config).await.unwrap()
//!     }
//! }
//! ```

mod config;
mod error;
mod services;

pub use config::{AzureConfig, AzureConfigBuilder, CredentialsSource};
pub use error::{AzureError, Result};
pub use services::AzureServices;

// Re-export Azure SDK types
#[cfg(feature = "auth")]
pub use azure_core;

#[cfg(feature = "auth")]
pub use azure_identity;

#[cfg(feature = "blob")]
pub use azure_storage_blob;

#[cfg(feature = "queue")]
pub use azure_storage_queue;

#[cfg(feature = "cosmos")]
pub use azure_data_cosmos;

#[cfg(feature = "servicebus")]
pub use azure_messaging_servicebus;

#[cfg(feature = "keyvault")]
pub use azure_security_keyvault_secrets;
