//! # Armature AWS
//!
//! AWS cloud services integration with dynamic loading and dependency injection.
//!
//! ## Features
//!
//! Services are loaded dynamically based on feature flags and configuration.
//! Only the services you enable are compiled and loaded.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use armature_aws::{AwsServices, AwsConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure which services to load
//!     let config = AwsConfig::builder()
//!         .region("us-east-1")
//!         .enable_s3()
//!         .enable_dynamodb()
//!         .build();
//!
//!     // Load services
//!     let services = AwsServices::new(config).await?;
//!
//!     // Use S3
//!     let s3 = services.s3()?;
//!     let buckets = s3.list_buckets().send().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## With Dependency Injection
//!
//! ```rust,ignore
//! use armature::prelude::*;
//! use armature_aws::{AwsServices, AwsConfig};
//!
//! #[module]
//! struct AwsModule;
//!
//! #[module_impl]
//! impl AwsModule {
//!     #[provider(singleton)]
//!     async fn aws_services() -> AwsServices {
//!         let config = AwsConfig::from_env()
//!             .enable_s3()
//!             .enable_sqs()
//!             .build();
//!         AwsServices::new(config).await.unwrap()
//!     }
//!
//!     #[provider]
//!     fn s3_client(services: &AwsServices) -> aws_sdk_s3::Client {
//!         services.s3().unwrap().clone()
//!     }
//! }
//! ```

mod config;
mod error;
mod services;

#[cfg(feature = "dynamodb")]
pub mod dynamodb;

#[cfg(feature = "sqs")]
pub mod sqs;

#[cfg(feature = "sns")]
pub mod sns;

#[cfg(feature = "ses")]
pub mod ses;

#[cfg(feature = "lambda")]
pub mod lambda;

#[cfg(feature = "secrets-manager")]
pub mod secrets;

#[cfg(feature = "ssm")]
pub mod ssm;

#[cfg(feature = "cloudwatch")]
pub mod cloudwatch;

#[cfg(feature = "kinesis")]
pub mod kinesis;

#[cfg(feature = "kms")]
pub mod kms;

#[cfg(feature = "cognito")]
pub mod cognito;

pub use config::{AwsConfig, AwsConfigBuilder, CredentialsSource};
pub use error::{AwsError, Result};
pub use services::AwsServices;

// Re-export AWS types for convenience
pub use aws_config;
pub use aws_credential_types;
pub use aws_types;

// Re-export enabled service clients
#[cfg(feature = "s3")]
pub use aws_sdk_s3;

#[cfg(feature = "dynamodb")]
pub use aws_sdk_dynamodb;

#[cfg(feature = "sqs")]
pub use aws_sdk_sqs;

#[cfg(feature = "sns")]
pub use aws_sdk_sns;

#[cfg(feature = "ses")]
pub use aws_sdk_sesv2;

#[cfg(feature = "lambda")]
pub use aws_sdk_lambda;

#[cfg(feature = "secrets-manager")]
pub use aws_sdk_secretsmanager;

#[cfg(feature = "ssm")]
pub use aws_sdk_ssm;

#[cfg(feature = "cloudwatch")]
pub use aws_sdk_cloudwatch;

#[cfg(feature = "kinesis")]
pub use aws_sdk_kinesis;

#[cfg(feature = "kms")]
pub use aws_sdk_kms;

#[cfg(feature = "cognito")]
pub use aws_sdk_cognito_idp as aws_sdk_cognito;
