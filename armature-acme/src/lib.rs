//! # Armature ACME
//!
//! ACME (Automatic Certificate Management Environment) client for obtaining
//! and renewing SSL/TLS certificates from providers like Let's Encrypt.
//!
//! ## Features
//!
//! - ✅ **Automatic Certificate Management** - Obtain and renew certificates automatically
//! - ✅ **Multiple Providers** - Support for Let's Encrypt, ZeroSSL, BuyPass, and more
//! - ✅ **Challenge Types** - HTTP-01, DNS-01, and TLS-ALPN-01 challenges
//! - ✅ **Account Management** - Register and manage ACME accounts
//! - ✅ **External Account Binding** - Support for providers requiring EAB
//! - ✅ **Automatic Renewal** - Check and renew certificates before expiration
//!
//! ## Quick Start
//!
//! ```no_run
//! use armature_acme::{AcmeClient, AcmeConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure ACME client for Let's Encrypt staging (testing)
//!     let config = AcmeConfig::lets_encrypt_staging(
//!         vec!["admin@example.com".to_string()],
//!         vec!["example.com".to_string(), "www.example.com".to_string()],
//!     ).with_accept_tos(true);
//!
//!     // Create client
//!     let mut client = AcmeClient::new(config).await?;
//!
//!     // Order certificate
//!     let (cert_pem, key_pem) = client.order_certificate().await?;
//!
//!     // Save certificate and key
//!     client.save_certificate(&cert_pem, &key_pem).await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Certificate Providers
//!
//! ### Let's Encrypt (Production)
//!
//! ```
//! use armature_acme::AcmeConfig;
//!
//! let config = AcmeConfig::lets_encrypt_production(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//! );
//! ```
//!
//! ### Let's Encrypt (Staging - for testing)
//!
//! ```
//! use armature_acme::AcmeConfig;
//!
//! let config = AcmeConfig::lets_encrypt_staging(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//! );
//! ```
//!
//! ### ZeroSSL (requires EAB)
//!
//! ```
//! use armature_acme::AcmeConfig;
//!
//! let config = AcmeConfig::zerossl(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//!     "your_eab_kid".to_string(),
//!     "your_eab_hmac_key".to_string(),
//! );
//! ```
//!
//! ## Challenge Types
//!
//! ### HTTP-01 Challenge
//!
//! HTTP-01 challenges require serving a file at a specific URL on port 80.
//!
//! ```
//! use armature_acme::{AcmeConfig, ChallengeType};
//!
//! let config = AcmeConfig::lets_encrypt_staging(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//! ).with_challenge_type(ChallengeType::Http01);
//! ```
//!
//! ### DNS-01 Challenge
//!
//! DNS-01 challenges require creating a TXT record in your DNS zone.
//! This is required for wildcard certificates.
//!
//! ```
//! use armature_acme::{AcmeConfig, ChallengeType};
//!
//! let config = AcmeConfig::lets_encrypt_staging(
//!     vec!["admin@example.com".to_string()],
//!     vec!["*.example.com".to_string()],
//! ).with_challenge_type(ChallengeType::Dns01);
//! ```
//!
//! ### TLS-ALPN-01 Challenge
//!
//! TLS-ALPN-01 challenges require TLS configuration on port 443.
//!
//! ```
//! use armature_acme::{AcmeConfig, ChallengeType};
//!
//! let config = AcmeConfig::lets_encrypt_staging(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//! ).with_challenge_type(ChallengeType::TlsAlpn01);
//! ```
//!
//! ## Integration with Armature
//!
//! Use ACME certificates with Armature's HTTPS server:
//!
//! ```no_run
//! use armature_acme::{AcmeClient, AcmeConfig};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Obtain certificate
//! let config = AcmeConfig::lets_encrypt_production(
//!     vec!["admin@example.com".to_string()],
//!     vec!["example.com".to_string()],
//! ).with_accept_tos(true);
//!
//! let mut client = AcmeClient::new(config).await?;
//! let (cert_pem, key_pem) = client.order_certificate().await?;
//! let (cert_path, key_path) = client.save_certificate(&cert_pem, &key_pem).await?;
//!
//! // Use with Armature
//! // let tls_config = TlsConfig::from_pem_files(&cert_path, &key_path)?;
//! // app.listen_https(443, tls_config).await?;
//! # Ok(())
//! # }
//! ```

pub mod account;
pub mod challenge;
pub mod client;
pub mod config;
pub mod directory;
pub mod error;
pub mod jws;
pub mod order;

pub use account::*;
pub use challenge::*;
pub use client::*;
pub use config::*;
pub use directory::*;
pub use error::*;
pub use jws::dns01_txt_value;
pub use order::*;
